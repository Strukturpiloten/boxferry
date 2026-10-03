#!/usr/bin/env python3
"""Adversarial admission tests for reviewed PR application catalogues."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest


ROOT = pathlib.Path(__file__).resolve().parent.parent
HELPER = ROOT / "scripts/reviewed-application-catalogue.py"
SPEC = importlib.util.spec_from_file_location("reviewed_application_catalogue", HELPER)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("could not load reviewed application catalogue admission")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ReviewedApplicationCatalogueTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        root = pathlib.Path(self.temporary.name)
        self.trusted = root / "trusted"
        self.candidate = root / "candidate"
        self.verifier = root / "verifier"
        for checkout in (self.trusted, self.candidate):
            for label in (MODULE.CATALOGUE, MODULE.RUNNER, MODULE.SCHEMA):
                destination = checkout / label
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / label, destination)

    def candidate_source(self) -> str:
        return (self.candidate / MODULE.CATALOGUE).read_text(encoding="utf-8")

    def replace_task_field(self, task_id: str, field: str, value: str | list[str]) -> None:
        source = self.candidate_source()
        marker = f'[[tasks]]\nid = "{task_id}"\n'
        self.assertEqual(source.count(marker), 1)
        before, block = source.split(marker, 1)
        task, separator, after = block.partition("\n[[tasks]]")
        matches = [line for line in task.splitlines() if line.startswith(f"{field} = ")]
        self.assertEqual(len(matches), 1)
        task = task.replace(matches[0], f"{field} = {json.dumps(value)}", 1)
        (self.candidate / MODULE.CATALOGUE).write_text(
            before + marker + task + separator + after, encoding="utf-8"
        )

    def admit(self, task: str = "nextcloud-application") -> None:
        MODULE.admit(self.trusted, self.candidate, self.verifier, task)

    def test_metadata_and_full_sha_lens_revisions_use_candidate_evidence_contract(self) -> None:
        tasks = tomllib.loads(self.candidate_source())["tasks"]
        selected = next(task for task in tasks if task["id"] == "nextcloud-application")
        self.replace_task_field(
            selected["id"], "sources", ["reviewed-native-podman", *selected["sources"][1:]]
        )
        self.replace_task_field(
            selected["id"], "targets", ["reviewed-compose-specification", *selected["targets"][1:]]
        )
        self.replace_task_field("compose-lens-candidate", "revision", "a" * 40)
        self.replace_task_field("quadlet-lens-candidate", "revision", "b" * 40)
        self.admit()
        for label in (MODULE.RUNNER, MODULE.SCHEMA):
            self.assertEqual((self.verifier / label).read_bytes(), (self.trusted / label).read_bytes())
        candidate_bytes = (self.candidate / MODULE.CATALOGUE).read_bytes()
        self.assertEqual((self.verifier / MODULE.CATALOGUE).read_bytes(), candidate_bytes)
        plan = subprocess.run(
            [sys.executable, str(self.verifier / MODULE.RUNNER), "plan", "--tier",
             "pre-release", "--task", "nextcloud-application", "--format", "json"],
            check=True, text=True, capture_output=True,
        )
        self.assertEqual(json.loads(plan.stdout)["tasks"][0]["id"], "nextcloud-application")

        # The final independent validator must accept only candidate-bound evidence.
        test_spec = importlib.util.spec_from_file_location(
            "test_migration_readiness_fixture", ROOT / "scripts/test-migration-readiness.py"
        )
        assert test_spec is not None and test_spec.loader is not None
        fixture_module = importlib.util.module_from_spec(test_spec)
        test_spec.loader.exec_module(fixture_module)
        evidence, revision = fixture_module.MigrationReadinessTests().evidence_fixture(
            tier_id="pre-release", task_id="nextcloud-application"
        )
        admitted = MODULE.trusted_runner(self.verifier).load_catalogue(self.verifier / MODULE.CATALOGUE)
        candidate_task = next(task for task in admitted["tasks"] if task["id"] == "nextcloud-application")
        evidence["tasks"][0]["sources"] = candidate_task["sources"]
        evidence["tasks"][0]["targets"] = candidate_task["targets"]
        evidence["run"]["lens_revisions"] = {"compose-lens": "a" * 40, "quadlet-lens": "b" * 40}
        evidence["run"]["catalogue_sha256"] = hashlib.sha256(candidate_bytes).hexdigest()
        evidence_path = pathlib.Path(self.temporary.name) / "evidence.json"
        evidence_path.write_text(json.dumps(evidence), encoding="utf-8")
        validated = subprocess.run(
            [sys.executable, str(self.verifier / MODULE.RUNNER), "validate-evidence",
             "--evidence", str(evidence_path), "--tier", "pre-release", "--task",
             "nextcloud-application", "--revision", revision, "--require-success"],
            check=False, text=True, capture_output=True,
        )
        self.assertEqual(validated.returncode, 0, validated.stderr)
        evidence["run"]["catalogue_sha256"] = "0" * 64
        evidence_path.write_text(json.dumps(evidence), encoding="utf-8")
        rejected = subprocess.run(
            [sys.executable, str(self.verifier / MODULE.RUNNER), "validate-evidence",
             "--evidence", str(evidence_path), "--tier", "pre-release", "--task",
             "nextcloud-application", "--revision", revision, "--require-success"],
            check=False, text=True, capture_output=True,
        )
        self.assertNotEqual(rejected.returncode, 0)

    def test_execution_policy_and_shape_changes_are_rejected(self) -> None:
        tasks = tomllib.loads(self.candidate_source())["tasks"]
        lens = next(task for task in tasks if task["id"] == "compose-lens-candidate")
        forbidden = (
            ('privileged = true', 'privileged = false'),
            ('deadline-seconds = 3600', 'deadline-seconds = 7200'),
            ('maximum-disk-growth-mib = 10240', 'maximum-disk-growth-mib = 20480'),
            ('required-tools = ["podman"]', 'required-tools = ["bash"]'),
            ('required-environment = ["BOXFERRY_BIN", "BOXFERRY_COMPOSE_BIN"]',
             'required-environment = ["BOXFERRY_BIN"]'),
            ('command = ["bash", "scripts/podman-live-conformance.sh", "--profile", "application",',
             'command = ["bash", "scripts/unsafe.sh", "--profile", "application",'),
            ('max-concurrency = 4', 'max-concurrency = 3'),
            ('tier-deadline-seconds = 1200', 'tier-deadline-seconds = 2400'),
            ('approved-losses = "reviewed application assertions in scripts/lib/nextcloud-application.sh"',
             'approved-losses = "anything"'),
            ('runtime-claim = "WebDAV, database/cache use, HTTP publication, selection, persistence, and cleanup"',
             'runtime-claim = "unbounded live deployment"'),
            ('repository = "https://github.com/Strukturpiloten/compose-lens.git"',
             'repository = "https://example.invalid/malicious.git"'),
            ('commands = [["cargo", "ci-application"]]', 'commands = [["bash", "unsafe.sh"]]'),
            ('schema = 2', 'schema = 3'),
            ('schema = 2', 'schema = 2\nunexpected = "candidate"'),
            ('state = "not-executed"', 'state = "passed"'),
            ('id = "nextcloud-application"', 'id = "renamed-application"'),
            (f'revision = "{lens["revision"]}"', 'revision = "main"'),
        )
        original = self.candidate_source()
        for before, after in forbidden:
            with self.subTest(change=after):
                (self.candidate / MODULE.CATALOGUE).write_text(original.replace(before, after, 1), encoding="utf-8")
                self.assertNotEqual(original, self.candidate_source())
                with self.assertRaises(MODULE.AdmissionError):
                    self.admit()

    def test_reordered_task_array_is_not_metadata(self) -> None:
        source = self.candidate_source()
        head, first, second_and_rest = source.split("[[tasks]]", 2)
        second, rest = second_and_rest.split("[[tasks]]", 1)
        (self.candidate / MODULE.CATALOGUE).write_text(
            head + "[[tasks]]" + second + "[[tasks]]" + first + "[[tasks]]" + rest,
            encoding="utf-8",
        )
        with self.assertRaises(MODULE.AdmissionError):
            self.admit()

    def test_runner_schema_symlink_size_and_task_guards(self) -> None:
        with self.assertRaises(MODULE.AdmissionError):
            self.admit("supabase-application")
        for label in (MODULE.RUNNER, MODULE.SCHEMA):
            with self.subTest(label=label):
                path = self.candidate / label
                original = path.read_bytes()
                path.write_bytes(original + b"\n")
                with self.assertRaises(MODULE.AdmissionError):
                    self.admit()
                path.write_bytes(original)
        path = self.candidate / MODULE.CATALOGUE
        original = path.read_bytes()
        path.unlink()
        path.symlink_to(self.trusted / MODULE.CATALOGUE)
        with self.assertRaises(MODULE.AdmissionError):
            self.admit()
        path.unlink()
        path.write_bytes(original + b" " * MODULE.MAX_CONTRACT_BYTES)
        with self.assertRaises(MODULE.AdmissionError):
            self.admit()


if __name__ == "__main__":
    unittest.main()
