from __future__ import annotations

import copy
import json
import tempfile
import unittest
from pathlib import Path

from boxferry_ci.contracts import ContractError
from boxferry_ci.gate import verify_gate
from boxferry_ci.planner import Comparison
from boxferry_ci.policy import load_policy


class Gate(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        config = Path(temporary.name) / "policy.json"
        config.write_text(
            json.dumps(
                {
                    "schema": 2,
                    "repository": "Strukturpiloten/example",
                    "jobs": ["rust", "docs", "python"],
                    "profile_jobs": {
                        "prose": ["docs"],
                        "executable-docs": ["docs"],
                        "python": ["python"],
                    },
                    "public_documentation_root": "docs/public/",
                    "full_documentation_roots": [],
                    "executable_documentation_roots": [],
                    "executable_documentation_paths": [],
                    "documentation_examples_manifest": None,
                    "local_commands": {
                        name: [["true"]] for name in ("prose", "executable-docs", "python", "full")
                    },
                }
            )
        )
        self.policy = load_policy(config)
        self.comparison = Comparison("merge_group", "a" * 40, "b" * 40, "c" * 40)
        self.plan: dict[str, object] = {
            "schema": 3,
            "repository": "Strukturpiloten/example",
            "event": "merge_group",
            "base_sha": "a" * 40,
            "head_sha": "b" * 40,
            "tested_sha": "c" * 40,
            "profile": "mixed",
            "profiles": ["prose", "python"],
            "jobs": {"rust": False, "docs": True, "python": True},
            "reasons": ["known mixed changes"],
            "local_fingerprint": None,
        }
        self.needs: dict[str, object] = {
            "validation-plan": {"result": "success"},
            "rust": {"result": "skipped"},
            "docs": {"result": "success"},
            "python": {"result": "success"},
        }

    def test_only_planned_skips_are_accepted(self) -> None:
        self.assertEqual(len(verify_gate(self.plan, self.needs, self.policy, self.comparison)), 4)
        self.needs["docs"] = {"result": "skipped"}
        with self.assertRaisesRegex(ContractError, "selected docs"):
            verify_gate(self.plan, self.needs, self.policy, self.comparison)

    def test_planning_always_required(self) -> None:
        self.needs["validation-plan"] = {"result": "skipped"}
        with self.assertRaisesRegex(ContractError, "selected validation-plan"):
            verify_gate(self.plan, self.needs, self.policy, self.comparison)

    def test_every_malformed_outcome_fails_closed(self) -> None:
        for malformed in (
            None,
            [],
            "success",
            {},
            {"result": None},
            {"result": True},
            {"result": "neutral"},
        ):
            with self.subTest(outcome=malformed):
                needs = dict(self.needs, docs=malformed)
                with self.assertRaises(ContractError):
                    verify_gate(self.plan, needs, self.policy, self.comparison)

    def test_missing_unknown_or_unexpected_failure_is_rejected(self) -> None:
        for mode in ("missing", "extra", "failure", "cancelled"):
            with self.subTest(mode=mode):
                needs = dict(self.needs)
                if mode == "missing":
                    del needs["docs"]
                elif mode == "extra":
                    needs["unknown"] = {"result": "success"}
                else:
                    needs["rust"] = {"result": mode}
                with self.assertRaises(ContractError):
                    verify_gate(self.plan, needs, self.policy, self.comparison)

    def test_every_exact_revision_is_bound(self) -> None:
        for key in ("base_sha", "head_sha", "tested_sha"):
            with self.subTest(key=key):
                plan = dict(self.plan, **{key: "d" * 40})
                with self.assertRaisesRegex(ContractError, "exact"):
                    verify_gate(plan, self.needs, self.policy, self.comparison)

    def test_profile_and_job_union_cannot_be_weakened(self) -> None:
        cases = [
            {"jobs": {"rust": False, "docs": True, "python": False}},
            {"jobs": {"rust": 0, "docs": True, "python": True}},
            {"profiles": ["unknown"]},
            {"profiles": ["full", "prose"]},
            {"profiles": ["python", "prose"]},
            {"profile": "prose"},
            {"schema": True},
            {"schema": 2},
            {"reasons": []},
            {"local_fingerprint": "a" * 64},
        ]
        for patch in cases:
            with self.subTest(patch=patch), self.assertRaises(ContractError):
                verify_gate(dict(self.plan, **patch), self.needs, self.policy, self.comparison)

    def test_independent_comparison_prevents_forged_prose_subset(self) -> None:
        expected = copy.deepcopy(self.plan)
        expected["profiles"] = ["full"]
        with self.assertRaisesRegex(ContractError, "independent"):
            verify_gate(self.plan, self.needs, self.policy, self.comparison, expected_plan=expected)

    def test_pushes_cannot_use_selective_plan(self) -> None:
        plan = dict(self.plan, event="push")
        comparison = Comparison("push", "a" * 40, "b" * 40, "c" * 40)
        with self.assertRaisesRegex(ContractError, "complete"):
            verify_gate(plan, self.needs, self.policy, comparison)
