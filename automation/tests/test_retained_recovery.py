"""Authored retained-graph simulations; recorded skips are shape-only negative evidence."""

from __future__ import annotations

import base64
import hashlib
import json
import sys
import tempfile
import unittest
from collections.abc import Callable
from copy import deepcopy
from pathlib import Path
from typing import Any
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

import test_recovery as authored_cases
from test_recovery import (
    NOW,
    OLD_MERGE,
    REPOSITORY,
    B,
    C,
    H,
    P,
    archive,
    passing,
)

from boxferry_ci import eligibility as core
from boxferry_ci import recovery as subject

FIXTURE = Path(__file__).parent / "fixtures/recovery/static-pr-skips-20261107/index.json"


class RetainedOriginalTests(unittest.TestCase):
    def setUp(self) -> None:
        authored = authored_cases.RecoveryTests()
        authored.setUp()
        self.api = authored.api
        self.original = authored.original
        self.jobs = authored.jobs
        self.check = authored.check
        self.pull: dict[str, Any] = {
            "number": 17,
            "state": "open",
            "draft": False,
            "mergeable": True,
            "merge_commit_sha": OLD_MERGE,
            "title": "chore(deps): update authored dependency",
            "user": {"login": "renovate[bot]", "type": "Bot"},
            "base": {"ref": "main", "sha": P, "repo": {"full_name": REPOSITORY}},
            "head": {
                "sha": H,
                "ref": "renovate/authored-dependency",
                "repo": {"full_name": REPOSITORY},
            },
        }
        self.api.responses["pulls/17"] = self.pull
        self.api.responses["git/ref/heads/main"] = {
            "ref": "refs/heads/main",
            "object": {"type": "commit", "sha": B},
        }
        self.api.responses[f"compare/{B}...{OLD_MERGE}"] = {
            "status": "diverged",
            "merge_base_commit": {"sha": P},
        }
        bot = {"login": "renovate[bot]", "type": "Bot"}
        commit = {"sha": H, "author": bot, "committer": bot}
        self.api.responses[f"commits/{H}"] = commit
        self.api.responses[f"compare/{P}...{H}"] = {
            "status": "ahead",
            "base_commit": {"sha": P},
            "merge_base_commit": {"sha": P},
            "ahead_by": 1,
            "total_commits": 1,
            "commits": [commit],
            "files": [{"filename": "Cargo.lock", "status": "modified"}],
        }
        # Preserve the existing authored caller fixture, not an executed workflow.
        self.files = {
            path: base64.b64decode(self.api.responses[f"contents/{path}?ref={P}"]["content"])
            for path in (".github/workflows/ci.yml", ".github/automation.json")
        }
        self.files["scripts/validation-policy.json"] = json.dumps(
            {"schema": 2, "jobs": ["rust", "lockfile-release-age"]}
        ).encode()
        self.files["Cargo.lock"] = b'[[package]]\nname = "authored"\nversion = "1.0.0"\n'
        for sha in (P, B, H, OLD_MERGE):
            values = dict(self.files)
            if sha in {H, OLD_MERGE}:
                values["Cargo.lock"] = values["Cargo.lock"].replace(b'"1.0.0"', b'"2.0.0"')
            self.api.files(sha, values)
        self.api.responses[f"actions/workflows/ci.yml/runs?head_sha={H}&per_page=20"] = {
            "workflow_runs": [self.original]
        }
        self.state: dict[str, Any] = {
            "schema_version": 1,
            "first_seen": {"1" * 64: "2026-11-01T12:00:00Z"},
        }
        self.reservations: dict[str, Any] = {}
        self.reports: dict[str, Any] = {}

    def call(
        self, previous: dict[str, Any] | None = None, *, status: str = "pass"
    ) -> dict[str, Any]:
        with (
            patch.object(subject, "reader_closure"),
            patch.object(subject, "require_effect"),
            patch.object(
                core,
                "evaluate",
                side_effect=lambda _api, candidate, state, _now: {
                    **passing(candidate),
                    "status": status,
                    "state": deepcopy(state),
                },
            ),
        ):
            return subject.recover_retained(
                self.api,
                17,
                self.state,
                previous,
                self.reservations,
                self.reports,
                implementation=C,
                service_run=101 if previous else 100,
                branch="main",
                now=NOW,
                context=subject.ServiceContext(
                    previous,
                    {"fence": None},
                    NOW,
                    101 if previous else 100,
                    1,
                    True,
                    previous is not None,
                ),
            )

    def prepared(self) -> dict[str, Any]:
        self.state = self.call()
        return {"run_id": 100, "reports": deepcopy(self.reports), "reruns": {}}

    def effects(self) -> list[tuple[str, dict[str, Any]]]:
        return [(path, data) for path, data in self.api.posts if not path.startswith("statuses/")]

    def source_fork(self, files: dict[str, bytes] | None = None) -> str:
        fork = "7" * 40
        self.api.responses[f"compare/{P}...{H}"].update(
            status="diverged", merge_base_commit={"sha": fork}
        )
        for head in (P, H):
            self.api.responses[f"compare/{fork}...{head}"] = {
                "status": "ahead",
                "merge_base_commit": {"sha": fork},
            }
        self.api.files(fork, self.files if files is None else files)
        return fork

    def test_source_fork_may_predate_tested_p_without_relabelling_reports(self) -> None:
        fork = self.source_fork()
        self.assertNotEqual(fork, P)
        previous = self.prepared()
        self.call(previous)
        self.assertEqual(set(self.reports), {OLD_MERGE})
        self.assertEqual(self.reports[OLD_MERGE]["base_sha"], P)
        self.assertNotIn(fork, self.reports)
        self.assertEqual(self.effects(), [("actions/runs/50/rerun-failed-jobs", {})])

    def test_source_fork_anchor_ancestry_and_same_closed_catalog_are_required(self) -> None:
        for mutation in ("anchor", "missing", "status", "ancestry-p", "ancestry-h", "catalog"):
            with self.subTest(mutation=mutation):
                self.setUp()
                fork = self.source_fork()
                comparison = self.api.responses[f"compare/{P}...{H}"]
                if mutation == "anchor":
                    comparison["base_commit"]["sha"] = B
                elif mutation == "missing":
                    comparison["merge_base_commit"] = None
                elif mutation == "status":
                    comparison["status"] = "ahead"
                elif mutation.startswith("ancestry"):
                    head = P if mutation.endswith("p") else H
                    self.api.responses[f"compare/{fork}...{head}"]["status"] = "diverged"
                else:
                    files = dict(self.files)
                    files[".github/workflows/ci.yml"] += b"\n# different source-fork catalog"
                    self.api.files(fork, files)
                with self.assertRaises(ValueError):
                    self.call()
                self.assertEqual(self.reports, {})
                self.assertEqual(self.reservations, {})
                self.assertEqual(self.effects(), [])

    def test_positive_probe_is_historical_while_live_resolver_still_refuses(self) -> None:
        with self.assertRaises(core.EligibilityError):
            core.pr_candidate(self.api, 17)
        binding = subject.probe_retained(self.api, 17, "main", run=self.original)
        self.assertIsNotNone(binding)
        assert binding is not None
        self.assertEqual(
            (binding.historical.base, binding.historical.head, binding.target), (P, OLD_MERGE, B)
        )
        self.assertFalse(core.still_current(self.api, binding.historical, "main"))
        self.assertEqual(self.reports, {})
        self.assertEqual(self.effects(), [])

    def test_hint_absence_does_not_consume_normal_live_pr_observations(self) -> None:
        self.api.responses["actions/runs/50/artifacts?per_page=100"] = {
            "total_count": 0,
            "artifacts": [],
        }
        get = self.api.get

        def without_pr_read(path: str) -> dict[str, Any]:
            if path == "pulls/17" or path == "git/ref/heads/main":
                raise AssertionError("ordinary no-hint resolution must keep its original reads")
            return get(path)

        with patch.object(self.api, "get", side_effect=without_pr_read):
            self.assertIsNone(subject.probe_retained(self.api, 17, "main", run=self.original))

    def test_probe_observed_movement_cannot_be_hidden_by_restarting_resolution(self) -> None:
        get, reads = self.api.get, 0

        def moving(path: str) -> dict[str, Any]:
            nonlocal reads
            result = get(path)
            if path == "pulls/17":
                reads += 1
                if reads == 2:
                    result["base"]["ref"] = "release/next"
            return result

        with patch.object(self.api, "get", side_effect=moving):
            with self.assertRaises(core.EligibilityError):
                subject.probe_retained(self.api, 17, "main", run=self.original)
        self.assertEqual(self.reports, {})
        self.assertEqual(self.reservations, {})
        self.assertEqual(self.effects(), [])

    def test_prepared_original_receipt_then_one_effect_survives_base_churn(self) -> None:
        previous = self.prepared()
        self.assertEqual(set(self.reports), {OLD_MERGE})
        self.assertTrue(self.reports[OLD_MERGE]["protected_queue_required"])
        self.assertEqual(self.effects(), [])
        newer = "8" * 40
        self.api.responses["git/ref/heads/main"]["object"]["sha"] = newer
        self.api.responses[f"compare/{P}...{newer}"] = {
            "status": "ahead",
            "merge_base_commit": {"sha": P},
        }
        self.api.responses[f"compare/{newer}...{OLD_MERGE}"] = {
            "status": "diverged",
            "merge_base_commit": {"sha": P},
        }
        self.api.files(newer, self.files)
        self.call(previous)
        self.call(previous)
        self.assertEqual(self.effects(), [("actions/runs/50/rerun-failed-jobs", {})])
        self.assertEqual(next(iter(self.reservations.values()))["phase"], "attempted")
        self.assertEqual(self.state["first_seen"], {"1" * 64: "2026-11-01T12:00:00Z"})

    def test_located_o_never_approves_actual_x_source_h_or_future_queue(self) -> None:
        previous = self.prepared()
        self.call(previous)
        actual_x = "9" * 40
        self.api.responses[f"commits/{actual_x}/statuses?per_page=100"] = []
        self.assertIsNone(
            core.candidate_receipt(self.api, actual_x, C, "main", NOW, expected_base_sha=P)
        )
        self.assertNotIn(actual_x, self.reports)
        self.assertNotIn(H, self.reports)
        self.assertTrue(
            all(
                path not in {f"statuses/{actual_x}", f"statuses/{H}"}
                and data.get("context") != "Required CI"
                for path, data in self.api.posts
            )
        )

    def test_waiting_age_never_consumes_retry_or_first_seen(self) -> None:
        previous = self.prepared()
        before = deepcopy(self.state)
        self.state = self.call(previous, status="waiting")
        self.assertEqual(self.effects(), [])
        self.assertEqual(next(iter(self.reservations.values()))["phase"], "prepared")
        self.assertEqual(self.state, before)

    def test_invalid_pr_or_stale_graph_never_grants_probe_authority(self) -> None:
        mutations = (
            lambda: self.pull.update(mergeable=None),
            lambda: self.pull.update(mergeable=False),
            lambda: self.pull.update(state="closed"),
            lambda: self.pull.update(draft=True),
            lambda: self.pull.update(title=None),
            lambda: self.pull["user"].update(login="human", type="User"),
            lambda: self.pull["head"]["repo"].update(full_name="other/repository"),
            lambda: self.pull["head"].update(ref="feature/not-renovate"),
            lambda: self.api.responses[f"git/commits/{OLD_MERGE}"].update(
                parents=[{"sha": H}, {"sha": P}]
            ),
            lambda: self.api.responses[f"compare/{B}...{OLD_MERGE}"].update(status="ahead"),
            lambda: self.api.responses[f"compare/{P}...{B}"].update(status="diverged"),
        )
        for index, mutate in enumerate(mutations):
            with self.subTest(index=index):
                self.setUp()
                mutate()
                self.assertIsNone(subject.probe_retained(self.api, 17, "main", run=self.original))
                self.assertEqual(self.reservations, {})
                self.assertEqual(self.reports, {})
                self.assertEqual(self.effects(), [])

    def test_original_failures_provider_attempt_and_hint_remain_mandatory(self) -> None:
        mutations = (
            lambda: self.jobs[2].update(conclusion="failure"),
            lambda: self.jobs[2].update(conclusion="skipped"),
            lambda: self.jobs[2].update(conclusion="cancelled"),
            lambda: self.check["app"].update(id=999),
            lambda: self.original.update(run_attempt=2),
            lambda: self.api.responses["actions/runs/50/attempts/1/jobs?per_page=100"].update(
                total_count=4
            ),
            lambda: self.api.archives.update(
                {
                    1: archive(
                        {
                            "schema": 1,
                            "repository": REPOSITORY,
                            "run_id": 50,
                            "run_attempt": 1,
                            "pull_request": 17,
                            "source_head_sha": H,
                            "candidate_sha": "9" * 40,
                            "base_sha": P,
                        }
                    )
                }
            ),
        )
        for index, mutate in enumerate(mutations):
            with self.subTest(index=index):
                self.setUp()
                mutate()
                with self.assertRaises(ValueError):
                    self.call()
                self.assertEqual(self.reservations, {})
                self.assertEqual(self.reports, {})
                self.assertEqual(self.effects(), [])

    def test_closed_catalog_policy_mode_and_commit_ownership_cannot_be_relaxed(self) -> None:
        for mutation in ("catalog", "policy", "mode", "human", "unrelated"):
            with self.subTest(mutation=mutation):
                self.setUp()
                if mutation in {"catalog", "policy"}:
                    files = dict(self.files)
                    path = (
                        ".github/workflows/ci.yml"
                        if mutation == "catalog"
                        else "scripts/validation-policy.json"
                    )
                    files[path] += b"\n# changed" if mutation == "catalog" else b" "
                    self.api.files(H, files)
                elif mutation == "mode":
                    tree = self.api.responses[f"git/trees/{H}?recursive=1"]["tree"]
                    next(item for item in tree if item["path"] == "Cargo.lock")["mode"] = "100755"
                elif mutation == "human":
                    self.api.responses[f"commits/{H}"]["author"] = {
                        "login": "human",
                        "type": "User",
                    }
                else:
                    self.api.responses[f"compare/{P}...{H}"]["files"][0]["filename"] = "README.md"
                with self.assertRaises(ValueError):
                    self.call()
                self.assertEqual(self.reports, {})
                self.assertEqual(self.reservations, {})
                self.assertEqual(self.effects(), [])

    def test_serialized_new_kind_and_union_preserve_identity_and_spent_budget(self) -> None:
        previous = self.prepared()
        self.call(previous)
        payload: dict[str, Any] = {
            "schema": 1,
            "repository": REPOSITORY,
            "workflow_path": core.WORKFLOW_PATH,
            "run_id": 101,
            "implementation_revision": C,
            "observed_at": NOW.isoformat(),
            "dependency_state": self.state,
            "reports": self.reports,
            "reruns": {},
            "recoveries": self.reservations,
        }
        core.validate_state(payload, repository=REPOSITORY, run_id=101, implementation=C, now=NOW)
        selected = {"reruns": {}, "recoveries": {}, "dependency_state": deepcopy(self.state)}
        merged = subject.merge_budgets(selected, [payload])
        entry = next(iter(merged["recoveries"].values()))
        self.assertEqual(entry["phase"], "attempted")
        self.assertEqual(entry["kind"], "retained-original")
        malformed = deepcopy(payload)
        next(iter(malformed["recoveries"].values())).pop("original_identity_sha256")
        with self.assertRaises(core.EligibilityError):
            core.validate_state(
                malformed, repository=REPOSITORY, run_id=101, implementation=C, now=NOW
            )
        conflict = deepcopy(payload)
        next(iter(conflict["recoveries"].values()))["original_identity_sha256"] = "8" * 64
        with self.assertRaises(ValueError):
            subject.merge_budgets(merged, [conflict])
        rendered = core.summary(
            {**payload, "state_source": "authored", "infrastructure_errors": []}
        )
        self.assertIn("fresh protected queue must pass", rendered)

    def test_normal_retry_consumption_and_ambiguous_post_do_not_reset_allowance(self) -> None:
        previous = self.prepared()
        previous["reruns"] = {"7" * 40: 50}
        self.call(previous)
        self.assertEqual(self.effects(), [])
        previous["reruns"] = {}
        self.api.fail_post = True
        self.call(previous)
        self.assertEqual(next(iter(self.reservations.values()))["phase"], "uncertain")
        self.call(previous)
        self.assertEqual(self.effects(), [("actions/runs/50/rerun-failed-jobs", {})])

    def test_title_change_suspends_existing_original_budget(self) -> None:
        previous = self.prepared()
        self.pull["title"] += " changed"
        with self.assertRaises(ValueError):
            self.call(previous)
        self.assertEqual(self.effects(), [])
        self.assertEqual(next(iter(self.reservations.values()))["phase"], "suspended")

    def test_source_or_attempt_change_before_effect_refuses(self) -> None:
        previous = self.prepared()
        real = subject.retained_proof
        count = 0

        def advance(
            api: core.GitHub, number: int, implementation: str, branch: str
        ) -> subject.RetainedBinding:
            nonlocal count
            result = real(api, number, implementation, branch)
            count += 1
            if count == 2:
                self.original["run_attempt"] = 2
            return result

        with patch.object(subject, "retained_proof", side_effect=advance):
            with self.assertRaises(ValueError):
                self.call(previous)
        self.assertEqual(self.effects(), [])
        self.assertEqual(next(iter(self.reservations.values()))["phase"], "suspended")

    def test_actual_actor_guard_refuses_cancellation_during_retained_proof(self) -> None:
        variants = (
            ("live", "in_progress", None),
            ("cancelled", "completed", "cancelled"),
            ("failed", "completed", "failure"),
            ("finished", "completed", "success"),
            ("unknown", "unknown", None),
            ("missing-status", None, None),
            ("missing-conclusion", "in_progress", None),
        )
        for kind, status, conclusion in variants:
            with self.subTest(kind=kind):
                self.setUp()
                previous = self.prepared()
                checkpoint = {
                    "id": 100,
                    "run_attempt": 1,
                    "created_at": NOW.isoformat(),
                    "event": "workflow_run",
                    "path": core.WORKFLOW_PATH,
                    "head_branch": "main",
                    "head_sha": B,
                    "status": "completed",
                    "conclusion": "success",
                    "repository": {"full_name": REPOSITORY},
                    "referenced_workflows": [
                        {"path": f"{core.SHARED_WORKFLOW_PATH}@{C}", "sha": C}
                    ],
                }
                actor = {**checkpoint, "id": 101, "status": "in_progress", "conclusion": None}
                self.api.responses["actions/runs/100"] = checkpoint
                self.api.responses["actions/runs/101"] = actor
                self.api.responses[f"compare/{B}...main"] = {"status": "ahead"}
                context = subject.ServiceContext(
                    previous, {"fence": None}, NOW, 101, 1, True, True, deepcopy(checkpoint)
                )
                real, proofs = subject.retained_proof, 0

                def proved(
                    api: core.GitHub,
                    number: int,
                    implementation: str,
                    branch: str,
                    *,
                    real: Callable[[core.GitHub, int, str, str], subject.RetainedBinding] = real,
                    actor: dict[str, Any] = actor,
                    status: str | None = status,
                    conclusion: str | None = conclusion,
                    kind: str = kind,
                ) -> subject.RetainedBinding:
                    nonlocal proofs
                    result = real(api, number, implementation, branch)
                    proofs += 1
                    if proofs == 2:
                        actor.update(status=status, conclusion=conclusion)
                        if kind == "missing-status":
                            actor.pop("status")
                        if kind == "missing-conclusion":
                            actor.pop("conclusion")
                    return result

                # Actual require_effect reads the changed fake API actor after proof.
                with (
                    patch.object(subject, "reader_closure"),
                    patch.object(subject, "retained_proof", side_effect=proved),
                    patch.object(
                        core,
                        "evaluate",
                        side_effect=lambda _api, candidate, state, _now: {
                            **passing(candidate),
                            "state": state,
                        },
                    ),
                ):

                    def recover(
                        previous: dict[str, Any] = previous,
                        context: subject.ServiceContext = context,
                    ) -> None:
                        subject.recover_retained(
                            self.api,
                            17,
                            self.state,
                            previous,
                            self.reservations,
                            self.reports,
                            implementation=C,
                            service_run=101,
                            branch="main",
                            now=NOW,
                            context=context,
                        )

                    if kind == "live":
                        recover()
                        self.assertEqual(
                            self.effects(), [("actions/runs/50/rerun-failed-jobs", {})]
                        )
                        self.assertEqual(
                            next(iter(self.reservations.values()))["phase"], "attempted"
                        )
                    else:
                        with self.assertRaisesRegex(ValueError, "effect-time actor/checkpoint"):
                            recover()
                        self.assertEqual(self.effects(), [])
                        self.assertEqual(
                            next(iter(self.reservations.values()))["phase"], "suspended"
                        )
                        self.assertFalse(context.can_effect)
                        self.assertFalse(context.can_prepare)
                        self.assertLess(len(context.frontier["fence"]), 2000)
                self.assertEqual(proofs, 2)
                self.assertEqual(set(self.reports), {OLD_MERGE})
                self.assertTrue(self.reports[OLD_MERGE]["protected_queue_required"])
                self.assertEqual(self.state["first_seen"], {"1" * 64: "2026-11-01T12:00:00Z"})
                self.assertTrue(
                    all(data.get("context") != "Required CI" for _, data in self.api.posts)
                )
                self.call(previous)  # Neither suspended nor consumed allowance resets.
                self.assertEqual(len(self.effects()), 1 if kind == "live" else 0)

    def test_retained_budget_stop_preserves_last_evaluated_scheduled_cursor(self) -> None:
        self.api.responses[""] = {"default_branch": "main"}
        self.api.responses[f"compare/{B}...main"] = {"status": "ahead"}
        self.api.responses["actions/runs/100"] = {
            "id": 100,
            "run_attempt": 1,
            "created_at": NOW.isoformat(),
            "event": "schedule",
            "path": core.WORKFLOW_PATH,
            "head_branch": "main",
            "head_sha": B,
            "repository": {"full_name": REPOSITORY},
            "referenced_workflows": [{"path": f"{core.SHARED_WORKFLOW_PATH}@{C}", "sha": C}],
        }
        self.api.responses["pulls?state=open&per_page=100&sort=updated&direction=desc"] = [
            {"number": 17},
            {"number": 18},
        ]
        self.api.responses["actions/workflows/ci.yml/runs?event=merge_group&per_page=20"] = {
            "workflow_runs": []
        }
        frontier = {
            "protocol": 1,
            "authority": False,
            "fence": None,
            "service_created_at": NOW.isoformat(),
            "pending": [],
        }
        context = subject.ServiceContext(None, frontier, NOW, 100, 1, True, False)

        def probe(
            _api: core.GitHub, number: int, _branch: str, **_kwargs: object
        ) -> subject.RetainedBinding:
            return subject.RetainedBinding(
                core.Candidate(P, OLD_MERGE, number, H, "renovate[bot]", "Bot", "pull_request"),
                B,
                "1" * 64,
            )

        def recover(
            api: core.GitHub, number: int, *args: object, **_kwargs: object
        ) -> dict[str, Any]:
            state, reports = args[0], args[3]
            assert isinstance(state, dict) and isinstance(reports, dict)
            if number == 18:
                api.requests = 451
                raise core.EligibilityError("authored request budget stop")
            reports[OLD_MERGE] = {
                **passing(
                    core.Candidate(P, OLD_MERGE, 17, H, "renovate[bot]", "Bot", "pull_request")
                ),
                "protected_queue_required": True,
            }
            return state

        with (
            tempfile.TemporaryDirectory() as temporary,
            patch.object(core, "previous_state", return_value=None),
            patch.object(subject, "service_context", return_value=context),
            patch.object(subject, "probe_retained", side_effect=probe),
            patch.object(subject, "recover_retained", side_effect=recover),
        ):
            payload = core.service(
                self.api,
                event_name="schedule",
                event={},
                implementation=C,
                run_id=100,
                output=Path(temporary) / "state.json",
                now=NOW,
            )
        self.assertEqual(payload["scan_cursor"], 17)
        self.assertEqual(set(payload["reports"]), {OLD_MERGE})
        self.assertEqual(self.effects(), [])

    def test_real_service_prepares_completed_receipt_before_one_original_effect(self) -> None:
        self.api.responses[""] = {"default_branch": "main"}
        self.api.responses[f"compare/{B}...main"] = {"status": "ahead"}
        previous = None
        with tempfile.TemporaryDirectory() as temporary:
            for identifier in (100, 101, 102, 103):
                self.api.responses[f"actions/runs/{identifier}"] = {
                    "id": identifier,
                    "run_attempt": 1,
                    "created_at": "2026-11-04T11:00:00Z",
                    "event": "workflow_run",
                    "path": core.WORKFLOW_PATH,
                    "head_branch": "main",
                    "head_sha": B,
                    "status": "in_progress",
                    "conclusion": None,
                    "repository": {"full_name": REPOSITORY},
                    "referenced_workflows": [
                        {"path": f"{core.SHARED_WORKFLOW_PATH}@{C}", "sha": C}
                    ],
                }
                self.api.history = {
                    "total_count": identifier - 99,
                    "workflow_runs": [{"id": run} for run in range(100, identifier + 1)],
                }
                with (
                    patch.object(core, "previous_state", return_value=previous),
                    patch.object(subject, "reader_closure"),
                    patch.object(
                        core,
                        "evaluate",
                        side_effect=lambda _api, candidate, state, _now: {
                            **passing(candidate),
                            "state": state,
                        },
                    ),
                    patch.object(
                        core,
                        "pr_candidate",
                        side_effect=AssertionError(
                            "a proved historical binding must not become a live Candidate"
                        ),
                    ),
                ):
                    payload = core.service(
                        self.api,
                        event_name="workflow_run",
                        event={"workflow_run": {"id": 50}},
                        implementation=C,
                        run_id=identifier,
                        output=Path(temporary) / f"{identifier}.json",
                        now=NOW,
                    )
                self.assertEqual(payload["infrastructure_errors"], [])
                if identifier == 100:
                    self.assertEqual(set(payload["reports"]), {OLD_MERGE})
                    self.assertEqual(self.effects(), [])
                else:
                    self.assertEqual(self.effects(), [("actions/runs/50/rerun-failed-jobs", {})])
                self.api.responses[f"actions/runs/{identifier}"].update(
                    status="completed", conclusion="success"
                )
                self.api.archives[identifier + 1000] = archive(payload, core.STATE_FILE)
                self.api.responses[f"actions/runs/{identifier}/artifacts?per_page=100"] = {
                    "artifacts": [
                        {
                            "id": identifier + 1000,
                            "name": core.ARTIFACT_NAME,
                            "size_in_bytes": len(self.api.archives[identifier + 1000]),
                            "expired": False,
                        }
                    ]
                }
                previous = payload
        assert previous is not None
        self.assertEqual(next(iter(previous["recoveries"].values()))["phase"], "attempted")


class StaticEventExclusionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.records = json.loads(FIXTURE.read_text())["records"]

    def exclusions(self, name: str, caller: str | None = None) -> frozenset[str]:
        jobs = (
            ["quality", "lockfile-release-age"]
            if name == "boxferry-website"
            else ["rust", "msrv", "dependencies", "api", "documentation", "lockfile-release-age"]
        )
        return subject.static_pr_exclusions(
            "Strukturpiloten/" + name,
            (caller or self.records[name]["workflow"]).encode(),
            json.dumps({"jobs": jobs}).encode(),
        )

    def test_exact_recorded_three_skips_are_only_static_shape_evidence(self) -> None:
        expected = {
            "docker-lens": {
                "Hosted prerequisite diagnostic (not native validation)",
                "Native Engine ${{ matrix.lane }}",
            },
            "boxferry-website": {"Publish validated website"},
        }
        for name, record in self.records.items():
            self.assertEqual(
                hashlib.sha256(record["workflow"].encode()).hexdigest(), record["workflow_sha256"]
            )
            self.assertFalse(record["positive_recovery_evidence"])
            self.assertEqual(self.exclusions(name), expected[name])
            observed = record["observed"]
            self.assertEqual(observed["jobs_total_count"], observed["retained_job_count"])
            self.assertEqual(
                {job["name"] for job in observed["jobs"] if job["conclusion"] == "skipped"},
                expected[name],
            )
        self.assertEqual(
            self.records["boxferry-website"]["observed"]["run"]["conclusion"], "success"
        )
        self.assertTrue(
            any(
                job["conclusion"] == "failure" and "Rust" in job["name"]
                for job in self.records["docker-lens"]["observed"]["jobs"]
            )
        )

    def test_changed_condition_name_policy_membership_or_gate_refuses(self) -> None:
        for name, record in self.records.items():
            caller = record["workflow"]
            for before, after in [
                ("github.event_name == 'push'", "github.event_name == 'pull_request'"),
                ("    name: Publish validated website", "    name: Different publisher"),
                ("Native Engine ${{ matrix.lane }}", "Native Engine prefix"),
                ('needs.pop("native-conformance")', 'needs.pop("different")'),
                ("debian11-rootful,", "unknown-lane,"),
            ]:
                if before not in caller:
                    continue
                with self.subTest(repository=name, change=before):
                    with self.assertRaises(ValueError):
                        self.exclusions(name, caller.replace(before, after))
            identifier = "publish-site" if name == "boxferry-website" else "native-conformance"
            with self.assertRaises(ValueError):
                subject.static_pr_exclusions(
                    "Strukturpiloten/" + name,
                    caller.encode(),
                    json.dumps({"jobs": [identifier]}).encode(),
                )

    def test_complete_docker_aggregate_refuses_enforcement_body_mutations(self) -> None:
        caller = self.records["docker-lens"]["workflow"]
        old_gate = subject.job_blocks(caller.encode())["check-gate"]
        mutations = (
            ('test "$NATIVE_RESULT" = success', "true"),
            ('test "$NATIVE_RESULT" = skipped', "true"),
            ('test "$SCAFFOLD_RESULT" = success', "true"),
            ("            exit 1", "            true"),
            ('"$EVENT_REF" == refs/heads/main', '"$EVENT_REF" == refs/heads/other'),
            ("NATIVE_RESULT: ${{ needs.native-conformance.result }}", "NATIVE_RESULT: success"),
            ('"$EVENT_NAME" == pull_request || "$EVENT_NAME" == merge_group', "true"),
            ("        native-conformance,", "        different-job,"),
        )
        for before, after in mutations:
            with self.subTest(change=before):
                self.assertIn(before, old_gate)
                changed_gate = old_gate.replace(before, after)
                with self.assertRaises(ValueError):
                    self.exclusions("docker-lens", caller.replace(old_gate, changed_gate))
        appended = old_gate + "      - name: Unreviewed extra step\n        run: true\n"
        with self.assertRaises(ValueError):
            self.exclusions("docker-lens", caller.replace(old_gate, appended))

    def test_complete_docker_shape_only_parameterizes_coherent_immutable_pins(self) -> None:
        caller = self.records["docker-lens"]["workflow"]
        changed = caller.replace("95f5514f00f0f27475f9a58d5e17d149c2983b03", "e" * 40)
        changed = changed.replace(
            "actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4.2.2",
            "actions/checkout@" + "a" * 40 + " # v9.8.7",
        )
        self.assertEqual(self.exclusions("docker-lens", changed), self.exclusions("docker-lens"))
        gate = subject.job_blocks(caller.encode())["check-gate"]
        for changed_gate in (
            gate.replace("ref: 95f5514f00f0f27475f9a58d5e17d149c2983b03", "ref: " + "b" * 40),
            gate.replace(
                "actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683", "actions/checkout@main"
            ),
            gate.replace("# v4.2.2", "# v4"),
        ):
            with self.assertRaises(ValueError):
                self.exclusions("docker-lens", caller.replace(gate, changed_gate))

    def test_synthetic_retained_docker_flow_reproves_closed_catalog_event_exclusions(self) -> None:
        case = RetainedOriginalTests()
        case.setUp()
        repository = "Strukturpiloten/docker-lens"
        case.api.repository = repository
        case.pull["base"]["repo"]["full_name"] = repository
        case.pull["head"]["repo"]["full_name"] = repository
        case.original.update(name="Check", path=".github/workflows/check.yml")
        case.api.responses[f"actions/workflows/check.yml/runs?head_sha={H}&per_page=20"] = {
            "workflow_runs": [case.original]
        }
        case.original["repository"]["full_name"] = repository
        case.original["head_repository"]["full_name"] = repository
        case.jobs[1]["check_run_url"] = f"https://api.github.com/repos/{repository}/check-runs/800"
        hint = {
            "schema": 1,
            "repository": repository,
            "run_id": 50,
            "run_attempt": 1,
            "pull_request": 17,
            "source_head_sha": H,
            "candidate_sha": OLD_MERGE,
            "base_sha": P,
        }
        case.api.archives[1] = archive(hint)
        case.files.pop(".github/workflows/ci.yml")
        # Explicit synthetic pin normalization only, as in recovery-callers.json.
        case.files[".github/workflows/check.yml"] = (
            self.records["docker-lens"]["workflow"]
            .replace("95f5514f00f0f27475f9a58d5e17d149c2983b03", C)
            .encode()
        )
        case.files["scripts/validation-policy.json"] = json.dumps(
            {
                "jobs": [
                    "rust",
                    "msrv",
                    "dependencies",
                    "api",
                    "documentation",
                    "lockfile-release-age",
                ]
            }
        ).encode()
        for sha in (P, B, H, OLD_MERGE):
            files = dict(case.files)
            if sha in {H, OLD_MERGE}:
                files["Cargo.lock"] = files["Cargo.lock"].replace(b'"1.0.0"', b'"2.0.0"')
            case.api.files(sha, files)
        for name in ["MSRV", "Dependencies", "API", "Documentation"]:
            case.jobs.append({"name": name, "status": "completed", "conclusion": "success"})
        for name in self.exclusions("docker-lens"):
            case.jobs.append({"name": name, "status": "completed", "conclusion": "skipped"})
        case.api.responses["actions/runs/50/attempts/1/jobs?per_page=100"]["total_count"] = len(
            case.jobs
        )
        previous = case.prepared()
        case.call(previous)
        self.assertEqual(case.effects(), [("actions/runs/50/rerun-failed-jobs", {})])
        self.assertEqual(set(case.reports), {OLD_MERGE})
        self.assertTrue(case.reports[OLD_MERGE]["protected_queue_required"])

    def test_synthetic_eligible_original_uses_source_bound_exclusions_not_plan_flags(self) -> None:
        authored = authored_cases.RecoveryTests()
        authored.setUp()
        excluded = self.exclusions("docker-lens")
        repository = "Strukturpiloten/docker-lens"
        authored.api.repository = repository
        authored.original.update(name="Check", path=".github/workflows/check.yml")
        authored.original["repository"]["full_name"] = repository
        authored.original["head_repository"]["full_name"] = repository
        authored.jobs[1]["check_run_url"] = (
            f"https://api.github.com/repos/{repository}/check-runs/800"
        )
        for name in excluded:
            authored.jobs.append({"name": name, "status": "completed", "conclusion": "skipped"})
        listing = authored.api.responses["actions/runs/50/attempts/1/jobs?per_page=100"]
        listing["total_count"] = len(authored.jobs)
        self.assertEqual(
            subject.original_run(
                authored.api,
                50,
                core.Candidate(B, OLD_MERGE, 17, H, "renovate[bot]", "Bot", "pull_request"),
                C,
                excluded_jobs=excluded,
            )["id"],
            50,
        )
        for conclusion in ("failure", "cancelled", "success", None):
            with self.subTest(conclusion=conclusion):
                authored.jobs[-1]["conclusion"] = conclusion
                with self.assertRaises(ValueError):
                    subject.original_run(
                        authored.api,
                        50,
                        core.Candidate(B, OLD_MERGE, 17, H, "renovate[bot]", "Bot", "pull_request"),
                        C,
                        excluded_jobs=excluded,
                    )
        authored.jobs[-1]["conclusion"] = "skipped"
        authored.jobs.append(
            {"name": "Dev Container image", "status": "completed", "conclusion": "skipped"}
        )
        listing["total_count"] = len(authored.jobs)
        with self.assertRaisesRegex(ValueError, "unsupported job outcomes"):
            subject.original_run(
                authored.api,
                50,
                core.Candidate(B, OLD_MERGE, 17, H, "renovate[bot]", "Bot", "pull_request"),
                C,
                excluded_jobs=excluded,
            )


class PythonRetainedProofTests(unittest.TestCase):
    """Synthetic PyPI identities/hashes only; no registry or resolver execution."""

    @staticmethod
    def lock(version: str, *, name: str = "authored-tool", digest: str = "a") -> bytes:
        return (
            f'[[package]]\nname = "{name}"\nversion = "{version}"\n'
            'source = { registry = "https://pypi.org/simple" }\n'
            f'sdist = {{ url = "https://example.invalid/authored.tar.gz", '
            f'hash = "sha256:{digest * 64}" }}\n'
        ).encode()

    def python_case(
        self, *, root: bool = False, builder: bool = False
    ) -> tuple[RetainedOriginalTests, dict[str, bytes], dict[str, bytes]]:
        case = RetainedOriginalTests()
        case.setUp()
        prefix = "" if root else "automation/"
        project, lock = prefix + "pyproject.toml", prefix + "uv.lock"
        manifest = (
            '[project]\nname = "authored"\nversion = "0.1.0"\n'
            'requires-python = ">=3.13"\ndependencies = []\n'
            '[dependency-groups]\ndev = ["authored-tool==1.0.0"]\n'
        )
        if builder:
            manifest += (
                '[build-system]\nrequires = ["authored-tool==1.0.0"]\n'
                'build-backend = "authored.backend"\n'
            )
        before = {**case.files, project: manifest.encode(), lock: self.lock("1.0.0")}
        after = {
            **before,
            project: manifest.replace("==1.0.0", "==2.0.0").encode(),
            lock: self.lock("2.0.0", digest="b"),
        }
        for sha, files in ((P, before), (B, before), (H, after), (OLD_MERGE, after)):
            case.api.files(sha, files)
        case.api.responses[f"compare/{P}...{H}"]["files"] = [
            {"filename": project, "status": "modified"},
            {"filename": lock, "status": "modified"},
        ]
        return case, before, after

    def test_root_and_automation_python_pins_and_mirrored_builder_are_supported(self) -> None:
        for root, builder in ((False, False), (True, False), (False, True)):
            with self.subTest(root=root, builder=builder):
                case, _, _ = self.python_case(root=root, builder=builder)
                previous = case.prepared()
                case.call(previous)
                self.assertEqual(case.effects(), [("actions/runs/50/rerun-failed-jobs", {})])
                self.assertEqual(set(case.reports), {OLD_MERGE})
                self.assertEqual(case.reports[OLD_MERGE]["base_sha"], P)

    def test_python_source_fork_is_not_the_original_tested_base(self) -> None:
        case, before, _ = self.python_case()
        fork = case.source_fork(before)
        previous = case.prepared()
        case.call(previous)
        self.assertEqual(case.reports[OLD_MERGE]["base_sha"], P)
        self.assertNotIn(fork, case.reports)
        self.assertEqual(case.effects(), [("actions/runs/50/rerun-failed-jobs", {})])

    def test_normal_native_default_remains_unchanged_for_python_manifest(self) -> None:
        case, _, _ = self.python_case()
        modes: dict[str, dict[str, str]] = {P: {}, H: {}}
        trees = {sha: subject.inventory(case.api, sha, modes=modes[sha]) for sha in modes}
        with self.assertRaises(ValueError):
            subject.ordinary_native_changes(
                case.api,
                case.api.get(f"compare/{P}...{H}"),
                core.Candidate(P, OLD_MERGE, 17, H, "renovate[bot]", "Bot", "pull_request"),
                P,
                trees,
                modes,
            )

    def test_exact_four_toml_locations_keep_structure_and_builder_mirror(self) -> None:
        before = {
            "project": {
                "name": "authored",
                "dependencies": ["authored-main==1.0.0"],
                "optional-dependencies": {"optional": ["authored-extra==1.0.0"]},
            },
            "dependency-groups": {
                "dev": ["authored-builder==1.0.0", {"include-group": "base"}],
                "base": ["authored-base==1.0.0"],
            },
            "build-system": {
                "requires": ["authored-builder==1.0.0"],
                "build-backend": "authored.backend",
            },
        }
        after = json.loads(json.dumps(before).replace("==1.0.0", "==2.0.0"))
        self.assertEqual(
            subject.python_pin_changes(before, after),
            {
                ("authored-main", "1.0.0", "2.0.0"),
                ("authored-extra", "1.0.0", "2.0.0"),
                ("authored-builder", "1.0.0", "2.0.0"),
                ("authored-base", "1.0.0", "2.0.0"),
            },
        )
        for pin in (
            "different==2.0.0",
            "authored-main>=2.0.0",
            "authored-main==2.*",
            "authored-main[extra]==2.0.0",
            "authored-main==2.0.0; python_version>'3.11'",
            "authored-main @ https://example.invalid/package.whl",
            "authored-main==2.0.0rc1",
        ):
            with self.subTest(pin=pin):
                invalid = deepcopy(after)
                invalid["project"]["dependencies"] = [pin]
                with self.assertRaises(ValueError):
                    subject.python_pin_changes(before, invalid)
        for mutation in ("requires-python", "includes", "metadata", "added", "builder-only"):
            with self.subTest(mutation=mutation):
                invalid = deepcopy(after)
                if mutation == "requires-python":
                    invalid["project"]["requires-python"] = ">=3.14"
                elif mutation == "includes":
                    invalid["dependency-groups"]["dev"][1] = {"include-group": "other"}
                elif mutation == "metadata":
                    invalid["project"]["name"] = "renamed"
                elif mutation == "added":
                    invalid["project"]["dependencies"].append("added==2.0.0")
                else:
                    invalid["dependency-groups"]["dev"][0] = "authored-builder==1.0.0"
                with self.assertRaises(ValueError):
                    subject.python_pin_changes(before, invalid)

    def test_same_project_unique_identity_and_hashes_not_changed_lock_bytes(self) -> None:
        mutations = (
            "comment-only",
            "unrelated",
            "missing-old",
            "both-in-both",
            "multi-after",
            "same-version-ambiguous-hash",
            "bad-hash",
            "missing-hash",
            "wrong-source",
            "wrong-lock",
            "missing-lock",
        )
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                case, before, after = self.python_case()
                lock = "automation/uv.lock"
                if mutation == "comment-only":
                    after[lock] = before[lock] + b"# only a comment changed\n"
                elif mutation == "unrelated":
                    after[lock] = before[lock] + self.lock("3.0.0", name="unrelated")
                elif mutation == "missing-old":
                    before[lock] = self.lock("1.0.0", name="unrelated")
                elif mutation == "both-in-both":
                    before[lock] += self.lock("2.0.0", digest="b")
                    after[lock] = before[lock] + b"# comment with both versions already present\n"
                elif mutation == "multi-after":
                    after[lock] += self.lock("1.0.0")
                elif mutation == "same-version-ambiguous-hash":
                    after[lock] += self.lock("2.0.0", digest="c")
                elif mutation == "bad-hash":
                    after[lock] = after[lock].replace(b"b" * 64, b"not-a-sha256")
                elif mutation == "missing-hash":
                    after[lock] = b'[[package]]\nname="authored-tool"\nversion="2.0.0"\n' + (
                        b'source={registry="https://pypi.org/simple"}\n'
                    )
                elif mutation == "wrong-source":
                    after[lock] = after[lock].replace(
                        b"https://pypi.org/simple", b"https://other.invalid"
                    )
                else:
                    items = case.api.responses[f"compare/{P}...{H}"]["files"]
                    if mutation == "wrong-lock":
                        items[1]["filename"] = "uv.lock"
                    else:
                        items.pop()
                for sha, values in ((P, before), (B, before), (H, after), (OLD_MERGE, after)):
                    case.api.files(sha, values)
                with self.assertRaises(ValueError):
                    case.call()
                self.assertEqual(case.reports, {})
                self.assertEqual(case.reservations, {})
                self.assertEqual(case.effects(), [])

    def test_builder_only_and_tool_configuration_changes_refuse_before_receipt(self) -> None:
        for mutation in ("builder-only", "backend", "tool", "requires-python"):
            with self.subTest(mutation=mutation):
                case, before, after = self.python_case(builder=True)
                project = "automation/pyproject.toml"
                if mutation == "builder-only":
                    after[project] = after[project].replace(
                        b'dev = ["authored-tool==2.0.0"]', b'dev = ["authored-tool==1.0.0"]'
                    )
                elif mutation == "backend":
                    after[project] = after[project].replace(b"authored.backend", b"other.backend")
                elif mutation == "tool":
                    after[project] += b'[tool.uv]\nrequired-version="==9.9.9"\n'
                else:
                    after[project] = after[project].replace(b">=3.13", b">=3.14")
                for sha, values in ((P, before), (B, before), (H, after), (OLD_MERGE, after)):
                    case.api.files(sha, values)
                with self.assertRaises(ValueError):
                    case.call()
                self.assertEqual(case.reports, {})
                self.assertEqual(case.effects(), [])
