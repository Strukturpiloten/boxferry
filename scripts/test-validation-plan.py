#!/usr/bin/env python3
"""Regression contracts for the shared change-aware validation planner."""

from __future__ import annotations

import argparse
import copy
import importlib.util
import json
import os
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SOURCE_ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location(
    "validation_plan", SOURCE_ROOT / "scripts/validation-plan.py"
)
assert SPEC is not None and SPEC.loader is not None
planner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(planner)
POLICY = planner.policy_at(SOURCE_ROOT)


def git(root: Path, *args: str) -> str:
    result = subprocess.run(["git", *args], cwd=root, check=True, text=True, capture_output=True)
    return result.stdout.strip()


def write(root: Path, name: str, content: str) -> None:
    path = root / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def arguments(root: Path, event: str, base: str = "", head: str = "") -> argparse.Namespace:
    return argparse.Namespace(
        repository_root=root,
        policy_root=SOURCE_ROOT,
        event=event,
        base_sha=base,
        head_sha=head,
        tested_sha=head or git(root, "rev-parse", "HEAD"),
        base_ref="main",
        force_full=False,
    )


def adopted_fixture(event: str = "pull_request") -> tuple:
    """Independently expected complete inventory from approved historical main bytes."""
    sys.path.insert(0, str(SOURCE_ROOT / "automation/src"))
    from boxferry_ci import gate
    from boxferry_ci.planner import Comparison
    from boxferry_ci.policy import load_policy

    policy = load_policy(SOURCE_ROOT / "scripts/shared-validation-policy.json")
    comparison = Comparison(event, "a" * 40, "b" * 40, "c" * 40)
    plan = {
        "schema": 3,
        "repository": policy.repository,
        "event": event,
        "base_sha": comparison.base_sha,
        "head_sha": comparison.head_sha,
        "tested_sha": comparison.tested_sha,
        "profile": "full",
        "profiles": ["full"],
        "jobs": dict.fromkeys(policy.jobs, True),
        "reasons": ["independent complete ordinary inventory"],
        "local_fingerprint": None,
    }
    needs = {name: {"result": "success"} for name in ("validation-plan", *policy.jobs)}
    return gate, policy, comparison, plan, needs


class ValidationPlanTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory(prefix="boxferry-validation-plan-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        git(self.root, "init", "-q", "-b", "main")
        git(self.root, "config", "user.name", "Validation Test")
        git(self.root, "config", "user.email", "validation@example.invalid")
        write(self.root, "docs/public/concepts/index.md", "# Concepts\n\nPlain text.\n")
        write(
            self.root,
            "docs/public/getting-started/index.md",
            "# Start\n\n```console\n$ boxferry --version\n```\n",
        )
        write(self.root, "src/main.rs", "fn main() {}\n")
        git(self.root, "add", ".")
        git(self.root, "commit", "-qm", "baseline")
        self.base = git(self.root, "rev-parse", "HEAD")

    def commit(self, message: str = "candidate") -> str:
        git(self.root, "add", "-A")
        git(self.root, "commit", "-qm", message)
        return git(self.root, "rev-parse", "HEAD")

    def plan(self, base: str, head: str) -> dict:
        return planner.make_plan(arguments(self.root, "pull_request", base, head), POLICY)

    def test_public_prose_selects_only_file_and_lockfile_checks(self) -> None:
        write(self.root, "docs/public/concepts/index.md", "# Concepts\n\nUpdated prose.\n")
        head = self.commit()
        plan = self.plan(self.base, head)
        self.assertEqual(plan["profile"], "prose")
        self.assertEqual(
            {name for name, selected in plan["jobs"].items() if selected},
            set(POLICY["profile_jobs"]["prose"]),
        )

    def test_executable_docs_and_new_code_fences_select_focused_suite(self) -> None:
        write(self.root, "docs/public/getting-started/index.md", "# Start\n\nUpdated example.\n")
        head = self.commit()
        self.assertEqual(self.plan(self.base, head)["profile"], "executable-docs")
        write(
            self.root,
            "docs/public/concepts/index.md",
            "# Concepts\n\n```console\n$ boxferry --version\n```\n",
        )
        next_head = self.commit("new example")
        self.assertEqual(self.plan(head, next_head)["profile"], "executable-docs")

    def test_new_inline_cli_examples_are_not_treated_as_prose(self) -> None:
        write(
            self.root,
            "docs/public/concepts/index.md",
            "# Concepts\n\nUse `boxferry convert --help`.\n",
        )
        head = self.commit("inline command")
        self.assertEqual(self.plan(self.base, head)["profile"], "executable-docs")

    def test_indented_cli_examples_are_not_treated_as_prose(self) -> None:
        write(
            self.root,
            "docs/public/concepts/index.md",
            "# Concepts\n\n    boxferry convert --help\n",
        )
        head = self.commit("indented command")
        self.assertEqual(self.plan(self.base, head)["profile"], "executable-docs")

    def test_tab_indented_examples_in_edited_and_new_pages(self) -> None:
        path = "docs/public/concepts/index.md"
        for spaces in range(4):
            with self.subTest(edited_page_spaces=spaces):
                write(self.root, path, "# Concepts\n\nOrdinary prose.\n")
                prose_head = self.commit("restore prose")
                write(self.root, path, f"# Concepts\n\n{' ' * spaces}\tboxferry convert --help\n")
                example_head = self.commit("edit tab-indented example")
                self.assertEqual(self.plan(prose_head, example_head)["profile"], "executable-docs")

        before_new_page = git(self.root, "rev-parse", "HEAD")
        write(
            self.root, "docs/public/concepts/new.md", "# New page\n\n  \tboxferry convert --help\n"
        )
        new_page_head = self.commit("add tab-indented example")
        self.assertEqual(self.plan(before_new_page, new_page_head)["profile"], "executable-docs")

    def test_one_to_three_spaces_without_tab_remain_prose(self) -> None:
        for spaces in range(1, 4):
            with self.subTest(spaces=spaces):
                path = f"docs/public/concepts/prose-{spaces}.md"
                before_new_page = git(self.root, "rev-parse", "HEAD")
                write(self.root, path, f"# Prose\n\n{' ' * spaces}Ordinary prose.\n")
                head = self.commit("add indented prose")
                self.assertEqual(self.plan(before_new_page, head)["profile"], "prose")

    def test_policy_rejects_executable_documentation_downgrades(self) -> None:
        policy = copy.deepcopy(POLICY)
        policy["profile_jobs"]["executable-docs"] = ["documentation"]
        write(self.root, "scripts/validation-policy.json", json.dumps(policy))
        with self.assertRaisesRegex(planner.PlanError, "fewer jobs"):
            planner.policy_at(self.root)
        policy = copy.deepcopy(POLICY)
        policy["local_commands"]["executable-docs"] = [["cargo", "test", "--test", "public_guides"]]
        write(self.root, "scripts/validation-policy.json", json.dumps(policy))
        with self.assertRaisesRegex(planner.PlanError, "include prose commands"):
            planner.policy_at(self.root)
        policy["local_commands"]["executable-docs"] = list(policy["local_commands"]["full"])
        write(self.root, "scripts/validation-policy.json", json.dumps(policy))
        planner.policy_at(self.root)

    def test_optional_manifest_and_profile_specific_jobs_are_fail_closed(self) -> None:
        policy = {
            **POLICY,
            "documentation_examples_manifest": None,
            "executable_documentation_roots": ["docs/public/guides/"],
            "profile_jobs": {
                "prose": list(POLICY["profile_jobs"]["prose"]),
                "executable-docs": list(POLICY["jobs"]),
            },
        }
        write(self.root, "docs/public/guides/new.md", "# New guide\n\nPlain text.\n")
        head = self.commit("new guide")
        plan = planner.make_plan(arguments(self.root, "pull_request", self.base, head), policy)
        self.assertEqual(plan["profile"], "executable-docs")
        self.assertTrue(all(plan["jobs"].values()))
        needs = {job: {"result": "success"} for job in policy["jobs"]}
        needs["validation-plan"] = {"result": "success"}
        planner.verify_gate(
            plan, needs, policy, arguments(self.root, "pull_request", self.base, head)
        )
        needs["rust"]["result"] = "skipped"
        with self.assertRaises(planner.PlanError):
            planner.verify_gate(
                plan, needs, policy, arguments(self.root, "pull_request", self.base, head)
            )

    def test_code_policy_unknown_and_mixed_paths_are_full(self) -> None:
        for path in (
            "src/main.rs",
            "scripts/validation-plan.py",
            "scripts/validation-policy.json",
            "docs/documentation-examples.toml",
            "fixtures/sample.yaml",
            "README.md",
            "docs/public/development/testing/index.md",
            "docs/public/unknown.json",
        ):
            with self.subTest(path=path):
                write(self.root, path, "changed\n")
                head = self.commit(path)
                self.assertEqual(self.plan(self.base, head)["profile"], "full")
                git(self.root, "revert", "--no-edit", head)
                self.base = git(self.root, "rev-parse", "HEAD")
        write(self.root, "docs/public/concepts/index.md", "# Concepts\n\nNew prose.\n")
        write(self.root, "src/main.rs", 'fn main() { println!("x"); }\n')
        head = self.commit("mixed")
        self.assertEqual(self.plan(self.base, head)["profile"], "full")

    def test_add_delete_rename_and_special_path_bytes(self) -> None:
        write(self.root, "docs/public/new page ü.md", "# New\n")
        added = self.commit("add")
        self.assertEqual(self.plan(self.base, added)["profile"], "prose")
        git(self.root, "mv", "docs/public/new page ü.md", "docs/public/renamed page ü.md")
        renamed = self.commit("rename")
        self.assertEqual(self.plan(added, renamed)["profile"], "prose")
        git(self.root, "rm", "docs/public/renamed page ü.md")
        deleted = self.commit("delete")
        self.assertEqual(self.plan(renamed, deleted)["profile"], "prose")
        write(self.root, "docs/public/line\nbreak.md", "# A\n")
        newline = self.commit("newline")
        self.assertEqual(self.plan(deleted, newline)["profile"], "prose")
        git(self.root, "mv", "docs/public/line\nbreak.md", "src/line-break.rs")
        moved_to_code = self.commit("rename across boundary")
        self.assertEqual(self.plan(newline, moved_to_code)["profile"], "full")

    def test_missing_comparison_and_empty_comparison_fall_back_to_full(self) -> None:
        self.assertEqual(self.plan(self.base, self.base)["profile"], "full")
        self.assertEqual(self.plan("0" * 40, self.base)["profile"], "full")
        with self.assertRaises(planner.PlanError):
            planner.parse_diff(b"R100\0old\0")
        with self.assertRaises(planner.PlanError):
            planner.parse_diff(b"X\0path\0")

    def test_local_staged_unstaged_and_untracked_changes(self) -> None:
        write(self.root, "docs/public/concepts/index.md", "# Concepts\n\nStaged.\n")
        git(self.root, "add", "docs/public/concepts/index.md")
        staged = planner.make_plan(arguments(self.root, "local"), POLICY)
        self.assertEqual(staged["profile"], "prose")
        write(self.root, "docs/public/concepts/index.md", "# Concepts\n\nUnstaged.\n")
        unstaged = planner.make_plan(arguments(self.root, "local"), POLICY)
        self.assertEqual(unstaged["profile"], "prose")
        self.assertNotEqual(staged["local_fingerprint"], unstaged["local_fingerprint"])
        write(self.root, "src/untracked.rs", "fn extra() {}\n")
        self.assertEqual(
            planner.make_plan(arguments(self.root, "local"), POLICY)["profile"], "full"
        )

    def test_local_committed_branch_changes_are_included(self) -> None:
        git(self.root, "switch", "-qc", "issue")
        write(self.root, "docs/public/concepts/index.md", "# Concepts\n\nCommitted prose.\n")
        self.commit()
        self.assertEqual(
            planner.make_plan(arguments(self.root, "local"), POLICY)["profile"], "prose"
        )

    def test_local_index_cannot_be_hidden_by_restoring_only_worktree_bytes(self) -> None:
        write(self.root, "src/main.rs", 'fn main() { println!("staged"); }\n')
        git(self.root, "add", "src/main.rs")
        write(self.root, "src/main.rs", "fn main() {}\n")
        write(
            self.root,
            "docs/public/concepts/index.md",
            "# Concepts\n\nOnly prose in the worktree.\n",
        )
        plan = planner.make_plan(arguments(self.root, "local"), POLICY)
        self.assertEqual(plan["profile"], "full")

    def test_local_index_content_can_promote_restored_docs_to_executable(self) -> None:
        write(
            self.root,
            "docs/public/concepts/index.md",
            "# Concepts\n\n```console\n$ boxferry --version\n```\n",
        )
        git(self.root, "add", "docs/public/concepts/index.md")
        write(self.root, "docs/public/concepts/index.md", "# Concepts\n\nPlain text.\n")
        plan = planner.make_plan(arguments(self.root, "local"), POLICY)
        self.assertEqual(plan["profile"], "executable-docs")

    def test_local_runner_rejects_target_from_a_relocated_worktree(self) -> None:
        former_target = self.root.parent / "former-worktree" / "target"
        with patch.dict(os.environ, {"CARGO_TARGET_DIR": str(former_target)}):
            with self.assertRaisesRegex(planner.PlanError, "inside this worktree"):
                planner.run_local(arguments(self.root, "local"), POLICY)
        with patch.dict(os.environ, {"CARGO_TARGET_DIR": str(self.root / "target")}):
            planner.require_worktree_local_target(self.root)

    def test_main_dispatch_and_release_caller_are_always_full(self) -> None:
        for event in ("push", "workflow_dispatch", "workflow_call"):
            with self.subTest(event=event):
                plan = planner.make_plan(arguments(self.root, event), POLICY)
                self.assertEqual(plan["profile"], "full")
                self.assertTrue(all(plan["jobs"].values()))

    def test_gate_rejects_missing_skipped_failed_and_cancelled_required_jobs(self) -> None:
        write(self.root, "docs/public/concepts/index.md", "# Concepts\n\nNew prose.\n")
        head = self.commit()
        plan = self.plan(self.base, head)
        needs = {
            job: {"result": "success" if plan["jobs"][job] else "skipped"} for job in POLICY["jobs"]
        }
        needs["validation-plan"] = {"result": "success"}
        args = arguments(self.root, "pull_request", self.base, head)
        self.assertEqual(len(planner.verify_gate(plan, needs, POLICY, args)), len(POLICY["jobs"]))
        for result in ("skipped", "failure", "cancelled", "timed_out", None):
            with self.subTest(result=result):
                broken = json.loads(json.dumps(needs))
                broken["documentation"]["result"] = result
                with self.assertRaises(planner.PlanError):
                    planner.verify_gate(plan, broken, POLICY, args)
        broken = json.loads(json.dumps(needs))
        broken["rust"]["result"] = "failure"
        with self.assertRaises(planner.PlanError):
            planner.verify_gate(plan, broken, POLICY, args)
        with self.assertRaises(planner.PlanError):
            planner.verify_gate(plan, {}, POLICY, args)
        broken_plan = json.loads(json.dumps(plan))
        broken_plan["jobs"]["documentation"] = "true"
        with self.assertRaises(planner.PlanError):
            planner.verify_gate(broken_plan, needs, POLICY, args)

    def test_gate_requires_all_jobs_for_main_and_release_events(self) -> None:
        for event in ("push", "workflow_dispatch", "workflow_call"):
            with self.subTest(event=event):
                plan = planner.make_plan(arguments(self.root, event), POLICY)
                needs = {job: {"result": "success"} for job in POLICY["jobs"]}
                needs["validation-plan"] = {"result": "success"}
                args = arguments(self.root, event)
                planner.verify_gate(plan, needs, POLICY, args)
                needs["coverage"]["result"] = "skipped"
                with self.assertRaises(planner.PlanError):
                    planner.verify_gate(plan, needs, POLICY, args)
                needs["coverage"]["result"] = "success"
                plan["profile"] = "prose"
                with self.assertRaises(planner.PlanError):
                    planner.verify_gate(plan, needs, POLICY, args)

    def test_merge_queue_is_complete_and_bound_to_exact_base_and_tested_head(self) -> None:
        write(self.root, "docs/public/concepts/index.md", "# Concepts\n\nQueue prose.\n")
        head = self.commit()
        args = arguments(self.root, "merge_group", self.base, head)
        plan = planner.make_plan(args, POLICY)
        self.assertEqual(plan["profile"], "full")
        self.assertTrue(all(plan["jobs"].values()))
        needs = {job: {"result": "success"} for job in ["validation-plan", *POLICY["jobs"]]}
        planner.verify_gate(plan, needs, POLICY, args)
        for key, value in (("base_sha", head), ("head_sha", self.base), ("profile", "prose")):
            with self.subTest(key=key), self.assertRaises(planner.PlanError):
                planner.verify_gate({**plan, key: value}, needs, POLICY, args)
        for result in ("skipped", "failure", "cancelled", None):
            needs["rust"]["result"] = result
            with self.subTest(result=result), self.assertRaises(planner.PlanError):
                planner.verify_gate(plan, needs, POLICY, args)
        args.tested_sha = self.base
        with self.assertRaises(planner.PlanError):
            planner.make_plan(args, POLICY)
        for base in ("", "main", self.base[:12], "0" * 40):
            with self.subTest(base=base), self.assertRaises(planner.PlanError):
                planner.make_plan(arguments(self.root, "merge_group", base, head), POLICY)
        with self.assertRaises(planner.PlanError):
            planner.make_plan(arguments(self.root, "merge_group", head, self.base), POLICY)

    def test_required_ci_is_the_real_always_running_aggregate(self) -> None:
        workflow = (SOURCE_ROOT / ".github/workflows/ci.yml").read_text()
        matches = re.findall(r"(?ms)^  pr-gate:\n(.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)", workflow)
        self.assertEqual(len(matches), 1)
        aggregate = matches[0]
        self.assertEqual(workflow.count("    name: Required CI\n"), 1)
        self.assertIn("    if: always()\n", aggregate)
        dependencies = re.search(r"(?s)    needs:\n\s*\[(.*?)\]", aggregate)
        self.assertIsNotNone(dependencies)
        assert dependencies is not None
        jobs = [name.strip() for name in dependencies[1].split(",") if name.strip()]
        expected = json.loads((SOURCE_ROOT / "scripts/shared-validation-policy.json").read_text())[
            "jobs"
        ]
        self.assertEqual(len(jobs), len(set(jobs)))
        self.assertEqual(set(jobs), {"validation-plan", *expected})
        self.assertIn("python -m boxferry_ci gate", aggregate)
        self.assertIn('--plan-json "${PLAN_JSON}" --needs-json "${NEEDS_JSON}"', aggregate)

    def test_mandatory_gate_projection_is_monotonic_and_fail_closed(self) -> None:
        gate, policy, comparison, plan, needs = adopted_fixture()
        self.assertEqual(len(policy.jobs), 16)
        gate.verify_gate(plan, needs, policy, comparison, expected_plan=plan)
        mandatory = (
            "release-binaries",
            "automation",
            "canonical-artifact-integrity",
            "kubernetes-artifact-integrity",
            "compose-provider-integrity",
        )
        for name in mandatory:
            for outcome in ("skipped", "failure", "cancelled", "timed_out", None, ""):
                broken = copy.deepcopy(needs)
                broken[name]["result"] = outcome
                with self.subTest(name=name, outcome=outcome), self.assertRaises(ValueError):
                    gate.verify_gate(plan, broken, policy, comparison, expected_plan=plan)
            for invalid in (None, [], "success", {}):
                broken = copy.deepcopy(needs)
                broken[name] = invalid
                with self.subTest(name=name, invalid=invalid), self.assertRaises(ValueError):
                    gate.verify_gate(plan, broken, policy, comparison, expected_plan=plan)
            missing = {key: value for key, value in needs.items() if key != name}
            with self.assertRaises(ValueError):
                gate.verify_gate(plan, missing, policy, comparison, expected_plan=plan)
        for mutate in ("failed-rust", "missing-rust", "extra"):
            broken = copy.deepcopy(needs)
            if mutate == "failed-rust":
                broken["rust"]["result"] = "failure"
            elif mutate == "missing-rust":
                del broken["rust"]
            else:
                broken["extra"] = {"result": "success"}
            with self.subTest(mutate=mutate), self.assertRaises(ValueError):
                gate.verify_gate(plan, broken, policy, comparison, expected_plan=plan)

    def test_producer_artifact_jobs_are_separate_mandatory_read_only_candidate_checks(self) -> None:
        workflow = (SOURCE_ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        contracts = {
            "canonical-artifact-integrity": (
                15,
                "boxferry_ci.asset_verification",
                "CANONICAL_CANDIDATE_SHA: ${{ github.sha }}",
                '--candidate-sha "${CANONICAL_CANDIDATE_SHA}"',
            ),
            "kubernetes-artifact-integrity": (
                17,
                "uv run --locked --project _automation/automation python "
                "_automation/scripts/kubernetes_tool_checksum_refresh.py",
                '--verify-git-ref "${GITHUB_SHA}"',
                "--repository-root _automation",
            ),
            "compose-provider-integrity": (
                17,
                "boxferry_ci.compose_provider",
                "CANDIDATE_SHA: ${{ github.sha }}",
                '--candidate-sha "${CANDIDATE_SHA}"',
            ),
        }
        renovate = json.loads((SOURCE_ROOT / ".github/renovate.json").read_text(encoding="utf-8"))
        automation = re.search(r"(?ms)^  automation:\n(.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)", workflow)
        self.assertIsNotNone(automation)
        assert automation is not None
        self.assertIn("    timeout-minutes: 10\n", automation[1])
        for job, (timeout, command, binding, mode) in contracts.items():
            with self.subTest(job=job):
                matches = re.findall(
                    r"(?ms)^  " + re.escape(job) + r":\n(.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)", workflow
                )
                self.assertEqual(len(matches), 1)
                body = matches[0]
                runner = re.search(
                    r"(?m)^    runs-on: (ubuntu-([0-9]{2})\.([0-9]{2})(-arm)?)$", body
                )
                self.assertIsNotNone(
                    runner, "producer checks require a fixed GitHub-hosted Ubuntu label"
                )
                assert runner is not None
                # Exercise a normal future Renovate upgrade without freezing the current version.
                advanced = body.replace(
                    runner[1], f"ubuntu-{int(runner[2]) + 2:02d}.{runner[3]}{runner[4] or ''}", 1
                )
                self.assertNotEqual(advanced, body)
                for candidate_body in (body, advanced):
                    owners = []
                    for manager in renovate["customManagers"]:
                        if manager.get("datasourceTemplate") != "github-runners":
                            continue
                        paths = manager.get("managerFilePatterns", [])
                        if not any(
                            pattern.startswith("/")
                            and pattern.endswith("/")
                            and re.search(pattern[1:-1], ".github/workflows/ci.yml")
                            for pattern in paths
                        ):
                            continue
                        for pattern in manager.get("matchStrings", []):
                            matches = re.findall(pattern.replace("(?<", "(?P<"), candidate_body)
                            owners.extend((manager, match) for match in matches)
                    self.assertEqual(
                        len(owners), 1, "each runner must have one existing Renovate owner"
                    )
                    self.assertEqual(owners[0][1][0], "ubuntu")
                for expected in (
                    f"    timeout-minutes: {timeout}\n",
                    "ref: ${{ github.sha }}",
                    "path: _automation",
                    "persist-credentials: false",
                    "uses: ./_automation/.github/actions/shared-python",
                    command,
                    binding,
                    mode,
                    "--repository-root _automation",
                ):
                    self.assertIn(expected, body)
                self.assertIn("    needs: validation-plan", body)
                self.assertIn(
                    f"if: needs.validation-plan.outputs.select_{job.replace('-', '_')} == 'true'",
                    body,
                )
                for forbidden in (
                    "continue-on-error",
                    "secrets.",
                    "secrets:",
                    "token:",
                    "permissions:",
                    "contents: write",
                    "--write",
                    "--working-tree",
                    "--refresh",
                    "install-kubernetes-tools.sh",
                    "compose-provider.sh",
                ):
                    self.assertNotIn(forbidden, body)
                self.assertNotIn(command, automation[1])
                self.assertNotIn(job, POLICY["jobs"])

    def test_first_rollout_never_executes_candidate_classifier_or_accepts_a_skip(self) -> None:
        workflow = (SOURCE_ROOT / ".github/workflows/ci.yml").read_text()
        self.assertIn(
            "run: uv run --locked --project automation bash scripts/test-application-probes.sh",
            workflow,
        )
        self.assertIn(
            "contains(fromJSON(needs.validation-plan.outputs.plan).profiles, 'executable-docs')",
            workflow,
        )
        self.assertNotIn("scripts/validation-plan.py plan --force-full", workflow)
        self.assertNotIn("if [[ -f .validation-base", workflow)
        self.assertIn("--config .validation-base/scripts/shared-validation-policy.json", workflow)
        gate, policy, comparison, plan, needs = adopted_fixture()
        gate.verify_gate(plan, needs, policy, comparison, expected_plan=plan)
        needs["rust"]["result"] = "skipped"
        with self.assertRaises(ValueError):
            gate.verify_gate(plan, needs, policy, comparison, expected_plan=plan)

    def test_queue_workflow_bootstrap_is_full_and_never_accepts_stale_or_skipped_results(
        self,
    ) -> None:
        workflow = (SOURCE_ROOT / ".github/workflows/ci.yml").read_text()
        self.assertIn("  merge_group:\n    types: [checks_requested]", workflow)
        self.assertIn("github.event.merge_group.base_sha", workflow)
        self.assertIn("--tested-sha", workflow)
        gate, policy, comparison, plan, needs = adopted_fixture("merge_group")
        gate.verify_gate(plan, needs, policy, comparison, expected_plan=plan)
        for outcome in ("skipped", "failure", "cancelled"):
            broken = copy.deepcopy(needs)
            broken["rust"]["result"] = outcome
            with self.subTest(outcome=outcome), self.assertRaises(ValueError):
                gate.verify_gate(plan, broken, policy, comparison, expected_plan=plan)
        for key in ("base_sha", "head_sha", "tested_sha"):
            wrong = copy.deepcopy(plan)
            wrong[key] = "a" * 12
            with self.subTest(key=key), self.assertRaises(ValueError):
                gate.verify_gate(wrong, needs, policy, comparison, expected_plan=plan)

    def test_automation_package_is_mandatory_and_age_checks_include_queue(self) -> None:
        workflow = (SOURCE_ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        automation = workflow.split("  automation:\n", 1)[1].split(
            "  canonical-artifact-integrity:\n", 1
        )[0]
        self.assertIn("if: needs.validation-plan.outputs.select_automation == 'true'", automation)
        self.assertIn("uses: ./_automation/.github/actions/shared-python", automation)
        self.assertIn("    timeout-minutes: 10\n", automation)
        for command in ("ruff format --check", "ruff check", "ty check"):
            self.assertIn(command, automation)
        reporter_command = (
            "uv run --locked --project _automation/automation python -m boxferry_ci.test_reporting "
            "--repository-root _automation --start-directory _automation/automation/tests"
        )
        self.assertEqual(automation.count(f"        run: {reporter_command}\n"), 1)
        self.assertEqual(workflow.count("python -m boxferry_ci.test_reporting"), 1)
        self.assertNotIn("python -m unittest", automation)
        policy = json.loads((SOURCE_ROOT / "scripts/shared-validation-policy.json").read_text())
        for selected in policy["profile_jobs"].values():
            self.assertIn("automation", selected)
        age = workflow.split("  lockfile-release-age:\n", 1)[1].split("  pr-gate:\n", 1)[0]
        self.assertIn("shared-dependency-wait.yml@", age)
        self.assertIn("github.event.merge_group.base_sha", age)
        self.assertIn("candidate_sha: ${{ github.sha }}", age)
        self.assertIn(
            "if: needs.validation-plan.outputs.select_lockfile_release_age == 'true'", age
        )


if __name__ == "__main__":
    unittest.main()
