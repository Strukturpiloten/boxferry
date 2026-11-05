"""Approved N/B-only handoff from a proved unchanged historical native caller."""

from __future__ import annotations

import json
import unittest
from copy import deepcopy
from dataclasses import replace
from pathlib import Path
from typing import Any
from unittest.mock import patch

import test_recovery as fixture

from boxferry_ci import eligibility as core
from boxferry_ci import recovery as subject

M, B, H, N, C = fixture.P, fixture.B, fixture.H, fixture.N, fixture.C
REPOSITORY = "Strukturpiloten/boxferry"
ROOT = Path(__file__).resolve().parents[2]


class NativeHandoffTests(unittest.TestCase):
    def setUp(self) -> None:
        seed = fixture.RecoveryTests()
        seed.setUp()
        self.api = seed.api
        self.api.repository = REPOSITORY
        self.api.timeline = json.loads(
            json.dumps(self.api.timeline).replace(fixture.REPOSITORY, REPOSITORY)
        )
        self.api.responses = json.loads(
            json.dumps(self.api.responses).replace(fixture.REPOSITORY, REPOSITORY)
        )
        self.original = self.api.responses["actions/runs/50"]
        self.original["referenced_workflows"] = []
        self.api.responses["actions/runs/50/artifacts?per_page=100"] = {
            "total_count": 2,
            "artifacts": [{"id": 2, "name": "native-test-evidence"}, {"id": 3, "name": "coverage"}],
        }
        self.candidate = fixture.CANDIDATE
        self.report = fixture.passing(self.candidate)
        self.reports = {N: self.report}
        self.reservations: dict[str, Any] = {}
        self.api.responses["pulls/17"] = {
            "number": 17,
            "state": "open",
            "draft": False,
            "mergeable": True,
            "merge_commit_sha": N,
            "user": {"login": "renovate[bot]", "type": "Bot"},
            "base": {"ref": "main", "sha": B, "repo": {"full_name": REPOSITORY}},
            "head": {
                "ref": "renovate/lock-file-maintenance",
                "sha": H,
                "repo": {"full_name": REPOSITORY},
            },
        }
        self.native = json.loads(
            (ROOT / "automation/tests/fixtures/native-box-caller.json").read_text()
        )["caller"].encode()
        self.current = json.loads(
            (ROOT / "automation/tests/fixtures/recovery-callers.json").read_text()
        )["repositories"]["boxferry"]["caller"].encode()
        self.old_manifest = json.dumps(
            {
                "schema": 1,
                "interface": 1,
                "repository": REPOSITORY,
                "revision": "7" * 40,
                "policy": "scripts/validation-policy.json",
            }
        ).encode()
        self.new_manifest = json.dumps(
            {
                "schema": 1,
                "interface": 1,
                "repository": REPOSITORY,
                "revision": C,
                "policy": "scripts/validation-policy.json",
            }
        ).encode()
        self.native_files = {
            ".github/workflows/ci.yml": self.native,
            ".github/automation.json": self.old_manifest,
            "Cargo.lock": b"old lock",
            "scripts/validation-policy.json": b'{"schema": 1}',
        }
        self.source_files = {**self.native_files, "Cargo.lock": b"new lock"}
        self.current_files = {
            **self.source_files,
            ".github/workflows/ci.yml": self.current,
            ".github/automation.json": self.new_manifest,
        }
        self.api.files(M, self.native_files)
        self.api.files(H, self.source_files)
        for sha in (B, N):
            self.api.files(sha, self.current_files)
        self.commit: dict[str, Any] = {
            "sha": H,
            "author": {"login": "renovate[bot]", "type": "Bot"},
            "committer": {"login": "web-flow", "type": "User"},
            "commit": {"verification": {"verified": True, "reason": "valid"}},
        }
        self.comparison: dict[str, Any] = {
            "status": "diverged",
            "base_commit": {"sha": B},
            "merge_base_commit": {"sha": M},
            "ahead_by": 1,
            "behind_by": 1,
            "total_commits": 1,
            "commits": [self.commit],
            "files": [{"filename": "Cargo.lock", "status": "modified"}],
        }
        self.api.responses[f"compare/{B}...{H}"] = self.comparison
        for descendant in (B, H):
            self.api.responses[f"compare/{M}...{descendant}"] = {
                "status": "ahead",
                "merge_base_commit": {"sha": M},
            }
        self.api.responses[f"commits/{H}"] = self.commit
        self.jobs = self.api.responses["actions/runs/50/attempts/1/jobs?per_page=100"]["jobs"]
        self.jobs[0].update(
            name="Lockfile release age",
            run_id=50,
            run_attempt=1,
            check_run_url=f"https://api.github.com/repos/{REPOSITORY}/check-runs/801",
            steps=[
                {"name": "Check out policy", "status": "completed", "conclusion": "success"},
                {
                    "name": "Reject newly introduced dependencies younger than three days",
                    "status": "completed",
                    "conclusion": "failure",
                },
            ],
        )
        self.api.responses["check-runs/801"] = {
            "id": 801,
            "name": "Lockfile release age",
            "head_sha": H,
            "app": {"id": 15368, "slug": "github-actions"},
            "status": "completed",
            "conclusion": "failure",
        }

    def invoke(
        self,
        previous: dict[str, Any] | None = None,
        *,
        can_effect: bool | None = None,
        can_prepare: bool = True,
    ) -> None:
        context = subject.ServiceContext(
            previous,
            {"fence": None},
            fixture.NOW,
            101 if previous else 100,
            1,
            can_prepare,
            previous is not None if can_effect is None else can_effect,
        )
        with (
            patch.object(core, "pr_candidate", return_value=self.candidate),
            patch.object(subject, "reader_closure"),
            patch.object(subject, "require_effect") as guard,
            patch.object(subject, "historical_report") as historical,
            patch.object(core, "evaluate") as evaluate,
        ):
            subject.recover_run(
                self.api,
                50,
                self.candidate,
                self.report,
                {},
                previous,
                self.reservations,
                self.reports,
                implementation=C,
                service_run=context.run_id,
                branch="main",
                now=fixture.NOW,
                context=context,
            )
            historical.assert_not_called()
            evaluate.assert_not_called()
            self.effect_guard_calls = guard.call_count

    def prepared(self) -> dict[str, Any]:
        self.invoke()
        self.assertEqual(self.api.posts, [])
        return {
            "run_id": 100,
            "implementation_revision": C,
            "reports": deepcopy(self.reports),
            "recoveries": deepcopy(self.reservations),
            "reruns": {},
        }

    def test_actual_shapes_prepare_then_only_fixed_label_without_historical_claims(self) -> None:
        previous = self.prepared()
        entry = next(iter(self.reservations.values()))
        self.assertEqual(entry["kind"], "native-legacy-handoff")
        self.assertEqual(entry["merge_base_sha"], M)
        self.assertNotIn("hinted_candidate_sha", entry)
        self.assertNotIn("hinted_base_sha", entry)
        self.invoke(previous)
        self.assertEqual(self.api.posts, [("issues/17/labels", {"labels": [subject.REBASE_LABEL]})])
        self.assertEqual(set(self.reports), {N})
        self.assertEqual(self.effect_guard_calls, 1)
        self.assertEqual(entry["phase"], "attempted")
        self.invoke(previous)
        self.assertEqual(len(self.api.posts), 1)

    def test_present_legacy_manifest_is_not_interpreted_as_original_authority(self) -> None:
        self.old_manifest = b"opaque native-era manifest evidence\n"
        self.native_files[".github/automation.json"] = self.old_manifest
        self.source_files[".github/automation.json"] = self.old_manifest
        self.api.files(M, self.native_files)
        self.api.files(H, self.source_files)
        previous = self.prepared()
        self.invoke(previous)
        self.assertEqual(len(self.api.posts), 1)

    def test_pre_adoption_equal_base_and_no_reader_transition_refuse(self) -> None:
        self.comparison["merge_base_commit"]["sha"] = B
        with self.assertRaisesRegex(ValueError, "no genuine"):
            self.invoke()
        self.assertEqual(self.reservations, {})
        self.assertEqual(self.api.posts, [])

    def test_missing_truncated_or_malformed_artifact_listing_is_not_absence(self) -> None:
        path = "actions/runs/50/artifacts?per_page=100"
        for value in (
            None,
            {"total_count": 3, "artifacts": []},
            {"total_count": 1, "artifacts": [None]},
        ):
            with self.subTest(value=value):
                self.api.responses[path] = value
                with patch.object(subject, "native_proof") as proof, self.assertRaises(ValueError):
                    self.invoke()
                proof.assert_not_called()
        self.assertEqual(self.api.posts, [])

    def test_present_invalid_duplicate_or_wrong_scope_hints_never_fall_back(self) -> None:
        path = "actions/runs/50/artifacts?per_page=100"
        artifact = {
            "id": 1,
            "name": "dependency-candidate-hint-50-1",
            "expired": True,
            "size_in_bytes": 100,
        }
        for artifacts in (
            [artifact],
            [artifact, artifact],
            [{**artifact, "name": "dependency-candidate-hint-50-2"}],
        ):
            with self.subTest(artifacts=artifacts):
                self.api.responses[path] = {"total_count": len(artifacts), "artifacts": artifacts}
                with patch.object(subject, "native_proof") as proof, self.assertRaises(ValueError):
                    self.invoke()
                proof.assert_not_called()
        self.assertEqual(self.api.posts, [])

    def test_valid_present_hint_stays_on_the_existing_hint_path(self) -> None:
        hint = {
            "schema": 1,
            "repository": REPOSITORY,
            "run_id": 50,
            "run_attempt": 1,
            "pull_request": 17,
            "source_head_sha": H,
            "candidate_sha": "8" * 40,
            "base_sha": M,
        }
        self.api.archives[1] = fixture.archive(hint)
        self.api.responses["actions/runs/50/artifacts?per_page=100"] = {
            "total_count": 1,
            "artifacts": [
                {
                    "id": 1,
                    "name": "dependency-candidate-hint-50-1",
                    "expired": False,
                    "size_in_bytes": 500,
                }
            ],
        }
        with (
            patch.object(
                subject, "graph_proof", side_effect=subject.RecoveryRefused("hint path")
            ) as graph,
            patch.object(subject, "native_proof") as native,
        ):
            with self.assertRaisesRegex(ValueError, "hint path"):
                self.invoke()
            graph.assert_called_once()
            native.assert_not_called()

    def test_changed_source_catalog_manifest_presence_mode_or_bytes_refuses(self) -> None:
        for change in (
            "catalog",
            "manifest-absent",
            "manifest-mode",
            "manifest-bytes",
            "manifest-link",
        ):
            with self.subTest(change=change):
                self.setUp()
                if change == "catalog":
                    self.source_files[".github/workflows/extra.yml"] = b"---\nname: extra\n"
                elif change == "manifest-absent":
                    self.source_files.pop(".github/automation.json")
                elif change == "manifest-bytes":
                    self.source_files[".github/automation.json"] += b"\n"
                self.api.files(H, self.source_files)
                if change in {"manifest-mode", "manifest-link"}:
                    tree = self.api.responses[f"git/trees/{H}?recursive=1"]["tree"]
                    next(item for item in tree if item["path"] == ".github/automation.json")[
                        "mode"
                    ] = "100755" if change == "manifest-mode" else "120000"
                with self.assertRaises(ValueError):
                    self.invoke()
                self.assertEqual(self.api.posts, [])

    def test_arbitrary_merge_base_or_changed_head_never_authorizes_label(self) -> None:
        for change in ("merge-base", "head", "candidate"):
            with self.subTest(change=change):
                self.setUp()
                previous = self.prepared()
                if change == "merge-base":
                    self.comparison["merge_base_commit"]["sha"] = "9" * 40
                elif change == "head":
                    self.api.responses["pulls/17"]["head"]["sha"] = "9" * 40
                else:
                    self.api.responses["pulls/17"]["merge_commit_sha"] = "9" * 40
                with self.assertRaises(ValueError):
                    self.invoke(previous)
                self.assertEqual(self.api.posts, [])
                self.assertEqual(next(iter(self.reservations.values()))["phase"], "suspended")

    def test_stale_original_run_wrong_provider_or_unrelated_failure_refuses(self) -> None:
        for change in ("head", "attempt", "provider", "other-job", "policy-step"):
            with self.subTest(change=change):
                self.setUp()
                if change == "head":
                    self.original["head_sha"] = "9" * 40
                elif change == "attempt":
                    self.original["run_attempt"] = 2
                elif change == "provider":
                    self.api.responses["check-runs/801"]["app"]["id"] = 7
                elif change == "other-job":
                    self.jobs[2]["conclusion"] = "failure"
                else:
                    self.jobs[0]["steps"][1]["name"] = "Check out policy"
                with self.assertRaises(ValueError):
                    self.invoke()
                self.assertEqual(self.api.posts, [])

    def test_waiting_failure_wrong_current_base_or_missing_prior_receipt_cannot_effect(
        self,
    ) -> None:
        for status in ("waiting", "failure"):
            self.report["status"] = status
            with self.subTest(status=status), self.assertRaisesRegex(ValueError, "current N/B"):
                self.invoke()
        self.report = fixture.passing(self.candidate)
        self.reports = {N: self.report}
        previous = self.prepared()
        previous["reports"][N]["base_sha"] = M
        self.invoke(previous)
        self.assertEqual(self.api.posts, [])
        previous["reports"] = {}
        self.invoke(previous)
        self.assertEqual(self.api.posts, [])

    def test_owned_bot_author_and_signed_webflow_profiles_are_closed(self) -> None:
        bot = {"login": "renovate[bot]", "type": "Bot"}
        self.assertTrue(subject.renovate_commit({"author": bot, "committer": bot}))
        self.assertTrue(subject.renovate_commit(self.commit))
        for change in (
            "human-author",
            "unknown-author",
            "unsigned",
            "invalid-reason",
            "human-committer",
        ):
            value = deepcopy(self.commit)
            if change == "human-author":
                value["author"] = {"login": "human", "type": "User"}
            elif change == "unknown-author":
                value["author"] = None
            elif change == "unsigned":
                value["commit"]["verification"]["verified"] = False
            elif change == "invalid-reason":
                value["commit"]["verification"]["reason"] = "unknown_key"
            else:
                value["committer"] = {"login": "human", "type": "User"}
            with self.subTest(change=change):
                self.assertFalse(subject.renovate_commit(value))

    def test_human_later_commit_unknown_tip_branch_and_product_scope_refuse(self) -> None:
        for change in ("later-human", "tip-human", "branch", "foreign", "product", "publication"):
            with self.subTest(change=change):
                self.setUp()
                if change == "later-human":
                    self.comparison["commits"].append(
                        {
                            "sha": "9" * 40,
                            "author": {"login": "human", "type": "User"},
                            "committer": {"login": "human", "type": "User"},
                        }
                    )
                    self.comparison.update(total_commits=2, ahead_by=2)
                elif change == "tip-human":
                    self.api.responses[f"commits/{H}"] = {**self.commit, "author": None}
                elif change == "branch":
                    self.api.responses["pulls/17"]["head"]["ref"] = "human/maintenance"
                elif change == "foreign":
                    self.api.responses["pulls/17"]["head"]["repo"]["full_name"] = "foreign/repo"
                else:
                    self.comparison["files"][0]["filename"] = (
                        "crates/core/src/lib.rs"
                        if change == "product"
                        else ".github/workflows/release.yml"
                    )
                with self.assertRaises(ValueError):
                    self.invoke()
                self.assertEqual(self.api.posts, [])

    def test_consumed_normal_ledger_label_history_and_transport_uncertainty_never_replay(
        self,
    ) -> None:
        previous = self.prepared()
        previous["reruns"] = {"8" * 40: 50}
        self.invoke(previous)
        self.assertEqual(self.api.posts, [])
        previous["reruns"] = {}
        timeline = self.api.timeline["data"]["repository"]
        timeline["nameWithOwner"] = REPOSITORY
        timeline["pullRequest"]["timelineItems"]["nodes"] = [
            {
                "__typename": "UnlabeledEvent",
                "createdAt": "2026-11-04T11:00:00Z",
                "label": {"name": "rebase"},
            }
        ]
        with self.assertRaises(ValueError):
            self.invoke(previous)
        self.assertEqual(self.api.posts, [])
        self.setUp()
        previous = self.prepared()
        self.api.fail_post = True
        self.invoke(previous)
        self.assertEqual(next(iter(self.reservations.values()))["phase"], "uncertain")
        self.invoke(previous)
        self.assertEqual(len(self.api.posts), 1)

    def test_context_without_prepare_or_effect_authority_cannot_label(self) -> None:
        self.invoke(can_prepare=False)
        self.assertEqual(self.reservations, {})
        previous = self.prepared()
        self.invoke(previous, can_effect=False)
        self.assertEqual(self.api.posts, [])

    def test_dependency_manifest_version_only_scope_rejects_product_metadata(self) -> None:
        self.assertTrue(
            subject.version_only({"dependencies": {"serde": "1"}}, {"dependencies": {"serde": "2"}})
        )
        self.assertTrue(
            subject.version_only(
                {"devDependencies": {"test": "1"}}, {"devDependencies": {"test": "2"}}
            )
        )
        self.assertFalse(
            subject.version_only({"package": {"version": "1"}}, {"package": {"version": "2"}})
        )
        self.assertFalse(
            subject.version_only({"scripts": {"build": "safe"}}, {"scripts": {"build": "changed"}})
        )

    def test_base_churn_keeps_one_budget_until_current_completed_receipt_matches(self) -> None:
        previous = self.prepared()
        original_key = next(iter(self.reservations))
        original_proof = self.reservations[original_key]["transition_fingerprint"]
        newer_base, newer_candidate = "1" * 40, "2" * 40
        self.candidate = replace(self.candidate, base=newer_base, head=newer_candidate)
        self.report = fixture.passing(self.candidate)
        self.reports = {newer_candidate: self.report}
        for sha in (newer_base, newer_candidate):
            self.api.files(sha, self.current_files)
        self.api.responses[f"compare/{newer_base}...{H}"] = {
            **self.comparison,
            "base_commit": {"sha": newer_base},
        }
        self.api.responses[f"compare/{M}...{newer_base}"] = {
            "status": "ahead",
            "merge_base_commit": {"sha": M},
        }
        self.api.responses[f"git/commits/{newer_candidate}"] = {
            "sha": newer_candidate,
            "parents": [{"sha": newer_base}, {"sha": H}],
        }
        self.api.responses["pulls/17"]["merge_commit_sha"] = newer_candidate
        self.api.responses["pulls/17"]["base"]["sha"] = newer_base
        self.invoke(previous)
        self.assertEqual(self.api.posts, [])
        self.assertEqual(set(self.reservations), {original_key})
        self.assertEqual(self.reservations[original_key]["transition_fingerprint"], original_proof)
        previous["reports"] = deepcopy(self.reports)
        previous["recoveries"] = deepcopy(self.reservations)
        self.invoke(previous)
        self.assertEqual(self.api.posts, [("issues/17/labels", {"labels": [subject.REBASE_LABEL]})])

    def test_effect_time_head_recheck_refuses_drift_after_initial_proof(self) -> None:
        previous = self.prepared()
        original = subject.native_proof
        count = 0

        def drift(
            api: core.GitHub,
            run: dict[str, Any],
            candidate: core.Candidate,
            implementation: str,
            branch: str,
        ) -> dict[str, str]:
            nonlocal count
            result = original(api, run, candidate, implementation, branch)
            count += 1
            if count == 1:
                self.api.responses["pulls/17"]["head"]["sha"] = "9" * 40
            return result

        with patch.object(subject, "native_proof", side_effect=drift):
            with self.assertRaises(ValueError):
                self.invoke(previous)
        self.assertEqual(self.api.posts, [])
        self.assertEqual(next(iter(self.reservations.values()))["phase"], "suspended")

    def test_current_only_reservation_schema_excludes_invented_historical_identity(self) -> None:
        previous = self.prepared()
        payload: dict[str, Any] = {
            **previous,
            "schema": 1,
            "repository": REPOSITORY,
            "workflow_path": core.WORKFLOW_PATH,
            "run_attempt": 1,
            "observed_at": fixture.NOW.isoformat(),
            "dependency_state": {"schema_version": 1, "first_seen": {}},
        }
        self.assertIs(
            core.validate_state(
                payload, repository=REPOSITORY, run_id=100, implementation=C, now=fixture.NOW
            ),
            payload,
        )
        next(iter(payload["recoveries"].values()))["hinted_candidate_sha"] = "9" * 40
        with self.assertRaises(ValueError):
            core.validate_state(
                payload, repository=REPOSITORY, run_id=100, implementation=C, now=fixture.NOW
            )

    def test_exact_manifest_absence_in_native_baseline_is_supported_but_not_required(self) -> None:
        self.native_files.pop(".github/automation.json")
        self.source_files.pop(".github/automation.json")
        self.api.files(M, self.native_files)
        self.api.files(H, self.source_files)
        previous = self.prepared()
        self.invoke(previous)
        self.assertEqual(len(self.api.posts), 1)

    def test_native_handoff_rejects_unproved_skipped_required_job(self) -> None:
        self.jobs[2]["conclusion"] = "skipped"
        with self.assertRaisesRegex(ValueError, "unsupported job outcomes"):
            self.invoke()
        self.assertEqual(set(self.reports), {N})
        self.assertEqual(self.api.posts, [])
        self.assertEqual(self.reservations, {})
