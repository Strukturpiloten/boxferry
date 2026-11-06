from __future__ import annotations

import hashlib
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from typing import Any, cast
from unittest.mock import patch

from boxferry_ci import runner as subject
from boxferry_ci.contracts import ContractError
from boxferry_ci.runner import RunCancelled, run


class Runner(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def test_resource_monitor_failure_cleans_the_held_process_group(self) -> None:
        pid = self.root / "child.pid"
        program = (
            "import os,time; from pathlib import Path; "
            "Path('child.pid').write_text(str(os.getpid())); time.sleep(30)"
        )

        def monitor() -> None:
            if pid.exists():
                raise ContractError("fixture disk reserve exhausted")

        with self.assertRaisesRegex(ContractError, "fixture disk reserve"):
            run(
                (sys.executable, "-c", program),
                cwd=self.root,
                stage="monitor",
                timeout_seconds=5,
                monitor=monitor,
            )
        with self.assertRaises(ProcessLookupError):
            os.kill(int(pid.read_text()), 0)

    def test_both_streams_and_original_exit_failure_are_preserved(self) -> None:
        result = run(
            (
                sys.executable,
                "-c",
                "import sys; print('original output'); "
                "print('original diagnostic', file=sys.stderr); sys.exit(7)",
            ),
            cwd=self.root,
            stage="fixture check",
        )
        self.assertEqual(result.status, "failed")
        self.assertEqual(result.returncode, 7)
        self.assertEqual(result.stdout, "original output\n")
        self.assertEqual(result.stderr, "original diagnostic\n")
        self.assertEqual(result.diagnostic.stage, "fixture check")
        self.assertTrue(result.diagnostic.next_action)

    def test_secrets_are_redacted_without_printing_environment(self) -> None:
        secret = "fixture-long-private-value"
        result = run(
            (
                sys.executable,
                "-c",
                "import os,sys; print(os.environ['TEST_TOKEN']); "
                "print(os.environ['TEST_TOKEN'],file=sys.stderr)",
            ),
            cwd=self.root,
            stage="redaction",
            environment={**os.environ, "TEST_TOKEN": secret},
            affected_input=secret,
            evidence_link=secret,
        )
        self.assertEqual(result.status, "success")
        encoded = json.dumps(result.to_dict())
        self.assertNotIn(secret, encoded)
        self.assertNotIn("PATH", encoded)
        self.assertIn("[REDACTED]", encoded)

    def test_secret_arguments_and_checkout_paths_are_redacted(self) -> None:
        env_secret = "fixture-environment-secret"
        explicit_secret = "fixture-explicit-secret"
        argument_secret = "fixture-unregistered-token"
        credential_path = self.root / "private" / "credential.pem"
        result = run(
            (
                sys.executable,
                "-c",
                "import sys; print(sys.argv[1:]); sys.exit(1)",
                env_secret,
                explicit_secret,
                "--token",
                argument_secret,
                "--password=fixture-unregistered-password",
                str(credential_path),
                str(self.root / "public-input.toml"),
            ),
            cwd=self.root,
            stage="secret arguments",
            environment={**os.environ, "TEST_TOKEN": env_secret},
            secrets=(explicit_secret, str(credential_path)),
        )
        self.assertEqual(result.status, "failed")
        self.assertEqual(result.argv[0], Path(sys.executable).name)
        self.assertEqual(result.argv[-1], "<repository>/public-input.toml")
        encoded = json.dumps(result.to_dict())
        for private in (
            env_secret,
            explicit_secret,
            argument_secret,
            "fixture-unregistered-password",
            str(credential_path),
        ):
            self.assertNotIn(private, encoded)
        self.assertNotIn(str(self.root), json.dumps(result.argv))
        self.assertEqual(
            result.argv[5:9],
            (
                "--token",
                "[REDACTED]",
                "--password=[REDACTED]",
                "[REDACTED]",
            ),
        )

    def test_timeout_has_distinct_infrastructure_status(self) -> None:
        result = run(
            (sys.executable, "-c", "import time; time.sleep(10)"),
            cwd=self.root,
            stage="timeout",
            timeout_seconds=0.1,
        )
        self.assertEqual(result.status, "infrastructure")
        self.assertIn("timed out", result.diagnostic.cause)
        self.assertLess(result.duration_seconds, 2)

    def test_output_flood_is_bounded(self) -> None:
        result = run(
            (sys.executable, "-c", "import sys; sys.stdout.write('x'*100000)"),
            cwd=self.root,
            stage="output",
            max_output_bytes=1024,
        )
        self.assertEqual(result.status, "infrastructure")
        self.assertLessEqual(len(result.stdout), 1024)

    def test_normal_build_artifacts_are_not_limited_by_log_capture(self) -> None:
        artifact = self.root / "large-artifact.bin"
        result = run(
            (
                sys.executable,
                "-c",
                f"from pathlib import Path; Path({str(artifact)!r}).write_bytes(b'x'*3000000); "
                "print('artifact complete')",
            ),
            cwd=self.root,
            stage="build artifact",
            max_output_bytes=1024,
        )
        self.assertEqual(result.status, "success")
        self.assertEqual(artifact.stat().st_size, 3_000_000)
        self.assertEqual(result.stdout, "artifact complete\n")

    def test_missing_executable_is_infrastructure_problem(self) -> None:
        result = run(("missing-boxferry-test-tool",), cwd=self.root, stage="tool setup")
        self.assertEqual(result.status, "infrastructure")
        self.assertIsNone(result.returncode)
        self.assertIn("could not start", result.diagnostic.cause)

    def test_cancellation_cleans_owned_child(self) -> None:
        pidfile = self.root / "child.pid"
        code = (
            "import os,time,signal; "
            f"open({str(pidfile)!r},'w').write(str(os.getpid())); "
            "signal.signal(signal.SIGTERM,signal.SIG_IGN); time.sleep(10)"
        )

        def cancel(_signal: int, _frame: object) -> None:
            raise RunCancelled("fixture cancellation")

        previous = signal.signal(signal.SIGALRM, cancel)
        signal.setitimer(signal.ITIMER_REAL, 0.1)
        try:
            result = run((sys.executable, "-c", code), cwd=self.root, stage="cancel")
        finally:
            signal.setitimer(signal.ITIMER_REAL, 0)
            signal.signal(signal.SIGALRM, previous)
        self.assertEqual(result.status, "cancelled")
        pid = int(pidfile.read_text())
        with self.assertRaises(ProcessLookupError):
            os.kill(pid, 0)

    def test_exited_leader_does_not_leave_a_running_descendant(self) -> None:
        pidfile = self.root / "descendant.pid"
        child = "import signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); time.sleep(10)"
        code = (
            "import subprocess,sys; "
            f"p=subprocess.Popen([sys.executable,'-c',{child!r}]); "
            f"open({str(pidfile)!r},'w').write(str(p.pid))"
        )
        result = run((sys.executable, "-c", code), cwd=self.root, stage="child cleanup")
        self.assertEqual(result.status, "success")
        pid = int(pidfile.read_text())
        stat = Path(f"/proc/{pid}/stat")
        state: str | None = None
        for _ in range(100):
            try:
                state = stat.read_text().split()[2]
            except (FileNotFoundError, ProcessLookupError):
                state = None
                break
            if state == "Z":
                break
            time.sleep(0.005)
        # Reparented zombies await the container's init; they execute no work.
        self.assertIn(state, (None, "Z"))

    def test_abort_hook_persists_original_timeout_output_monitor_and_exit_before_cleanup(
        self,
    ) -> None:
        original_cleanup = subject._cleanup
        for case in ("timeout", "output", "monitor", "exit"):
            with self.subTest(case=case):
                marker = self.root / (case + ".json")
                events: list[str] = []

                def abort(
                    context: subject.AbortContext,
                    *,
                    marker: Path = marker,
                    events: list[str] = events,
                ) -> None:
                    marker.write_text(
                        json.dumps(
                            {
                                "stage": context.stage,
                                "status": context.status,
                                "cause": context.cause,
                            }
                        )
                    )
                    events.append("abort")

                def cleanup(
                    process: subprocess.Popen[bytes],
                    grace: float,
                    *,
                    marker: Path = marker,
                    events: list[str] = events,
                ) -> None:
                    self.assertTrue(marker.is_file(), "handoff must precede ALL cleanup waits")
                    events.append("cleanup")
                    original_cleanup(process, grace)

                def monitor() -> None:
                    raise ContractError("original monitor reserve failure")

                program = {
                    "timeout": "import time; time.sleep(20)",
                    "output": "import sys; sys.stdout.write('x'*10000);sys.stdout.flush()",
                    "monitor": "import time; time.sleep(20)",
                    "exit": "raise SystemExit(7)",
                }[case]
                with patch.object(subject, "_cleanup", cleanup):
                    if case == "monitor":
                        with self.assertRaisesRegex(ContractError, "original monitor reserve"):
                            run(
                                (sys.executable, "-c", program),
                                cwd=self.root,
                                stage=case,
                                monitor=monitor,
                                on_abort=abort,
                            )
                    else:
                        result = run(
                            (sys.executable, "-c", program),
                            cwd=self.root,
                            stage=case,
                            timeout_seconds=0.1 if case == "timeout" else 2,
                            max_output_bytes=128,
                            on_abort=abort,
                        )
                        self.assertNotEqual(result.status, "success")
                self.assertEqual(events.count("abort"), 1)
                self.assertEqual(events[0], "abort")
                preserved = json.loads(marker.read_text())
                self.assertEqual(preserved["stage"], case)
                self.assertIn(
                    {
                        "timeout": "timed out",
                        "output": "output limit",
                        "monitor": "original monitor reserve",
                        "exit": "exited 7",
                    }[case],
                    preserved["cause"],
                )

    def test_actual_sigterm_and_sigint_deliver_abort_before_owned_cleanup(self) -> None:
        original_cleanup = subject._cleanup
        for number in (signal.SIGTERM, signal.SIGINT):
            with self.subTest(number=number):
                ready = self.root / (str(number) + ".ready")
                events: list[str] = []

                def cancel(_number: int, _frame: object) -> None:
                    raise RunCancelled("actual fixture signal")

                def monitor(*, ready: Path = ready, number: int = number) -> None:
                    if ready.exists():
                        os.kill(os.getpid(), number)

                def abort(context: subject.AbortContext, *, events: list[str] = events) -> None:
                    self.assertEqual(context.status, "cancelled")
                    events.append("abort")

                def cleanup(
                    process: subprocess.Popen[bytes], grace: float, *, events: list[str] = events
                ) -> None:
                    self.assertEqual(events[0], "abort")
                    events.append("cleanup")
                    original_cleanup(process, grace)

                previous = signal.signal(number, cancel)
                try:
                    with patch.object(subject, "_cleanup", cleanup):
                        result = run(
                            (
                                sys.executable,
                                "-c",
                                "from pathlib import Path;import time;"
                                f"Path({str(ready)!r}).touch();"
                                "time.sleep(20)",
                            ),
                            cwd=self.root,
                            stage="signal",
                            timeout_seconds=3,
                            monitor=monitor,
                            on_abort=abort,
                        )
                finally:
                    signal.signal(number, previous)
                self.assertEqual(result.status, "cancelled")
                self.assertEqual(events.count("abort"), 1)

    def test_abort_hook_failure_never_skips_cleanup_or_erases_primary_and_redacts(self) -> None:
        secret = "fixture-hook-private-value"
        calls: list[str] = []
        original_cleanup = subject._cleanup

        def abort(_context: subject.AbortContext) -> None:
            calls.append("abort")
            raise ValueError(secret)

        def cleanup(process: subprocess.Popen[bytes], grace: float) -> None:
            calls.append("cleanup")
            original_cleanup(process, grace)

        with patch.object(subject, "_cleanup", cleanup):
            result = run(
                (sys.executable, "-c", "import time;time.sleep(20)"),
                cwd=self.root,
                stage="hook-failure",
                timeout_seconds=0.05,
                environment={**os.environ, "TEST_TOKEN": secret},
                on_abort=abort,
            )
        self.assertEqual(calls[0], "abort")
        self.assertIn("cleanup", calls)
        self.assertEqual(calls.count("abort"), 1)
        self.assertIn("timed out", result.diagnostic.cause)
        self.assertEqual(result.status, "infrastructure")
        self.assertIn("[REDACTED]", result.abort_hook_failure)
        self.assertNotIn(secret, json.dumps(result.to_dict()))

    def test_unprintable_abort_exception_still_reaches_cleanup_and_preserves_timeout(self) -> None:
        class Unprintable(Exception):
            def __str__(self) -> str:
                raise RuntimeError("must not escape callback formatting")

        def abort(_context: subject.AbortContext) -> None:
            raise Unprintable()

        original_cleanup = subject._cleanup
        with patch.object(subject, "_cleanup", wraps=original_cleanup) as cleanup:
            result = run(
                (sys.executable, "-c", "import time;time.sleep(20)"),
                cwd=self.root,
                stage="unprintable-hook",
                timeout_seconds=0.05,
                on_abort=abort,
            )
        cleanup.assert_called()
        self.assertEqual(result.status, "infrastructure")
        self.assertIn("timed out", result.diagnostic.cause)
        self.assertEqual(result.abort_hook_failure, "unprintable abort hook exception")

    def test_success_and_default_callers_do_not_gain_abort_hooks_or_fields(self) -> None:
        called: list[subject.AbortContext] = []
        result = run(
            (sys.executable, "-c", "print('same output')"),
            cwd=self.root,
            stage="success",
            on_abort=called.append,
        )
        default = run(
            (sys.executable, "-c", "print('same output')"), cwd=self.root, stage="success"
        )
        self.assertEqual(called, [])
        for value in (result, default):
            self.assertEqual(value.status, "success")
            self.assertEqual(value.stdout, "same output\n")
            self.assertEqual(value.returncode, 0)
            self.assertNotIn("abort_hook_failure", value.to_dict())

    def test_invalid_bounds_and_argv_are_rejected(self) -> None:
        for timeout in (0, -1, 7201):
            with self.subTest(timeout=timeout), self.assertRaises(ContractError):
                run(("true",), cwd=self.root, stage="bounds", timeout_seconds=timeout)
        with self.assertRaises(ContractError):
            run((), cwd=self.root, stage="bounds")

    def test_closed_opaque_modes_parse_only_private_success_and_publish_no_raw_channel(
        self,
    ) -> None:
        from boxferry_ci import devcontainer_validation as validation

        original = subprocess.Popen
        fixture = (
            "import sys;sys.stdout.write('{\"safe\":1}');sys.stderr.write('private error bytes')"
        )
        for mode in subject.OpaqueObservation:
            private: list[bytes] = []

            def spawn(_argv: tuple[str, ...], **options: object) -> subprocess.Popen[bytes]:
                return cast(
                    subprocess.Popen[bytes],
                    original((sys.executable, "-c", fixture), **cast(dict[str, Any], options)),
                )

            argv = (
                (
                    "/usr/bin/python3",
                    "-I",
                    "-S",
                    "-c",
                    validation.LIVE_PROCESS_OBSERVER_PROGRAM,
                    "{}",
                )
                if mode is subject.OpaqueObservation.PROCESSES
                else ("fixture-inspect", "held")
            )
            with (
                self.subTest(mode=mode),
                patch.object(subject.subprocess, "Popen", side_effect=spawn),
            ):
                result = run(
                    argv,
                    cwd=self.root,
                    stage=mode.stage,
                    timeout_seconds=mode.timeout,
                    max_output_bytes=mode.output_cap,
                    opaque_observation=mode,
                    on_private_stdout=private.append,
                    on_abort=lambda _context: None,
                )
            self.assertEqual(private, [b'{"safe":1}'])
            self.assertIsInstance(result, subject.OpaqueRunResult)
            assert isinstance(result, subject.OpaqueRunResult)
            self.assertEqual((result.stdout, result.stderr, result.stdout_bytes), ("", "", b""))
            public = result.to_dict()["opaque_observation"]
            assert isinstance(public, dict)
            self.assertEqual(public["reason"], "completed")
            self.assertNotIn("private error bytes", repr(result))
            self.assertNotIn(validation.LIVE_PROCESS_OBSERVER_PROGRAM, repr(result))
            self.assertEqual(
                result.metrics.stdout_prefix_sha256, hashlib.sha256(private[0]).hexdigest()
            )

    def test_opaque_parser_precedes_abort_cleanup_and_cleanup_failure_discards_success(
        self,
    ) -> None:
        mode = subject.OpaqueObservation.INSPECT_BEFORE
        events: list[str] = []
        original = subprocess.Popen

        def spawn(_argv: tuple[str, ...], **options: object) -> subprocess.Popen[bytes]:
            return cast(
                subprocess.Popen[bytes],
                original((sys.executable, "-c", "print('{}')"), **cast(dict[str, Any], options)),
            )

        def parse(_value: bytes) -> None:
            events.append("parse")
            raise subject.OpaqueParseError(subject.OpaqueReason.INVALID_JSON)

        cleanup = subject._cleanup

        def cleaned(process: subprocess.Popen[bytes], grace: float) -> None:
            events.append("cleanup")
            cleanup(process, grace)

        with (
            patch.object(subject.subprocess, "Popen", side_effect=spawn),
            patch.object(subject, "_cleanup", side_effect=cleaned),
        ):
            result = run(
                ("fixture",),
                cwd=self.root,
                stage=mode.stage,
                timeout_seconds=mode.timeout,
                max_output_bytes=mode.output_cap,
                opaque_observation=mode,
                on_private_stdout=parse,
                on_abort=lambda _context: events.append("abort"),
            )
        self.assertEqual(events, ["parse", "abort", "cleanup"])
        assert isinstance(result, subject.OpaqueRunResult)
        self.assertEqual(result.metrics.reason, subject.OpaqueReason.INVALID_JSON)
        events.clear()

        def failed_cleanup(process: subprocess.Popen[bytes], grace: float) -> None:
            cleanup(process, grace)
            raise OSError("cleanup fixture")

        with (
            patch.object(subject.subprocess, "Popen", side_effect=spawn),
            patch.object(subject, "_cleanup", side_effect=failed_cleanup),
        ):
            with self.assertRaises(subject.OpaqueRunError) as caught:
                run(
                    ("fixture",),
                    cwd=self.root,
                    stage=mode.stage,
                    timeout_seconds=mode.timeout,
                    max_output_bytes=mode.output_cap,
                    opaque_observation=mode,
                    on_private_stdout=lambda _value: events.append("provisional"),
                    on_abort=lambda _ctx: None,
                )
        self.assertEqual(events, ["provisional"])
        assert caught.exception.metrics is not None
        self.assertTrue(caught.exception.metrics.cleanup_failed)

    def test_fixed_observer_exit_codes_do_not_parse_failed_streams(self) -> None:
        from boxferry_ci import devcontainer_validation as validation

        original = subprocess.Popen
        mode = subject.OpaqueObservation.PROCESSES
        expected = {
            2: subject.OpaqueReason.INVALID_REQUEST,
            3: subject.OpaqueReason.PROCESS_UNAVAILABLE,
            4: subject.OpaqueReason.PROCESS_CHANGED,
            5: subject.OpaqueReason.NAMESPACE_MISMATCH,
            6: subject.OpaqueReason.UID_MISMATCH,
            7: subject.OpaqueReason.OBSERVER_LIMIT,
            124: subject.OpaqueReason.ROOT_DEADLINE,
            9: subject.OpaqueReason.OBSERVER_FAILED,
        }
        for code, reason in expected.items():

            def spawn(
                _argv: tuple[str, ...], code: int = code, **options: object
            ) -> subprocess.Popen[bytes]:
                return cast(
                    subprocess.Popen[bytes],
                    original(
                        (
                            sys.executable,
                            "-c",
                            "import sys;print('{}');sys.exit(" + str(code) + ")",
                        ),
                        **cast(dict[str, Any], options),
                    ),
                )

            consumed: list[bytes] = []
            with (
                self.subTest(code=code),
                patch.object(subject.subprocess, "Popen", side_effect=spawn),
            ):
                result = run(
                    (
                        "/usr/bin/python3",
                        "-I",
                        "-S",
                        "-c",
                        validation.LIVE_PROCESS_OBSERVER_PROGRAM,
                        "{}",
                    ),
                    cwd=self.root,
                    stage=mode.stage,
                    timeout_seconds=mode.timeout,
                    max_output_bytes=mode.output_cap,
                    opaque_observation=mode,
                    on_private_stdout=consumed.append,
                    on_abort=lambda _failure: None,
                )
            assert isinstance(result, subject.OpaqueRunResult)
            self.assertEqual(result.returncode, code)
            self.assertEqual(result.metrics.reason, reason)
            self.assertFalse(consumed)
            self.assertEqual(result.stdout_bytes, b"")

    def test_opaque_prefix_hash_includes_detection_byte_and_never_parses_overflow(self) -> None:
        mode = subject.OpaqueObservation.INSPECT_AFTER
        original = subprocess.Popen

        def spawn(_argv: tuple[str, ...], **options: object) -> subprocess.Popen[bytes]:
            return cast(
                subprocess.Popen[bytes],
                original(
                    (sys.executable, "-c", "import sys;sys.stdout.write('q'*70000)"),
                    **cast(dict[str, Any], options),
                ),
            )

        parsed: list[bytes] = []
        with patch.object(subject.subprocess, "Popen", side_effect=spawn):
            result = run(
                ("fixture",),
                cwd=self.root,
                stage=mode.stage,
                timeout_seconds=mode.timeout,
                max_output_bytes=mode.output_cap,
                opaque_observation=mode,
                on_private_stdout=parsed.append,
            )
        self.assertFalse(parsed)
        assert isinstance(result, subject.OpaqueRunResult)
        self.assertEqual(result.metrics.reason, subject.OpaqueReason.OUTPUT_LIMIT)
        self.assertEqual(result.metrics.stdout_prefix_bytes, mode.output_cap + 1)
        self.assertEqual(
            result.metrics.stdout_prefix_sha256,
            hashlib.sha256(b"q" * (mode.output_cap + 1)).hexdigest(),
        )
        self.assertEqual(result.stdout_bytes, b"")

    def test_opaque_mode_mapping_is_closed_and_default_result_shape_remains_legacy(self) -> None:
        mode = subject.OpaqueObservation.INSPECT_BEFORE
        for extra in (
            {"stage": "unreviewed"},
            {"timeout_seconds": 3},
            {"max_output_bytes": 99},
            {"opaque_observation": "inspect-before"},
            {"on_private_stdout": None},
        ):
            values: dict[str, Any] = {
                "stage": mode.stage,
                "timeout_seconds": mode.timeout,
                "max_output_bytes": mode.output_cap,
                "opaque_observation": mode,
                "on_private_stdout": lambda _bytes: None,
            }
            values.update(extra)
            with self.subTest(extra=tuple(extra)), self.assertRaises(subject.OpaqueRunError):
                run(("not-executed",), cwd=self.root, **values)
        with self.assertRaises(ContractError):
            run(
                ("not-executed",),
                cwd=self.root,
                stage="legacy",
                on_private_stdout=lambda _bytes: None,
            )
        result = run(
            (sys.executable, "-c", "print('legacy useful stdout')"), cwd=self.root, stage="legacy"
        )
        self.assertIs(type(result), subject.RunResult)
        self.assertEqual(result.stdout, "legacy useful stdout\n")
        self.assertEqual(result.stdout_bytes, b"legacy useful stdout\n")
        self.assertEqual(
            set(result.to_dict()),
            {"status", "returncode", "stdout", "stderr", "duration_seconds", "diagnostic", "argv"},
        )


if __name__ == "__main__":
    unittest.main()
