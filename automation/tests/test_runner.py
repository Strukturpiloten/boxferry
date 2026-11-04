from __future__ import annotations

import json
import os
import signal
import sys
import tempfile
import time
import unittest
from pathlib import Path

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
            except FileNotFoundError:
                state = None
                break
            if state == "Z":
                break
            time.sleep(0.005)
        # Reparented zombies await the container's init; they execute no work.
        self.assertIn(state, (None, "Z"))

    def test_invalid_bounds_and_argv_are_rejected(self) -> None:
        for timeout in (0, -1, 7201):
            with self.subTest(timeout=timeout), self.assertRaises(ContractError):
                run(("true",), cwd=self.root, stage="bounds", timeout_seconds=timeout)
        with self.assertRaises(ContractError):
            run((), cwd=self.root, stage="bounds")


if __name__ == "__main__":
    unittest.main()
