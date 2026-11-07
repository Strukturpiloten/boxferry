from __future__ import annotations

import contextlib
import copy
import hashlib
import io
import json
import subprocess
import tempfile
import unittest
from collections.abc import Generator
from dataclasses import replace
from itertools import combinations
from pathlib import Path
from typing import Any
from unittest.mock import patch

from boxferry_ci import devcontainer_seed as seed
from boxferry_ci.__main__ import main
from boxferry_ci.contracts import ContractError
from boxferry_ci.gate import verify_devcontainer_gate, verify_gate
from boxferry_ci.planner import Comparison
from boxferry_ci.policy import load_policy

JOB = "devcontainer-validation"
OWNERS = (
    "Strukturpiloten/boxferry",
    "Strukturpiloten/compose-lens",
    "Strukturpiloten/podman-lens",
    "Strukturpiloten/quadlet-lens",
)


class GateFixture(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.config = self.root / "scripts/validation-policy.json"
        self.config.parent.mkdir()
        self.config.write_text(
            json.dumps(
                {
                    "schema": 2,
                    "repository": OWNERS[0],
                    "jobs": ["rust", "documentation"],
                    "profile_jobs": {
                        "prose": ["documentation"],
                        "executable-docs": ["documentation"],
                    },
                    "public_documentation_root": "docs/public/",
                    "full_documentation_roots": [],
                    "executable_documentation_roots": [],
                    "executable_documentation_paths": [],
                    "documentation_examples_manifest": None,
                    "local_commands": {
                        name: [["true"]] for name in ("prose", "executable-docs", "full")
                    },
                }
            )
        )
        self.policy = load_policy(self.config)
        self.comparison = Comparison("pull_request", "a" * 40, "b" * 40, "c" * 40)
        self.plan: dict[str, object] = {
            "schema": 3,
            "repository": OWNERS[0],
            "event": "pull_request",
            "base_sha": "a" * 40,
            "head_sha": "b" * 40,
            "tested_sha": "c" * 40,
            "profile": "prose",
            "profiles": ["prose"],
            "jobs": {"rust": False, "documentation": True},
            "reasons": ["independently expected prose fixture"],
            "local_fingerprint": None,
        }
        self.needs: dict[str, object] = {
            "validation-plan": {"result": "success"},
            "rust": {"result": "skipped"},
            "documentation": {"result": "success"},
            JOB: {"result": "success", "outputs": {"candidate_sha": "c" * 40}},
        }

    @contextlib.contextmanager
    def approved(self) -> Generator[None]:
        approvals = dict(seed.APPROVED_SEED_POLICIES)
        approvals[self.policy.repository] = frozenset(
            {hashlib.sha256(self.config.read_bytes()).hexdigest()}
        )
        with patch.object(seed, "APPROVED_SEED_POLICIES", approvals):
            yield

    def check(self) -> list[tuple[str, str]]:
        return verify_devcontainer_gate(
            self.plan,
            self.needs,
            self.policy,
            self.comparison,
            expected_plan=copy.deepcopy(self.plan),
        )

    def adopt(self) -> None:
        raw = json.loads(self.config.read_text())
        raw["jobs"].append(JOB)
        self.config.write_text(json.dumps(raw))
        self.policy = load_policy(self.config)
        self.plan["jobs"] = {"rust": False, "documentation": True, JOB: False}

    def full(self, event: str) -> None:
        revision = "c" * 40
        self.comparison = Comparison(event, revision, revision, revision)
        self.plan.update(
            event=event,
            base_sha=revision,
            head_sha=revision,
            tested_sha=revision,
            profile="full",
            profiles=["full"],
            jobs=dict.fromkeys(self.policy.jobs, True),
        )
        self.needs["rust"] = {"result": "success"}


class DevcontainerGate(GateFixture):
    def test_synthetic_policy_is_unapproved_by_the_four_owner_registry(self) -> None:
        self.assertEqual(set(seed.APPROVED_SEED_POLICIES), set(OWNERS))
        self.assertTrue(
            all(
                isinstance(hashes, frozenset) and hashes
                for hashes in seed.APPROVED_SEED_POLICIES.values()
            )
        )
        with self.assertRaisesRegex(ContractError, "unapproved pre-seed"):
            self.check()

    def test_default_gate_does_not_authorize_any_supplemental_dependency(self) -> None:
        with self.assertRaisesRegex(ContractError, "dependencies"):
            verify_gate(self.plan, self.needs, self.policy, self.comparison)
        original = {name: result for name, result in self.needs.items() if name != JOB}
        self.assertEqual(len(verify_gate(self.plan, original, self.policy, self.comparison)), 3)

    def test_recognized_seed_preserves_original_plan_policy_and_outcomes(self) -> None:
        before = copy.deepcopy((self.plan, self.needs, self.policy))
        with self.approved():
            self.assertEqual(
                self.check(),
                [
                    ("validation-plan", "success"),
                    ("rust", "skipped"),
                    ("documentation", "success"),
                    (JOB, "success"),
                ],
            )
        self.assertEqual((self.plan, self.needs, self.policy), before)

    def test_each_owner_requires_its_own_explicit_fixture_approval(self) -> None:
        for repository in OWNERS:
            with self.subTest(repository=repository):
                raw = json.loads(self.config.read_text())
                raw["repository"] = repository
                self.config.write_text(json.dumps(raw))
                self.policy = load_policy(self.config)
                self.plan["repository"] = repository
                with self.assertRaisesRegex(ContractError, "unapproved pre-seed"):
                    self.check()
                with self.approved():
                    self.assertIn((JOB, "success"), self.check())

    def test_approval_is_exact_bytes_and_cannot_be_borrowed_by_another_owner(self) -> None:
        with self.approved():
            self.config.write_text(self.config.read_text() + "\n")
            with self.assertRaisesRegex(ContractError, "unapproved pre-seed"):
                self.check()
        with self.approved():
            self.policy = replace(self.policy, repository=OWNERS[1])
            with self.assertRaisesRegex(ContractError, "unapproved pre-seed"):
                self.check()

    def test_approved_bytes_do_not_authorize_partial_in_memory_policy(self) -> None:
        with self.approved():
            self.policy = replace(self.policy, jobs=("documentation",))
            with self.assertRaisesRegex(ContractError, "differs from its approved"):
                self.check()

    def test_even_fixture_approved_malformed_policy_is_reparsed_and_rejected(self) -> None:
        for contents in ("not json", "{}", '{"schema":true}'):
            with self.subTest(contents=contents):
                self.config.write_text(contents)
                with self.approved(), self.assertRaises(ContractError):
                    self.check()

    def test_original_job_failures_and_unplanned_skips_remain_fatal(self) -> None:
        for job, result in (
            ("validation-plan", "skipped"),
            ("validation-plan", "failure"),
            ("documentation", "skipped"),
            ("documentation", "failure"),
            ("documentation", "cancelled"),
            ("rust", "failure"),
            ("rust", "cancelled"),
        ):
            with self.subTest(job=job, result=result), self.approved():
                needs = dict(self.needs, **{job: {"result": result}})
                with self.assertRaises(ContractError):
                    verify_devcontainer_gate(
                        self.plan, needs, self.policy, self.comparison, expected_plan=self.plan
                    )

    def test_every_original_dependency_and_only_the_named_extra_are_required(self) -> None:
        for job in ("validation-plan", "rust", "documentation", JOB):
            with self.subTest(missing=job), self.approved():
                needs = {name: result for name, result in self.needs.items() if name != job}
                with self.assertRaises(ContractError):
                    verify_devcontainer_gate(
                        self.plan, needs, self.policy, self.comparison, expected_plan=self.plan
                    )
        with self.approved(), self.assertRaisesRegex(ContractError, "dependencies"):
            verify_devcontainer_gate(
                self.plan,
                dict(self.needs, unknown={"result": "success"}),
                self.policy,
                self.comparison,
                expected_plan=self.plan,
            )

    def test_seed_cannot_add_new_job_to_trusted_plan_or_forge_independent_selection(self) -> None:
        fabricated = dict(self.plan, jobs={"rust": False, "documentation": True, JOB: True})
        with self.approved(), self.assertRaisesRegex(ContractError, "unknown, missing"):
            verify_devcontainer_gate(
                fabricated, self.needs, self.policy, self.comparison, expected_plan=fabricated
            )
        forged = dict(
            self.plan,
            profile="full",
            profiles=["full"],
            jobs={"rust": True, "documentation": True},
        )
        with self.approved(), self.assertRaisesRegex(ContractError, "independent"):
            verify_devcontainer_gate(
                forged, self.needs, self.policy, self.comparison, expected_plan=self.plan
            )

    def test_seed_image_missing_malformed_or_non_success_outcome_is_rejected(self) -> None:
        for outcome in (
            None,
            "success",
            {},
            {"result": None},
            {"result": True},
            {"result": "neutral"},
            {"result": "failure"},
            {"result": "cancelled"},
            {"result": "skipped"},
        ):
            with self.subTest(outcome=outcome), self.approved(), self.assertRaises(ContractError):
                verify_devcontainer_gate(
                    self.plan,
                    dict(self.needs, **{JOB: outcome}),
                    self.policy,
                    self.comparison,
                    expected_plan=self.plan,
                )

    def test_seed_success_requires_exact_tested_candidate_not_head_base_or_old_receipt(
        self,
    ) -> None:
        for outputs in (
            None,
            [],
            {},
            {"candidate_sha": None},
            {"candidate_sha": True},
            {"candidate_sha": "a" * 40},
            {"candidate_sha": "b" * 40},
            {"candidate_sha": "d" * 40},
            {"candidate_sha": "C" * 40},
            {"candidate_sha": "c" * 40 + "\n"},
        ):
            with self.subTest(outputs=outputs), self.approved(), self.assertRaises(ContractError):
                verify_devcontainer_gate(
                    self.plan,
                    dict(self.needs, **{JOB: {"result": "success", "outputs": outputs}}),
                    self.policy,
                    self.comparison,
                    expected_plan=self.plan,
                )

    def test_seeded_inventory_accepts_planned_skip_without_seed_approval(self) -> None:
        self.adopt()
        self.needs[JOB] = {"result": "skipped"}
        self.assertIn((JOB, "skipped"), self.check())

    def test_seeded_success_even_when_unselected_requires_fresh_candidate_receipt(self) -> None:
        self.adopt()
        self.assertIn((JOB, "success"), self.check())
        for outcome in (
            {"result": "success"},
            {"result": "success", "outputs": {"candidate_sha": "d" * 40}},
            {"result": "failure"},
            {"result": "cancelled"},
        ):
            with self.subTest(outcome=outcome), self.assertRaises(ContractError):
                verify_devcontainer_gate(
                    self.plan,
                    dict(self.needs, **{JOB: outcome}),
                    self.policy,
                    self.comparison,
                    expected_plan=self.plan,
                )

    def test_seeded_selected_image_cannot_skip_or_omit_identity(self) -> None:
        self.adopt()
        self.full("pull_request")
        for outcome in ({"result": "skipped"}, {"result": "success"}):
            with self.subTest(outcome=outcome), self.assertRaises(ContractError):
                verify_devcontainer_gate(
                    self.plan,
                    dict(self.needs, **{JOB: outcome}),
                    self.policy,
                    self.comparison,
                    expected_plan=self.plan,
                )

    def test_full_events_keep_every_old_job_and_image_for_both_inventories(self) -> None:
        events = ("push", "workflow_dispatch", "workflow_call", "local")
        with self.approved():
            for event in events:
                with self.subTest(seed=True, event=event):
                    self.full(event)
                    self.assertEqual(len(self.check()), 4)
        self.adopt()
        for event in events:
            with self.subTest(seed=False, event=event):
                self.full(event)
                self.assertEqual(len(self.check()), 4)

    def test_full_events_cannot_use_a_narrow_plan_during_seed(self) -> None:
        for event in ("push", "workflow_dispatch", "workflow_call"):
            self.full(event)
            self.plan.update(profile="prose", profiles=["prose"])
            with (
                self.subTest(event=event),
                self.approved(),
                self.assertRaisesRegex(ContractError, "must be complete"),
            ):
                self.check()

    def test_nonowners_are_rejected_even_with_seeded_inventory(self) -> None:
        self.adopt()
        for repository in (
            "Strukturpiloten/docker-lens",
            "Strukturpiloten/kubernetes-lens",
            "Strukturpiloten/boxferry-website",
            "Other/boxferry",
        ):
            self.policy = replace(self.policy, repository=repository)
            with (
                self.subTest(repository=repository),
                self.assertRaisesRegex(ContractError, "four reviewed owners"),
            ):
                self.check()


class DevcontainerGateCli(GateFixture):
    def invoke(self, *, enabled: bool = True, extra: tuple[str, ...] = ()) -> dict[str, object]:
        report = self.root / "report.json"
        arguments = [
            "gate",
            "--repository-root",
            str(self.root),
            "--config",
            str(self.config),
            "--event",
            self.comparison.event,
            "--tested-sha",
            self.comparison.tested_sha,
            "--base-sha",
            self.comparison.base_sha,
            "--head-sha",
            self.comparison.head_sha,
            "--plan-json",
            json.dumps(self.plan),
            "--needs-json",
            json.dumps(self.needs),
            "--report-json",
            str(report),
            *extra,
        ]
        if enabled:
            arguments.append("--devcontainer-validation")
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            result = main(arguments)
        evidence = json.loads(report.read_text())
        self.assertEqual(result == 0, evidence["status"] == "success")
        return evidence

    def test_cli_is_opt_in_and_retains_ordinary_gate(self) -> None:
        self.full("push")
        self.assertEqual(self.invoke(enabled=False)["status"], "infrastructure")
        with self.approved():
            self.assertEqual(self.invoke()["status"], "success")
        del self.needs[JOB]
        self.assertEqual(self.invoke(enabled=False)["status"], "success")

    def test_cli_default_seed_registry_cannot_authorize_draft_policy(self) -> None:
        self.full("push")
        evidence = self.invoke()
        self.assertEqual(evidence["status"], "infrastructure")
        self.assertIn("unapproved pre-seed", str(evidence["cause"]))

    def test_cli_distinguishes_failed_image_from_malformed_identity(self) -> None:
        self.full("push")
        with self.approved():
            self.needs[JOB] = {"result": "cancelled"}
            self.assertEqual(self.invoke()["status"], "failed")
            self.needs[JOB] = {"result": "success", "outputs": {"candidate_sha": "d" * 40}}
            self.assertEqual(self.invoke()["status"], "infrastructure")

    def test_cli_seeded_full_inventory_needs_no_legacy_registry_entry(self) -> None:
        self.adopt()
        self.full("workflow_call")
        self.assertEqual(self.invoke()["status"], "success")
        self.needs[JOB] = {"result": "skipped"}
        self.assertEqual(self.invoke()["status"], "failed")

    def test_missing_policy_never_enters_a_new_bootstrap_fallback(self) -> None:
        self.full("push")
        self.config.unlink()
        self.assertEqual(self.invoke()["status"], "infrastructure")

    def git(self, *args: str) -> str:
        return subprocess.run(
            ["git", *args], cwd=self.root, capture_output=True, text=True, check=True
        ).stdout.strip()

    def test_pr_queue_verify_real_base_and_reject_candidate_policy_narrowing(self) -> None:
        self.adopt()
        self.git("init", "--initial-branch=main")
        self.git("config", "user.name", "Gate fixture")
        self.git("config", "user.email", "gate@example.invalid")
        (self.root / "README.md").write_text("base\n")
        self.git("add", "--all")
        self.git("commit", "--quiet", "-m", "trusted fixture policy")
        base = self.git("rev-parse", "HEAD")
        (self.root / "README.md").write_text("candidate\n")
        self.git("add", "README.md")
        self.git("commit", "--quiet", "-m", "candidate fixture")
        candidate = self.git("rev-parse", "HEAD")
        for event in ("pull_request", "merge_group"):
            self.full(event)
            self.comparison = Comparison(event, base, candidate, candidate)
            self.plan.update(base_sha=base, head_sha=candidate, tested_sha=candidate)
            self.needs[JOB] = {"result": "success", "outputs": {"candidate_sha": candidate}}
            with self.subTest(event=event):
                self.assertEqual(self.invoke(extra=("--policy-sha", base))["status"], "success")
                wrong = self.invoke(extra=("--policy-sha", candidate))
                self.assertEqual(wrong["status"], "infrastructure")
                self.assertIn("exact comparison base", str(wrong["cause"]))
        raw = json.loads(self.config.read_text())
        raw["jobs"] = ["documentation"]
        self.config.write_text(json.dumps(raw))
        rejected = self.invoke(extra=("--policy-sha", base))
        self.assertEqual(rejected["status"], "infrastructure")
        self.assertIn("differ from the trusted base", str(rejected["cause"]))


class ActualMainPolicySeeds(GateFixture):
    """Independent captured policy inventories, never future draft approval."""

    def setUp(self) -> None:
        super().setUp()
        self.fixtures = Path(__file__).parent / "fixtures/devcontainer_seed"
        self.evidence = json.loads((self.fixtures / "approval.json").read_text())
        self.adopted_evidence = json.loads(
            (self.fixtures / "post-adoption-20261107/approval.json").read_text()
        )
        self.historical_records = self.evidence["policies"]
        self.adopted_records = self.adopted_evidence["policies"]
        self.records = self.historical_records + self.adopted_records

    def actual(self, record: dict[str, Any]) -> None:
        self.config.write_bytes((self.fixtures / record["policy_fixture"]).read_bytes())
        self.policy = load_policy(self.config)

    def expected(
        self, record: dict[str, Any], profiles: tuple[str, ...], event: str = "pull_request"
    ) -> tuple[dict[str, object], dict[str, object], Comparison]:
        selected = (
            set(record["jobs"])
            if profiles == ("full",)
            else {job for profile in profiles for job in record["profile_jobs"][profile]}
        )
        tested = "c" * 40
        comparison = Comparison(
            event,
            record["head"] if event in {"pull_request", "merge_group"} else tested,
            "b" * 40 if event in {"pull_request", "merge_group"} else tested,
            tested,
        )
        plan: dict[str, object] = {
            "schema": 3,
            "repository": record["repository"],
            "event": event,
            "base_sha": comparison.base_sha,
            "head_sha": comparison.head_sha,
            "tested_sha": tested,
            "profile": profiles[0] if len(profiles) == 1 else "mixed",
            "profiles": list(profiles),
            "jobs": {job: job in selected for job in record["jobs"]},
            "reasons": ["independently captured actual-main policy inventory"],
            "local_fingerprint": None,
        }
        needs: dict[str, object] = {
            "validation-plan": {"result": "success"},
            **{
                job: {"result": "success" if job in selected else "skipped"}
                for job in record["jobs"]
            },
            JOB: {"result": "success", "outputs": {"candidate_sha": tested}},
        }
        return plan, needs, comparison

    def test_registry_matches_recorded_real_heads_blobs_and_raw_policy_semantics(self) -> None:
        heads = {
            "Strukturpiloten/boxferry": "2b131a7d87b3cd10f7db47a8d11cab824ef5f3e5",
            "Strukturpiloten/compose-lens": "7fd59faf53464779b2c79a83bea0b68233b92bf9",
            "Strukturpiloten/podman-lens": "3783b3faf85741b88f571e8bb4fa055a461a3cb8",
            "Strukturpiloten/quadlet-lens": "e9cae624fc87e0c205a367a47be83ea63f1aa6e6",
        }
        adopted_heads = {
            "Strukturpiloten/compose-lens": "8a03c38264ecc80e05f2bb26c5317e1f10f813c4",
            "Strukturpiloten/podman-lens": "ad2468a5d50d01911e855a5cc6346667abad36e0",
            "Strukturpiloten/quadlet-lens": "51322f3b08cef7fe47af987f74bdf48d83dfcc73",
        }
        self.assertEqual(self.evidence["schema"], 1)
        self.assertEqual(self.adopted_evidence["schema"], 1)
        for evidence in (self.evidence, self.adopted_evidence):
            self.assertEqual(
                evidence["review_decision"],
                "approve_exact_actual_main_policy_hashes_for_seed_registry_only",
            )
        self.assertEqual(
            self.adopted_evidence["review_issue"],
            "https://github.com/Strukturpiloten/boxferry/issues/616",
        )
        self.assertEqual(len(self.historical_records), len(heads))
        self.assertEqual(len(self.adopted_records), len(adopted_heads))
        self.assertEqual(
            {record["repository"]: record["head"] for record in self.historical_records}, heads
        )
        self.assertEqual(
            {record["repository"]: record["head"] for record in self.adopted_records}, adopted_heads
        )
        approved_heads = set(heads.items()) | set(adopted_heads.items())
        self.assertEqual(len(approved_heads), len(self.records))
        self.assertEqual({record["repository"] for record in self.records}, set(heads))
        recorded_approvals: dict[str, set[str]] = {}
        for record in self.records:
            recorded_approvals.setdefault(record["repository"], set()).add(record["policy_sha256"])
        self.assertEqual(
            dict(seed.APPROVED_SEED_POLICIES),
            {owner: frozenset(hashes) for owner, hashes in recorded_approvals.items()},
        )
        self.assertEqual(
            seed.APPROVED_SEED_POLICIES["Strukturpiloten/boxferry"],
            frozenset({self.historical_records[0]["policy_sha256"]}),
        )
        for record in self.records:
            with self.subTest(repository=record["repository"], head=record["head"]):
                self.assertIn((record["repository"], record["head"]), approved_heads)
                raw = (self.fixtures / record["policy_fixture"]).read_bytes()
                self.assertEqual(len(raw), record["policy_bytes"])
                self.assertEqual(hashlib.sha256(raw).hexdigest(), record["policy_sha256"])
                for kind in ("policy", "manifest"):
                    data = (self.fixtures / record[kind + "_fixture"]).read_bytes()
                    git_blob = b"blob " + str(len(data)).encode() + b"\0" + data
                    self.assertEqual(hashlib.sha1(git_blob).hexdigest(), record[kind + "_blob"])
                    self.assertEqual(record[kind + "_mode"], "100644")
                manifest = json.loads((self.fixtures / record["manifest_fixture"]).read_text())
                self.assertEqual(manifest["policy"], record["policy_path"])
                self.assertEqual(manifest["revision"], record["automation_revision"])
                self.assertEqual(manifest["interface"], 1)
                self.actual(record)
                self.assertEqual(json.loads(raw)["schema"], record["schema"])
                self.assertEqual(self.policy.repository, record["repository"])
                self.assertEqual(self.policy.jobs, tuple(record["jobs"]))
                self.assertEqual(
                    self.policy.profile_jobs,
                    {name: tuple(jobs) for name, jobs in record["profile_jobs"].items()},
                )
                if record in self.adopted_records:
                    evidence = record["main_ci"]
                    self.assertEqual(evidence["head_sha"], record["head"])
                    self.assertEqual(evidence["event"], "push")
                    self.assertEqual(evidence["conclusion"], "success")
                    self.assertIs(evidence["all_jobs_success"], True)
                    self.assertGreater(evidence["job_count"], 0)

    def test_actual_pr_queue_unions_and_full_events_keep_every_original_result(self) -> None:
        for record in self.records:
            self.actual(record)
            names = sorted(record["profile_jobs"])
            profiles = [("full",)] + [
                group for size in range(1, len(names) + 1) for group in combinations(names, size)
            ]
            cases = [
                (event, group) for event in ("pull_request", "merge_group") for group in profiles
            ] + [
                (event, ("full",))
                for event in ("push", "workflow_dispatch", "workflow_call", "local")
            ]
            for event, group in cases:
                with self.subTest(repository=record["repository"], event=event, profiles=group):
                    plan, needs, comparison = self.expected(record, group, event)
                    original = {name: value for name, value in needs.items() if name != JOB}
                    baseline = verify_gate(
                        plan, original, self.policy, comparison, expected_plan=copy.deepcopy(plan)
                    )
                    before = copy.deepcopy((plan, needs))
                    self.assertEqual(
                        verify_devcontainer_gate(
                            plan, needs, self.policy, comparison, expected_plan=copy.deepcopy(plan)
                        ),
                        [*baseline, (JOB, "success")],
                    )
                    self.assertEqual((plan, needs), before)
                    with self.assertRaisesRegex(ContractError, "dependencies"):
                        verify_gate(plan, needs, self.policy, comparison, expected_plan=plan)

    def test_each_actual_original_dependency_stays_mandatory_in_full_mode(self) -> None:
        for record in self.records:
            self.actual(record)
            plan, needs, comparison = self.expected(record, ("full",), "push")
            for job in ("validation-plan", *record["jobs"]):
                for result in ("missing", "failure", "cancelled", "skipped"):
                    with self.subTest(repository=record["repository"], job=job, result=result):
                        changed = dict(needs)
                        if result == "missing":
                            del changed[job]
                        else:
                            changed[job] = {"result": result}
                        with self.assertRaises(ContractError):
                            verify_devcontainer_gate(
                                plan, changed, self.policy, comparison, expected_plan=plan
                            )

    def test_future_task_changes_and_recorded_future_lens_hashes_are_unapproved(self) -> None:
        for record in self.records:
            with self.subTest(repository=record["repository"]):
                future_digest = record["rejected_future_draft_sha256"]
                if future_digest is not None:
                    self.assertNotIn(
                        future_digest, seed.APPROVED_SEED_POLICIES[record["repository"]]
                    )
                self.actual(record)
                plan, needs, comparison = self.expected(record, ("full",))
                future = json.loads(self.config.read_text())
                future.setdefault("tasks", {})["unreviewed-future"] = {"commands": [["true"]]}
                self.config.write_text(json.dumps(future))
                policy = load_policy(self.config)
                self.assertEqual(policy.jobs, self.policy.jobs)
                self.assertEqual(policy.profile_jobs, self.policy.profile_jobs)
                with self.assertRaisesRegex(ContractError, "unapproved pre-seed"):
                    verify_devcontainer_gate(plan, needs, policy, comparison, expected_plan=plan)

    def test_actual_approved_hash_cannot_be_borrowed_by_another_owner(self) -> None:
        for record in self.records:
            self.actual(record)
            plan, needs, comparison = self.expected(record, ("full",))
            for owner in set(OWNERS) - {record["repository"]}:
                with self.subTest(source=record["repository"], borrowed_by=owner):
                    forged = dict(plan, repository=owner)
                    with self.assertRaisesRegex(ContractError, "unapproved pre-seed"):
                        verify_devcontainer_gate(
                            forged,
                            needs,
                            replace(self.policy, repository=owner),
                            comparison,
                            expected_plan=forged,
                        )

    def test_actual_base_or_stale_candidate_receipt_cannot_satisfy_new_job(self) -> None:
        for record in self.records:
            self.actual(record)
            plan, needs, comparison = self.expected(record, ("full",))
            for receipt in (record["head"], comparison.head_sha, "d" * 40):
                with self.subTest(repository=record["repository"], receipt=receipt):
                    changed = dict(
                        needs, **{JOB: {"result": "success", "outputs": {"candidate_sha": receipt}}}
                    )
                    with self.assertRaisesRegex(ContractError, "exact tested candidate"):
                        verify_devcontainer_gate(
                            plan, changed, self.policy, comparison, expected_plan=plan
                        )

    def test_actual_approvals_reject_unknown_malformed_and_partial_policy_bytes(self) -> None:
        for record in self.records:
            self.actual(record)
            original = self.config.read_bytes()
            trusted = self.policy
            seed.require_devcontainer_policy(trusted, seed=True)
            for kind, raw in (
                ("empty", b""),
                ("malformed", b'{"schema":'),
                ("partial", json.dumps({"schema": record["schema"]}).encode()),
                ("unknown", json.dumps({**json.loads(original), "unapproved": True}).encode()),
            ):
                with self.subTest(repository=record["repository"], head=record["head"], kind=kind):
                    self.assertNotEqual(raw, original)
                    self.assertNotIn(
                        hashlib.sha256(raw).hexdigest(),
                        seed.APPROVED_SEED_POLICIES[record["repository"]],
                    )
                    self.config.write_bytes(raw)
                    with self.assertRaisesRegex(ContractError, "unapproved pre-seed"):
                        seed.require_devcontainer_policy(trusted, seed=True)

    def test_actual_approved_bytes_cannot_authorize_partial_policy_object(self) -> None:
        for record in self.records:
            with self.subTest(repository=record["repository"], head=record["head"]):
                self.actual(record)
                partial = replace(self.policy, jobs=self.policy.jobs[:-1])
                self.assertNotEqual(partial, self.policy)
                with self.assertRaisesRegex(
                    ContractError, "differs from its approved policy bytes"
                ):
                    seed.require_devcontainer_policy(partial, seed=True)


if __name__ == "__main__":
    unittest.main()
