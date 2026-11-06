"""No root/namespace/engine execution: finite lease, FD, handoff and custody controls."""

from __future__ import annotations

import ast
import fcntl
import hashlib
import io
import json
import os
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import unittest
from contextlib import ExitStack
from pathlib import Path
from typing import Any
from unittest.mock import ANY, Mock, call, patch

from boxferry_ci import devcontainer_lifetime as subject
from boxferry_ci import devcontainer_validation as validation
from boxferry_ci.contracts import ContractError
from boxferry_ci.runner import AbortContext, Diagnostic, RunResult, run


class LifetimeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.root.chmod(0o700)

    def test_payloads_compile_are_deterministic_and_import_no_package_as_root(self) -> None:
        for name, source in (
            ("bootstrap", subject.ROOT_PROGRAM),
            ("init", subject.INIT_PROGRAM),
            ("worker", subject.WORKER_PROGRAM),
        ):
            compile(source, name, "exec")
        script = (
            "from boxferry_ci.devcontainer_lifetime import BOOTSTRAP_SHA256; "
            "print(BOOTSTRAP_SHA256)"
        )
        for seed in ("1", "23"):
            result = subprocess.run(
                [sys.executable, "-c", script],
                env={**os.environ, "PYTHONHASHSEED": seed},
                capture_output=True,
                text=True,
                check=True,
                timeout=10,
            )
            self.assertEqual(result.stdout.strip(), subject.BOOTSTRAP_SHA256)
        for source in (subject.ROOT_PROGRAM, subject.INIT_PROGRAM):
            tree = ast.parse(source)
            imports = [
                node for node in ast.walk(tree) if isinstance(node, (ast.Import, ast.ImportFrom))
            ]
            self.assertFalse(any("boxferry_ci" in ast.unparse(node) for node in imports))
        self.assertEqual(
            hashlib.sha256(subject.ROOT_PROGRAM.encode()).hexdigest(), subject.BOOTSTRAP_SHA256
        )
        self.assertLess(len(subject.ROOT_PROGRAM.encode()), 100000)

    def test_fifo_claim_mutates_only_verified_open_inode_and_existing_writer_survives(self) -> None:
        os.mkfifo(self.root / "lease", 0o600)
        parent = subject._r_directory(str(self.root))
        writer = subject._lease_writer(parent)
        plan = {
            "preparation": str(self.root),
            "preparation_identity": subject._r_identity(parent),
            "uid": os.getuid(),
            "lease_identity": [os.fstat(writer).st_dev, os.fstat(writer).st_ino],
        }
        os.close(parent)
        locked = [False]
        original_fstat = os.fstat
        expected_inode = original_fstat(writer).st_ino

        def fstat(fd: int) -> os.stat_result:
            facts = original_fstat(fd)
            if locked[0] and facts.st_ino == expected_inode:
                values = list(facts)
                values[4] = 0
                return os.stat_result(values)
            return facts

        def root_claim(_fd: int, _uid: int, _gid: int) -> None:
            locked[0] = True

        try:
            with (
                patch.object(subject.os, "fchown", side_effect=root_claim) as claim,
                patch.object(subject.os, "fstat", side_effect=fstat),
            ):
                reader = subject._r_control(plan, "lease", os.O_RDONLY, claim=True)
            try:
                claim.assert_called_once_with(reader, 0, 0)
                self.assertFalse(os.get_inheritable(reader))
                self.assertEqual(fcntl.fcntl(reader, fcntl.F_GETFL) & os.O_ACCMODE, os.O_RDONLY)
                self.assertTrue(subject._r_lease(reader))
                with self.assertRaises(PermissionError):
                    os.open(self.root / "lease", os.O_WRONLY | os.O_NONBLOCK)
                os.write(writer, b"X")
                with self.assertRaisesRegex(ValueError, "unexpected lease"):
                    subject._r_lease(reader)
                os.close(writer)
                writer = -1
                self.assertFalse(subject._r_lease(reader))
            finally:
                os.close(reader)
        finally:
            if writer >= 0:
                os.close(writer)

    def test_forged_fifo_identity_or_symlink_is_never_claimed(self) -> None:
        os.mkfifo(self.root / "lease", 0o600)
        parent = subject._r_directory(str(self.root))
        plan = {
            "preparation": str(self.root),
            "preparation_identity": subject._r_identity(parent),
            "uid": os.getuid(),
            "lease_identity": [0, 0],
        }
        os.close(parent)
        with patch.object(subject.os, "fchown") as mutation:
            with self.assertRaisesRegex(ValueError, "inode custody"):
                subject._r_control(plan, "lease", os.O_RDONLY, claim=True)
            mutation.assert_not_called()
        (self.root / "lease").unlink()
        sentinel = self.root / "sentinel"
        sentinel.write_text("untouched")
        (self.root / "lease").symlink_to(sentinel)
        with patch.object(subject.os, "fchown") as mutation:
            with self.assertRaises(OSError):
                subject._r_control(plan, "lease", os.O_RDONLY, claim=True)
            mutation.assert_not_called()
        self.assertEqual(sentinel.read_text(), "untouched")

    def test_canonical_runner_exec_never_inherits_sole_writer_and_eof_is_real(self) -> None:
        os.mkfifo(self.root / "lease", 0o600)
        parent = subject._r_directory(str(self.root))
        try:
            original = subject._lease_writer(parent)
        finally:
            os.close(parent)
        writer = fcntl.fcntl(original, fcntl.F_DUPFD_CLOEXEC, 100)
        os.close(original)
        reader = os.open(self.root / "lease", os.O_RDONLY | os.O_NONBLOCK | os.O_CLOEXEC)
        try:
            code = (
                f"import os,sys\ntry: os.fstat({writer})\nexcept OSError: sys.exit(0)\nsys.exit(42)"
            )
            result = run(
                (sys.executable, "-c", code), cwd=self.root, stage="writer-fd", timeout_seconds=3
            )
            self.assertEqual(result.status, "success")
            self.assertFalse(os.get_inheritable(writer))
            os.close(writer)
            writer = -1
            self.assertEqual(os.read(reader, 1), b"")
            # No outer fork-only path may retain a writer; only canonical exec is used.
            tree = ast.parse(subject.inspect.getsource(subject.execute))
            self.assertFalse(
                any(
                    isinstance(node, ast.Call) and ast.unparse(node.func) == "os.fork"
                    for node in ast.walk(tree)
                )
            )
        finally:
            os.close(reader)
            if writer >= 0:
                os.close(writer)

    def test_sole_writer_startup_has_no_reader_until_root_opens(self) -> None:
        os.mkfifo(self.root / "lease", 0o600)
        parent = subject._r_directory(str(self.root))
        self.addCleanup(os.close, parent)
        writer = subject._lease_writer(parent)
        self.assertFalse(os.get_inheritable(writer))
        self.assertEqual(fcntl.fcntl(writer, fcntl.F_GETFL) & os.O_ACCMODE, os.O_WRONLY)
        try:
            with self.assertRaises(BrokenPipeError):
                os.write(writer, b"X")  # No dummy reader survived setup.
            reader = os.open(self.root / "lease", os.O_RDONLY | os.O_NONBLOCK)
            try:
                with self.assertRaises(BlockingIOError):
                    os.read(reader, 1)  # Alive writer is EAGAIN, never premature EOF.
                os.close(writer)
                writer = -1
                self.assertEqual(os.read(reader, 1), b"")
            finally:
                os.close(reader)
        finally:
            if writer >= 0:
                os.close(writer)

    def test_lease_setup_open_failures_and_inode_substitution_close_descriptors(self) -> None:
        os.mkfifo(self.root / "lease", 0o600)
        parent = subject._r_directory(str(self.root))
        self.addCleanup(os.close, parent)
        original_open, original_fstat = os.open, os.fstat
        for fault in ("reader", "writer", "inode"):
            opened: list[int] = []

            def open_fd(
                path: str | Path,
                flags: int,
                mode: int = 0o777,
                *,
                dir_fd: int | None = None,
                fault: str = fault,
                opened: list[int] = opened,
            ) -> int:
                access = flags & os.O_ACCMODE
                if (fault == "reader" and access == os.O_RDONLY) or (
                    fault == "writer" and access == os.O_WRONLY
                ):
                    raise OSError("lease open denied")
                descriptor = original_open(path, flags, mode, dir_fd=dir_fd)
                opened.append(descriptor)
                return descriptor

            def fstat(
                fd: int,
                fault: str = fault,
                opened: list[int] = opened,
            ) -> os.stat_result:
                facts = original_fstat(fd)
                if fault == "inode" and fd in opened:
                    values = list(facts)
                    values[1] += 1
                    return os.stat_result(values)
                return facts

            with (
                self.subTest(fault=fault),
                patch.object(subject.os, "open", side_effect=open_fd),
                patch.object(subject.os, "fstat", side_effect=fstat),
                self.assertRaises((OSError, ValueError)),
            ):
                subject._lease_writer(parent)
            for descriptor in opened:
                with self.assertRaises(OSError):
                    original_fstat(descriptor)

    def test_fork_only_duplicate_writer_prevents_eof_until_closed(self) -> None:
        os.mkfifo(self.root / "lease", 0o600)
        parent = subject._r_directory(str(self.root))
        self.addCleanup(os.close, parent)
        writer = subject._lease_writer(parent)
        reader = os.open(self.root / "lease", os.O_RDONLY | os.O_NONBLOCK)
        release_read, release_write = os.pipe()
        pid = os.fork()
        if pid == 0:
            os.close(release_write)
            os.read(release_read, 1)
            os.close(writer)
            os._exit(0)
        try:
            os.close(release_read)
            os.close(writer)
            with self.assertRaises(BlockingIOError):
                os.read(reader, 1)  # CLOEXEC alone cannot fix a fork-only duplicate.
        finally:
            os.write(release_write, b"C")
            os.close(release_write)
            os.waitpid(pid, 0)
        try:
            self.assertEqual(os.read(reader, 1), b"")
        finally:
            os.close(reader)

    def test_outer_setup_failure_returns_retained_custody_without_root_launch(self) -> None:
        automation = self.root / "setup"
        selected = automation / "automation/src/boxferry_ci/devcontainer_lifetime.py"
        selected.parent.mkdir(parents=True)
        selected.write_bytes(Path(subject.__file__).read_bytes())
        original_open = os.open
        for fault in ("lease", "witness"):
            opened: list[int] = []

            def open_fd(
                path: str | Path,
                flags: int,
                mode: int = 0o777,
                *,
                dir_fd: int | None = None,
                fault: str = fault,
                opened: list[int] = opened,
            ) -> int:
                if (fault == "lease" and path == "lease" and flags & os.O_WRONLY) or (
                    fault == "witness" and Path(path).name == "witness"
                ):
                    raise PermissionError("setup open denied")
                descriptor = original_open(path, flags, mode, dir_fd=dir_fd)
                opened.append(descriptor)
                return descriptor

            with (
                self.subTest(fault=fault),
                patch.object(subject.os, "open", side_effect=open_fd),
                patch.object(subject, "run") as launch,
            ):
                report = subject.execute(
                    {"work": self.root, "automation": automation, "local_sources": None}
                )
                launch.assert_not_called()
            self.assertEqual(report["status"], "failure")
            self.assertFalse(report["namespace_closed"])
            self.assertEqual(report["retained_scope_owner_uid"], os.getuid())
            self.assertEqual(report["retained_control_mode"], 0o700)
            self.assertIn("setup open denied", str(report["cause"]))
            for key in ("retained_private_directory", "retained_lifetime_preparation"):
                shutil.rmtree(str(report[key]))
            for descriptor in opened:
                with self.assertRaises(OSError):
                    os.fstat(descriptor)

    def bootstrap(self, case: str) -> tuple[list[tuple[str, object]], Mock, int | None]:
        events: list[tuple[str, object]] = []
        uid = os.getuid()
        plan: dict[str, Any] = {key: None for key in subject.PLAN_KEYS}
        plan.update(
            schema=1,
            nonce="a" * 32,
            uid=uid,
            gid=os.getgid(),
            groups=[],
            scope="/owned/scope",
            scope_identity=[1, 2, uid, 0o700],
            host_pid_namespace="pid:[outside]",
            host_mount_namespace="mnt:[outside]",
            environment=subject._r_environment("/owned/scope"),
        )
        process = Mock(pid=55)
        process.returncode = None
        process.poll.side_effect = lambda: process.returncode
        process.kill.side_effect = lambda: setattr(process, "returncode", -9)
        process.terminate.side_effect = lambda: events.append(("terminate-supervisor", 55))
        process.wait.return_value = -9
        tick = [0.0]

        def now() -> float:
            tick[0] += 0.25
            return tick[0]

        def lease(_fd: int) -> bool:
            if case == "before-start":
                return False
            if case == "normal":
                return True
            return tick[0] < 0.5

        def read(_fd: int, _size: int) -> bytes:
            if _fd == 31:
                return b"a" * 32
            if case == "no-ready":
                return b""
            return (
                json.dumps(
                    {
                        "nonce": "a" * 32,
                        "pid": 1,
                        "uid": 0,
                        "pid_namespace": "pid:[inside]",
                        "mount_namespace": "mnt:[inside]",
                    }
                ).encode()
                + b"\n"
            )

        def write(fd: int, data: bytes) -> int:
            events.append(("write", (fd, data)))
            if data == b"G":
                self.assertIn(("pidfd", 123), events)
                process.returncode = 0
            return len(data)

        def selection(
            fds: list[int], _a: object, _b: object, _timeout: int
        ) -> tuple[list[int], list[int], list[int]]:
            return ([99] if process.returncode is not None else [], [], [])

        status = {
            "PPid": "999" if case == "forged-pid" else "55",
            "Uid": "0 0 0 0",
            "NSpid": "123 1",
        }
        with ExitStack() as held:
            replacements = {
                "_r_auth": lambda _plan: (uid, os.getgid(), []),
                "_r_payload_hash": lambda: subject.BOOTSTRAP_SHA256,
                "_r_only_child": lambda _pid: 123,
                "_r_directory": lambda _path: 88,
                "_r_identity": lambda _fd: [1, 2, uid, 0o700],
                "_r_control": lambda _plan, name, _flags, **_kwargs: 20 if name == "lease" else 21,
                "_r_lease": lease,
                "_r_start": lambda _pid: "7",
                "_r_status": lambda _pid: status,
                "_r_event": lambda _fd, _nonce, event, **values: events.append((event, values)),
            }
            for name, value in replacements.items():
                held.enter_context(patch.object(subject, name, value))
            held.enter_context(patch.object(subject.subprocess, "Popen", return_value=process))
            held.enter_context(
                patch.object(
                    subject.os,
                    "readlink",
                    side_effect=lambda name: (
                        ("pid:[outside]" if name.endswith("/pid") else "mnt:[outside]")
                        if name.startswith("/proc/self/")
                        else ("pid:[inside]" if name.endswith("/pid") else "mnt:[inside]")
                    ),
                )
            )
            held.enter_context(patch.object(subject.os, "open", return_value=31))
            held.enter_context(
                patch.object(subject.os, "listdir", return_value=[".lifetime-nonce"])
            )
            held.enter_context(
                patch.object(
                    subject.os,
                    "fstat",
                    return_value=Mock(st_uid=uid, st_mode=stat.S_IFREG | 0o600, st_nlink=1),
                )
            )
            held.enter_context(patch.object(subject.os, "pipe2", side_effect=[(11, 12), (13, 14)]))
            held.enter_context(
                patch.object(
                    subject.os, "close", side_effect=lambda fd: events.append(("close", fd))
                )
            )
            held.enter_context(patch.object(subject.os, "set_blocking"))
            held.enter_context(patch.object(subject.os, "read", side_effect=read))
            held.enter_context(patch.object(subject.os, "write", side_effect=write))
            held.enter_context(
                patch.object(
                    subject.os,
                    "pidfd_open",
                    side_effect=lambda pid, _flags: events.append(("pidfd", pid)) or 99,
                )
            )
            held.enter_context(patch.object(subject.signal, "signal"))
            held.enter_context(
                patch.object(
                    subject.signal,
                    "pidfd_send_signal",
                    side_effect=lambda fd, sig: events.append(("signal", (fd, sig))),
                )
            )
            held.enter_context(patch.object(subject.select, "select", side_effect=selection))
            held.enter_context(patch.object(subject.time, "monotonic", side_effect=now))
            held.enter_context(patch.object(subject.time, "sleep"))
            if case == "forged-pid":
                with self.assertRaisesRegex(ValueError, "custody"):
                    subject._r_bootstrap(plan)
                return events, process, None
            return events, process, subject._r_bootstrap(plan)

    def test_bootstrap_reads_only_the_held_supervisors_single_bounded_child(self) -> None:
        for content in (b"123 ", b"", b"123 456", b"-3", b"0", b"9" * 257):
            with (
                self.subTest(content=content),
                patch("builtins.open", return_value=io.BytesIO(content)) as opened,
            ):
                if content == b"123 ":
                    self.assertEqual(subject._r_only_child(55), 123)
                else:
                    with self.assertRaisesRegex(ValueError, "exactly one"):
                        subject._r_only_child(55)
                opened.assert_called_once_with("/proc/55/task/55/children", "rb")

    def test_before_start_eof_never_launches_or_releases_worker(self) -> None:
        events, process, code = self.bootstrap("before-start")
        self.assertEqual(code, 125)
        process.poll.assert_not_called()
        self.assertFalse(any(event == "write" for event, _ in events))

    def test_go_requires_pidfd_binding_and_late_eof_uses_root_hard_escalation(self) -> None:
        events, _, code = self.bootstrap("normal")
        self.assertEqual(code, 0)
        self.assertIn(("write", (14, b"G")), events)
        events, process, code = self.bootstrap("late-eof")
        self.assertEqual(code, 125)
        self.assertFalse(any(event == "write" for event, _ in events))
        self.assertIn(("signal", (99, signal.SIGTERM)), events)
        self.assertIn(("signal", (99, signal.SIGKILL)), events)
        process.kill.assert_called()

    def test_no_ready_timeout_and_forged_pid_never_signal_unknown_or_send_go(self) -> None:
        events, process, code = self.bootstrap("no-ready")
        self.assertEqual(code, 125)
        process.kill.assert_called()
        self.assertFalse(any(event in {"signal", "write"} for event, _ in events))
        self.assertTrue(any(event == "uncertain" for event, _ in events))
        events, _, _ = self.bootstrap("forged-pid")
        self.assertNotIn(("write", (14, b"G")), events)
        self.assertFalse(any(event == "signal" for event, _ in events))
        self.assertIn(("close", 99), events)

    def test_authority_comes_from_live_operator_not_plan_groups(self) -> None:
        uid, gid = os.getuid(), os.getgid()
        caller = {
            "Uid": " ".join([str(uid)] * 4),
            "Gid": " ".join([str(gid)] * 4),
            "Groups": "12 34",
            "PPid": "77",
        }
        with (
            patch.object(subject.os, "getuid", return_value=0),
            patch.object(subject.os, "geteuid", return_value=0),
            patch.object(subject.os, "getppid", return_value=66),
            patch.dict(os.environ, {"SUDO_UID": str(uid), "SUDO_GID": str(gid)}),
            patch.object(subject, "_r_start", return_value="held"),
            patch.object(
                subject,
                "_r_status",
                side_effect=lambda pid: caller if pid == 77 else {"PPid": "77"},
            ),
        ):
            self.assertEqual(
                subject._r_auth(
                    {
                        "uid": uid,
                        "gid": gid,
                        "caller_pid": 77,
                        "caller_start": "held",
                        "groups": [0],
                    }
                ),
                (uid, gid, [12, 34]),
            )

    def test_saved_identity_drop_and_worker_fd_closure_precede_exec_imports(self) -> None:
        events: list[str] = []
        with (
            patch.object(
                subject.os, "setgroups", side_effect=lambda _groups: events.append("groups")
            ),
            patch.object(subject.os, "setresgid", side_effect=lambda *_args: events.append("gid")),
            patch.object(subject.os, "setresuid", side_effect=lambda *_args: events.append("uid")),
            patch.object(subject.os, "getresuid", return_value=(1000,) * 3),
            patch.object(subject.os, "getresgid", return_value=(1000,) * 3),
            patch.object(subject.os, "getgroups", return_value=[12, 34]),
        ):
            subject._r_drop(1000, 1000, [12, 34])
        self.assertEqual(events, ["groups", "gid", "uid"])
        source = subject.inspect.getsource(subject._r_init)
        self.assertNotIn("hostproc", source)
        self.assertNotIn('"/proc"', subject.inspect.getsource(subject._r_bootstrap))
        self.assertLess(source.index("_r_drop("), source.index("os.execve("))
        self.assertLess(source.index("os.close(fd)"), source.index("_r_drop("))
        self.assertLess(
            subject.WORKER_PROGRAM.index("os.set_inheritable"),
            subject.WORKER_PROGRAM.index("from boxferry_ci"),
        )

    def test_fixed_worker_rejects_missing_bad_or_different_attestation_before_imports(self) -> None:
        prefix = subject.WORKER_PROGRAM.split("sys.path.insert", 1)[0]
        base: dict[str, Any] = {
            "uid": os.getuid(),
            "gid": os.getgid(),
            "groups": os.getgroups(),
            "abort_fd": 30,
            "host_pid_namespace": "pid:[100]",
            "host_mount_namespace": "mnt:[200]",
            "pid1_pid_namespace": "pid:[101]",
            "pid1_mount_namespace": "mnt:[201]",
        }
        self.assertLess(
            subject.WORKER_PROGRAM.index("os.set_inheritable"),
            subject.WORKER_PROGRAM.index("root-attested worker"),
        )
        self.assertLess(
            subject.WORKER_PROGRAM.index("root-attested worker"),
            subject.WORKER_PROGRAM.index("from boxferry_ci"),
        )
        for fault in (
            "none",
            "missing",
            "type",
            "format",
            "host",
            "pid-mismatch",
            "mount-mismatch",
            "mount-missing",
            "mount-host",
        ):
            plan = dict(base)
            if fault == "missing":
                plan.pop("pid1_pid_namespace")
            if fault == "type":
                plan["pid1_pid_namespace"] = 101
            if fault == "format":
                plan["pid1_pid_namespace"] = "pid:[invalid]"
            if fault == "host":
                plan["pid1_pid_namespace"] = plan["host_pid_namespace"]
            if fault == "pid-mismatch":
                plan["pid1_pid_namespace"] = "pid:[999]"
            if fault == "mount-mismatch":
                plan["pid1_mount_namespace"] = "mnt:[999]"
            if fault == "mount-missing":
                plan.pop("pid1_mount_namespace")
            if fault == "mount-host":
                plan["pid1_mount_namespace"] = plan["host_mount_namespace"]

            def link(name: str) -> str:
                if name.startswith("/proc/1/ns/"):
                    raise PermissionError("forbidden root PID1 link")
                return "pid:[101]" if name.endswith("/pid") else "mnt:[201]"

            with (
                self.subTest(fault=fault),
                patch.object(sys, "argv", ["worker", json.dumps(plan)]),
                patch.object(subject.os, "listdir", return_value=["0", "1", "2", "30"]),
                patch.object(subject.os, "set_inheritable"),
                patch.object(subject.os, "readlink", side_effect=link),
            ):
                if fault == "none":
                    exec(compile(prefix, "fixed-worker-prefix", "exec"), {})
                else:
                    with self.assertRaisesRegex(RuntimeError, "root-attested worker"):
                        exec(compile(prefix, "fixed-worker-prefix", "exec"), {})

    def test_root_pid1_injects_only_ready_snapshot_after_go_into_worker_argv(self) -> None:
        def run_case(case: str) -> None:
            events: list[str] = []
            observed: list[dict[str, Any]] = []
            ready: list[dict[str, Any]] = []
            plan: dict[str, Any] = {
                "nonce": "a" * 32,
                "uid": os.getuid(),
                "gid": os.getgid(),
                "groups": os.getgroups(),
                "environment": {},
            }
            self.assertFalse({"pid1_pid_namespace", "pid1_mount_namespace"} & subject.PLAN_KEYS)

            def write(_fd: int, content: bytes) -> int:
                events.append("ready")
                ready.append(json.loads(content))
                return len(content)

            def read(_fd: int, _size: int) -> bytes:
                events.append("go")
                return b"G" if case != "bad-go" else b"N"

            def fork() -> int:
                events.append("fork")
                self.assertNotIn("pid1_pid_namespace", plan)
                return 0

            def execute(_path: str, argv: tuple[str, ...], _environment: object) -> None:
                events.append("exec")
                observed.append(json.loads(argv[-1]))
                raise RuntimeError("fixed worker exec reached")

            with ExitStack() as held:
                for name, value in (
                    ("getuid", lambda: 0),
                    ("getpid", lambda: 1),
                    ("geteuid", lambda: 0),
                    (
                        "readlink",
                        lambda name: "pid:[101]" if name.endswith("/pid") else "mnt:[201]",
                    ),
                    ("write", write),
                    ("read", read),
                    ("close", lambda _fd: None),
                    ("pipe2", lambda _flags: (3, 6)),
                    ("fork", fork),
                    ("set_inheritable", lambda *_args: None),
                    ("execve", execute),
                    ("_exit", lambda _code: (_ for _ in ()).throw(RuntimeError("no worker GO"))),
                ):
                    held.enter_context(patch.object(subject.os, name, value))
                held.enter_context(patch.object(subject.signal, "signal"))
                held.enter_context(
                    patch.object(subject.select, "select", return_value=([7], [], []))
                )
                held.enter_context(
                    patch.object(
                        subject, "_r_drop", side_effect=lambda *_args: events.append("drop")
                    )
                )
                held.enter_context(
                    patch.object(subject, "_r_lease", side_effect=[True, case != "late-eof"])
                )
                with self.assertRaisesRegex(
                    RuntimeError, "exec reached" if case == "go" else "no worker GO"
                ):
                    subject._r_init(plan, 4, 5, 6, 7)
            if case == "go":
                self.assertEqual(events, ["ready", "go", "fork", "drop", "exec"])
                for key in ("pid_namespace", "mount_namespace"):
                    self.assertEqual(observed[0]["pid1_" + key], ready[0][key])
            else:
                self.assertNotIn("fork", events)
                self.assertFalse(observed)
            self.assertNotIn("pid1_pid_namespace", plan)

        for case in ("go", "bad-go", "late-eof"):
            with self.subTest(case=case):
                run_case(case)

    def test_outer_plan_cannot_supply_root_namespace_attestation(self) -> None:
        plan = {key: None for key in subject.PLAN_KEYS}
        plan.update(schema=1, nonce="a" * 32, pid1_pid_namespace="pid:[forged]")
        with (
            patch.object(subject, "_r_auth", return_value=(1000, 1000, [])),
            patch.object(subject, "_r_control") as controls,
            self.assertRaisesRegex(ValueError, "closed lifetime plan"),
        ):
            subject._r_bootstrap(plan)
        controls.assert_not_called()

    def test_worker_context_binds_protected_root_pid1_code_and_plan(self) -> None:
        private = self.root / "context"
        private.mkdir(mode=0o700)
        nonce = "a" * 32
        (private / ".lifetime-nonce").write_text(nonce)
        context = {
            "private": str(private),
            "nonce": nonce,
            "pid_namespace": "pid:[inside]",
            "mount_namespace": "mnt:[inside]",
            "host_pid_namespace": "pid:[outside]",
            "host_mount_namespace": "mnt:[outside]",
            "pid1_pid_namespace": "pid:[inside]",
            "pid1_mount_namespace": "mnt:[inside]",
            "abort_fd": 30,
            "result": str(self.root / "result"),
            "result_identity": [1, 2],
        }
        parent = {
            "uid": os.getuid(),
            "scope": str(private),
            "nonce": nonce,
            "host_pid_namespace": context["host_pid_namespace"],
            "host_mount_namespace": context["host_mount_namespace"],
        }
        original = Path.open

        def readlink(name: str) -> str:
            if name.startswith("/proc/1/ns/"):
                raise PermissionError("root PID1 namespace access denied")
            return "pid:[inside]" if name.endswith("/pid") else "mnt:[inside]"

        def check(code: bytes) -> Path:
            command = b"\0".join(
                [
                    b"/usr/bin/python3",
                    b"-I",
                    b"-S",
                    b"-c",
                    code,
                    json.dumps(parent).encode(),
                    b"20",
                    b"21",
                ]
            )

            def opened(
                path: Path,
                mode: str = "r",
                buffering: int = -1,
                encoding: str | None = None,
                errors: str | None = None,
                newline: str | None = None,
            ) -> object:
                if path == Path("/proc/1/cmdline"):
                    return io.BytesIO(command)
                return original(
                    path,
                    mode,
                    buffering=buffering,
                    encoding=encoding,
                    errors=errors,
                    newline=newline,
                )

            with (
                patch.object(subject.os, "getppid", return_value=1),
                patch.object(subject, "_r_status", return_value={"Uid": "0 0 0 0"}),
                patch.object(Path, "open", opened),
                patch.object(
                    subject.os,
                    "readlink",
                    side_effect=readlink,
                ),
            ):
                return subject.validate_worker_context(context)

        self.assertEqual(check(subject.INIT_PROGRAM.encode()), private)
        with self.assertRaisesRegex(ContractError, "fixed init payload"):
            check(b"unreviewed root code")
        context["pid1_pid_namespace"] = "pid:[forged]"
        with self.assertRaisesRegex(ContractError, "exact private"):
            check(subject.INIT_PROGRAM.encode())
        context["pid1_pid_namespace"] = "pid:[inside]"
        context["host_pid_namespace"] = "pid:[forged]"
        with self.assertRaisesRegex(ContractError, "protected PID1 plan"):
            check(subject.INIT_PROGRAM.encode())

    def test_outer_receipt_first_write_is_durable_and_never_overwrites_inner_or_first_outer(
        self,
    ) -> None:
        inner = self.root / "inner-result"
        inner.write_text("original inner first failure")
        path = self.root / "outer-abort"
        events: list[str] = []

        def close() -> None:
            self.assertEqual(json.loads(path.read_text())["cause"], "actual SIGTERM")
            events.append("close")

        evidence = subject._OuterAbortEvidence(path, "a" * 32, close)
        with patch.object(
            subject, "_persist_outer_abort", wraps=subject._persist_outer_abort
        ) as write:
            evidence.abort(
                AbortContext("outer", "cancelled", "actual SIGTERM"), int(signal.SIGTERM)
            )
            original = path.read_bytes()
            evidence.abort(AbortContext("outer", "cancelled", "command was cancelled"))
            write.assert_called_once_with(path, evidence.first)
        self.assertEqual(evidence.persistence_failure, "")
        self.assertEqual(evidence.close_failure, "")
        self.assertNotIn("outer_abort_persistence_failure", evidence.fields())
        self.assertNotIn("outer_abort_close_failure", evidence.fields())
        self.assertEqual(path.read_bytes(), original)
        self.assertEqual(inner.read_text(), "original inner first failure")
        self.assertEqual(events, ["close", "close"])
        self.assertIs(evidence.fields()["outer_abort_persisted"], True)
        self.assertEqual(evidence.fields()["outer_abort_signal_number"], int(signal.SIGTERM))
        self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
        self.assertEqual(
            evidence.fields()["outer_abort_receipt_sha256"], hashlib.sha256(original).hexdigest()
        )
        with self.assertRaises(FileExistsError):
            subject._persist_outer_abort(path, {"cause": "replacement"})
        self.assertEqual(path.read_bytes(), original)

    def test_outer_persistence_failure_closes_lease_and_cannot_claim_retention_or_retry(
        self,
    ) -> None:
        events: list[str] = []

        def persist(_path: Path, _record: object) -> str:
            events.append("persist-failed")
            raise OSError("original receipt disk failure")

        evidence = subject._OuterAbortEvidence(
            self.root / "failed-outer", "a" * 32, lambda: events.append("close")
        )
        with patch.object(subject, "_persist_outer_abort", side_effect=persist) as write:
            for cause, number in (
                ("actual SIGINT", int(signal.SIGINT)),
                ("command was cancelled", None),
            ):
                with self.assertRaisesRegex(ContractError, "receipt disk failure"):
                    evidence.abort(AbortContext("outer", "cancelled", cause), number)
                self.assertEqual(evidence.persistence_failure, "original receipt disk failure")
                self.assertIs(evidence.persisted, False)
            write.assert_called_once()
        self.assertEqual(events, ["persist-failed", "close", "close"])
        fields = evidence.fields()
        self.assertIs(fields["outer_abort_persisted"], False)
        self.assertEqual(fields["outer_abort_cause"], "actual SIGINT")
        self.assertIn("receipt disk failure", str(fields["outer_abort_persistence_failure"]))
        self.assertNotIn("outer_abort_receipt_sha256", fields)

    def test_outer_failed_persistence_stays_distinct_in_final_receipt_and_does_not_skip_cleanup(
        self,
    ) -> None:
        automation = self.root / "persist-failure"
        selected = automation / "automation/src/boxferry_ci/devcontainer_lifetime.py"
        selected.parent.mkdir(parents=True)
        selected.write_bytes(Path(subject.__file__).read_bytes())
        phases: list[str] = []

        def invoke(argv: tuple[str, ...], **options: object) -> RunResult:
            phase, plan = argv[-2], json.loads(argv[-1])
            phases.append(phase)
            scope, control = Path(plan["scope"]), Path(plan["preparation"])
            if phase == "qualify":
                self.addCleanup(shutil.rmtree, scope, ignore_errors=True)
                self.addCleanup(shutil.rmtree, control, ignore_errors=True)
                hook = options["on_abort"]
                assert callable(hook)
                with self.assertRaisesRegex(ContractError, "original receipt disk failure"):
                    hook(
                        AbortContext(
                            "local-rootful-lifetime", "infrastructure", "original outer monitor"
                        )
                    )
                reader = os.open(control / "lease", os.O_RDONLY | os.O_NONBLOCK)
                try:
                    self.assertEqual(os.read(reader, 1), b"")
                finally:
                    os.close(reader)
                Path(plan["result"]).write_text(
                    json.dumps(
                        {
                            "status": "success",
                            "outer_abort_persisted": True,
                            "outer_abort_receipt_sha256": "f" * 64,
                            "outer_abort_receipt_owner_uid": 0,
                        }
                    )
                )
                return RunResult(
                    "infrastructure",
                    1,
                    "",
                    "",
                    0,
                    Diagnostic("outer", "generic monitor failure", "", "", ""),
                    argv,
                    abort_hook_failure="original receipt disk failure",
                )
            for directory in (scope, control):
                for path in directory.iterdir():
                    path.unlink()
            return RunResult(
                "success", 0, "", "", 0, Diagnostic("finalize", "done", "", "", ""), argv
            )

        with (
            patch.object(subject, "run", side_effect=invoke),
            patch.object(
                subject,
                "_persist_outer_abort",
                side_effect=OSError("original receipt disk failure"),
            ),
            patch.object(
                subject,
                "_witness",
                return_value=[
                    {"event": "closed", "namespace_closed": True, "supervisor_closed": True}
                ],
            ),
        ):
            report = subject.execute(
                {"work": self.root, "automation": automation, "local_sources": None}
            )
        self.assertEqual(phases, ["qualify", "finalize"])
        self.assertEqual(report["status"], "failure")
        self.assertIs(report["outer_abort_persisted"], False)
        self.assertEqual(report["outer_abort_cause"], "original outer monitor")
        self.assertIn(
            "original receipt disk failure", str(report["outer_abort_persistence_failure"])
        )
        self.assertEqual(report["abort_hook_failure"], "original receipt disk failure")
        self.assertEqual(report["directory_cleanup"], "success")
        self.assertNotIn("outer_abort_receipt_sha256", report)
        self.assertNotIn("outer_abort_receipt_owner_uid", report)

    def test_actual_outer_term_and_int_persist_before_writer_close_and_keep_first_causes(
        self,
    ) -> None:
        def run_case(number: signal.Signals, earlier_inner: bool = False) -> None:
            automation = self.root / (number.name + str(earlier_inner))
            selected = automation / "automation/src/boxferry_ci/devcontainer_lifetime.py"
            selected.parent.mkdir(parents=True)
            selected.write_bytes(Path(subject.__file__).read_bytes())
            state: dict[str, Any] = {}
            ordered: list[str] = []
            real_close, real_fstat = os.close, os.fstat
            real_persist = subject._persist_outer_abort

            def durable(path: Path, record: object) -> str:
                assert isinstance(record, dict)
                state["persist_calls"] = state.get("persist_calls", 0) + 1
                result = real_persist(path, record)
                state["receipt"] = path
                ordered.append("persisted")
                return result

            def close(fd: int) -> None:
                facts = real_fstat(fd)
                if (facts.st_dev, facts.st_ino) == state.get("lease_identity") and fcntl.fcntl(
                    fd, fcntl.F_GETFL
                ) & os.O_ACCMODE == os.O_WRONLY:
                    self.assertEqual(ordered, ["persisted"])
                    packet = json.loads(state["receipt"].read_text())
                    self.assertEqual(packet["signal_number"], int(number))
                    self.assertEqual(packet["status"], "cancelled")
                    ordered.append("lease-closed")
                real_close(fd)

            def invoke(argv: tuple[str, ...], **options: object) -> RunResult:
                plan = json.loads(argv[-1])
                phase = argv[-2]
                scope, preparation = Path(plan["scope"]), Path(plan["preparation"])
                if phase == "qualify":
                    self.addCleanup(shutil.rmtree, scope, ignore_errors=True)
                    self.addCleanup(shutil.rmtree, preparation, ignore_errors=True)
                    state["lease_identity"] = tuple(plan["lease_identity"])
                    if earlier_inner:
                        state["inner"] = json.dumps(
                            {
                                "status": "failure",
                                "cause": "original inner timeout",
                                "command_failure_stage": "inner-probe",
                                "command_failure_status": "timeout",
                                "command_failure_cause": "original inner timeout",
                            }
                        )
                        Path(plan["result"]).write_text(state["inner"])
                    reader = os.open(preparation / "lease", os.O_RDONLY | os.O_NONBLOCK)
                    try:
                        with self.assertRaises(subject.RunCancelled):
                            os.kill(os.getpid(), number)  # Actual unprivileged kernel signal.
                        self.assertEqual(os.read(reader, 1), b"")
                        before = state["receipt"].read_bytes()
                        hook = options["on_abort"]
                        assert callable(hook)
                        hook(
                            AbortContext(
                                "local-rootful-lifetime", "cancelled", "command was cancelled"
                            )
                        )
                        self.assertEqual(state["receipt"].read_bytes(), before)
                    finally:
                        os.close(reader)
                    if earlier_inner:
                        self.assertEqual(Path(plan["result"]).read_text(), state["inner"])
                    return RunResult(
                        "cancelled",
                        -int(number),
                        "",
                        "",
                        0,
                        Diagnostic("local-rootful-lifetime", "command was cancelled", "", "", ""),
                        argv,
                    )
                if not earlier_inner:
                    for directory in (scope, preparation):
                        for path in directory.iterdir():
                            path.unlink()
                return RunResult(
                    "failed" if earlier_inner else "success",
                    1 if earlier_inner else 0,
                    "",
                    "",
                    0,
                    Diagnostic("finalize", "separate cleanup failure", "", "", ""),
                    argv,
                )

            with (
                patch.object(subject, "run", side_effect=invoke),
                patch.object(subject, "_persist_outer_abort", side_effect=durable),
                patch.object(subject.os, "close", side_effect=close),
                patch.object(
                    subject,
                    "_witness",
                    return_value=[
                        {"event": "closed", "namespace_closed": True, "supervisor_closed": True}
                    ],
                ),
            ):
                report = subject.execute(
                    {"work": self.root, "automation": automation, "local_sources": None}
                )
            self.assertEqual(ordered, ["persisted", "lease-closed"])
            self.assertEqual(state["persist_calls"], 1)
            self.assertNotIn("outer_abort_persistence_failure", report)
            self.assertNotIn("outer_abort_close_failure", report)
            self.assertNotIn("abort_hook_failure", report)
            self.assertIs(report["outer_abort_persisted"], True)
            self.assertEqual(report["outer_abort_signal_number"], int(number))
            self.assertEqual(report["outer_abort_signal_name"], number.name)
            self.assertEqual(report["outer_abort_stage"], "local-rootful-lifetime")
            self.assertEqual(report["outer_abort_status"], "cancelled")
            self.assertIn(number.name, str(report["outer_abort_cause"]))
            self.assertEqual(report["status"], "failure")
            if earlier_inner:
                self.assertEqual(report["cause"], "original inner timeout")
                self.assertEqual(report["command_failure_cause"], "original inner timeout")
                self.assertIn("separate cleanup failure", str(report["cleanup_cause"]))
            else:
                self.assertEqual(report["cause"], report["outer_abort_cause"])
                self.assertEqual(report["directory_cleanup"], "success")

        for number, earlier in (
            (signal.SIGTERM, False),
            (signal.SIGINT, False),
            (signal.SIGTERM, True),
        ):
            with self.subTest(signal=number.name, earlier_inner=earlier):
                run_case(number, earlier)

    def test_serialized_lifecycle_payloads_remain_exact_approved_v3_bytes(self) -> None:
        expected = {
            "bootstrap": "d367708b6c614e470bbb3a1eaa10b1364499ed7718d3903b4c5c16b2f02d37d0",
            "init": "24a1480b6dde3bae978e86a996b07d54828348fe8a1782ede6b536863bc782a7",
            "worker": "5eb289298564bd59623ef64e9c606a2abcbfefecc04e4c4e3999a3980c92ae42",
        }
        for name, value in (
            ("bootstrap", subject.ROOT_PROGRAM),
            ("init", subject.INIT_PROGRAM),
            ("worker", subject.WORKER_PROGRAM),
        ):
            self.assertEqual(hashlib.sha256(value.encode()).hexdigest(), expected[name])

    def test_original_failure_is_written_before_fixed_abort_and_is_idempotent(self) -> None:
        result = self.root / "result"
        result.touch(mode=0o600)
        read_fd, write_fd = os.pipe2(os.O_CLOEXEC | os.O_NONBLOCK)
        context = {
            "result": str(result),
            "result_identity": [result.stat().st_dev, result.stat().st_ino],
            "abort_fd": write_fd,
        }
        try:
            failure = AbortContext("inner-probe", "infrastructure", "original output limit")
            subject.notify_abort(context, failure, {"requested_outer_mode": "rootful"})
            self.assertEqual(os.read(read_fd, 2), b"A")
            preserved = json.loads(result.read_text())
            self.assertEqual(preserved["command_failure_cause"], failure.cause)
            self.assertEqual(preserved["command_failure_stage"], failure.stage)
            self.assertEqual(preserved["command_failure_status"], failure.status)
            subject.notify_abort(context, failure, {})
            with self.assertRaises(BlockingIOError):
                os.read(read_fd, 1)
        finally:
            os.close(read_fd)
            os.close(write_fd)

    def test_anchored_recursive_removal_never_follows_links_or_crosses_mounts(self) -> None:
        scope = self.root / "scope"
        scope.mkdir()
        (scope / "store").mkdir()
        (scope / "store/payload").write_text("owned")
        sentinel = self.root / "sentinel"
        sentinel.write_text("unrelated")
        (scope / "store/link").symlink_to(sentinel)
        fd = subject._r_directory(str(scope))
        try:
            with patch.object(subject.os, "fchown"):
                subject._r_remove(fd, "store", os.fstat(fd).st_dev, subject._r_mount(fd), [20])
            self.assertEqual(sentinel.read_text(), "unrelated")
            self.assertEqual(list(scope.iterdir()), [])
            (scope / "store").mkdir()
            with (
                patch.object(subject, "_r_mount", return_value="different"),
                patch.object(subject.os, "fchown"),
                self.assertRaisesRegex(ValueError, "mount"),
            ):
                subject._r_remove(fd, "store", os.fstat(fd).st_dev, "held", [20])
            self.assertTrue((scope / "store").exists())
        finally:
            os.close(fd)

    def test_actual_unprivileged_inner_timeout_output_and_monitor_preserve_primary(self) -> None:
        def case(kind: str) -> None:
            directory = self.root / kind
            directory.mkdir()
            result = directory / "result"
            result.touch(mode=0o600)
            read_fd, write_fd = os.pipe2(os.O_CLOEXEC | os.O_NONBLOCK)
            commands = validation.Commands(directory / "private")
            commands.lifetime_context = {
                "result": str(result),
                "result_identity": [result.stat().st_dev, result.stat().st_ino],
                "abort_fd": write_fd,
            }
            commands.lifetime_report = {"requested_outer_mode": "rootful"}
            ready = directory / "ready"

            def monitor() -> None:
                if ready.exists():
                    raise ContractError("original monitor disk reserve")

            if kind == "monitor":
                commands.budget = Mock(check=Mock(side_effect=monitor))
            program = (
                "import sys;sys.stdout.write('x'*2000100);sys.stdout.flush()"
                if kind == "output"
                else (
                    "from pathlib import Path;import time;"
                    f"Path({str(ready)!r}).touch();time.sleep(20)"
                )
            )
            try:
                with self.assertRaises(ValueError):
                    commands.call(
                        (sys.executable, "-c", program),
                        "inner-" + kind,
                        1 if kind == "timeout" else 3,
                    )
                self.assertEqual(os.read(read_fd, 2), b"A")
                preserved = json.loads(result.read_text())
                self.assertEqual(preserved["command_failure_stage"], "inner-" + kind)
                self.assertIn(
                    {
                        "output": "output limit",
                        "timeout": "timed out",
                        "monitor": "original monitor disk reserve",
                    }[kind],
                    preserved["command_failure_cause"],
                )
                self.assertEqual(preserved["status"], "failure")
            finally:
                os.close(read_fd)
                os.close(write_fd)

        for kind in ("timeout", "output", "monitor"):
            with self.subTest(kind=kind):
                case(kind)

    def test_handoff_failure_keeps_original_on_bounded_outer_channel_before_notice(self) -> None:
        result = self.root / "handoff"
        result.touch(mode=0o600)
        read_fd, write_fd = os.pipe2(os.O_CLOEXEC | os.O_NONBLOCK)
        context = {"result": str(result), "result_identity": [0, 0], "abort_fd": write_fd}
        events: list[tuple[str, bytes]] = []
        original = os.write

        def write(fd: int, data: bytes) -> int:
            if fd == 2:
                events.append(("fallback", data))
                return len(data)
            if fd == write_fd:
                events.append(("notice", data))
            return original(fd, data)

        try:
            with (
                patch.object(subject.os, "write", side_effect=write),
                self.assertRaisesRegex(ContractError, "custody"),
            ):
                subject.notify_abort(
                    context,
                    AbortContext("inner-monitor", "infrastructure", "original reserve failure"),
                    {},
                )
            self.assertEqual([event for event, _ in events], ["fallback", "notice"])
            packet = json.loads(events[0][1].removeprefix(b"BOXFERRY_LIFETIME_FAILURE "))
            self.assertEqual(packet["cause"], "original reserve failure")
            self.assertEqual(packet["stage"], "inner-monitor")
            self.assertIn("custody", packet["abort_hook_failure"])
            self.assertEqual(os.read(read_fd, 2), b"A")
        finally:
            os.close(read_fd)
            os.close(write_fd)

    def test_worker_descriptor_reuse_does_not_treat_notice_as_old_hostproc_pipe(self) -> None:
        plan: dict[str, Any] = {
            "nonce": "a" * 32,
            "uid": os.getuid(),
            "gid": os.getgid(),
            "groups": [],
            "environment": {},
        }
        observed: list[dict[str, Any]] = []

        def execute(_path: str, argv: tuple[str, ...], _environment: object) -> None:
            observed.append(json.loads(argv[-1]))
            raise RuntimeError("exec fixture reached after drop")

        with ExitStack() as held:
            for name, value in (
                ("getuid", lambda: 0),
                ("getpid", lambda: 1),
                (
                    "readlink",
                    lambda name: "pid:[inside]" if name.endswith("/pid") else "mnt:[inside]",
                ),
                ("open", lambda *_args, **_kwargs: 40),
                (
                    "read",
                    lambda fd, _size: (
                        ("42 (fixture) " + " ".join(["0"] * 19 + ["7"])).encode()
                        if fd == 40
                        else b"G"
                    ),
                ),
                ("close", lambda _fd: None),
                ("write", lambda _fd, data: len(data)),
                ("pipe2", lambda _flags: (3, 6)),
                ("fork", lambda: 0),
                ("set_inheritable", lambda *_args: None),
                ("execve", execute),
            ):
                held.enter_context(patch.object(subject.os, name, value))
            held.enter_context(patch.object(subject.signal, "signal"))
            held.enter_context(patch.object(subject, "_r_lease", return_value=True))
            drop = held.enter_context(patch.object(subject, "_r_drop"))
            held.enter_context(patch.object(subject.select, "select", return_value=([7], [], [])))
            with self.assertRaisesRegex(RuntimeError, "exec fixture"):
                subject._r_init(plan, 4, 5, 6, 7)
        drop.assert_called_once()
        self.assertEqual(observed[0]["abort_fd"], 6)
        self.assertNotIn(6, observed[0]["control_fds"])
        self.assertTrue({4, 5}.issubset(observed[0]["control_fds"]))
        self.assertNotIn(9, observed[0]["control_fds"])

    def test_substituted_scope_never_authorizes_unknown_finalizer_deletion(self) -> None:
        original = self.root / "scope"
        original.mkdir(mode=0o700)
        held = subject._r_directory(str(original))
        identity = subject._r_identity(held)
        os.close(held)
        original.rename(self.root / "moved")
        original.mkdir(mode=0o700)
        sentinel = original / "sentinel"
        sentinel.write_text("unrelated")
        witness = self.root / "witness"
        witness.write_text(
            "\n".join(
                json.dumps(record)
                for record in (
                    {"nonce": "a" * 32, "event": "pid1-bound", "pid": 123, "start": "7"},
                    {
                        "nonce": "a" * 32,
                        "event": "closed",
                        "namespace_closed": True,
                        "supervisor_closed": True,
                    },
                )
            )
            + "\n"
        )
        fd = os.open(witness, os.O_RDONLY | os.O_CLOEXEC)
        plan = {key: None for key in subject.PLAN_KEYS}
        plan.update(nonce="a" * 32, scope=str(original), scope_identity=identity)
        records = [json.loads(line) for line in witness.read_text().splitlines()]
        records[0].update(
            scope_identity=identity,
            preparation_identity=None,
            caller_uid=os.getuid(),
            bootstrap_sha256=subject.BOOTSTRAP_SHA256,
        )
        witness.write_text("\n".join(json.dumps(record) for record in records) + "\n")
        with (
            patch.object(subject, "_r_payload_hash", return_value=subject.BOOTSTRAP_SHA256),
            patch.object(subject, "_r_auth", return_value=(os.getuid(), os.getgid(), [])),
            patch.object(subject, "_r_control", return_value=fd),
            patch.object(subject.os, "fchown") as claim,
        ):
            with self.assertRaisesRegex(ValueError, "custody"):
                subject._r_finalize(plan)
            claim.assert_not_called()
        self.assertEqual(sentinel.read_text(), "unrelated")

    def test_anchored_finalizer_removes_only_children_and_restores_verified_parents(self) -> None:
        scope, control = self.root / "scope", self.root / "control"
        scope.mkdir(mode=0o700)
        control.mkdir(mode=0o700)
        nonce = "a" * 32
        (scope / ".lifetime-nonce").touch(mode=0o600)
        (scope / ".lifetime-nonce").write_text(nonce)
        (scope / "store").mkdir()
        (scope / "store/payload").write_text("owned")
        os.mkfifo(control / "lease", 0o600)
        (control / "result").touch(mode=0o600)
        records: list[dict[str, Any]] = [
            {"nonce": nonce, "event": "pid1-bound", "pid": 123, "start": "7"},
            {
                "nonce": nonce,
                "event": "closed",
                "namespace_closed": True,
                "supervisor_closed": True,
            },
        ]
        witness = control / "witness"
        witness.write_text("\n".join(json.dumps(record) for record in records) + "\n")
        scope_fd, control_fd = subject._r_directory(str(scope)), subject._r_directory(str(control))
        plan: dict[str, Any] = {key: None for key in subject.PLAN_KEYS}
        plan.update(
            nonce=nonce,
            scope=str(scope),
            scope_identity=subject._r_identity(scope_fd),
            preparation=str(control),
            preparation_identity=subject._r_identity(control_fd),
        )
        for leaf in ("lease", "result", "witness"):
            facts = (control / leaf).stat()
            plan[leaf + "_identity"] = [facts.st_dev, facts.st_ino]
        records[0].update(
            scope_identity=plan["scope_identity"],
            preparation_identity=plan["preparation_identity"],
            caller_uid=os.getuid(),
            bootstrap_sha256=subject.BOOTSTRAP_SHA256,
        )
        witness.write_text("\n".join(json.dumps(record) for record in records) + "\n")
        reader = os.open(witness, os.O_RDONLY | os.O_CLOEXEC)
        try:
            with (
                patch.object(subject, "_r_payload_hash", return_value=subject.BOOTSTRAP_SHA256),
                patch.object(subject, "_r_auth", return_value=(os.getuid(), os.getgid(), [])),
                patch.object(subject, "_r_control", return_value=reader),
                patch.object(subject.os, "fchown") as ownership,
            ):
                subject._r_finalize(plan)
            self.assertTrue(scope.is_dir())
            self.assertTrue(control.is_dir())
            self.assertEqual(os.listdir(scope_fd), [])
            self.assertEqual(os.listdir(control_fd), [])
            self.assertIn(call(ANY, 0, 0), ownership.call_args_list)
            self.assertEqual(subject._r_identity(scope_fd), plan["scope_identity"])
            self.assertEqual(subject._r_identity(control_fd), plan["preparation_identity"])
        finally:
            os.close(scope_fd)
            os.close(control_fd)

    def test_stale_user_witness_is_rejected_before_claim(self) -> None:
        witness = self.root / "witness"
        witness.touch(mode=0o600)
        witness.write_text('{"event":"closed"}\n')
        directory = subject._r_directory(str(self.root))
        plan = {
            "preparation": str(self.root),
            "preparation_identity": subject._r_identity(directory),
            "uid": os.getuid(),
            "witness_identity": [witness.stat().st_dev, witness.stat().st_ino],
        }
        os.close(directory)
        with (
            patch.object(subject.os, "fchown") as claim,
            self.assertRaisesRegex(ValueError, "start empty"),
        ):
            subject._r_control(plan, "witness", os.O_WRONLY | os.O_APPEND, claim=True)
        claim.assert_not_called()
        self.assertIn("closed", witness.read_text())

    def test_outer_finalization_order_preserves_primary_and_retains_unknown_closure(self) -> None:
        def case(kind: str) -> None:
            automation = self.root / kind
            selected = automation / "automation/src/boxferry_ci/devcontainer_lifetime.py"
            selected.parent.mkdir(parents=True)
            selected.write_bytes(Path(subject.__file__).read_bytes())
            stages: list[str] = []

            def invoke(argv: tuple[str, ...], **options: object) -> RunResult:
                phase = argv[-2]
                stages.append(phase)
                plan = json.loads(argv[-1])
                scope = Path(plan["scope"])
                preparation = Path(plan["preparation"])
                self.addCleanup(shutil.rmtree, scope, ignore_errors=True)
                self.addCleanup(shutil.rmtree, preparation, ignore_errors=True)
                if phase == "qualify":
                    (scope / "store").mkdir()
                    (scope / "store/layer").write_text("owned")
                    report = {
                        "status": "success" if kind == "normal" else "failure",
                        "cleanup": "success",
                        "cause": "original inner timeout",
                    }
                    Path(plan["result"]).write_text(json.dumps(report))
                    events = [
                        {"nonce": plan["nonce"], "event": "pid1-bound", "pid": 123, "start": "7"},
                        {
                            "nonce": plan["nonce"],
                            "event": "closed" if kind != "unknown" else "uncertain",
                            "namespace_closed": kind != "unknown",
                            "supervisor_closed": True,
                        },
                    ]
                    (preparation / "witness").write_text(
                        "\n".join(json.dumps(event) for event in events) + "\n"
                    )
                    if kind != "normal":
                        hook = options["on_abort"]
                        assert callable(hook)
                        hook(AbortContext("outer", "infrastructure", "outer original timeout"))
                        reader = os.open(preparation / "lease", os.O_RDONLY | os.O_NONBLOCK)
                        try:
                            self.assertEqual(
                                os.read(reader, 1), b"", "lease must close before canonical cleanup"
                            )
                        finally:
                            os.close(reader)
                else:
                    self.assertEqual(stages, ["qualify", "finalize"])
                    if kind == "normal":
                        for leaf in scope.iterdir():
                            if leaf.is_dir():
                                shutil.rmtree(leaf)
                            else:
                                leaf.unlink()
                        for leaf in preparation.iterdir():
                            leaf.unlink()
                failed = kind != "normal"
                return RunResult(
                    "failed" if failed else "success",
                    1 if failed else 0,
                    "",
                    "",
                    0,
                    Diagnostic(
                        str(options["stage"]),
                        "original outer failure"
                        if phase == "qualify"
                        else "anchored finalizer timeout",
                        "fixture",
                        "fixture",
                        "inspect",
                    ),
                    argv,
                )

            def root_witness(fd: int, nonce: str) -> list[dict[str, Any]]:
                records = [
                    json.loads(line) for line in os.pread(fd, subject.MAX_WITNESS, 0).splitlines()
                ]
                self.assertTrue(all(record["nonce"] == nonce for record in records))
                return records

            with (
                patch.object(subject, "run", side_effect=invoke),
                patch.object(subject, "_witness", side_effect=root_witness),
            ):
                report = subject.execute(
                    {"work": self.root, "automation": automation, "local_sources": None}
                )
            if kind == "normal":
                self.assertEqual(report["status"], "success", report)
                self.assertTrue(report["namespace_closed"])
                self.assertEqual(report["directory_cleanup"], "success")
            else:
                self.assertEqual(report["status"], "failure", report)
                self.assertEqual(report["cause"], "original inner timeout")
                self.assertIn("retained_private_directory", report)
                self.assertIn("cleanup_cause", report)
            self.assertEqual(stages, ["qualify"] if kind == "unknown" else ["qualify", "finalize"])

        for kind in ("normal", "unknown", "cleanup-failure"):
            with self.subTest(kind=kind):
                case(kind)

    def test_finalizer_requires_closed_witness_before_touching_scope(self) -> None:
        plan: dict[str, Any] = {key: None for key in subject.PLAN_KEYS}
        plan["nonce"] = "a" * 32
        with (
            patch.object(subject, "_r_auth", return_value=(1000, 1000, [])),
            patch.object(subject, "_r_control", return_value=20),
            patch.object(subject.os, "read", return_value=b""),
            patch.object(subject.os, "close"),
            patch.object(subject, "_r_directory") as directory,
        ):
            with self.assertRaisesRegex(ValueError, "proved exact namespace"):
                subject._r_finalize(plan)
            directory.assert_not_called()


if __name__ == "__main__":
    unittest.main()
