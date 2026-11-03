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
import socket
import stat
import struct
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
    "mechanism-runtime", "mechanism-context", "mechanism-source", "mechanism-copy", "mechanism-binding",
    "mechanism-import", "mechanism-observer", "mechanism-execution", "mechanism-evidence",
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


def restore_mechanism_receipt(uid, gid):
    """Hand off only the fixed producer receipt; never qualify its result.

    All reads and mutations bind held descriptors to no-follow directory
    entries. The root-owned directory is transferred last. An observed race or
    partial handoff refuses upload; there is no retry or recursive repair.
    """
    try:
        from contextlib import ExitStack

        auth.require(os.getuid() == os.geteuid() == os.getgid() == os.getegid() == 0
                     and type(uid) is int and 0 < uid < 2**32 - 1
                     and type(gid) is int and 0 < gid < 2**32 - 1)
        identity = lambda item: (item.st_dev, item.st_ino, item.st_mode, item.st_uid, item.st_gid,
                                 item.st_nlink, item.st_size, item.st_mtime_ns, item.st_ctime_ns)
        with ExitStack() as stack:
            def opened(name, parent=None, *, directory=True):
                flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_NOATIME
                if directory:
                    flags |= os.O_DIRECTORY
                descriptor = os.open(name, flags, dir_fd=parent)
                stack.callback(os.close, descriptor)
                return descriptor

            root = opened("/")
            bindings = []
            parent = root
            for name in (*transfer.WORKSPACE.parts[1:], "staged", "custody"):
                descriptor = opened(name, parent)
                item = os.fstat(descriptor)
                auth.require(stat.S_ISDIR(item.st_mode))
                bindings.append([parent, name, descriptor, item])
                parent = descriptor
            staged, publication = bindings[-2][3], bindings[-1][3]
            auth.require((staged.st_uid, staged.st_gid, stat.S_IMODE(staged.st_mode)) == (uid, gid, 0o700)
                         and (publication.st_uid, publication.st_gid, stat.S_IMODE(publication.st_mode)) == (0, 0, 0o700))
            receipt = opened("mechanism.json", parent, directory=False)
            item = os.fstat(receipt)
            auth.require(stat.S_ISREG(item.st_mode) and stat.S_IMODE(item.st_mode) == 0o600
                         and item.st_uid == item.st_gid == 0 and item.st_nlink == 1
                         and 0 < item.st_size <= transfer.MAX_JSON)
            bindings.append([parent, "mechanism.json", receipt, item])

            def recheck():
                for directory, name, descriptor, expected in bindings:
                    auth.require(identity(os.fstat(descriptor)) == identity(expected)
                                 and identity(os.stat(name, dir_fd=directory, follow_symlinks=False)) == identity(expected))
                auth.require(os.listdir(parent) == ["mechanism.json"])

            recheck()
            raw = os.read(receipt, transfer.MAX_JSON + 1)
            auth.require(len(raw) == item.st_size and not os.read(receipt, 1))
            value = json.loads(raw, object_pairs_hook=auth.no_duplicates,
                               parse_constant=lambda _value: auth.require(False))
            auth.require(raw == proof.encoded(value))
            result = value.get("result") if type(value) is dict else None
            auth.require(result in {"checks-passed", "refused"})
            auth.keys(value, ("schema", "scope", "result", "case", "context", "sdk", "applications_accepted",
                              "full_driver", "target_recreation", "native_qualified",
                              "observations" if result == "checks-passed" else "residuals"))
            auth.require(type(value["schema"]) is int and value["schema"] == 1
                         and value["scope"] == "paperless-custody-mechanism" and value["case"] in auth.CUSTODY_CASES
                         and type(value["context"]) is dict and type(value["sdk"]) is dict
                         and type(value["applications_accepted"]) is int and value["applications_accepted"] == 0
                         and all(value[key] is False for key in ("full_driver", "target_recreation", "native_qualified")))
            if result == "checks-passed":
                auth.require(type(value["observations"]) is dict)
            else:
                auth.keys(value["residuals"], ("verification", "last_verified_phase", "owner_pid", "owner_start",
                    "guard_pid", "guard_start", "core_id", "core_name", "storage_name", "private_directory",
                    "expiry_observation", "expiry_observation_pending", "worker_teardown_authority"))
                auth.require(value["residuals"]["verification"] == "incomplete"
                             and value["residuals"]["worker_teardown_authority"] is False)
            # The trusted producer validates successful evidence before writing;
            # this envelope check confers no evidence/cleanup authority.
            recheck()
            for binding in (bindings[-1], bindings[-2]):
                descriptor, before = binding[2], binding[3]
                os.fchown(descriptor, uid, gid)
                after = os.fstat(descriptor)
                auth.require((after.st_uid, after.st_gid) == (uid, gid)
                             and (after.st_dev, after.st_ino, after.st_mode, after.st_nlink, after.st_size, after.st_mtime_ns)
                             == (before.st_dev, before.st_ino, before.st_mode, before.st_nlink, before.st_size, before.st_mtime_ns))
                binding[3] = after
                recheck()
        return 0
    except Exception:
        print("reviewed mechanism receipt handoff rejected; no passing evidence", file=sys.stderr)
        return 1


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


class MechanismFailureDiagnostic:
    """Bounded metadata from captured bytes, never an execution authority."""
    STAGES = (
        "prepared-ack", "outer-pull", "outer-volume-create", "outer-volume-path", "outer-data-prepare",
        "apparmor-prepare", "apparmor-load", "outer-daemon-create", "outer-daemon-identity",
        "daemon-socket-readiness", "legacy-ready", "http-helper-bind", "native-peer-bind",
        "native-version", "native-info", "inner-volume-data", "inner-volume-media",
        "inner-volume-consume", "inner-volume-export", "inner-volume-pgdata", "inner-volume-redisdata",
        "paperless-hold", "paperless-check", "held-publication",
    )
    PREFIX = b"BOXFERRY_CUSTODY_FAILURE"
    MARKERS = {b"BOXFERRY_CUSTODY_FAILURE last_entered_stage=" + stage.encode("ascii") + b" reason=rejected": stage
               for stage in STAGES}
    MAX_LINE = max(map(len, MARKERS))

    def __init__(self):
        self.pending = bytearray()
        self.discard = False
        self.invalid = False
        self.stage = None

    def feed(self, raw):
        for value in raw:
            if value == 10:
                if not self.discard and self.PREFIX in self.pending:
                    stage = self.MARKERS.get(bytes(self.pending))
                    if stage is None or self.stage is not None:
                        self.invalid = True
                    else:
                        self.stage = stage
                self.pending.clear()
                self.discard = False
            elif not self.discard:
                if len(self.pending) == self.MAX_LINE:
                    if self.PREFIX in self.pending:
                        self.invalid = True
                    self.pending.clear()
                    self.discard = True
                else:
                    self.pending.append(value)

    def last_entered_stage(self):
        if self.invalid or self.pending or self.discard:
            return None
        return self.stage

    def emit(self):
        try:
            stage = self.last_entered_stage()
            if type(stage) is str and stage in self.STAGES:
                print(f"mechanism launcher rejected: last_entered_stage={stage} reason=rejected", file=sys.stderr)
        except Exception:
            pass


def execute(arguments, *, root: Path, log: Path, seconds: float, maximum=16 * 1024 * 1024,
            cleanup_seconds=10, preserve_guard=False, observe=None, accepted_statuses=(0,), mechanism_diagnostics=False):
    auth.require(seconds > 0)
    deadline = time.monotonic() + seconds
    failed = False
    cancelled = None
    cancellation_failure = False
    handlers = {}
    status = None
    diagnostic = None
    diagnostic_complete = False
    if mechanism_diagnostics:
        try:
            diagnostic = MechanismFailureDiagnostic()
        except Exception:
            pass
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
                    if observe is not None:
                        observe(process)
                    if cancelled is not None:
                        stop_work(cancellation=True)
                    if time.monotonic() >= deadline:
                        if failed:
                            break
                        stop_work()
                    for event, _ in selector.select(0.1):
                        raw = os.read(event.fileobj.fileno(), 65536)
                        if not raw:
                            diagnostic_complete = True
                            selector.unregister(event.fileobj)
                            continue
                        total += len(raw)
                        if total > maximum:
                            stop_work()
                        if not failed:
                            stream.write(raw)
                            if diagnostic is not None:
                                try:
                                    diagnostic.feed(raw)
                                except Exception:
                                    diagnostic = None
                        else:
                            diagnostic = None
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
        if (diagnostic is not None and diagnostic_complete and status is not None and status not in accepted_statuses
                and cancelled is None and not cancellation_failure):
            try:
                diagnostic.emit()
            except Exception:
                pass
    if cancellation_failure or (cancelled is not None and not failed and status == 0):
        raise SystemExit(128 + cancelled)
    auth.require(not failed and status in accepted_statuses)
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


class CustodyObserver:
    """Existing worker's read-only proof; never a holder or recovery authority."""

    def __init__(self, module, private, lens, context, case):
        self.module, self.c, self.private, self.lens = module, module.contract(), private, lens
        self.context, self.case = context, case
        self.owner = self.guard = self.cid = None
        self.owner_fd = self.guard_fd = self.outer_fd = None
        self.phase = self.calls = self.bytes = self.ack_calls = self.ack_bytes = 0
        self.known_calls = self.known_bytes = 0
        self.roles = None
        self.closed_before_removal = False
        self.observation = private / "observation"
        self.observation.mkdir(mode=0o700)

    def alive(self, fd):
        return fd is not None and self.c.parent_alive(fd)

    def uncertainty(self):
        replacement = self.case.startswith("replace-") and not getattr(self, "replacement_observed", False)
        check = self.case.startswith("replace-") or self.case == "lost-ack"
        return (2 if replacement else 0) + (3 if check else 0), (2 * self.module.LIMIT if replacement else 0) + (
            self.module.CHECK_BYTES if check else 0)

    def charge(self, calls, size):
        self.calls += calls
        self.bytes += size
        # A failed post-hold check can consume its entire original reservation;
        # uncertainty never gives the observer a second allowance.
        uncertain_calls, uncertain_bytes = self.uncertainty()
        auth.require(self.known_calls + self.calls - self.ack_calls + uncertain_calls <= 256
                     and self.known_bytes + self.bytes - self.ack_bytes + uncertain_bytes <= 16 * self.module.LIMIT)

    def reserve(self, calls, size):
        uncertain_calls, uncertain_bytes = self.uncertainty()
        auth.require(self.known_calls + self.calls - self.ack_calls + uncertain_calls + calls <= 256
                     and self.known_bytes + self.bytes - self.ack_bytes + uncertain_bytes + size <= 16 * self.module.LIMIT)

    def read_phase(self, path):
        raw = self.module.read_private(path)
        self.charge(0, len(raw))
        return json.loads(raw, object_pairs_hook=auth.no_duplicates)

    def command(self, arguments):
        """Only canonical preflight or exact previously retained presence reads."""
        if arguments != ["preflight"]:
            auth.require(self.owner is not None and len(arguments) == 3 and arguments[0] == "presence"
                         and (arguments[1], arguments[2]) in {
                             ("container", self.cid), ("container", self.owner["outer_name"]),
                             ("volume", self.owner["volume_name"])})
            auth.require(self.alive(self.outer_fd))
            current = os.stat("/var/run/docker.sock")
            auth.require((current.st_dev, current.st_ino) == self.outer_socket)
        deadline = self.owner["cleanup_deadline"]
        auth.require(self.c.owner_clock() + .1 < deadline)
        maximum = self.module.LIMIT if arguments == ["preflight"] else 16
        self.reserve(1, maximum + 1)
        self.charge(1, 0)
        process = subprocess.Popen(["bash", str(self.lens / "scripts/native-outer-command.sh"),
            "--engine", "docker", "--directory", str(self.observation), *arguments],
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            env=child_environment(), start_new_session=True)
        raw = bytearray()
        try:
            os.set_blocking(process.stdout.fileno(), False)
            with selectors.DefaultSelector() as poll:
                poll.register(process.stdout, selectors.EVENT_READ)
                eof = False
                while not eof:
                    auth.require(self.c.owner_clock() < deadline)
                    for key, _event in poll.select(min(.1, max(.001, deadline - self.c.owner_clock()))):
                        block = os.read(key.fd, min(65536, maximum + 1 - len(raw)))
                        self.charge(0, len(block))
                        if not block:
                            eof = True
                            break
                        raw.extend(block)
                        auth.require(len(raw) <= maximum)
            status = process.wait(timeout=max(.001, deadline - self.c.owner_clock()))
            auth.require(status in (0, 1) and (arguments == ["preflight"] or not raw))
            return status, bytes(raw)
        finally:
            self.c.owner_command_quiescence(process, min(deadline, self.c.owner_clock() + 2))
            process.wait(timeout=2)
            process.stdout.close()

    def object_identity(self, role):
        auth.require(role in self.module.ROLES and self.owner is not None and self.cid is not None)
        peer = self.inner_peer
        pidfd = self.c.verified_parent_fd(peer["pid"], peer["start"])
        rootfd = graphfd = rolefd = None
        try:
            auth.require(self.alive(pidfd))
            # Same fixed intentional daemon proc-root transition as the holder;
            # no setns, guest command, provider or retained directory capability.
            rootfd = os.open(f"/proc/{peer['pid']}/root", self.fd_code.OPATH)
            root = os.fstat(rootfd)
            auth.require([root.st_dev, root.st_ino] == self.held_view["root"])
            graphfd = self.fd_code.directory(rootfd, self.fd_code.GRAPH)
            graph = os.fstat(graphfd)
            auth.require([graph.st_dev, graph.st_ino] == self.held_view["graph"])
            rolefd = self.fd_code.directory(graphfd, "volumes/" + self.owner["prefix"] + "-paper-" + role + "/_data")
            current = os.fstat(rolefd)
            auth.require(self.alive(pidfd))
            for name, key in (("user", "user_namespace"), ("mnt", "mount_namespace")):
                namespace = os.stat(f"/proc/{peer['pid']}/ns/{name}")
                auth.require([namespace.st_dev, namespace.st_ino] == self.held_view[key])
            return [current.st_dev, current.st_ino]
        finally:
            for fd in (rolefd, graphfd, rootfd, pidfd):
                if fd is not None:
                    os.close(fd)

    def inventory(self, *, strict=False):
        auth.require(self.guard is not None)
        if not self.alive(self.guard_fd):
            return {}
        directory = Path(f"/proc/{self.guard['pid']}/fd")
        self.reserve(0, 128 * 4096)
        result = {}
        objects = set()
        with os.scandir(directory) as entries:
            for index, entry in enumerate(entries):
                auth.require(index < 128 and entry.name.isdigit())
                try:
                    target = os.readlink(directory / entry.name)
                except FileNotFoundError:
                    continue
                self.charge(0, len(target.encode()))
                auth.require(len(target.encode()) <= 4096)
                try:
                    metadata = os.stat(directory / entry.name)
                except FileNotFoundError:
                    continue
                objects.add((metadata.st_dev, metadata.st_ino, stat.S_IFMT(metadata.st_mode)))
                for role in self.module.ROLES:
                    suffix = "/volumes/" + self.owner["prefix"] + "-paper-" + role + "/_data"
                    if target.removesuffix(" (deleted)") == "/home/docker/.local/share/docker" + suffix:
                        auth.require(stat.S_ISDIR(metadata.st_mode) and (not strict or role not in result))
                        result.setdefault(role, [metadata.st_dev, metadata.st_ino])
        self.last_fd_objects = objects
        return result

    def view(self, peer):
        auth.keys(peer, ("pid", "start", "uid", "gid"))
        for value in peer.values():
            auth.positive(value)
        socket_path = Path(self.owner["socket_directory"]) / "docker.sock"
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as stream:
            stream.settimeout(min(.2, max(.001, self.owner["execution_deadline"] - self.c.owner_clock())))
            stream.connect(str(socket_path))
            auth.require(struct.unpack("3i", stream.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
                         == (peer["pid"], peer["uid"], peer["gid"]))
        auth.require(self.c.process_start(peer["pid"]) == peer["start"])
        paths = {"root": f"/proc/{peer['pid']}/root", "user_namespace": f"/proc/{peer['pid']}/ns/user",
                 "mount_namespace": f"/proc/{peer['pid']}/ns/mnt",
                 "graph": f"/proc/{peer['pid']}/root/home/docker/.local/share/docker",
                 "outer_graph": self.owner["volume_path"]}
        result = {}
        for key, path in paths.items():
            info = os.stat(path)
            auth.require((info.st_dev, info.st_ino, stat.S_IFMT(info.st_mode)) in self.last_fd_objects)
            result[key] = [info.st_dev, info.st_ino]
        auth.require(result["graph"] == result["outer_graph"])
        return result

    def tick(self, process):
        if self.owner_fd is None:
            self.owner_start = self.c.process_start(process.pid)
            self.owner_fd = self.c.verified_parent_fd(process.pid, self.owner_start)
        path = self.private / f"phase-{self.phase}.json"
        if path.exists():
            row = self.read_phase(path)
            auth.keys(row, ("stage", "owner", "custody_sha256", "guard", "cid", "case", "calls",
                            "read_bytes", "callback_pids", "callbacks", "inner_peer"))
            auth.require(row["case"] == self.case and row["owner"]["parent_pid"] == process.pid
                         and row["owner"]["parent_start"] == self.owner_start and self.alive(self.owner_fd))
            owner_path = Path(row["owner"]["directory"]) / "owner-custody.json"
            owner = self.c.owner_read(owner_path, row["custody_sha256"])
            auth.require(owner == row["owner"] and owner["guard_source_sha256"] ==
                         self.context["box"]["files"]["scripts/lib/docker-application-contract.py"])
            auth.require(type(row["calls"]) is int and 0 <= row["calls"] <= 256
                         and type(row["read_bytes"]) is int and 0 <= row["read_bytes"] <= 16 * self.module.LIMIT)
            auth.require(row["calls"] >= self.known_calls and row["read_bytes"] >= self.known_bytes)
            self.known_calls, self.known_bytes = row["calls"], row["read_bytes"]
            if self.owner is None:
                auth.require(self.phase == 0 and row["stage"] == "prepared" and row["cid"] is None)
                marker = self.read_phase(owner_path.parent / "owner-ready.json")
                auth.require(marker == row["guard"] and marker["custody_sha256"] == row["custody_sha256"])
                self.guard_fd = self.c.verified_parent_fd(marker["pid"], marker["start"])
                auth.require(self.alive(self.guard_fd) and os.getsid(marker["pid"]) == marker["pid"]
                             and os.getsid(process.pid) == process.pid)
                auth.require(Path(f"/proc/{marker['pid']}/cmdline").read_bytes().split(b"\0")[:-1]
                             == [os.fsencode(arg) for arg in self.c.owner_guard_command(owner, owner_path, row["custody_sha256"])])
                self.owner, self.guard = owner, marker
                self.custody_sha = row["custody_sha256"]
                self.fd_code = self.c.owner_paperless_module(owner)
                original = self.read_phase(owner_path.parent / "outer-runtime.json")
                status, _raw = self.command(["preflight"])
                auth.require(status == 0)
                fresh = self.read_phase(self.observation / "outer-runtime.json")
                auth.require(original["peer"] == fresh["peer"] and original["storage_root"] == fresh["storage_root"] == owner["graph_root"])
                peer = fresh["peer"]
                self.outer_fd = self.c.verified_parent_fd(peer["daemon_pid"], peer["daemon_starttime"])
                self.outer_socket = (peer["socket_device"], peer["socket_inode"])
                auth.require(self.alive(self.outer_fd))
            else:
                auth.require(owner == self.owner and row["guard"] == self.guard)
                if row["stage"] == "held":
                    auth.require(self.phase == 1 and self.alive(self.guard_fd))
                    self.cid = self.c.owner_cid(owner)
                    auth.require(self.cid is not None and row["cid"] == self.cid)
                    auth.require(self.command(["presence", "container", self.cid])[0] == 0)
                    self.roles = self.inventory(strict=True)
                    auth.require(set(self.roles) == set(self.module.ROLES)
                                 and len({tuple(value) for value in self.roles.values()}) == 6
                                 and len(set(row["callback_pids"])) == 2)
                    self.inner_peer = row["inner_peer"]
                    self.held_view = self.view(self.inner_peer)
                    auth.require(all(self.object_identity(role) == self.roles[role] for role in self.module.ROLES))
                    lease = self.read_phase(owner_path.parent / "paperless-transport/binding.json")
                    auth.require(lease["owner"] == owner and lease["arguments"]["custody_sha256"] == row["custody_sha256"]
                                 and type(lease["cutoff"]) in (int, float)
                                 and self.c.owner_clock() < lease["cutoff"] <= owner["execution_deadline"])
                    self.lease_cutoff = lease["cutoff"]
                    self.lease_binding_hash = hashlib.sha256(self.module.encoded(lease)).hexdigest()
                    if self.case == "expiry":
                        self.reserve(0, 128 * 1024 + 8192)
                        manifest_path = Path(lease["arguments"]["manifest_path"])
                        auth.require(manifest_path.parent.parent == owner_path.parent and manifest_path.name == "manifest.json")
                        manifest_raw = self.module.read_private(manifest_path)
                        self.charge(0, len(manifest_raw))
                        auth.require(hashlib.sha256(manifest_raw).hexdigest() == lease["arguments"]["manifest_sha256"])
                        self.lease_wall_cutoff = json.loads(manifest_raw, object_pairs_hook=auth.no_duplicates)["config"]["work_deadline_ms"]
                        auth.positive(self.lease_wall_cutoff)
                        auth.require(not os.path.lexists(self.c.owner_expiry_observation_path(owner)))
                        auth.require(not os.path.lexists(str(self.c.owner_expiry_observation_path(owner)) + ".pending"))
                    self.callbacks = row["callbacks"]
                elif row["stage"] == "replaced":
                    auth.require(self.case.startswith("replace-") and self.phase == 2
                                 and self.roles is not None and self.alive(self.guard_fd))
                    role = self.case.removeprefix("replace-")
                    auth.require(self.inventory(strict=True) == self.roles
                                 and self.object_identity(role) != self.roles[role])
                    self.replacement_observed = True
                else:
                    auth.require(row["stage"] == "closed" and self.phase == 2 and self.roles is not None)
                    auth.require(not self.inventory() and self.command(["presence", "container", self.cid])[0] == 0)
                    self.closed_before_removal = True
                    self.callbacks = row["callbacks"]
            self.module.write_new(self.private / f"ack-{self.phase}.json", {"stage": row["stage"],
                "calls": self.calls - self.ack_calls, "read_bytes": self.bytes - self.ack_bytes})
            self.ack_calls, self.ack_bytes = self.calls, self.bytes
            self.phase += 1
        elif self.roles is not None and not self.closed_before_removal and self.case not in {"positive", "guard-loss"}:
            if not self.inventory() and self.command(["presence", "container", self.cid])[0] == 0:
                self.closed_before_removal = True

    def expiry_observation(self):
        # Only the previously authenticated original guard can produce this
        # fixed private observation. A late verifier clock is never evidence.
        self.reserve(0, 8192)
        path = self.c.owner_expiry_observation_path(self.owner)
        before = path.lstat()
        raw = self.module.read_private(path, maximum=4096)
        self.charge(0, 2 * len(raw))  # Original guard write plus this verifier read.
        decision = json.loads(raw, object_pairs_hook=auth.no_duplicates)
        proof.validate_expiry_decision(decision, owner_pid=self.owner["parent_pid"],
            owner_start=self.owner["parent_start"], guard_pid=self.guard["pid"], guard_start=self.guard["start"],
            guard_source=self.owner["guard_source_sha256"])
        for key, expected in (("custody_sha256", self.custody_sha), ("lease_binding_sha256", self.lease_binding_hash),
                ("lease_cutoff", self.lease_cutoff), ("lease_wall_cutoff", self.lease_wall_cutoff),
                ("execution_deadline", self.owner["execution_deadline"]), ("cleanup_deadline", self.owner["cleanup_deadline"])):
            auth.require(decision[key] == expected)
        auth.require(before == path.lstat())
        self.expiry_decision = decision  # Retain bounded validated data before disposal.
        # Remove only this exact observation data, never a runtime resource.
        auth.require(before == path.lstat())
        path.unlink()
        auth.require(not os.path.lexists(path))
        return decision

    def finish(self):
        auth.require(self.owner is not None and self.cid is not None and self.roles is not None)
        while self.alive(self.guard_fd):
            auth.require(self.c.owner_clock() < self.owner["cleanup_deadline"])
            if not self.closed_before_removal and not self.inventory():
                self.closed_before_removal = self.command(["presence", "container", self.cid])[0] == 0
            time.sleep(.01)
        auth.require(not self.alive(self.owner_fd) and self.closed_before_removal and not self.inventory())
        auth.require(not self.case.startswith("replace-") or getattr(self, "replacement_observed", False))
        expiry_decision = self.expiry_observation() if self.case == "expiry" else None
        if self.case.startswith("replace-") or self.case == "lost-ack":
            request = self.read_phase(self.private / "request-2.json")
            auth.keys(request, ("phase", "sequence", "owner_pid", "owner_start", "pid", "start", "sid",
                                "sent_boottime", "binding_sha256", "challenge_sha256"))
            auth.require(request["phase"] == "paperless-check" and request["sequence"] == 2
                         and request["owner_pid"] == self.owner["parent_pid"]
                         and request["owner_start"] == self.owner["parent_start"] and request["sid"] == self.owner["parent_pid"]
                         and request["binding_sha256"] == self.lease_binding_hash
                         and type(request["sent_boottime"]) in (int, float)
                         and 0 < request["sent_boottime"] < self.owner["execution_deadline"])
            auth.positive(request["pid"])
            auth.positive(request["start"])
            auth.sha(request["challenge_sha256"])
        if self.case == "lost-ack":
            control = self.read_phase(self.private / "lost-ack.json")
            auth.keys(control, ("phase", "owner_pid", "owner_start", "pid", "start", "sid", "sequence",
                                "binding_sha256", "challenge_sha256"))
            auth.require(control["phase"] == "paperless-check" and control["owner_pid"] == self.owner["parent_pid"]
                         and control["owner_start"] == self.owner["parent_start"] and control["sid"] == self.owner["parent_pid"]
                         and control["sequence"] == 3 and control["binding_sha256"] == self.lease_binding_hash)
            auth.positive(control["pid"])
            auth.positive(control["start"])
            auth.sha(control["challenge_sha256"])
        for kind, identity in (("container", self.cid), ("container", self.owner["outer_name"]), ("volume", self.owner["volume_name"])):
            auth.require(self.command(["presence", kind, identity])[0] == 1)
        auth.require(not os.path.lexists(self.owner["directory"]))
        result_path = self.private / "result.json"
        if self.case in {"positive", "guard-loss"}:
            result = self.read_phase(result_path)
            auth.require(result["case"] == self.case)
            self.known_calls, self.known_bytes = result["calls"], result["read_bytes"]
            self.callbacks = result["callbacks"]
        upper_calls, upper_bytes = self.uncertainty()
        return {"owner_pid": self.owner["parent_pid"], "owner_start": self.owner["parent_start"],
            "guard_pid": self.guard["pid"], "guard_start": self.guard["start"], "inner_peer": self.inner_peer,
            "role_objects": self.roles, "view": self.held_view, "callbacks": self.callbacks, "close_before_removal": True, "owner_exited": True, "guard_exited": True,
            "core_absent": True, "storage_absent": True, "private_absent": True,
            "calls": self.known_calls + self.calls - self.ack_calls,
            "read_bytes": self.known_bytes + self.bytes - self.ack_bytes,
            "calls_upper_bound": self.known_calls + self.calls - self.ack_calls + upper_calls,
            "read_bytes_upper_bound": self.known_bytes + self.bytes - self.ack_bytes + upper_bytes,
            "uncertain_attempt": self.case != "positive", "allowance_reused": False, "expiry_decision": expiry_decision}

    def close(self):
        for fd in (self.owner_fd, self.guard_fd, self.outer_fd):
            if fd is not None:
                os.close(fd)


def mechanism(args, *, phase=None):
    def mark(name):
        if phase is not None:
            phase(name)

    mark("mechanism-runtime")
    runtime_environment(os.environ, require_docker=True)
    mark("mechanism-context")
    auth.require(os.getuid() == os.geteuid() == os.getgid() == os.getegid() == 0
                 and args.box_sha == args.trusted_sha == os.environ.get("GITHUB_SHA")
                 and args.trusted_sha == os.environ.get("GITHUB_WORKFLOW_SHA")
                 and args.box_root == args.trusted_root
                 and str(args.run_id) == os.environ.get("GITHUB_RUN_ID")
                 and str(args.attempt) == os.environ.get("GITHUB_RUN_ATTEMPT")
                 and args.box_pr is None and args.lens_pr is None and args.build_artifact_id is None
                 and args.case in auth.CUSTODY_CASES and args.lane == "upstream-rootless"
                 and not os.environ.get("GITHUB_TOKEN") and not os.environ.get("GH_TOKEN"))
    mark("mechanism-source")
    box, sdk, files = auth.custody_sources(args.trusted_root, args.lens_root, args.trusted_sha)
    auth.require(args.lens_sha == sdk["revision"])
    mark("mechanism-copy")
    auth.copy_closure(files, args.execution_root)
    mark("mechanism-binding")
    args.private.mkdir(mode=0o700)
    actors = {name: os.environ.get(name, "") for name in ("GITHUB_ACTOR", "GITHUB_TRIGGERING_ACTOR")}
    auth.require(all(re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9-]{0,38}", value) is not None for value in actors.values())
                 and args.checked_at is not None and 0 <= time.time() - args.checked_at <= 300)
    context = {"run_id": str(args.run_id), "attempt": str(args.attempt), "workflow_sha": args.trusted_sha,
               "box": box, "actors": actors, "checked_at": args.checked_at, "job": os.environ["GITHUB_JOB"]}
    binding = {"schema": 1, "scope": "paperless-custody-mechanism", "case": args.case,
               "box": box, "sdk": sdk, "sdk_root": str(args.lens_root), "context": context}
    path = args.private / "mechanism-binding.json"
    write(path, binding)
    mark("mechanism-import")
    source = args.execution_root / auth.CUSTODY_LAUNCHER
    spec = importlib.util.spec_from_file_location("held_main_custody_mechanism", source)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    mark("mechanism-observer")
    observer = CustodyObserver(module, args.private, args.lens_root, context, args.case)
    args.output.mkdir(mode=0o700)
    def observe(process):
        mark("mechanism-observer")
        result = observer.tick(process)
        mark("mechanism-execution")
        return result

    try:
        mark("mechanism-execution")
        execute([sys.executable, str(source), "run", "--binding", str(path)], root=args.execution_root,
                log=args.private / "mechanism.log", seconds=900, cleanup_seconds=125, preserve_guard=True,
                observe=observe, accepted_statuses=(0,) if args.case in {"positive", "guard-loss"} else (-signal.SIGKILL,),
                mechanism_diagnostics=True)
        mark("mechanism-observer")
        observations = observer.finish()
        mark("mechanism-evidence")
        auth.custody_sources(args.trusted_root, args.lens_root, args.trusted_sha)
        for name, raw in files.items():
            auth.require(auth.read(args.execution_root / name) == raw)
        value = {"schema": 1, "scope": "paperless-custody-mechanism", "result": "checks-passed",
                 "case": args.case, "context": context, "sdk": sdk, "observations": observations,
                 "applications_accepted": 0, "full_driver": False, "target_recreation": False,
                 "native_qualified": False}
        proof.validate_mechanism(value, trusted=args.trusted_root, revision=args.trusted_sha,
                                 run_id=str(args.run_id), attempt=str(args.attempt), case=args.case, actors=actors)
        write(args.output / "mechanism.json", value)
    except BaseException:
        # Only already retained identities are reported. A partial journal is
        # never promoted to custody or permission for worker-side teardown.
        owner = observer.owner or {}
        guard = observer.guard or {}
        failure = {"schema": 1, "scope": "paperless-custody-mechanism", "result": "refused",
            "case": args.case, "context": context, "sdk": sdk, "applications_accepted": 0,
            "full_driver": False, "target_recreation": False, "native_qualified": False,
            "residuals": {"verification": "incomplete", "last_verified_phase": observer.phase,
                "owner_pid": owner.get("parent_pid"), "owner_start": owner.get("parent_start"),
                "guard_pid": guard.get("pid"), "guard_start": guard.get("start"), "core_id": observer.cid,
                "core_name": owner.get("outer_name"), "storage_name": owner.get("volume_name"),
                "private_directory": owner.get("directory"),
                "expiry_observation": str(observer.c.owner_expiry_observation_path(owner)) if owner else None,
                "expiry_observation_pending": str(observer.c.owner_expiry_observation_path(owner)) + ".pending" if owner else None,
                "worker_teardown_authority": False}}
        if not (args.output / "mechanism.json").exists():
            write(args.output / "mechanism.json", failure)
        raise
    finally:
        observer.close()


class ClosedArgumentParser(argparse.ArgumentParser):
    def error(self, message):
        raise ValueError("rejected") from None


def arguments():
    parser = ClosedArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("build", "run", "mechanism"))
    for name in ("trusted-root", "box-root", "lens-root", "execution-root", "private"):
        parser.add_argument("--" + name, type=Path, required=True)
    for name in ("run-id", "attempt"):
        parser.add_argument("--" + name, type=int, required=True)
    for name in ("box-pr", "lens-pr"):
        parser.add_argument("--" + name, type=int)
    for name in ("box-sha", "lens-sha", "trusted-sha"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--lane", choices=auth.LANES)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--bundle-root", type=Path, required=True)
    parser.add_argument("--build-artifact-id")
    parser.add_argument("--case", choices=auth.CUSTODY_CASES)
    parser.add_argument("--checked-at", type=int)
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
        if args.command == "mechanism":
            auth.require(args.output is not None)
            mechanism(args, phase=mark)
        elif args.command == "build":
            auth.require(args.case is None and args.checked_at is None)
            auth.require(args.lane in auth.LANES and args.output is None and args.build_artifact_id is None)
            build(args, phase=mark)
        else:
            auth.require(args.case is None and args.checked_at is None)
            auth.require(args.lane is not None and args.output is not None and args.build_artifact_id is not None)
            mark("native-runtime")
            run(args)
    except Exception:
        print(failure_message(phase), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
