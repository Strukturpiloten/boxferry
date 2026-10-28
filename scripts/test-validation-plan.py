#!/usr/bin/env python3
"""Regression contracts for the shared change-aware validation planner."""

from __future__ import annotations

import argparse
import copy
import hashlib
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
        self.assertFalse(plan["jobs"]["docker-forgejo-application"])
        self.assertEqual({name for name, selected in plan["jobs"].items() if selected}, set(POLICY["profile_jobs"]["prose"]))

    def test_executable_docs_and_new_code_fences_select_focused_suite(self) -> None:
        write(self.root, "docs/public/getting-started/index.md", "# Start\n\nUpdated example.\n")
        head = self.commit()
        self.assertEqual(self.plan(self.base, head)["profile"], "executable-docs")
        self.assertFalse(self.plan(self.base, head)["jobs"]["docker-forgejo-application"])
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
                self.assertTrue(self.plan(self.base, head)["jobs"]["docker-forgejo-application"])
                git(self.root, "revert", "--no-edit", head)
                self.base = git(self.root, "rev-parse", "HEAD")
        write(self.root, "docs/public/concepts/index.md", "# Concepts\n\nNew prose.\n")
        write(self.root, "src/main.rs", "fn main() { println!(\"x\"); }\n")
        head = self.commit("mixed")
        self.assertEqual(self.plan(self.base, head)["profile"], "full")
        self.assertTrue(self.plan(self.base, head)["jobs"]["docker-forgejo-application"])

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
        for plan in (self.plan(self.base, self.base), self.plan("0" * 40, self.base)):
            self.assertEqual(plan["profile"], "full")
            self.assertTrue(plan["jobs"]["docker-forgejo-application"])
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
                self.assertTrue(plan["jobs"]["docker-forgejo-application"])

    def test_forgejo_source_contract_job_is_independent_and_full_only(self) -> None:
        name = "docker-forgejo-application"
        self.assertIn(name, POLICY["jobs"])
        for profile in ("prose", "executable-docs"):
            self.assertNotIn(name, POLICY["profile_jobs"][profile])
        workflow = (SOURCE_ROOT / ".github/workflows/ci.yml").read_text()
        declaration = re.search(r"(?ms)^  docker-forgejo-application:\n(.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)", workflow)
        self.assertIsNotNone(declaration)
        job = declaration[1]
        self.assertIn("    needs: validation-plan\n", job)
        self.assertIn("    if: needs.validation-plan.outputs.select_docker_forgejo_application == 'true'\n", job)
        self.assertIn("    runs-on: ubuntu-24.04\n", job)
        self.assertIn("    timeout-minutes: 10\n", job)
        self.assertIn("      - *checkout\n", job)
        command = "run: bash scripts/test-docker-forgejo-application.sh"
        self.assertEqual(workflow.count(command), 1)
        self.assertIn(command, job)
        self.assertEqual(job.count("        run:"), 1)
        self.assertNotIn("secrets.", job)
        self.assertNotIn("sudo", job)
        self.assertIn("select_docker_forgejo_application: ${{ steps.plan.outputs.select_docker_forgejo_application }}", workflow)
        self.assertNotIn("test-docker-forgejo-application.sh", (SOURCE_ROOT / "scripts/test-application-probes.sh").read_text())

    def test_forgejo_source_job_selected_failure_or_missing_needs_blocks_gate(self) -> None:
        plan = planner.make_plan(arguments(self.root, "push"), POLICY)
        needs = {job: {"result": "success"} for job in POLICY["jobs"]}
        needs["validation-plan"] = {"result": "success"}
        args = arguments(self.root, "push")
        planner.verify_gate(plan, needs, POLICY, args)
        for result in ("skipped", "failure", "cancelled", "timed_out", None):
            with self.subTest(result=result):
                changed = copy.deepcopy(needs)
                changed["docker-forgejo-application"]["result"] = result
                with self.assertRaises(planner.PlanError):
                    planner.verify_gate(plan, changed, POLICY, args)
        for change in ("missing", "extra"):
            with self.subTest(needs=change):
                changed = copy.deepcopy(needs)
                if change == "missing":
                    del changed["docker-forgejo-application"]
                else:
                    changed["unexpected-job"] = {"result": "success"}
                with self.assertRaises(planner.PlanError):
                    planner.verify_gate(plan, changed, POLICY, args)

    def test_gate_rejects_missing_skipped_failed_and_cancelled_required_jobs(self) -> None:
        write(self.root, "docs/public/concepts/index.md", "# Concepts\n\nNew prose.\n")
        head = self.commit()
        plan = self.plan(self.base, head)
        needs = {job: {"result": "success" if plan["jobs"][job] else "skipped"} for job in POLICY["jobs"]}
        needs["validation-plan"] = {"result": "success"}
        args = arguments(self.root, "pull_request", self.base, head)
        self.assertEqual(needs["docker-forgejo-application"]["result"], "skipped")
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

    def test_required_ci_is_the_real_always_running_aggregate(self) -> None:
        workflow = (SOURCE_ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        aggregates = re.findall(
            r"(?ms)^  pr-gate:\n(.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)", workflow,
        )
        self.assertEqual(len(aggregates), 1)
        aggregate = aggregates[0]
        self.assertIn("    name: Required CI\n", aggregate)
        self.assertEqual(workflow.count("    name: Required CI\n"), 1)
        self.assertIn("    if: always()\n", aggregate)
        dependencies = re.search(r"(?s)    needs:\n\s*\[(.*?)\]", aggregate)
        self.assertIsNotNone(dependencies)
        assert dependencies is not None
        jobs = [name.strip() for name in dependencies[1].split(",") if name.strip()]
        self.assertEqual(len(jobs), len(set(jobs)))
        self.assertEqual(set(jobs), {"validation-plan", *POLICY["jobs"]})
        self.assertIn('python3 "${verifier}" gate --policy-root "${policy_root}"', aggregate)

    def test_reviewed_legacy_base_rollout_uses_only_exact_two_file_cohort(self) -> None:
        policy_digest = "2c2b99cc7e5e1baf3ad00a6f1ab27a2f9e3036e0fa8a19a90d4cf7271a7806b7"
        classifier_digest = "df2f7fa2adfc0002bd3984c91e72ba8e1412ced36e7f3acd7003960242412417"
        workflow = (SOURCE_ROOT / ".github/workflows/ci.yml").read_text()
        for name, digest in (("POLICY", policy_digest), ("CLASSIFIER", classifier_digest)):
            self.assertIn(f"  FORGEJO_LEGACY_BASE_{name}_SHA256: {digest}\n", workflow)
        self.assertEqual(workflow.count("known_legacy_base() {"), 2)
        self.assertEqual(workflow.count("sha256sum --check --status"), 2)

        def shell_block(job: str) -> str:
            section = re.search(rf"(?ms)^  {job}:\n(.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)", workflow)[1]
            return textwrap.dedent(re.search(r"(?ms)^        run: \|\n(.*)", section)[1])

        select_shell, gate_shell = shell_block("validation-plan"), shell_block("pr-gate")
        classifier = (SOURCE_ROOT / "scripts/validation-plan.py").read_bytes()
        self.assertEqual(hashlib.sha256(classifier).hexdigest(), classifier_digest)
        current_policy = (SOURCE_ROOT / "scripts/validation-policy.json").read_bytes()
        replacement = b'    "lockfile-release-age",\n    "docker-forgejo-application"\n'
        self.assertEqual(current_policy.count(replacement), 1)
        legacy_policy = current_policy.replace(replacement, b'    "lockfile-release-age"\n')
        self.assertEqual(hashlib.sha256(legacy_policy).hexdigest(), policy_digest)
        base_directory = self.root / ".validation-base/scripts"
        base_directory.mkdir(parents=True)
        output = self.root / "github-output"
        environment = {
            **os.environ, "EVENT_NAME": "pull_request", "BASE_SHA": self.base,
            "HEAD_SHA": self.base, "GITHUB_SHA": self.base, "GITHUB_WORKSPACE": str(self.root),
            "GITHUB_OUTPUT": str(output), "GITHUB_STEP_SUMMARY": str(self.root / "summary"),
            "FORGEJO_LEGACY_BASE_POLICY_SHA256": policy_digest,
            "FORGEJO_LEGACY_BASE_CLASSIFIER_SHA256": classifier_digest,
        }
        expected_needs = {job: {"result": "success"} for job in POLICY["jobs"]}
        expected_needs["validation-plan"] = {"result": "success"}

        def run(script: str, extra: dict[str, str] | None = None) -> subprocess.CompletedProcess:
            return subprocess.run(["bash", "--noprofile", "--norc", "-c", script], cwd=self.root,
                                  env={**environment, **(extra or {})}, capture_output=True, text=True)

        for cohort in ("exact", "policy-byte-drift", "classifier-byte-drift", "next-main", "absent"):
            with self.subTest(cohort=cohort):
                policy_bytes = current_policy if cohort == "next-main" else legacy_policy
                (base_directory / "validation-policy.json").write_bytes(policy_bytes + (b"\n" if cohort == "policy-byte-drift" else b""))
                (base_directory / "validation-plan.py").write_bytes(classifier + (b"\n" if cohort == "classifier-byte-drift" else b""))
                if cohort == "absent":
                    (base_directory / "validation-policy.json").unlink()
                    (base_directory / "validation-plan.py").unlink()
                output.unlink(missing_ok=True)
                selected = run(select_shell)
                self.assertEqual(selected.returncode, 0, selected.stderr)
                fields = dict(line.split("=", 1) for line in output.read_text().splitlines())
                plan = json.loads(fields["plan"])
                extra = {"PLAN_JSON": fields["plan"], "NEEDS_JSON": json.dumps(expected_needs)}
                verified = run(gate_shell, extra)
                if cohort.endswith("byte-drift"):
                    self.assertNotIn("docker-forgejo-application", plan["jobs"])
                    self.assertNotEqual(verified.returncode, 0)
                    self.assertNotIn("First-rollout complete validation", verified.stdout)
                    continue
                self.assertEqual(plan["profile"], "full")
                self.assertEqual(plan["jobs"], {job: True for job in POLICY["jobs"]})
                self.assertEqual(fields["select_docker_forgejo_application"], "true")
                self.assertEqual(verified.returncode, 0, verified.stderr)
                if cohort == "next-main":
                    self.assertNotIn("First-rollout complete validation", verified.stdout)
                    continue
                self.assertIn("First-rollout complete validation", verified.stdout)
                for broken in ("new-skipped", "old-failed", "missing-new", "missing-old", "missing-plan", "extra"):
                    with self.subTest(cohort=cohort, needs=broken):
                        needs = copy.deepcopy(expected_needs)
                        if broken == "new-skipped":
                            needs["docker-forgejo-application"]["result"] = "skipped"
                        elif broken == "old-failed":
                            needs["rust"]["result"] = "failure"
                        elif broken.startswith("missing"):
                            del needs[{"missing-new": "docker-forgejo-application", "missing-old": "rust", "missing-plan": "validation-plan"}[broken]]
                        else:
                            needs["unexpected"] = {"result": "success"}
                        refused = run(gate_shell, {**extra, "NEEDS_JSON": json.dumps(needs)})
                        self.assertNotEqual(refused.returncode, 0)

    def test_first_rollout_never_executes_candidate_classifier_or_accepts_a_skip(self) -> None:
        workflow = (SOURCE_ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        self.assertIn("run: bash scripts/test-application-probes.sh", workflow)
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
            for result in ("skipped", "failure", "cancelled", None):
                with self.subTest(bootstrap_forgejo_result=result):
                    changed = {job: {"result": "success"} for job in POLICY["jobs"]}
                    changed["validation-plan"] = {"result": "success"}
                    changed["docker-forgejo-application"]["result"] = result
                    environment["NEEDS_JSON"] = json.dumps(changed)
                    rejected = subprocess.run(
                        [sys.executable, "-c", gate_code], env=environment,
                        capture_output=True, text=True, check=False,
                    )
                    self.assertNotEqual(rejected.returncode, 0)


if __name__ == "__main__":
    unittest.main()
