"""Independent actual-CLI and canonical workflow failure-report contracts."""

from __future__ import annotations

import contextlib
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from collections.abc import Generator
from pathlib import Path
from unittest.mock import patch

import yaml
from source_report_fixture import CASES, SECRET, exercise

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
from boxferry_ci import dependencies
from boxferry_ci import devcontainer_validation as validation
from boxferry_ci.gate import GateFailure, verify_devcontainer_gate
from boxferry_ci.planner import Comparison, make_plan
from boxferry_ci.policy import load_policy

ROOT = Path(__file__).resolve().parents[2]
CANDIDATE, POLICY, AUTOMATION = "a" * 40, "b" * 40, "c" * 40


class SourceReportTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.output = self.root / "report.json"
        self.exports = self.root / "exports"
        self.argv = [
            "resolve",
            "--repository",
            "boxferry",
            "--candidate-sha",
            CANDIDATE,
            "--policy-sha",
            POLICY,
            "--automation-sha",
            AUTOMATION,
            "--automation",
            str(ROOT),
            "--output",
            str(self.output),
            "--github-output",
            str(self.exports),
        ]

    def invoke(self) -> tuple[int, str]:
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            code = validation.main(self.argv)
        return code, stderr.getvalue()

    def test_real_cli_http_reader_failure_report_and_exact_bindings(self) -> None:
        for case in CASES:
            with self.subTest(case=case):
                directory = self.root / case
                summary = exercise(ROOT, directory, case, CANDIDATE, POLICY, AUTOMATION)
                self.assertEqual(
                    summary["actual_cli_exit"], 0 if case in ("success", "missing-report") else 1
                )
                if case in ("unavailable", "malformed", "invalid-revision"):
                    result = json.loads((directory / "devcontainer-sources.json").read_text())
                    self.assertEqual(result["command"], "resolve")
                    self.assertEqual(result["repository"], "boxferry")
                    self.assertEqual(result["stage"], "metadata")
                    self.assertTrue(result["cause"])
                    self.assertFalse(summary["exports_present"])
                    self.assertNotIn(SECRET, (directory / "resolver.stderr").read_text())

    def test_malformed_json_and_unavailable_http_metadata_do_not_escape_cli(self) -> None:
        for body in (b"not json", b"[]", b'{"sha": "main"}'):
            with (
                self.subTest(body=body),
                patch.object(dependencies.urllib.request, "build_opener") as opener,
            ):
                opener.return_value.open.return_value = io.BytesIO(body)
                code, stderr = self.invoke()
                self.assertEqual(code, 1)
                self.assertEqual(json.loads(self.output.read_text())["status"], "failure")
                self.assertFalse(self.exports.exists())
                self.assertIn("diagnostic artifact", stderr)

    def test_invalid_identity_is_not_recorded_as_trusted_or_sent_to_http(self) -> None:
        for flag, field in (
            ("--candidate-sha", "candidate_sha"),
            ("--policy-sha", "policy_sha"),
            ("--automation-sha", "automation_sha"),
        ):
            with self.subTest(field=field), patch.object(validation, "resolve_sources") as resolve:
                old = list(self.argv)
                self.argv[self.argv.index(flag) + 1] = "invalid"
                code, _ = self.invoke()
                self.argv = old
                self.assertEqual(code, 1)
                self.assertIsNone(json.loads(self.output.read_text())[field])
                self.assertFalse(self.exports.exists())
                resolve.assert_not_called()

    def test_late_tool_authority_failure_has_no_success_exports(self) -> None:
        manifest = {
            "schema": 1,
            "repositories": {name: CANDIDATE for name in validation.ALL_REPOSITORIES},
        }
        with (
            patch.object(validation, "resolve_sources", return_value=manifest),
            patch.object(
                validation, "canonical_versions", side_effect=ValueError("tool authority missing")
            ),
        ):
            code, _ = self.invoke()
        self.assertEqual(code, 1)
        self.assertFalse(self.exports.exists())
        result = json.loads(self.output.read_text())
        self.assertEqual(result["stage"], "tool-authority")
        self.assertIn("tool authority missing", result["cause"])

    def test_writable_report_also_logs_safe_cause_if_later_upload_cannot_retain_it(self) -> None:
        with (
            patch.object(
                validation,
                "resolve_sources",
                side_effect=ValueError(
                    "controlled sibling HTTP unavailable :::error:::: ##[error] " + SECRET
                ),
            ),
            patch.dict(os.environ, {"SOURCE_REPORT_TEST_TOKEN": SECRET}),
        ):
            code, stderr = self.invoke()
        self.assertEqual(code, 1)
        artifact = json.loads(self.output.read_text())
        self.assertEqual(artifact["status"], "failure")
        self.assertFalse(self.exports.exists())
        self.assertIn("controlled sibling HTTP unavailable", stderr)
        self.assertNotIn(SECRET, stderr)
        self.assertNotIn("::", stderr)
        self.assertNotIn("##[", stderr)
        records = [line for line in stderr.splitlines() if line.startswith("{")]
        self.assertEqual(len(records), 1)
        logged = json.loads(records[0])
        for key in ("status", "stage", "candidate_sha", "policy_sha", "automation_sha"):
            self.assertEqual(logged[key], artifact[key])
        self.assertLessEqual(len(records[0].encode()), validation.MAX_FAILURE_STREAM)

    def test_large_failure_is_bounded_redacted_and_private(self) -> None:
        with (
            patch.object(
                validation, "resolve_sources", side_effect=ValueError(SECRET + "x" * 100000)
            ),
            patch.dict(os.environ, {"SOURCE_REPORT_TEST_TOKEN": SECRET}),
        ):
            code, stderr = self.invoke()
        self.assertEqual(code, 1)
        data = self.output.read_bytes()
        self.assertLessEqual(len(data), validation.MAX_REPORT)
        self.assertNotIn(SECRET.encode(), data)
        self.assertNotIn(SECRET, stderr)
        self.assertEqual(self.output.stat().st_mode & 0o777, 0o600)
        self.assertFalse(self.exports.exists())

    def test_report_write_failure_is_actionable_redacted_and_cleans_only_owned_temporaries(
        self,
    ) -> None:
        sentinel = self.root / "unrelated"
        sentinel.write_text("preserved")
        with (
            patch.object(
                validation,
                "resolve_sources",
                side_effect=ValueError(
                    "controlled sibling HTTP unavailable :::error:::: ##[error] " + SECRET
                ),
            ),
            patch.dict(os.environ, {"SOURCE_REPORT_TEST_TOKEN": SECRET}),
            patch.object(Path, "replace", side_effect=PermissionError(SECRET)),
        ):
            code, stderr = self.invoke()
        self.assertEqual(code, 1)
        self.assertIn("could not be written", stderr)
        self.assertIn("controlled sibling HTTP unavailable", stderr)
        self.assertNotIn("::", stderr)
        self.assertNotIn("##[", stderr)
        self.assertNotIn(SECRET, stderr)
        records = [line for line in stderr.splitlines() if line.startswith("{")]
        self.assertEqual(len(records), 1)
        fallback = json.loads(records[0])
        self.assertEqual(fallback["stage"], "metadata")
        self.assertEqual(fallback["candidate_sha"], CANDIDATE)
        self.assertLessEqual(len(records[0].encode()), validation.MAX_FAILURE_STREAM)
        self.assertFalse(self.exports.exists())
        self.assertFalse(self.output.exists())
        self.assertEqual(sorted(x.name for x in self.root.iterdir()), ["unrelated"])
        self.assertEqual(sentinel.read_text(), "preserved")

    def test_partial_write_and_close_failures_preserve_prior_command_file_bytes(self) -> None:
        manifest = {
            "schema": 1,
            "repositories": {name: CANDIDATE for name in validation.ALL_REPOSITORIES},
        }
        factory = validation.tempfile.NamedTemporaryFile
        for failure in ("short-write", "close"):
            self.exports.write_bytes(b"unrelated=prior-value\n")
            self.exports.chmod(0o640)
            calls = 0

            @contextlib.contextmanager
            def temporary(
                *, mode: str, dir: Path, prefix: str, delete: bool, selected_failure: str = failure
            ) -> Generator[object]:
                nonlocal calls
                calls += 1
                with factory(mode=mode, dir=dir, prefix=prefix, delete=delete) as stream:
                    if calls != 2:
                        yield stream
                    elif selected_failure == "short-write":
                        write = stream.write
                        with patch.object(
                            stream, "write", side_effect=lambda data: write(data[:5])
                        ):
                            yield stream
                    else:
                        yield stream
                        raise OSError("controlled command-file close failure")

            with (
                self.subTest(failure=failure),
                patch.object(validation, "resolve_sources", return_value=manifest),
                patch.object(validation.tempfile, "NamedTemporaryFile", side_effect=temporary),
            ):
                code, _ = self.invoke()
            self.assertEqual(code, 1)
            self.assertEqual(self.exports.read_bytes(), b"unrelated=prior-value\n")
            self.assertEqual(self.exports.stat().st_mode & 0o777, 0o640)
            self.assertEqual(json.loads(self.output.read_text())["stage"], "output-export")
            self.assertEqual(list(self.root.glob(".devcontainer-sources-*")), [])

    def test_success_appends_atomically_and_refuses_symlinks_fifos_or_oversized_prior_outputs(
        self,
    ) -> None:
        self.exports.write_bytes(b"unrelated=prior-value\n")
        self.exports.chmod(0o640)
        validation._append_resolution_outputs(self.exports, "sources=controlled\n")
        self.assertEqual(self.exports.read_bytes(), b"unrelated=prior-value\nsources=controlled\n")
        self.assertEqual(self.exports.stat().st_mode & 0o777, 0o640)
        target = self.root / "target"
        target.write_text("untouched")
        for kind in ("symlink", "fifo", "oversized"):
            self.exports.unlink()
            if kind == "symlink":
                self.exports.symlink_to(target)
            elif kind == "fifo":
                os.mkfifo(self.exports)
            else:
                self.exports.write_bytes(b"x" * (validation.MAX_REPORT + 1))
            with self.subTest(kind=kind), self.assertRaises((OSError, ValueError)):
                validation._append_resolution_outputs(self.exports, "sources=controlled\n")
            self.assertEqual(target.read_text(), "untouched")
        self.assertEqual(list(self.root.glob(".devcontainer-sources-*")), [])

    def test_export_failure_overwrites_success_manifest_with_failure_report(self) -> None:
        self.exports.mkdir()
        manifest = {
            "schema": 1,
            "repositories": {name: CANDIDATE for name in validation.ALL_REPOSITORIES},
        }
        with patch.object(validation, "resolve_sources", return_value=manifest):
            code, _ = self.invoke()
        self.assertEqual(code, 1)
        result = json.loads(self.output.read_text())
        self.assertEqual(result["status"], "failure")
        self.assertEqual(result["stage"], "output-export")
        self.assertNotIn("repositories", result)
        self.assertEqual(list(self.exports.iterdir()), [])


class SourceWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.document = yaml.load(
            (ROOT / ".github/workflows/shared-devcontainer-validation.yml").read_text(),
            Loader=yaml.BaseLoader,
        )

    def test_exact_report_upload_is_always_attempted_without_failure_waivers(self) -> None:
        job = self.document["jobs"]["sources"]
        self.assertEqual(job["timeout-minutes"], "5")
        steps = job["steps"]
        resolve = next(step for step in steps if step.get("id") == "resolve")
        upload = next(step for step in steps if step.get("id") == "retain")
        self.assertNotIn("continue-on-error", resolve)
        self.assertIn("set -euo pipefail", resolve["run"])
        for flag in ("--candidate-sha", "--policy-sha", "--automation-sha"):
            self.assertIn(flag, resolve["run"])
        self.assertEqual(upload["if"], "always()")
        self.assertEqual(upload["with"]["path"], "${{ runner.temp }}/devcontainer-sources.json")
        self.assertEqual(upload["with"]["if-no-files-found"], "error")
        self.assertEqual(upload["with"]["retention-days"], "7")
        self.assertTrue(upload["uses"].startswith("actions/upload-artifact@"))
        for lane in ("x64", "arm64"):
            self.assertEqual(self.document["jobs"][lane]["needs"], "sources")
        for step in steps:
            self.assertNotIn("continue-on-error", step)

    def test_retained_failure_still_blocks_native_aggregate_and_required_ci(self) -> None:
        policy = load_policy(ROOT / "scripts/shared-validation-policy.json")
        plan = make_plan(ROOT, policy, event="workflow_dispatch", tested_sha=CANDIDATE)
        comparison = Comparison("workflow_dispatch", CANDIDATE, CANDIDATE, CANDIDATE)
        aggregate = self.document["jobs"]["aggregate"]["steps"][0]["run"]
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "aggregate-output"
            # Even fabricated green images cannot override an actual failed source prerequisite.
            result = subprocess.run(
                ["bash", "-c", aggregate],
                capture_output=True,
                timeout=5,
                env={
                    "PATH": "/usr/bin:/bin",
                    "SOURCES_RESULT": "failure",
                    "X64_RESULT": "success",
                    "ARM64_RESULT": "success",
                    "GITHUB_OUTPUT": str(output),
                },
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(output.exists())
        needs = {job: {"result": "success"} for job in ("validation-plan", *policy.jobs)}
        needs["devcontainer-validation"] = {"result": "failure"}
        with self.assertRaisesRegex(GateFailure, "devcontainer-validation concluded failure"):
            verify_devcontainer_gate(plan, needs, policy, comparison, expected_plan=plan)

    def test_actual_final_workflow_step_fails_for_every_missing_failed_or_skipped_outcome(
        self,
    ) -> None:
        step = self.document["jobs"]["sources"]["steps"][-1]
        self.assertEqual(step["if"], "always()")
        self.assertEqual(
            step["env"],
            {
                "RESOLVE_OUTCOME": "${{ steps.resolve.outcome }}",
                "RETAIN_OUTCOME": "${{ steps.retain.outcome }}",
            },
        )
        for resolve in ("success", "failure", "cancelled", "skipped", ""):
            for retained in ("success", "failure", "cancelled", "skipped", ""):
                with self.subTest(resolve=resolve, retained=retained):
                    result = subprocess.run(
                        ["bash", "-c", step["run"]],
                        capture_output=True,
                        timeout=5,
                        env={
                            "PATH": "/usr/bin:/bin",
                            "RESOLVE_OUTCOME": resolve,
                            "RETAIN_OUTCOME": retained,
                        },
                    )
                    self.assertEqual(
                        result.returncode, 0 if resolve == retained == "success" else 1
                    )
                    if resolve != "success":
                        self.assertIn(b"source resolution failed", result.stdout)
                    if retained != "success":
                        self.assertIn(b"report is missing or could not be retained", result.stdout)


if __name__ == "__main__":
    unittest.main()
