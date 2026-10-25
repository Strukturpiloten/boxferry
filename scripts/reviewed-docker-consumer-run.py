#!/usr/bin/env python3
"""Closed two-job builds, exact inert tests and the reviewed volume-only profile."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import selectors
import signal
import stat
import subprocess
import sys
import time


SPEC = importlib.util.spec_from_file_location(
    "reviewed_docker_evidence", Path(__file__).with_name("reviewed-docker-consumer-evidence.py"))
assert SPEC is not None and SPEC.loader is not None
proof = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(proof)
auth = proof.auth
TRANSFER_SPEC = importlib.util.spec_from_file_location(
    "reviewed_docker_transfer", Path(__file__).with_name("reviewed-docker-consumer-transfer.py"))
assert TRANSFER_SPEC is not None and TRANSFER_SPEC.loader is not None
transfer = importlib.util.module_from_spec(TRANSFER_SPEC)
TRANSFER_SPEC.loader.exec_module(transfer)
EXAMPLES = {"volume-fixtures": "docker-volume-fixture-rehearsal", "host-metadata": "docker-host-metadata"}
FAILURE_PHASES = frozenset({
    "arguments", "build-layout", "build-context", "build-authorization", "build-copy",
    "volume-fixtures-build", "volume-fixtures-capture", "volume-fixtures-receipt",
    "host-metadata-build", "host-metadata-capture", "host-metadata-receipt", "inert-build",
    "inert-discovery", "inert-capture", "inert-tests", "build-source-recheck", "build-state", "build-export",
    "native-runtime",
})
RUNTIME_FACTS = {
    "GITHUB_ACTIONS": "true", "RUNNER_ENVIRONMENT": "github-hosted", "RUNNER_OS": "Linux",
    "GITHUB_REF": "refs/heads/main", "GITHUB_EVENT_NAME": "workflow_dispatch",
    "GITHUB_REPOSITORY": "Strukturpiloten/boxferry",
    "GITHUB_JOB": "lanes",
    "GITHUB_WORKFLOW_REF": "Strukturpiloten/boxferry/.github/workflows/reviewed-docker-consumer-validation.yml@refs/heads/main",
    "ImageOS": "ubuntu24",
}


def runtime_environment(environ, *, require_docker=False):
    """Explicit accident-guard facts only; trusted admission remains separate."""
    engine = environ.get("DOCKERLENS_OUTER_ENGINE", "podman")
    auth.require(engine in {"podman", "docker"} and (not require_docker or engine == "docker"))
    selected = {"DOCKERLENS_OUTER_ENGINE": engine}
    forbidden = ("DOCKER_HOST", "DOCKER_CONTEXT", "DOCKER_TLS_VERIFY", "DOCKER_CERT_PATH", "DOCKER_API_VERSION",
                 "CONTAINER_HOST", "CONTAINER_CONNECTION", "CONTAINER_SSHKEY", "PODMAN_CONNECTIONS_CONF")
    auth.require(not any(environ.get(key) for key in forbidden))
    if engine == "docker":
        auth.require(all(environ.get(key) == value for key, value in RUNTIME_FACTS.items()))
        image = environ.get("ImageVersion", "")
        auth.require(type(image) is str and re.fullmatch(r"[0-9]{8}\.[0-9]+\.[0-9]+", image) is not None)
        selected.update(RUNTIME_FACTS)
        selected["ImageVersion"] = image
        for key in ("GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT"):
            value = environ.get(key, "")
            auth.require(type(value) is str and re.fullmatch(r"[1-9][0-9]{0,19}", value) is not None)
            selected[key] = value
        candidate = environ.get("DOCKERLENS_NATIVE_CANDIDATE_SHA", "")
        auth.sha(candidate, auth.SHA)
        selected["DOCKERLENS_NATIVE_CANDIDATE_SHA"] = candidate
    return selected


def write(path: Path, value) -> None:
    auth.require(path.is_absolute() and path.parent.resolve(strict=True) == path.parent)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "wb") as stream:
        stream.write(proof.encoded(value))


def binary_digest(path: Path, root: Path) -> str:
    auth.require(path.is_absolute() and path.resolve(strict=True) == path and path.is_relative_to(root / "target"))
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as stream:
        before = os.fstat(stream.fileno())
        auth.require(stat.S_ISREG(before.st_mode) and before.st_nlink == 1 and before.st_mode & stat.S_IXUSR
                     and 0 < before.st_size <= 512 * 1024 * 1024)
        digest = hashlib.sha256()
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
        after = os.fstat(stream.fileno())
        auth.require((before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns)
                     == (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns))
    return digest.hexdigest()


def capture_compiler_executable(path: Path, root: Path, profile: str, *, deadline: float) -> str:
    """Break Cargo's output aliases only at the three known build-owned paths.

    Runtime and bundle readers still require single links. The independent inode
    replaces only the observed compiler output, never a downloaded artifact.
    """
    auth.require(root.is_absolute() and root.resolve(strict=True) == root
                 and path.is_absolute() and path.resolve(strict=True) == path)
    if profile in EXAMPLES:
        auth.require(path == root / "target/debug/examples" / EXAMPLES[profile])
    else:
        auth.require(profile == "inert-tests" and path.parent == root / "target/debug/deps"
                     and re.fullmatch(r"docker_authored_bind_rehearsal-[0-9a-f]{16}", path.name) is not None)
    auth.require(time.monotonic() < deadline)
    directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    temporary = None
    owned = None
    handlers = {}
    cancelled = None
    identity = lambda item: (item.st_dev, item.st_ino, item.st_mode, item.st_uid, item.st_nlink,
                             item.st_size, item.st_mtime_ns, item.st_ctime_ns)

    def cancel_capture(kind, _frame):
        nonlocal cancelled
        if cancelled is None:
            cancelled = kind

    def check_budget():
        if cancelled is not None:
            raise SystemExit(128 + cancelled)
        auth.require(time.monotonic() < deadline)

    try:
        parent = os.fstat(directory)
        auth.require(stat.S_ISDIR(parent.st_mode) and parent.st_uid == os.geteuid())
        for kind in (signal.SIGTERM, signal.SIGINT):
            handlers[kind] = signal.signal(kind, cancel_capture)
        descriptor = os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
        with os.fdopen(descriptor, "rb") as source:
            before = os.fstat(source.fileno())
            auth.require(stat.S_ISREG(before.st_mode) and before.st_uid == os.geteuid()
                         and before.st_nlink >= 1 and before.st_mode & stat.S_IXUSR
                         and 64 <= before.st_size <= 512 * 1024 * 1024)
            temporary = ".reviewed-compiler-" + secrets.token_hex(16)
            output = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=directory)
            with os.fdopen(output, "wb") as snapshot:
                item = os.fstat(snapshot.fileno())
                owned = item.st_dev, item.st_ino
                digest, size = hashlib.sha256(), 0
                while chunk := source.read(1024 * 1024):
                    check_budget()
                    if size == 0:
                        auth.require(chunk.startswith(b"\x7fELF"))
                    size += len(chunk)
                    auth.require(size <= before.st_size)
                    digest.update(chunk)
                    snapshot.write(chunk)
                snapshot.flush()
                os.fchmod(snapshot.fileno(), 0o700)
                os.fsync(snapshot.fileno())
                copied = os.fstat(snapshot.fileno())
                auth.require(size == before.st_size and copied.st_size == size and copied.st_nlink == 1
                             and copied.st_uid == os.geteuid() and stat.S_ISREG(copied.st_mode))
            auth.require(identity(before) == identity(os.fstat(source.fileno()))
                         and identity(before) == identity(os.stat(path.name, dir_fd=directory, follow_symlinks=False)))
            current_parent = path.parent.stat(follow_symlinks=False)
            auth.require(path.parent.resolve(strict=True) == path.parent
                         and (current_parent.st_dev, current_parent.st_ino) == (parent.st_dev, parent.st_ino))
            check_budget()
            auth.require(identity(copied) == identity(os.stat(temporary, dir_fd=directory, follow_symlinks=False)))
            os.replace(temporary, path.name, src_dir_fd=directory, dst_dir_fd=directory)
            temporary = None
            installed = os.stat(path.name, dir_fd=directory, follow_symlinks=False)
            auth.require((installed.st_dev, installed.st_ino) == owned and installed.st_nlink == 1)
        return digest.hexdigest()
    finally:
        try:
            if temporary is not None and owned is not None:
                try:
                    item = os.stat(temporary, dir_fd=directory, follow_symlinks=False)
                    if (item.st_dev, item.st_ino) == owned:
                        os.unlink(temporary, dir_fd=directory)
                except FileNotFoundError:
                    pass
        finally:
            try:
                os.close(directory)
            finally:
                for kind, handler in handlers.items():
                    signal.signal(kind, handler)
        if cancelled is not None:
            raise SystemExit(128 + cancelled)


def child_environment():
    # Candidate builds and native runtime never receive API/Actions credentials.
    return {"PATH": os.environ["PATH"], "HOME": os.environ["HOME"], "LC_ALL": "C",
            **{key: os.environ[key] for key in ("CARGO_HOME", "RUSTUP_HOME") if key in os.environ},
            "PYTHONDONTWRITEBYTECODE": "1", **runtime_environment(os.environ)}


def execute(arguments, *, root: Path, log: Path, seconds: float, maximum=16 * 1024 * 1024,
            cleanup_seconds=10, preserve_guard=False):
    auth.require(seconds > 0)
    deadline = time.monotonic() + seconds
    failed = False
    cancelled = None
    cancellation_failure = False
    handlers = {}
    def cancel(kind, _frame):
        nonlocal cancelled
        # Never raise inside Popen or cleanup, or re-enter a native TERM handler.
        # Repeated signals cannot restart the first absolute cleanup reserve.
        if cancelled is None:
            cancelled = kind
    try:
        for kind in (signal.SIGTERM, signal.SIGINT):
            handlers[kind] = signal.signal(kind, cancel)
        with log.open("xb") as stream:
            log.chmod(0o600)
            if cancelled is not None:
                raise SystemExit(128 + cancelled)
            process = subprocess.Popen(arguments, cwd=root, env=child_environment(), stdin=subprocess.DEVNULL,
                                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT, start_new_session=True)
            assert process.stdout is not None
            def signal_owned(kind, *, leader_only=False):
                try:
                    if leader_only:
                        os.kill(process.pid, kind)
                    else:
                        os.killpg(process.pid, kind)
                except ProcessLookupError:
                    pass
            def exited():
                # Keep the leader waitable until group cleanup: its PID/PGID cannot
                # be reused and accidentally target an unrelated process group.
                return os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT) is not None
            def stop_work(*, cancellation=False):
                nonlocal failed, deadline, cancellation_failure
                if not failed:
                    failed = True
                    cancellation_failure = cancellation
                    deadline = time.monotonic() + cleanup_seconds
                    # Native TERM reaches its owner first, leaving its watchdog
                    # alive for the original cleanup reserve. Build/test trees
                    # receive TERM as a whole owned group immediately.
                    signal_owned(signal.SIGTERM, leader_only=preserve_guard)
            selector = None
            try:
                selector = selectors.DefaultSelector()
                selector.register(process.stdout, selectors.EVENT_READ)
                total = 0
                while selector.get_map() or not exited():
                    if cancelled is not None:
                        stop_work(cancellation=True)
                    if time.monotonic() >= deadline:
                        if failed:
                            break
                        stop_work()
                    for event, _ in selector.select(0.1):
                        raw = os.read(event.fileobj.fileno(), 65536)
                        if not raw:
                            selector.unregister(event.fileobj)
                            continue
                        total += len(raw)
                        if total > maximum:
                            stop_work()
                        if not failed:
                            stream.write(raw)
            finally:
                if cancelled is not None:
                    stop_work(cancellation=True)
                if not exited():
                    stop_work()
                    while not exited() and time.monotonic() < deadline:
                        time.sleep(0.01)
                # Includes descendants that outlive their leader or retain stdout.
                # Never reap the leader before this final owned-group signal.
                signal_owned(signal.SIGKILL)
                status = process.wait(timeout=10)
                if selector is not None:
                    selector.close()
                process.stdout.close()
    finally:
        for kind, handler in handlers.items():
            signal.signal(kind, handler)
    if cancellation_failure or (cancelled is not None and not failed and status == 0):
        raise SystemExit(128 + cancelled)
    auth.require(not failed and status == 0)
    return auth.read(log, maximum)


def exact_test(binary: Path, name: str, *, root: Path, private: Path, index: int, deadline: float):
    listed = execute([str(binary), "--list", "--exact", name], root=root, log=private / f"list-{index}.log",
                     seconds=min(30, deadline - time.monotonic()), maximum=1024 * 1024).decode()
    auth.require([line for line in listed.splitlines() if line.endswith(": test")] == [name + ": test"])
    output = execute([str(binary), "--exact", name, "--nocapture"], root=root,
                     log=private / f"test-{index}.log", seconds=min(120, deadline - time.monotonic()),
                     maximum=1024 * 1024).decode()
    summaries = re.findall(r"(?m)^test result: (ok|FAILED)\. ([0-9]+) passed; ([0-9]+) failed; "
                           r"([0-9]+) ignored; ([0-9]+) measured; [0-9]+ filtered out;", output)
    auth.require(summaries == [("ok", "1", "0", "0", "0")])
    return {"name": name, "selected": 1, "passed": 1, "failed": 0, "ignored": 0}


def inputs(args):
    auth.require(not os.environ.get("GITHUB_TOKEN") and not os.environ.get("GH_TOKEN"))
    pair, digest = auth.authorize(args.trusted_root, args.box_pr, args.box_sha, args.lens_pr, args.lens_sha)
    auth.require(auth.git(args.trusted_root, "rev-parse", "HEAD") == args.trusted_sha)
    for root in (args.trusted_root, args.box_root, args.lens_root, args.execution_root, args.private):
        auth.require(root.is_absolute() and re.fullmatch(r"[A-Za-z0-9_./-]+", str(root)) is not None)
    box_files = auth.verify_source(args.box_root, pair["boxferry"])
    lens_files = auth.verify_source(args.lens_root, pair["docker_lens"])
    return pair, digest, box_files, lens_files


def build(args, *, phase=None):
    def mark(name):
        if phase is not None:
            phase(name)

    mark("build-layout")
    transfer.layout(args, os.environ)
    mark("build-context")
    runtime_environment(os.environ, require_docker=True)
    mark("build-authorization")
    pair, digest, box_files, _ = inputs(args)
    # Full unconditional import closure and all repository-relative inputs are
    # copied before any candidate helper is imported or sourced.
    mark("build-copy")
    auth.copy_closure(box_files, args.execution_root)
    args.private.mkdir(mode=0o700)
    deadline = time.monotonic() + 1200
    builds = {}
    for profile, example in EXAMPLES.items():
        mark(profile + "-build")
        execute(proof.build_command(args.lens_root, profile), root=args.box_root,
                log=args.private / (profile + "-build.log"), seconds=deadline - time.monotonic())
        binary = args.box_root / "target/debug/examples" / example
        mark(profile + "-capture")
        captured = capture_compiler_executable(binary, args.box_root, profile, deadline=deadline)
        digest_binary = binary_digest(binary, args.box_root)
        auth.require(digest_binary == captured)
        mark(profile + "-receipt")
        write(args.private / (profile + "-receipt.json"), proof.build_receipt(pair, args.lens_root, profile, digest_binary))
        builds[profile] = proof.sanitized_build(pair, args.lens_root, profile, digest_binary)
    command = ["cargo", "test", "--locked", "--package", "boxferry", "--test", "docker_authored_bind_rehearsal",
               "--no-default-features", "--features", "compose,docker", "--jobs", "2",
               "--config", f'patch.crates-io.docker-lens.path="{args.lens_root}"', "--no-run", "--message-format=json"]
    mark("inert-build")
    messages = execute(command, root=args.box_root, log=args.private / "inert-build.log",
                       seconds=deadline - time.monotonic()).decode()
    mark("inert-discovery")
    executables = []
    for line in messages.splitlines():
        if line.startswith("{"):
            item = json.loads(line, object_pairs_hook=auth.no_duplicates)
            if (item.get("reason") == "compiler-artifact" and item.get("target", {}).get("name") == "docker_authored_bind_rehearsal"
                    and item.get("profile", {}).get("test") is True and item.get("executable") is not None):
                executables.append(Path(item["executable"]))
    auth.require(len(executables) == 1)
    binary = executables[0]
    mark("inert-capture")
    captured = capture_compiler_executable(binary, args.box_root, "inert-tests", deadline=deadline)
    digest_binary = binary_digest(binary, args.box_root)
    auth.require(digest_binary == captured)
    mark("inert-tests")
    inert = {"kind": "inert-only-no-runtime", "binary_sha256": digest_binary,
             "tests": [exact_test(binary, name, root=args.box_root, private=args.private, index=index, deadline=deadline)
                       for index, name in enumerate(auth.INERT_TESTS)]}
    mark("build-source-recheck")
    auth.verify_source(args.box_root, pair["boxferry"])
    auth.verify_source(args.lens_root, pair["docker_lens"])
    state = {
        "context": proof.context(pair, digest, args.trusted_sha, args.run_id, args.attempt),
        "builds": builds, "inert": inert, "test_binary": str(binary)}
    mark("build-state")
    write(args.private / "build-state.json", state)
    mark("build-export")
    transfer.export(args, pair=pair, context=state["context"], state=state)


def run(args):
    auth.require(os.geteuid() == 0 and args.lane in auth.LANES)
    environment = runtime_environment(os.environ, require_docker=True)
    auth.require(environment["GITHUB_RUN_ID"] == str(args.run_id)
                 and environment["GITHUB_RUN_ATTEMPT"] == str(args.attempt)
                 and environment["DOCKERLENS_NATIVE_CANDIDATE_SHA"] == args.lens_sha)
    pair, digest, box_files, lens_files = inputs(args)
    expected = proof.context(pair, digest, args.trusted_sha, args.run_id, args.attempt)
    binding = transfer.restore(args, pair=pair, context=expected, box_files=box_files)
    state = auth.document(args.private / "build-state.json", 128 * 1024)
    auth.keys(state, ("context", "builds", "inert", "test_binary"))
    auth.require(state["context"] == expected)
    auth.keys(state["builds"], EXAMPLES)
    proof.validate_inert(state["inert"])
    auth.require(binary_digest(Path(state["test_binary"]), args.box_root) == state["inert"]["binary_sha256"])
    for profile, example in EXAMPLES.items():
        binary = args.box_root / "target/debug/examples" / example
        digest_binary = binary_digest(binary, args.box_root)
        auth.require(state["builds"][profile] == proof.sanitized_build(pair, args.lens_root, profile, digest_binary))
        auth.require(auth.read(args.private / (profile + "-receipt.json")) ==
                     proof.encoded(proof.build_receipt(pair, args.lens_root, profile, digest_binary)))
    # Recheck copied executable/import bytes immediately before sourcing them.
    for path, raw in box_files.items():
        auth.require(auth.read(args.execution_root / path) == raw)
    native = proof.native_catalogue(lens_files)
    directory = args.private / "runtime-evidence"
    directory.mkdir(mode=0o700)
    command = [
        "bash", str(args.execution_root / "scripts/docker-application-conformance.sh"),
        "--profile", "volume-fixtures", "--lane", args.lane,
        "--docker-lens-root", str(args.lens_root), "--docker-lens-revision", pair["docker_lens"]["revision"],
        "--native-script-sha256", pair["docker_lens"]["files"]["scripts/native-conformance.sh"],
        "--unix-http-helper", str(args.lens_root / "scripts/native-unix-http.py"),
        "--unix-http-sha256", pair["docker_lens"]["files"]["scripts/native-unix-http.py"],
        "--outer-runtime-closure-json", proof.encoded(proof.runtime_source_binding(pair)["files"]).decode().strip(),
        "--api-version", native["profiles"][args.lane]["rendering_api_version"],
        "--boxferry-root", str(args.box_root),
        "--boxferry-binary", str(args.box_root / "target/debug/examples/docker-volume-fixture-rehearsal"),
        "--boxferry-receipt", str(args.private / "volume-fixtures-receipt.json"),
        "--host-metadata-binary", str(args.box_root / "target/debug/examples/docker-host-metadata"),
        "--host-metadata-receipt", str(args.private / "host-metadata-receipt.json"),
        "--evidence-directory", str(directory),
    ]
    # The existing owned watchdog remains authoritative (780s + 120s cleanup).
    # This backstop does not change its inner budgets or nesting/host policy.
    transfer.binding(binding, pair=pair, context=expected, lane=args.lane, lens=args.lens_root,
                     identifier=args.build_artifact_id, fresh=True)
    execute(command, root=args.execution_root, log=args.private / "runtime.log", seconds=1020,
            cleanup_seconds=125, preserve_guard=True)
    auth.require(set(path.name for path in directory.iterdir()) == {"volume-proof.json"})
    runtime_log = auth.read(args.private / "runtime.log", 16 * 1024 * 1024)
    outer_runtime = proof.outer_runtime_from_log(runtime_log, args.lane)
    prerequisite = proof.runner_prerequisite_from_log(runtime_log, args.lane, expected)
    volume = auth.document(directory / "volume-proof.json", 16 * 1024)
    auth.verify_source(args.box_root, pair["boxferry"])
    auth.verify_source(args.lens_root, pair["docker_lens"])
    for path, raw in box_files.items():
        auth.require(auth.read(args.execution_root / path) == raw)
    consumer = {"schema": 2, "scope": auth.POLICY["scope"], "lane": args.lane, "result": "checks-passed",
                "context": expected, "builds": state["builds"], "inert": state["inert"],
                "source_binding": proof.runtime_source_binding(pair), "outer_runtime": outer_runtime,
                "runner_prerequisite": prerequisite,
                "volume_proof_sha256": hashlib.sha256(proof.encoded(volume)).hexdigest()}
    proof.validate_lane(consumer, volume, expected_context=expected, lane=args.lane, pair=pair,
                        lens=args.lens_root, files=lens_files, box_files=box_files)
    args.output.mkdir(mode=0o700)
    write(args.output / "consumer.json", consumer)
    write(args.output / "volume-proof.json", volume)


class ClosedArgumentParser(argparse.ArgumentParser):
    def error(self, message):
        raise ValueError("rejected") from None


def arguments():
    parser = ClosedArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("build", "run"))
    for name in ("trusted-root", "box-root", "lens-root", "execution-root", "private"):
        parser.add_argument("--" + name, type=Path, required=True)
    for name in ("box-pr", "lens-pr", "run-id", "attempt"):
        parser.add_argument("--" + name, type=int, required=True)
    for name in ("box-sha", "lens-sha", "trusted-sha"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--lane", choices=auth.LANES)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--bundle-root", type=Path, required=True)
    parser.add_argument("--build-artifact-id")
    return parser.parse_args()


def failure_message(phase):
    if type(phase) is not str or phase not in FAILURE_PHASES:
        phase = "arguments"
    return f"reviewed Docker consumer execution rejected: phase={phase} reason=rejected; no passing evidence"


def main():
    phase = "arguments"

    def mark(name):
        nonlocal phase
        phase = name

    try:
        args = arguments()
        if args.command == "build":
            auth.require(args.lane in auth.LANES and args.output is None and args.build_artifact_id is None)
            build(args, phase=mark)
        else:
            auth.require(args.lane is not None and args.output is not None and args.build_artifact_id is not None)
            mark("native-runtime")
            run(args)
    except Exception:
        print(failure_message(phase), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
