#!/usr/bin/env python3
"""Contract tests for the migration-readiness catalogue and evidence helper."""

from __future__ import annotations

import argparse
import copy
import ctypes
import importlib.util
import json
import math
import os
import pathlib
import re
import subprocess
import struct
import sys
import tempfile
import time
import tomllib
import types
import unittest
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parent.parent
RUNNER = ROOT / "scripts/migration-readiness.py"
SPEC = importlib.util.spec_from_file_location("migration_readiness", RUNNER)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("could not load migration-readiness helper")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class MigrationReadinessTests(unittest.TestCase):
    def test_native_pins_have_unique_renovate_extraction_and_manual_admission(self) -> None:
        renovate = json.loads((ROOT / ".github/renovate.json").read_text())
        catalogue = MODULE.load_catalogue()

        def extract(description: str, path: str) -> list[dict[str, str]]:
            owners = [
                item for item in renovate["customManagers"]
                if item.get("description") == description
            ]
            self.assertEqual(len(owners), 1)
            owner = owners[0]
            self.assertEqual(len(owner["matchStrings"]), 1)
            self.assertIsNotNone(re.search(owner["managerFilePatterns"][0][1:-1], path))
            matching_owners = [
                item for item in renovate["customManagers"]
                if any(re.search(pattern[1:-1], path) for pattern in item["managerFilePatterns"])
            ]
            self.assertEqual(matching_owners, [owner], "operational pins need one manager")
            # These RE2 patterns share Python syntax except for named groups.
            pattern = re.sub(r"\(\?<([A-Za-z]+)>", r"(?P<\1>", owner["matchStrings"][0])
            return [
                match.groupdict()
                for match in re.finditer(pattern, (ROOT / path).read_text())
            ]

        image_pins = extract(
            "Track reviewed live Podman matrix images",
            "fixtures/conformance/podman-live/matrix.tsv",
        )
        self.assertEqual(len(image_pins), len(MODULE.tabular_ids(MODULE.PODMAN_MATRIX)))
        self.assertEqual(len({pin["depName"] for pin in image_pins}), len(image_pins))
        for minor, patch in [("5.8", "5.8.7"), ("6.1", "6.1.2")]:
            for mode in ["rootful", "rootless"]:
                pin = next(
                    pin for pin in image_pins
                    if pin["depName"].endswith(f"/podman-{minor}-{mode}")
                )
                self.assertEqual(pin["currentValue"], f"v{patch}")
                self.assertRegex(pin["currentDigest"], r"^sha256:[0-9a-f]{64}$")

        lens_pins = extract(
            "Track release-bound native Lens conformance revisions",
            "fixtures/conformance/migration-readiness/tiers.toml",
        )
        self.assertEqual(len(lens_pins), 2)
        self.assertEqual(
            {pin["depName"] for pin in lens_pins},
            {"Strukturpiloten/compose-lens", "Strukturpiloten/quadlet-lens"},
        )
        lockfile = tomllib.loads((ROOT / "Cargo.lock").read_text())
        for lens in ("compose-lens", "quadlet-lens"):
            with self.subTest(lens=lens):
                pin = next(
                    item for item in lens_pins if item["depName"] == f"Strukturpiloten/{lens}"
                )
                task = MODULE.by_id(catalogue["tasks"], f"{lens}-candidate", "task")
                self.assertEqual(pin["currentDigest"], task["revision"])
                self.assertRegex(pin["currentValue"], r"^v[0-9]+\.[0-9]+\.[0-9]+$")
                adapter = lens.removesuffix("-lens")
                manifest = tomllib.loads(
                    (ROOT / f"crates/boxferry-{adapter}/Cargo.toml").read_text()
                )
                dependency = manifest["dependencies"][lens]
                self.assertEqual(set(dependency), {"version", "default-features"})
                self.assertFalse(dependency["default-features"])
                self.assertEqual(pin["currentValue"], f"v{dependency['version']}")
                packages = [item for item in lockfile["package"] if item["name"] == lens]
                self.assertEqual(len(packages), 1)
                package = packages[0]
                self.assertEqual(package["version"], dependency["version"])
                self.assertEqual(
                    package["source"], "registry+https://github.com/rust-lang/crates.io-index"
                )
                self.assertRegex(package["checksum"], r"^[0-9a-f]{64}$")
        rule = next(
            rule for rule in renovate["packageRules"]
            if rule.get("description") == "Require native revalidation of Lens conformance revisions"
        )
        self.assertEqual(
            rule["matchFileNames"], ["fixtures/conformance/migration-readiness/tiers.toml"]
        )
        self.assertEqual(rule["matchDatasources"], ["github-tags"])
        self.assertTrue(rule["pinDigests"])
        self.assertTrue(rule["dependencyDashboardApproval"])
        self.assertFalse(rule["automerge"])

    def assert_catalogue_rejected(self, source: str, pattern: str) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "tiers.toml"
            path.write_text(source, encoding="utf-8")
            with self.assertRaisesRegex(MODULE.ContractError, pattern):
                MODULE.load_catalogue(path)

    def evidence_fixture(
        self,
        *,
        tier_id: str = "offline",
        task_id: str | None = None,
        state: str = "passed",
    ) -> tuple[dict[str, object], str]:
        catalogue = MODULE.load_catalogue()
        tier, tasks = MODULE.selected_tasks(catalogue, tier_id, task_id)
        revision = "1" * 40
        lens_revisions = {
            "compose-lens": MODULE.by_id(
                catalogue["tasks"], "compose-lens-candidate", "task"
            )["revision"],
            "quadlet-lens": MODULE.by_id(
                catalogue["tasks"], "quadlet-lens-candidate", "task"
            )["revision"],
        }
        task_results = []
        for index, task in enumerate(tasks):
            second = index * 2
            started_at = f"2026-09-10T12:00:{second:02d}Z"
            finished_at = f"2026-09-10T12:00:{second + 1:02d}Z"
            roles = ["checkout", "temporary-directory"]
            paths = [str(ROOT), tempfile.gettempdir()]
            if "podman" in task["required-tools"]:
                roles.append("podman-graph-root")
                paths.append("/var/lib/containers/storage")
            observed = {
                "wall_seconds": 1.0,
                "concurrency": 1,
                "available_memory_mib": task["minimum-memory-mib"],
                "available_disk_mib": task["minimum-disk-mib"],
                "peak_memory_delta_mib": 1,
                "peak_rss_kib": 1024,
                "disk_growth_mib": 1,
                "exit_status": 0,
                "timed_out": False,
                "sample_interval_milliseconds": 250,
                "memory_scope": "system-available-memory",
                "rss_scope": "child-process-tree",
                "disk_scope": "deduplicated-filesystems",
                "filesystems": [
                    {
                        "device": "fixture-device",
                        "roles": roles,
                        "paths": paths,
                        "baseline_free_mib": task["minimum-disk-mib"],
                        "minimum_free_mib": task["minimum-disk-mib"] - 1,
                        "peak_growth_mib": 1,
                    }
                ],
                "missing_tools": [],
                "missing_environment": [],
                "effective_uid": 0 if task["privileged"] else 1000,
            }
            if task["kind"] == "lens-consumer":
                observed["lens_revision"] = lens_revisions[task["lens"]]
            reason = None if state == "passed" else "fixture failure"
            result = MODULE.task_evidence(
                task,
                started_at,
                time.monotonic(),
                state,
                observed,
                reason,
            )
            result["started_at"] = started_at
            result["finished_at"] = finished_at
            task_results.append(result)

        evidence = {
            "schema_version": 2,
            "outcome": "passed" if state == "passed" else "failed",
            "run": {
                "id": (
                    "8b2cb4cf-7ec3-4ba2-b87e-d4eca327ca52"
                    if task_id is not None else "0a20a918-93bd-43a2-b346-8f7dd63b08be"
                ),
                "tier": tier_id,
                "revision": revision,
                "started_at": "2026-09-10T12:00:00Z",
                "finished_at": "2026-09-10T12:01:00Z",
                "runner": "scripts/migration-readiness.py",
                "catalogue": "fixtures/conformance/migration-readiness/tiers.toml",
                "maximum_concurrency": tier["max-concurrency"],
                "actual_concurrency": 1,
                "tier_deadline_seconds": tier["tier-deadline-seconds"],
                "wall_seconds": 60.0,
                "total_worker_wall_seconds": 60.0,
                "timed_out": False,
                "fresh": True,
                "github_run_attempt": 1,
                "attempt_timings": [],
                "selection": {
                    "kind": "task" if task_id is not None else "tier",
                    "task": task_id,
                },
                "coordinator_id": "0a20a918-93bd-43a2-b346-8f7dd63b08be",
                "catalogue_sha256": MODULE.catalogue_digest(MODULE.CATALOGUE),
                "boxferry_binary_sha256": None,
                "worker_id": task_id or "serial",
                "evidence_kind": "worker" if task_id is not None else "serial",
                "lens_revisions": lens_revisions,
            },
            "manual_prerequisites": tier["manual-prerequisites"],
            "tasks": task_results,
            "gaps": catalogue["gaps"],
        }
        return evidence, revision

    def collector_fixture(
        self, root: pathlib.Path
    ) -> tuple[str, str, str, list[pathlib.Path], list[dict[str, object]]]:
        coordinator = "0a20a918-93bd-43a2-b346-8f7dd63b08be"
        binary_sha256 = "a" * 64
        expected = MODULE.selected_tasks(MODULE.load_catalogue(), "pre-release", None)[1]
        paths: list[pathlib.Path] = []
        documents: list[dict[str, object]] = []
        revision = "1" * 40
        for index, task in enumerate(expected):
            document, revision = self.evidence_fixture(
                tier_id="pre-release", task_id=task["id"]
            )
            second = (index // 4) * 2
            started_at = f"2026-09-10T12:00:{second:02d}Z"
            finished_at = f"2026-09-10T12:00:{second + 1:02d}Z"
            document["tasks"][0]["started_at"] = started_at
            document["tasks"][0]["finished_at"] = finished_at
            document["run"].update(
                id=f"8b2cb4cf-7ec3-4ba2-b87e-{index + 1:012x}",
                started_at=started_at,
                finished_at=finished_at,
                coordinator_id=coordinator,
                worker_id=task["id"],
                evidence_kind="worker",
                boxferry_binary_sha256=binary_sha256,
            )
            path = root / f"{task['id']}.json"
            paths.append(path)
            documents.append(document)
        return coordinator, binary_sha256, revision, paths, documents

    def collect_args(
        self,
        coordinator: str,
        binary_sha256: str,
        revision: str,
        paths: list[pathlib.Path],
        output: pathlib.Path,
    ) -> argparse.Namespace:
        arguments = [item for path in paths for item in ("--evidence", str(path))]
        return MODULE.parser().parse_args(
            [
                "collect-evidence",
                "--tier",
                "pre-release",
                "--revision",
                revision,
                "--coordinator-id",
                coordinator,
                "--boxferry-binary-sha256",
                binary_sha256,
                *arguments,
                "--evidence-output",
                str(output),
            ]
        )

    def test_tier_plans_are_exact_and_gaps_never_pass(self) -> None:
        expected = {
            "offline": ["offline-application-contracts"],
            "trusted-live": [
                "podman-api-5.4-rootless",
                "podman-api-6.1-rootful",
                "podman-api-6.1-rootless",
                "forgejo-root-modes",
            ],
            "pre-release": [
                "offline-application-contracts",
                "podman-complete-matrix-shard-1",
                "podman-complete-matrix-shard-2",
                "podman-complete-matrix-shard-3",
                "podman-complete-matrix-shard-4",
                "nextcloud-application",
                "forgejo-root-modes",
                "paperless-application",
                "immich-application",
                "observability-application",
                "supabase-application",
                "compose-lens-candidate",
                "quadlet-lens-candidate",
            ],
        }
        tier_deadlines = {
            "offline": 1200,
            "trusted-live": 2700,
            "pre-release": 1200,
        }
        for tier, tasks in expected.items():
            output = subprocess.run(
                [sys.executable, str(RUNNER), "plan", "--tier", tier, "--format", "json"],
                cwd=ROOT,
                check=True,
                text=True,
                stdout=subprocess.PIPE,
            )
            plan = json.loads(output.stdout)
            self.assertEqual([task["id"] for task in plan["tasks"]], tasks)
            self.assertEqual(plan["max_concurrency"], 4 if tier == "pre-release" else 1)
            self.assertEqual(plan["tier_deadline_seconds"], tier_deadlines[tier])
            self.assertEqual(len(plan["gaps"]), 4)
            self.assertFalse(any(gap["state"] == "passed" for gap in plan["gaps"]))

    def test_supabase_task_retains_reviewed_bounds_and_runner_command(self) -> None:
        catalogue = MODULE.load_catalogue()
        task = MODULE.by_id(catalogue["tasks"], "supabase-application", "task")
        tier = MODULE.by_id(catalogue["tiers"], "pre-release", "tier")

        self.assertTrue(task["privileged"])
        self.assertIn("at least 4 CPUs", tier["manual-prerequisites"])
        self.assertEqual(
            task["sources"],
            [
                "native-podman",
                "docker-compose-5.5.0",
                "podman-api-6.1.2-rootless",
            ],
        )
        self.assertEqual(task["deadline-seconds"], 5400)
        self.assertEqual(task["minimum-memory-mib"], 12288)
        self.assertEqual(task["minimum-disk-mib"], 24576)
        self.assertEqual(task["maximum-rss-mib"], 14336)
        self.assertEqual(task["maximum-disk-growth-mib"], 20480)
        self.assertEqual(task["required-tools"], ["podman", "skopeo"])
        self.assertEqual(
            task["required-environment"], ["BOXFERRY_BIN", "BOXFERRY_COMPOSE_BIN"]
        )
        self.assertEqual(
            task["command"],
            [
                "bash",
                "scripts/podman-live-conformance.sh",
                "--profile",
                "supabase-application",
                "--matrix-cell",
                "podman-6.1-rootless",
                "--engine",
                "podman",
            ],
        )

    def test_supabase_preflight_reports_missing_skopeo(self) -> None:
        catalogue = MODULE.load_catalogue()
        task = MODULE.by_id(catalogue["tasks"], "supabase-application", "task")
        sampler = mock.Mock()
        sampler.snapshot.return_value = {
            "available_memory_mib": task["minimum-memory-mib"],
            "filesystems": [
                {"baseline_free_mib": task["minimum-disk-mib"]},
            ],
        }
        identity_evidence = {
            "uid": 0,
            "gid": 0,
            "user": "root",
            "home": "/root",
            "groups": [0],
        }

        with (
            mock.patch.object(
                MODULE.shutil,
                "which",
                side_effect=lambda tool: None if tool == "skopeo" else f"/usr/bin/{tool}",
            ),
            mock.patch.object(
                MODULE,
                "execution_identity",
                return_value=(identity_evidence, None),
            ),
        ):
            observed, reason, identity = MODULE.preflight(task, sampler, None)

        self.assertEqual(reason, "missing required tools: skopeo")
        self.assertEqual(identity, identity_evidence)
        self.assertEqual(observed, sampler.snapshot.return_value)

    def test_complete_podman_matrix_retains_disk_growth_cap(self) -> None:
        catalogue = MODULE.load_catalogue()
        task = MODULE.by_id(catalogue["tasks"], "podman-complete-matrix-shard-1", "task")
        self.assertEqual(task["maximum-disk-growth-mib"], 10240)

    def test_matrix_shards_cover_all_rows_and_limitations_once(self) -> None:
        shards = [
            MODULE.expected_matrix_evidence(f"podman-complete-matrix-shard-{index}")
            for index in range(1, 5)
        ]
        self.assertTrue(all(shard is not None for shard in shards))
        self.assertEqual([len(shard["row_ids"]) for shard in shards], [12, 12, 12, 12])
        row_ids = [identifier for shard in shards for identifier in shard["row_ids"]]
        limitation_ids = [
            identifier for shard in shards for identifier in shard["limitation_row_ids"]
        ]
        self.assertEqual(len(row_ids), len(set(row_ids)))
        self.assertEqual(set(row_ids), set(MODULE.tabular_ids(MODULE.PODMAN_MATRIX)))
        self.assertEqual(len(limitation_ids), len(set(limitation_ids)))
        self.assertEqual(
            set(limitation_ids), set(MODULE.tabular_ids(MODULE.PODMAN_LIMITATIONS))
        )
        live_runner = (ROOT / "scripts/podman-live-conformance.sh").read_text(encoding="utf-8")
        for required in (
            '--matrix-shard <N/4>',
            'if ((cells != 12)); then',
            'if ((limited_cells != expected_limited_cells)); then',
        ):
            self.assertIn(required, live_runner)

    def test_catalogue_rejects_a_successful_gap(self) -> None:
        source = MODULE.CATALOGUE.read_text(encoding="utf-8")
        changed = source.replace('state = "not-executed"', 'state = "passed"', 1)
        self.assert_catalogue_rejected(changed, "successful state")

    def test_catalogue_shape_rejects_unknown_missing_and_mistyped_fields(self) -> None:
        source = MODULE.CATALOGUE.read_text(encoding="utf-8")

        def replace_line(prefix: str, replacement: str) -> str:
            line = next(item for item in source.splitlines() if item.startswith(prefix))
            return source.replace(line, replacement, 1)

        mutations = {
            "unknown top-level": source.replace(
                'evidence-schema = "docs/schemas/migration-readiness-evidence-v2.schema.json"',
                'evidence-schema = "docs/schemas/migration-readiness-evidence-v2.schema.json"\nunknown-top = true',
                1,
            ),
            "missing top-level": source.replace("schema = 2\n", "", 1),
            "mistyped top-level": source.replace("schema = 2", 'schema = "2"', 1),
            "unknown gap": source.replace(
                'id = "gpu"',
                'id = "gpu"\nunknown-gap = true',
                1,
            ),
            "mistyped gap": source.replace('state = "not-executed"', "state = 1", 1),
            "missing tier deadline": source.replace(
                "tier-deadline-seconds = 1200\n", "", 1
            ),
            "mistyped tier concurrency": source.replace(
                "max-concurrency = 1", "max-concurrency = true", 1
            ),
            "mistyped tier tasks": source.replace(
                'tasks = ["offline-application-contracts"]',
                'tasks = "offline-application-contracts"',
                1,
            ),
            "unknown task": source.replace(
                'id = "offline-application-contracts"',
                'id = "offline-application-contracts"\nunknown-task = true',
                1,
            ),
            "missing task kind": source.replace('kind = "command"\n', "", 1),
            "mistyped task privilege": source.replace(
                "privileged = false", 'privileged = "false"', 1
            ),
            "mistyped command": replace_line("command = [\"bash\"", 'command = "bash"'),
            "mistyped Lens commands": source.replace(
                'commands = [["cargo", "ci-application"]]',
                'commands = ["cargo"]',
                1,
            ),
            "missing Lens revision": replace_line('revision = "', ""),
        }
        for label, changed in mutations.items():
            with self.subTest(label=label):
                self.assertNotEqual(changed, source, "mutation must change the catalogue")
                self.assert_catalogue_rejected(changed, "catalogue")

    def test_schema_valid_evidence_binds_catalogue_and_revision(self) -> None:
        evidence, revision = self.evidence_fixture()
        MODULE.validate_evidence(evidence, "offline", revision)

        changed = copy.deepcopy(evidence)
        changed["run"]["revision"] = "2" * 40
        with self.assertRaisesRegex(MODULE.ContractError, "revision"):
            MODULE.validate_evidence(changed, "offline", revision)

        changed = copy.deepcopy(evidence)
        changed["tasks"][0]["approved_losses"] = "unreviewed"
        with self.assertRaisesRegex(MODULE.ContractError, "approved_losses"):
            MODULE.validate_evidence(changed, "offline", revision)

        changed = copy.deepcopy(evidence)
        changed["run"]["lens_revisions"]["compose-lens"] = "2" * 40
        with self.assertRaisesRegex(MODULE.ContractError, "catalogue pins"):
            MODULE.validate_evidence(changed, "offline", revision)

        changed = copy.deepcopy(evidence)
        changed["tasks"][0]["observed"]["effective_uid"] = 0
        with self.assertRaisesRegex(MODULE.ContractError, "execution identity"):
            MODULE.validate_evidence(changed, "offline", revision)

    def test_failure_evidence_remains_valid_but_cannot_claim_success(self) -> None:
        evidence, revision = self.evidence_fixture(state="unavailable")
        MODULE.validate_evidence(evidence, "offline", revision)

        changed = copy.deepcopy(evidence)
        changed["outcome"] = "passed"
        with self.assertRaisesRegex(MODULE.ContractError, "outcome"):
            MODULE.validate_evidence(changed, "offline", revision)

        changed = copy.deepcopy(evidence)
        changed["tasks"][0]["reason"] = None
        with self.assertRaisesRegex(MODULE.ContractError, "lacks a reason"):
            MODULE.validate_evidence(changed, "offline", revision)

    def test_checked_in_schema_rejects_shape_format_and_type_mutations(self) -> None:
        evidence, revision = self.evidence_fixture()
        mutations = {
            "missing required": lambda item: item["run"].pop("started_at"),
            "unexpected property": lambda item: item.update({"unexpected": True}),
            "invalid enum": lambda item: item.update({"outcome": "unknown"}),
            "invalid uuid": lambda item: item["run"].update({"id": "not-a-uuid"}),
            "invalid date-time": lambda item: item["run"].update(
                {"started_at": "2026-09-10 12:00:00"}
            ),
            "invalid sha": lambda item: item["run"].update({"revision": "A" * 40}),
            "boolean integer": lambda item: item["tasks"][0]["observed"].update(
                {"peak_rss_kib": True}
            ),
            "negative number": lambda item: item["tasks"][0]["observed"].update(
                {"wall_seconds": -1}
            ),
            "non-finite number": lambda item: item["tasks"][0]["observed"].update(
                {"wall_seconds": math.inf}
            ),
            "invalid const": lambda item: item["tasks"][0]["observed"].update(
                {"sample_interval_milliseconds": 100}
            ),
            "duplicate array": lambda item: item["tasks"][0].update(
                {"sources": item["tasks"][0]["sources"] * 2}
            ),
        }
        for label, mutate in mutations.items():
            with self.subTest(label=label):
                changed = copy.deepcopy(evidence)
                mutate(changed)
                with self.assertRaises(MODULE.ContractError):
                    MODULE.validate_evidence(changed, "offline", revision)

    def test_schema_definition_rejects_unaudited_keywords(self) -> None:
        schema = MODULE.load_evidence_schema(MODULE.load_catalogue())
        schema["description"] = "not implemented by the local evaluator"
        with self.assertRaisesRegex(MODULE.ContractError, "unsupported schema keyword"):
            MODULE.validate_schema_definition(schema)

    def test_success_measurements_and_chronology_are_fail_closed(self) -> None:
        evidence, revision = self.evidence_fixture()

        def observed(name: str, value: object):
            return lambda item: item["tasks"][0]["observed"].update({name: value})

        mutations = {
            "missing wall": lambda item: item["tasks"][0]["observed"].pop(
                "wall_seconds"
            ),
            "missing rss": lambda item: item["tasks"][0]["observed"].pop(
                "peak_rss_kib"
            ),
            "missing memory peak": lambda item: item["tasks"][0]["observed"].pop(
                "peak_memory_delta_mib"
            ),
            "missing disk peak": lambda item: item["tasks"][0]["observed"].pop(
                "disk_growth_mib"
            ),
            "wall over budget": observed(
                "wall_seconds", evidence["tasks"][0]["budgets"]["deadline_seconds"] + 1
            ),
            "rss over budget": observed(
                "peak_rss_kib",
                evidence["tasks"][0]["budgets"]["maximum_rss_mib"] * 1024 + 1,
            ),
            "memory over budget": observed(
                "peak_memory_delta_mib",
                evidence["tasks"][0]["budgets"]["maximum_rss_mib"] + 1,
            ),
            "memory below preflight minimum": observed("available_memory_mib", 1),
            "disk over budget": observed(
                "disk_growth_mib",
                evidence["tasks"][0]["budgets"]["maximum_disk_growth_mib"] + 1,
            ),
            "nonzero exit": observed("exit_status", 1),
            "timed out": observed("timed_out", True),
            "concurrency over budget": observed("concurrency", 2),
            "low filesystem": lambda item: item["tasks"][0]["observed"][
                "filesystems"
            ][0].update({"baseline_free_mib": 1}),
            "inconsistent filesystem peak": lambda item: item["tasks"][0][
                "observed"
            ]["filesystems"][0].update({"peak_growth_mib": 3}),
            "reversed task": lambda item: item["tasks"][0].update(
                {
                    "started_at": "2026-09-10T12:00:02Z",
                    "finished_at": "2026-09-10T12:00:01Z",
                }
            ),
            "reversed run": lambda item: item["run"].update(
                {"finished_at": "2026-09-10T11:59:59Z"}
            ),
        }
        for label, mutate in mutations.items():
            with self.subTest(label=label):
                changed = copy.deepcopy(evidence)
                mutate(changed)
                with self.assertRaises(MODULE.ContractError):
                    MODULE.validate_evidence(changed, "offline", revision)

    def test_tier_deadline_is_bound_to_catalogue_and_fail_closed(self) -> None:
        evidence, revision = self.evidence_fixture()

        changed = copy.deepcopy(evidence)
        changed["run"]["tier_deadline_seconds"] += 1
        with self.assertRaisesRegex(MODULE.ContractError, "tier_deadline_seconds"):
            MODULE.validate_evidence(changed, "offline", revision)

        changed = copy.deepcopy(evidence)
        changed["run"]["wall_seconds"] = changed["run"]["tier_deadline_seconds"] + 1
        changed["run"]["total_worker_wall_seconds"] = changed["run"]["wall_seconds"]
        changed["outcome"] = "failed"
        with self.assertRaisesRegex(MODULE.ContractError, "without timing out"):
            MODULE.validate_evidence(changed, "offline", revision)

        changed["run"]["timed_out"] = True
        MODULE.validate_evidence(changed, "offline", revision)

    def test_partial_selection_is_explicit_and_requires_matching_task(self) -> None:
        task_id = "offline-application-contracts"
        evidence, revision = self.evidence_fixture(
            tier_id="pre-release", task_id=task_id
        )
        MODULE.validate_evidence(evidence, "pre-release", revision, task_id=task_id)
        with self.assertRaisesRegex(MODULE.ContractError, "explicit matching --task"):
            MODULE.validate_evidence(evidence, "pre-release", revision)
        with self.assertRaisesRegex(MODULE.ContractError, "does not match"):
            MODULE.validate_evidence(
                evidence,
                "pre-release",
                revision,
                task_id="forgejo-root-modes",
            )

    def test_reviewed_pr_application_tasks_bind_focused_success_to_exact_head(self) -> None:
        for task_id in (
            "nextcloud-application",
            "paperless-application",
            "immich-application",
        ):
            with self.subTest(task=task_id):
                evidence, revision = self.evidence_fixture(
                    tier_id="pre-release", task_id=task_id
                )
                MODULE.validate_evidence(evidence, "pre-release", revision, task_id=task_id)
                with self.assertRaisesRegex(MODULE.ContractError, "revision"):
                    MODULE.validate_evidence(
                        evidence, "pre-release", "2" * 40, task_id=task_id
                    )
                with self.assertRaisesRegex(MODULE.ContractError, "does not match"):
                    MODULE.validate_evidence(
                        evidence, "pre-release", revision,
                        task_id="forgejo-root-modes",
                    )
                failed, _ = self.evidence_fixture(
                    tier_id="pre-release", task_id=task_id, state="failed"
                )
                MODULE.validate_evidence(failed, "pre-release", revision, task_id=task_id)
                with tempfile.TemporaryDirectory() as directory:
                    path = pathlib.Path(directory) / "focused.json"
                    path.write_text(json.dumps(failed), encoding="utf-8")
                    args = MODULE.parser().parse_args([
                        "validate-evidence", "--evidence", str(path),
                        "--tier", "pre-release", "--task", task_id,
                        "--revision", revision, "--require-success",
                    ])
                    with self.assertRaisesRegex(MODULE.ContractError, "does not prove"):
                        MODULE.validate(args)

    def test_sampler_retains_peaks_and_deduplicates_filesystems(self) -> None:
        mebibyte = 1024 * 1024
        memory_values = iter([8000, 7800, 7400])
        disk_values = iter([10 * mebibyte, 9 * mebibyte, 8 * mebibyte])

        def memory_reader() -> int:
            return next(memory_values)

        def disk_reader(_path: pathlib.Path) -> types.SimpleNamespace:
            return types.SimpleNamespace(free=next(disk_values))

        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary)
            sampler = MODULE.ResourceSampler(
                [("checkout", path), ("podman-graph-root", path / "future")],
                memory_reader=memory_reader,
                disk_usage_reader=disk_reader,
            )
            sampler.sample()
            sampler.sample()
            snapshot = sampler.snapshot(sample=False)

        self.assertEqual(snapshot["peak_memory_delta_mib"], 600)
        self.assertEqual(snapshot["disk_growth_mib"], 2)
        self.assertEqual(len(snapshot["filesystems"]), 1)
        self.assertEqual(
            snapshot["filesystems"][0]["roles"],
            ["checkout", "podman-graph-root"],
        )

    @staticmethod
    def authored_view(path: pathlib.Path, raw_device: int, pool: str | None = None) -> dict[str, object]:
        fsid = "12345678-1234-5678-9abc-123456789abc"
        return {
            "measurement_path": str(path),
            "view": {
                "device": str(raw_device) if pool is None else f"btrfs:{fsid}:{pool}",
                "st_dev": str(raw_device), "statfs_fsid": str(raw_device * 99),
                "mount_id": raw_device, "mount_device": f"0:{raw_device}",
                "mount_root": f"/subvolume-{raw_device}", "mount_point": str(path),
                "mount_options": "rw", "optional_fields": [],
                "filesystem_type": "ext4" if pool is None else "btrfs",
                "mount_source": "fixture", "super_options": "rw",
                "btrfs": None if pool is None else {
                    "mounted_fsid": fsid, "num_devices": 1, "max_id": 100,
                    "backing_devices": [{"device": pool, "sysfs_path": f"/sys/devices/fixture-{pool}"}],
                },
            },
        }

    def test_btrfs_subvolumes_share_one_counter_but_keep_raw_provenance(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            paths = [pathlib.Path(directory) / name for name in ["checkout", "graph"]]
            views = {path: self.authored_view(path, raw, "259:3") for path, raw in zip(paths, [39, 54], strict=True)}
            free = 20 * 1024 * 1024
            counter_paths = []
            def reader(path: pathlib.Path, counter: bool) -> tuple[dict[str, object], int | None]:
                if counter:
                    counter_paths.append(path)
                return copy.deepcopy(views[path]), free if counter else None
            sampler = MODULE.ResourceSampler(
                list(zip(["checkout", "podman-graph-root"], paths, strict=True)),
                memory_reader=lambda: 10000, filesystem_reader=reader,
            )
            free -= 2 * 1024 * 1024
            sampler.sample()
            observed = sampler.snapshot(sample=False)
        self.assertIsNone(observed["disk_measurement_error"])
        self.assertEqual(observed["disk_growth_mib"], 2)
        self.assertEqual(counter_paths, [paths[0], paths[0]])
        self.assertEqual(len(observed["filesystems"]), 1)
        sources = observed["filesystems"][0]["path_observations"]
        self.assertEqual({source["view"]["st_dev"] for source in sources}, {"39", "54"})
        self.assertEqual({source["path"] for source in sources}, {str(path) for path in paths})
        self.assertEqual({source["role"] for source in sources}, {"checkout", "podman-graph-root"})

    def test_equal_counters_on_distinct_and_cloned_filesystems_are_summed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            paths = [pathlib.Path(directory) / name for name in ["one", "two"]]
            for pools in [(None, None), ("259:3", "260:3")]:
                with self.subTest(pools=pools):
                    views = {path: self.authored_view(path, raw, pool) for path, raw, pool in zip(paths, [39, 54], pools, strict=True)}
                    free = 20 * 1024 * 1024
                    def reader(path: pathlib.Path, counter: bool) -> tuple[dict[str, object], int | None]:
                        return copy.deepcopy(views[path]), free if counter else None
                    sampler = MODULE.ResourceSampler(
                        [("checkout", paths[0]), ("temporary-directory", paths[1])],
                        memory_reader=lambda: 10000, filesystem_reader=reader,
                    )
                    free -= 2 * 1024 * 1024
                    observed = sampler.snapshot()
                    self.assertEqual(len(observed["filesystems"]), 2)
                    self.assertEqual(observed["disk_growth_mib"], 4)
                    self.assertIsNone(observed["disk_measurement_error"])

    def test_changed_subvolume_mount_or_pool_identity_is_sticky_failure(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            paths = [pathlib.Path(directory) / name for name in ["one", "two"]]
            for field, changed in [("mount_id", 999), ("st_dev", "999"), ("mount_options", "ro"), ("device", "changed-pool")]:
                with self.subTest(field=field):
                    views = {path: self.authored_view(path, raw, "259:3") for path, raw in zip(paths, [39, 54], strict=True)}
                    def reader(path: pathlib.Path, counter: bool) -> tuple[dict[str, object], int | None]:
                        return copy.deepcopy(views[path]), 20 * 1024 * 1024 if counter else None
                    sampler = MODULE.ResourceSampler(
                        [("checkout", paths[0]), ("temporary-directory", paths[1])],
                        memory_reader=lambda: 10000, filesystem_reader=reader,
                    )
                    original = views[paths[1]]["view"][field]
                    views[paths[1]]["view"][field] = changed
                    sampler.sample()
                    self.assertIn("identity changed", sampler.disk_measurement_error)
                    views[paths[1]]["view"][field] = original
                    observed = sampler.snapshot()
                    self.assertIsNone(observed["available_disk_mib"])
                    self.assertIsNotNone(observed["disk_measurement_error"])

    def test_btrfs_fs_info_uses_readonly_fixed_abi_and_sparse_device_ids(self) -> None:
        fsid = bytes.fromhex("12345678123456789abc123456789abc")
        def ioctl(fd: int, request: int, buffer: bytearray, mutate: bool) -> int:
            self.assertEqual((fd, request, len(buffer), mutate), (42, 0x8400941f, 1024, True))
            self.assertEqual(buffer, bytes(1024))
            struct.pack_into("=QQ16s", buffer, 0, 100, 2, fsid)
            return 0
        with mock.patch.object(MODULE.fcntl, "ioctl", side_effect=ioctl), mock.patch.object(MODULE.platform, "machine", return_value="x86_64"):
            self.assertEqual(MODULE.btrfs_info(42), {
                "max_id": 100, "num_devices": 2,
                "mounted_fsid": "12345678-1234-5678-9abc-123456789abc",
            })
        for maximum, count, raw in [(0, 0, fsid), (1, 2, fsid), (200, 129, fsid), (1, 1, bytes(16))]:
            with self.subTest(count=count, maximum=maximum):
                def malformed(_fd: int, _request: int, buffer: bytearray, _mutate: bool) -> int:
                    struct.pack_into("=QQ16s", buffer, 0, maximum, count, raw)
                    return 0
                with mock.patch.object(MODULE.fcntl, "ioctl", side_effect=malformed), self.assertRaises(MODULE.ContractError):
                    MODULE.btrfs_info(42)
        with mock.patch.object(MODULE.fcntl, "ioctl", side_effect=OSError("authored ioctl failure")), self.assertRaises(OSError):
            MODULE.btrfs_info(42)
        with mock.patch.object(MODULE.platform, "machine", return_value="unsupported"), self.assertRaises(MODULE.ContractError):
            MODULE.btrfs_info(42)

    def test_btrfs_sysfs_membership_is_complete_canonical_and_bounded(self) -> None:
        fsid = "12345678-1234-5678-9abc-123456789abc"
        for mutation in ["none", "missing", "dangling", "nonlink", "duplicate", "alias", "invalid-number", "cardinality", "escape"]:
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                target = root / "devices/physical"
                target.mkdir(parents=True)
                (target / "dev").write_text("259:3\n")
                members = root / "fs/btrfs" / fsid / "devices"
                members.mkdir(parents=True)
                member = members / "physical"
                member.symlink_to(target, target_is_directory=True)
                aliases = root / "dev/block"
                aliases.mkdir(parents=True)
                alias = aliases / "259:3"
                alias.symlink_to(target, target_is_directory=True)
                count = 1
                if mutation == "missing":
                    member.unlink()
                elif mutation in {"dangling", "nonlink", "escape"}:
                    member.unlink()
                    if mutation == "nonlink":
                        member.mkdir()
                    else:
                        member.symlink_to(root / ("nonexistent" if mutation == "dangling" else "dev"))
                elif mutation == "duplicate":
                    (members / "other-alias").symlink_to(target, target_is_directory=True)
                    count = 2
                elif mutation == "alias":
                    alias.unlink()
                    alias.symlink_to(members, target_is_directory=True)
                elif mutation == "invalid-number":
                    (target / "dev").write_text("0:0\n")
                elif mutation == "cardinality":
                    count = 2
                with mock.patch.object(MODULE, "SYSFS", root):
                    if mutation == "none":
                        self.assertEqual(MODULE.btrfs_devices({"mounted_fsid": fsid, "num_devices": count}), [{"device": "259:3", "sysfs_path": str(target)}])
                    else:
                        with self.assertRaises((MODULE.ContractError, OSError)):
                            MODULE.btrfs_devices({"mounted_fsid": fsid, "num_devices": count})

    def test_mount_detection_uses_exact_fd_mount_and_component_boundaries(self) -> None:
        mounts = "1 0 8:1 / / rw - ext4 /dev/root rw\n2 1 0:39 /subvol /data rw shared:7 - btrfs /dev/disk rw\n3 1 0:54 /other /data rw future:1 - btrfs /dev/clone rw\n"
        def metadata(path: pathlib.Path, _maximum: int) -> str:
            return mounts if path == MODULE.MOUNTINFO else "pos:\t0\nmnt_id:\t3\n"
        with mock.patch.object(MODULE, "bounded_text", side_effect=metadata):
            self.assertEqual(MODULE.mounted_view(42, pathlib.Path("/data/application"))["mount_id"], 3)
            with self.assertRaises(MODULE.ContractError):
                MODULE.mounted_view(42, pathlib.Path("/database"))
        for invalid in ["", mounts + mounts, "1 0 bad / / rw - btrfs /dev/disk rw\n", "1 0 8:1 / /bad\\999 rw - ext4 disk rw\n", "1 0 8:1 / /x/../y rw - ext4 disk rw\n"]:
            with self.subTest(invalid=invalid), mock.patch.object(MODULE, "bounded_text", return_value=invalid), self.assertRaises(MODULE.ContractError):
                MODULE.read_mounts()
        self.assertEqual(MODULE.mount_path("/with\\040space\\134040"), "/with space\\040")

    def test_unrelated_namespace_mount_churn_does_not_change_opened_view(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory)
            selected = f"3 1 0:39 /subvol {path} rw - btrfs /dev/disk rw\n"
            inventories = iter([
                "1 0 8:1 / / rw - ext4 /dev/root rw\n" + selected
                + "9 1 0:4 net:[4026533210] /run/netns/fixture rw - nsfs nsfs rw\n",
                "1 0 8:1 / / rw - ext4 /dev/root rw\n" + selected
                + "10 1 0:4 mnt:[4026533211] /run/namespaces/other rw - nsfs nsfs rw\n",
                "1 0 8:1 / / rw - ext4 /dev/root rw\n" + selected,
                "1 0 8:1 / / rw - ext4 /dev/root rw\n" + selected,
            ])
            def metadata(metadata_path: pathlib.Path, _maximum: int) -> str:
                return next(inventories) if metadata_path == MODULE.MOUNTINFO else "mnt_id:\t3\n"
            info = {"mounted_fsid": "12345678-1234-5678-9abc-123456789abc", "num_devices": 1, "max_id": 100}
            with (
                mock.patch.object(MODULE, "bounded_text", side_effect=metadata),
                mock.patch.object(MODULE, "btrfs_info", return_value=info),
                mock.patch.object(MODULE, "btrfs_devices", return_value=[{"device": "259:3", "sysfs_path": "/sys/devices/fixture"}]),
                mock.patch.object(MODULE.os, "fstat", return_value=types.SimpleNamespace(st_dev=39)),
                mock.patch.object(MODULE.os, "fstatvfs", return_value=types.SimpleNamespace(f_bavail=512, f_frsize=4096, f_fsid=12345)),
            ):
                observed, free = MODULE.filesystem_view(path, True)
            self.assertEqual(observed["view"]["mount_id"], 3)
            self.assertEqual(free, 512 * 4096)
        for filesystem_type, root, mountpoint, field in [
            ("btrfs", "net:[4026533210]", "/run/netns/fixture", "root"),
            ("nsfs", "net:[0]", "/run/netns/fixture", "root"),
            ("nsfs", "net:[4026533210]", "relative", "mountpoint"),
        ]:
            row = f"9 1 0:4 {root} {mountpoint} rw - {filesystem_type} fixture rw\n"
            with self.subTest(row=row), mock.patch.object(MODULE, "bounded_text", return_value=row):
                with self.assertRaisesRegex(MODULE.ContractError, f"mount inventory row 1 {field}:"):
                    MODULE.read_mounts()

    def test_raw_evidence_paths_are_not_mountinfo_decoded_twice(self) -> None:
        path = pathlib.Path("/with space\\040")
        source = {"role": "checkout", "path": str(path)} | self.authored_view(path, 39, "259:3")
        source["view"]["mount_root"] = "/literal\\root"
        item = {"device": source["view"]["device"], "roles": ["checkout"], "paths": [str(path)], "path_observations": [source]}
        MODULE.validate_filesystem_observations(item)
        for invalid in ["relative", "/a/../b", "/a//b", "/a\x00b"]:
            with self.subTest(invalid=invalid), self.assertRaises(MODULE.ContractError):
                MODULE.canonical_path(invalid)

    def test_measurement_failure_terminates_and_reaps_child_without_timeout(self) -> None:
        for already_exited in [False, True]:
            with self.subTest(already_exited=already_exited):
                sampler = mock.Mock(peak_rss_kib=0, disk_measurement_error=None)
                def sample(pid: int | None = None) -> None:
                    if pid is not None:
                        sampler.disk_measurement_error = "authored mount change"
                sampler.sample.side_effect = sample
                child = mock.Mock(pid=4242, returncode=0)
                child.wait.return_value = 0
                identity = {"uid": 1000, "gid": 1000, "home": "/tmp", "user": "fixture"}
                with mock.patch.object(MODULE.os, "geteuid", return_value=1000), mock.patch.object(MODULE.subprocess, "Popen", return_value=child), mock.patch.object(MODULE.os, "killpg") as kill, mock.patch.object(MODULE, "child_exited_unreaped", return_value=already_exited), mock.patch.object(MODULE, "PROCESS_GROUP_TERM_GRACE_SECONDS", 0):
                    status, _rss, timed_out = MODULE.run_process(["fixture"], ROOT, time.monotonic() + 10, sampler, identity)
                self.assertEqual(status, 1, "even a clean child exit cannot repair missing measurements")
                self.assertFalse(timed_out)
                self.assertEqual(kill.call_args_list, [mock.call(4242, MODULE.signal.SIGTERM), mock.call(4242, MODULE.signal.SIGKILL)])
                child.wait.assert_called_once_with(timeout=20)

    @unittest.skipUnless(sys.platform == "linux", "owned process-group regression requires Linux")
    def test_abort_kills_resistant_descendant_after_leader_exits_and_reaps_owned_children(self) -> None:
        # Become a temporary subreaper only to collect this fixture's orphaned child.
        # Production cleanup promises the direct leader's reap, not global reaping.
        libc = ctypes.CDLL(None, use_errno=True)
        previous = ctypes.c_int()
        if libc.prctl(37, ctypes.byref(previous), 0, 0, 0) != 0 or libc.prctl(36, 1, 0, 0, 0) != 0:
            self.skipTest("temporary fixture subreaper is unavailable")
        try:
            for failure in ["measurement", "exited-leader", "timeout"]:
                with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                    marker = pathlib.Path(directory)
                    descendant_program = "import pathlib,signal,sys,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); pathlib.Path(sys.argv[1],'descendant-ready').write_text('ready'); time.sleep(100)"
                    leader_program = (
                        "import pathlib,signal,subprocess,sys,time\n"
                        "root=pathlib.Path(sys.argv[1])\n"
                        f"descendant=subprocess.Popen([sys.executable,'-c',{descendant_program!r},str(root)])\n"
                        "def terminate(_signal,_frame):\n"
                        "    (root/'leader-term').write_text('exit-zero')\n"
                        "    raise SystemExit(0)\n"
                        "signal.signal(signal.SIGTERM,terminate)\n"
                        "while not (root/'descendant-ready').exists(): time.sleep(0.01)\n"
                        "(root/'leader-ready').write_text(str(descendant.pid))\n"
                        + ("(root/'leader-natural-exit').write_text('exit-zero')\nraise SystemExit(0)\n" if failure == "exited-leader" else "")
                        +
                        "while True: time.sleep(0.01)\n"
                    )
                    children = []
                    popen = subprocess.Popen
                    def launch(*args: object, **kwargs: object) -> subprocess.Popen:
                        child = popen(*args, **kwargs)
                        children.append(child)
                        return child
                    sampler = mock.Mock(peak_rss_kib=0, disk_measurement_error=None)
                    def sample(pid: int | None = None) -> None:
                        if pid is None:
                            return
                        ready_deadline = time.monotonic() + 3
                        while not (marker / "leader-ready").exists() and time.monotonic() < ready_deadline:
                            time.sleep(0.01)
                        self.assertTrue((marker / "leader-ready").exists(), "fixture must install both signal handlers before abort")
                        if failure == "exited-leader":
                            exit_deadline = time.monotonic() + 3
                            while not MODULE.child_exited_unreaped(types.SimpleNamespace(pid=pid)) and time.monotonic() < exit_deadline:
                                time.sleep(0.01)
                            self.assertTrue(MODULE.child_exited_unreaped(types.SimpleNamespace(pid=pid)))
                        if failure != "timeout":
                            sampler.disk_measurement_error = "authored measurement failure"
                    sampler.sample.side_effect = sample
                    identity = {"uid": os.geteuid(), "gid": os.getegid(), "home": directory, "user": "fixture"}
                    reaped_descendant = False
                    try:
                        with mock.patch.object(MODULE.subprocess, "Popen", side_effect=launch), mock.patch.object(MODULE, "PROCESS_GROUP_TERM_GRACE_SECONDS", 0.1), mock.patch.object(MODULE, "SAMPLE_INTERVAL_SECONDS", 0.01):
                            status, _rss, timed_out = MODULE.run_process([sys.executable, "-c", leader_program, directory], ROOT, time.monotonic() + (0.2 if failure == "timeout" else 5), sampler, identity)
                        descendant_pid = int((marker / "leader-ready").read_text())
                        self.assertEqual(status, 124 if failure == "timeout" else 1)
                        self.assertEqual(timed_out, failure == "timeout")
                        self.assertEqual(children[0].returncode, 0, "leader must exit before group KILL")
                        self.assertEqual((marker / ("leader-natural-exit" if failure == "exited-leader" else "leader-term")).read_text(), "exit-zero")
                        reap_deadline = time.monotonic() + 3
                        while time.monotonic() < reap_deadline:
                            pid, wait_status = os.waitpid(descendant_pid, os.WNOHANG)
                            if pid:
                                reaped_descendant = True
                                self.assertTrue(os.WIFSIGNALED(wait_status))
                                self.assertEqual(os.WTERMSIG(wait_status), MODULE.signal.SIGKILL)
                                break
                            time.sleep(0.01)
                        self.assertTrue(reaped_descendant, "TERM-resistant descendant must be killed and collected")
                        self.assertFalse(pathlib.Path(f"/proc/{descendant_pid}").exists())
                        with self.assertRaises(ChildProcessError):
                            os.waitpid(children[0].pid, os.WNOHANG)
                        if failure != "timeout":
                            self.assertEqual(sampler.disk_measurement_error, "authored measurement failure")
                    finally:
                        if children and not reaped_descendant:
                            # The leader or adopted, unreaped fixture descendant pins this group.
                            try:
                                os.killpg(children[0].pid, MODULE.signal.SIGKILL)
                            except ProcessLookupError:
                                pass
                            children[0].wait(timeout=3)
                            cleanup_deadline = time.monotonic() + 3
                            while time.monotonic() < cleanup_deadline:
                                try:
                                    pid, _status = os.waitpid(-children[0].pid, os.WNOHANG)
                                except ChildProcessError:
                                    break
                                if not pid:
                                    time.sleep(0.01)
        finally:
            self.assertEqual(libc.prctl(36, previous.value, 0, 0, 0), 0)

    def test_btrfs_counter_and_identity_share_a_closed_descriptor(self) -> None:
        real_fstat = os.fstat
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory)
            authored = self.authored_view(path, 39, "259:3")["view"]
            mount = {name: value for name, value in authored.items() if name not in {"device", "st_dev", "statfs_fsid", "btrfs"}}
            info = {"mounted_fsid": authored["btrfs"]["mounted_fsid"], "num_devices": 1, "max_id": 100}
            devices = authored["btrfs"]["backing_devices"]
            counters = types.SimpleNamespace(f_bavail=512, f_frsize=4096, f_fsid=12345)
            for failure in [None, "ioctl", "sysfs", "statvfs", "mount"]:
                with (
                    self.subTest(failure=failure),
                    mock.patch.object(MODULE, "mounted_view", side_effect=[mount, mount | {"mount_id": 1000}] if failure == "mount" else None, return_value=mount),
                    mock.patch.object(MODULE, "btrfs_info", side_effect=[info, info | {"mounted_fsid": "changed"}] if failure == "ioctl" else None, return_value=info) as identity,
                    mock.patch.object(MODULE, "btrfs_devices", side_effect=[devices, [{"device": "260:3", "sysfs_path": "/sys/devices/other"}]] if failure == "sysfs" else None, return_value=devices),
                    mock.patch.object(MODULE.os, "fstat", return_value=types.SimpleNamespace(st_dev=39)),
                    mock.patch.object(MODULE.os, "fstatvfs", side_effect=OSError("unavailable") if failure == "statvfs" else None, return_value=counters) as counter,
                    mock.patch.object(MODULE.os, "close", wraps=os.close) as close,
                ):
                    if failure is None:
                        observed, free = MODULE.filesystem_view(path, True)
                        self.assertEqual(free, 512 * 4096)
                        self.assertEqual(observed["view"]["st_dev"], "39")
                        self.assertEqual(observed["view"]["statfs_fsid"], "12345")
                    else:
                        with self.assertRaises((MODULE.ContractError, OSError)):
                            MODULE.filesystem_view(path, True)
                    descriptor = identity.call_args_list[0].args[0]
                    self.assertEqual(counter.call_args_list[0].args[0], descriptor)
                    self.assertIn(mock.call(descriptor), close.call_args_list)
                    self.assertEqual(close.call_count, 2 if failure is None else 1)
                    for call in close.call_args_list:
                        with self.assertRaises(OSError):
                            real_fstat(call.args[0])

    def test_missing_identity_and_later_counter_failures_are_not_repaired(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory)
            failed_reader = mock.Mock(side_effect=OSError("authored missing Btrfs identity"))
            sampler = MODULE.ResourceSampler([("checkout", path)], memory_reader=lambda: 10000, filesystem_reader=failed_reader)
            self.assertIn("identity unavailable", sampler.disk_measurement_error)
            self.assertIsNone(sampler.snapshot()["available_disk_mib"])
            self.assertEqual(failed_reader.call_count, 1)
            reader = mock.Mock(return_value=(self.authored_view(path, 39, "259:3"), 10 * 1024 * 1024))
            sampler = MODULE.ResourceSampler([("checkout", path)], memory_reader=lambda: 10000, filesystem_reader=reader)
            reader.side_effect = OSError("authored failed counter sample")
            sampler.sample()
            error = sampler.disk_measurement_error
            reader.side_effect = None
            observed = sampler.snapshot()
            self.assertEqual(observed["disk_measurement_error"], error)
            self.assertIsNone(observed["available_disk_mib"])

    def test_successful_evidence_cannot_retain_a_disk_measurement_failure(self) -> None:
        evidence, revision = self.evidence_fixture()
        evidence["tasks"][0]["observed"]["disk_measurement_error"] = "authored identity change"
        with self.assertRaisesRegex(MODULE.ContractError, "incomplete disk measurements"):
            MODULE.validate_evidence(evidence, "offline", revision)

    def test_reopened_path_detects_same_target_overmount_with_valid_old_descriptor(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory)
            mounts = f"1 0 8:1 / / rw - ext4 root rw\n3 1 0:39 /subvol {path} rw - btrfs disk rw\n4 1 0:39 /subvol {path} rw - btrfs disk rw\n"
            original_fd = None
            def metadata(metadata_path: pathlib.Path, _maximum: int) -> str:
                nonlocal original_fd
                if metadata_path == MODULE.MOUNTINFO:
                    return mounts
                fd = int(metadata_path.name)
                if original_fd is None:
                    original_fd = fd
                return f"mnt_id:\t{3 if fd == original_fd else 4}\n"
            info = {"mounted_fsid": "12345678-1234-5678-9abc-123456789abc", "num_devices": 1, "max_id": 100}
            with (
                mock.patch.object(MODULE, "bounded_text", side_effect=metadata),
                mock.patch.object(MODULE, "btrfs_info", return_value=info),
                mock.patch.object(MODULE, "btrfs_devices", return_value=[{"device": "259:3", "sysfs_path": "/sys/devices/fixture"}]),
                mock.patch.object(MODULE.os, "fstat", return_value=types.SimpleNamespace(st_dev=39)),
                mock.patch.object(MODULE.os, "fstatvfs", return_value=types.SimpleNamespace(f_bavail=512, f_frsize=4096, f_fsid=12345)),
                mock.patch.object(MODULE.os, "close", wraps=os.close) as close,
            ):
                with self.assertRaisesRegex(MODULE.ContractError, "pathname identity changed"):
                    MODULE.filesystem_view(path, True)
            self.assertEqual(close.call_count, 2)
            for call in close.call_args_list:
                with self.assertRaises(OSError):
                    os.fstat(call.args[0])

    def test_reopening_uses_newly_created_ancestor_without_rejecting_same_filesystem(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            ancestor = pathlib.Path(directory)
            path = ancestor / "future"
            def counters(_fd: int) -> object:
                path.mkdir(exist_ok=True)
                return types.SimpleNamespace(f_bavail=512, f_frsize=4096, f_fsid=12345)
            mounts = "1 0 8:1 / / rw - ext4 root rw\n"
            with (
                mock.patch.object(MODULE, "bounded_text", side_effect=lambda metadata_path, _maximum: mounts if metadata_path == MODULE.MOUNTINFO else "mnt_id:\t1\n"),
                mock.patch.object(MODULE.os, "fstatvfs", side_effect=counters),
                mock.patch.object(MODULE.os, "open", wraps=os.open) as opened,
            ):
                observation, _free = MODULE.filesystem_view(path, True)
            self.assertEqual(observation["measurement_path"], str(ancestor))
            self.assertEqual([call.args[0] for call in opened.call_args_list], [ancestor, path])

    def test_btrfs_evidence_keeps_and_validates_each_raw_subvolume_source(self) -> None:
        evidence, revision = self.evidence_fixture()
        record = evidence["tasks"][0]["observed"]["filesystems"][0]
        record["device"] = "btrfs:12345678-1234-5678-9abc-123456789abc:259:3"
        record["path_observations"] = [
            {"role": role, "path": path} | self.authored_view(pathlib.Path(path), index + 1, "259:3")
            for index, (role, path) in enumerate(zip(record["roles"], record["paths"], strict=True))
        ]
        MODULE.validate_evidence(evidence, "offline", revision)
        for mutation in ["missing", "wrong-pool", "incomplete", "raw-device-loss"]:
            with self.subTest(mutation=mutation):
                changed = copy.deepcopy(evidence)
                candidate = changed["tasks"][0]["observed"]["filesystems"][0]
                if mutation == "missing":
                    del candidate["path_observations"]
                elif mutation == "wrong-pool":
                    candidate["path_observations"][0]["view"]["btrfs"]["backing_devices"][0]["device"] = "260:3"
                elif mutation == "incomplete":
                    candidate["path_observations"][0]["view"]["btrfs"]["num_devices"] = 2
                else:
                    del candidate["path_observations"][0]["view"]["st_dev"]
                with self.assertRaises(MODULE.ContractError):
                    MODULE.validate_evidence(changed, "offline", revision)

    def test_passed_btrfs_evidence_rejects_noncanonical_or_impossible_backing_provenance(self) -> None:
        evidence, revision = self.evidence_fixture()
        record = evidence["tasks"][0]["observed"]["filesystems"][0]
        record["device"] = "btrfs:12345678-1234-5678-9abc-123456789abc:259:3"
        record["path_observations"] = [
            {"role": role, "path": path} | self.authored_view(pathlib.Path(path), index + 1, "259:3")
            for index, (role, path) in enumerate(zip(record["roles"], record["paths"], strict=True))
        ]
        MODULE.validate_evidence(evidence, "offline", revision)
        for target in ["/sys/devices/../../private", "/sys/devices/./disk", "/sys/devices//disk", "/sys/devices", "/sys/devices-other/disk", "relative"]:
            changed = copy.deepcopy(evidence)
            candidate = changed["tasks"][0]["observed"]["filesystems"][0]
            for source in candidate["path_observations"]:
                source["view"]["btrfs"]["backing_devices"][0]["sysfs_path"] = target
            with self.subTest(target=target), self.assertRaises(MODULE.ContractError):
                MODULE.validate_evidence(changed, "offline", revision)
        for device in ["0:0", "99999:3", "259:1048576", "0259:3"]:
            changed = copy.deepcopy(evidence)
            candidate = changed["tasks"][0]["observed"]["filesystems"][0]
            candidate["device"] = f"btrfs:12345678-1234-5678-9abc-123456789abc:{device}"
            for source in candidate["path_observations"]:
                source["view"]["device"] = candidate["device"]
                source["view"]["btrfs"]["backing_devices"][0]["device"] = device
            with self.subTest(device=device), self.assertRaises(MODULE.ContractError):
                MODULE.validate_evidence(changed, "offline", revision)
        changed = copy.deepcopy(evidence)
        candidate = changed["tasks"][0]["observed"]["filesystems"][0]
        candidate["device"] = "btrfs:12345678-1234-5678-9abc-123456789abc:259:3,260:3"
        for source in candidate["path_observations"]:
            source["view"]["device"] = candidate["device"]
            info = source["view"]["btrfs"]
            info["num_devices"] = 2
            info["backing_devices"].append(info["backing_devices"][0] | {"device": "260:3"})
        with self.assertRaisesRegex(MODULE.ContractError, "canonical kernel device provenance"):
            MODULE.validate_evidence(changed, "offline", revision)

    def test_unavailable_identity_prevents_execution_and_midrun_error_retains_failure(self) -> None:
        task = copy.deepcopy(MODULE.load_catalogue()["tasks"][0])
        task["required-tools"] = []
        task["minimum-memory-mib"] = 1
        task["minimum-disk-mib"] = 1
        revisions = MODULE.catalogue_lens_revisions(MODULE.load_catalogue())
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory)
            view = self.authored_view(path, 39, "259:3")
            reader = mock.Mock(side_effect=MODULE.ContractError("authored missing identity"))
            sampler = MODULE.ResourceSampler([("checkout", path)], memory_reader=lambda: 10000, filesystem_reader=reader)
            identity = {"uid": 1000, "gid": 1000, "user": "fixture", "home": "/tmp", "groups": [1000]}
            with mock.patch.object(MODULE, "execution_identity", return_value=(identity, None)), mock.patch.object(MODULE, "resource_sampler_for_task", return_value=(sampler, None)), mock.patch.object(MODULE, "run_process") as execute, mock.patch.object(MODULE, "emit"):
                result, _step = MODULE.run_task(task, revisions, 1, 3, time.monotonic() + 10)
                self.assertEqual(result["state"], "unavailable")
                self.assertIsNotNone(result["observed"]["disk_measurement_error"])
                execute.assert_not_called()
            reader.side_effect = None
            reader.return_value = (view, 10000 * 1024 * 1024)
            sampler = MODULE.ResourceSampler([("checkout", path)], memory_reader=lambda: 10000, filesystem_reader=reader)
            def clean_exit_with_missing_sample(*_args: object) -> tuple[int, int, bool]:
                reader.side_effect = OSError("authored midrun failure")
                sampler.sample()
                return 0, 0, False
            with mock.patch.object(MODULE, "execution_identity", return_value=(identity, None)), mock.patch.object(MODULE, "resource_sampler_for_task", return_value=(sampler, None)), mock.patch.object(MODULE, "run_process", side_effect=clean_exit_with_missing_sample), mock.patch.object(MODULE, "emit"):
                result, _step = MODULE.run_task(task, revisions, 1, 3, time.monotonic() + 10)
                self.assertEqual(result["state"], "failed")
                self.assertEqual(result["observed"]["exit_status"], 0)
                self.assertFalse(result["observed"]["timed_out"])
                self.assertIn("midrun failure", result["reason"])

    def test_mount_inventory_and_sysfs_traversal_limits_fail_closed(self) -> None:
        for metadata in ["pos:\t0\n", "mnt_id:\tno\n", "mnt_id:\t1\nmnt_id:\t1\n"]:
            with self.subTest(metadata=metadata), mock.patch.object(MODULE, "read_mounts", return_value={}), mock.patch.object(MODULE, "bounded_text", return_value=metadata), self.assertRaises(MODULE.ContractError):
                MODULE.mounted_view(42, pathlib.Path("/"))
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            metadata = root / "oversized"
            metadata.write_bytes(b"12345")
            with self.assertRaises(MODULE.ContractError):
                MODULE.bounded_text(metadata, 4)
            fsid = "12345678-1234-5678-9abc-123456789abc"
            devices = root / "fs/btrfs" / fsid / "devices"
            devices.mkdir(parents=True)
            (devices / "one").touch()
            (devices / "two").touch()
            with mock.patch.object(MODULE, "SYSFS", root), mock.patch.object(MODULE, "MAX_BTRFS_DEVICES", 1), self.assertRaisesRegex(MODULE.ContractError, "exceeds"):
                MODULE.btrfs_devices({"mounted_fsid": fsid, "num_devices": 2})

    def test_task_sampler_always_registers_checkout_and_temporary_filesystems(self) -> None:
        task = copy.deepcopy(MODULE.load_catalogue()["tasks"][0])
        sampler, discovery_error = MODULE.resource_sampler_for_task(task)
        snapshot = sampler.snapshot(sample=False)
        roles = {
            role for filesystem in snapshot["filesystems"] for role in filesystem["roles"]
        }
        paths = {
            path for filesystem in snapshot["filesystems"] for path in filesystem["paths"]
        }

        self.assertIsNone(discovery_error)
        self.assertEqual(roles, {"checkout", "temporary-directory"})
        self.assertIn(str(pathlib.Path(tempfile.gettempdir()).resolve()), paths)

    def test_podman_graph_root_discovery_is_read_only_and_bounded(self) -> None:
        self.assertEqual(MODULE.PODMAN_GRAPH_ROOT_DISCOVERY_TIMEOUT_SECONDS, 30.0)
        completed = subprocess.CompletedProcess(
            args=[], returncode=0, stdout="/var/lib/containers/storage\n", stderr=""
        )
        with (
            mock.patch.object(MODULE.shutil, "which", return_value="/usr/bin/podman"),
            mock.patch.object(MODULE.subprocess, "run", return_value=completed) as run,
        ):
            graph_root = MODULE.discover_podman_graph_root()

        self.assertEqual(graph_root, pathlib.Path("/var/lib/containers/storage"))
        run.assert_called_once_with(
            ["podman", "info", "--format", "{{.Store.GraphRoot}}"],
            check=False,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=MODULE.PODMAN_GRAPH_ROOT_DISCOVERY_TIMEOUT_SECONDS,
        )

    def test_podman_graph_root_discovery_failures_remain_unavailable(self) -> None:
        failures = (
            subprocess.CompletedProcess(args=[], returncode=1, stdout="ignored\n", stderr="error"),
            subprocess.CompletedProcess(args=[], returncode=0, stdout="\n", stderr=""),
        )
        for completed in failures:
            with (
                self.subTest(returncode=completed.returncode, stdout=completed.stdout),
                mock.patch.object(MODULE.shutil, "which", return_value="/usr/bin/podman"),
                mock.patch.object(MODULE.subprocess, "run", return_value=completed),
            ):
                self.assertIsNone(MODULE.discover_podman_graph_root())

        with (
            mock.patch.object(MODULE.shutil, "which", return_value="/usr/bin/podman"),
            mock.patch.object(
                MODULE.subprocess,
                "run",
                side_effect=subprocess.TimeoutExpired(
                    cmd=["podman", "info"],
                    timeout=MODULE.PODMAN_GRAPH_ROOT_DISCOVERY_TIMEOUT_SECONDS,
                ),
            ),
        ):
            self.assertIsNone(MODULE.discover_podman_graph_root())

    def test_task_sampler_admits_discovered_graph_root_and_caps_its_deadline(self) -> None:
        task = copy.deepcopy(MODULE.load_catalogue()["tasks"][0])
        task["required-tools"] = ["podman"]
        with tempfile.TemporaryDirectory() as temporary:
            graph_root = pathlib.Path(temporary).resolve()
            with (
                mock.patch.object(MODULE.shutil, "which", return_value="/usr/bin/podman"),
                mock.patch.object(MODULE.time, "monotonic", return_value=100.0),
                mock.patch.object(
                    MODULE, "discover_podman_graph_root", return_value=graph_root
                ) as discover,
            ):
                sampler, discovery_error = MODULE.resource_sampler_for_task(
                    task, tier_deadline=112.5
                )

        self.assertIsNone(discovery_error)
        discover.assert_called_once_with(12.5)
        snapshot = sampler.snapshot(sample=False)
        measured_paths = {
            path
            for filesystem in snapshot["filesystems"]
            for path in filesystem["paths"]
        }
        self.assertIn(str(graph_root), measured_paths)

    def test_task_sampler_fails_closed_when_graph_root_is_unavailable(self) -> None:
        task = copy.deepcopy(MODULE.load_catalogue()["tasks"][0])
        task["required-tools"] = ["podman"]
        with (
            mock.patch.object(MODULE.shutil, "which", return_value="/usr/bin/podman"),
            mock.patch.object(MODULE, "discover_podman_graph_root", return_value=None),
        ):
            _sampler, discovery_error = MODULE.resource_sampler_for_task(task)

        self.assertEqual(
            discovery_error, "Podman graph root could not be discovered read-only"
        )

    def test_preflight_checks_every_monitored_filesystem(self) -> None:
        mebibyte = 1024 * 1024
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            checkout = root / "checkout"
            graph_root = root / "graph-root"
            checkout.mkdir()
            graph_root.mkdir()

            def stat_reader(path: pathlib.Path) -> types.SimpleNamespace:
                return types.SimpleNamespace(st_dev=1 if path == checkout else 2)

            def disk_reader(path: pathlib.Path) -> types.SimpleNamespace:
                free = 10_000 if path == checkout else 1
                return types.SimpleNamespace(free=free * mebibyte)

            sampler = MODULE.ResourceSampler(
                [("checkout", checkout), ("podman-graph-root", graph_root)],
                memory_reader=lambda: 10_000,
                filesystem_reader=lambda path, counter: (
                    self.authored_view(path, stat_reader(path).st_dev),
                    disk_reader(path).free if counter else None,
                ),
            )
            task = copy.deepcopy(MODULE.load_catalogue()["tasks"][0])
            task["required-tools"] = []
            task["minimum-memory-mib"] = 1
            task["minimum-disk-mib"] = 2
            _observed, reason, _identity = MODULE.preflight(task, sampler, None)

            self.assertIn("filesystem 2", reason)

    def test_non_privileged_identity_uses_current_non_root_account(self) -> None:
        task = {"privileged": False}
        account = types.SimpleNamespace(pw_name="builder", pw_dir="/home/builder")
        with (
            mock.patch.object(MODULE.os, "geteuid", return_value=1001),
            mock.patch.object(MODULE.os, "getegid", return_value=1002),
            mock.patch.object(MODULE.os, "getgroups", return_value=[1002, 1003]),
            mock.patch.object(MODULE.pwd, "getpwuid", return_value=account),
        ):
            identity, reason = MODULE.execution_identity(task)

        self.assertIsNone(reason)
        self.assertEqual(
            identity,
            {
                "uid": 1001,
                "gid": 1002,
                "user": "builder",
                "home": "/home/builder",
                "groups": [1002, 1003],
            },
        )

    def test_root_runner_resolves_complete_non_root_sudo_identity(self) -> None:
        task = {"privileged": False}
        account = types.SimpleNamespace(
            pw_name="runner", pw_uid=1001, pw_gid=1001, pw_dir="/home/runner"
        )
        with (
            mock.patch.object(MODULE.os, "geteuid", return_value=0),
            mock.patch.dict(
                MODULE.os.environ,
                {"SUDO_UID": "1001", "SUDO_GID": "1001", "SUDO_USER": "runner"},
                clear=True,
            ),
            mock.patch.object(MODULE.pwd, "getpwnam", return_value=account),
            mock.patch.object(MODULE.os, "getgrouplist", return_value=[1001, 998]),
        ):
            identity, reason = MODULE.execution_identity(task)

        self.assertIsNone(reason)
        self.assertEqual(identity["uid"], 1001)
        self.assertEqual(identity["gid"], 1001)
        self.assertEqual(identity["groups"], [1001, 998])

    def test_root_runner_rejects_incomplete_or_forged_sudo_identity(self) -> None:
        task = {"privileged": False}
        account = types.SimpleNamespace(
            pw_name="runner", pw_uid=1001, pw_gid=1001, pw_dir="/home/runner"
        )
        environments = [
            {},
            {"SUDO_UID": "0", "SUDO_GID": "0", "SUDO_USER": "root"},
            {"SUDO_UID": "1002", "SUDO_GID": "1001", "SUDO_USER": "runner"},
            {"SUDO_UID": "not-a-number", "SUDO_GID": "1001", "SUDO_USER": "runner"},
        ]
        with mock.patch.object(MODULE.os, "geteuid", return_value=0):
            for environment in environments:
                with (
                    self.subTest(environment=environment),
                    mock.patch.dict(MODULE.os.environ, environment, clear=True),
                    mock.patch.object(MODULE.pwd, "getpwnam", return_value=account),
                ):
                    identity, reason = MODULE.execution_identity(task)
                    self.assertIsNone(identity)
                    self.assertIsNotNone(reason)

    @unittest.skipUnless(MODULE.os.geteuid() == 0, "requires a root test runner")
    def test_root_runner_executes_non_privileged_child_as_selected_user(self) -> None:
        account = MODULE.pwd.getpwnam("nobody")
        groups = MODULE.os.getgrouplist(account.pw_name, account.pw_gid)
        if 0 in groups:
            self.skipTest("nobody unexpectedly belongs to the root group")
        identity = {
            "uid": account.pw_uid,
            "gid": account.pw_gid,
            "user": account.pw_name,
            "home": account.pw_dir,
            "groups": groups,
        }
        expected = repr(
            (
                account.pw_uid,
                account.pw_gid,
                account.pw_name,
                account.pw_dir,
            )
        )
        program = (
            "import os; "
            "actual=(os.geteuid(),os.getegid(),os.environ['USER'],os.environ['HOME']); "
            f"raise SystemExit(0 if actual == {expected} and 'SUDO_UID' not in os.environ else 1)"
        )
        sampler = mock.Mock(peak_rss_kib=0, disk_measurement_error=None)
        sampler.sample.return_value = None
        status, _rss, timed_out = MODULE.run_process(
            [sys.executable, "-c", program],
            ROOT,
            time.monotonic() + 10,
            sampler,
            identity,
        )

        self.assertEqual(status, 0)
        self.assertFalse(timed_out)

    def test_expired_tier_marks_task_failed_without_execution(self) -> None:
        catalogue = MODULE.load_catalogue()
        task = copy.deepcopy(catalogue["tasks"][0])
        revisions = MODULE.catalogue_lens_revisions(catalogue)
        with mock.patch.object(MODULE, "emit") as emit:
            result, next_step = MODULE.run_task(
                task,
                revisions,
                1,
                3,
                time.monotonic() - 1,
            )

        self.assertEqual(result["state"], "failed")
        self.assertIn("tier deadline exhausted", result["reason"])
        self.assertEqual(next_step, 3)
        emit.assert_any_call(1, 3, "FAIL", f"{task['id']}:preflight", result["reason"])
        emit.assert_any_call(2, 3, "GAP", f"{task['id']}:execute", "not run")

    def test_task_process_deadline_is_capped_to_tier_time_remaining(self) -> None:
        catalogue = MODULE.load_catalogue()
        task = copy.deepcopy(catalogue["tasks"][0])
        task["required-tools"] = ["python3"]
        task["minimum-memory-mib"] = 1
        task["minimum-disk-mib"] = 1
        resources = {
            "available_memory_mib": 10_000,
            "available_disk_mib": 10_000,
            "peak_memory_delta_mib": 0,
            "peak_rss_kib": 0,
            "disk_growth_mib": 0,
            "sample_interval_milliseconds": 250,
            "memory_scope": "system-available-memory",
            "rss_scope": "child-process-tree",
            "disk_scope": "deduplicated-filesystems",
            "filesystems": [
                {
                    "device": "fixture-device",
                    "roles": ["checkout", "temporary-directory"],
                    "paths": [str(ROOT), tempfile.gettempdir()],
                    "baseline_free_mib": 10_000,
                    "minimum_free_mib": 10_000,
                    "peak_growth_mib": 0,
                }
            ],
        }
        sampler = mock.Mock()
        sampler.snapshot.return_value = resources
        tier_deadline = time.monotonic() + 30
        with (
            mock.patch.object(
                MODULE, "resource_sampler_for_task", return_value=(sampler, None)
            ),
            mock.patch.object(
                MODULE, "run_process", return_value=(0, 0, False)
            ) as run_process,
            mock.patch.object(MODULE, "emit"),
        ):
            result, next_step = MODULE.run_task(
                task,
                MODULE.catalogue_lens_revisions(catalogue),
                1,
                3,
                tier_deadline,
            )

        self.assertEqual(result["state"], "passed")
        self.assertEqual(next_step, 3)
        self.assertEqual(run_process.call_args.args[2], tier_deadline)

    def test_unavailable_and_not_run_tasks_fill_every_event_slot(self) -> None:
        revision = "1" * 40
        catalogue = MODULE.load_catalogue()
        task_count = len(MODULE.selected_tasks(catalogue, "trusted-live", None)[1])
        with tempfile.TemporaryDirectory() as temporary:
            evidence_path = pathlib.Path(temporary) / "evidence.json"
            args = MODULE.parser().parse_args(
                [
                    "run",
                    "--tier",
                    "trusted-live",
                    "--revision",
                    revision,
                    "--evidence",
                    str(evidence_path),
                ]
            )
            with (
                mock.patch.object(MODULE, "git_revision", return_value=revision),
                mock.patch.object(
                    MODULE, "preflight", return_value=({}, "fixture unavailable", None)
                ),
                mock.patch.object(MODULE, "emit") as emit,
            ):
                status = MODULE.run(args)
            evidence = json.loads(evidence_path.read_text(encoding="utf-8"))

        terminal = [
            (call.args[0], call.args[2])
            for call in emit.call_args_list
            if call.args[2] in {"PASS", "FAIL", "GAP"}
            ]
        self.assertEqual(status, 1)
        final_step = task_count * 2 + 1
        self.assertEqual(
            [step for step, _state in terminal], list(range(1, final_step + 1))
        )
        self.assertTrue(all(state == "GAP" for _step, state in terminal[:-1]))
        self.assertEqual(terminal[-1], (final_step, "PASS"))
        self.assertEqual(evidence["tasks"][0]["state"], "unavailable")
        self.assertTrue(all(task["state"] == "not-run" for task in evidence["tasks"][1:]))

    def test_run_rejects_unfiltered_pre_release_before_execution(self) -> None:
        args = MODULE.parser().parse_args(["run", "--tier", "pre-release"])
        with self.assertRaisesRegex(MODULE.ContractError, "parallel GitHub coordinator"):
            MODULE.run(args)

    def test_hosted_worker_identity_rejected_before_execution(self) -> None:
        task_id = "nextcloud-application"
        base = ["run", "--tier", "pre-release", "--task", task_id]
        cases = (
            (["--worker-id", task_id], "requires --coordinator-id"),
            (["--worker-id", "forgejo-root-modes"], "must match the selected --task"),
            (["--coordinator-id", "not-a-uuid"], "canonical UUID"),
            (["--coordinator-id", "0a20a918-93bd-43a2-b346-8f7dd63b08be"], "positive GITHUB_RUN_ATTEMPT"),
        )
        for arguments, pattern in cases:
            with (
                self.subTest(arguments=arguments),
                mock.patch.object(MODULE, "git_revision", return_value="1" * 40),
                mock.patch.object(MODULE, "run_task") as execute,
                mock.patch.dict(MODULE.os.environ, {"GITHUB_RUN_ATTEMPT": ""}),
            ):
                args = MODULE.parser().parse_args([*base, *arguments])
                with self.assertRaisesRegex(MODULE.ContractError, pattern):
                    MODULE.run(args)
                execute.assert_not_called()

    def test_focused_local_run_serializes_success_and_failure(self) -> None:
        task_id = "nextcloud-application"
        for state, expected_status in (("passed", 0), ("failed", 1)):
            with self.subTest(state=state), tempfile.TemporaryDirectory() as temporary:
                template, revision = self.evidence_fixture(
                    tier_id="pre-release", task_id=task_id, state=state
                )
                task = copy.deepcopy(template["tasks"][0])
                if state == "failed":
                    task["observed"]["exit_status"] = 1
                output = pathlib.Path(temporary) / "local-evidence.json"
                args = MODULE.parser().parse_args(
                    ["run", "--tier", "pre-release", "--task", task_id,
                     "--revision", revision, "--evidence", str(output)]
                )
                with (
                    mock.patch.object(MODULE, "git_revision", return_value=revision),
                    mock.patch.object(MODULE, "run_task", return_value=(task, 3)),
                    mock.patch.object(MODULE, "now", side_effect=[
                        "2026-09-10T12:00:00Z", "2026-09-10T12:00:00Z",
                        "2026-09-10T12:01:00Z", "2026-09-10T12:01:00Z"
                    ]),
                    mock.patch.object(MODULE.time, "monotonic", side_effect=[100.0, 160.0]),
                    mock.patch.object(MODULE, "configured_boxferry_binary_digest", return_value="a" * 64),
                    mock.patch.dict(MODULE.os.environ, {"GITHUB_RUN_ATTEMPT": "7"}),
                ):
                    self.assertEqual(MODULE.run(args), expected_status)
                evidence = json.loads(output.read_text(encoding="utf-8"))
                self.assertEqual(evidence["outcome"], state)
                self.assertEqual(evidence["run"]["evidence_kind"], "local")
                self.assertEqual(evidence["run"]["coordinator_id"], evidence["run"]["id"])
                self.assertEqual(evidence["run"]["boxferry_binary_sha256"], "a" * 64)
                self.assertNotIn("github_run_attempt", evidence["run"])
                self.assertEqual(evidence["tasks"][0]["state"], state)
                MODULE.validate_evidence(evidence, "pre-release", revision, task_id=task_id)

    def test_focused_identity_rejects_hosted_claims(self) -> None:
        task_id = "nextcloud-application"
        evidence, revision = self.evidence_fixture(tier_id="pre-release", task_id=task_id)
        evidence["run"]["evidence_kind"] = "local"
        evidence["run"]["coordinator_id"] = evidence["run"]["id"]
        evidence["run"].pop("github_run_attempt")
        MODULE.validate_evidence(evidence, "pre-release", revision, task_id=task_id)
        for label, mutate, pattern in (
            ("local claims attempt", lambda run: run.update(github_run_attempt=1), "local evidence must not claim"),
            ("local claims coordinator", lambda run: run.update(coordinator_id="0a20a918-93bd-43a2-b346-8f7dd63b08be"), "local evidence must own"),
            ("worker claims local identity", lambda run: run.update(evidence_kind="worker", github_run_attempt=1), "separate coordinator"),
            ("worker lacks attempt", lambda run: run.update(evidence_kind="worker", coordinator_id="0a20a918-93bd-43a2-b346-8f7dd63b08be"), "positive GitHub run attempt"),
        ):
            with self.subTest(label=label):
                changed = copy.deepcopy(evidence)
                mutate(changed["run"])
                with self.assertRaisesRegex(MODULE.ContractError, pattern):
                    MODULE.validate_evidence(changed, "pre-release", revision, task_id=task_id)

    def test_run_rejects_abbreviated_revision_before_execution(self) -> None:
        output = subprocess.run(
            [
                sys.executable,
                str(RUNNER),
                "run",
                "--tier",
                "offline",
                "--revision",
                "deadbeef",
            ],
            cwd=ROOT,
            check=False,
            text=True,
            stderr=subprocess.PIPE,
        )
        self.assertEqual(output.returncode, 2)
        self.assertIn("full lowercase 40-character Git SHA", output.stderr)

    def test_run_rejects_unreviewed_lens_revision_overrides(self) -> None:
        output = subprocess.run(
            [
                sys.executable,
                str(RUNNER),
                "run",
                "--tier",
                "offline",
                "--compose-lens-revision",
                "2" * 40,
            ],
            cwd=ROOT,
            check=False,
            text=True,
            stderr=subprocess.PIPE,
        )
        self.assertEqual(output.returncode, 2)
        self.assertIn("unrecognized arguments", output.stderr)

    def test_github_and_release_use_the_shared_exact_sha_gate_once(self) -> None:
        ci = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        tiers = (ROOT / ".github/workflows/migration-readiness.yml").read_text(
            encoding="utf-8"
        )
        legacy_live = (
            ROOT / ".github/workflows/podman-live-conformance.yml"
        ).read_text(encoding="utf-8")
        release = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        self.assertEqual(
            ci.count("scripts/migration-readiness.py run\n          --tier offline"), 1
        )
        self.assertNotIn("pull_request:", tiers)
        self.assertNotIn("pull_request:", legacy_live)
        self.assertIn(
            "github.ref_name == github.event.repository.default_branch", tiers
        )
        self.assertIn("workflow_call:", tiers)
        for required in (
            "uses: ./.github/workflows/ci.yml",
            "uses: ./.github/workflows/migration-readiness.yml",
            "name: ${{ needs.migration-readiness.outputs.evidence_artifact }}",
            "--tier pre-release",
            '--revision "${GITHUB_SHA}"',
            "--require-aggregate",
            "--require-success",
            "needs: [deterministic, release-metadata, migration-readiness, migration-readiness-evidence]",
            "!inputs.validation_only",
        ):
            self.assertIn(required, release)
        self.assertNotIn("gh run list", release)
        self.assertNotIn("gh run download", release)
        self.assertIn(
            "migration-readiness-worker-${{ github.sha }}-${{ github.run_id }}-${{ matrix.task }}",
            tiers,
        )
        self.assertIn(
            "pattern: migration-readiness-worker-${{ github.sha }}-${{ github.run_id }}-*",
            tiers,
        )
        self.assertGreaterEqual(tiers.count("overwrite: true"), 4)

    def test_collector_requires_every_bound_pre_release_worker(self) -> None:
        coordinator = "0a20a918-93bd-43a2-b346-8f7dd63b08be"
        binary_sha256 = "a" * 64
        catalogue = MODULE.load_catalogue()
        expected = MODULE.selected_tasks(catalogue, "pre-release", None)[1]
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            arguments = []
            revision = "1" * 40
            for index, task in enumerate(expected):
                document, revision = self.evidence_fixture(
                    tier_id="pre-release", task_id=task["id"]
                )
                second = (index // 4) * 2
                started_at = f"2026-09-10T12:00:{second:02d}Z"
                finished_at = f"2026-09-10T12:00:{second + 1:02d}Z"
                document["tasks"][0]["started_at"] = started_at
                document["tasks"][0]["finished_at"] = finished_at
                document["run"].update(
                    started_at=started_at,
                    finished_at=finished_at,
                    coordinator_id=coordinator,
                    worker_id=task["id"],
                    evidence_kind="worker",
                    boxferry_binary_sha256=binary_sha256,
                )
                path = root / f"{task['id']}.json"
                MODULE.atomic_json(path, document)
                arguments.extend(("--evidence", str(path)))
            output = root / "aggregate.json"
            args = MODULE.parser().parse_args([
                "collect-evidence", "--tier", "pre-release", "--revision", revision,
                "--coordinator-id", coordinator, *arguments,
                "--boxferry-binary-sha256", binary_sha256,
                "--evidence-output", str(output),
            ])
            self.assertEqual(MODULE.collect(args), 0)
            MODULE.validate_evidence(
                json.loads(output.read_text(encoding="utf-8")), "pre-release", revision
            )
            missing = arguments[:-2]
            args = MODULE.parser().parse_args([
                "collect-evidence", "--tier", "pre-release", "--revision", revision,
                "--coordinator-id", coordinator, *missing,
                "--boxferry-binary-sha256", binary_sha256,
                "--evidence-output", str(output),
            ])
            with self.assertRaisesRegex(MODULE.ContractError, "every selected task"):
                MODULE.collect(args)

    def test_collector_excludes_inactive_failed_job_retry_wait(self) -> None:
        """Retained workers and a later retried worker keep one active-time budget."""
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            coordinator, binary_sha256, revision, paths, documents = self.collector_fixture(root)
            retried = False
            retained_count = 0
            retained_intervals = (
                ("2026-09-10T14:59:00Z", "2026-09-10T15:04:08.348Z"),
                ("2026-09-10T15:04:08.348Z", "2026-09-10T15:09:16.696Z"),
                ("2026-09-10T15:09:16.696Z", "2026-09-10T15:14:25.045Z"),
            )
            for path, document in zip(paths, documents, strict=True):
                task = document["tasks"][0]
                if task["id"] == "supabase-application":
                    task["started_at"] = "2026-09-10T15:37:00Z"
                    task["finished_at"] = "2026-09-10T15:42:07.582Z"
                    document["run"]["github_run_attempt"] = 2
                    retried = True
                else:
                    task["started_at"], task["finished_at"] = retained_intervals[
                        retained_count // 4
                    ]
                    retained_count += 1
                document["run"]["started_at"] = task["started_at"]
                document["run"]["finished_at"] = task["finished_at"]
                MODULE.atomic_json(path, document)
            self.assertTrue(retried)

            output = root / "aggregate.json"
            args = self.collect_args(
                coordinator, binary_sha256, revision, paths, output
            )
            self.assertEqual(MODULE.collect(args), 0)
            aggregate = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(aggregate["run"]["started_at"], "2026-09-10T14:59:00Z")
            self.assertEqual(aggregate["run"]["finished_at"], "2026-09-10T15:42:07.582Z")
            self.assertEqual(aggregate["run"]["wall_seconds"], 925.045)
            self.assertEqual(
                [item["active_wall_seconds"] for item in aggregate["run"]["attempt_timings"]],
                [925.045, 307.582],
            )
            self.assertEqual(
                aggregate["run"]["total_worker_wall_seconds"],
                float(len(documents)),
            )
            MODULE.validate_evidence(aggregate, "pre-release", revision)

            offset_aggregate = json.loads(json.dumps(aggregate))
            first_attempt = offset_aggregate["run"]["attempt_timings"][0]
            first_task = next(
                task
                for task in offset_aggregate["tasks"]
                if task["id"] in first_attempt["task_ids"] and task["started_at"] == first_attempt["started_at"]
            )
            first_task["started_at"] = "2026-09-10T16:59:00+02:00"
            first_task["finished_at"] = "2026-09-10T17:04:08.348+02:00"
            first_attempt["started_at"] = first_task["started_at"]
            self.assertEqual(
                MODULE.validate_attempt_timings(offset_aggregate["run"], offset_aggregate["tasks"]),
                925.045,
            )

            overlapping = json.loads(json.dumps(aggregate))
            retry_timing = overlapping["run"]["attempt_timings"][1]
            retry_task = next(task for task in overlapping["tasks"] if task["id"] == retry_timing["task_ids"][0])
            retry_task["started_at"] = "2026-09-10T15:14:00Z"
            retry_task["finished_at"] = "2026-09-10T15:19:07.582Z"
            retry_timing["started_at"] = retry_task["started_at"]
            retry_timing["finished_at"] = retry_task["finished_at"]
            with self.assertRaisesRegex(MODULE.ContractError, "overlap or reverse"):
                MODULE.validate_attempt_timings(overlapping["run"], overlapping["tasks"])

    def test_collector_rejects_invalid_worker_sets_and_bindings(self) -> None:
        def failed(documents: list[dict[str, object]]) -> None:
            document = documents[0]
            document["outcome"] = "failed"
            document["tasks"][0]["state"] = "failed"
            document["tasks"][0]["reason"] = "fixture failure"
            document["tasks"][0]["observed"]["exit_status"] = 1

        def timed_out(documents: list[dict[str, object]]) -> None:
            failed(documents)
            documents[0]["run"]["timed_out"] = True
            documents[0]["tasks"][0]["observed"]["exit_status"] = 124
            documents[0]["tasks"][0]["observed"]["timed_out"] = True

        def missing_attempt(documents: list[dict[str, object]]) -> None:
            documents[0]["run"].pop("github_run_attempt")

        def local_fragment(documents: list[dict[str, object]]) -> None:
            documents[0]["run"]["evidence_kind"] = "local"
            documents[0]["run"].pop("github_run_attempt")
            documents[0]["run"]["coordinator_id"] = documents[0]["run"]["id"]

        def malformed_attempt(documents: list[dict[str, object]]) -> None:
            documents[0]["run"]["github_run_attempt"] = 0

        def wrong_coordinator(documents: list[dict[str, object]]) -> None:
            documents[0]["run"]["coordinator_id"] = "8b2cb4cf-7ec3-4ba2-b87e-d4eca327ca52"

        def wrong_revision(documents: list[dict[str, object]]) -> None:
            documents[0]["run"]["revision"] = "2" * 40

        def wrong_catalogue(documents: list[dict[str, object]]) -> None:
            documents[0]["run"]["catalogue_sha256"] = "0" * 64

        def wrong_worker(documents: list[dict[str, object]]) -> None:
            documents[0]["run"]["worker_id"] = "wrong-worker"

        def wrong_binary(documents: list[dict[str, object]]) -> None:
            documents[0]["run"]["boxferry_binary_sha256"] = "b" * 64

        def wrong_shard(documents: list[dict[str, object]]) -> None:
            shard = next(
                document
                for document in documents
                if document["tasks"][0]["id"] == "podman-complete-matrix-shard-1"
            )
            shard["tasks"][0]["matrix_evidence"]["shard"] = "2/4"

        def wrong_matrix_digest(documents: list[dict[str, object]]) -> None:
            shard = next(
                document
                for document in documents
                if document["tasks"][0]["id"] == "podman-complete-matrix-shard-1"
            )
            shard["tasks"][0]["matrix_evidence"]["matrix_sha256"] = "0" * 64

        def incomplete_matrix(documents: list[dict[str, object]]) -> None:
            shard = next(
                document
                for document in documents
                if document["tasks"][0]["id"] == "podman-complete-matrix-shard-1"
            )
            shard["tasks"][0]["matrix_evidence"]["row_ids"].pop()

        def excessive_concurrency(documents: list[dict[str, object]]) -> None:
            for document in documents:
                document["run"]["started_at"] = "2026-09-10T12:00:00Z"
                document["run"]["finished_at"] = "2026-09-10T12:00:01Z"
                document["tasks"][0]["started_at"] = "2026-09-10T12:00:00Z"
                document["tasks"][0]["finished_at"] = "2026-09-10T12:00:01Z"

        def aggregate_timeout(documents: list[dict[str, object]]) -> None:
            document = documents[-1]
            document["run"]["started_at"] = "2026-09-10T12:00:00Z"
            document["run"]["finished_at"] = "2026-09-10T12:20:01Z"
            document["tasks"][0]["started_at"] = "2026-09-10T12:00:00Z"
            document["tasks"][0]["finished_at"] = "2026-09-10T12:20:01Z"

        cases = [
            ("failed", failed, "successful worker evidence"),
            ("timed out", timed_out, "timed-out worker evidence"),
            ("local fragment", local_fragment, "not a single worker evidence"),
            ("missing attempt", missing_attempt, "positive GitHub run attempt"),
            ("malformed attempt", malformed_attempt, "minimum"),
            ("wrong coordinator", wrong_coordinator, "different coordinator"),
            ("wrong revision", wrong_revision, "revision is"),
            ("wrong catalogue", wrong_catalogue, "catalogue digest"),
            ("wrong worker", wrong_worker, "worker identity"),
            ("wrong binary", wrong_binary, "different BoxFerry binary"),
            ("wrong shard", wrong_shard, "matrix_evidence"),
            ("wrong matrix digest", wrong_matrix_digest, "matrix_evidence"),
            ("incomplete matrix", incomplete_matrix, "minItems"),
            ("excessive concurrency", excessive_concurrency, "concurrency limit"),
            ("aggregate timeout", aggregate_timeout, "aggregate tier deadline"),
        ]
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            coordinator, binary_sha256, revision, paths, original = self.collector_fixture(root)
            for label, mutate, pattern in cases:
                with self.subTest(label=label):
                    documents = copy.deepcopy(original)
                    mutate(documents)
                    for path, document in zip(paths, documents, strict=True):
                        MODULE.atomic_json(path, document)
                    args = self.collect_args(
                        coordinator, binary_sha256, revision, paths, root / "aggregate.json"
                    )
                    with self.assertRaisesRegex(MODULE.ContractError, pattern):
                        MODULE.collect(args)

            for path, document in zip(paths, original, strict=True):
                MODULE.atomic_json(path, document)
            duplicate_paths = [*paths, paths[0]]
            args = self.collect_args(
                coordinator, binary_sha256, revision, duplicate_paths, root / "aggregate.json"
            )
            with self.assertRaisesRegex(MODULE.ContractError, "duplicate worker"):
                MODULE.collect(args)


if __name__ == "__main__":
    unittest.main()
