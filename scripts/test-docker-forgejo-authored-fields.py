#!/usr/bin/env python3
"""Independent authored Forgejo assertions; no renderer, native client or runtime."""

from __future__ import annotations
import copy
import hashlib
import importlib.util
import io
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[1]
HELPER = ROOT / "scripts/lib/docker-forgejo-authored-fields.py"
SPEC = importlib.util.spec_from_file_location("docker_forgejo_authored_fields", HELPER)
assert SPEC is not None and SPEC.loader is not None
contract = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(contract)
CATALOGUE = (ROOT / contract.topology.CATALOGUE_PATH).read_bytes()
AUTHORED = json.loads(CATALOGUE)["applications"]["forgejo"]


def encode(value):
    return (json.dumps(value, separators=(",", ":")) + "\n").encode()


def request(path, body):
    return {"method": "POST", "path": "/v1.56/" + path, "body": body}


class ForgejoAuthoredFields(unittest.TestCase):
    def setUp(self):
        self.profile = {"kind": "target", "build": {"kind": "upstream"}, "engine_release": "29.8.1",
                        "advertised_api_version": "1.56", "acquisition_api_version": "1.49",
                        "rendering_api_version": "1.56", "daemon_mode": "rootless", "evidence_sha256": "ab" * 32}
        self.images = {"forgejo": "registry.invalid/offline/forgejo:reviewed",
                       "postgres": "registry.invalid/offline/postgres:reviewed"}
        self.password, self.key = "private-db-canary=密碼", "private-key-canary=κλειδί"
        self.context = {"application": "forgejo", "lane": "upstream-rootless", "profile": self.profile,
                        "image_aliases": self.images, "prefix": "unit", "run_id": "offline-run",
                        "fixture_root": "/tmp/offline-fixture"}
        self.interpolation = {"schema_version": 1, "kind": "boxferry-docker-forgejo-interpolation-expectations",
                              "context": copy.deepcopy(self.context), "interpolation": {
                                  "BF_DB_PASSWORD": self.password, "BF_FORGEJO_SECRET_KEY": self.key}}
        labels = {"io.boxferry.live-run": "offline-run", "io.boxferry.application": "unit-forgejo"}
        # Literal test artifacts are independent of the helper's required-field maps,
        # catalogue projection and any native renderer or acquired image defaults.
        self.db = {
            "Image": self.images["postgres"], "Labels": labels,
            "Env": ["POSTGRES_DB=forgejo", "POSTGRES_USER=forgejo", "POSTGRES_PASSWORD=" + self.password],
            "Healthcheck": {"Test": ["CMD-SHELL", "pg_isready -U forgejo -d forgejo"],
                            "Interval": 2_000_000_000, "Timeout": 5_000_000_000, "Retries": 60},
            "HostConfig": {"NetworkMode": "unit-forge-backend", "Mounts": [
                {"Type": "volume", "Source": "unit-forge-db", "Target": "/var/lib/postgresql/data", "ReadOnly": False}]},
            "NetworkingConfig": {"EndpointsConfig": {"unit-forge-backend": {"Aliases": ["db"]}}}}
        self.app = {
            "Image": self.images["forgejo"], "Labels": labels, "User": "1000:1000",
            "Env": ["FORGEJO__database__DB_TYPE=postgres", "FORGEJO__database__HOST=db:5432",
                    "FORGEJO__database__NAME=forgejo", "FORGEJO__database__USER=forgejo",
                    "FORGEJO__database__PASSWD=" + self.password, "FORGEJO__database__SSL_MODE=disable",
                    "FORGEJO__security__INSTALL_LOCK=true", "FORGEJO__security__SECRET_KEY=" + self.key,
                    "FORGEJO__service__DISABLE_REGISTRATION=true", "FORGEJO__server__DOMAIN=127.0.0.1",
                    "FORGEJO__server__ROOT_URL=http://127.0.0.1:13000/", "FORGEJO__server__SSH_DOMAIN=127.0.0.1",
                    "FORGEJO__server__SSH_PORT=12222", "FORGEJO__server__SSH_LISTEN_PORT=2222",
                    "FORGEJO__server__START_SSH_SERVER=true"],
            "ExposedPorts": {"3000/tcp": {}, "2222/tcp": {}},
            "HostConfig": {"NetworkMode": "unit-forge-backend", "Mounts": [
                {"Type": "volume", "Source": "unit-forge-data", "Target": "/var/lib/gitea", "ReadOnly": False}],
                "PortBindings": {"3000/tcp": [{"HostIp": "127.0.0.1", "HostPort": "13000"}],
                                 "2222/tcp": [{"HostIp": "127.0.0.1", "HostPort": "12222"}]}},
            "NetworkingConfig": {"EndpointsConfig": {"unit-forge-backend": {"Aliases": ["forgejo"]}}}}
        self.plan = {"schema_version": 1, "context": copy.deepcopy(self.profile), "requests": [
            request("networks/create", {"Name": "unit-forge-backend", "Driver": "bridge", "Internal": True, "Labels": labels}),
            request("volumes/create", {"Name": "unit-forge-db", "Labels": labels}),
            request("volumes/create", {"Name": "unit-forge-data", "Labels": labels}),
            request("containers/create?name=unit-forge-db", self.db),
            request("containers/create?name=unit-forge-app", self.app),
            request("networks/unit-shared-edge/connect", {"Container": "unit-forge-app", "EndpointConfig": {"Aliases": ["forgejo"]}})
        ], "prerequisites": [{"kind": "network", "reference": "18446744073709551615",
                             "identity": "unit-shared-edge", "expected_driver": "bridge"}]}

    def admission(self, raw):
        return {"schema_version": 1, "kind": "boxferry-docker-application-offline-admission",
                "evidence_kind": "offline-contract-prerequisite", "source_kind": "authored-compose",
                "native_execution": False, "application": "forgejo", "lane": "upstream-rootless",
                "prefix": "unit", "run_id": "offline-run", "fixture_root": "/tmp/offline-fixture",
                "docker_plan_sha256": hashlib.sha256(raw).hexdigest(),
                "expectations_sha256": hashlib.sha256(CATALOGUE).hexdigest(),
                "source_sha256": contract.topology.source_digest(AUTHORED["sources"]),
                **{name: copy.deepcopy(AUTHORED[name]) for name in
                   ("required_checks", "dependencies", "excluded_peers", "shared_services")},
                "runtime_evidence": "unmeasured", "budget_measurements": None}

    def sidecar(self, raw):
        return {"schema_version": 1, "kind": "boxferry-docker-dependency-decisions", "native_execution": False,
                "docker_plan_sha256": hashlib.sha256(raw).hexdigest(), "decisions": [{
                    "service": "forgejo", "service_runtime_name": "unit-forge-app",
                    "dependency": "db", "dependency_runtime_name": "unit-forge-db", "condition": "healthy",
                    "condition_explicit": True, "required": True, "required_explicit": True,
                    "restart": False, "restart_explicit": False, "fidelity": "approximate", "native_engine_field": False,
                    "provenance": {name: ["source_document"] for name in ("reference", "condition", "required", "restart")}}]}

    def check(self, *, plan=None, admission=None, sidecar=None, interpolation=None, **options):
        raw = encode(self.plan) if plan is None else plan
        return contract.validate_authored_fields(raw, encode(self.admission(raw)) if admission is None else admission,
            encode(self.sidecar(raw)) if sidecar is None else sidecar,
            encode(self.interpolation) if interpolation is None else interpolation,
            catalogue_bytes=CATALOGUE, **{**self.context, **options})

    def reject(self, **options):
        with self.assertRaises(contract.topology.ExpectationError) as caught:
            self.check(**options)
        for private in (self.password, self.key, "private-db-canary", "private-key-canary"):
            self.assertNotIn(private, str(caught.exception))

    def test_positive_has_only_offline_claims_and_no_private_values_or_private_input_digest(self):
        result = self.check()
        self.assertEqual(set(result), {"schema_version", "kind", "evidence_kind", "source_kind", "application", "lane",
            "native_execution", "replay_authority", "native_admission", "runtime_evidence", "budget_measurements",
            "docker_plan_sha256", "admission_sha256", "dependency_sidecar_sha256", "expectations_sha256", "source_sha256",
            "external_prerequisites", "native_request_order", "review_operations", "service_review_layers",
            "dependency_decisions", "authored_checks"})
        self.assertEqual(result["kind"], "boxferry-docker-forgejo-authored-fields")
        self.assertEqual(result["source_kind"], "authored-compose")
        self.assertEqual(result["authored_checks"], ["db.environment", "forgejo.environment", "db.healthcheck", "forgejo.user"])
        self.assertEqual(result["evidence_kind"], "offline-contract-prerequisite")
        for name in ("native_execution", "native_admission", "replay_authority"):
            self.assertIs(result[name], False)
        self.assertEqual(result["runtime_evidence"], "unmeasured")
        self.assertIsNone(result["budget_measurements"])
        serialized = json.dumps(result)
        for value in (self.password, self.key, "private-db-canary", "private-key-canary",
                      "pg_isready", "POSTGRES_PASSWORD", hashlib.sha256(encode(self.interpolation)).hexdigest()):
            self.assertNotIn(value, serialized)
        self.assertEqual(result["docker_plan_sha256"], hashlib.sha256(encode(self.plan)).hexdigest())
        self.assertEqual(result["admission_sha256"], hashlib.sha256(encode(self.admission(encode(self.plan)))).hexdigest())
        self.assertEqual(result["dependency_sidecar_sha256"], hashlib.sha256(encode(self.sidecar(encode(self.plan)))).hexdigest())

    def test_all_authored_environment_assignments_must_be_present_and_exact(self):
        for index in (3, 4):
            for position in range(len(self.plan["requests"][index]["body"]["Env"])):
                for replacement in (None, "wrong", ""):
                    with self.subTest(container=index, position=position, replacement=replacement):
                        changed = copy.deepcopy(self.plan)
                        values = changed["requests"][index]["body"]["Env"]
                        assignment = values.pop(position)
                        if replacement is not None:
                            values.insert(position, assignment.split("=", 1)[0] + "=" + replacement)
                        self.reject(plan=encode(changed))

    def test_environment_missing_malformed_or_duplicate_names_rejected(self):
        for index in (3, 4):
            for value in (None, {}, "private-db-canary", [False], ["=empty-name"], ["bare-name"],
                          ["BAD=nul\x00value"], ["BAD=\ud800"]):
                with self.subTest(container=index, value_type=type(value).__name__):
                    changed = copy.deepcopy(self.plan)
                    changed["requests"][index]["body"]["Env"] = value
                    self.reject(plan=encode(changed))
            for extras in (["EXTRA=one", "EXTRA=two"], [self.plan["requests"][index]["body"]["Env"][0]]):
                changed = copy.deepcopy(self.plan)
                changed["requests"][index]["body"]["Env"].extend(extras)
                self.reject(plan=encode(changed))

    def test_reordered_environment_and_extra_well_formed_assignments_are_unassessed(self):
        self.db["Env"].reverse()
        self.app["Env"].reverse()
        self.app["Env"].extend(["UNAUTHORED_OVERRIDE=unassessed", "EXTRA=one=two"])
        self.db["Env"].append("OTHER=unassessed")
        self.assertEqual(self.check()["authored_checks"], ["db.environment", "forgejo.environment", "db.healthcheck", "forgejo.user"])

    def test_values_are_independent_not_inferred_from_matching_artifact_assignments(self):
        changed = copy.deepcopy(self.plan)
        changed["requests"][3]["body"]["Env"][-1] = "POSTGRES_PASSWORD=matching-but-not-reviewed"
        changed["requests"][4]["body"]["Env"][4] = "FORGEJO__database__PASSWD=matching-but-not-reviewed"
        self.reject(plan=encode(changed))
        for key in ("BF_DB_PASSWORD", "BF_FORGEJO_SECRET_KEY"):
            changed_expectations = copy.deepcopy(self.interpolation)
            changed_expectations["interpolation"][key] = "different-independent-value"
            self.reject(interpolation=encode(changed_expectations))

    def test_database_health_command_and_each_authored_number_are_exact_and_typed(self):
        for field, values in {"Test": [None, ["CMD", "pg_isready -U forgejo -d forgejo"],
                                      ["CMD-SHELL", "pg_isready -U nobody -d forgejo"],
                                      ["CMD-SHELL", "pg_isready -U forgejo -d forgejo", "extra"]],
                              "Interval": [None, 2, "2000000000", True, 2_000_000_000.0],
                              "Timeout": [None, 5, "5000000000", False], "Retries": [None, 59, "60", True]}.items():
            for value in values:
                with self.subTest(field=field, value_type=type(value).__name__):
                    changed = copy.deepcopy(self.plan)
                    health = changed["requests"][3]["body"]["Healthcheck"]
                    if value is None:
                        del health[field]
                    else:
                        health[field] = value
                    self.reject(plan=encode(changed))
        del self.db["Healthcheck"]
        self.reject()

    def test_user_is_required_exact_and_string(self):
        for value in (None, "0:0", "1000", 1000, True, "1000:1000\x00"):
            if value is None:
                self.app.pop("User", None)
            else:
                self.app["User"] = value
            self.reject()

    def test_unauthored_image_settings_and_health_defaults_remain_shape_only(self):
        self.app.update(Cmd=["unassessed-command"], Entrypoint=[], WorkingDir="/unassessed", StopSignal="SIGTERM",
                        StopTimeout=5, Healthcheck={"Test": ["CMD", "unassessed-health"]})
        self.db.update(Cmd=[], Entrypoint=["unassessed-entrypoint"], User="unassessed-db-user")
        self.db["Healthcheck"].update(StartPeriod=10_000_000_000, StartInterval=1_000_000_000)
        self.assertFalse(self.check()["native_admission"])

    def test_interpolation_schema_and_value_bounds_fail_closed(self):
        for field in ("schema_version", "kind", "context", "interpolation"):
            changed = copy.deepcopy(self.interpolation)
            del changed[field]
            self.reject(interpolation=encode(changed))
        for field, value in (("schema_version", True), ("schema_version", 2), ("kind", "other"),
                             ("unexpected", False), ("interpolation", []), ("context", {})):
            changed = copy.deepcopy(self.interpolation)
            changed[field] = value
            self.reject(interpolation=encode(changed))
        for key in ("BF_DB_PASSWORD", "BF_FORGEJO_SECRET_KEY"):
            for value in (None, False, 7, [], {}, "", "nul\x00private-db-canary", "\ud800", "a" * 4097, "密" * 1366):
                changed = copy.deepcopy(self.interpolation)
                changed["interpolation"][key] = value
                self.reject(interpolation=encode(changed))
            changed = copy.deepcopy(self.interpolation)
            del changed["interpolation"][key]
            self.reject(interpolation=encode(changed))
        changed = copy.deepcopy(self.interpolation)
        changed["interpolation"]["OTHER"] = "private-db-canary"
        self.reject(interpolation=encode(changed))

    def test_authored_fixture_corresponds_to_independent_field_assertions(self):
        source = (ROOT / "fixtures/conformance/forgejo-application/compose.yaml").read_text()
        self.assertEqual(len(self.db["Env"]), 3)
        self.assertEqual(len(self.app["Env"]), 15)
        quoted = {"true", "12222", "2222"}
        for assignment in [*self.db["Env"], *self.app["Env"]]:
            name, value = assignment.split("=", 1)
            if value == self.password:
                value = "${BF_DB_PASSWORD:?}"
            elif value == self.key:
                value = "${BF_FORGEJO_SECRET_KEY:?}"
            elif value in quoted:
                value = '"' + value + '"'
            self.assertIn("      " + name + ": " + value + "\n", source)
        for line in ('      test: ["CMD-SHELL", "pg_isready -U forgejo -d forgejo"]\n',
                     "      interval: 2s\n", "      timeout: 5s\n", "      retries: 60\n", '    user: "1000:1000"\n'):
            self.assertIn(line, source)
        contract.topology.check_sources(ROOT, CATALOGUE)

    def test_interpolation_boundary_and_significant_whitespace_are_retained(self):
        password = " " + "a" * 4094 + " "
        self.interpolation["interpolation"]["BF_DB_PASSWORD"] = password
        self.db["Env"][-1] = "POSTGRES_PASSWORD=" + password
        self.app["Env"][4] = "FORGEJO__database__PASSWD=" + password
        self.assertFalse(self.check()["native_execution"])
        self.interpolation["interpolation"]["BF_DB_PASSWORD"] = password.strip()
        self.reject()

    def test_interpolation_document_reuses_duplicate_depth_nonfinite_and_size_limits(self):
        raw = encode(self.interpolation)
        malformed = [b"", b"[", b"\xff", b'{"schema_version":1,"schema_version":1}',
                     raw.replace(b'"schema_version":1', b'"schema_version":NaN'),
                     raw.replace(b'"BF_DB_PASSWORD":', b'"BF_DB_PASSWORD":"private-db-canary","BF_DB_PASSWORD":'),
                     b"[" * 33 + b"0" + b"]" * 33, b" " * (contract.topology.LIMIT + 1)]
        for document in malformed:
            self.reject(interpolation=document)

    def test_every_independent_context_field_must_match_exactly(self):
        for name, value in {"application": "nextcloud", "lane": "upstream-rootful", "prefix": "other",
                            "run_id": "other", "fixture_root": "/tmp/elsewhere", "profile": {}, "image_aliases": {}}.items():
            changed = copy.deepcopy(self.interpolation)
            changed["context"][name] = value
            self.reject(interpolation=encode(changed))
        changed = copy.deepcopy(self.interpolation)
        changed["context"]["extra"] = "private-db-canary"
        self.reject(interpolation=encode(changed))

    def test_exact_raw_plan_admission_sidecar_and_source_bindings_are_reused(self):
        raw = encode(self.plan)
        self.reject(plan=raw + b" ", admission=encode(self.admission(raw)), sidecar=encode(self.sidecar(raw)))
        admission = self.admission(raw)
        admission["source_sha256"] = "00" * 32
        self.reject(admission=encode(admission))
        admission = self.admission(raw)
        admission["expectations_sha256"] = "00" * 32
        self.reject(admission=encode(admission))
        sidecar = self.sidecar(raw)
        sidecar["decisions"][0]["condition"] = "started"
        self.reject(sidecar=encode(sidecar))
        sidecar["docker_plan_sha256"] = "00" * 32
        self.reject(sidecar=encode(sidecar))

    def test_topology_request_context_and_version_failures_are_not_bypassed(self):
        for index, field, value in [(0, "Internal", False), (3, "Labels", {}), (4, "Image", self.images["postgres"])]:
            changed = copy.deepcopy(self.plan)
            changed["requests"][index]["body"][field] = value
            self.reject(plan=encode(changed))
        self.reject(application="nextcloud")
        self.reject(lane="debian-rootless")
        changed = copy.deepcopy(self.plan)
        changed["schema_version"] = 2
        self.reject(plan=encode(changed))
        changed = copy.deepcopy(self.plan)
        changed["context"]["engine_release"] = "29.8.2"
        self.reject(plan=encode(changed))

    def test_finite_upstream_rootful_context_is_supported_without_native_admission(self):
        self.context["lane"] = "upstream-rootful"
        self.profile["daemon_mode"] = "rootful"
        self.plan["context"] = copy.deepcopy(self.profile)
        self.interpolation["context"] = copy.deepcopy(self.context)
        admission = self.admission(encode(self.plan))
        admission["lane"] = "upstream-rootful"
        result = self.check(admission=encode(admission))
        self.assertEqual(result["lane"], "upstream-rootful")
        self.assertFalse(result["native_admission"])

    def cli_files(self, root):
        raw = encode(self.plan)
        documents = {"plan": raw, "admission": encode(self.admission(raw)), "sidecar": encode(self.sidecar(raw)),
                     "profile": encode(self.profile), "image-aliases": encode(self.images),
                     "interpolation-expectations": encode(self.interpolation)}
        args = ["--repository", str(ROOT), "--lane", "upstream-rootless", "--prefix", "unit",
                "--run-id", "offline-run", "--fixture-root", "/tmp/offline-fixture"]
        for name, document in documents.items():
            path = root / (name + ".json")
            path.write_bytes(document)
            args.extend(["--" + name, str(path)])
        return args

    def run_cli(self, args):
        return subprocess.run([sys.executable, str(HELPER), *args], check=False, capture_output=True, text=True,
                              env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"}, timeout=10)

    def test_cli_success_and_protected_mismatch_rejection_never_emit_values(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            args = self.cli_files(root)
            result = self.run_cli(args)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout)["runtime_evidence"], "unmeasured")
            private_file = root / "interpolation-expectations.json"
            for value in ("different-independent-value", "\ud800", None):
                changed = copy.deepcopy(self.interpolation)
                changed["interpolation"]["BF_DB_PASSWORD"] = value
                private_file.write_bytes(encode(changed))
                failure = self.run_cli(args)
                self.assertEqual(failure.returncode, 1)
                self.assertEqual(failure.stdout, "")
                for private in (self.password, self.key, "private-db-canary", "private-key-canary", "different-independent-value"):
                    self.assertNotIn(private, failure.stderr)
            for private in (self.password, self.key, "private-db-canary", "private-key-canary"):
                self.assertNotIn(private, result.stdout + result.stderr)

    def test_cli_checks_sources_before_opening_private_inputs_and_keeps_errors_static(self):
        with tempfile.TemporaryDirectory() as directory:
            args = self.cli_files(pathlib.Path(directory))
            calls = []
            stdout, stderr = io.StringIO(), io.StringIO()
            def read(path):
                calls.append("read-catalogue")
                return CATALOGUE
            def sources(repository, catalogue):
                calls.append("check-sources")
                raise contract.topology.ExpectationError("private-db-canary")
            with mock.patch.object(sys, "argv", [str(HELPER), *args]), \
                 mock.patch.object(contract.schedule, "read_document", side_effect=read), \
                 mock.patch.object(contract.topology, "check_sources", side_effect=sources), \
                 redirect_stdout(stdout), redirect_stderr(stderr):
                self.assertEqual(contract.main(), 1)
            self.assertEqual(calls, ["read-catalogue", "check-sources"])
            self.assertEqual(stdout.getvalue(), "")
            self.assertNotIn("private-db-canary", stderr.getvalue())

    def test_cli_private_file_read_is_bounded_regular_and_nofollow(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            args = self.cli_files(root)
            private_file = root / "interpolation-expectations.json"
            private_file.write_bytes(b" " * (contract.topology.LIMIT + 1))
            self.assertEqual(self.run_cli(args).returncode, 1)
            private_file.unlink()
            private_file.symlink_to(root / "plan.json")
            self.assertEqual(self.run_cli(args).returncode, 1)
            private_file.unlink()
            os.mkfifo(private_file)
            self.assertEqual(self.run_cli(args).returncode, 1)


if __name__ == "__main__":
    unittest.main()
