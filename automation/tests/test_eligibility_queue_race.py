"""Authenticated no-effect queue movement, real checkpoint persistence and strict failures.

Only the selected observed fields come from the recorded public Compose failure.
New target/PR/source/tree/healthy-checkpoint facts are explicit unit-test assumptions;
no fixture retroactively blesses the actual failed9eb run or claims hosted recovery.
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
from datetime import datetime, timedelta
from pathlib import Path
from typing import Any, cast
from unittest.mock import patch

from test_recovery import CheckpointAPI

from boxferry_ci import eligibility as core
from boxferry_ci import queue_tree, recovery

FIXTURE = Path(__file__).parent / "fixtures/eligibility-queue-race/compose-42326283108.json"
OBSERVED = json.loads(FIXTURE.read_text())["observed"]
C = OBSERVED["implementation_revision"]
B, N = OBSERVED["base_sha"], OBSERVED["tested_sha"]
SERVICE_ID, SOURCE_ID = OBSERVED["service_run_id"], OBSERVED["source_run_id"]
CREATED = core.utc(OBSERVED["service_created_at"])
NOW = CREATED + timedelta(minutes=10)
# Explicitly synthetic, never observed replacement target/PR/source/proof facts.
MOVED, H, REBUILT = (letter * 40 for letter in "679")
NUMBER = 17


class QueueRaceAPI(CheckpointAPI):
    def __init__(self) -> None:
        super().__init__()
        self.target = B
        self.target_object: dict[str, Any] | None = None
        self.nodes = [
            {
                "baseCommit": {"oid": B},
                "headCommit": {"oid": N},
                "pullRequest": {"number": NUMBER, "headRefOid": H},
            }
        ]
        self.rules: list[dict[str, Any]] = [
            {
                "type": "merge_queue",
                "parameters": {"max_entries_to_build": 1, "max_entries_to_merge": 1},
            }
        ]
        self.responses[""] = {"default_branch": "main"}
        self.responses[f"compare/{B}...main"] = {"status": "ahead"}
        self.responses[f"compare/{B}...{N}"] = {"status": "ahead", "merge_base_commit": {"sha": B}}
        self.responses[f"pulls/{NUMBER}"] = {
            "number": NUMBER,
            "state": "open",
            "mergeable": True,
            "merge_commit_sha": N,
            "base": {"ref": "main", "sha": B, "repo": {"full_name": self.repository}},
            "head": {"sha": H, "repo": {"full_name": self.repository}},
            "title": "ci: reviewed synthetic fixture",
            "user": {"login": "renovate[bot]", "type": "Bot"},
        }
        self.responses[f"actions/runs/{SOURCE_ID}"] = {
            "id": SOURCE_ID,
            "run_attempt": OBSERVED["source_run_attempt"],
            "event": OBSERVED["source_event"],
            "name": "CI",
            "path": ".github/workflows/ci.yml",
            "repository": {"full_name": self.repository},
            "head_repository": {"full_name": self.repository},
            "head_sha": N,
            "status": "completed",
            "conclusion": "success",
            "pull_requests": [],
        }

    def get(self, suffix: str) -> dict[str, Any]:
        if suffix == "git/ref/heads/main":
            return (
                deepcopy(self.target_object)
                if self.target_object is not None
                else {"ref": "refs/heads/main", "object": {"type": "commit", "sha": self.target}}
            )
        return super().get(suffix)

    def items(self, suffix: str) -> list[dict[str, Any]]:
        if suffix == "rules/branches/main?per_page=100&page=1":
            return deepcopy(self.rules)
        return super().items(suffix)

    def request(self, method: str, path: str, data: dict[str, Any] | None = None) -> object:
        if method == "GET" and path == f"/repos/Strukturpiloten/boxferry/git/commits/{C}":
            return {"sha": C, "committer": {"date": self.floor.isoformat()}}
        if method == "POST" and path == "/graphql":
            assert data is not None and "mergeQueue" in data["query"]
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


class QueueRaceTests(unittest.TestCase):
    def setUp(self) -> None:
        self.api = QueueRaceAPI()
        self.prior = self.service_run(
            SERVICE_ID - 10, CREATED - timedelta(hours=1), "completed", "success"
        )
        self.current = self.service_run(SERVICE_ID, CREATED, "in_progress", None)
        self.previous = self.state(self.prior)
        self.previous["dependency_state"]["first_seen"] = {
            "a" * 64: (CREATED - timedelta(days=2)).isoformat()
        }
        self.previous["reruns"] = {"b" * 40: 123}
        self.previous["scan_cursor"] = 37
        for index, phase in enumerate(("prepared", "attempted", "uncertain"), 1):
            self.previous["recoveries"][str(index) * 64] = {
                "phase": phase,
                "kind": "retained-graph",
                "original_run_id": 100 + index,
                "original_attempt": 1,
                "source_head_sha": H,
                "authority": C,
                "prepared_at": self.prior["created_at"],
                "prepared_service_run": self.prior["id"],
                "hinted_candidate_sha": "d" * 40,
                "hinted_base_sha": "e" * 40,
            }
        self.api.add(self.prior, self.previous)
        self.api.add(self.current)
        self.api.history = {"workflow_runs": [{"id": self.prior["id"]}], "total_count": 1}

    def service_run(
        self, identifier: int, created: datetime, status: str, conclusion: str | None
    ) -> dict[str, Any]:
        return {
            "id": identifier,
            "run_attempt": 1,
            "event": "workflow_run",
            "path": core.WORKFLOW_PATH,
            "repository": {"full_name": self.api.repository},
            "head_sha": B,
            "head_branch": "main",
            "created_at": created.isoformat(),
            "status": status,
            "conclusion": conclusion,
            "referenced_workflows": [{"path": f"{core.SHARED_WORKFLOW_PATH}@{C}", "sha": C}],
        }

    def state(self, run: dict[str, Any]) -> dict[str, Any]:
        return {
            "schema": 1,
            "repository": self.api.repository,
            "workflow_path": core.WORKFLOW_PATH,
            "implementation_revision": C,
            "run_id": run["id"],
            "run_attempt": 1,
            "observed_at": CREATED.isoformat(),
            "dependency_state": {"schema_version": 1, "first_seen": {}},
            "reruns": {},
            "recoveries": {},
            "reports": {},
            "scan_cursor": 0,
            "recovery_frontier": {
                "protocol": 1,
                "floor": self.api.floor.isoformat(),
                "service_created_at": run["created_at"],
                "authority": True,
                "fence": None,
                "pending": [],
                "predecessor_run_id": None,
            },
        }

    def service(self, output: Path, *, source_run_id: int = SOURCE_ID) -> dict[str, Any]:
        with contextlib.redirect_stdout(io.StringIO()):
            return core.service(
                self.api,
                event_name="workflow_run",
                event={"action": "completed", "workflow_run": {"id": source_run_id}},
                implementation=C,
                run_id=self.current["id"],
                run_attempt=1,
                output=output,
                now=NOW,
            )

    def move(self, *_args: object, **_kwargs: object) -> None:
        self.api.target = MOVED

    def assert_preserved(self, payload: dict[str, Any]) -> None:
        for field in ("dependency_state", "reruns", "recoveries", "scan_cursor"):
            self.assertEqual(payload[field], self.previous[field], field)

    def test_recorded_shape_defers_and_persists_real_checkpoint_without_any_effect(self) -> None:
        with (
            tempfile.TemporaryDirectory() as temporary,
            patch.object(queue_tree, "verify_queue_tree", side_effect=self.move),
            patch.object(core, "evaluate") as evaluate,
            patch.object(core, "report_status") as status,
            patch.object(core, "rerun_ready") as retry,
            patch.object(recovery, "recover") as recover,
        ):
            output = Path(temporary) / core.STATE_FILE
            expected_frontier = recovery.service_context(
                self.api, self.current, self.previous, C, "main", NOW
            ).frontier
            payload = self.service(output)
            self.assertEqual(payload["recovery_frontier"], expected_frontier)
            self.assertEqual(json.loads(output.read_text()), payload)
            self.assertEqual(payload["reports"], {})
            self.assertEqual(payload["infrastructure_errors"], [])
            self.assertEqual(len(payload["deferred_candidates"]), 1)
            deferred = payload["deferred_candidates"][0]
            self.assertEqual(deferred["source_run_id"], SOURCE_ID)
            self.assertEqual(deferred["source_head_sha"], H)
            self.assertEqual(
                deferred["queue_movement"],
                {
                    "kind": "queue-target-moved",
                    "stage": "queue-current",
                    "target_branch": "main",
                    "before_sha": B,
                    "after_sha": MOVED,
                    "tested_sha": N,
                    "source_run_attempt": 1,
                },
            )
            self.assert_preserved(payload)
            for operation in (evaluate, status, retry, recover):
                operation.assert_not_called()
            self.assertEqual(self.api.posts, [])
            completed = {**self.current, "status": "completed", "conclusion": "success"}
            self.api.add(completed, payload)
            # Real validation/ZIP decoding/checkpoint provenance; no authority mock.
            restored = recovery.checkpoint_payload(
                self.api, completed, C, "main", self.api.floor, NOW
            )
            self.assertEqual(restored, payload)

    def test_next_stable_event_consumes_closed_checkpoint_and_resumes_normal_hold(self) -> None:
        with (
            tempfile.TemporaryDirectory() as temporary,
            patch.object(queue_tree, "verify_queue_tree", side_effect=self.move),
        ):
            output = Path(temporary) / core.STATE_FILE
            original_source = deepcopy(self.api.responses[f"actions/runs/{SOURCE_ID}"])
            deferred = self.service(output)
            self.api.add({**self.current, "status": "completed", "conclusion": "success"}, deferred)
            self.api.history = {
                "workflow_runs": [{"id": self.current["id"]}, {"id": self.prior["id"]}],
                "total_count": 2,
            }
            self.current = self.service_run(
                SERVICE_ID + 1, CREATED + timedelta(minutes=1), "in_progress", None
            )
            self.api.add(self.current)
            self.api.target = MOVED
            self.api.nodes[0]["baseCommit"]["oid"] = MOVED
            self.api.nodes[0]["headCommit"]["oid"] = REBUILT
            self.api.responses[f"compare/{MOVED}...{REBUILT}"] = {
                "status": "ahead",
                "merge_base_commit": {"sha": MOVED},
            }
            # A new tested head belongs to a distinct synthetic CI run.
            stable_source_id = SOURCE_ID + 1
            self.api.responses[f"actions/runs/{stable_source_id}"] = {
                **deepcopy(original_source),
                "id": stable_source_id,
                "head_sha": REBUILT,
            }
            report = {
                "status": "waiting",
                "candidate_sha": REBUILT,
                "base_sha": MOVED,
                "source_head_sha": H,
                "pull_request": NUMBER,
                "scope": "merge_group",
                "state": deepcopy(self.previous["dependency_state"]),
            }
            with (
                patch.object(queue_tree, "verify_queue_tree"),
                patch.object(core, "evaluate", return_value=report) as evaluate,
                patch.object(core, "rerun_ready") as retry,
            ):
                stable = self.service(output, source_run_id=stable_source_id)
            self.assertEqual(self.api.responses[f"actions/runs/{SOURCE_ID}"], original_source)
            self.assertEqual(stable["deferred_candidates"], [])
            self.assertEqual(stable["reports"][REBUILT]["status"], "waiting")
            self.assertEqual(stable["recovery_frontier"]["predecessor_run_id"], SERVICE_ID)
            self.assertIsNone(stable["recovery_frontier"]["fence"])
            self.assert_preserved(stable)
            evaluate.assert_called_once()
            retry.assert_not_called()
            self.assertTrue(all(path == f"statuses/{REBUILT}" for path, _ in self.api.posts))

    def test_existing_fence_and_active_attempt_remain_durable_in_no_action_closure(self) -> None:
        self.previous["recovery_frontier"]["fence"] = "existing9eb uncertainty remains suspended"
        self.api.add(self.prior, self.previous)
        active = self.service_run(
            SERVICE_ID - 1, CREATED - timedelta(minutes=1), "in_progress", None
        )
        self.api.add(active)
        with (
            tempfile.TemporaryDirectory() as temporary,
            patch.object(queue_tree, "verify_queue_tree", side_effect=self.move),
        ):
            output = Path(temporary) / core.STATE_FILE
            expected_frontier = recovery.service_context(
                self.api, self.current, self.previous, C, "main", NOW
            ).frontier
            payload = self.service(output)
            self.assertEqual(payload["recovery_frontier"], expected_frontier)
            self.assert_preserved(payload)
            self.assertEqual(
                payload["recovery_frontier"]["fence"], self.previous["recovery_frontier"]["fence"]
            )
            self.assertEqual(
                payload["recovery_frontier"]["pending"],
                [{"run_id": active["id"], "run_attempt": 1}],
            )
            self.assertFalse(payload["recovery_frontier"]["authority"])
            self.assertEqual(json.loads(output.read_text()), payload)
            self.assertEqual(self.api.posts, [])

    def test_actual_failed9eb_history_is_never_retroactively_accepted(self) -> None:
        self.current = self.service_run(
            SERVICE_ID + 1, CREATED + timedelta(minutes=1), "in_progress", None
        )
        self.api.add(self.service_run(SERVICE_ID, CREATED, "completed", "failure"))
        self.api.add(self.current)
        with (
            tempfile.TemporaryDirectory() as temporary,
            patch.object(queue_tree, "verify_queue_tree", side_effect=self.move),
        ):
            payload = self.service(Path(temporary) / core.STATE_FILE)
        self.assertIn("may have effected", payload["recovery_frontier"]["fence"])
        self.assert_preserved(payload)
        self.assertEqual(self.api.posts, [])
        self.api.add({**self.current, "status": "completed", "conclusion": "success"}, payload)
        later = self.service_run(
            SERVICE_ID + 2, CREATED + timedelta(minutes=2), "in_progress", None
        )
        self.api.add(later)
        advanced = recovery.service_context(self.api, later, payload, C, "main", NOW)
        self.assertEqual(advanced.frontier["fence"], payload["recovery_frontier"]["fence"])
        self.assertFalse(advanced.can_prepare)
        self.assertFalse(advanced.can_effect)

    def test_target_plus_any_other_movement_or_unknown_proof_remains_strict(self) -> None:
        cases = (
            "queue",
            "protection",
            "source",
            "title",
            "author",
            "source-repository",
            "target-branch",
            "malformed-ref",
            "foreign-ref",
            "expired",
            "tree-error",
            "generic-error",
        )
        for case in cases:
            with self.subTest(case=case):
                self.setUp()

                def mutate(*args: object, case: str = case, **kwargs: object) -> None:
                    self.move()
                    if case == "queue":
                        self.api.nodes[0]["headCommit"]["oid"] = "8" * 40
                    elif case == "protection":
                        self.api.rules[0]["parameters"]["max_entries_to_merge"] = 2
                    elif case in {
                        "source",
                        "title",
                        "author",
                        "source-repository",
                        "target-branch",
                    }:
                        pr = self.api.responses[f"pulls/{NUMBER}"]
                        if case == "source":
                            pr["head"]["sha"] = "8" * 40
                        elif case == "title":
                            pr["title"] = "feat!: changed intent"
                        elif case == "author":
                            pr["user"]["login"] = "another-actor"
                        elif case == "source-repository":
                            pr["head"]["repo"]["full_name"] = "other/repository"
                        else:
                            pr["base"]["ref"] = "other-target"
                    elif case in {"malformed-ref", "foreign-ref"}:
                        self.api.target_object = {
                            "ref": "refs/heads/main",
                            "object": {
                                "type": "commit",
                                "sha": "not-a-sha" if case == "malformed-ref" else MOVED,
                            },
                        }
                        if case == "foreign-ref":
                            self.api.target_object["ref"] = "refs/heads/another-target"
                    elif case == "expired":
                        self.api.deadline = time.monotonic() - 1
                    elif case == "tree-error":
                        raise queue_tree.QueueTreeError("independent tree proof failed")
                    else:
                        # The old target-movement message alone is never authority.
                        raise core.EligibilityError(
                            "queue target branch moved during verification; next event rechecks"
                        )

                with (
                    tempfile.TemporaryDirectory() as temporary,
                    patch.object(queue_tree, "verify_queue_tree", side_effect=mutate),
                    patch.object(core, "evaluate") as evaluate,
                    patch.object(core, "report_status") as status,
                    patch.object(core, "rerun_ready") as retry,
                    patch.object(recovery, "recover") as recover,
                ):
                    output = Path(temporary) / core.STATE_FILE
                    with self.assertRaises(core.EligibilityError) as caught:
                        self.service(output)
                    self.assertNotIsInstance(caught.exception, core.QueueTargetMoved)
                    self.assertFalse(output.exists())
                    for operation in (evaluate, status, retry, recover):
                        operation.assert_not_called()
                    self.assertEqual(self.api.posts, [])

    def test_type_only_catch_rejects_unbound_source_id_or_attempt(self) -> None:
        for field, value in (
            ("id", SOURCE_ID + 1),
            ("id", True),
            ("run_attempt", None),
            ("run_attempt", 0),
            ("run_attempt", True),
        ):
            with self.subTest(field=field, value=value):
                self.setUp()
                self.api.responses[f"actions/runs/{SOURCE_ID}"][field] = value
                with (
                    tempfile.TemporaryDirectory() as temporary,
                    patch.object(queue_tree, "verify_queue_tree", side_effect=self.move),
                ):
                    output = Path(temporary) / core.STATE_FILE
                    with self.assertRaisesRegex(
                        core.EligibilityError, "exact trusted source callback"
                    ):
                        self.service(output)
                    self.assertFalse(output.exists())
                    self.assertEqual(self.api.posts, [])

    def test_optional_movement_metadata_is_closed_bounded_and_not_other_outcome(self) -> None:
        with (
            tempfile.TemporaryDirectory() as temporary,
            patch.object(queue_tree, "verify_queue_tree", side_effect=self.move),
        ):
            payload = self.service(Path(temporary) / core.STATE_FILE)
        cases = (
            ("kind", "unknown"),
            ("stage", "queue-fallback"),
            ("target_branch", "/main"),
            ("target_branch", "main\nforged"),
            ("target_branch", "a" * 513),
            ("source_run_attempt", True),
            ("source_run_attempt", 0),
            ("tested_sha", "invalid"),
            ("after_sha", B),
            ("unknown", "unexpected"),
        )
        for field, value in cases:
            changed = deepcopy(payload)
            changed["deferred_candidates"][0]["queue_movement"][field] = value
            with self.subTest(field=field, value=value), self.assertRaises(core.EligibilityError):
                core.validate_state(
                    changed,
                    repository=self.api.repository,
                    run_id=SERVICE_ID,
                    implementation=C,
                    now=NOW,
                )
        changed = deepcopy(payload)
        changed["no_action_candidates"] = changed.pop("deferred_candidates")
        with self.assertRaises(core.EligibilityError):
            core.validate_state(
                changed,
                repository=self.api.repository,
                run_id=SERVICE_ID,
                implementation=C,
                now=NOW,
            )

    def test_target_change_in_post_effect_path_never_becomes_no_action(self) -> None:
        self.previous["reports"][N] = {
            "status": "pass",
            "candidate_sha": N,
            "base_sha": B,
            "source_head_sha": H,
            "pull_request": NUMBER,
            "scope": "merge_group",
        }
        self.api.add(self.prior, self.previous)
        report = {
            **self.previous["reports"][N],
            "state": deepcopy(self.previous["dependency_state"]),
        }
        binding = core.Candidate(B, N, NUMBER, H, "renovate[bot]", "Bot", "merge_group", "", "main")

        def attempted(
            api: QueueRaceAPI,
            candidate: core.Candidate,
            ledger: dict[str, int],
            *args: object,
            **kwargs: object,
        ) -> None:
            ledger[candidate.head] = SOURCE_ID  # Existing before-request reservation contract.
            api.posts.append((f"actions/runs/{SOURCE_ID}/rerun-failed-jobs", {}))
            raise core.QueueTargetMoved(B, MOVED, binding)

        with (
            tempfile.TemporaryDirectory() as temporary,
            patch.object(queue_tree, "verify_queue_tree"),
            patch.object(core, "evaluate", return_value=report),
            patch.object(core, "rerun_ready", side_effect=attempted),
        ):
            payload = self.service(Path(temporary) / core.STATE_FILE)
        self.assertEqual(payload["deferred_candidates"], [])
        self.assertIn(N, payload["reports"])
        self.assertEqual(payload["reruns"][N], SOURCE_ID)
        self.assertTrue(payload["infrastructure_errors"])
        self.assertIn((f"actions/runs/{SOURCE_ID}/rerun-failed-jobs", {}), self.api.posts)

    def test_main_persists_terminal_deferred_artifact_and_exits_successfully(self) -> None:
        real_service = core.service

        def clocked(*args: object, **kwargs: object) -> dict[str, Any]:
            kwargs["now"] = NOW
            return real_service(*cast("Any", args), **cast("Any", kwargs))

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            event = root / "event.json"
            event.write_text(json.dumps({"action": "completed", "workflow_run": {"id": SOURCE_ID}}))
            output, summary = root / core.STATE_FILE, root / "summary.md"
            with (
                patch.object(queue_tree, "verify_queue_tree", side_effect=self.move),
                patch.object(core, "GitHub", return_value=self.api),
                patch.object(core, "service", side_effect=clocked),
                patch.dict(os.environ, {"GH_TOKEN": "not-a-real-token"}),
                patch.object(core, "evaluate") as evaluate,
                contextlib.redirect_stdout(io.StringIO()),
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
                        str(SERVICE_ID),
                        "--run-attempt",
                        "1",
                        "--output",
                        str(output),
                        "--summary",
                        str(summary),
                    ]
                )
            self.assertEqual(code, 0)
            payload = json.loads(output.read_text())
            self.assertEqual(payload["reports"], {})
            self.assertIn("Deferred PR", summary.read_text())
            self.assert_preserved(payload)
            evaluate.assert_not_called()
            self.assertEqual(self.api.posts, [])

    def test_final_ref_movement_rechecks_all_other_facts_and_deadline(self) -> None:
        for fault in (None, "late-source", "late-protection", "late-queue", "late-expired"):
            with self.subTest(fault=fault):
                self.setUp()
                reads = {"target": 0, "source": 0, "rules": 0, "queue": 0}
                original_get, original_items, original_request = (
                    self.api.get,
                    self.api.items,
                    self.api.request,
                )

                def get(
                    path: str,
                    fault: str | None = fault,
                    reads: dict[str, int] = reads,
                    original_get: Callable[[str], dict[str, Any]] = original_get,
                ) -> dict[str, Any]:
                    if path == "git/ref/heads/main":
                        reads["target"] += 1
                        if reads["target"] == 3:
                            self.api.target = MOVED
                    if path == f"pulls/{NUMBER}":
                        reads["source"] += 1
                        if reads["source"] == 3:
                            if fault == "late-source":
                                self.api.responses[path]["head"]["sha"] = "8" * 40
                            elif fault == "late-expired":
                                self.api.deadline = time.monotonic() - 1
                    return original_get(path)

                def items(
                    path: str,
                    fault: str | None = fault,
                    reads: dict[str, int] = reads,
                    original_items: Callable[[str], list[dict[str, Any]]] = original_items,
                ) -> list[dict[str, Any]]:
                    if path.startswith("rules/"):
                        reads["rules"] += 1
                        if fault == "late-protection" and reads["rules"] == 3:
                            self.api.rules[0]["parameters"]["max_entries_to_merge"] = 2
                    return original_items(path)

                def request(
                    method: str,
                    path: str,
                    data: dict[str, Any] | None = None,
                    fault: str | None = fault,
                    reads: dict[str, int] = reads,
                    original_request: Callable[
                        [str, str, dict[str, Any] | None], object
                    ] = original_request,
                ) -> object:
                    if path == "/graphql":
                        reads["queue"] += 1
                        if fault == "late-queue" and reads["queue"] == 3:
                            self.api.nodes = []
                    return original_request(method, path, data)

                with (
                    tempfile.TemporaryDirectory() as temporary,
                    patch.object(queue_tree, "verify_queue_tree"),
                    patch.object(self.api, "get", side_effect=get),
                    patch.object(self.api, "items", side_effect=items),
                    patch.object(self.api, "request", side_effect=request),
                    patch.object(core, "evaluate") as evaluate,
                ):
                    output = Path(temporary) / core.STATE_FILE
                    if fault is None:
                        payload = self.service(output)
                        self.assertEqual(
                            payload["deferred_candidates"][0]["queue_movement"]["after_sha"], MOVED
                        )
                    else:
                        with self.assertRaises(core.EligibilityError) as caught:
                            self.service(output)
                        self.assertNotIsInstance(caught.exception, core.QueueTargetMoved)
                        self.assertFalse(output.exists())
                    evaluate.assert_not_called()
                    self.assertEqual(self.api.posts, [])
