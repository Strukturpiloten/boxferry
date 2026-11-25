"""Recorded empty-queue completion, through the real service/checkpoint entry points.

Only transport is replaced. The successful closure is a prospective offline replay,
never reclassification of run 45135575420 or permission to reset its historical fence.
"""

from __future__ import annotations

import contextlib
import io
import json
import os
import tempfile
import time
import unittest
from collections.abc import Callable
from copy import deepcopy
from datetime import timedelta
from pathlib import Path
from typing import Any, cast
from unittest.mock import patch

from test_recovery import CheckpointAPI

from boxferry_ci import eligibility as core
from boxferry_ci import recovery

FIXTURE = json.loads(
    (Path(__file__).parent / "fixtures/eligibility-queue-race/quadlet-45135575420.json").read_text()
)
C, B, N = (FIXTURE[key] for key in ("implementation", "base", "head"))
NOW = core.utc("2026-11-23T08:03:50Z")


class MergedAPI(CheckpointAPI):
    def add(self, run: dict[str, Any], state: dict[str, Any] | None = None) -> None:
        super().add(run, state)
        if state is not None:
            listing = self.responses[f"actions/runs/{run['id']}/artifacts?per_page=100"]
            listing["total_count"] = len(listing["artifacts"])

    def __init__(self) -> None:
        super().__init__()
        self.repository = "Strukturpiloten/quadlet-lens"
        self.floor = core.utc("2026-11-23T04:08:07Z")
        self.target = N
        self.nodes: list[dict[str, Any]] = []
        self.rules = [
            {
                "type": "merge_queue",
                "parameters": {
                    "max_entries_to_build": 1,
                    "max_entries_to_merge": 1,
                },
            }
        ]
        self.responses.update(deepcopy(FIXTURE["files"]))
        self.responses.update(
            {path.replace(B, N): deepcopy(value) for path, value in FIXTURE["files"].items()}
        )
        self.responses[""] = {"default_branch": "main"}
        for head in (B, N):
            self.responses[f"compare/{head}...main"] = {"status": "ahead"}
        self.responses[f"compare/{N}...{N}"] = {
            "status": "identical",
            "merge_base_commit": {"sha": N},
        }
        self.responses[f"git/commits/{N}"] = deepcopy(FIXTURE["commit"])
        self.source = deepcopy(FIXTURE["source"])
        self.responses[f"actions/runs/{self.source['id']}"] = self.source
        self.responses["pulls/209"] = deepcopy(FIXTURE["pr"])
        self.responses[f"commits/{N}/statuses?per_page=100"] = [
            {
                "context": core.STATUS_CONTEXT,
                "state": "success",
                "creator": {"login": "github-actions[bot]"},
                "target_url": "https://github.com/Strukturpiloten/quadlet-lens/actions/runs/45134111159",
            }
        ]
        self.add(deepcopy(FIXTURE["receipt_run"]), deepcopy(FIXTURE["receipt_state"]))
        self.previous = deepcopy(FIXTURE["previous_state"])
        self.prior = {
            **deepcopy(FIXTURE["service"]),
            "id": self.previous["run_id"],
            "created_at": "2026-11-23T07:51:25Z",
            "status": "completed",
            "conclusion": "success",
        }
        self.add(self.prior, self.previous)
        self.current: dict[str, Any] = {
            **deepcopy(FIXTURE["service"]),
            "status": "in_progress",
            "conclusion": None,
        }
        self.add(self.current)
        self.history = {"total_count": 1, "workflow_runs": [{"id": self.prior["id"]}]}

    def get(self, suffix: str) -> dict[str, Any]:
        if suffix == "git/ref/heads/main":
            return {"ref": "refs/heads/main", "object": {"type": "commit", "sha": self.target}}
        return super().get(suffix)

    def items(self, suffix: str) -> list[dict[str, Any]]:
        if suffix.startswith("rules/branches/main?"):
            return deepcopy(self.rules)
        return super().items(suffix)

    def request(self, method: str, path: str, data: dict[str, Any] | None = None) -> object:
        if method == "GET" and path == f"/repos/Strukturpiloten/boxferry/git/commits/{C}":
            return {"sha": C, "committer": {"date": self.floor.isoformat()}}
        if method == "POST" and path == "/graphql":
            return {
                "data": {
                    "repository": {
                        "mergeQueue": {
                            "entries": {
                                "totalCount": len(self.nodes),
                                "nodes": deepcopy(self.nodes),
                            }
                        }
                    }
                }
            }
        return super().request(method, path, data)


class MergedQueueTests(unittest.TestCase):
    def setUp(self) -> None:
        self.api = MergedAPI()

    def service(self, output: Path, *, action: str = "completed") -> dict[str, Any]:
        with contextlib.redirect_stdout(io.StringIO()):
            return core.service(
                self.api,
                event_name="workflow_run",
                event={"action": action, "workflow_run": {"id": self.api.source["id"]}},
                implementation=C,
                run_id=self.api.current["id"],
                run_attempt=1,
                output=output,
                now=NOW,
            )

    def test_recorded_incident_closes_without_candidate_or_effect_and_restores_checkpoint(
        self,
    ) -> None:
        with (
            tempfile.TemporaryDirectory() as temporary,
            patch.object(core, "evaluate") as evaluate,
            patch.object(core, "rerun_ready") as retry,
            patch.object(recovery, "recover") as recover,
        ):
            output = Path(temporary) / core.STATE_FILE
            with self.assertRaises(core.QueueGroupAbsent):
                core.queue_candidate(self.api, N, "main")
            payload = self.service(output)
            self.assertEqual(json.loads(output.read_text()), payload)
            self.assertEqual(payload["reports"], {})
            self.assertEqual(payload["infrastructure_errors"], [])
            self.assertEqual(payload["no_action_candidates"][0]["pull_request"], 209)
            self.assertIn(N, payload["no_action_candidates"][0]["reason"])
            for key in ("dependency_state", "reruns", "recoveries", "scan_cursor"):
                self.assertEqual(payload[key], self.api.previous[key])
            for operation in (evaluate, retry, recover):
                operation.assert_not_called()
            self.assertEqual(self.api.posts, [])
            completed: dict[str, Any] = {
                **self.api.current,
                "status": "completed",
                "conclusion": "success",
            }
            self.api.add(completed, payload)
            self.assertEqual(
                recovery.checkpoint_payload(
                    self.api,
                    completed,
                    C,
                    "main",
                    self.api.floor,
                    NOW,
                ),
                payload,
            )
            later = {
                **completed,
                "id": completed["id"] + 1,
                "created_at": (NOW + timedelta(minutes=1)).isoformat(),
                "status": "in_progress",
                "conclusion": None,
            }
            self.api.add(later)
            context = recovery.service_context(self.api, later, payload, C, "main", NOW)
            self.assertTrue(context.can_prepare)
            self.assertTrue(context.can_effect)

    def test_all_unknown_failed_malformed_and_changed_bindings_fail_without_checkpoint(
        self,
    ) -> None:
        cases = (
            "failed",
            "active",
            "wrong-event",
            "wrong-reader",
            "bad-attempt",
            "future",
            "unmerged",
            "wrong-merge",
            "wrong-source",
            "foreign-pr",
            "wrong-branch",
            "bad-title",
            "bad-merged-at",
            "missing-receipt",
            "waiting-receipt",
            "wrong-receipt-scope",
            "wrong-receipt-base",
            "wrong-receipt-source",
            "expired-artifact",
            "multiple-parents",
            "malformed-parent",
            "wrong-commit",
            "policy-changed",
            "caller-changed",
            "manifest-changed",
            "protection",
            "expired",
        )
        for case in cases:
            with self.subTest(case=case):
                self.setUp()
                source, pr = self.api.source, self.api.responses["pulls/209"]
                report = deepcopy(FIXTURE["receipt_state"])
                r = report["reports"][N]
                if case == "failed":
                    source["conclusion"] = "failure"
                elif case == "active":
                    source["status"] = "in_progress"
                elif case == "wrong-event":
                    source["event"] = "push"
                elif case == "wrong-reader":
                    source["referenced_workflows"] = []
                elif case == "bad-attempt":
                    source["run_attempt"] = True
                elif case == "future":
                    source["updated_at"] = "2027-01-01T00:00:00Z"
                elif case == "unmerged":
                    pr["merged"] = False
                elif case == "wrong-merge":
                    pr["merge_commit_sha"] = B
                elif case == "wrong-source":
                    pr["head"]["sha"] = B
                elif case == "foreign-pr":
                    pr["head"]["repo"]["full_name"] = "other/repo"
                elif case == "wrong-branch":
                    pr["base"]["ref"] = "other"
                elif case == "bad-title":
                    pr["title"] = "invalid\ntitle"
                elif case == "bad-merged-at":
                    pr["merged_at"] = None
                elif case == "missing-receipt":
                    self.api.responses[f"commits/{N}/statuses?per_page=100"] = []
                elif case == "waiting-receipt":
                    r["status"] = "waiting"
                elif case == "wrong-receipt-scope":
                    r["scope"] = "pull_request"
                elif case == "wrong-receipt-base":
                    r["base_sha"] = N
                elif case == "wrong-receipt-source":
                    r["source_head_sha"] = B
                elif case == "multiple-parents":
                    self.api.responses[f"git/commits/{N}"]["parents"] *= 2
                elif case == "malformed-parent":
                    self.api.responses[f"git/commits/{N}"]["parents"] = [None]
                elif case == "wrong-commit":
                    self.api.responses[f"git/commits/{N}"]["sha"] = B
                elif case in {"policy-changed", "caller-changed", "manifest-changed"}:
                    path = {
                        "policy-changed": "scripts/validation-policy.json",
                        "caller-changed": ".github/workflows/ci.yml",
                        "manifest-changed": ".github/automation.json",
                    }[case]
                    self.api.responses[f"contents/{path}?ref={N}"]["content"] = "e30="
                elif case == "protection":
                    self.api.rules = []
                elif case == "expired":
                    self.api.deadline = time.monotonic() - 1
                self.api.add(deepcopy(FIXTURE["receipt_run"]), report)
                if case == "expired-artifact":
                    self.api.responses["actions/runs/45134111159/artifacts?per_page=100"][
                        "artifacts"
                    ][0]["expired"] = True
                with tempfile.TemporaryDirectory() as temporary:
                    output = Path(temporary) / core.STATE_FILE
                    with self.assertRaises(ValueError):
                        self.service(output)
                    self.assertFalse(output.exists())
                    self.assertEqual(self.api.posts, [])

    def test_existing_fence_is_preserved_and_requested_callback_cannot_close(self) -> None:
        self.api.previous["recovery_frontier"]["fence"] = "historical uncertainty"
        # Authored nonempty budget state, distinct from the recorded empty ledger.
        self.api.previous["reruns"] = {"7" * 40: 123}
        self.api.previous["recoveries"] = {
            "8" * 64: {
                "phase": "uncertain",
                "kind": "retained-graph",
                "original_run_id": 124,
                "original_attempt": 1,
                "source_head_sha": B,
                "authority": C,
                "prepared_service_run": self.api.prior["id"],
                "prepared_at": self.api.prior["created_at"],
                "hinted_candidate_sha": "9" * 40,
                "hinted_base_sha": "6" * 40,
            }
        }
        self.api.add(self.api.prior, self.api.previous)
        active = {
            **self.api.current,
            "id": self.api.current["id"] - 1,
            "created_at": "2026-11-23T08:02:00Z",
        }
        self.api.add(active)
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / core.STATE_FILE
            with self.assertRaisesRegex(core.EligibilityError, "completed callback"):
                self.service(output, action="requested")
            self.assertFalse(output.exists())
            payload = self.service(output)
            self.assertEqual(payload["recovery_frontier"]["fence"], "historical uncertainty")
            self.assertEqual(
                payload["recovery_frontier"]["pending"],
                [
                    {
                        "run_id": active["id"],
                        "run_attempt": 1,
                    }
                ],
            )
            self.assertFalse(payload["recovery_frontier"]["authority"])
            for key in ("dependency_state", "reruns", "recoveries", "scan_cursor"):
                self.assertEqual(payload[key], self.api.previous[key])
            self.assertEqual(self.api.posts, [])

    def test_omitted_requested_queue_nodes_fail_at_either_read(self) -> None:
        for missing_read in (1, 2):
            with self.subTest(missing_read=missing_read):
                self.setUp()
                original = self.api.request
                queue_reads = 0

                def request(
                    method: str,
                    path: str,
                    data: dict[str, Any] | None = None,
                    missing_read: int = missing_read,
                    original: Callable[[str, str, dict[str, Any] | None], object] = original,
                ) -> object:
                    nonlocal queue_reads
                    result = original(method, path, data)
                    if path == "/graphql":
                        queue_reads += 1
                        if queue_reads == missing_read:
                            assert isinstance(result, dict)
                            del result["data"]["repository"]["mergeQueue"]["entries"]["nodes"]
                    return result

                with (
                    tempfile.TemporaryDirectory() as temporary,
                    patch.object(self.api, "request", side_effect=request),
                ):
                    output = Path(temporary) / core.STATE_FILE
                    with self.assertRaisesRegex(core.EligibilityError, "queue entry metadata"):
                        self.service(output)
                    self.assertFalse(output.exists())
                    self.assertEqual(self.api.posts, [])

    def test_callback_cannot_close_a_different_valid_looking_response_run(self) -> None:
        callback_id = self.api.source["id"]
        wrong_id = callback_id + 1000
        substituted = {**self.api.source, "id": wrong_id}
        self.api.responses[f"actions/runs/{callback_id}"] = substituted
        self.api.responses[f"actions/runs/{wrong_id}"] = substituted
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / core.STATE_FILE
            with self.assertRaisesRegex(core.EligibilityError, "exact completed callback"):
                self.service(output)
            self.assertFalse(output.exists())
            self.assertEqual(self.api.posts, [])

    def test_new_closure_requires_explicit_live_complete_artifact_metadata(self) -> None:
        for fault in (
            "missing-expired",
            "null-expired",
            "integer-expired",
            "missing-total",
            "incomplete-total",
        ):
            with self.subTest(fault=fault):
                self.setUp()
                listing = self.api.responses["actions/runs/45134111159/artifacts?per_page=100"]
                if fault == "missing-expired":
                    del listing["artifacts"][0]["expired"]
                    # Legacy reader compatibility is unchanged; only the new proof opts in.
                    self.assertIn(N, core.run_artifact(self.api, 45134111159)["reports"])
                elif fault == "null-expired":
                    listing["artifacts"][0]["expired"] = None
                elif fault == "integer-expired":
                    listing["artifacts"][0]["expired"] = 0
                elif fault == "missing-total":
                    del listing["total_count"]
                else:
                    listing["total_count"] += 1
                with tempfile.TemporaryDirectory() as temporary:
                    output = Path(temporary) / core.STATE_FILE
                    with self.assertRaisesRegex(core.EligibilityError, "receipt artifact"):
                        self.service(output)
                    self.assertFalse(output.exists())
                    self.assertEqual(self.api.posts, [])

    def test_final_mutable_readbacks_and_unknown_transport_fail_closed(self) -> None:
        for fault in (
            "source-rerun",
            "pr-author",
            "queue",
            "target",
            "rules",
            "deadline",
            "transport",
        ):
            with self.subTest(fault=fault):
                self.setUp()
                original = self.api.get
                count = 0

                def get(
                    path: str,
                    fault: str = fault,
                    original: Callable[[str], dict[str, Any]] = original,
                ) -> dict[str, Any]:
                    nonlocal count
                    result = original(path)
                    if path == f"actions/runs/{self.api.source['id']}":
                        count += 1
                        if count == 2:
                            if fault == "source-rerun":
                                result["run_attempt"] = 2
                            elif fault == "pr-author":
                                self.api.responses["pulls/209"]["user"]["login"] = "different-actor"
                            elif fault == "queue":
                                self.api.nodes = [{"unknown": "new entry"}]
                            elif fault == "target":
                                self.api.target = B
                            elif fault == "rules":
                                self.api.rules.append({"type": "required_signatures"})
                            elif fault == "deadline":
                                self.api.deadline = time.monotonic() - 1
                            else:
                                raise core.EligibilityError("transport proof unavailable")
                    return result

                with (
                    tempfile.TemporaryDirectory() as temporary,
                    patch.object(self.api, "get", side_effect=get),
                ):
                    output = Path(temporary) / core.STATE_FILE
                    with self.assertRaises(core.EligibilityError):
                        self.service(output)
                    self.assertFalse(output.exists())
                    self.assertEqual(self.api.posts, [])

    def test_cli_writes_the_no_action_checkpoint_and_summary(self) -> None:
        real_service = core.service

        def clocked(*args: object, **kwargs: object) -> dict[str, Any]:
            kwargs["now"] = NOW
            return real_service(*cast("Any", args), **cast("Any", kwargs))

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            event = root / "event.json"
            event.write_text(
                json.dumps(
                    {
                        "action": "completed",
                        "workflow_run": {"id": self.api.source["id"]},
                    }
                )
            )
            output, summary = root / core.STATE_FILE, root / "summary.md"
            with (
                patch.object(core, "GitHub", return_value=self.api),
                patch.object(core, "service", side_effect=clocked),
                patch.dict(os.environ, {"GH_TOKEN": "offline-fixture-token"}),
                contextlib.redirect_stdout(io.StringIO()),
                contextlib.redirect_stderr(io.StringIO()),
            ):
                code = core.main(
                    [
                        "service",
                        "--repository",
                        self.api.repository,
                        "--event-name",
                        "workflow_run",
                        "--event-path",
                        str(event),
                        "--implementation-revision",
                        C,
                        "--run-id",
                        str(self.api.current["id"]),
                        "--run-attempt",
                        "1",
                        "--output",
                        str(output),
                        "--summary",
                        str(summary),
                    ]
                )
            self.assertEqual(code, 0)
            self.assertEqual(json.loads(output.read_text())["reports"], {})
            self.assertIn("No action for PR #209", summary.read_text())
            self.assertEqual(self.api.posts, [])

    def test_later_ordinary_pr_can_spend_only_its_existing_retry_allowance(self) -> None:
        # Authored later same-authority PR: this is not a real hosted retry and
        # does not use the historically fenced #197 or change its dependency graph.
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / core.STATE_FILE
            closed = self.service(output)
            self.api.add(
                {**self.api.current, "status": "completed", "conclusion": "success"}, closed
            )
            source, merge = "4" * 40, "5" * 40
            original_id = self.api.current["id"] + 5
            self.api.source = {
                **deepcopy(FIXTURE["source"]),
                "id": original_id,
                "event": "pull_request",
                "head_sha": source,
                "conclusion": "failure",
                "created_at": NOW.isoformat(),
                "pull_requests": [{"number": 200, "head": {"sha": source}, "base": {"sha": N}}],
            }
            self.api.responses[f"actions/runs/{original_id}"] = self.api.source
            self.api.responses[f"actions/runs/{original_id}/artifacts?per_page=100"] = {
                "total_count": 0,
                "artifacts": [],
            }
            self.api.responses[f"actions/runs/{original_id}/jobs?per_page=100"] = {
                "total_count": 2,
                "jobs": [
                    {"name": name, "status": "completed", "conclusion": "failure"}
                    for name in ("Dependency eligibility", "Required CI")
                ],
            }
            self.api.responses[
                "actions/workflows/ci.yml/runs?head_sha=" + source + "&per_page=20"
            ] = {
                "workflow_runs": [self.api.source],
            }
            self.api.responses["pulls/200"] = {
                **deepcopy(FIXTURE["pr"]),
                "number": 200,
                "state": "open",
                "mergeable": True,
                "merge_commit_sha": merge,
                "head": {"sha": source, "repo": {"full_name": self.api.repository}},
            }
            for sha in (N, source):
                self.api.responses[f"compare/{sha}...{merge}"] = {
                    "status": "ahead",
                    "merge_base_commit": {"sha": sha},
                }
            self.api.responses["contents/.github/workflows/ci.yml?ref=main"] = deepcopy(
                self.api.responses[f"contents/.github/workflows/ci.yml?ref={N}"]
            )
            self.api.responses[f"contents/.github/workflows/ci.yml?ref={merge}"] = deepcopy(
                self.api.responses[f"contents/.github/workflows/ci.yml?ref={N}"]
            )
            previous = closed
            candidate = core.Candidate(
                N, merge, 200, source, "renovate[bot]", "Bot", "pull_request"
            )
            for step in (1, 2):
                self.api.current = {
                    **self.api.current,
                    "id": original_id + step,
                    "created_at": (NOW + timedelta(minutes=step)).isoformat(),
                }
                self.api.add(self.api.current)
                report = {
                    "schema_version": 1,
                    "status": "pass",
                    "candidate_sha": merge,
                    "base_sha": N,
                    "source_head_sha": source,
                    "pull_request": 200,
                    "scope": "pull_request",
                    "state": deepcopy(previous["dependency_state"]),
                }
                with patch.object(core, "evaluate", return_value=report):
                    payload = self.service(output)
                self.assertIsNone(payload["recovery_frontier"]["fence"])
                self.api.add(
                    {**self.api.current, "status": "completed", "conclusion": "success"}, payload
                )
                previous = payload
                effects = [path for path, _ in self.api.posts if not path.startswith("statuses/")]
                self.assertEqual(
                    effects, [] if step == 1 else [f"actions/runs/{original_id}/rerun-failed-jobs"]
                )
            self.assertEqual(previous["reruns"], {candidate.head: original_id})


if __name__ == "__main__":
    unittest.main()
