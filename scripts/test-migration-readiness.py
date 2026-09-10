#!/usr/bin/env python3
"""Contract tests for the migration-readiness catalogue and evidence helper."""

from __future__ import annotations

import copy
import importlib.util
import json
import math
import pathlib
import subprocess
import sys
import tempfile
import time
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
            "schema_version": 1,
            "outcome": "passed" if state == "passed" else "failed",
            "run": {
                "id": "0a20a918-93bd-43a2-b346-8f7dd63b08be",
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
                "timed_out": False,
                "fresh": True,
                "selection": {
                    "kind": "task" if task_id is not None else "tier",
                    "task": task_id,
                },
                "lens_revisions": lens_revisions,
            },
            "manual_prerequisites": tier["manual-prerequisites"],
            "tasks": task_results,
            "gaps": catalogue["gaps"],
        }
        return evidence, revision

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
                "podman-complete-matrix",
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
            "pre-release": 19800,
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
            self.assertEqual(plan["max_concurrency"], 1)
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
                "podman-api-6.1.0-rootless",
            ],
        )
        self.assertEqual(task["deadline-seconds"], 5400)
        self.assertEqual(task["minimum-memory-mib"], 12288)
        self.assertEqual(task["minimum-disk-mib"], 24576)
        self.assertEqual(task["maximum-rss-mib"], 14336)
        self.assertEqual(task["maximum-disk-growth-mib"], 20480)
        self.assertEqual(task["required-tools"], ["podman"])
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
                'evidence-schema = "docs/schemas/migration-readiness-evidence-v1.schema.json"',
                'evidence-schema = "docs/schemas/migration-readiness-evidence-v1.schema.json"\nunknown-top = true',
                1,
            ),
            "missing top-level": source.replace("schema = 1\n", "", 1),
            "mistyped top-level": source.replace("schema = 1", 'schema = "1"', 1),
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
            "missing Lens revision": source.replace(
                'revision = "44c9b6a4c72e8ea0db2cd2d2c73f2bf4d07b6a0e"\n',
                "",
                1,
            ),
        }
        for label, changed in mutations.items():
            with self.subTest(label=label):
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
            timeout=10.0,
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
                disk_usage_reader=disk_reader,
                stat_reader=stat_reader,
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
        sampler = mock.Mock(peak_rss_kib=0)
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
        with tempfile.TemporaryDirectory() as temporary:
            evidence_path = pathlib.Path(temporary) / "evidence.json"
            args = MODULE.parser().parse_args(
                [
                    "run",
                    "--tier",
                    "pre-release",
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
        self.assertEqual([step for step, _state in terminal], list(range(1, 22)))
        self.assertTrue(all(state == "GAP" for _step, state in terminal[:-1]))
        self.assertEqual(terminal[-1], (21, "PASS"))
        self.assertEqual(evidence["tasks"][0]["state"], "unavailable")
        self.assertTrue(all(task["state"] == "not-run" for task in evidence["tasks"][1:]))

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
        for required in (
            "--workflow migration-readiness.yml",
            '--commit "${GITHUB_SHA}"',
            "migration-readiness-pre-release-${{ github.sha }}",
            "--tier pre-release",
            '--revision "${GITHUB_SHA}"',
            "--require-success",
            "needs: [validate, semver, migration-readiness-evidence]",
        ):
            self.assertIn(required, release)


if __name__ == "__main__":
    unittest.main()
