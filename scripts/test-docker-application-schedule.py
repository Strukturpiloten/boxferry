#!/usr/bin/env python3
"""Independent Forgejo offline schedule expectations; no runtime or renderer."""

from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("docker_application_schedule", ROOT / "scripts/lib/docker-application-schedule.py")
assert SPEC is not None and SPEC.loader is not None
schedule = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(schedule)
CATALOGUE = (ROOT / schedule.topology.CATALOGUE_PATH).read_bytes()
AUTHORED = json.loads(CATALOGUE)["applications"]["forgejo"]


def encode(value):
    return (json.dumps(value, separators=(",", ":")) + "\n").encode()


def request(path, body):
    return {"method": "POST", "path": "/v1.56/" + path, "body": body}


class ForgejoSchedule(unittest.TestCase):
    def setUp(self):
        self.profile = {"kind": "target", "build": {"kind": "upstream"}, "engine_release": "29.8.1",
                        "advertised_api_version": "1.56", "acquisition_api_version": "1.49",
                        "rendering_api_version": "1.56", "daemon_mode": "rootless", "evidence_sha256": "ab" * 32}
        self.images = {"forgejo": "registry.invalid/offline/forgejo:reviewed",
                       "postgres": "registry.invalid/offline/postgres:reviewed"}
        labels = {"io.boxferry.live-run": "offline-run", "io.boxferry.application": "unit-forgejo"}
        # Literal native requests and independent ordering/edge assertions below
        # never come from a renderer, generated plan, or the expectation catalogue.
        self.plan = {"schema_version": 1, "context": copy.deepcopy(self.profile), "requests": [
            request("networks/create", {"Name": "unit-forge-backend", "Driver": "bridge", "Internal": True, "Labels": labels}),
            request("volumes/create", {"Name": "unit-forge-db", "Labels": labels}),
            request("volumes/create", {"Name": "unit-forge-data", "Labels": labels}),
            request("containers/create?name=unit-forge-db", {
                "Image": self.images["postgres"], "Labels": labels,
                "HostConfig": {"NetworkMode": "unit-forge-backend", "Mounts": [
                    {"Type": "volume", "Source": "unit-forge-db", "Target": "/var/lib/postgresql/data", "ReadOnly": False}]},
                "NetworkingConfig": {"EndpointsConfig": {"unit-forge-backend": {"Aliases": ["db"]}}}}),
            request("containers/create?name=unit-forge-app", {
                "Image": self.images["forgejo"], "Labels": labels,
                "ExposedPorts": {"3000/tcp": {}, "2222/tcp": {}},
                "HostConfig": {"NetworkMode": "unit-forge-backend", "Mounts": [
                    {"Type": "volume", "Source": "unit-forge-data", "Target": "/var/lib/gitea", "ReadOnly": False}],
                    "PortBindings": {"3000/tcp": [{"HostIp": "127.0.0.1", "HostPort": "13000"}],
                                     "2222/tcp": [{"HostIp": "127.0.0.1", "HostPort": "12222"}]}},
                "NetworkingConfig": {"EndpointsConfig": {"unit-forge-backend": {"Aliases": ["forgejo"]}}}}),
            request("networks/unit-shared-edge/connect", {"Container": "unit-forge-app", "EndpointConfig": {"Aliases": ["forgejo"]}})
        ], "prerequisites": [{"kind": "network", "reference": "18446744073709551615",
                             "identity": "unit-shared-edge", "expected_driver": "bridge"}]}
        self.decision = {"service": "forgejo", "service_runtime_name": "unit-forge-app",
                         "dependency": "db", "dependency_runtime_name": "unit-forge-db",
                         "condition": "healthy", "condition_explicit": True,
                         "required": True, "required_explicit": True, "restart": False, "restart_explicit": True,
                         "fidelity": "approximate", "native_engine_field": False,
                         "provenance": {key: ["source_document"] for key in ("reference", "condition", "required", "restart")}}

    def admission(self, plan_bytes):
        return {"schema_version": 1, "kind": "boxferry-docker-application-offline-admission",
                "evidence_kind": "offline-contract-prerequisite", "source_kind": "authored-compose",
                "native_execution": False, "application": "forgejo", "lane": "upstream-rootless",
                "prefix": "unit", "run_id": "offline-run", "fixture_root": "/tmp/offline-fixture",
                "docker_plan_sha256": hashlib.sha256(plan_bytes).hexdigest(),
                "expectations_sha256": hashlib.sha256(CATALOGUE).hexdigest(),
                "source_sha256": schedule.topology.source_digest(AUTHORED["sources"]),
                **{key: copy.deepcopy(AUTHORED[key]) for key in
                   ("required_checks", "dependencies", "excluded_peers", "shared_services")},
                "runtime_evidence": "unmeasured", "budget_measurements": None}

    def sidecar(self, plan_bytes):
        return {"schema_version": 1, "kind": "boxferry-docker-dependency-decisions",
                "docker_plan_sha256": hashlib.sha256(plan_bytes).hexdigest(), "native_execution": False,
                "decisions": [copy.deepcopy(self.decision)]}

    def check(self, *, plan=None, admission=None, sidecar=None, **options):
        plan_bytes = encode(self.plan) if plan is None else plan
        admission_bytes = encode(self.admission(plan_bytes)) if admission is None else admission
        sidecar_bytes = encode(self.sidecar(plan_bytes)) if sidecar is None else sidecar
        arguments = {"catalogue_bytes": CATALOGUE, "application": "forgejo", "lane": "upstream-rootless",
                     "profile": self.profile, "image_aliases": self.images, "prefix": "unit",
                     "run_id": "offline-run", "fixture_root": "/tmp/offline-fixture"}
        arguments.update(options)
        return schedule.validate_schedule(plan_bytes, admission_bytes, sidecar_bytes, **arguments)

    def test_complete_authored_pair_has_closed_non_admitting_result_and_explicit_order(self):
        result = self.check()
        self.assertEqual(set(result), {"schema_version", "kind", "evidence_kind", "application", "lane",
            "native_execution", "replay_authority", "native_admission", "runtime_evidence", "budget_measurements",
            "docker_plan_sha256", "admission_sha256", "dependency_sidecar_sha256", "expectations_sha256", "source_sha256",
            "external_prerequisites", "native_request_order", "review_operations", "service_review_layers", "dependency_decisions"})
        self.assertEqual(result["schema_version"], 1)
        self.assertEqual(result["kind"], "boxferry-docker-forgejo-offline-schedule")
        self.assertEqual(result["evidence_kind"], "offline-contract-prerequisite")
        for flag in ("native_execution", "replay_authority", "native_admission"):
            self.assertIs(result[flag], False)
        self.assertEqual(result["runtime_evidence"], "unmeasured")
        self.assertIsNone(result["budget_measurements"])
        self.assertEqual(result["external_prerequisites"], [{"kind": "network", "reference": "18446744073709551615",
                         "identity": "unit-shared-edge", "expected_driver": "bridge"}])
        self.assertEqual(result["native_request_order"], [0, 1, 2, 3, 4, 5])
        self.assertEqual(result["review_operations"], [
            {"kind": "network", "request_index": 0, "identity": "unit-forge-backend"},
            {"kind": "volume", "request_index": 2, "identity": "unit-forge-data"},
            {"kind": "volume", "request_index": 1, "identity": "unit-forge-db"},
            {"kind": "container", "request_index": 3, "identity": "unit-forge-db", "service": "db"},
            {"kind": "container", "request_index": 4, "identity": "unit-forge-app", "service": "forgejo"},
            {"kind": "attachment", "request_index": 5, "identity": "unit-shared-edge", "service": "forgejo"}])
        self.assertEqual(result["service_review_layers"], [["db"], ["forgejo"]])
        edge = result["dependency_decisions"][0]
        self.assertEqual((edge["service"], edge["dependency"], edge["condition"], edge["required"], edge["restart"]),
                         ("forgejo", "db", "healthy", True, False))

    def test_reordered_valid_native_requests_keep_dependency_review_order_not_fabricated_start_requests(self):
        self.plan["requests"][3:5] = reversed(self.plan["requests"][3:5])
        result = self.check()
        self.assertEqual(result["native_request_order"], [0, 1, 2, 3, 4, 5])
        self.assertEqual([row["request_index"] for row in result["review_operations"]], [0, 2, 1, 4, 3, 5])
        self.assertEqual(result["service_review_layers"], [["db"], ["forgejo"]])

    def test_three_exact_raw_byte_hashes_are_not_reserialized(self):
        raw = encode(self.plan)
        admission = encode(self.admission(raw)) + b" "
        sidecar = encode(self.sidecar(raw)) + b"\t"
        result = self.check(plan=raw, admission=admission, sidecar=sidecar)
        for field, value in [("docker_plan_sha256", raw), ("admission_sha256", admission),
                             ("dependency_sidecar_sha256", sidecar)]:
            self.assertEqual(result[field], hashlib.sha256(value).hexdigest())
        self.assertNotEqual(result["admission_sha256"], hashlib.sha256(admission.rstrip()).hexdigest())
        self.assertNotEqual(result["dependency_sidecar_sha256"], hashlib.sha256(sidecar.rstrip()).hexdigest())

    def test_whitespace_only_or_swapped_plan_breaks_each_exact_pair_binding(self):
        raw = encode(self.plan)
        for changed in (raw + b" ", raw.replace(b"unit-forge-db", b"unit-other-db")):
            with self.subTest(changed=changed[-12:]), self.assertRaises(schedule.topology.ExpectationError):
                self.check(plan=changed, admission=encode(self.admission(raw)))
            with self.subTest(sidecar=changed[-12:]), self.assertRaises(schedule.topology.ExpectationError):
                self.check(plan=changed, sidecar=encode(self.sidecar(raw)))

    def test_missing_dependency_sidecar_or_edge_is_rejected(self):
        raw = encode(self.plan)
        with self.assertRaisesRegex(schedule.topology.ExpectationError, "missing"):
            schedule.validate_schedule(raw, encode(self.admission(raw)), None, catalogue_bytes=CATALOGUE,
                application="forgejo", lane="upstream-rootless", profile=self.profile, image_aliases=self.images,
                prefix="unit", run_id="offline-run", fixture_root="/tmp/offline-fixture")
        sidecar = self.sidecar(raw)
        sidecar["decisions"] = []
        with self.assertRaises(schedule.topology.ExpectationError):
            self.check(sidecar=encode(sidecar))

    def test_extra_duplicate_unknown_self_reversed_and_cyclic_edges_fail(self):
        for mutation in ("unknown", "duplicate", "self", "reversed", "cycle"):
            with self.subTest(mutation=mutation):
                sidecar = self.sidecar(encode(self.plan))
                edge = sidecar["decisions"][0]
                if mutation == "unknown":
                    edge["dependency"] = "peer-app"
                elif mutation == "duplicate":
                    sidecar["decisions"].append(copy.deepcopy(edge))
                elif mutation == "self":
                    edge["dependency"], edge["dependency_runtime_name"] = "forgejo", "unit-forge-app"
                else:
                    reverse = copy.deepcopy(edge)
                    reverse.update(service="db", service_runtime_name="unit-forge-db",
                                   dependency="forgejo", dependency_runtime_name="unit-forge-app")
                    sidecar["decisions"] = [reverse] if mutation == "reversed" else [edge, reverse]
                with self.assertRaisesRegex(schedule.topology.ExpectationError, "cycle" if mutation == "cycle" else "."):
                    self.check(sidecar=encode(sidecar))

    def test_edge_flags_conditions_runtime_identity_and_provenance_are_closed(self):
        for field, bad in [("condition", "started"), ("required", False), ("restart", True),
                           ("condition_explicit", False), ("required", 1), ("restart_explicit", 0),
                           ("service_runtime_name", "unit-forge-db"), ("dependency_runtime_name", "unit-private"),
                           ("fidelity", "exact"), ("native_engine_field", True), ("condition", [])]:
            with self.subTest(field=field, bad=bad):
                self.decision[field] = bad
                with self.assertRaises(schedule.topology.ExpectationError):
                    self.check()
                self.setUp()
        self.decision["provenance"]["reference"] = ["private-source-canary"]
        with self.assertRaises(schedule.topology.ExpectationError) as caught:
            self.check()
        self.assertNotIn("private-source-canary", str(caught.exception))

    def test_missing_extra_swapped_or_unknown_native_operations_fail(self):
        original = copy.deepcopy(self.plan)
        for index in range(6):
            with self.subTest(missing=index):
                self.plan = copy.deepcopy(original)
                self.plan["requests"].pop(index)
                with self.assertRaises(schedule.topology.ExpectationError):
                    self.check()
        for extra in (request("containers/private/start", {}), request("volumes/create", {"Name": "private-extra"}),
                      copy.deepcopy(original["requests"][0])):
            with self.subTest(extra=extra["path"]):
                self.plan = copy.deepcopy(original)
                self.plan["requests"].append(extra)
                with self.assertRaises(schedule.topology.ExpectationError):
                    self.check()
        self.plan = copy.deepcopy(original)
        self.plan["requests"][1], self.plan["requests"][3] = self.plan["requests"][3], self.plan["requests"][1]
        with self.assertRaises(schedule.topology.ExpectationError):
            self.check()

    def test_external_edge_omission_extra_or_bad_decimal_reference_is_rejected(self):
        original = copy.deepcopy(self.plan)
        for prerequisite in ([], original["prerequisites"] * 2,
                             [{**original["prerequisites"][0], "identity": "unit-extra-edge"}]):
            self.plan = copy.deepcopy(original)
            self.plan["prerequisites"] = prerequisite
            with self.assertRaises(schedule.topology.ExpectationError):
                self.check()
        for reference in ("18446744073709551616", "01", 1):
            self.plan = copy.deepcopy(original)
            self.plan["prerequisites"][0]["reference"] = reference
            with self.assertRaises(schedule.topology.ExpectationError):
                self.check()

    def test_independent_identity_and_lane_boundaries_fail(self):
        for override in ({"application": "nextcloud"}, {"lane": "debian11-rootless"}, {"lane": "upstream-rootful"},
                         {"prefix": "other"}, {"run_id": "other"}, {"fixture_root": "/tmp/other"},
                         {"profile": {**self.profile, "engine_release": "29.8.2"}},
                         {"image_aliases": {**self.images, "postgres": "registry.invalid/other:reviewed"}}):
            with self.subTest(override=override), self.assertRaises(schedule.topology.ExpectationError):
                self.check(**override)

    def test_admission_cannot_change_byte_binding_or_claim_runtime_success(self):
        for field, bad in [("docker_plan_sha256", "00" * 32), ("expectations_sha256", "00" * 32),
                           ("source_sha256", "00" * 32), ("native_execution", True),
                           ("runtime_evidence", "success"), ("budget_measurements", {})]:
            admission = self.admission(encode(self.plan))
            admission[field] = bad
            with self.subTest(field=field), self.assertRaises(schedule.topology.ExpectationError):
                self.check(admission=encode(admission))

    def test_sidecar_schema_unknown_fields_boolean_versions_and_duplicate_keys_fail(self):
        for field, bad in [("schema_version", 2), ("schema_version", True), ("kind", "other"),
                           ("native_execution", True), ("docker_plan_sha256", "00" * 32), ("private-extra", "value")]:
            sidecar = self.sidecar(encode(self.plan))
            sidecar[field] = bad
            with self.subTest(field=field), self.assertRaises(schedule.topology.ExpectationError):
                self.check(sidecar=encode(sidecar))
        raw = encode(self.sidecar(encode(self.plan))).replace(b'"schema_version":1', b'"schema_version":1,"schema_version":1')
        with self.assertRaises(schedule.topology.ExpectationError):
            self.check(sidecar=raw)

    def test_implicit_required_and_restart_defaults_are_retained_as_decisions_not_native_fields(self):
        self.decision["required_explicit"] = False
        self.decision["restart_explicit"] = False
        decision = self.check()["dependency_decisions"][0]
        self.assertIs(decision["required"], True)
        self.assertIs(decision["restart"], False)
        self.assertIs(decision["native_engine_field"], False)

    def test_every_json_artifact_uses_shared_strict_size_depth_and_numeric_bounds(self):
        for key in ("plan", "admission", "sidecar", "catalogue_bytes"):
            for raw in (b"", b"{} trailing", b'{"x":NaN}', b'{"x":1,"x":2}', b"[" * 34 + b"0" + b"]" * 34,
                        b" " * (schedule.topology.LIMIT + 1)):
                with self.subTest(key=key, raw=raw[:16]), self.assertRaises(schedule.topology.ExpectationError):
                    self.check(**{key: raw})

    def test_rootful_forgejo_remains_offline_when_profile_and_admission_agree(self):
        self.profile["daemon_mode"] = "rootful"
        self.plan["context"] = copy.deepcopy(self.profile)
        admission = self.admission(encode(self.plan))
        admission["lane"] = "upstream-rootful"
        result = self.check(admission=encode(admission), lane="upstream-rootful")
        self.assertEqual(result["lane"], "upstream-rootful")
        self.assertIs(result["native_admission"], False)

    def test_cli_checks_sources_before_reading_artifacts_and_prints_value_free_errors(self):
        arguments = ["schedule", "--plan", "private-plan-canary", "--admission", "private-admission-canary",
                     "--sidecar", "private-sidecar-canary", "--profile", "private-profile-canary",
                     "--image-aliases", "private-images-canary", "--lane", "upstream-rootless", "--prefix", "unit",
                     "--run-id", "offline-run", "--fixture-root", "/tmp/offline-fixture"]
        with mock.patch.object(sys, "argv", arguments), mock.patch.object(schedule.topology, "check_sources",
                side_effect=schedule.topology.ExpectationError("reviewed source bytes differ")) as checker, \
                mock.patch.object(schedule, "read_document", return_value=CATALOGUE) as opened, \
                mock.patch("sys.stderr") as stderr:
            self.assertEqual(schedule.main(), 1)
            checker.assert_called_once()
            self.assertEqual(opened.call_count, 1)
            self.assertNotIn("canary", "".join(str(call) for call in stderr.write.call_args_list))

    def test_nonregular_symlink_and_oversized_files_fail_before_unbounded_read(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            fifo = root / "pipe"
            os.mkfifo(fifo)
            with self.assertRaises(schedule.topology.ExpectationError):
                schedule.read_document(fifo)
            target = root / "input.json"
            target.write_bytes(b"{}")
            link = root / "link.json"
            link.symlink_to(target)
            with self.assertRaises(OSError):
                schedule.read_document(link)
            target.write_bytes(b" " * (schedule.topology.LIMIT + 1))
            with self.assertRaises(schedule.topology.ExpectationError):
                schedule.read_document(target)

    def test_cli_success_and_missing_input_are_offline_without_runtime_programs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            documents = {"plan": self.plan, "admission": self.admission(encode(self.plan)),
                         "sidecar": self.sidecar(encode(self.plan)), "profile": self.profile, "image-aliases": self.images}
            arguments = [sys.executable, str(ROOT / "scripts/lib/docker-application-schedule.py")]
            for name, document in documents.items():
                path = root / f"{name}.json"
                path.write_bytes(encode(document))
                arguments.extend([f"--{name}", str(path)])
            arguments.extend(["--lane", "upstream-rootless", "--prefix", "unit", "--run-id", "offline-run",
                              "--fixture-root", "/tmp/offline-fixture"])
            result = subprocess.run(arguments, capture_output=True, check=False, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            self.assertEqual(json.loads(result.stdout)["service_review_layers"], [["db"], ["forgejo"]])
            self.assertEqual(result.stderr, b"")
            (root / "plan.json").unlink()
            result = subprocess.run(arguments, capture_output=True, check=False, timeout=10)
            self.assertEqual(result.returncode, 1)
            self.assertEqual(result.stdout, b"")
            self.assertNotIn(directory.encode(), result.stderr)


if __name__ == "__main__":
    unittest.main()
