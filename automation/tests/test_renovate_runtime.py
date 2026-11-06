"""Bounded stopped-image acquisition, cache identity, and cleanup behavior."""

from __future__ import annotations

import errno
import io
import json
import os
import signal
import sys
import tempfile
import time
import traceback
import unittest
from collections.abc import Sequence
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
from boxferry_ci import renovate_runtime as runtime
from boxferry_ci.renovate_runtime import RenovateRuntimeError, prepare_runtime, run_command

IMAGE = "ghcr.io/renovatebot/renovate:44.139.0@sha256:" + "a" * 64
IDENTITY = "ghcr.io/renovatebot/renovate@sha256:" + "a" * 64


class FakeEngine:
    def __init__(self, *, failure: str | None = None, version: str = "44.139.0") -> None:
        self.calls: list[list[str]] = []
        self.failure = failure
        self.version = version

    def __call__(self, command: Sequence[str], timeout: float) -> str:
        parts = list(command)
        self.calls.append(parts)
        if not 0 < timeout <= 300:
            raise AssertionError("unbounded runtime command")
        operation = "node" if parts[0] == "node" else parts[1]
        if operation == self.failure:
            raise RenovateRuntimeError("injected failure")
        if operation == "image":
            return json.dumps([IDENTITY])
        if operation == "create":
            return "c" * 64
        if operation == "cp":
            destination = Path(parts[-1])
            (destination / "package.json").write_text(json.dumps({"version": self.version}))
        if operation == "node":
            return "native-re2-available"
        return ""


class RenovateRuntimeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.pin = self.root / "runtime.json"
        self.pin.write_text(json.dumps({"image": IMAGE}))
        self.destination = self.root / "target/tools/renovate"

    def test_explicit_engine_immutable_identity_and_stopped_copy(self) -> None:
        for engine in ("podman", "docker"):
            with self.subTest(engine=engine):
                fake = FakeEngine()
                target = self.destination / engine
                result = prepare_runtime(self.pin, target, engine=engine, command=fake)
                self.assertEqual(result, target.absolute())
                self.assertEqual((target / "source-image.txt").read_text(), IMAGE + "\n")
                create = next(
                    call for call in fake.calls if call[0] == engine and call[1] == "create"
                )
                self.assertEqual(create[-3:], ["--network", "none", IMAGE])
                self.assertIn("--name", create)
                self.assertTrue(any(call[1:3] == ["image", "inspect"] for call in fake.calls))
                self.assertEqual(fake.calls[-1][1:3], ["rm", "--force"])
                self.assertFalse(any(call[1] in {"run", "start", "build"} for call in fake.calls))

    def test_verified_cache_hit_uses_no_container_engine(self) -> None:
        first = FakeEngine()
        prepare_runtime(self.pin, self.destination, engine="podman", command=first)
        second = FakeEngine()
        prepare_runtime(self.pin, self.destination, engine="podman", command=second)
        self.assertEqual(len(second.calls), 1)
        self.assertEqual(second.calls[0][0], "node")

    def test_damaged_managed_cache_is_reacquired(self) -> None:
        prepare_runtime(self.pin, self.destination, engine="podman", command=FakeEngine())
        (self.destination / "package.json").write_text('{"version":"wrong"}')
        fake = FakeEngine()
        prepare_runtime(self.pin, self.destination, engine="podman", command=fake)
        self.assertTrue(any(call[1] == "pull" for call in fake.calls))

    def test_failures_after_create_clean_up_stopped_container(self) -> None:
        for failure in ("cp", "node"):
            with self.subTest(failure=failure):
                fake = FakeEngine(failure=failure)
                with self.assertRaises(RenovateRuntimeError):
                    prepare_runtime(self.pin, self.destination, engine="podman", command=fake)
                self.assertEqual(fake.calls[-1][1:3], ["rm", "--force"])
                self.assertFalse(self.destination.exists())
                self.assertFalse(list(self.destination.parent.glob(".renovate-extract-*")))

    def test_version_or_image_identity_mismatch_blocks_installation(self) -> None:
        fake = FakeEngine(version="99.0.0")
        with self.assertRaisesRegex(RenovateRuntimeError, "version differs"):
            prepare_runtime(self.pin, self.destination, engine="podman", command=fake)
        self.assertEqual(fake.calls[-1][1], "rm")

        def wrong_identity(command: Sequence[str], timeout: float) -> str:
            return "[]" if command[1] == "image" else FakeEngine()(command, timeout)

        with self.assertRaisesRegex(RenovateRuntimeError, "OCI image identity"):
            prepare_runtime(self.pin, self.destination, engine="podman", command=wrong_identity)

    def test_cleanup_failure_is_not_success(self) -> None:
        with self.assertRaises(RenovateRuntimeError):
            prepare_runtime(
                self.pin, self.destination, engine="podman", command=FakeEngine(failure="rm")
            )
        self.assertFalse(self.destination.exists())

    def test_unknown_engine_and_mutable_pin_are_rejected(self) -> None:
        with self.assertRaisesRegex(RenovateRuntimeError, "explicitly"):
            prepare_runtime(self.pin, self.destination, engine="auto", command=FakeEngine())
        for image in (
            "ghcr.io/renovatebot/renovate:latest",
            "someone/renovate:1.2.3@sha256:" + "a" * 64,
        ):
            self.pin.write_text(json.dumps({"image": image}))
            with self.assertRaises(RenovateRuntimeError):
                prepare_runtime(self.pin, self.destination, engine="podman", command=FakeEngine())

    def test_output_and_time_limits_are_enforced_for_real_subprocesses(self) -> None:
        self.assertEqual(
            run_command([sys.executable, "-c", "print('bounded')"], 5).strip(), "bounded"
        )
        for stream in ("stdout", "stderr"):
            with (
                self.subTest(stream=stream),
                self.assertRaisesRegex(RenovateRuntimeError, "byte bound"),
            ):
                run_command([sys.executable, "-c", f"import sys; sys.{stream}.write('x'*70000)"], 5)
        with self.assertRaisesRegex(RenovateRuntimeError, "time bound"):
            run_command([sys.executable, "-c", "import time; time.sleep(5)"], 0.01)

    def test_real_command_failures_publish_only_closed_categories_and_status(self) -> None:
        canaries = (
            "credential-canary-592",
            "program-canary-592",
            "argument-canary-592",
            "environment-canary-592",
            "https://user:password-canary-592@example.invalid/private",
        )
        for phrase, reason in (
            ("No space left on device", "storage"),
            ("Disk quota exceeded", "storage"),
            ("authentication required", "authentication"),
            ("unauthorized", "authentication"),
            ("requested access to the resource is denied", "authentication"),
            ("Permission denied", "permission"),
            ("Operation not permitted", "permission"),
            ("network is unreachable", "network"),
            ("connection refused", "network"),
            ("could not resolve host", "network"),
            ("no such host", "network"),
            ("connection reset by peer", "network"),
            ("connection timed out", "network"),
            ("temporary failure in name resolution", "network"),
            ("i/o timeout", "network"),
            ("tls handshake timeout", "network"),
            ("executable file not found in $PATH", "missing-runtime"),
            ("command not found", "missing-runtime"),
            ("unrecognized diagnostic", "unknown"),
        ):
            for stream in ("stdout", "stderr"):
                with self.subTest(reason=reason, stream=stream, phrase=phrase):
                    code = (
                        "import os,sys; "
                        f"sys.{stream}.write({phrase!r}+{str(canaries)!r}); "
                        "sys.stderr.write(os.environ['RUNTIME592_CANARY']); sys.exit(23)"
                    )
                    with (
                        patch.dict(os.environ, {"RUNTIME592_CANARY": canaries[3]}),
                        self.assertRaises(RenovateRuntimeError) as caught,
                    ):
                        run_command([sys.executable, "-c", code, canaries[2]], 5)
                    self.assertEqual(
                        str(caught.exception),
                        f"runtime operation unknown failed (exit status 23; reason {reason})",
                    )
                    rendered = "".join(traceback.format_exception(caught.exception))
                    for canary in canaries:
                        self.assertNotIn(canary, rendered)
                    self.assertIsNone(caught.exception.__cause__)
                    self.assertTrue(caught.exception.__suppress_context__)

    def test_real_spawn_failures_and_injected_exceptions_do_not_publish_paths_or_chains(
        self,
    ) -> None:
        unavailable = self.root / "missing-credential-canary-592"
        forbidden = self.root / "permission-credential-canary-592"
        forbidden.write_text("inert fixture")
        forbidden.chmod(0o600)
        for executable, reason in ((unavailable, "missing-runtime"), (forbidden, "permission")):
            with self.subTest(reason=reason), self.assertRaises(RenovateRuntimeError) as caught:
                run_command([str(executable), "argument-canary-592"], 5)
            self.assertIn(f"reason {reason}", str(caught.exception))
            self.assertIn("exit status unavailable", str(caught.exception))
            self.assertNotIn("canary-592", "".join(traceback.format_exception(caught.exception)))
            self.assertIsNone(caught.exception.__cause__)
        for fault in (
            ValueError("https://user:credential-canary-592@example.invalid/secret"),
            RuntimeError("program-canary-592"),
            OSError(errno.EIO, "environment-canary-592"),
        ):

            def unavailable_command(
                _argv: Sequence[str], _timeout: float, fault: Exception = fault
            ) -> str:
                raise fault

            with (
                self.subTest(fault=type(fault).__name__),
                self.assertRaises(RenovateRuntimeError) as caught,
            ):
                prepare_runtime(
                    self.pin, self.destination, engine="podman", command=unavailable_command
                )
            self.assertEqual(
                str(caught.exception),
                "runtime operation image-pull failed (exit status unavailable; reason unknown)",
            )
            self.assertNotIn("canary-592", "".join(traceback.format_exception(caught.exception)))
            self.assertIsNone(caught.exception.__cause__)

    def test_real_failure_is_bound_to_each_fixed_acquisition_operation(self) -> None:
        for failed, operation in (
            ("pull", "image-pull"),
            ("image", "digest-inspect"),
            ("create", "stopped-create"),
            ("cp", "copy"),
            ("rm", "cleanup"),
            ("node", "native-RE2"),
        ):
            fake = FakeEngine()

            def command(
                argv: Sequence[str], timeout: float, failed: str = failed, fake: FakeEngine = fake
            ) -> str:
                current = "node" if argv[0] == "node" else argv[1]
                if current == failed:
                    fake.calls.append(list(argv))
                    return run_command(
                        [
                            sys.executable,
                            "-c",
                            "import sys;sys.stderr.write("
                            "'Permission denied credential-canary-592');"
                            "sys.exit(19)",
                        ],
                        timeout,
                    )
                return fake(argv, timeout)

            with (
                self.subTest(operation=operation),
                self.assertRaises(RenovateRuntimeError) as caught,
            ):
                prepare_runtime(self.pin, self.destination, engine="podman", command=command)
            self.assertTrue(
                str(caught.exception).startswith(
                    f"runtime operation {operation} failed (exit status 19; reason permission)"
                )
            )
            self.assertNotIn("canary-592", "".join(traceback.format_exception(caught.exception)))
            if failed in ("create", "cp", "rm", "node"):
                self.assertEqual(fake.calls[-1][1:3], ["rm", "--force"])
            self.assertFalse(self.destination.exists())
            self.assertFalse(list(self.destination.parent.glob(".renovate-extract-*")))

    def test_real_dual_failure_retains_acquisition_before_cleanup_and_blocks_install(self) -> None:
        fake = FakeEngine()

        def command(argv: Sequence[str], timeout: float) -> str:
            if argv[1] in ("cp", "rm"):
                fake.calls.append(list(argv))
                phrase, status = (
                    ("No space left on device", 17)
                    if argv[1] == "cp"
                    else ("Permission denied", 29)
                )
                return run_command(
                    [
                        sys.executable,
                        "-c",
                        f"import sys;sys.stderr.write({phrase!r}+' credential-canary-592');"
                        f"sys.exit({status})",
                    ],
                    timeout,
                )
            return fake(argv, timeout)

        with self.assertRaises(RenovateRuntimeError) as caught:
            prepare_runtime(self.pin, self.destination, engine="podman", command=command)
        self.assertEqual(
            str(caught.exception),
            "runtime operation copy failed (exit status 17; reason storage); "
            "cleanup failure: runtime operation cleanup failed "
            "(exit status 29; reason permission)",
        )
        self.assertNotIn("canary-592", "".join(traceback.format_exception(caught.exception)))
        self.assertEqual(
            [entry[1] for entry in fake.calls], ["pull", "image", "create", "cp", "rm"]
        )
        self.assertFalse(self.destination.exists())
        self.assertFalse(list(self.destination.parent.glob(".renovate-extract-*")))

    def test_scratch_cleanup_failure_keeps_first_error_and_hides_exception_paths(self) -> None:
        fake = FakeEngine()

        def command(argv: Sequence[str], timeout: float) -> str:
            if argv[1] == "cp":
                fake.calls.append(list(argv))
                return run_command(
                    [
                        sys.executable,
                        "-c",
                        "import sys;sys.stderr.write('No space left on device');sys.exit(27)",
                    ],
                    timeout,
                )
            return fake(argv, timeout)

        with (
            patch.object(
                runtime.shutil,
                "rmtree",
                side_effect=PermissionError(
                    errno.EACCES, "credential-canary-592", "https://secret-canary-592.invalid/path"
                ),
            ),
            self.assertRaises(RenovateRuntimeError) as caught,
        ):
            prepare_runtime(self.pin, self.destination, engine="podman", command=command)
        self.assertEqual(
            str(caught.exception),
            "runtime operation copy failed (exit status 27; reason storage); "
            "cleanup failure: runtime operation cleanup failed "
            "(exit status unavailable; reason permission)",
        )
        self.assertNotIn("canary-592", "".join(traceback.format_exception(caught.exception)))
        self.assertEqual(fake.calls[-1][1:3], ["rm", "--force"])
        self.assertFalse(self.destination.exists())

    def test_cleanup_retry_success_still_blocks_install_after_first_cleanup_failure(self) -> None:
        fake = FakeEngine()
        attempts = [0]

        def command(argv: Sequence[str], timeout: float) -> str:
            if argv[0] != "node" and argv[1] == "rm":
                attempts[0] += 1
                if attempts[0] == 1:
                    fake.calls.append(list(argv))
                    return run_command(
                        [
                            sys.executable,
                            "-c",
                            "import sys;sys.stderr.write('Permission denied');sys.exit(13)",
                        ],
                        timeout,
                    )
            return fake(argv, timeout)

        with self.assertRaises(RenovateRuntimeError) as caught:
            prepare_runtime(self.pin, self.destination, engine="podman", command=command)
        self.assertEqual(
            str(caught.exception),
            "runtime operation cleanup failed (exit status 13; reason permission)",
        )
        self.assertEqual(attempts[0], 2)
        self.assertFalse(self.destination.exists())

    def test_aggregate_stream_limit_and_wait_deadline_keep_safe_status(self) -> None:
        half = runtime.MAX_OUTPUT // 2
        code = f"import sys;sys.stdout.write('x'*{half});sys.stderr.write('y'*{half})"
        self.assertEqual(len(run_command([sys.executable, "-c", code], 5)), half)
        for body, reason, timeout in (
            (code + ";sys.stderr.write('z')", "output byte bound", 5),
            ("import os,time;os.close(1);os.close(2);time.sleep(30)", "time bound", 0.2),
            ("import os;os.write(1,b'\\xff')", "invalid-output", 5),
        ):
            start = time.monotonic()
            with self.subTest(reason=reason), self.assertRaises(RenovateRuntimeError) as caught:
                run_command([sys.executable, "-c", body], timeout)
            self.assertLess(time.monotonic() - start, 3)
            self.assertRegex(str(caught.exception), rf"exit status -?[0-9]+; reason {reason}")

    def test_inherited_output_drain_is_bounded_and_exact_child_group_is_killed(self) -> None:
        pid_file = self.root / "drain-child.pid"
        code = (
            "import os,pathlib,time;pid=os.fork();"
            'exec("if pid == 0:\\n pathlib.Path('
            + repr(str(pid_file))
            + ").write_text(str(os.getpid()))"
            '\\n time.sleep(30)")'
        )
        started = time.monotonic()
        with self.assertRaisesRegex(RenovateRuntimeError, "time bound"):
            run_command([sys.executable, "-c", code], 0.5)
        self.assertLess(time.monotonic() - started, 3)
        child = int(pid_file.read_text())

        def stop_child() -> None:
            try:
                os.kill(child, signal.SIGKILL)
            except ProcessLookupError:
                pass

        self.addCleanup(stop_child)
        # Only the exact controlled child's state is observed; a killed orphan may be a zombie.
        state = Path(f"/proc/{child}/stat")
        until = time.monotonic() + 1
        while state.exists() and state.read_text().rsplit(")", 1)[1].split()[0] != "Z":
            self.assertLess(time.monotonic(), until, "descendant outlived process-group cleanup")
            time.sleep(0.01)

    def test_cli_emits_safe_failure_and_keeps_success_path_output(self) -> None:
        error = None
        try:
            run_command(
                [
                    sys.executable,
                    "-c",
                    "import sys;sys.stderr.write('Unauthorized secret-canary-592');sys.exit(9)",
                ],
                5,
            )
        except RenovateRuntimeError as observed:
            error = observed
        self.assertIsNotNone(error)
        arguments = [
            "runtime",
            "--engine",
            "podman",
            "--pin",
            str(self.pin),
            "--destination",
            str(self.destination),
        ]
        output, errors = io.StringIO(), io.StringIO()
        with (
            patch.object(sys, "argv", arguments),
            patch.object(runtime, "prepare_runtime", side_effect=error),
            redirect_stdout(output),
            redirect_stderr(errors),
        ):
            self.assertEqual(runtime.main(), 1)
        self.assertEqual(output.getvalue(), "")
        self.assertNotIn("canary-592", errors.getvalue())
        self.assertIn("exit status 9; reason authentication", errors.getvalue())
        output, errors = io.StringIO(), io.StringIO()
        with (
            patch.object(sys, "argv", arguments),
            patch.object(runtime, "prepare_runtime", return_value=self.destination),
            redirect_stdout(output),
            redirect_stderr(errors),
        ):
            self.assertEqual(runtime.main(), 0)
        self.assertEqual(output.getvalue(), str(self.destination) + "\n")
        self.assertEqual(errors.getvalue(), "")

    def test_invalid_create_output_still_cleans_owned_container_name(self) -> None:
        fake = FakeEngine()

        def invalid_id(command: Sequence[str], timeout: float) -> str:
            value = fake(command, timeout)
            return "invalid" if command[1] == "create" else value

        with self.assertRaisesRegex(RenovateRuntimeError, "invalid stopped-container"):
            prepare_runtime(self.pin, self.destination, engine="podman", command=invalid_id)
        self.assertEqual(fake.calls[-1][1:3], ["rm", "--force"])
        self.assertTrue(fake.calls[-1][-1].startswith("boxferry-renovate-"))

    def test_unrelated_existing_destination_is_preserved(self) -> None:
        self.destination.mkdir(parents=True)
        existing = self.destination / "important.txt"
        existing.write_text("preserve")
        with self.assertRaisesRegex(RenovateRuntimeError, "not a managed"):
            prepare_runtime(self.pin, self.destination, engine="podman", command=FakeEngine())
        self.assertEqual(existing.read_text(), "preserve")


if __name__ == "__main__":
    unittest.main()
