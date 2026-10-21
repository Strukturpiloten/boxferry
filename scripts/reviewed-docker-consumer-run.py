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
import selectors
import signal
import stat
import subprocess
import time


SPEC = importlib.util.spec_from_file_location(
    "reviewed_docker_evidence", Path(__file__).with_name("reviewed-docker-consumer-evidence.py"))
assert SPEC is not None and SPEC.loader is not None
proof = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(proof)
auth = proof.auth
EXAMPLES = {"volume-fixtures": "docker-volume-fixture-rehearsal", "host-metadata": "docker-host-metadata"}
RUNTIME_FACTS = {
    "GITHUB_ACTIONS": "true", "RUNNER_ENVIRONMENT": "github-hosted", "RUNNER_OS": "Linux",
    "GITHUB_REF": "refs/heads/main", "GITHUB_EVENT_NAME": "workflow_dispatch",
    "GITHUB_REPOSITORY": "Strukturpiloten/boxferry",
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


def build(args):
    pair, digest, box_files, _ = inputs(args)
    # Full unconditional import closure and all repository-relative inputs are
    # copied before any candidate helper is imported or sourced.
    auth.copy_closure(box_files, args.execution_root)
    args.private.mkdir(mode=0o700)
    deadline = time.monotonic() + 1200
    builds = {}
    for profile, example in EXAMPLES.items():
        execute(proof.build_command(args.lens_root, profile), root=args.box_root,
                log=args.private / (profile + "-build.log"), seconds=deadline - time.monotonic())
        binary = args.box_root / "target/debug/examples" / example
        digest_binary = binary_digest(binary, args.box_root)
        write(args.private / (profile + "-receipt.json"), proof.build_receipt(pair, args.lens_root, profile, digest_binary))
        builds[profile] = proof.sanitized_build(pair, args.lens_root, profile, digest_binary)
    command = ["cargo", "test", "--locked", "--package", "boxferry", "--test", "docker_authored_bind_rehearsal",
               "--no-default-features", "--features", "compose,docker", "--jobs", "2",
               "--config", f'patch.crates-io.docker-lens.path="{args.lens_root}"', "--no-run", "--message-format=json"]
    messages = execute(command, root=args.box_root, log=args.private / "inert-build.log",
                       seconds=deadline - time.monotonic()).decode()
    executables = []
    for line in messages.splitlines():
        if line.startswith("{"):
            item = json.loads(line, object_pairs_hook=auth.no_duplicates)
            if (item.get("reason") == "compiler-artifact" and item.get("target", {}).get("name") == "docker_authored_bind_rehearsal"
                    and item.get("profile", {}).get("test") is True and item.get("executable") is not None):
                executables.append(Path(item["executable"]))
    auth.require(len(executables) == 1)
    binary = executables[0]
    inert = {"kind": "inert-only-no-runtime", "binary_sha256": binary_digest(binary, args.box_root),
             "tests": [exact_test(binary, name, root=args.box_root, private=args.private, index=index, deadline=deadline)
                       for index, name in enumerate(auth.INERT_TESTS)]}
    auth.verify_source(args.box_root, pair["boxferry"])
    auth.verify_source(args.lens_root, pair["docker_lens"])
    write(args.private / "build-state.json", {
        "context": proof.context(pair, digest, args.trusted_sha, args.run_id, args.attempt),
        "builds": builds, "inert": inert, "test_binary": str(binary)})


def run(args):
    auth.require(os.geteuid() == 0 and args.lane in auth.LANES)
    runtime_environment(os.environ, require_docker=True)
    pair, digest, box_files, lens_files = inputs(args)
    expected = proof.context(pair, digest, args.trusted_sha, args.run_id, args.attempt)
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
    execute(command, root=args.execution_root, log=args.private / "runtime.log", seconds=1020,
            cleanup_seconds=125, preserve_guard=True)
    auth.require(set(path.name for path in directory.iterdir()) == {"volume-proof.json"})
    outer_runtime = proof.outer_runtime_from_log(auth.read(args.private / "runtime.log", 16 * 1024 * 1024), args.lane)
    volume = auth.document(directory / "volume-proof.json", 16 * 1024)
    auth.verify_source(args.box_root, pair["boxferry"])
    auth.verify_source(args.lens_root, pair["docker_lens"])
    for path, raw in box_files.items():
        auth.require(auth.read(args.execution_root / path) == raw)
    consumer = {"schema": 2, "scope": auth.POLICY["scope"], "lane": args.lane, "result": "checks-passed",
                "context": expected, "builds": state["builds"], "inert": state["inert"],
                "source_binding": proof.runtime_source_binding(pair), "outer_runtime": outer_runtime,
                "volume_proof_sha256": hashlib.sha256(proof.encoded(volume)).hexdigest()}
    proof.validate_lane(consumer, volume, expected_context=expected, lane=args.lane, pair=pair,
                        lens=args.lens_root, files=lens_files, box_files=box_files)
    args.output.mkdir(mode=0o700)
    write(args.output / "consumer.json", consumer)
    write(args.output / "volume-proof.json", volume)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("build", "run"))
    for name in ("trusted-root", "box-root", "lens-root", "execution-root", "private"):
        parser.add_argument("--" + name, type=Path, required=True)
    for name in ("box-pr", "lens-pr", "run-id", "attempt"):
        parser.add_argument("--" + name, type=int, required=True)
    for name in ("box-sha", "lens-sha", "trusted-sha"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--lane", choices=auth.LANES)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.command == "build":
        auth.require(args.lane is None and args.output is None)
        build(args)
    else:
        auth.require(args.output is not None)
        run(args)


if __name__ == "__main__":
    try:
        main()
    except Exception:
        raise SystemExit("reviewed Docker consumer execution rejected; no passing evidence") from None
