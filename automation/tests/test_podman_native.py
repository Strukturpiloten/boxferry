"""Offline CLI coverage for PodmanLens conditional native qualification."""

from __future__ import annotations

import contextlib
import io
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from typing import Any
from unittest.mock import patch

from boxferry_ci.__main__ import main

# Both fixture files come from PodmanLens main 20914f2cce74066fd1ff9474c12b56c3fe874244.
# The digest replacement is the sole change in PodmanLens PR #184 at 605cf5d2.
FIXTURE = Path(__file__).with_name("fixtures") / "podman_native_conformance"
PIN_PATH = "scripts/native-runtime-pins.sh"
POLICY_PATH = "scripts/validation-policy.json"
OPT_IN = "podman_native_conformance"
BOOTSTRAP = "--bootstrap-podman-native-conformance"
JOB = "native-conformance"
COMPLETE_EVENTS = ("push", "workflow_dispatch", "workflow_call")
OLD = "4c23d9596db49777190ff3a6ed7bae33374c409d1f62affc8ef654da10416ffc"
PROPOSED = "6d8462b7b0474af891c3e84e680df7bdfb487cf37ccb6239b5e05444d1b92cbd"


class NativeConformanceCli(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name) / "source"
        self.root.mkdir()
        self.trusted = self.root.parent / "trusted"
        self.output = self.root.parent / "github-output"
        self.git("init", "-q", "-b", "main")
        self.git("config", "user.name", "Fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        (self.root / "scripts").mkdir()
        (self.root / POLICY_PATH).write_bytes(
            (FIXTURE / "pre-adoption-policy.json.txt").read_bytes()
        )
        self.pins = (FIXTURE / "fedora-pins.sh.txt").read_text()
        (self.root / PIN_PATH).write_text(self.pins)
        (self.root / "README.md").write_text("Maintenance documentation.\n")
        (self.root / "lib.rs").write_text("struct Source;\n")
        self.git("add", ".")
        self.git("commit", "-qm", "trusted base")
        self.base = self.git("rev-parse", "HEAD")
        self.git("worktree", "add", "--quiet", "--detach", str(self.trusted), self.base)

    def git(self, *args: str) -> str:
        return subprocess.check_output(["git", "-C", str(self.root), *args], text=True).strip()

    def commit(self, *parents: str) -> str:
        self.git("add", "-A")
        return self.git(
            "commit-tree",
            self.git("write-tree"),
            "-m",
            "test candidate",
            *(arg for parent in parents for arg in ("-p", parent)),
        )

    def checkout(self, revision: str) -> None:
        self.git("checkout", "--force", "--quiet", "--detach", revision)

    def mark_policy(self, value: object) -> None:
        path = self.root / POLICY_PATH
        policy = json.loads(path.read_text())
        policy[OPT_IN] = value
        path.write_text(json.dumps(policy))

    def use_current_as_base(self) -> None:
        self.base = self.commit(self.base)
        self.checkout(self.base)
        subprocess.run(
            ["git", "-C", str(self.trusted), "checkout", "--quiet", "--detach", self.base],
            check=True,
        )

    def candidate(self, *, event: str = "pull_request", parent: str | None = None) -> list[str]:
        head = self.commit(parent or self.base)
        tested = head if event == "merge_group" else self.commit(self.base, head)
        self.checkout(tested)
        return [
            "--repository-root",
            str(self.root),
            "--config",
            str(self.trusted / POLICY_PATH),
            "--event",
            event,
            "--base-sha",
            self.base,
            "--head-sha",
            head,
            "--tested-sha",
            tested,
            "--policy-sha",
            self.base,
        ]

    def run_cli(self, argv: list[str], *, expected: str = "") -> str:
        stdout = io.StringIO()
        stderr = io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            result = main(argv)
        if expected:
            self.assertEqual(result, 1, stdout.getvalue())
            self.assertIn(expected, stderr.getvalue())
        else:
            self.assertEqual(result, 0, stderr.getvalue())
        return stdout.getvalue()

    def complete_args(self, event: str, config: str = POLICY_PATH) -> list[str]:
        return [
            "--repository-root",
            str(self.root),
            "--config",
            str(self.trusted / config),
            "--event",
            event,
            "--tested-sha",
            self.base,
            "--policy-sha",
            self.base,
        ]

    def plan(
        self, args: list[str], *, selected: bool | None, bootstrap: bool = True
    ) -> dict[str, Any]:
        self.output.unlink(missing_ok=True)
        stdout = self.run_cli(
            [
                "plan",
                *args,
                "--github-output",
                str(self.output),
                *([BOOTSTRAP] if bootstrap else []),
            ]
        )
        plan: dict[str, Any] = json.loads(stdout)
        outputs = self.output.read_text().splitlines()
        native = [out for out in outputs if out.startswith("select_native_conformance=")]
        self.assertEqual(
            native,
            [] if selected is None else [f"select_native_conformance={str(selected).lower()}"],
        )
        self.assertEqual(plan["schema"], 3)
        self.assertNotIn(JOB, plan["jobs"])
        return plan

    def needs(self, plan: dict[str, Any], selected: bool | None) -> dict[str, Any]:
        tested = plan["tested_sha"]
        needs: dict[str, Any] = {
            "validation-plan": {"result": "success"},
            **{
                job: {"result": "success" if required else "skipped"}
                for job, required in plan["jobs"].items()
            },
        }
        if plan["jobs"]["devcontainer-validation"]:
            needs["devcontainer-validation"]["outputs"] = {"candidate_sha": tested}
        if selected is not None:
            needs[JOB] = (
                {"result": "success", "outputs": {"candidate_sha": tested}}
                if selected
                else {"result": "skipped"}
            )
        return needs

    def gate(
        self,
        args: list[str],
        plan: dict[str, Any],
        needs: dict[str, Any],
        *,
        bootstrap: bool = True,
        expected: str = "",
    ) -> str:
        return self.run_cli(
            [
                "gate",
                *args,
                "--plan-json",
                json.dumps(plan),
                "--needs-json",
                json.dumps(needs),
                "--devcontainer-validation",
                *([BOOTSTRAP] if bootstrap else []),
            ],
            expected=expected,
        )

    def change_digest(self) -> None:
        self.assertEqual(self.pins.count(OLD), 1)
        (self.root / PIN_PATH).write_text(self.pins.replace(OLD, PROPOSED))

    def test_actual_184_digest_and_combined_group_select_without_changing_the_base_plan(
        self,
    ) -> None:
        self.change_digest()
        args = self.candidate()
        plan = self.plan(args, selected=True)
        self.assertEqual(plan, self.plan(args, selected=None, bootstrap=False))
        self.assertIn(
            [JOB, "success"], json.loads(self.gate(args, plan, self.needs(plan, True)))["results"]
        )

        self.checkout(self.base)
        self.change_digest()
        earlier_in_group = self.commit(self.base)
        (self.root / "README.md").write_text("Different maintenance documentation.\n")
        args = self.candidate(event="merge_group", parent=earlier_in_group)
        plan = self.plan(args, selected=True)
        self.assertEqual(plan["profile"], "full")
        self.gate(args, plan, self.needs(plan, True))

    def test_unaffected_full_and_narrow_changes_require_exactly_a_skip(self) -> None:
        for path, content, profile in (
            ("lib.rs", "struct Unrelated;\n", "full"),
            ("README.md", "Different maintenance documentation.\n", "prose"),
        ):
            for event in ("pull_request", "merge_group"):
                with self.subTest(profile=profile, event=event):
                    self.checkout(self.base)
                    (self.root / path).write_text(content)
                    args = self.candidate(event=event)
                    plan = self.plan(args, selected=False)
                    self.assertEqual(plan["profile"], profile)
                    self.assertIn(
                        [JOB, "skipped"],
                        json.loads(self.gate(args, plan, self.needs(plan, False)))["results"],
                    )
                    self.gate(
                        args, plan, self.needs(plan, True), expected="unselected native-conformance"
                    )
                    self.gate(
                        args, plan, self.needs(plan, None), expected="native-conformance outcome"
                    )

    def test_all_other_dependencies_and_the_devcontainer_receipt_still_apply(self) -> None:
        self.change_digest()
        args = self.candidate()
        plan = self.plan(args, selected=True)
        needs = self.needs(plan, True)
        for outcome, diagnostic in (
            ({"result": "failure"}, "selected native-conformance"),
            ({"result": "cancelled"}, "selected native-conformance"),
            ({"result": "skipped"}, "selected native-conformance"),
            ({"result": None}, "missing or unknown result"),
            ({"result": "success"}, "native-conformance outputs"),
            ({"result": "success", "outputs": {}}, "native conformance candidate receipt"),
            ({"result": "success", "outputs": {"candidate_sha": self.base}}, "tested candidate"),
        ):
            with self.subTest(outcome=outcome):
                self.gate(args, plan, {**needs, JOB: outcome}, expected=diagnostic)
        self.gate(args, plan, self.needs(plan, None), expected="native-conformance outcome")
        self.gate(
            args,
            plan,
            {**needs, "unexpected": {"result": "success"}},
            expected="aggregate job dependencies do not match policy",
        )
        self.gate(args, plan, {**needs, "rust": {"result": "failure"}}, expected="selected rust")
        self.gate(
            args,
            plan,
            {
                **needs,
                "devcontainer-validation": {
                    "result": "success",
                    "outputs": {"candidate_sha": self.base},
                },
            },
            expected="Dev Container receipt",
        )

    def test_introducing_policy_keeps_native_unselected_and_needs_the_bootstrap_flag(self) -> None:
        self.mark_policy(True)
        args = self.candidate()
        plan = self.plan(args, selected=False)
        self.gate(args, plan, self.needs(plan, False))
        self.plan(args, selected=None, bootstrap=False)
        self.gate(
            args,
            plan,
            self.needs(plan, False),
            bootstrap=False,
            expected="aggregate job dependencies do not match policy",
        )

    def test_trusted_activation_does_not_allow_omission_or_candidate_disabling(self) -> None:
        self.mark_policy(True)
        self.use_current_as_base()
        self.change_digest()
        args = self.candidate()
        plan = self.plan(args, selected=True, bootstrap=False)
        self.gate(args, plan, self.needs(plan, True), bootstrap=False)
        self.gate(
            args,
            plan,
            self.needs(plan, None),
            bootstrap=False,
            expected="native-conformance outcome",
        )
        for replacement in (False, None):
            with self.subTest(replacement=replacement):
                self.checkout(self.base)
                if replacement is None:
                    (self.root / POLICY_PATH).write_bytes(
                        (FIXTURE / "pre-adoption-policy.json.txt").read_bytes()
                    )
                else:
                    self.mark_policy(replacement)
                if replacement is False:
                    self.change_digest()
                args = self.candidate()
                self.run_cli(["plan", *args], expected="cannot disable")
                self.gate(
                    args, plan, self.needs(plan, None), bootstrap=False, expected="cannot disable"
                )
                untrusted = list(args)
                untrusted[untrusted.index("--config") + 1] = str(self.root / POLICY_PATH)
                self.run_cli(["plan", *untrusted], expected="differ from the trusted base")

    def test_bootstrap_requires_the_reviewed_policy_and_cannot_be_borrowed(self) -> None:
        config = self.root / POLICY_PATH
        config.write_text(config.read_text() + "\n")
        self.use_current_as_base()
        args = self.candidate()
        self.run_cli(["plan", *args, BOOTSTRAP], expected="unapproved native conformance")
        self.plan(args, selected=None, bootstrap=False)

        self.checkout(self.base)
        policy = json.loads(config.read_text())
        policy["repository"] = "Strukturpiloten/compose-lens"
        config.write_text(json.dumps(policy))
        self.use_current_as_base()
        args = self.candidate()
        self.run_cli(["plan", *args, BOOTSTRAP], expected="supplemental PodmanLens prerequisite")

    def test_legacy_explicit_config_path_works_until_the_canonical_policy_activates(self) -> None:
        alternative = "scripts/alternate-policy.json"
        canonical = self.root / POLICY_PATH
        (self.root / alternative).write_bytes(canonical.read_bytes() + b"\n")
        self.use_current_as_base()
        self.change_digest()
        args = self.candidate()
        index = args.index("--config") + 1
        args[index] = str(self.trusted / alternative)
        plan = self.plan(args, selected=None, bootstrap=False)
        self.gate(args, plan, self.needs(plan, None), bootstrap=False)

        self.checkout(self.base)
        for event in COMPLETE_EVENTS:
            args = self.complete_args(event, alternative)
            plan = self.plan(args, selected=None, bootstrap=False)
            self.gate(args, plan, self.needs(plan, None), bootstrap=False)

        self.checkout(self.base)
        self.mark_policy(True)
        foreign = json.loads(canonical.read_text())
        foreign.pop(OPT_IN)
        foreign["repository"] = "Strukturpiloten/compose-lens"
        (self.root / "scripts/foreign-policy.json").write_text(json.dumps(foreign))
        self.use_current_as_base()
        self.change_digest()
        args = self.candidate()
        plan = self.plan(args, selected=True, bootstrap=False)
        args[index] = str(self.trusted / alternative)
        self.run_cli(["plan", *args], expected="canonical trusted PodmanLens policy")
        self.gate(
            args,
            plan,
            self.needs(plan, None),
            bootstrap=False,
            expected="canonical trusted PodmanLens policy",
        )
        args[index] = str(self.trusted / "scripts/foreign-policy.json")
        with patch.dict(os.environ, {"GITHUB_REPOSITORY": "Strukturpiloten/podman-lens"}):
            self.run_cli(["plan", *args], expected="repository does not match PodmanLens")

        self.checkout(self.base)
        for event in COMPLETE_EVENTS:
            with self.subTest(event=event):
                plan = self.plan(self.complete_args(event), selected=False, bootstrap=False)
                args = self.complete_args(event, alternative)
                message = "canonical trusted PodmanLens policy"
                self.run_cli(["plan", *args], expected=message)
                self.gate(args, plan, self.needs(plan, None), bootstrap=False, expected=message)

    def test_invalid_declarations_and_git_identities_refuse_selection(self) -> None:
        duplicate = "readonly PODMAN_NATIVE_BASE_IMAGE=registry.fedoraproject.org/fedora:45@sha256:"
        for bad, phrase in (
            (None, "missing owned Fedora"),
            (self.pins.replace("fedora:45@", "fedora:46@"), "reviewed Fedora"),
            (self.pins.replace(OLD, "invalid"), "canonical repository"),
            (self.pins + "\necho not-inert\n", "inert"),
            (self.pins + duplicate + OLD + "\n", "one annotated Fedora"),
        ):
            with self.subTest(declaration=bad):
                self.checkout(self.base)
                path = self.root / PIN_PATH
                if bad is None:
                    path.unlink()
                else:
                    path.write_text(bad)
                self.output.unlink(missing_ok=True)
                args = self.candidate()
                self.run_cli(
                    ["plan", *args, BOOTSTRAP, "--github-output", str(self.output)], expected=phrase
                )
                self.assertFalse(self.output.exists())

        for event in ("pull_request", "merge_group"):
            with self.subTest(event=event):
                self.checkout(self.base)
                self.change_digest()
                args = self.candidate(event=event)
                args[args.index("--head-sha") + 1] = self.base
                self.run_cli(
                    ["plan", *args, BOOTSTRAP],
                    expected="native PR comparison"
                    if event == "pull_request"
                    else "native group comparison",
                )

    def test_main_manual_and_release_call_keep_the_original_plan_with_native_skipped(self) -> None:
        self.mark_policy(True)
        self.use_current_as_base()
        for event in COMPLETE_EVENTS:
            with self.subTest(event=event):
                args = self.complete_args(event)
                plan = self.plan(args, selected=False, bootstrap=False)
                self.assertEqual(plan["profile"], "full")
                self.gate(args, plan, self.needs(plan, False), bootstrap=False)
                self.gate(
                    args,
                    plan,
                    self.needs(plan, None),
                    bootstrap=False,
                    expected="native-conformance outcome",
                )
