"""Independent bounded fixtures for capture, protocol neutrality and real failure reporting."""

from __future__ import annotations

import errno
import json
import os
import signal
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest
from collections.abc import Mapping
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from boxferry_ci import test_reporting as reporting
from boxferry_ci.runner import RunResult
from boxferry_ci.test_reporting_data import Bounds, Logs, Shield, Window, read_json

SOURCE = Path(__file__).resolve().parents[1] / "src"


class ByteSafety(unittest.TestCase):
    def test_every_chunk_boundary_redacts_before_retention_and_decoding(self) -> None:
        shield = Shield(Path("/fixture/checkout"), {"TEST_TOKEN": "split-private-value"})
        original = b"prefix::error::##[error] split-private-value /fixture/checkout \xfftail"
        for boundary in range(len(original) + 1):
            with self.subTest(boundary=boundary):
                stream = shield.stream()
                safe = stream.feed(original[:boundary]) + stream.feed(original[boundary:])
                safe += stream.finish()
                self.assertNotIn(b"::", safe)
                self.assertNotIn(b"##[", safe)
                self.assertNotIn(b"split-private-value", safe)
                self.assertNotIn(b"/fixture/checkout", safe)
                self.assertIn(b"<repository>", safe)
                self.assertIn("\ufffd", safe.decode("utf-8", "replace"))

    def test_retention_is_not_a_hard_budget_and_omissions_are_exact(self) -> None:
        logs = Logs(Shield(Path("/fixture"), {}), Bounds(100, 4, 8))
        logs.add("stdout", b"abcdefghij")
        logs.add("stderr", b"klmnopqrst")
        logs.finish()
        value = logs.report()
        self.assertEqual(value["original_bytes"], 20)
        self.assertEqual(value["sanitized_bytes"], 20)
        self.assertEqual(value["omitted_bytes"], 8)
        self.assertFalse(logs.exceeded)
        window = Window(2, 3)
        window.add(b"0123456789")
        self.assertEqual(window.omitted, 5)
        self.assertEqual(window.render(), b"01\n[omitted 5 sanitized bytes]\n789")

    def test_pattern_preflight_fails_without_disclosing_values(self) -> None:
        for environment in (
            {f"SECRET_{index}": str(index) * 8 for index in range(65)},
            {"SECRET_ONE": "private" * 3000},
            {f"SECRET_{index}": str(index) + "x" * 14000 for index in range(5)},
        ):
            with self.subTest(patterns=len(environment)), self.assertRaises(ValueError):
                Shield(Path("/fixture"), environment)

    def test_maximum_pattern_overlap_and_read_chunk_are_bounded(self) -> None:
        shield = Shield(Path("/fixture"), {"TEST_SECRET": "p" * 16384})
        stream = shield.stream()
        for _ in range(3):
            safe = stream.feed(b"x" * 32768)
            self.assertLessEqual(len(stream.pending), 16383)
            self.assertLessEqual(len(safe), 32768)
        stream.finish()
        self.assertEqual(len(stream.pending), 0)
        with self.assertRaises(ValueError):
            shield.stream().feed(b"x" * 32769)

    def test_strict_result_and_handoff_bounds(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "result"
            path.write_bytes(b"x" * 100)
            with self.assertRaises(ValueError):
                read_json(path, 10)
        for value in ({}, {"schema": True}, {"schema": 1, "test_returncode": 0}):
            with self.subTest(value=value), self.assertRaises(ValueError):
                reporting.result_value(value)


class CanonicalCalls(unittest.TestCase):
    def test_local_ci_and_inherited_release_keep_one_complete_entrypoint(self) -> None:
        root = Path(__file__).resolve().parents[2]
        local = (root / "scripts/check-all.sh").read_text()
        ci = (root / ".github/workflows/ci.yml").read_text()
        release = (root / ".github/workflows/release.yml").read_text()
        local_command = (
            "uv run --locked --project automation python -m boxferry_ci.test_reporting "
            "--repository-root . --start-directory automation/tests"
        )
        ci_command = (
            "uv run --locked --project _automation/automation python -m "
            "boxferry_ci.test_reporting --repository-root _automation "
            "--start-directory _automation/automation/tests"
        )
        self.assertEqual(local.count(local_command), 1)
        self.assertEqual(ci.count(ci_command), 1)
        self.assertNotIn("-m unittest discover", local + ci)
        automation = ci.split("  automation:\n", 1)[1].split(
            "  canonical-artifact-integrity:\n", 1
        )[0]
        self.assertIn("timeout-minutes: 10", automation)
        self.assertIn("ref: ${{ github.sha }}", automation)
        self.assertIn("uses: ./_automation/.github/actions/shared-python", automation)
        self.assertEqual(release.count("uses: ./.github/workflows/ci.yml"), 1)
        self.assertNotIn("test_reporting", release)
        self.assertNotIn("--pattern", ci_command + local_command)


class CompleteFixtures(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="reporting-fixture-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.tests = self.root / "tests"
        self.tests.mkdir()
        self.temps = self.root / "private"
        self.temps.mkdir()
        self.environment = {
            **os.environ,
            "PYTHONPATH": str(SOURCE),
            "PYTHONDONTWRITEBYTECODE": "1",
            "RUNNER_TEMP": str(self.temps),
            "TEST_TOKEN": "fixture-reporting-private-value",
            "GITHUB_SHA": "a" * 40,
            "GITHUB_SERVER_URL": "https://github.com",
            "GITHUB_REPOSITORY": "Strukturpiloten/boxferry",
            "GITHUB_RUN_ID": "123",
        }

    def fixture(self, source: str) -> None:
        (self.tests / "test_cases.py").write_text(textwrap.dedent(source))

    def execute(self, *, timeout: float = 10, bounds: Bounds | None = None) -> tuple[int, str]:
        code, output = reporting.execute(
            self.root, self.tests, timeout=timeout, bounds=bounds, environment=self.environment
        )
        self.assertNotIn("fixture-reporting-private-value", output)
        self.assertNotIn("::", output)
        self.assertNotIn("##[", output)
        self.assertNotIn(str(self.root), output)
        self.assertEqual(list(self.temps.iterdir()), [])
        return code, output

    def test_expected_negative_commands_and_native_subprocess_output_stay_data(self) -> None:
        self.fixture("""
            import os,sys,subprocess,unittest
            from pathlib import Path
            class Cases(unittest.TestCase):
                def test_expected(self):
                    Path('once').write_text('discovered once')
                    parts = (b'embedded:', b':error:: missing token ##',
                             b'[error] oversized event')
                    for part in parts:
                        os.write(1,part)
                    os.write(1,b':')
                    os.write(2,b':error::cross-channel')
                    secret=os.environ['TEST_TOKEN'].encode()
                    os.write(2,secret[:8]); os.write(2,secret[8:])
                    os.write(1,str(Path.cwd()).encode()+b'\\xff')
                    code = 'import os; os.write(2,b"::error::native")'
                    subprocess.run([sys.executable,'-c',code],check=True)
        """)
        code, output = self.execute()
        self.assertEqual(code, 0, output)
        self.assertIn("tests_run: 1", output)
        self.assertIn("missing token", output)
        self.assertIn("oversized event", output)
        self.assertIn("native", output)
        self.assertIn("https://github.com/Strukturpiloten/boxferry/actions/runs/123", output)
        self.assertEqual((self.root / "once").read_text(), "discovered once")

    def test_existing_negative_distribution_fixture_passes_without_commands(self) -> None:
        distribution_test = SOURCE.parent / "tests/test_distribution.py"
        self.fixture(f"""
            import importlib.util,unittest
            spec=importlib.util.spec_from_file_location('real_distribution',{str(distribution_test)!r})
            module=importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            class Cases(unittest.TestCase):
                def test_original_negative(self):
                    name='test_missing_token_and_oversized_event_have_actionable_failures'
                    result=unittest.TestResult()
                    module.APIBoundaryTests(name).run(result)
                    self.assertTrue(result.wasSuccessful(),result.errors+result.failures)
        """)
        code, output = self.execute()
        self.assertEqual(code, 0, output)
        self.assertIn("tests_run: 1", output)
        self.assertIn("error", output.lower())
        self.assertIn("event", output.lower())

    def test_actual_failure_identifies_test_exception_location_and_trace(self) -> None:
        self.fixture("""
            import unittest
            class Cases(unittest.TestCase):
                def test_real_failure(self):
                    raise ValueError('concrete useful failure')
        """)
        code, output = self.execute()
        self.assertEqual(code, 1, output)
        self.assertIn("primary_test_returncode: 1", output)
        self.assertIn("test_cases.Cases.test_real_failure", output)
        self.assertIn("ValueError: concrete useful failure", output)
        self.assertIn("test_cases.py", output)
        self.assertIn("line 5", output)

    def test_late_daemon_and_atexit_native_writes_remain_captured(self) -> None:
        self.fixture("""
            import atexit,os,threading,time,unittest
            def final():
                time.sleep(.15)
                os.write(2,b'##[error] callback-after-suite')
            atexit.register(final)
            class Cases(unittest.TestCase):
                def test_late(self):
                    def writer():
                        time.sleep(.08)
                        text = b'::error::daemon-after-suite '+os.environ['TEST_TOKEN'].encode()
                        os.write(1,text)
                    threading.Thread(target=writer,daemon=True).start()
        """)
        code, output = self.execute()
        self.assertEqual(code, 0, output)
        self.assertIn("daemon-after-suite", output)
        self.assertIn("callback-after-suite", output)

    def test_late_inherited_subprocess_is_drained_through_eof(self) -> None:
        self.fixture("""
            import subprocess,sys,unittest
            class Cases(unittest.TestCase):
                def test_late(self):
                    code = "import os,time; time.sleep(.15); "
                    code += "os.write(1,b'::error::inherited-late')"
                    subprocess.Popen([sys.executable,'-c',code])
        """)
        code, output = self.execute()
        self.assertEqual(code, 0, output)
        self.assertIn("inherited-late", output)

    def test_inherited_unclosed_writer_times_out_and_preserves_primary(self) -> None:
        self.fixture("""
            import subprocess,sys,unittest
            class Cases(unittest.TestCase):
                def test_fail_then_hold(self):
                    subprocess.Popen([sys.executable,'-c','import time; time.sleep(20)'])
                    self.fail('reserved failure before finalization timeout')
        """)
        started = time.monotonic()
        code, output = self.execute(timeout=0.6)
        self.assertEqual(code, 124, output)
        self.assertLess(time.monotonic() - started, 3)
        self.assertIn("primary_test_returncode: 1", output)
        self.assertIn("reserved failure before finalization timeout", output)
        self.assertIn("capture-infrastructure", output)

    def test_hard_original_budget_is_distinct_from_retention(self) -> None:
        self.fixture("""
            import os,unittest
            class Cases(unittest.TestCase):
                def test_chatty(self):
                    for i in range(70): os.write(1,b'x'*32768)
                def test_later_failure(self): self.fail('last reserved concrete failure')
        """)
        code, output = self.execute()
        self.assertEqual(code, 2, output)
        self.assertIn("hard-original-output-budget-exceeded", output)
        self.assertIn("primary_test_returncode: 1", output)
        self.assertIn("last reserved concrete failure", output)
        self.assertIn("omitted_transcript_bytes:", output)
        self.assertLess(len(output.encode()), 600_000)

    def test_passing_tests_still_fail_the_hard_original_budget(self) -> None:
        self.fixture("""
            import os,unittest
            class Cases(unittest.TestCase):
                def test_chatter(self):
                    for index in range(70): os.write(1,b'x'*32768)
                    os.write(1,b'ORDINARY-AFTER-HARD')
                def test_last(self): pass
        """)
        code, output = self.execute()
        self.assertEqual(code, 2, output)
        self.assertIn("primary_test_returncode: 0", output)
        self.assertIn("tests_run: 2", output)
        self.assertNotIn("ORDINARY-AFTER-HARD", output)
        self.assertIn("original_output_bytes:", output)
        self.assertIn("hard-original-output-budget-exceeded", output)
        self.assertLess(len(output.encode()), 600000)

    def test_encoded_result_budget_keeps_reserved_first_last_and_all_counts(self) -> None:
        self.fixture("""
            import unittest
            class Cases(unittest.TestCase):
                def test_many(self):
                    for index in range(40):
                        with self.subTest(index=index):
                            self.fail('budget-primary-'+str(index)+'🔥'*10000)
        """)
        code, output = self.execute()
        self.assertEqual(code, 2, output)
        self.assertIn("primary_test_returncode: 1", output)
        self.assertIn("'failure': 40", output)
        self.assertIn("budget-primary-0", output)
        self.assertIn("budget-primary-39", output)
        self.assertIn("result-handoff-missing-malformed-or-oversized", output)

    def test_retention_clipping_alone_does_not_abort_discovery(self) -> None:
        self.fixture("""
            import os,unittest
            class Cases(unittest.TestCase):
                def test_chatter(self): os.write(1,b'x'*10000)
                def test_last(self): os.write(2,b'last-test-completed')
        """)
        code, output = self.execute(bounds=Bounds(20000, 64, 256))
        self.assertEqual(code, 0, output)
        self.assertIn("tests_run: 2", output)
        self.assertIn("last-test-completed", output)
        self.assertIn("[omitted", output)

    def test_failure_record_first_last_reserve_keeps_all_counters(self) -> None:
        self.fixture("""
            import unittest
            class Cases(unittest.TestCase):
                def test_subtests(self):
                    for index in range(40):
                        with self.subTest(index=index): self.fail('failure-'+str(index))
        """)
        code, output = self.execute()
        self.assertEqual(code, 1, output)
        self.assertIn("'failure': 40", output)
        self.assertIn("omitted_failure_records: 16", output)
        self.assertIn("index=0", output)
        self.assertIn("index=39", output)

    def test_empty_and_broken_discovery_are_reporting_failures(self) -> None:
        code, output = self.execute()
        self.assertEqual(code, 2, output)
        self.fixture("this is not python !")
        code, output = self.execute()
        self.assertNotEqual(code, 0, output)
        self.assertIn("SyntaxError", output)

    def test_class_setup_failure_and_class_skip_keep_normal_unittest_semantics(self) -> None:
        self.fixture("""
            import unittest
            class Cases(unittest.TestCase):
                @classmethod
                def setUpClass(cls): raise RuntimeError('class setup cause')
                def test_never(self): pass
        """)
        code, output = self.execute()
        self.assertEqual(code, 1, output)
        self.assertIn("class setup cause", output)
        self.assertIn("tests_discovered: 1", output)
        self.fixture("""
            import unittest
            class Cases(unittest.TestCase):
                @classmethod
                def setUpClass(cls): raise unittest.SkipTest('explicit skip')
                def test_skipped(self): pass
        """)
        code, output = self.execute()
        self.assertEqual(code, 0, output)
        self.assertIn("'skipped': 1", output)
        self.assertIn("tests_discovered: 1", output)

    def test_preflight_never_starts_discovery_for_oversized_secret(self) -> None:
        self.fixture("raise RuntimeError('must not discover')")
        self.environment["SECRET_OVERSIZED"] = "hidden" * 3000
        code, output = self.execute()
        self.assertEqual(code, 2, output)
        self.assertNotIn("must not discover", output)
        self.assertNotIn("hidden", output)

    def test_worker_interruption_preserves_known_test_failure_and_signal_status(self) -> None:
        for action, expected in (
            ("raise KeyboardInterrupt", 130),
            ("os.kill(os.getpid(),signal.SIGTERM)", 143),
        ):
            with self.subTest(action=action):
                self.fixture(f"""
                    import os,signal,unittest
                    class Cases(unittest.TestCase):
                        def test_a_failure(self): self.fail('known before worker interruption')
                        def test_b_interrupt(self): {action}
                """)
                code, output = self.execute()
                self.assertEqual(code, expected, output)
                self.assertIn("primary_test_returncode: 1", output)
                self.assertIn("known before worker interruption", output)
                self.assertIn("worker-cancelled", output)

    def test_discovery_exception_kind_is_redacted_before_handoff_encoding(self) -> None:
        self.fixture("""
            import os,unittest
            Error=type(os.environ['TEST_TOKEN']+'::error::',(Exception,),{})
            class Broken(unittest.TestSuite):
                def countTestCases(self): raise Error()
            def load_tests(loader, tests, pattern): return Broken()
        """)
        original = reporting.run

        def inspect(
            command: tuple[str, ...],
            *,
            cwd: Path,
            stage: str,
            timeout_seconds: float,
            environment: Mapping[str, str],
        ) -> RunResult:
            outcome = original(
                command,
                cwd=cwd,
                stage=stage,
                timeout_seconds=timeout_seconds,
                environment=environment,
            )
            path = Path(command[command.index("--result") + 1])
            raw = path.read_bytes()
            self.assertNotIn(b"fixture-reporting-private-value", raw)
            self.assertNotIn(b"::", raw)
            self.assertNotIn(b"##[", raw)
            return outcome

        with patch.object(reporting, "run", side_effect=inspect):
            code, output = self.execute()
        self.assertEqual(code, 2, output)
        self.assertIn("discovery-or-interruption", output)

    def test_argument_setup_error_does_not_echo_workflow_commands(self) -> None:
        command = [
            sys.executable,
            "-m",
            "boxferry_ci.test_reporting",
            "--start-directory",
            str(self.tests),
            "--unknown",
            "::error:: raw",
        ]
        result = subprocess.run(
            command, env=self.environment, capture_output=True, text=True, check=False, timeout=5
        )
        self.assertEqual(result.returncode, 2)
        self.assertNotIn("::", result.stdout + result.stderr)
        self.assertNotIn("##[", result.stdout + result.stderr)
        self.assertNotIn("raw", result.stdout + result.stderr)

    def test_spawn_failure_has_no_raw_fallback(self) -> None:
        self.fixture("import unittest")
        with patch.object(reporting, "run", side_effect=OSError(errno.EIO, "::error:: private")):
            code, output = self.execute()
        self.assertEqual(code, 2, output)
        self.assertNotIn("private", output)

    def test_signals_during_finalization_preserve_failure_and_exit_status(self) -> None:
        self.fixture("""
            import subprocess,sys,unittest
            class Cases(unittest.TestCase):
                def test_known(self):
                    subprocess.Popen([sys.executable,'-c','import time; time.sleep(20)'])
                    self.fail('known before signal')
        """)
        for signum, expected in ((signal.SIGINT, 130), (signal.SIGTERM, 143)):
            with self.subTest(signal=signum):
                command = [
                    sys.executable,
                    "-m",
                    "boxferry_ci.test_reporting",
                    "--repository-root",
                    str(self.root),
                    "--start-directory",
                    str(self.tests),
                ]
                process = subprocess.Popen(
                    command,
                    env=self.environment,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    start_new_session=True,
                )
                try:
                    deadline = time.monotonic() + 5
                    while not list(self.temps.glob("*/result.json")):
                        if time.monotonic() > deadline:
                            self.fail("worker did not produce bounded checkpoint")
                        time.sleep(0.01)
                    process.send_signal(signum)
                    out, err = process.communicate(timeout=5)
                finally:
                    if process.poll() is None:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait(timeout=5)
                self.assertEqual(process.returncode, expected, (out, err))
                self.assertIn(b"known before signal", out)
                self.assertNotIn(b"::", out + err)
                self.assertNotIn(b"##[", out + err)
                self.assertEqual(list(self.temps.iterdir()), [])

    def collector_fault(self, code: str) -> tuple[int, str]:
        original = reporting.run
        script = (
            "import sys,os,time; from pathlib import Path\n"
            "import boxferry_ci.test_reporting as m\n"
            + code
            + "\nsys.argv=['collector']+sys.argv[1:]\nraise SystemExit(m.main())\n"
        )

        def invoke(
            command: tuple[str, ...],
            *,
            cwd: Path,
            stage: str,
            timeout_seconds: float,
            environment: Mapping[str, str],
        ) -> RunResult:
            return original(
                (sys.executable, "-c", script, *command[3:]),
                cwd=cwd,
                stage=stage,
                timeout_seconds=timeout_seconds,
                environment=environment,
            )

        with patch.object(reporting, "run", side_effect=invoke):
            return self.execute(timeout=3)

    def known_failure(self) -> None:
        self.fixture("""
            import unittest
            class Cases(unittest.TestCase):
                def test_known(self): self.fail('reserved concrete primary failure')
        """)

    def test_collector_output_handle_is_noninheritable(self) -> None:
        self.fixture("""
            import unittest
            class Cases(unittest.TestCase):
                def test_ok(self): pass
        """)
        code, output = self.collector_fault("""
write=m.os.write
def checked(fd,data):
    assert fd not in (1,2)
    assert not os.get_inheritable(fd)
    return write(fd,data)
m.os.write=checked
""")
        self.assertEqual(code, 0, output)

    def test_actual_drain_error_keeps_known_failure_and_never_replays_raw(self) -> None:
        self.known_failure()
        code, output = self.collector_fault("""
read=m.os.read
result=Path(sys.argv[sys.argv.index('--result')+1])
def broken(fd,count):
    if count==32768:
        end=time.monotonic()+1
        while not result.exists() and time.monotonic()<end: time.sleep(.01)
        raise OSError('drain failed ::error::')
    return read(fd,count)
m.os.read=broken
""")
        self.assertEqual(code, 2, output)
        self.assertIn("primary_test_returncode: 1", output)
        self.assertIn("reserved concrete primary failure", output)
        self.assertIn("collector-capture-or-render-failure", output)

    def test_result_write_and_collector_encoding_errors_remain_nonzero(self) -> None:
        for injection in (
            "Path(sys.argv[sys.argv.index('--result')+1]).mkdir()",
            "def broken(*args): raise ValueError('encode failure')\nm.encoded=broken",
        ):
            with self.subTest(injection=injection):
                self.known_failure()
                code, output = self.collector_fault(injection)
                self.assertEqual(code, 2, output)
                self.assertIn("primary_test_returncode: 1", output)
                self.assertIn("reserved concrete primary failure", output)

    def test_parent_report_render_failure_uses_already_sanitized_primary_reserve(self) -> None:
        self.known_failure()
        with patch.object(reporting, "render", side_effect=ValueError("raw ::error:: private")):
            code, output = self.execute()
        self.assertEqual(code, 2, output)
        self.assertIn("primary_test_returncode: 1", output)
        self.assertIn("reserved concrete primary failure", output)
        self.assertIn("parent-report-render-failure", output)

    def test_redaction_failure_never_emits_raw_details_and_keeps_primary_status(self) -> None:
        self.known_failure()
        with patch.object(Shield, "stream", side_effect=ValueError("unsafe ::error::")):
            code, output = self.execute()
        self.assertEqual(code, 2, output)
        self.assertIn("primary_test_returncode: 1", output)
        self.assertIn("sanitization failed", output)
        self.assertNotIn("reserved concrete", output)

    def test_hard_summary_budget_preserves_primary_reserve(self) -> None:
        self.known_failure()
        with patch.object(reporting, "SUMMARY_BYTES", 128):
            code, output = self.execute()
        self.assertEqual(code, 2, output)
        self.assertIn("primary_test_returncode: 1", output)
        self.assertIn("reserved concrete primary failure", output)

    def test_interruption_during_parent_render_is_not_reclassified(self) -> None:
        self.known_failure()
        with patch.object(reporting, "render", side_effect=KeyboardInterrupt):
            code, output = self.execute()
        self.assertEqual(code, 130, output)
        self.assertIn("primary_test_returncode: 1", output)
        self.assertIn("interrupted report finalization", output)

    def test_malformed_handoff_is_nonzero_and_original_failure_is_separate(self) -> None:
        record = {
            "test_id": "Cases.failure",
            "exception": "ValueError",
            "cause": "known cause",
            "location": "test_cases.py:4",
            "traceback": "ValueError: known cause",
            "omitted_trace_bytes": 0,
        }

        def failed(command: tuple[str, ...], **_kwargs: object) -> SimpleNamespace:
            primary = Path(command[command.index("--primary") + 1])
            primary.write_text(
                json.dumps(
                    {
                        "schema": 1,
                        "test_returncode": 1,
                        "tests_run": 1,
                        "tests_discovered": 1,
                        "counts": dict(
                            success=0, failure=1, error=0, skipped=0, expected=0, unexpected=0
                        ),
                        "records": [record],
                        "omitted_records": 0,
                        "omitted_trace_bytes": 0,
                        "report_errors": [],
                        "duration_seconds": 0.0,
                    }
                )
            )
            result = Path(command[command.index("--result") + 1])
            result.write_bytes(b"invalid")
            return SimpleNamespace(
                status="failed",
                returncode=2,
                stdout="{}",
                diagnostic=SimpleNamespace(cause="failed"),
            )

        with patch.object(reporting, "run", side_effect=failed):
            code, output = self.execute()
        self.assertEqual(code, 2, output)
        self.assertIn("primary_test_returncode: 1", output)
        self.assertIn("known cause", output)
        self.assertIn("result-handoff-missing-malformed-or-oversized", output)


if __name__ == "__main__":
    unittest.main()
