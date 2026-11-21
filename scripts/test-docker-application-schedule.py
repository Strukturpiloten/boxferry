#!/usr/bin/env python3
"""Independent Forgejo/Nextcloud/Immich offline expectations; no runtime or renderer."""

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


class NextcloudSchedule(unittest.TestCase):
    def setUp(self):
        self.profile = {"kind": "target", "build": {"kind": "upstream"}, "engine_release": "29.8.1",
                        "advertised_api_version": "1.56", "acquisition_api_version": "1.49",
                        "rendering_api_version": "1.56", "daemon_mode": "rootless", "evidence_sha256": "ab" * 32}
        self.images = {role: f"registry.invalid/offline/{role}:reviewed"
                       for role in ("postgres", "redis", "nextcloud", "nginx")}
        labels = {"io.boxferry.live-run": "offline-run", "io.boxferry.application": "unit-cloud"}
        # Native topology, commands and dependency decisions are literal expectations,
        # independent of the catalogue, renderer and implementation edge constants.
        self.plan = {"schema_version": 1, "context": copy.deepcopy(self.profile), "requests": [
            request("networks/create", {"Name": "unit-cloud-backend", "Driver": "bridge", "Internal": True, "Labels": labels}),
            request("volumes/create", {"Name": "unit-cloud-database", "Labels": labels}),
            request("volumes/create", {"Name": "unit-cloud-redis", "Labels": labels}),
            request("volumes/create", {"Name": "unit-cloud-nextcloud", "Labels": labels}),
            request("containers/create?name=unit-cloud-db", {
                "Image": self.images["postgres"], "Labels": labels,
                "HostConfig": {"NetworkMode": "unit-cloud-backend", "Mounts": [
                    {"Type": "volume", "Source": "unit-cloud-database", "Target": "/var/lib/postgresql/data", "ReadOnly": False}]},
                "NetworkingConfig": {"EndpointsConfig": {"unit-cloud-backend": {"Aliases": ["db"]}}}}),
            request("containers/create?name=unit-cloud-cache", {
                "Image": self.images["redis"], "Labels": labels,
                "HostConfig": {"NetworkMode": "unit-cloud-backend", "Mounts": [
                    {"Type": "volume", "Source": "unit-cloud-redis", "Target": "/data", "ReadOnly": False},
                    {"Type": "bind", "Source": "/tmp/offline-fixture/redis.docker.conf", "Target": "/usr/local/etc/redis/redis.conf", "ReadOnly": True}]},
                "NetworkingConfig": {"EndpointsConfig": {"unit-cloud-backend": {"Aliases": ["cache"]}}}}),
            request("containers/create?name=unit-cloud-app", {
                "Image": self.images["nextcloud"], "Labels": labels,
                "HostConfig": {"NetworkMode": "unit-cloud-backend", "Mounts": [
                    {"Type": "volume", "Source": "unit-cloud-nextcloud", "Target": "/var/www/html", "ReadOnly": False}]},
                "NetworkingConfig": {"EndpointsConfig": {"unit-cloud-backend": {"Aliases": ["app"]}}}}),
            request("containers/create?name=unit-cloud-init", {
                "Image": self.images["nextcloud"], "Labels": labels,
                "Cmd": ["php", "/var/www/html/occ", "status"], "User": "www-data",
                "HostConfig": {"NetworkMode": "unit-cloud-backend", "Mounts": [
                    {"Type": "volume", "Source": "unit-cloud-nextcloud", "Target": "/var/www/html", "ReadOnly": False}]},
                "NetworkingConfig": {"EndpointsConfig": {"unit-cloud-backend": {}}}}),
            request("containers/create?name=unit-cloud-cron", {
                "Image": self.images["nextcloud"], "Labels": labels, "Cmd": ["/cron.sh"],
                "HostConfig": {"NetworkMode": "unit-cloud-backend", "Mounts": [
                    {"Type": "volume", "Source": "unit-cloud-nextcloud", "Target": "/var/www/html", "ReadOnly": False}]},
                "NetworkingConfig": {"EndpointsConfig": {"unit-cloud-backend": {}}}}),
            request("containers/create?name=unit-cloud-frontend", {
                "Image": self.images["nginx"], "Labels": labels,
                "HostConfig": {"NetworkMode": "unit-cloud-backend", "Mounts": [
                    {"Type": "volume", "Source": "unit-cloud-nextcloud", "Target": "/var/www/html", "ReadOnly": True},
                    {"Type": "bind", "Source": "/tmp/offline-fixture/frontend.conf", "Target": "/etc/nginx/conf.d/default.conf", "ReadOnly": True}]},
                "NetworkingConfig": {"EndpointsConfig": {"unit-cloud-backend": {"Aliases": ["frontend"]}}}}),
            request("networks/unit-shared-edge/connect", {"Container": "unit-cloud-frontend", "EndpointConfig": {"Aliases": ["frontend"]}})
        ], "prerequisites": [{"kind": "network", "reference": "18446744073709551615",
                             "identity": "unit-shared-edge", "expected_driver": "bridge"}]}
        self.plan["requests"][5]["body"].update(
            Cmd=["redis-server", "/usr/local/etc/redis/redis.conf"],
            Env=["REDISCLI_AUTH=boxferry-public-cache-canary"],
            Healthcheck={"Test": ["CMD", "redis-cli", "-e", "ping"], "Interval": 2_000_000_000,
                         "Timeout": 2_000_000_000, "Retries": 60})
        for index in (6, 7, 8):
            self.plan["requests"][index]["body"]["Env"] = [
                "REDIS_HOST=cache", "REDIS_HOST_PASSWORD=boxferry-public-cache-canary"]
        self.edges = [("app", "db", "healthy"), ("app", "cache", "healthy"), ("init", "app", "healthy"),
                      ("cron", "init", "completed_successfully"), ("frontend", "app", "healthy")]
        self.decisions = [{"service": service, "service_runtime_name": f"unit-cloud-{service}",
                           "dependency": dependency, "dependency_runtime_name": f"unit-cloud-{dependency}",
                           "condition": condition, "condition_explicit": True, "required": True,
                           "required_explicit": False, "restart": False, "restart_explicit": False,
                           "fidelity": "approximate", "native_engine_field": False,
                           "provenance": {"reference": ["source_document"], "condition": ["source_document"],
                                          "required": [], "restart": []}}
                          for service, dependency, condition in self.edges]

    def admission(self, plan_bytes):
        authored = json.loads(CATALOGUE)["applications"]["nextcloud"]
        return {"schema_version": 1, "kind": "boxferry-docker-application-offline-admission",
                "evidence_kind": "offline-contract-prerequisite", "source_kind": "authored-compose",
                "native_execution": False, "application": "nextcloud", "lane": "upstream-rootless",
                "prefix": "unit", "run_id": "offline-run", "fixture_root": "/tmp/offline-fixture",
                "docker_plan_sha256": hashlib.sha256(plan_bytes).hexdigest(),
                "expectations_sha256": hashlib.sha256(CATALOGUE).hexdigest(),
                "source_sha256": schedule.topology.source_digest(authored["sources"]),
                **{key: copy.deepcopy(authored[key]) for key in
                   ("required_checks", "dependencies", "excluded_peers", "shared_services")},
                "runtime_evidence": "unmeasured", "budget_measurements": None}

    def sidecar(self, plan_bytes):
        return {"schema_version": 1, "kind": "boxferry-docker-dependency-decisions",
                "docker_plan_sha256": hashlib.sha256(plan_bytes).hexdigest(), "native_execution": False,
                "decisions": copy.deepcopy(self.decisions)}

    def check(self, *, plan=None, admission=None, sidecar=None, **options):
        plan_bytes = encode(self.plan) if plan is None else plan
        arguments = {"catalogue_bytes": CATALOGUE, "application": "nextcloud", "lane": "upstream-rootless",
                     "profile": self.profile, "image_aliases": self.images, "prefix": "unit",
                     "run_id": "offline-run", "fixture_root": "/tmp/offline-fixture",
                     "redis_config_bytes": b"dir /data\nappendonly yes\nrequirepass boxferry-public-cache-canary\n"}
        arguments.update(options)
        return schedule.validate_schedule(plan_bytes,
            encode(self.admission(plan_bytes)) if admission is None else admission,
            encode(self.sidecar(plan_bytes)) if sidecar is None else sidecar, **arguments)

    def test_literal_six_service_plan_has_closed_non_admitting_intent(self):
        result = self.check()
        self.assertEqual(set(result), {"schema_version", "kind", "evidence_kind", "application", "lane",
            "native_execution", "replay_authority", "native_admission", "runtime_evidence", "budget_measurements",
            "docker_plan_sha256", "admission_sha256", "dependency_sidecar_sha256", "expectations_sha256", "source_sha256",
            "external_prerequisites", "native_request_order", "review_operations", "service_review_layers",
            "dependency_decisions", "authored_checks", "shared_service_expectations"})
        self.assertEqual(result["kind"], "boxferry-docker-nextcloud-offline-schedule")
        self.assertEqual(result["evidence_kind"], "offline-contract-prerequisite")
        self.assertEqual(result["application"], "nextcloud")
        for field in ("native_execution", "replay_authority", "native_admission"):
            self.assertIs(result[field], False)
        self.assertEqual(result["runtime_evidence"], "unmeasured")
        self.assertIsNone(result["budget_measurements"])
        self.assertEqual(result["authored_checks"], ["init.command", "init.user", "cron.command", "cache.command",
            "cache.config", "cache.authentication", "cache.health", "redis.client-authentication"])
        self.assertEqual(result["shared_service_expectations"], [
            {"identity": "unit-shared-proxy", "ownership": "shared", "runtime_evidence": "unmeasured"}])
        self.assertEqual(result["external_prerequisites"], self.plan["prerequisites"])
        self.assertEqual(result["service_review_layers"], [["cache", "db"], ["app"], ["frontend", "init"], ["cron"]])
        self.assertEqual([(row["service"], row["dependency"], row["condition"])
                          for row in result["dependency_decisions"]], self.edges)
        self.assertEqual(result["dependency_decisions"], self.decisions)
        self.assertEqual(result["native_request_order"], list(range(11)))
        self.assertEqual(result["review_operations"], [
            {"kind": "network", "request_index": 0, "identity": "unit-cloud-backend"},
            {"kind": "volume", "request_index": 1, "identity": "unit-cloud-database"},
            {"kind": "volume", "request_index": 3, "identity": "unit-cloud-nextcloud"},
            {"kind": "volume", "request_index": 2, "identity": "unit-cloud-redis"},
            {"kind": "container", "request_index": 5, "identity": "unit-cloud-cache", "service": "cache"},
            {"kind": "container", "request_index": 4, "identity": "unit-cloud-db", "service": "db"},
            {"kind": "container", "request_index": 6, "identity": "unit-cloud-app", "service": "app"},
            {"kind": "container", "request_index": 9, "identity": "unit-cloud-frontend", "service": "frontend"},
            {"kind": "container", "request_index": 7, "identity": "unit-cloud-init", "service": "init"},
            {"kind": "container", "request_index": 8, "identity": "unit-cloud-cron", "service": "cron"},
            {"kind": "attachment", "request_index": 10, "identity": "unit-shared-edge", "service": "frontend"}])
        serialized = json.dumps(result)
        for private in ("/var/www/html/occ", "/cron.sh", "www-data", "frontend.conf", "Cmd", "HostConfig", "Env"):
            self.assertNotIn(private, serialized)

    def test_reordered_requests_and_decisions_retain_every_original_index_once(self):
        self.plan["requests"][4:10] = reversed(self.plan["requests"][4:10])
        self.decisions.reverse()
        result = self.check()
        self.assertEqual(result["service_review_layers"], [["cache", "db"], ["app"], ["frontend", "init"], ["cron"]])
        self.assertEqual([row["request_index"] for row in result["review_operations"]], [0, 1, 3, 2, 8, 9, 7, 4, 6, 5, 10])
        self.assertEqual(sorted(row["request_index"] for row in result["review_operations"]), list(range(11)))

    def test_rootful_profile_requires_matching_plan_admission_and_lane(self):
        self.profile["daemon_mode"] = "rootful"
        self.plan["context"] = copy.deepcopy(self.profile)
        admission = self.admission(encode(self.plan))
        with self.assertRaises(schedule.topology.ExpectationError):
            self.check(admission=encode(admission), lane="upstream-rootful")
        admission["lane"] = "upstream-rootful"
        result = self.check(admission=encode(admission), lane="upstream-rootful")
        self.assertEqual(result["lane"], "upstream-rootful")
        self.assertIs(result["native_admission"], False)

    def test_missing_extra_duplicate_self_reversed_and_cyclic_edges_fail(self):
        raw = encode(self.plan)
        for index in range(5):
            sidecar = self.sidecar(raw)
            sidecar["decisions"].pop(index)
            with self.subTest(missing=index), self.assertRaises(schedule.topology.ExpectationError):
                self.check(sidecar=encode(sidecar))
        for mutation in ("extra", "duplicate", "duplicate-with-different-provenance", "self", "reversed", "cycle"):
            sidecar = self.sidecar(raw)
            edge = sidecar["decisions"][0]
            if mutation in {"extra", "duplicate", "duplicate-with-different-provenance"}:
                extra = copy.deepcopy(edge)
                if mutation == "extra":
                    extra.update(service="frontend", service_runtime_name="unit-cloud-frontend")
                elif mutation == "duplicate-with-different-provenance":
                    extra["provenance"]["reference"] = ["conversion_decision"]
                sidecar["decisions"].append(extra)
            elif mutation == "self":
                edge.update(dependency="app", dependency_runtime_name="unit-cloud-app")
            else:
                reverse = copy.deepcopy(edge)
                reverse.update(service="db", service_runtime_name="unit-cloud-db",
                               dependency="app", dependency_runtime_name="unit-cloud-app")
                if mutation == "reversed":
                    sidecar["decisions"][0] = reverse
                else:
                    sidecar["decisions"].append(reverse)
            with self.subTest(mutation=mutation), self.assertRaises(schedule.topology.ExpectationError):
                self.check(sidecar=encode(sidecar))

    def test_each_edge_condition_flags_identity_and_provenance_are_checked(self):
        for index in range(5):
            for field, bad in [("condition", "started"), ("required", False), ("restart", True),
                               ("condition_explicit", False), ("required_explicit", True), ("restart_explicit", True),
                               ("required", 1), ("service_runtime_name", "private-identity-canary"),
                               ("dependency_runtime_name", "private-identity-canary"), ("service", "second-app"),
                               ("fidelity", "exact"), ("native_engine_field", True),
                               ("provenance", {"reference": ["private-provenance-canary"], "condition": [], "required": [], "restart": []})]:
                sidecar = self.sidecar(encode(self.plan))
                sidecar["decisions"][index][field] = bad
                with self.subTest(index=index, field=field), self.assertRaises(schedule.topology.ExpectationError) as caught:
                    self.check(sidecar=encode(sidecar))
                self.assertNotIn("canary", str(caught.exception))
        self.decisions[3]["condition"] = "healthy"
        with self.assertRaises(schedule.topology.ExpectationError):
            self.check()

    def test_authored_fields_missing_null_wrong_type_and_private_changes_fail_without_values(self):
        original = copy.deepcopy(self.plan)
        for index, field, values in [(7, "Cmd", [None, [], "php /var/www/html/occ status", ["private-command-canary"], ["/cron.sh"]]),
                                     (7, "User", [None, [], 0, "root", "private-user-canary"]),
                                     (8, "Cmd", [None, [], "/cron.sh", ["private-command-canary"], ["php", "/var/www/html/occ", "status"]])]:
            for value in ["missing", *values]:
                self.plan = copy.deepcopy(original)
                body = self.plan["requests"][index]["body"]
                if value == "missing":
                    body.pop(field)
                else:
                    body[field] = value
                with self.subTest(index=index, field=field, value=value), self.assertRaises(schedule.topology.ExpectationError) as caught:
                    self.check()
                self.assertNotIn("canary", str(caught.exception))

    def test_redis_authentication_recipe_rejects_independent_mutations(self):
        original = copy.deepcopy(self.plan)
        for field, value in [("Cmd", ["redis-server", "--requirepass", "private-password-canary"]),
                             ("Env", []), ("Env", ["REDISCLI_AUTH=private-password-canary"]),
                             ("Env", ["REDISCLI_AUTH=boxferry-public-cache-canary"] * 2),
                             ("Healthcheck", {"Test": ["CMD", "redis-cli", "ping"]}),
                             ("Healthcheck", {"Test": ["CMD", "redis-cli", "-a", "private-password-canary", "ping"]})]:
            self.plan = copy.deepcopy(original)
            self.plan["requests"][5]["body"][field] = value
            with self.subTest(field=field, value=value), self.assertRaises(schedule.topology.ExpectationError) as caught:
                self.check()
            self.assertNotIn("private-password-canary", str(caught.exception))
        for index in (6, 7, 8):
            self.plan = copy.deepcopy(original)
            self.plan["requests"][index]["body"]["Env"] = ["REDIS_HOST=cache", "REDIS_HOST_PASSWORD=wrong-private-canary"]
            with self.subTest(role=index), self.assertRaises(schedule.topology.ExpectationError):
                self.check()
        self.plan = original
        for config in (None, True, "private-config-canary", b"", b"requirepass ${BF_REDIS_PASSWORD}\n", b"dir /data\nappendonly no\nrequirepass boxferry-public-cache-canary\n",
                       b"dir /data\nappendonly yes\nrequirepass private-password-canary\n"):
            with self.subTest(config=config), self.assertRaises(schedule.topology.ExpectationError) as caught:
                self.check(redis_config_bytes=config)
            self.assertNotIn("private-password-canary", str(caught.exception))
        catalogue = json.loads(CATALOGUE)
        for row in catalogue["applications"]["nextcloud"]["sources"]:
            if row["path"].endswith("redis.docker.conf"):
                row["sha256"] = "00" * 32
        changed_catalogue = encode(catalogue)
        admission = self.admission(encode(self.plan))
        admission["expectations_sha256"] = hashlib.sha256(changed_catalogue).hexdigest()
        admission["source_sha256"] = schedule.topology.source_digest(catalogue["applications"]["nextcloud"]["sources"])
        with self.assertRaisesRegex(schedule.topology.ExpectationError, "config source binding"):
            self.check(catalogue_bytes=changed_catalogue, admission=encode(admission))

    def test_unmeasured_fields_do_not_infer_entrypoint_environment_or_health(self):
        for index in (7, 8):
            self.plan["requests"][index]["body"].update(Entrypoint=["private-entrypoint-canary"],
                Env=["REDIS_HOST=cache", "REDIS_HOST_PASSWORD=boxferry-public-cache-canary", "SECRET=private-env-canary"], Healthcheck={"Test": ["CMD", "private-health-canary"]})
        result = self.check()
        self.assertNotIn("canary", json.dumps(result))
        self.assertEqual(result["authored_checks"], ["init.command", "init.user", "cron.command", "cache.command",
            "cache.config", "cache.authentication", "cache.health", "redis.client-authentication"])

    def test_proxy_peer_and_unsupported_operations_are_rejected_not_filtered(self):
        original = copy.deepcopy(self.plan)
        extras = [request("containers/create?name=unit-shared-proxy", copy.deepcopy(original["requests"][9]["body"])),
                  request("containers/create?name=unit-second-app", copy.deepcopy(original["requests"][9]["body"])),
                  request("networks/unit-shared-edge/connect", {"Container": "unit-shared-proxy", "EndpointConfig": {"Aliases": ["proxy"]}}),
                  request("networks/unit-shared-edge/connect", {"Container": "unit-second-app", "EndpointConfig": {}}),
                  request("containers/unit-cloud-init/start", {}), copy.deepcopy(original["requests"][0])]
        for extra in extras:
            self.plan = copy.deepcopy(original)
            self.plan["requests"].append(extra)
            with self.subTest(path=extra["path"]), self.assertRaises(schedule.topology.ExpectationError):
                self.check()
        for index in range(11):
            self.plan = copy.deepcopy(original)
            self.plan["requests"].pop(index)
            with self.subTest(missing=index), self.assertRaises(schedule.topology.ExpectationError):
                self.check()

    def test_raw_whitespace_bindings_and_context_authority_cannot_drift(self):
        raw = encode(self.plan)
        admission = encode(self.admission(raw)) + b" "
        sidecar = encode(self.sidecar(raw)) + b"\t"
        result = self.check(plan=raw, admission=admission, sidecar=sidecar)
        for field, value in [("docker_plan_sha256", raw), ("admission_sha256", admission), ("dependency_sidecar_sha256", sidecar)]:
            self.assertEqual(result[field], hashlib.sha256(value).hexdigest())
        for stale in ({"admission": admission}, {"sidecar": sidecar}):
            with self.assertRaises(schedule.topology.ExpectationError):
                self.check(plan=raw + b" ", **stale)
        for override in ({"application": "forgejo"}, {"application": "wordpress"}, {"application": "private-app-canary"},
                         {"lane": "debian11-rootless"}, {"lane": "upstream-rootful"}, {"prefix": "other"},
                         {"run_id": "other"}, {"fixture_root": "/tmp/other"},
                         {"profile": {**self.profile, "engine_release": "29.8.2"}},
                         {"profile": {**self.profile, "rendering_api_version": "1.55"}},
                         {"profile": {**self.profile, "build": {"kind": "distribution"}}},
                         {"image_aliases": {**self.images, "nextcloud": "registry.invalid/other:reviewed"}}):
            with self.subTest(override=override), self.assertRaises(schedule.topology.ExpectationError) as caught:
                self.check(**override)
            self.assertNotIn("canary", str(caught.exception))

    def test_admission_sidecar_and_source_drift_or_runtime_claims_fail(self):
        raw = encode(self.plan)
        for field, value in [("source_sha256", "00" * 32), ("expectations_sha256", "00" * 32),
                             ("native_execution", True), ("runtime_evidence", "success"), ("budget_measurements", {}),
                             ("shared_services", []), ("excluded_peers", [])]:
            admission = self.admission(raw)
            admission[field] = value
            with self.subTest(field=field), self.assertRaises(schedule.topology.ExpectationError):
                self.check(admission=encode(admission))
        for field, value in [("schema_version", True), ("kind", "other"), ("native_execution", True),
                             ("docker_plan_sha256", "00" * 32), ("private-extra", "private-canary")]:
            sidecar = self.sidecar(raw)
            sidecar[field] = value
            with self.subTest(field=field), self.assertRaises(schedule.topology.ExpectationError):
                self.check(sidecar=encode(sidecar))
        with self.assertRaises(schedule.topology.ExpectationError):
            schedule.validate_schedule(raw, encode(self.admission(raw)), None, catalogue_bytes=CATALOGUE,
                application="nextcloud", lane="upstream-rootless", profile=self.profile, image_aliases=self.images,
                prefix="unit", run_id="offline-run", fixture_root="/tmp/offline-fixture")

    def test_each_request_requires_selected_rendering_api_and_profile_bounds(self):
        original = copy.deepcopy(self.plan)
        for index in range(11):
            for api in ("1.55", "1.57"):
                self.plan = copy.deepcopy(original)
                row = self.plan["requests"][index]
                row["path"] = row["path"].replace("/v1.56/", f"/v{api}/")
                with self.subTest(index=index, api=api), self.assertRaises(schedule.topology.ExpectationError):
                    self.check()
        self.plan = original
        for field, bad in [("rendering_api_version", "1.57"), ("acquisition_api_version", "1.57"),
                           ("advertised_api_version", "1.55"), ("rendering_api_version", "v1.56")]:
            profile = {**self.profile, field: bad}
            self.plan["context"] = copy.deepcopy(profile)
            with self.subTest(field=field, bad=bad), self.assertRaises(schedule.topology.ExpectationError):
                self.check(profile=profile)

    def test_cli_sources_precede_artifact_reads_and_rejections_withhold_values(self):
        arguments = ["schedule", "--application", "nextcloud", "--plan", "private-plan-canary",
                     "--admission", "private-admission-canary", "--sidecar", "private-sidecar-canary",
                     "--profile", "private-profile-canary", "--image-aliases", "private-images-canary",
                     "--lane", "upstream-rootless", "--prefix", "unit", "--run-id", "offline-run",
                     "--fixture-root", "/tmp/offline-fixture"]
        with mock.patch.object(sys, "argv", arguments), mock.patch.object(schedule.topology, "check_sources",
                side_effect=schedule.topology.ExpectationError("reviewed source bytes differ")) as checker, \
                mock.patch.object(schedule, "read_document", return_value=CATALOGUE) as opened, mock.patch("sys.stderr") as stderr:
            self.assertEqual(schedule.main(), 1)
            checker.assert_called_once()
            self.assertEqual(opened.call_count, 1)
            self.assertNotIn("canary", "".join(str(call) for call in stderr.write.call_args_list))

    def test_cli_accepts_nextcloud_and_withholds_bad_authored_values(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            documents = {"plan": self.plan, "admission": self.admission(encode(self.plan)),
                         "sidecar": self.sidecar(encode(self.plan)), "profile": self.profile, "image-aliases": self.images}
            arguments = [sys.executable, str(ROOT / "scripts/lib/docker-application-schedule.py"), "--application", "nextcloud"]
            for name, document in documents.items():
                path = root / f"{name}.json"
                path.write_bytes(encode(document))
                arguments.extend([f"--{name}", str(path)])
            arguments.extend(["--lane", "upstream-rootless", "--prefix", "unit", "--run-id", "offline-run",
                              "--fixture-root", "/tmp/offline-fixture"])
            result = subprocess.run(arguments, capture_output=True, check=False, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            self.assertEqual(json.loads(result.stdout)["authored_checks"], ["init.command", "init.user", "cron.command", "cache.command",
            "cache.config", "cache.authentication", "cache.health", "redis.client-authentication"])
            self.assertEqual(result.stderr, b"")
            self.plan["requests"][7]["body"]["Cmd"] = ["private-command-canary"]
            raw = encode(self.plan)
            for name, document in {"plan": self.plan, "admission": self.admission(raw), "sidecar": self.sidecar(raw)}.items():
                (root / f"{name}.json").write_bytes(encode(document))
            result = subprocess.run(arguments, capture_output=True, check=False, timeout=10)
            self.assertEqual(result.returncode, 1)
            self.assertEqual(result.stdout, b"")
            self.assertNotIn(b"canary", result.stderr)
            self.assertNotIn(directory.encode(), result.stderr)


class ImmichSchedule(unittest.TestCase):
    def setUp(self):
        self.profile = {"kind": "target", "build": {"kind": "upstream"}, "engine_release": "29.8.1",
                        "advertised_api_version": "1.56", "acquisition_api_version": "1.49",
                        "rendering_api_version": "1.56", "daemon_mode": "rootless", "evidence_sha256": "ab" * 32}
        self.images = {"postgres": "registry.invalid/offline/immich-postgres:reviewed",
                       "valkey": "registry.invalid/offline/immich-valkey:reviewed",
                       "machine-learning": "registry.invalid/offline/immich-ml:reviewed",
                       "server": "registry.invalid/offline/immich-server:reviewed"}
        labels = {"io.boxferry.live-run": "offline-run", "io.boxferry.application": "unit-immich"}
        # Literal requests and edges below are authored independently of the
        # renderer, scheduler constants and catalogue topology.
        self.plan = {"schema_version": 1, "context": copy.deepcopy(self.profile), "requests": [
            request("networks/create", {"Name": "unit-immich-backend", "Driver": "bridge", "Internal": True, "Labels": labels}),
            request("networks/create", {"Name": "unit-immich-edge", "Driver": "bridge", "Labels": labels}),
            request("volumes/create", {"Name": "unit-immich-library", "Labels": labels}),
            request("volumes/create", {"Name": "unit-immich-model-cache", "Labels": labels}),
            request("volumes/create", {"Name": "unit-immich-pgdata", "Labels": labels}),
            request("volumes/create", {"Name": "unit-immich-redisdata", "Labels": labels}),
            request("containers/create?name=unit-immich-database", {
                "Image": self.images["postgres"], "Labels": labels,
                "HostConfig": {"NetworkMode": "unit-immich-backend", "Mounts": [
                    {"Type": "volume", "Source": "unit-immich-pgdata", "Target": "/var/lib/postgresql/data", "ReadOnly": False}]},
                "NetworkingConfig": {"EndpointsConfig": {"unit-immich-backend": {"Aliases": ["database"]}}}}),
            request("containers/create?name=unit-immich-redis", {
                "Image": self.images["valkey"], "Labels": labels,
                "HostConfig": {"NetworkMode": "unit-immich-backend", "Mounts": [
                    {"Type": "volume", "Source": "unit-immich-redisdata", "Target": "/data", "ReadOnly": False}]},
                "NetworkingConfig": {"EndpointsConfig": {"unit-immich-backend": {"Aliases": ["redis"]}}}}),
            request("containers/create?name=unit-immich-machine-learning", {
                "Image": self.images["machine-learning"], "Labels": labels,
                "HostConfig": {"NetworkMode": "unit-immich-backend", "Mounts": [
                    {"Type": "volume", "Source": "unit-immich-model-cache", "Target": "/cache", "ReadOnly": False},
                    {"Type": "bind", "Source": "/tmp/offline-fixture", "Target": "/fixture", "ReadOnly": False}]},
                "NetworkingConfig": {"EndpointsConfig": {"unit-immich-backend": {"Aliases": ["immich-machine-learning"]}}}}),
            request("containers/create?name=unit-immich-server", {
                "Image": self.images["server"], "Labels": labels, "ExposedPorts": {"2283/tcp": {}},
                "HostConfig": {"NetworkMode": "unit-immich-backend", "Mounts": [
                    {"Type": "volume", "Source": "unit-immich-library", "Target": "/data", "ReadOnly": False}],
                    "PortBindings": {"2283/tcp": [{"HostIp": "127.0.0.1", "HostPort": "18283"}]}},
                "NetworkingConfig": {"EndpointsConfig": {"unit-immich-backend": {}}}}),
            request("networks/unit-immich-edge/connect", {
                "Container": "unit-immich-server", "EndpointConfig": {"Aliases": ["immich-server"]}})
        ], "prerequisites": []}
        runtime_names = {"database": "unit-immich-database", "redis": "unit-immich-redis",
                         "immich-machine-learning": "unit-immich-machine-learning", "immich-server": "unit-immich-server"}
        self.decisions = [{"service": "immich-server", "service_runtime_name": "unit-immich-server",
                           "dependency": dependency, "dependency_runtime_name": runtime_names[dependency],
                           "condition": "healthy", "condition_explicit": True, "required": True,
                           "required_explicit": False, "restart": False, "restart_explicit": False,
                           "fidelity": "approximate", "native_engine_field": False,
                           "provenance": {"reference": ["source_document"], "condition": ["source_document"],
                                          "required": [], "restart": []}}
                          for dependency in ("database", "redis", "immich-machine-learning")]

    def admission(self, plan_bytes):
        authored = json.loads(CATALOGUE)["applications"]["immich"]
        return {"schema_version": 1, "kind": "boxferry-docker-application-offline-admission",
                "evidence_kind": "offline-contract-prerequisite", "source_kind": "authored-compose",
                "native_execution": False, "application": "immich", "lane": "upstream-rootless",
                "prefix": "unit", "run_id": "offline-run", "fixture_root": "/tmp/offline-fixture",
                "docker_plan_sha256": hashlib.sha256(plan_bytes).hexdigest(),
                "expectations_sha256": hashlib.sha256(CATALOGUE).hexdigest(),
                "source_sha256": schedule.topology.source_digest(authored["sources"]),
                **{key: copy.deepcopy(authored[key]) for key in
                   ("required_checks", "dependencies", "excluded_peers", "shared_services")},
                "runtime_evidence": "unmeasured", "budget_measurements": None}

    def sidecar(self, plan_bytes):
        return {"schema_version": 1, "kind": "boxferry-docker-dependency-decisions",
                "docker_plan_sha256": hashlib.sha256(plan_bytes).hexdigest(), "native_execution": False,
                "decisions": copy.deepcopy(self.decisions)}

    def check(self, *, plan=None, admission=None, sidecar=None, **options):
        plan_bytes = encode(self.plan) if plan is None else plan
        arguments = {"catalogue_bytes": CATALOGUE, "application": "immich", "lane": "upstream-rootless",
                     "profile": self.profile, "image_aliases": self.images, "prefix": "unit",
                     "run_id": "offline-run", "fixture_root": "/tmp/offline-fixture"}
        arguments.update(options)
        return schedule.validate_schedule(plan_bytes,
            encode(self.admission(plan_bytes)) if admission is None else admission,
            encode(self.sidecar(plan_bytes)) if sidecar is None else sidecar, **arguments)

    def test_literal_four_service_graph_has_three_healthy_prerequisites_and_no_native_authority(self):
        result = self.check()
        self.assertEqual(set(result), {"schema_version", "kind", "evidence_kind", "application", "lane",
            "native_execution", "replay_authority", "native_admission", "runtime_evidence", "budget_measurements",
            "docker_plan_sha256", "admission_sha256", "dependency_sidecar_sha256", "expectations_sha256", "source_sha256",
            "external_prerequisites", "native_request_order", "review_operations", "service_review_layers", "dependency_decisions"})
        self.assertEqual(result["kind"], "boxferry-docker-immich-offline-schedule")
        self.assertEqual(result["application"], "immich")
        self.assertEqual(result["evidence_kind"], "offline-contract-prerequisite")
        self.assertEqual(result["service_review_layers"], [["database", "immich-machine-learning", "redis"], ["immich-server"]])
        self.assertEqual([(row["service"], row["dependency"], row["condition"], row["required"], row["restart"])
                          for row in result["dependency_decisions"]], [
            ("immich-server", "database", "healthy", True, False),
            ("immich-server", "redis", "healthy", True, False),
            ("immich-server", "immich-machine-learning", "healthy", True, False)])
        for edge in result["dependency_decisions"]:
            self.assertIs(edge["condition_explicit"], True)
            self.assertIs(edge["required_explicit"], False)
            self.assertIs(edge["restart_explicit"], False)
        self.assertEqual(result["native_request_order"], list(range(11)))
        self.assertEqual(result["review_operations"], [
            {"kind": "network", "request_index": 0, "identity": "unit-immich-backend"},
            {"kind": "network", "request_index": 1, "identity": "unit-immich-edge"},
            {"kind": "volume", "request_index": 2, "identity": "unit-immich-library"},
            {"kind": "volume", "request_index": 3, "identity": "unit-immich-model-cache"},
            {"kind": "volume", "request_index": 4, "identity": "unit-immich-pgdata"},
            {"kind": "volume", "request_index": 5, "identity": "unit-immich-redisdata"},
            {"kind": "container", "request_index": 6, "identity": "unit-immich-database", "service": "database"},
            {"kind": "container", "request_index": 8, "identity": "unit-immich-machine-learning", "service": "immich-machine-learning"},
            {"kind": "container", "request_index": 7, "identity": "unit-immich-redis", "service": "redis"},
            {"kind": "container", "request_index": 9, "identity": "unit-immich-server", "service": "immich-server"},
            {"kind": "attachment", "request_index": 10, "identity": "unit-immich-edge", "service": "immich-server"}])
        self.assertEqual(result["external_prerequisites"], [])
        for flag in ("native_execution", "replay_authority", "native_admission"):
            self.assertIs(result[flag], False)
        self.assertEqual(result["runtime_evidence"], "unmeasured")
        self.assertIsNone(result["budget_measurements"])

    def test_reordered_requests_and_edges_keep_each_original_index_and_prerequisite_order(self):
        self.plan["requests"][6:10] = reversed(self.plan["requests"][6:10])
        self.decisions.reverse()
        result = self.check()
        self.assertEqual(result["native_request_order"], list(range(11)))
        self.assertEqual(result["service_review_layers"], [["database", "immich-machine-learning", "redis"], ["immich-server"]])
        self.assertEqual([row["request_index"] for row in result["review_operations"]], [0, 1, 2, 3, 4, 5, 9, 7, 8, 6, 10])
        self.assertEqual(sorted(row["request_index"] for row in result["review_operations"]), list(range(11)))
        self.assertEqual([row["service"] for row in result["review_operations"] if row["kind"] == "container"],
                         ["database", "immich-machine-learning", "redis", "immich-server"])

    def test_missing_extra_duplicate_self_reversed_and_cyclic_edges_are_rejected(self):
        raw = encode(self.plan)
        for index in range(3):
            sidecar = self.sidecar(raw)
            sidecar["decisions"].pop(index)
            with self.subTest(missing=index), self.assertRaises(schedule.topology.ExpectationError):
                self.check(sidecar=encode(sidecar))
        for mutation in ("extra", "duplicate", "duplicate-with-different-provenance", "self", "reversed", "cycle"):
            sidecar = self.sidecar(raw)
            edge = sidecar["decisions"][0]
            extra = copy.deepcopy(edge)
            if mutation == "extra":
                extra.update(service="redis", service_runtime_name="unit-immich-redis")
                sidecar["decisions"].append(extra)
            elif mutation.startswith("duplicate"):
                if mutation == "duplicate-with-different-provenance":
                    extra["provenance"]["reference"] = ["conversion_decision"]
                sidecar["decisions"].append(extra)
            elif mutation == "self":
                edge.update(dependency="immich-server", dependency_runtime_name="unit-immich-server")
            else:
                extra.update(service="database", service_runtime_name="unit-immich-database",
                             dependency="immich-server", dependency_runtime_name="unit-immich-server")
                if mutation == "reversed":
                    sidecar["decisions"][0] = extra
                else:
                    sidecar["decisions"].append(extra)
            with self.subTest(mutation=mutation), self.assertRaisesRegex(
                    schedule.topology.ExpectationError, "cycle" if mutation == "cycle" else "."):
                self.check(sidecar=encode(sidecar))

    def test_each_edge_requires_authored_condition_flags_identity_and_closed_provenance(self):
        for index in range(3):
            for field, bad in [
                ("condition", "started"), ("condition", "completed_successfully"),
                ("condition", "private-condition-canary"), ("required", False), ("restart", True),
                ("condition_explicit", False), ("required_explicit", True), ("restart_explicit", True),
                ("condition_explicit", 1), ("required_explicit", None), ("restart_explicit", "false"),
                ("required", 1), ("restart", 0), ("service", "private-service-canary"),
                ("dependency", "private-service-canary"), ("service_runtime_name", "private-identity-canary"),
                ("dependency_runtime_name", "private-identity-canary"), ("fidelity", "exact"),
                ("native_engine_field", True), ("native_engine_field", 0),
                ("provenance", {"reference": ["private-provenance-canary"], "condition": [], "required": [], "restart": []}),
                ("provenance", {"reference": [], "condition": [], "required": [], "restart": [], "extra": []}),
                ("provenance", {"reference": [], "condition": "source_document", "required": [], "restart": []}),
                ("provenance", {"reference": [], "condition": [], "required": [True], "restart": []}),
                ("provenance", {"reference": [], "condition": [], "required": [], "restart": ["source_document"] * 33})]:
                sidecar = self.sidecar(encode(self.plan))
                sidecar["decisions"][index][field] = bad
                with self.subTest(index=index, field=field, bad=bad), self.assertRaises(schedule.topology.ExpectationError) as caught:
                    self.check(sidecar=encode(sidecar))
                self.assertNotIn("canary", str(caught.exception))

    def test_missing_extra_duplicate_or_wrong_service_requests_are_rejected(self):
        original = copy.deepcopy(self.plan)
        for index in range(11):
            self.plan = copy.deepcopy(original)
            self.plan["requests"].pop(index)
            with self.subTest(missing=index), self.assertRaises(schedule.topology.ExpectationError):
                self.check()
        for extra in (copy.deepcopy(original["requests"][6]),
                      request("containers/create?name=unit-other-app", copy.deepcopy(original["requests"][9]["body"])),
                      request("containers/unit-immich-server/start", {})):
            self.plan = copy.deepcopy(original)
            self.plan["requests"].append(extra)
            with self.subTest(extra=extra["path"]), self.assertRaises(schedule.topology.ExpectationError):
                self.check()

    def test_exact_raw_plan_sidecar_admission_and_source_bindings_remain_required(self):
        raw = encode(self.plan)
        admission = encode(self.admission(raw)) + b" "
        sidecar = encode(self.sidecar(raw)) + b"\t"
        result = self.check(plan=raw, admission=admission, sidecar=sidecar)
        for field, value in [("docker_plan_sha256", raw), ("admission_sha256", admission),
                             ("dependency_sidecar_sha256", sidecar)]:
            self.assertEqual(result[field], hashlib.sha256(value).hexdigest())
        reordered = copy.deepcopy(self.plan)
        reordered["requests"][6:10] = reversed(reordered["requests"][6:10])
        for changed in (raw + b" ", encode(reordered)):
            for stale in ({"admission": admission}, {"sidecar": sidecar}):
                with self.subTest(stale=next(iter(stale))), self.assertRaises(schedule.topology.ExpectationError):
                    self.check(plan=changed, **stale)
        for field in ("source_sha256", "expectations_sha256"):
            altered = self.admission(raw)
            altered[field] = "00" * 32
            with self.subTest(field=field), self.assertRaises(schedule.topology.ExpectationError):
                self.check(admission=encode(altered))
        with self.assertRaises(schedule.topology.ExpectationError):
            schedule.validate_schedule(raw, admission, None, catalogue_bytes=CATALOGUE,
                application="immich", lane="upstream-rootless", profile=self.profile, image_aliases=self.images,
                prefix="unit", run_id="offline-run", fixture_root="/tmp/offline-fixture")

    def test_false_native_claims_and_sidecar_shape_drift_fail(self):
        raw = encode(self.plan)
        for field, value in [("native_execution", True), ("runtime_evidence", "success"), ("budget_measurements", {}),
                             ("native_admission", True), ("replay_authority", True), ("dependencies", []),
                             ("required_checks", {}), ("excluded_peers", ["other"]),
                             ("shared_services", [{"identity": "private-service-canary"}])]:
            admission = self.admission(raw)
            admission[field] = value
            with self.subTest(field=field), self.assertRaises(schedule.topology.ExpectationError):
                self.check(admission=encode(admission))
        for field, value in [("schema_version", True), ("kind", "other"), ("native_execution", True),
                             ("docker_plan_sha256", "00" * 32), ("native_admission", True),
                             ("private-extra", "private-canary"), ("decisions", None)]:
            sidecar = self.sidecar(raw)
            sidecar[field] = value
            with self.subTest(field=field), self.assertRaises(schedule.topology.ExpectationError):
                self.check(sidecar=encode(sidecar))

    def test_application_context_and_profile_boundaries_stay_closed(self):
        for application in ("paperless-ngx", "private-application-canary"):
            with self.subTest(application=application), self.assertRaisesRegex(schedule.topology.ExpectationError, "unreviewed"):
                self.check(application=application)
        for override in ({"application": "nextcloud"}, {"application": "forgejo"}, {"application": "observability"},
                         {"application": "supabase"}, {"lane": "debian11-rootless"},
                         {"lane": "upstream-rootful"}, {"prefix": "other"}, {"run_id": "other"},
                         {"fixture_root": "/tmp/other"}, {"profile": {**self.profile, "engine_release": "29.8.2"}},
                         {"image_aliases": {**self.images, "server": "registry.invalid/other:reviewed"}}):
            with self.subTest(override=override), self.assertRaises(schedule.topology.ExpectationError):
                self.check(**override)
        self.profile["daemon_mode"] = "rootful"
        self.plan["context"] = copy.deepcopy(self.profile)
        admission = self.admission(encode(self.plan))
        admission["lane"] = "upstream-rootful"
        # The existing topology validator admits Immich only on rootless;
        # matching caller-owned rootful fields do not expand that finite lane.
        with self.assertRaisesRegex(schedule.topology.ExpectationError, "finite approved scope"):
            self.check(admission=encode(admission), lane="upstream-rootful")

    def test_cli_checks_source_before_inputs_and_labels_immich_refusals(self):
        arguments = ["schedule", "--application", "immich", "--plan", "private-plan-canary",
                     "--admission", "private-admission-canary", "--sidecar", "private-sidecar-canary",
                     "--profile", "private-profile-canary", "--image-aliases", "private-images-canary",
                     "--lane", "upstream-rootless", "--prefix", "unit", "--run-id", "offline-run",
                     "--fixture-root", "/tmp/offline-fixture"]
        with mock.patch.object(sys, "argv", arguments), mock.patch.object(schedule.topology, "check_sources",
                side_effect=schedule.topology.ExpectationError("reviewed source bytes differ")) as checker, \
                mock.patch.object(schedule, "read_document", return_value=CATALOGUE) as opened, mock.patch("sys.stderr") as stderr:
            self.assertEqual(schedule.main(), 1)
            checker.assert_called_once()
            self.assertEqual(opened.call_count, 1)
            output = "".join(str(call) for call in stderr.write.call_args_list)
            self.assertIn("offline Immich schedule rejected", output)
            self.assertNotIn("canary", output)

    def test_cli_accepts_literal_immich_inputs_without_qualifying_runtime(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            raw = encode(self.plan)
            documents = {"plan": self.plan, "admission": self.admission(raw), "sidecar": self.sidecar(raw),
                         "profile": self.profile, "image-aliases": self.images}
            arguments = [sys.executable, str(ROOT / "scripts/lib/docker-application-schedule.py"), "--application", "immich"]
            for name, document in documents.items():
                path = root / f"{name}.json"
                path.write_bytes(encode(document))
                arguments.extend([f"--{name}", str(path)])
            arguments.extend(["--lane", "upstream-rootless", "--prefix", "unit", "--run-id", "offline-run",
                              "--fixture-root", "/tmp/offline-fixture"])
            result = subprocess.run(arguments, capture_output=True, check=False, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            result_document = json.loads(result.stdout)
            self.assertEqual(result_document["service_review_layers"], [["database", "immich-machine-learning", "redis"], ["immich-server"]])
            self.assertIs(result_document["native_admission"], False)
            self.assertIs(result_document["native_execution"], False)
            self.assertEqual(result_document["runtime_evidence"], "unmeasured")
            self.assertEqual(result.stderr, b"")


if __name__ == "__main__":
    unittest.main()
