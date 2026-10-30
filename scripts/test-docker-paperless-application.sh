#!/usr/bin/env bash
# Hermetic source-contract tests. Never opens a runtime socket or starts a guest.
set -Eeuo pipefail
repository_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
PYTHONDONTWRITEBYTECODE=1 python3 - "${repository_root}" "$@" << 'PY'
import copy
import hashlib
import importlib.util
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = pathlib.Path(sys.argv[1])
SPEC = importlib.util.spec_from_file_location("expectations", ROOT / "scripts/lib/docker-application-expectations.py")
CONTRACT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CONTRACT)
CATALOGUE = (ROOT / CONTRACT.CATALOGUE_PATH).read_bytes()
APP = json.loads(CATALOGUE)["applications"]["paperless-ngx"]


def encode(value):
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


# Fake transport emits normalized observations and records argv. It deliberately
# has no Docker/Podman/Compose client, subprocess execution, network, or secrets.
FAKE = r'''
import copy, hashlib, json, pathlib, sys, time
context, request_path, reply_path = map(pathlib.Path, sys.argv[1:])
state = json.loads(context.read_text())
raw = request_path.read_bytes()
request = json.loads(raw)
action = request["action"]
state["log"].append(request)
scenario = state["scenario"]
def save():
    context.write_text(json.dumps(state))
if action == "cleanup":
    # Cleanup uses caller ledger + pre-held scope, even if the manifest vanished.
    assert request["cleanup_scope"]["labels"] == {
        "io.boxferry.live-run": "hermetic-run", "io.boxferry.application": "unit-paperless"}
    assert len(request["cleanup_scope"]["resources"]) == 13
    payload = {"created": state["ledger"], "removed": copy.deepcopy(state["ledger"]),
               "remaining": [], "unowned_touched": [], "reconciled": True, "uncertain": False}
else:
    manifest = json.loads(pathlib.Path(request["manifest"]).read_text())
    config, app = manifest["config"], manifest["application"]
    assert config["profile"]["daemon_mode"] == "rootless"
    assert request["endpoint"] == config["endpoint"]
    assert request["work_deadline_ms"] == state["work_deadline_ms"]
    assert request["cleanup_deadline_ms"] == state["cleanup_deadline_ms"]
    if config["stage"] == "generated-target":
        assert config["target"] is not None
        for name in ("artifact", "neutral_expectations", "authorization", "source_handoff"):
            held = pathlib.Path(config["target"][name]).read_bytes()
            assert hashlib.sha256(held).hexdigest() == config["target"][name + "_sha256"]
    def graph(generation):
        resources = {}
        for group, kind in (("networks", "network"), ("volumes", "volume"), ("services", "container")):
            for role, node in app[group].items():
                name = "unit-" + node["runtime_suffix"]
                identity = hashlib.sha256((name + str(generation if kind == "container" else 0)).encode()).hexdigest()
                row = {"kind": kind, "identity": identity,
                       "labels": {"io.boxferry.live-run": "hermetic-run", "io.boxferry.application": "unit-paperless"},
                       "specification": copy.deepcopy(node)}
                if kind == "container":
                    row.update(image_config_id=config["image_config_ids"][node["image_key"]],
                               running=True, healthy=role in ("db", "broker"), settings=copy.deepcopy(manifest["settings"][role]))
                resources[name] = row
                entry = {"name": name, "kind": kind, "identity": identity}
                if entry not in state["ledger"]:
                    state["ledger"].append(entry)
        return {"resources": resources, "dependencies": app["dependencies"],
                "storage": {"unit-paper-" + key: {"uid": 1000, "gid": 1000, "world_writable": False,
                    "application_uid": 1000, "application_gid": 1000, "read": True, "write": True, "create_remove": True}
                    for key in ("data", "media", "consume", "export")},
                "image_bindings": {key: {"reference": value, "alias": config["image_aliases"][key],
                    "config_id": config["image_config_ids"][key]} for key, value in manifest["images"].items()}}
    if action == "preflight":
        payload = {"collisions": [], "process_tree_watchdog": True, "absolute_deadlines": True,
                   "cleanup_reserved": True, "no_fallback": True, "no_implicit_pulls": True,
                   "budget": {"cpus": 2, "memory_kib": 6291456, "disk_kib": 12582912, "image_archive_bytes": 1}}
    elif action in ("provision", "recreate"):
        state["generation"] = int(action == "recreate")
        state["graph"] = graph(state["generation"])
        payload = copy.deepcopy(state["graph"])
        if action == "provision" and config["stage"] == "generated-target":
            handoff = json.loads(pathlib.Path(config["target"]["source_handoff"]).read_bytes())
            state["document_state"] = copy.deepcopy(handoff["source_state"])
            state["documents"] = 3
            if scenario == "missing-prepopulated-state":
                state["document_state"] = {"schema": 1, "documents": []}
                state["documents"] = 0
    elif action == "inspect":
        payload = copy.deepcopy(state["graph"])
    elif action == "probe":
        args = request["arguments"]
        assert args[:2] == [str(context), "unit"]
        if args[2] == "ready":
            assert len(args) == 3
        elif args[2] == "ingest":
            phase = "baseline" if state["documents"] == 0 else "second"
            assert args[3:] == ["--phase", phase, "--state", "/fixture/probe-state.json", "--work-dir", "/fixture/generated-" + phase]
            state["documents"] += 3
            for extension in ("pdf", "docx", "odt"):
                record = copy.deepcopy(manifest["document_expectations"][phase][extension])
                record["id"] = len(state["document_state"]["documents"]) + 1
                if extension != "pdf": record["archive_sha256"] = "ac" * 32
                state["document_state"]["documents"].append(record)
            if scenario != "stagnant-lpush":
                state["lpush"] += 3
        else:
            assert args[2:] == ["verify", "--state", "/fixture/probe-state.json"]
        payload = {"exit_code": 0, "probe_sha256": next(row["sha256"] for row in app["sources"] if row["path"].endswith("document-probe.py")),
                   "state": None if args[2] == "ready" else copy.deepcopy(state["document_state"])}
    elif action == "broker":
        assert request["arguments"] == [str(context), "unit"]
        payload = {"lpush_calls": state["lpush"]}
    elif action == "query":
        assert request["arguments"] == [str(context), "paperless", "boxferry-public-database-canary",
            "SELECT count(*) FROM documents_document WHERE title LIKE 'BoxFerry migration %';"]
        payload = {"rows": str(state["documents"])}
    elif action == "privacy":
        payload = {"report": {"schema_version": 1, "status": "success", "exit_category": "success",
            "fidelity": {"invalid": 0}, "output_artifacts": [{"path": "default-export"}], "diagnostics": [
                {"code": "BFP0002", "severity": "warning", "fields": [
                    {"name": "subject", "value": "services.unit-paper-web.environment.PAPERLESS_DBPASS"}, {"name": "decision", "value": "omitted"},
                    {"name": "required_loss_policy", "value": "partial"}]}]}, "artifacts": ["environment omitted\n"]}
    elif action == "handoff":
        payload = {"schema_version": 1, "kind": "paperless-source-handoff", "application": "paperless-ngx",
                   "prefix": "unit", "run_id": "hermetic-run", "plan_sha256": manifest["held"]["plan.json"],
                   "probe_sha256": manifest["probe_sha256"], "source_state": copy.deepcopy(state["document_state"])}
        state["handoff"] = payload
    else:
        raise AssertionError("unexpected action")

if scenario == "collision" and action == "preflight": payload["collisions"] = ["unit-paper-db"]
if scenario == "watchdog" and action == "preflight": payload["process_tree_watchdog"] = False
if scenario in ("low-cpu", "low-memory", "low-disk", "large-archive") and action == "preflight":
    key = {"low-cpu": "cpus", "low-memory": "memory_kib", "low-disk": "disk_kib", "large-archive": "image_archive_bytes"}[scenario]
    payload["budget"][key] = 1 if scenario != "large-archive" else 2684354561
if action == "provision":
    resources = payload["resources"]
    if scenario == "missing-container": del resources["unit-paper-tika"]
    if scenario == "extra-peer": resources["unit-foreign"] = resources["unit-paper-tika"]
    if scenario == "wrong-owner": resources["unit-paper-db"]["labels"]["io.boxferry.live-run"] = "foreign"
    if scenario == "wrong-image": resources["unit-paper-web"]["image_config_id"] = "sha256:" + "ff" * 32
    if scenario == "wrong-pin": payload["image_bindings"]["postgres"]["reference"] += "-changed"
    if scenario == "worker-limit": resources["unit-paper-web"]["settings"]["environment"]["PAPERLESS_TASK_WORKERS"] = "8"
    if scenario == "converter-limit": resources["unit-paper-gotenberg"]["settings"]["command"].pop()
    if scenario == "unhealthy-db": resources["unit-paper-db"]["healthy"] = False
    if scenario == "external-backend": resources["unit-paper-backend"]["specification"]["internal"] = False
    if scenario == "public-ingress": resources["unit-paper-web"]["specification"]["ingress"][0]["host_ip"] = "0.0.0.0"
    if scenario == "socket-bind": resources["unit-paper-web"]["specification"]["mounts"].append({"source": "/run/docker.sock"})
    if scenario == "storage-owner": payload["storage"]["unit-paper-data"]["uid"] = 0
    if scenario == "world-writable": payload["storage"]["unit-paper-data"]["world_writable"] = True
    if scenario == "no-storage-access": payload["storage"]["unit-paper-data"]["create_remove"] = False
    if scenario == "dependency": payload["dependencies"] = []
if action == "recreate":
    if scenario == "retain-container": payload["resources"]["unit-paper-tika"]["identity"] = hashlib.sha256(b"unit-paper-tika0").hexdigest()
    if scenario == "replace-volume": payload["resources"]["unit-paper-media"]["identity"] = "ed" * 32
    if scenario == "replace-network": payload["resources"]["unit-paper-backend"]["identity"] = "ec" * 32
    if scenario == "swap-old-ids": payload["resources"]["unit-paper-tika"]["identity"] = hashlib.sha256(b"unit-paper-gotenberg0").hexdigest()
if scenario == "inspect-drift" and action == "inspect": payload["resources"]["unit-paper-web"]["identity"] = "dd" * 32
if scenario == "noninteger-lpush" and action == "broker": payload["lpush_calls"] = "0"
if scenario == "boolean-lpush" and action == "broker": payload["lpush_calls"] = True
if scenario == "wrong-count" and action == "query": payload["rows"] = "6" if state["documents"] == 3 else "3"
if scenario == "wrong-final-count" and action == "query" and state["documents"] == 6: payload["rows"] = "3"
if scenario == "probe-hash" and action == "probe": payload["probe_sha256"] = "ff" * 32
if scenario == "probe-failure" and action == "probe": payload["exit_code"] = 1
if scenario == "source-original-changed" and action == "probe" and request["arguments"][2] == "verify": payload["state"]["documents"][0]["original_sha256"] = "bb" * 32
if scenario == "source-id-changed" and action == "probe" and request["arguments"][2] == "verify": payload["state"]["documents"][0]["id"] = 90
if scenario == "source-archive-changed" and action == "probe" and request["arguments"][2] == "verify": payload["state"]["documents"][1]["archive_sha256"] = "bb" * 32
if scenario == "handoff-ids" and action == "handoff":
    for record in payload["source_state"]["documents"]: record["id"] += 100
if scenario == "handoff-archive" and action == "handoff": payload["source_state"]["documents"][1]["archive_sha256"] = "bb" * 32
if scenario == "privacy-canary" and action == "privacy": payload["artifacts"].append("boxferry-public-admin-canary")
if scenario == "privacy-diagnostic" and action == "privacy": payload["report"]["diagnostics"] = []
if scenario == "privacy-report-canary" and action == "privacy": payload["report"]["detail"] = "boxferry-public-paperless-secret-canary"
if action == "cleanup":
    if scenario == "cleanup-uncertain": payload["uncertain"] = True
    if scenario == "cleanup-remaining": payload["remaining"] = ["unit-paper-db"]
    if scenario == "cleanup-foreign": payload["unowned_touched"] = ["foreign"]
    if scenario == "cleanup-omitted": payload["created"] = payload["removed"] = []
    if scenario == "cleanup-ledger": payload["removed"] = []
if scenario == "mutate-held-plan" and action == "provision": pathlib.Path(config["plan"]).write_text("{}")
if scenario == "mutate-held-manifest" and action == "provision": pathlib.Path(request["manifest"]).write_text("{}")
if scenario == "symlink-snapshot" and action == "provision": (request_path.parent / "initial.json").symlink_to(context)
if scenario == "mutate-request" and action == "provision":
    request["arguments"] = ["changed"]
    request_path.write_text(json.dumps(request))
    raw = request_path.read_bytes()
save()
if scenario == "late-provision" and action == "provision": time.sleep(1.3)
if scenario == "transport-failure" and action == "provision": raise SystemExit(17)
if scenario == "cancelled" and action == "provision": raise SystemExit(130)
if scenario == "terminated" and action == "provision": raise SystemExit(143)
if scenario == "cleanup-failure" and action == "cleanup": raise SystemExit(19)
reply = {"schema_version": 1, "request_sha256": hashlib.sha256(raw).hexdigest(),
         "manifest_sha256": request["manifest_sha256"], "action": action,
         "stage": request["stage"], "endpoint": request["endpoint"], "payload": payload}
if scenario == "wrong-stage" and action == "provision": reply["stage"] = "source-fixture"
if scenario == "wrong-endpoint" and action == "provision": reply["endpoint"]["id"] = "foreign"
if scenario == "wrong-receipt-hash" and action == "provision": reply["request_sha256"] = "ee" * 32
reply_path.write_text(json.dumps(reply))
if scenario == "oversized" and action == "provision": reply_path.write_text("x" * 1048577)
if scenario == "duplicate-key" and action == "provision": reply_path.write_text('{"schema_version":1,"schema_version":1}')
if scenario == "symlink-reply" and action == "provision":
    reply_path.unlink()
    reply_path.symlink_to(context)
'''


class PaperlessPrepared(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="paperless-hermetic-")
        self.addCleanup(self.temp.cleanup)
        self.directory = pathlib.Path(self.temp.name)
        self.fake = self.directory / "fake.py"
        self.fake.write_text(FAKE)
        self.profile = {"kind": "target", "build": {"kind": "upstream"}, "engine_release": "29.8.1",
                        "advertised_api_version": "1.56", "acquisition_api_version": "1.49",
                        "rendering_api_version": "1.56", "daemon_mode": "rootless", "evidence_sha256": "ab" * 32}
        self.images = {key: "registry.invalid/hermetic/" + key + ":reviewed"
                       for key in ("paperless", "postgres", "valkey", "gotenberg", "tika")}
        self.config_ids = {key: "sha256:" + sha(key.encode()) for key in self.images}
        labels = {"io.boxferry.live-run": "hermetic-run", "io.boxferry.application": "unit-paperless"}

        def request(path, body):
            return {"method": "POST", "path": "/v1.56/" + path, "body": body}

        # Independent authored native topology and settings, not renderer output
        # and not generated from the catalogue used by the implementation.
        self.plan = {"schema_version": 1, "context": copy.deepcopy(self.profile), "prerequisites": [], "requests": [
            request("networks/create", {"Name": "unit-paper-backend", "Driver": "bridge", "Internal": True, "Labels": labels}),
            request("networks/create", {"Name": "unit-paper-edge", "Driver": "bridge", "Labels": labels})]}
        for volume in ("data", "media", "consume", "export", "pgdata", "redisdata"):
            self.plan["requests"].append(request("volumes/create", {"Name": "unit-paper-" + volume, "Labels": labels}))
        environments = {
            "db": {"POSTGRES_DB": "paperless", "POSTGRES_USER": "paperless", "POSTGRES_PASSWORD": "boxferry-public-database-canary"},
            "web": {"PAPERLESS_REDIS": "redis://:boxferry-public-broker-canary@unit-paper-broker:6379",
                "PAPERLESS_DBHOST": "unit-paper-db", "PAPERLESS_DBNAME": "paperless", "PAPERLESS_DBUSER": "paperless",
                "PAPERLESS_DBPASS": "boxferry-public-database-canary", "PAPERLESS_SECRET_KEY": "boxferry-public-paperless-secret-canary",
                "PAPERLESS_ADMIN_USER": "boxferry-admin", "PAPERLESS_ADMIN_PASSWORD": "boxferry-public-admin-canary",
                "PAPERLESS_ADMIN_MAIL": "boxferry@example.invalid", "PAPERLESS_URL": "http://127.0.0.1:18000",
                "PAPERLESS_TIME_ZONE": "Etc/UTC", "PAPERLESS_OCR_LANGUAGE": "eng", "PAPERLESS_TASK_WORKERS": "1",
                "PAPERLESS_THREADS_PER_WORKER": "1", "PAPERLESS_WEBSERVER_WORKERS": "1", "PAPERLESS_TIKA_ENABLED": "1",
                "PAPERLESS_TIKA_ENDPOINT": "http://unit-paper-tika:9998",
                "PAPERLESS_TIKA_GOTENBERG_ENDPOINT": "http://unit-paper-gotenberg:3000", "PAPERLESS_CONSUMER_POLLING": "2",
                "USERMAP_UID": "1000", "USERMAP_GID": "1000"}}
        for role, image, alias, mounts in (
                ("db", "postgres", "db", [("pgdata", "/var/lib/postgresql")]),
                ("broker", "valkey", "broker", [("redisdata", "/data")]),
                ("gotenberg", "gotenberg", "gotenberg", []), ("tika", "tika", "tika", []),
                ("web", "paperless", "webserver", [(key, "/usr/src/paperless/" + key) for key in ("data", "media", "consume", "export")])):
            body = {"Image": self.images[image], "Labels": labels,
                    "HostConfig": {"NetworkMode": "unit-paper-backend", "Mounts": [
                        {"Type": "volume", "Source": "unit-paper-" + source, "Target": target, "ReadOnly": False}
                        for source, target in mounts]},
                    "NetworkingConfig": {"EndpointsConfig": {"unit-paper-backend": {"Aliases": [alias]}}}}
            if role in environments:
                body["Env"] = [key + "=" + value for key, value in environments[role].items()]
            if role == "broker": body["Cmd"] = ["valkey-server", "--requirepass", "boxferry-public-broker-canary"]
            if role == "gotenberg": body["Cmd"] = ["gotenberg", "--chromium-disable-javascript=true", "--chromium-allow-list=file:///tmp/.*"]
            if role in ("db", "broker"):
                body["Healthcheck"] = {"Test": ["CMD-SHELL", "pg_isready -U paperless -d paperless"] if role == "db" else
                    ["CMD", "valkey-cli", "-a", "boxferry-public-broker-canary", "ping"],
                    "Interval": 2000000000, "Timeout": 2000000000, "Retries": 90}
            if role == "web":
                body["HostConfig"]["Mounts"].append({"Type": "bind", "Source": "/tmp/hermetic-fixture", "Target": "/fixture", "ReadOnly": False})
                body["HostConfig"]["PortBindings"] = {"8000/tcp": [{"HostIp": "127.0.0.1", "HostPort": "18000"}]}
                body["ExposedPorts"] = {"8000/tcp": {}}
            self.plan["requests"].append(request("containers/create?name=unit-paper-" + role, body))
        self.plan["requests"].append(request("networks/unit-paper-edge/connect", {"Container": "unit-paper-web", "EndpointConfig": {"Aliases": ["paperless"]}}))
        self.config = {"schema_version": 1, "stage": "source-fixture", "prefix": "unit", "run_id": "hermetic-run",
            "fixture_root": "/tmp/hermetic-fixture", "profile": self.profile, "image_aliases": self.images,
            "image_config_ids": self.config_ids,
            "endpoint": {"id": "inner-paperless", "uri": "unix:///tmp/owned-inner/docker.sock", "scope": "owned-inner",
                         "mode": "rootless", "ownership_sha256": "cd" * 32},
            "plan": str(self.directory / "plan.json"), "admission": str(self.directory / "admission.json"),
            "work_deadline_ms": time.time_ns() // 1000000 + 300000,
            "cleanup_deadline_ms": time.time_ns() // 1000000 + 360000, "target": None}
        self.runner = self.directory / "runner.sh"
        self.runner.write_text('''#!/usr/bin/env bash
set -Eeuo pipefail
source "$1/scripts/lib/docker-paperless-application.sh"
fake_transport() { python3 "$PAPERLESS_FAKE" "$@"; }
docker_paperless_application_run "$1" fake_transport "$2" "$3"
''')

    def admission(self, raw):
        return {"schema_version": 1, "kind": "boxferry-docker-application-offline-admission",
            "evidence_kind": "offline-contract-prerequisite", "native_execution": False, "source_kind": "authored-compose",
            "application": "paperless-ngx", "lane": "upstream-rootless", "prefix": "unit", "run_id": "hermetic-run",
            "fixture_root": "/tmp/hermetic-fixture", "docker_plan_sha256": sha(raw), "expectations_sha256": sha(CATALOGUE),
            "source_sha256": CONTRACT.source_digest(APP["sources"]), "required_checks": APP["required_checks"],
            "dependencies": APP["dependencies"], "excluded_peers": [], "shared_services": [],
            "runtime_evidence": "unmeasured", "budget_measurements": None}

    def run_case(self, scenario="success", admission_change=None):
        raw = encode(self.plan)
        pathlib.Path(self.config["plan"]).write_bytes(raw)
        admission = self.admission(raw)
        if admission_change:
            admission.update(admission_change)
        pathlib.Path(self.config["admission"]).write_bytes(encode(admission))
        config_path = self.directory / "config.json"
        config_path.write_bytes(encode(self.config))
        state_path = self.directory / "state.json"
        state_path.write_bytes(encode({"scenario": scenario, "generation": 0, "documents": 0,
            "document_state": {"schema": 1, "documents": []},
            "lpush": 20, "ledger": [], "log": [], "work_deadline_ms": self.config["work_deadline_ms"],
            "cleanup_deadline_ms": self.config["cleanup_deadline_ms"]}))
        result = subprocess.run(["bash", str(self.runner), str(ROOT), str(state_path), str(config_path)],
            env={**os.environ, "PAPERLESS_FAKE": str(self.fake), "PYTHONDONTWRITEBYTECODE": "1"},
            capture_output=True, text=True, timeout=20)
        state = json.loads(state_path.read_bytes())
        self.assertNotIn("boxferry-public-", result.stdout + result.stderr)
        return result, state

    def test_literal_sequence_and_callback_arguments(self):
        self.target()
        result, state = self.run_case()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([row["action"] for row in state["log"]], [
            "preflight", "provision", "inspect", "probe", "probe", "query", "privacy",
            "recreate", "inspect", "probe", "probe", "query", "broker", "probe", "broker", "probe", "query", "inspect", "privacy", "cleanup"])
        probes = [row["arguments"][2:] for row in state["log"] if row["action"] == "probe"]
        self.assertEqual(probes, [["ready"], ["verify", "--state", "/fixture/probe-state.json"],
            ["ready"], ["verify", "--state", "/fixture/probe-state.json"],
            ["ingest", "--phase", "second", "--state", "/fixture/probe-state.json", "--work-dir", "/fixture/generated-second"],
            ["verify", "--state", "/fixture/probe-state.json"]])
        self.assertEqual(state["documents"], 6)
        self.assertEqual(state["lpush"], 23)
        self.assertEqual(len(state["ledger"]), 18)
        self.assertEqual(sum(row["kind"] == "container" for row in state["ledger"]), 10)
        self.assertEqual(sum(row["kind"] == "volume" for row in state["ledger"]), 6)
        self.assertEqual(sum(row["kind"] == "network" for row in state["ledger"]), 2)
        for row in state["log"]:
            self.assertEqual(row["deadline_ms"], self.config["cleanup_deadline_ms" if row["action"] == "cleanup" else "work_deadline_ms"])

    def test_counterfactual_receipts_fail_and_cleanup_runs(self):
        self.target()
        for scenario in ("missing-container", "extra-peer", "wrong-owner", "wrong-image", "wrong-pin", "worker-limit",
            "converter-limit", "unhealthy-db", "external-backend", "public-ingress", "socket-bind", "storage-owner",
            "world-writable", "no-storage-access", "dependency", "retain-container", "replace-volume", "replace-network",
            "swap-old-ids", "inspect-drift", "stagnant-lpush", "noninteger-lpush", "boolean-lpush", "wrong-count",
            "wrong-final-count", "probe-hash", "probe-failure", "privacy-canary", "privacy-diagnostic", "privacy-report-canary",
            "cleanup-uncertain", "cleanup-remaining", "cleanup-foreign", "cleanup-omitted", "cleanup-ledger", "cleanup-failure",
            "mutate-held-plan", "mutate-held-manifest", "mutate-request", "wrong-stage", "wrong-endpoint", "wrong-receipt-hash",
            "oversized", "duplicate-key", "symlink-reply", "symlink-snapshot", "missing-prepopulated-state", "source-original-changed",
            "source-id-changed", "source-archive-changed"):
            with self.subTest(scenario=scenario):
                result, state = self.run_case(scenario)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(state["log"][-1]["action"], "cleanup")
                if scenario in ("missing-prepopulated-state", "source-original-changed", "source-id-changed", "source-archive-changed"):
                    self.assertFalse(any(row["action"] == "probe" and row["arguments"][2] == "ingest" for row in state["log"]))
                if scenario.startswith("cleanup-"):
                    self.assertIn("cleanup is uncertain", result.stderr)

    def test_transport_failure_and_cancellation_preserve_status(self):
        for scenario, expected in (("transport-failure", 17), ("cancelled", 130), ("terminated", 143)):
            with self.subTest(scenario=scenario):
                result, state = self.run_case(scenario)
                self.assertEqual(result.returncode, expected)
                self.assertEqual([row["action"] for row in state["log"]], ["preflight", "provision", "cleanup"])

    def test_preflight_refusals_never_provision(self):
        for scenario in ("collision", "watchdog", "low-cpu", "low-memory", "low-disk", "large-archive"):
            with self.subTest(scenario=scenario):
                result, state = self.run_case(scenario)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual([row["action"] for row in state["log"]], ["preflight"])

    def test_input_bindings_and_deferred_fields_reject_before_transport(self):
        cases = ("rootful", "endpoint", "plan-profile", "wrong-route", "expired", "no-reserve", "image-config",
                 "env", "duplicate-env", "cmd", "entrypoint", "health", "target-without-custody")
        for case in cases:
            with self.subTest(case=case):
                saved_config, saved_plan = copy.deepcopy(self.config), copy.deepcopy(self.plan)
                if case == "rootful": self.config["profile"]["daemon_mode"] = "rootful"
                if case == "endpoint": self.config["endpoint"]["uri"] = "unix:///var/run/docker.sock"
                if case == "plan-profile": self.plan["context"]["evidence_sha256"] = "ef" * 32
                if case == "expired": self.config["work_deadline_ms"] = 1
                if case == "no-reserve": self.config["cleanup_deadline_ms"] = self.config["work_deadline_ms"]
                if case == "image-config": self.config["image_config_ids"]["paperless"] = "manifest-not-config"
                web = self.plan["requests"][-2]["body"]
                if case == "env": web["Env"] = []
                if case == "duplicate-env": web["Env"].append(web["Env"][0])
                if case == "cmd": web["Cmd"] = ["sh", "-c", "arbitrary"]
                if case == "entrypoint": web["Entrypoint"] = ["arbitrary"]
                if case == "health": del self.plan["requests"][8]["body"]["Healthcheck"]
                if case == "target-without-custody": self.config["stage"] = "generated-target"
                result, state = self.run_case(admission_change={"source_kind": "podman-cassette"} if case == "wrong-route" else None)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(state["log"], [])
                self.config, self.plan = saved_config, saved_plan

    def target(self):
        self.config["stage"] = "generated-target"
        target = {}
        for name, raw in (("artifact", b"held migration artifact\n"), ("neutral_expectations", b'{"documents":6,"source":"independent-caller"}\n')):
            path = self.directory / name
            path.write_bytes(raw)
            target[name], target[name + "_sha256"] = str(path), sha(raw)
        probe_spec = importlib.util.spec_from_file_location("canonical_probe", ROOT / "fixtures/conformance/paperless-ngx-application/document-probe.py")
        probe = importlib.util.module_from_spec(probe_spec)
        probe_spec.loader.exec_module(probe)
        records = []
        for extension, (marker, data) in probe.generated_documents("baseline").items():
            record = {"phase": "baseline", "extension": extension, "title": "BoxFerry migration baseline " + extension.upper(),
                      "marker": marker, "original_sha256": sha(data), "id": len(records) + 1}
            if extension != "pdf": record["archive_sha256"] = "ac" * 32
            records.append(record)
        handoff = {"schema_version": 1, "kind": "paperless-source-handoff", "application": "paperless-ngx",
                   "prefix": "source", "run_id": "source-hermetic-run", "plan_sha256": "ba" * 32,
                   "probe_sha256": sha((ROOT / "fixtures/conformance/paperless-ngx-application/document-probe.py").read_bytes()),
                   "source_state": {"schema": 1, "documents": records}}
        path = self.directory / "source-handoff"
        raw = encode(handoff)
        path.write_bytes(raw)
        target["source_handoff"], target["source_handoff_sha256"] = str(path), sha(raw)
        authorization = {"schema_version": 1, "kind": "caller-owned-allowlisted-paperless-target", "application": "paperless-ngx",
            "prefix": "unit", "run_id": "hermetic-run", "endpoint": self.config["endpoint"], "profile": self.profile,
            "plan_sha256": sha(encode(self.plan)), "artifact_sha256": target["artifact_sha256"],
            "neutral_expectations_sha256": target["neutral_expectations_sha256"], "authored_fields_reviewed": True,
            "source_handoff_sha256": target["source_handoff_sha256"],
            "loss_policy_reviewed": True, "isolated_test_harness_only": True}
        raw = encode(authorization)
        path = self.directory / "authorization"
        path.write_bytes(raw)
        target["authorization"], target["authorization_sha256"] = str(path), sha(raw)
        self.config["target"] = target

    def test_allowlisted_target_has_distinct_exact_custody(self):
        self.target()
        result, state = self.run_case()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(all(row["stage"] == "generated-target" for row in state["log"]))
        for field in ("artifact_sha256", "neutral_expectations_sha256", "authorization_sha256", "source_handoff_sha256"):
            with self.subTest(field=field):
                saved = self.config["target"][field]
                self.config["target"][field] = "ef" * 32
                result, state = self.run_case()
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(state["log"], [])
                self.config["target"][field] = saved
        self.config["stage"] = "source-fixture"
        result, state = self.run_case()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(state["log"], [])

    def test_source_seeds_three_and_emits_bound_handoff_without_recreation(self):
        result, state = self.run_case()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([row["action"] for row in state["log"]], ["preflight", "provision", "inspect", "probe", "broker",
            "probe", "broker", "probe", "query", "privacy", "handoff", "cleanup"])
        self.assertEqual(state["documents"], 3)
        self.assertEqual(state["lpush"], 23)
        self.assertEqual(len(state["ledger"]), 13)
        self.assertEqual(result.stdout.strip(), sha(encode(state["handoff"])))
        self.assertEqual(state["handoff"]["plan_sha256"], sha(encode(self.plan)))

    def test_actual_source_handoff_bytes_bind_target_before_ingest(self):
        result, source = self.run_case()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.target()
        target = self.config["target"]
        path = pathlib.Path(target["source_handoff"])
        raw = encode(source["handoff"])
        path.write_bytes(raw)
        target["source_handoff_sha256"] = sha(raw)
        authorization_path = pathlib.Path(target["authorization"])
        authorization = json.loads(authorization_path.read_bytes())
        authorization["source_handoff_sha256"] = sha(raw)
        raw = encode(authorization)
        authorization_path.write_bytes(raw)
        target["authorization_sha256"] = sha(raw)
        result, observed = self.run_case()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(observed["document_state"]["documents"][:3], source["handoff"]["source_state"]["documents"])
        before_ingest = []
        for row in observed["log"]:
            if row["action"] == "probe":
                if row["arguments"][2] == "ingest": break
                before_ingest.append(row["arguments"][2])
        self.assertEqual(before_ingest, ["ready", "verify", "ready", "verify"])
        # Matching hashes cannot make an empty source handoff admissible.
        source["handoff"]["source_state"]["documents"] = []
        raw = encode(source["handoff"])
        path.write_bytes(raw)
        target["source_handoff_sha256"] = sha(raw)
        authorization["source_handoff_sha256"] = sha(raw)
        raw = encode(authorization)
        authorization_path.write_bytes(raw)
        target["authorization_sha256"] = sha(raw)
        result, observed = self.run_case()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(observed["log"], [])

    def assert_handoff_only_mutation_fails(self, scenario):
        result, state = self.run_case(scenario)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "")
        self.assertNotIn("cleanup is uncertain", result.stderr)
        self.assertEqual([row["action"] for row in state["log"]], ["preflight", "provision", "inspect", "probe", "broker",
            "probe", "broker", "probe", "query", "privacy", "handoff", "cleanup"])
        self.assertEqual([row["id"] for row in state["document_state"]["documents"]], [1, 2, 3])
        self.assertEqual(state["document_state"]["documents"][1]["archive_sha256"], "ac" * 32)
        self.assertEqual(len(state["ledger"]), 13)
        return state

    def test_handoff_only_document_id_mutation_fails_after_successful_source_verify(self):
        state = self.assert_handoff_only_mutation_fails("handoff-ids")
        self.assertEqual([row["id"] for row in state["handoff"]["source_state"]["documents"]], [101, 102, 103])

    def test_handoff_only_archive_hash_mutation_fails_after_successful_source_verify(self):
        state = self.assert_handoff_only_mutation_fails("handoff-archive")
        self.assertEqual(state["handoff"]["source_state"]["documents"][1]["archive_sha256"], "bb" * 32)

    def test_late_provision_result_fails_with_original_cleanup_reserve(self):
        now = time.time_ns() // 1000000
        self.config["work_deadline_ms"], self.config["cleanup_deadline_ms"] = now + 1200, now + 6000
        result, state = self.run_case("late-provision")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual([row["action"] for row in state["log"]], ["preflight", "provision", "cleanup"])
        self.assertEqual(state["log"][-1]["deadline_ms"], now + 6000)


unittest.main(argv=[sys.argv[0], *sys.argv[2:]], verbosity=2)
PY
