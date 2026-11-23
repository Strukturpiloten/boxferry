"""Read-only inventory CLI and reusable boundary, without registry or publication."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from collections.abc import Mapping
from pathlib import Path

import yaml

from boxferry_ci import distribution, release

ROOT = Path(__file__).resolve().parents[2]
SHA = "a" * 40
AUTOMATION = "b" * 40
WORKFLOW = ".github/workflows/shared-release-inventory.yml"


class InventoryTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="boxferry-inventory-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.output = self.root / "output"
        self.calls = self.root / "calls"
        automation_root = Path(release.__file__).resolve().parents[3]
        git = self.bin / "git"
        git.write_text(
            f"#!{sys.executable}\nimport sys\n"
            f"identities = { {str(self.root): SHA, str(automation_root): AUTOMATION}!r}\n"
            "assert sys.argv[1] == '-C' and sys.argv[3:] == ['rev-parse', 'HEAD']\n"
            "print(identities[sys.argv[2]])\n"
        )
        git.chmod(0o755)
        cargo = self.bin / "cargo"
        cargo.write_text(
            f"#!{sys.executable}\nimport os, sys\n"
            "assert sys.argv[1:] == ['metadata', '--locked', '--no-deps', "
            "'--format-version', '1']\n"
            f"with open({str(self.calls)!r}, 'a') as stream: stream.write('metadata\\n')\n"
            f"os.execv({shutil.which('cargo')!r}, ['cargo', *sys.argv[1:]])\n"
        )
        cargo.chmod(0o755)
        # Enforce no Python network access even if a future regression enters registry code.
        (self.bin / "sitecustomize.py").write_text(
            "import sys\n"
            "def audit(event, args):\n"
            "    if event.startswith('socket.'): raise RuntimeError('network forbidden')\n"
            "sys.addaudithook(audit)\n"
        )
        self.env = {
            **os.environ,
            "PATH": str(self.bin) + os.pathsep + os.environ.get("PATH", ""),
            "PYTHONPATH": str(self.bin) + os.pathsep + str(ROOT / "automation/src"),
            "GITHUB_REPOSITORY": "Strukturpiloten/boxferry",
            "GITHUB_SHA": SHA,
            "GITHUB_OUTPUT": str(self.output),
            "CARGO_NET_OFFLINE": "true",
        }

    def workspace(self, names: list[str]) -> None:
        (self.root / "Cargo.toml").write_text(
            '[workspace]\nresolver = "3"\nmembers = ' + json.dumps(names) + "\n"
        )
        for index, name in enumerate(names):
            crate = self.root / name
            (crate / "src").mkdir(parents=True)
            (crate / "src/lib.rs").write_text("// Authored inert fixture.\n")
            dependencies = ""
            if index:
                # Exercise normal, build and dev order through real Cargo metadata.
                kind = ("dependencies", "build-dependencies", "dev-dependencies")[(index - 1) % 3]
                previous = names[index - 1]
                dependencies = (
                    f'[{kind}]\n{previous} = {{ path = "../{previous}", version = "0.1.0" }}\n'
                )
            (crate / "Cargo.toml").write_text(
                f'[package]\nname = "{name}"\nversion = "0.1.0"\nedition = "2024"\n' + dependencies
            )
        self.policy = {
            "schema": 1,
            "repository": "Strukturpiloten/boxferry",
            "packages": names,
            "required_jobs": ["validate"],
            "publication_enabled": True,
        }
        self.config = self.root / "release.json"
        self.config.write_text(json.dumps(self.policy))

    def invoke(
        self, changes: Mapping[str, str | None] | None = None
    ) -> subprocess.CompletedProcess[str]:
        options: dict[str, str | None] = {
            "repository-root": str(self.root),
            "config": str(self.config),
            "candidate-sha": SHA,
            "automation-sha": AUTOMATION,
            "validation-sha": SHA,
            "validation-result": "success",
            "published-api-candidate-sha": SHA,
        }
        options.update(changes or {})
        args = ["prepare-inventory"]
        for key, value in options.items():
            if value is not None:
                args.extend(["--" + key, value])
        self.output.write_text("existing=retained\n")
        self.calls.write_text("")
        return subprocess.run(
            [sys.executable, "-m", "boxferry_ci.release", *args],
            env=self.env,
            cwd=self.root,
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )

    def test_real_metadata_once_for_single_and_six_crates(self) -> None:
        names = json.loads((ROOT / "scripts/shared-release-policy.json").read_text())["packages"]
        for inventory in (["compose-lens"], names):
            with self.subTest(inventory=inventory):
                self.workspace(inventory)
                child = self.invoke()
                self.assertEqual(child.returncode, 0, child.stderr)
                self.assertEqual(self.calls.read_text(), "metadata\n")
                self.assertEqual(
                    self.output.read_text(),
                    f"existing=retained\ncandidate_sha={SHA}\npublished_api_candidate_sha={SHA}\n",
                )

    def test_failed_missing_stale_proofs_refuse_before_metadata_and_emit_nothing(self) -> None:
        self.workspace(["compose-lens"])
        for key in ("validation-sha", "validation-result", "published-api-candidate-sha"):
            for value in (None, "", "failure", "cancelled", "skipped", AUTOMATION, SHA[:12]):
                with self.subTest(key=key, value=value):
                    child = self.invoke({key: value})
                    self.assertNotEqual(child.returncode, 0)
                    self.assertEqual(self.output.read_text(), "existing=retained\n")
                    self.assertEqual(self.calls.read_text(), "")
        for key in ("candidate-sha", "automation-sha"):
            child = self.invoke({key: "c" * 40})
            self.assertNotEqual(child.returncode, 0)
            self.assertEqual(self.output.read_text(), "existing=retained\n")
            self.assertEqual(self.calls.read_text(), "")

    def test_membership_and_each_dependency_kind_order_fail_without_receipt(self) -> None:
        self.workspace(["base", "normal", "build", "dev", "last", "app"])
        for inventory in (
            ["base"],
            ["extra", "base", "normal", "build", "dev", "last", "app"],
            ["normal", "base", "build", "dev", "last", "app"],
            ["base", "build", "normal", "dev", "last", "app"],
            ["base", "normal", "dev", "build", "last", "app"],
        ):
            with self.subTest(inventory=inventory):
                self.config.write_text(json.dumps(self.policy | {"packages": inventory}))
                child = self.invoke()
                self.assertNotEqual(child.returncode, 0)
                self.assertEqual(self.calls.read_text(), "metadata\n")
                self.assertEqual(self.output.read_text(), "existing=retained\n")

    def test_publication_switches_cannot_enter_publication_path(self) -> None:
        self.workspace(["compose-lens"])
        for changes in (
            {"validation-only": "false"},
            {"bootstrap": "true"},
            {"package": "compose-lens"},
        ):
            child = self.invoke(changes)
            self.assertNotEqual(child.returncode, 0)
            self.assertEqual(self.calls.read_text(), "")
            self.assertEqual(self.output.read_text(), "existing=retained\n")

    def test_workflow_receipt_guard_runs_before_setup_and_forwards_actual_proof(self) -> None:
        text = (ROOT / WORKFLOW).read_text()
        workflow = yaml.safe_load(text)
        call = workflow[True]["workflow_call"]
        job = workflow["jobs"]["prepare"]
        steps = job["steps"]
        guard = steps[0]
        self.assertEqual(guard["env"]["AUTOMATION_SHA"], "${{ inputs.automation_sha }}")
        self.assertEqual(guard["env"]["WORKFLOW_SHA"], "${{ job.workflow_sha }}")
        self.assertEqual(steps[1]["with"]["ref"], "${{ job.workflow_sha }}")
        self.assertNotIn("uses", guard)
        self.assertEqual(job["runs-on"], "ubuntu-24.04")
        self.assertEqual(workflow["permissions"], {"contents": "read"})
        self.assertEqual(job["timeout-minutes"], 10)
        self.assertNotIn("secrets", call)
        self.assertNotIn("environment", job)
        self.assertNotIn("if", job)
        self.assertFalse(any("if" in step or "continue-on-error" in step for step in steps))
        for key in ("candidate_sha", "published_api_candidate_sha"):
            self.assertEqual(
                call["outputs"][key]["value"], "${{ jobs.prepare.outputs." + key + " }}"
            )
            self.assertEqual(job["outputs"][key], "${{ steps.inventory.outputs." + key + " }}")
        final = steps[-1]
        self.assertIn("release prepare-inventory", final["run"])
        self.assertIn(
            '--published-api-candidate-sha "${PUBLISHED_API_CANDIDATE_SHA}"', final["run"]
        )
        self.assertEqual(
            final["env"]["PUBLISHED_API_CANDIDATE_SHA"], "${{ inputs.published_api_candidate_sha }}"
        )
        self.assertEqual(final["env"]["VALIDATION_SHA"], "${{ inputs.validation_sha }}")
        self.assertEqual(final["env"]["VALIDATION_RESULT"], "${{ inputs.validation_result }}")
        self.assertTrue(distribution.shared_path(WORKFLOW))
        self.assertTrue(distribution.shared_path("automation/src/boxferry_ci/release.py"))
        for changes in (
            {},
            {"AUTOMATION_SHA": ""},
            {"WORKFLOW_SHA": ""},
            {"AUTOMATION_SHA": SHA},
            {"WORKFLOW_SHA": SHA},
            {"AUTOMATION_SHA": "short", "WORKFLOW_SHA": "short"},
            {"VALIDATION_RESULT": "failure"},
            {"VALIDATION_SHA": ""},
            {"PUBLISHED_API_CANDIDATE_SHA": ""},
            {"PUBLISHED_API_CANDIDATE_SHA": AUTOMATION},
        ):
            env = dict(
                self.env,
                AUTOMATION_SHA=AUTOMATION,
                WORKFLOW_SHA=AUTOMATION,
                CANDIDATE_SHA=SHA,
                VALIDATION_SHA=SHA,
                VALIDATION_RESULT="success",
                PUBLISHED_API_CANDIDATE_SHA=SHA,
            )
            env.update(changes)
            # Appending an inert setup marker proves shell failure stops at the guard;
            # the parsed first-step/checkout order above binds this to the real workflow.
            marker = self.root / "setup-reached"
            marker.unlink(missing_ok=True)
            script = guard["run"] + '\nprintf reached > "${SETUP_MARKER}"\n'
            env["SETUP_MARKER"] = str(marker)
            child = subprocess.run(
                ["bash", "-c", script], env=env, capture_output=True, timeout=5, check=False
            )
            self.assertEqual(child.returncode == 0, not changes)
            self.assertEqual(marker.exists(), not changes)
        for forbidden in (
            "publish-crate",
            "prepare-crate",
            "secrets.",
            "id-token:",
            "CARGO_REGISTRY_TOKEN",
        ):
            self.assertNotIn(forbidden, text)
        self.assertEqual(
            steps[1]["uses"], "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"
        )
        self.assertIn("# v7.0.1", text)
        self.assertEqual(steps[2]["uses"], "./_automation/.github/actions/shared-source")
        self.assertEqual(steps[3]["uses"], "./_automation/.github/actions/shared-python")
        self.assertEqual(steps[4]["working-directory"], "source")
