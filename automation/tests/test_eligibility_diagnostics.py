"""Non-authorizing callback diagnostics, independently checked with inert API fixtures."""

from __future__ import annotations

import io
import json
import os
import tempfile
import unittest
from contextlib import nullcontext, redirect_stderr, redirect_stdout
from pathlib import Path
from typing import Any
from unittest.mock import patch

import test_eligibility as fixtures

from boxferry_ci import eligibility as subject
from boxferry_ci import recovery

PREFIX = "Dependency eligibility resolution: "
OTHER = "9" * 40


def records(stream: io.StringIO) -> list[dict[str, Any]]:
    return [
        json.loads(line[len(PREFIX) :])
        for line in stream.getvalue().splitlines()
        if line.startswith(PREFIX)
    ]


class EligibilityDiagnosticsTests(unittest.TestCase):
    def setUp(self) -> None:
        self.fixture = fixtures.EligibilityTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.fixture.pending_callback("merge_group")
        self.api = self.fixture.api
        self.api.responses["actions/runs/50"].update(status="completed", run_attempt=3)
        self.event: dict[str, Any] = {"action": "completed", "workflow_run": {"id": 50}}

    def failed_resolution(
        self, event: dict[str, Any] | None = None
    ) -> tuple[list[dict[str, Any]], subject.EligibilityError]:
        stream = io.StringIO()
        with (
            tempfile.TemporaryDirectory() as temporary,
            redirect_stderr(stream),
            patch.object(subject, "evaluate") as evaluate,
            patch.object(subject, "report_status") as status,
            patch.object(subject, "rerun_ready") as retry,
            patch.object(recovery, "recover") as recover,
        ):
            output = Path(temporary) / subject.STATE_FILE
            with self.assertRaises(subject.EligibilityError) as failure:
                subject.service(
                    self.api,
                    event_name="workflow_run",
                    event=self.event if event is None else event,
                    implementation=fixtures.IMPLEMENTATION,
                    run_id=11,
                    run_attempt=1,
                    output=output,
                    now=fixtures.NOW,
                )
            self.assertFalse(output.exists())
            self.assertEqual(self.api.posted, [])
            for effect in (evaluate, status, retry, recover):
                effect.assert_not_called()
        self.assertIsNone(subject._ACTIVE_RESOLUTION.get())
        return records(stream), failure.exception

    def test_fallback_binding_retains_identity_and_only_known_operands(self) -> None:
        self.fixture.queue([self.fixture.node(head=OTHER)])
        self.api.responses["pulls/17"].update(state="closed", mergeable=None, merge_commit_sha=None)
        output, error = self.failed_resolution()
        self.assertEqual(str(error), "unready queue PR does not match the exact source binding")
        self.assertEqual(
            [r["stage"] for r in output], ["callback-event", "source-run", "queue-fallback"]
        )
        last = output[-1]
        expected = {
            "event_action": "completed",
            "source_run_id": 50,
            "observed_run_id": 50,
            "source_run_attempt": 3,
            "source_event": "merge_group",
            "tested_sha": fixtures.HEAD,
            "source_run_id_matches": True,
            "guard": "queue-fallback-binding",
            "queue_count": 1,
            "built_count": 1,
            "exact_count": 0,
            "selected_count": 0,
            "matching_count": None,
            "source_pr_number": 17,
            "resolution_base_sha": fixtures.BASE,
            "queue_source_sha": fixtures.SOURCE,
            "pr_source_sha": fixtures.SOURCE,
            "queue_head_sha": OTHER,
            "pr_merge_sha": None,
            "source_head_matches": True,
            "queue_head_matches_tested": False,
            "queue_head_matches_source": False,
            "queue_head_matches_merge": None,
        }
        for key, value in expected.items():
            with self.subTest(field=key):
                self.assertEqual(last[key], value)

    def test_current_and_fallback_binding_guards_stay_distinct(self) -> None:
        self.fixture.queue([self.fixture.node(source=OTHER)])
        self.api.responses["pulls/17"].update(state="closed", mergeable=None, merge_commit_sha=None)
        output, error = self.failed_resolution()
        self.assertEqual(str(error), "unready queue PR does not match its exact source binding")
        self.assertEqual(output[-1]["stage"], "queue-current")
        self.assertEqual(output[-1]["guard"], "queue-current-binding")
        self.assertIs(output[-1]["source_head_matches"], False)
        self.assertIsNone(output[-1]["matching_count"])

    def test_empty_fallback_reports_observed_zero_without_guessing_a_pr(self) -> None:
        self.fixture.queue([])
        output, error = self.failed_resolution()
        self.assertEqual(
            str(error),
            "queue candidate has no unique exact source PR; multi-entry groups are unsupported",
        )
        last = output[-1]
        self.assertEqual(last["guard"], "queue-fallback-cardinality")
        for field in (
            "queue_count",
            "built_count",
            "exact_count",
            "selected_count",
            "matching_count",
        ):
            self.assertEqual(last[field], 0)
        for field in ("source_pr_number", "pr_source_sha", "queue_source_sha", "pr_merge_sha"):
            self.assertIsNone(last[field])

    def test_multiple_fallback_reports_actual_two(self) -> None:
        tested = "8" * 40
        self.api.responses["actions/runs/50"]["head_sha"] = tested
        self.fixture.bind_merge(tested)
        self.api.responses["pulls/17"] = fixtures.pr_metadata()
        self.api.responses["pulls/18"] = fixtures.pr_metadata(18)
        self.fixture.queue([self.fixture.node(), self.fixture.node(number=18)])
        output, error = self.failed_resolution()
        self.assertIn("no unique exact source PR", str(error))
        self.assertEqual(output[-1]["guard"], "queue-fallback-cardinality")
        self.assertEqual(output[-1]["matching_count"], 2)
        self.assertEqual(output[-1]["queue_count"], 2)
        self.assertIsNone(output[-1]["source_pr_number"])

    def test_invalid_event_identity_stays_unknown_and_fail_closed(self) -> None:
        secret = "::error::https://user:credential@example.invalid/?token=private"
        for value in (None, True, 0, secret):
            with self.subTest(source_id=value):
                output, error = self.failed_resolution(
                    {"action": secret, "workflow_run": {"id": value}}
                )
                self.assertIn("no valid source identifier", str(error))
                self.assertEqual(output[-1]["guard"], "source-run-id-invalid")
                for field in ("event_action", "source_run_id", "tested_sha", "queue_count"):
                    self.assertIsNone(output[-1][field])
                self.assertNotIn(secret, json.dumps(output))

    def test_malformed_source_fields_are_not_rendered_or_promoted(self) -> None:
        secret = "https://user:credential@example.invalid/?token=private"
        self.api.responses["actions/runs/50"].update(
            head_sha=secret, run_attempt=True, id=-1, event=secret
        )
        output, _error = self.failed_resolution()
        last = output[-1]
        self.assertEqual(last["guard"], "initial-resolution-error")
        self.assertEqual(last["stage"], "source-run")
        for field in (
            "tested_sha",
            "observed_run_id",
            "source_run_attempt",
            "source_event",
            "source_run_id_matches",
        ):
            self.assertIsNone(last[field])
        self.assertNotIn(secret, json.dumps(output))

    def test_failed_snapshot_read_keeps_unobserved_counts_unknown(self) -> None:
        original = subject.EligibilityError("snapshot read failed")
        with patch.object(subject, "_queue_entries", side_effect=original):
            output, error = self.failed_resolution()
        self.assertIs(error, original)
        self.assertEqual(output[-1]["stage"], "queue-snapshot")
        self.assertEqual(output[-1]["guard"], "initial-resolution-error")
        for name in ("queue_count", "built_count", "exact_count", "matching_count"):
            self.assertIsNone(output[-1][name])

    def test_separate_callbacks_do_not_retain_prior_source_or_queue_facts(self) -> None:
        self.fixture.queue([])
        first, _error = self.failed_resolution()
        self.assertEqual(first[-1]["queue_count"], 0)
        second, _error = self.failed_resolution({"action": "requested", "workflow_run": {}})
        self.assertEqual(second[-1]["stage"], "callback-event")
        self.assertEqual(second[-1]["guard"], "source-run-id-invalid")
        self.assertEqual(second[-1]["event_action"], "requested")
        for name in ("source_run_id", "source_run_attempt", "tested_sha", "queue_count"):
            self.assertIsNone(second[-1][name])

    def test_renderer_revalidates_values_and_excludes_unrecognized_fields(self) -> None:
        secret = "https://user:private@example.invalid/"
        trace = subject._ResolutionDiagnostics(
            values={"tested_sha": secret, "queue_count": True, "private": secret}
        )
        stream = io.StringIO()
        with redirect_stderr(stream):
            trace.emit()
        output = records(stream)
        self.assertEqual(len(output), 1)
        self.assertIsNone(output[0]["tested_sha"])
        self.assertIsNone(output[0]["queue_count"])
        self.assertNotIn("private", output[0])
        self.assertNotIn(secret, stream.getvalue())

    def test_render_failure_preserves_original_guard_exception(self) -> None:
        original = subject.EligibilityError("preserved original failure")
        with (
            patch.object(subject, "queue_candidate", side_effect=original),
            patch.object(
                subject._ResolutionDiagnostics,
                "render",
                side_effect=RuntimeError("private render text"),
            ),
        ):
            output, error = self.failed_resolution()
        self.assertIs(error, original)
        self.assertEqual(output, [])

    def test_stderr_failure_preserves_original_exception(self) -> None:
        class BrokenStream(io.StringIO):
            def write(self, _value: str) -> int:
                raise OSError("private stream failure")

        original = subject.EligibilityError("preserved original failure")
        with redirect_stderr(BrokenStream()), self.assertRaises(subject.EligibilityError) as caught:
            with subject._resolution_context(self.event):
                raise original
        self.assertIs(caught.exception, original)
        self.assertIsNone(subject._ACTIVE_RESOLUTION.get())

    def test_whitelist_unknowns_and_record_byte_budgets(self) -> None:
        secret = "::error::https://user:credential@example.invalid/" + "private" * 10000
        stream = io.StringIO()
        with (
            redirect_stderr(stream),
            subject._resolution_context({"action": secret, "workflow_run": {"id": 2**80}}),
        ):
            for _number in range(100):
                subject._resolution_observe(
                    secret,
                    emit=True,
                    tested_sha=secret,
                    source_run_attempt=True,
                    queue_count=-1,
                    matching_count=101,
                    extra_token=secret,
                )
        output = records(stream)
        self.assertEqual(len(output), subject._MAX_RESOLUTION_RECORDS)
        self.assertNotIn(secret, stream.getvalue())
        for line in stream.getvalue().splitlines():
            self.assertLessEqual(len(line.encode()), subject._MAX_RESOLUTION_BYTES)
        for record in output:
            self.assertNotIn("extra_token", record)
            self.assertIsNone(record["source_run_id"])
        with patch.object(subject, "_MAX_RESOLUTION_BYTES", 1):
            self.fixture.queue([])
            output, _error = self.failed_resolution()
        self.assertEqual(output, [])

    def test_successful_defer_keeps_state_and_api_calls_identical(self) -> None:
        self.api.responses["actions/runs/50"].update(status="in_progress")
        event = {"action": "requested", "workflow_run": {"id": 50}}
        snapshots = []
        with tempfile.TemporaryDirectory() as temporary:
            for disabled in (False, True):
                self.api.read_paths.clear()
                self.api.requested.clear()
                stream = io.StringIO()
                scope = (
                    patch.object(subject, "_resolution_context", return_value=nullcontext())
                    if disabled
                    else nullcontext()
                )
                with scope, redirect_stderr(stream):
                    payload = subject.service(
                        self.api,
                        event_name="workflow_run",
                        event=event,
                        implementation=fixtures.IMPLEMENTATION,
                        run_id=11,
                        output=Path(temporary) / subject.STATE_FILE,
                        now=fixtures.NOW,
                    )
                snapshots.append((payload, list(self.api.read_paths), list(self.api.requested)))
                self.assertEqual(self.api.posted, [])
                self.assertNotIn("diagnostic", payload)
                self.assertNotIn("stage", payload)
                self.assertEqual(bool(records(stream)), not disabled)
        self.assertEqual(snapshots[0], snapshots[1])

    def test_legacy_service_arguments_and_causal_cli_result_stay_unchanged(self) -> None:
        self.fixture.queue([])
        with tempfile.TemporaryDirectory() as temporary:
            event = Path(temporary) / "event.json"
            event.write_text(json.dumps(self.event))
            output = Path(temporary) / subject.STATE_FILE
            stdout, stderr = io.StringIO(), io.StringIO()
            with (
                patch.object(subject, "GitHub", return_value=self.api),
                patch.dict(os.environ, {"GH_TOKEN": "fixture-not-a-credential"}),
                redirect_stdout(stdout),
                redirect_stderr(stderr),
            ):
                code = subject.main(
                    [
                        "service",
                        "--repository",
                        fixtures.REPOSITORY,
                        "--implementation-revision",
                        fixtures.IMPLEMENTATION,
                        "--event-name",
                        "workflow_run",
                        "--event-path",
                        str(event),
                        "--run-id",
                        "11",
                        "--run-attempt",
                        "1",
                        "--output",
                        str(output),
                    ]
                )
            self.assertEqual(code, 2)
            self.assertIn("Dependency eligibility infrastructure failure:", stdout.getvalue())
            self.assertEqual(records(stderr)[-1]["guard"], "queue-fallback-cardinality")
            self.assertFalse(output.exists())
            self.assertEqual(self.api.posted, [])
            self.assertEqual(subject.ADMISSION_READER_CONTRACT, 2)


if __name__ == "__main__":
    unittest.main()
