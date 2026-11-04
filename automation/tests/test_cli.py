from __future__ import annotations

import argparse
import contextlib
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from boxferry_ci.__main__ import _snapshot, _task, main
from boxferry_ci.contracts import ContractError
from boxferry_ci.policy import load_policy


class Cli(unittest.TestCase):
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
                    "repository": "Strukturpiloten/example",
                    "jobs": ["quality"],
                    "profile_jobs": {"maintenance": ["quality"]},
                    "public_documentation_root": "docs/",
                    "full_documentation_roots": [],
                    "executable_documentation_roots": [],
                    "executable_documentation_paths": [],
                    "documentation_examples_manifest": None,
                    "path_profiles": {"maintenance": ["README.md"]},
                    "local_commands": {"maintenance": [["true"]], "full": [["true"]]},
                    "tasks": {"quality": {"commands": [["true"]], "timeout_seconds": 1}},
                }
            )
        )

    def command(self, *args: str) -> str:
        return subprocess.run(
            ("git", *args),
            cwd=self.root,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()

    def arguments(self, command: str, *extra: str) -> list[str]:
        return [command, "--repository-root", str(self.root), "--config", str(self.config), *extra]

    def test_task_writes_actionable_json_and_summary(self) -> None:
        report, summary = self.root / "report.json", self.root / "summary.md"
        self.assertEqual(
            main(
                self.arguments(
                    "run",
                    "--task",
                    "quality",
                    "--report-json",
                    str(report),
                    "--summary",
                    str(summary),
                )
            ),
            0,
        )
        evidence = json.loads(report.read_text())
        self.assertEqual(evidence["status"], "success")
        diagnostic = evidence["results"][0]["diagnostic"]
        self.assertEqual(
            set(diagnostic),
            {
                "stage",
                "cause",
                "affected_input",
                "evidence_link",
                "next_action",
            },
        )
        self.assertEqual(diagnostic["evidence_link"], str(report))
        self.assertIn("stage:", summary.read_text())

    def test_unknown_task_fails_with_report_and_never_runs_shell(self) -> None:
        report = self.root / "report.json"
        with contextlib.redirect_stderr(io.StringIO()):
            result = main(self.arguments("run", "--task", "unknown", "--report-json", str(report)))
        self.assertEqual(result, 1)
        evidence = json.loads(report.read_text())
        self.assertEqual(evidence["status"], "infrastructure")
        self.assertIn("not configured", evidence["cause"])
        self.assertIn("next_action", evidence["results"][0]["diagnostic"])

    def test_silent_failure_identifies_redacted_command_in_json_and_summary(self) -> None:
        report, summary = self.root / "report.json", self.root / "summary.md"
        raw = json.loads(self.config.read_text())
        raw["tasks"]["quality"]["commands"] = [
            [
                sys.executable,
                "-c",
                "import sys; sys.exit(9)",
                "--token",
                "private-task-token",
            ]
        ]
        self.config.write_text(json.dumps(raw))
        self.assertEqual(
            main(
                self.arguments(
                    "run",
                    "--task",
                    "quality",
                    "--report-json",
                    str(report),
                    "--summary",
                    str(summary),
                )
            ),
            1,
        )
        evidence = json.loads(report.read_text())
        result = evidence["results"][0]
        self.assertEqual(result["stdout"], "")
        self.assertEqual(result["stderr"], "")
        self.assertEqual(result["returncode"], 9)
        self.assertEqual(
            result["argv"],
            [
                Path(sys.executable).name,
                "-c",
                "import sys; sys.exit(9)",
                "--token",
                "[REDACTED]",
            ],
        )
        self.assertIn("argv:", summary.read_text())
        self.assertIn("import sys; sys.exit(9)", summary.read_text())
        self.assertNotIn("private-task-token", report.read_text() + summary.read_text())

    def test_gate_check_failure_is_distinct_from_infrastructure(self) -> None:
        revision = "a" * 40
        plan = {
            "schema": 3,
            "repository": "Strukturpiloten/example",
            "event": "push",
            "base_sha": revision,
            "head_sha": revision,
            "tested_sha": revision,
            "profile": "full",
            "profiles": ["full"],
            "jobs": {"quality": True},
            "reasons": ["full push validation"],
            "local_fingerprint": None,
        }
        needs = {"validation-plan": {"result": "success"}, "quality": {"result": "failure"}}
        report = self.root / "report.json"
        with contextlib.redirect_stderr(io.StringIO()):
            result = main(
                self.arguments(
                    "gate",
                    "--event",
                    "push",
                    "--tested-sha",
                    revision,
                    "--plan-json",
                    json.dumps(plan),
                    "--needs-json",
                    json.dumps(needs),
                    "--report-json",
                    str(report),
                )
            )
        self.assertEqual(result, 1)
        self.assertEqual(json.loads(report.read_text())["status"], "failed")

    def test_policy_rejects_shell_expressions_and_boolean_timeout(self) -> None:
        original = json.loads(self.config.read_text())
        for task in (
            {"commands": [["bash", "-c", "touch unauthorized"]]},
            {"commands": [["true"]], "timeout_seconds": True},
        ):
            with self.subTest(task=task), self.assertRaises(ContractError):
                self.config.write_text(json.dumps(dict(original, tasks={"quality": task})))
                load_policy(self.config)
        self.assertFalse((self.root / "unauthorized").exists())

    def test_job_names_cannot_inject_github_output(self) -> None:
        raw = json.loads(self.config.read_text())
        raw["jobs"] = ["quality\ninjected=true"]
        self.config.write_text(json.dumps(raw))
        with self.assertRaises(ContractError):
            load_policy(self.config)

    def test_shared_cargo_target_is_rejected_before_execution(self) -> None:
        with patch.dict(os.environ, {"CARGO_TARGET_DIR": str(self.root.parent)}):
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(main(self.arguments("run", "--task", "quality")), 1)

    def test_rust_defaults_select_optional_aliases_from_trusted_base(self) -> None:
        self.command("init", "--initial-branch=main")
        self.command("config", "user.email", "fixture@example.invalid")
        self.command("config", "user.name", "Fixture")
        cargo = self.root / ".cargo/config.toml"
        cargo.parent.mkdir()
        aliases = (
            "ci-check",
            "ci-policy",
            "ci-clippy",
            "ci-test",
            "ci-doc",
            "ci-doctest",
            "ci-package",
        )
        cargo.write_text("[alias]\n" + "\n".join(f'{name} = "check"' for name in aliases))
        self.command("add", "--all")
        self.command("commit", "--quiet", "-m", "trusted base")
        base = self.command("rev-parse", "HEAD")
        # Candidate deletion cannot remove the previously required package step.
        cargo.write_text('[alias]\nci-check = "check"\n')
        task = _task(
            argparse.Namespace(task="rust-quality", policy_sha=base), load_policy(self.config)
        )
        self.assertIn(("cargo", "ci-package"), task.commands)

    def test_dependency_snapshot_is_exact_and_copies_only_metadata(self) -> None:
        self.command("init", "--initial-branch=main")
        self.command("config", "user.email", "fixture@example.invalid")
        self.command("config", "user.name", "Fixture")
        (self.root / "Cargo.lock").write_text("locked baseline\n")
        (self.root / "private.txt").write_text("not dependency input\n")
        (self.root / ".devcontainer").mkdir()
        dockerfile = self.root / ".devcontainer/Dockerfile"
        declared = (
            "# renovate: datasource=crate depName=cargo-semver-checks\n"
            "ARG CARGO_SEMVER_CHECKS_VERSION=0.51.0\n"
        )
        dockerfile.write_text(declared)
        self.command("add", "--all")
        self.command("commit", "--quiet", "-m", "baseline")
        revision = self.command("rev-parse", "HEAD")
        (self.root / "Cargo.lock").write_text("untrusted working copy\n")
        dockerfile.write_text("untrusted Dockerfile working copy\n")
        target = self.root / "snapshot"
        target.mkdir()
        _snapshot(self.root, revision, target)
        self.assertEqual((target / "Cargo.lock").read_text(), "locked baseline\n")
        self.assertFalse((target / "private.txt").exists())
        self.assertEqual((target / ".devcontainer/Dockerfile").read_text(), declared)
