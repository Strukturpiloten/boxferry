#!/usr/bin/env python3
"""Regression contracts for the shared change-aware validation planner."""

from __future__ import annotations

import argparse
import copy
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest
from unittest.mock import patch


SOURCE_ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location("validation_plan", SOURCE_ROOT / "scripts/validation-plan.py")
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
        repository_root=root, policy_root=SOURCE_ROOT, event=event,
        base_sha=base, head_sha=head, tested_sha=head or git(root, "rev-parse", "HEAD"),
        base_ref="main", force_full=False,
    )


class ValidationPlanTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory(prefix="boxferry-validation-plan-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        git(self.root, "init", "-q", "-b", "main")
        git(self.root, "config", "user.name", "Validation Test")
        git(self.root, "config", "user.email", "validation@example.invalid")
        write(self.root, "docs/public/concepts/index.md", "# Concepts\n\nPlain text.\n")
        write(self.root, "docs/public/getting-started/index.md", "# Start\n\n```console\n$ boxferry --version\n```\n")
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
        self.assertEqual({name for name, selected in plan["jobs"].items() if selected}, set(POLICY["profile_jobs"]["prose"]))

    def test_executable_docs_and_new_code_fences_select_focused_suite(self) -> None:
        write(self.root, "docs/public/getting-started/index.md", "# Start\n\nUpdated example.\n")
        head = self.commit()
        self.assertEqual(self.plan(self.base, head)["profile"], "executable-docs")
        write(self.root, "docs/public/concepts/index.md", "# Concepts\n\n```console\n$ boxferry --version\n```\n")
        next_head = self.commit("new example")
        self.assertEqual(self.plan(head, next_head)["profile"], "executable-docs")

    def test_new_inline_cli_examples_are_not_treated_as_prose(self) -> None:
        write(self.root, "docs/public/concepts/index.md", "# Concepts\n\nUse `boxferry convert --help`.\n")
        head = self.commit("inline command")
        self.assertEqual(self.plan(self.base, head)["profile"], "executable-docs")

    def test_indented_cli_examples_are_not_treated_as_prose(self) -> None:
        write(self.root, "docs/public/concepts/index.md", "# Concepts\n\n    boxferry convert --help\n")
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
        write(self.root, "docs/public/concepts/new.md", "# New page\n\n  \tboxferry convert --help\n")
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
        planner.verify_gate(plan, needs, policy, arguments(self.root, "pull_request", self.base, head))
        needs["rust"]["result"] = "skipped"
        with self.assertRaises(planner.PlanError):
            planner.verify_gate(plan, needs, policy, arguments(self.root, "pull_request", self.base, head))

    def test_code_policy_unknown_and_mixed_paths_are_full(self) -> None:
        for path in (
            "src/main.rs", "scripts/validation-plan.py", "scripts/validation-policy.json",
            "docs/documentation-examples.toml", "fixtures/sample.yaml", "README.md",
            "docs/public/development/testing/index.md", "docs/public/unknown.json",
        ):
            with self.subTest(path=path):
                write(self.root, path, "changed\n")
                head = self.commit(path)
                self.assertEqual(self.plan(self.base, head)["profile"], "full")
                git(self.root, "revert", "--no-edit", head)
                self.base = git(self.root, "rev-parse", "HEAD")
        write(self.root, "docs/public/concepts/index.md", "# Concepts\n\nNew prose.\n")
        write(self.root, "src/main.rs", "fn main() { println!(\"x\"); }\n")
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
        self.assertEqual(planner.make_plan(arguments(self.root, "local"), POLICY)["profile"], "full")

    def test_local_committed_branch_changes_are_included(self) -> None:
        git(self.root, "switch", "-qc", "issue")
        write(self.root, "docs/public/concepts/index.md", "# Concepts\n\nCommitted prose.\n")
        self.commit()
        self.assertEqual(planner.make_plan(arguments(self.root, "local"), POLICY)["profile"], "prose")

    def test_local_index_cannot_be_hidden_by_restoring_only_worktree_bytes(self) -> None:
        write(self.root, "src/main.rs", "fn main() { println!(\"staged\"); }\n")
        git(self.root, "add", "src/main.rs")
        write(self.root, "src/main.rs", "fn main() {}\n")
        write(self.root, "docs/public/concepts/index.md", "# Concepts\n\nOnly prose in the worktree.\n")
        plan = planner.make_plan(arguments(self.root, "local"), POLICY)
        self.assertEqual(plan["profile"], "full")

    def test_local_index_content_can_promote_restored_docs_to_executable(self) -> None:
        write(self.root, "docs/public/concepts/index.md", "# Concepts\n\n```console\n$ boxferry --version\n```\n")
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
        needs = {job: {"result": "success" if plan["jobs"][job] else "skipped"} for job in POLICY["jobs"]}
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

    def test_first_rollout_never_executes_candidate_classifier_or_accepts_a_skip(self) -> None:
        workflow = (SOURCE_ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        self.assertNotIn("scripts/validation-plan.py plan --force-full", workflow)
        self.assertNotIn("needs.validation-plan.outputs.profile", workflow)
        self.assertIn("if: fromJSON(needs.validation-plan.outputs.plan).profile == 'executable-docs'", workflow)
        job_declaration = f'jobs = "{" ".join(POLICY["jobs"])}".split()'
        self.assertEqual(workflow.count(job_declaration), 2)
        blocks = re.findall(
            r"(?ms)^[ \t]+python3 - <<'PY'(?: \| tee -a \"\$\{GITHUB_STEP_SUMMARY\}\")?\n(.*?)^[ \t]+PY$",
            workflow,
        )
        self.assertEqual(len(blocks), 2)
        planner_code, gate_code = (textwrap.dedent(block) for block in blocks)
        with tempfile.TemporaryDirectory(prefix="boxferry-bootstrap-plan-") as directory:
            output = Path(directory) / "github-output"
            environment = {
                **os.environ, "BASE_SHA": self.base, "HEAD_SHA": self.base,
                "GITHUB_SHA": self.base, "GITHUB_OUTPUT": str(output),
            }
            planned = subprocess.run(
                [sys.executable, "-c", planner_code], env=environment,
                capture_output=True, text=True, check=False,
            )
            self.assertEqual(planned.returncode, 0, planned.stderr)
            fields = dict(line.split("=", 1) for line in output.read_text().splitlines())
            plan = json.loads(fields["plan"])
            self.assertEqual(plan["schema"], POLICY["schema"])
            self.assertEqual(plan["profile"], "full")
            self.assertTrue(all(plan["jobs"].values()))
            self.assertEqual(
                {key for key, value in fields.items() if key.startswith("select_") and value == "true"},
                {"select_" + job.replace("-", "_") for job in POLICY["jobs"]},
            )
            needs = {job: {"result": "success"} for job in POLICY["jobs"]}
            needs["validation-plan"] = {"result": "success"}
            environment["PLAN_JSON"] = fields["plan"]
            environment["NEEDS_JSON"] = json.dumps(needs)
            verified = subprocess.run(
                [sys.executable, "-c", gate_code], env=environment,
                capture_output=True, text=True, check=False,
            )
            self.assertEqual(verified.returncode, 0, verified.stderr)
            needs["rust"]["result"] = "skipped"
            environment["NEEDS_JSON"] = json.dumps(needs)
            rejected = subprocess.run(
                [sys.executable, "-c", gate_code], env=environment,
                capture_output=True, text=True, check=False,
            )
            self.assertNotEqual(rejected.returncode, 0)


if __name__ == "__main__":
    unittest.main()
