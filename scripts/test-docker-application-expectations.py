#!/usr/bin/env python3
"""Offline tests with authored native artifacts; no runtime or BoxFerry output."""

from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import pathlib
import re
import subprocess
import sys
import unittest
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("docker_application_expectations", ROOT / "scripts/lib/docker-application-expectations.py")
assert SPEC is not None and SPEC.loader is not None
contract = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(contract)
RAW = (ROOT / contract.CATALOGUE_PATH).read_bytes()
EXPECTED = json.loads(RAW)


def encode(value):
    return (json.dumps(value, separators=(",", ":")) + "\n").encode()


def request(path, body):
    return {"method": "POST", "path": "/v1.56/" + path, "body": body}


class ApplicationExpectations(unittest.TestCase):
    def setUp(self):
        self.profile = {"kind": "target", "build": {"kind": "upstream"}, "engine_release": "29.8.1",
                        "advertised_api_version": "1.56", "acquisition_api_version": "1.49",
                        "rendering_api_version": "1.56", "daemon_mode": "rootless", "evidence_sha256": "ab" * 32}
        self.application = "forgejo"
        self.lane = "upstream-rootless"
        self.images = {"forgejo": "registry.invalid/offline/forgejo:reviewed", "postgres": "registry.invalid/offline/postgres:reviewed"}
        self.labels = {"io.boxferry.live-run": "offline-run", "io.boxferry.application": "unit-forgejo"}
        # Native requests are authored here; never construct them from EXPECTED,
        # BoxFerry output, a renderer, or a normalized observed projection.
        self.plan = {"schema_version": 1, "context": copy.deepcopy(self.profile), "requests": [
            request("networks/create", {"Name": "unit-forge-backend", "Driver": "bridge", "Internal": True, "Labels": self.labels}),
            request("volumes/create", {"Name": "unit-forge-db", "Labels": self.labels}),
            request("volumes/create", {"Name": "unit-forge-data", "Labels": self.labels}),
            request("containers/create?name=unit-forge-db", {
                "Image": self.images["postgres"], "Labels": self.labels,
                "HostConfig": {"NetworkMode": "unit-forge-backend", "Mounts": [
                    {"Type": "volume", "Source": "unit-forge-db", "Target": "/var/lib/postgresql/data", "ReadOnly": False}]},
                "NetworkingConfig": {"EndpointsConfig": {"unit-forge-backend": {"Aliases": ["db"]}}}}),
            request("containers/create?name=unit-forge-app", {
                "Image": self.images["forgejo"], "Labels": self.labels,
                "ExposedPorts": {"3000/tcp": {}, "2222/tcp": {}},
                "HostConfig": {"NetworkMode": "unit-forge-backend", "Mounts": [
                    {"Type": "volume", "Source": "unit-forge-data", "Target": "/var/lib/gitea", "ReadOnly": False}],
                    "PortBindings": {"3000/tcp": [{"HostIp": "127.0.0.1", "HostPort": "13000"}],
                                     "2222/tcp": [{"HostIp": "127.0.0.1", "HostPort": "12222"}]}},
                "NetworkingConfig": {"EndpointsConfig": {"unit-forge-backend": {"Aliases": ["forgejo"]}}}}),
            request("networks/unit-shared-edge/connect", {"Container": "unit-forge-app", "EndpointConfig": {"Aliases": ["forgejo"]}})
        ], "prerequisites": [{"kind": "network", "reference": "18446744073709551615", "identity": "unit-shared-edge", "expected_driver": "bridge"}]}

    def admission(self, raw):
        # These are declared requirements, not fabricated success observations.
        requirements = EXPECTED["applications"][self.application]
        return {"schema_version": 1, "kind": "boxferry-docker-application-offline-admission",
                "evidence_kind": "offline-contract-prerequisite", "native_execution": False,
                "source_kind": "authored-compose",
                "application": self.application, "lane": self.lane, "prefix": "unit", "run_id": "offline-run",
                "fixture_root": "/tmp/offline-fixture", "docker_plan_sha256": hashlib.sha256(raw).hexdigest(),
                "expectations_sha256": hashlib.sha256(RAW).hexdigest(),
                "source_sha256": contract.source_digest(requirements["sources"]),
                "required_checks": copy.deepcopy(requirements["required_checks"]),
                "dependencies": copy.deepcopy(requirements["dependencies"]),
                "excluded_peers": copy.deepcopy(requirements["excluded_peers"]),
                "shared_services": copy.deepcopy(requirements["shared_services"]),
                "runtime_evidence": "unmeasured", "budget_measurements": None}

    def validate(self, plan=None, admission=None, raw=None, **overrides):
        raw = encode(self.plan if plan is None else plan) if raw is None else raw
        options = {"catalogue_bytes": RAW, "application": self.application, "lane": self.lane,
                   "profile": self.profile, "image_aliases": self.images, "prefix": "unit",
                   "run_id": "offline-run", "fixture_root": "/tmp/offline-fixture"}
        options.update(overrides)
        return contract.validate_application(raw, encode(self.admission(raw) if admission is None else admission), **options)

    def test_authored_forgejo_topology_is_only_offline_prerequisite(self):
        result = self.validate()
        self.assertEqual(result["services"], ["db", "forgejo"])
        self.assertIs(result["native_execution"], False)
        self.assertEqual(result["runtime_evidence"], "unmeasured")
        self.assertEqual(result["evidence_kind"], "offline-contract-prerequisite")

    def assert_superseded_admission_bindings_reject(self):
        # Exact pre-ADR-0073 prospective bindings, not rewritten historical receipts.
        old_sources = {
            "observability": "7e0a2db171dda5fde3558b259b80aadc2888dc2fa9d7e219fd2f8ba416d8a6c9",
            "supabase": "0b1b5272a6bd98038098cc54808bffd9234c9de6a3ae630470aade7a78974b3d",
        }
        raw = encode(self.plan)
        for field, old_digest in (
            ("expectations_sha256", "81788c41a28c8950fdd6fc59ca3ab54cb247ee0a7841f6f77839e4a318196687"),
            ("source_sha256", old_sources[self.application]),
        ):
            with self.subTest(application=self.application, stale_binding=field):
                admission = self.admission(raw)
                self.assertNotEqual(admission[field], old_digest)
                admission[field] = old_digest
                with self.assertRaisesRegex(contract.ExpectationError, "identity binding differs"):
                    self.validate(admission=admission)

    def test_independent_six_application_inventories(self):
        inventories = {
            "forgejo": {"db", "forgejo"},
            "nextcloud": {"db", "cache", "app", "init", "cron", "frontend"},
            "paperless-ngx": {"db", "broker", "gotenberg", "tika", "webserver"},
            "immich": {"database", "redis", "immich-machine-learning", "immich-server"},
            "observability": {"metrics-producer", "log-producer", "prometheus", "loki", "alloy", "grafana"},
            "supabase": {"db", "auth", "rest", "realtime", "imgproxy", "storage", "meta", "supavisor", "functions", "studio", "kong"}}
        catalogue = contract.catalogue(RAW)
        self.assertEqual(set(catalogue["applications"]), set(inventories))
        for app, services in inventories.items():
            with self.subTest(application=app):
                self.assertEqual(set(catalogue["applications"][app]["services"]), services)
                self.assertTrue(catalogue["applications"][app]["networks"]["backend"]["internal"])
        cloud = catalogue["applications"]["nextcloud"]
        self.assertEqual(cloud["shared_services"][0]["runtime_suffix"], "shared-proxy")
        self.assertEqual(cloud["shared_services"][0]["ingress"],
                         [{"host_ip": "127.0.0.1", "host_port": 18443, "container_port": 8080, "protocol": "tcp"}])
        self.assertEqual(cloud["excluded_peers"][0]["runtime_suffix"], "second-app")
        self.assertIn({"service": "cron", "dependency": "init", "condition": "completed_successfully", "required": True, "restart": False}, cloud["dependencies"])

    def supabase(self):
        self.application = "supabase"
        self.labels = {"io.boxferry.live-run": "offline-run", "io.boxferry.application": "unit-supabase"}
        names = ["db", "auth", "rest", "realtime", "imgproxy", "storage", "meta", "supavisor", "functions", "studio", "kong"]
        self.images = {key: f"registry.invalid/offline/{key}:reviewed" for key in names}
        self.plan = {"schema_version": 1, "context": copy.deepcopy(self.profile), "requests": [
            request("networks/create", {"Name": "unit-supabase-backend", "Driver": "bridge", "Internal": True, "Labels": self.labels}),
            *[request("volumes/create", {"Name": f"unit-supabase-{name}", "Labels": self.labels}) for name in ["deno-cache", "pgdata", "storage"]]],
            "prerequisites": [{"kind": "network", "reference": "2", "identity": "unit-supabase-edge", "expected_driver": "bridge"}]}
        mounts = {
            "db": [("volume", "unit-supabase-pgdata", "/var/lib/postgresql/data", False),
                   ("bind", "/tmp/offline-fixture/db-init.sql", "/docker-entrypoint-initdb.d/zzzzzzzzzzzz-boxferry.sql", True)],
            "imgproxy": [("volume", "unit-supabase-storage", "/var/lib/storage", True)],
            "storage": [("volume", "unit-supabase-storage", "/var/lib/storage", False)],
            "functions": [("bind", "/tmp/offline-fixture/functions", "/home/deno/functions", True),
                          ("volume", "unit-supabase-deno-cache", "/root/.cache/deno", False)],
            "studio": [("bind", "/tmp/offline-fixture", "/boxferry-fixture", True)],
            "kong": [("bind", "/tmp/offline-fixture/kong.yml", "/etc/kong/kong.yml", True)]}
        for name in names:
            body = {"Image": self.images[name], "Labels": dict(self.labels),
                    "HostConfig": {"NetworkMode": "unit-supabase-backend"},
                    "NetworkingConfig": {"EndpointsConfig": {"unit-supabase-backend": {
                        "Aliases": ["realtime-dev.supabase-realtime", "realtime"] if name == "realtime" else [name]}}}}
            if name in mounts:
                body["HostConfig"]["Mounts"] = [{"Type": kind, "Source": source, "Target": target, "ReadOnly": ro}
                                                for kind, source, target, ro in mounts[name]]
            if name == "storage":
                body["Labels"]["io.boxferry.selection"] = "storage"
            if name == "kong":
                body["ExposedPorts"] = {"8000/tcp": {}}
                body["HostConfig"]["PortBindings"] = {"8000/tcp": [{"HostIp": "127.0.0.1", "HostPort": "18000"}]}
            self.plan["requests"].append(request(f"containers/create?name=unit-supabase-{name}", body))
        self.plan["requests"].append(request("networks/unit-supabase-edge/connect", {
            "Container": "unit-supabase-kong", "EndpointConfig": {"Aliases": ["supabase"]}}))

    def test_full_eleven_service_supabase_and_shared_mount_access(self):
        self.supabase()
        self.assertEqual(len(self.validate()["services"]), 11)
        self.assert_superseded_admission_bindings_reject()
        for missing in ["db", "auth", "rest", "realtime", "functions", "supavisor", "studio", "imgproxy", "meta", "storage", "kong"]:
            with self.subTest(missing=missing):
                plan = copy.deepcopy(self.plan)
                plan["requests"] = [row for row in plan["requests"] if row["path"] != f"/v1.56/containers/create?name=unit-supabase-{missing}"]
                with self.assertRaises(contract.ExpectationError):
                    self.validate(plan)
        for service, target in [("imgproxy", "/var/lib/storage"), ("functions", "/home/deno/functions")]:
            with self.subTest(read_only=service):
                plan = copy.deepcopy(self.plan)
                body = next(row["body"] for row in plan["requests"] if row["path"].endswith(f"name=unit-supabase-{service}"))
                next(mount for mount in body["HostConfig"]["Mounts"] if mount["Target"] == target)["ReadOnly"] = False
                with self.assertRaises(contract.ExpectationError):
                    self.validate(plan)

    def test_four_other_independently_authored_application_artifacts(self):
        # A separate, literal source of native test topology. These rows do not
        # consume the expectation catalogue's service/mount/attachment fields.
        # Tuple: runtime suffix, image role, backend aliases, edge aliases or
        # None, (kind/source/target/read-only) mounts, published port or None.
        topologies = {
            "nextcloud": ("cloud", "cloud", "shared-edge", True, ["database", "redis", "nextcloud"], [
                ("cloud-db", "postgres", ["db"], None, [("volume", "database", "/var/lib/postgresql/data", False)], None),
                ("cloud-cache", "redis", ["cache"], None, [("volume", "redis", "/data", False)], None),
                ("cloud-app", "nextcloud", ["app"], None, [("volume", "nextcloud", "/var/www/html", False)], None),
                ("cloud-init", "nextcloud", [], None, [("volume", "nextcloud", "/var/www/html", False)], None),
                ("cloud-cron", "nextcloud", [], None, [("volume", "nextcloud", "/var/www/html", False)], None),
                ("cloud-frontend", "nginx", ["frontend"], ["frontend"], [("volume", "nextcloud", "/var/www/html", True),
                     ("bind", "frontend.conf", "/etc/nginx/conf.d/default.conf", True)], None)]),
            "paperless-ngx": ("paperless", "paper", "paper-edge", False, ["data", "media", "consume", "export", "pgdata", "redisdata"], [
                ("paper-db", "postgres", ["db"], None, [("volume", "pgdata", "/var/lib/postgresql", False)], None),
                ("paper-broker", "valkey", ["broker"], None, [("volume", "redisdata", "/data", False)], None),
                ("paper-gotenberg", "gotenberg", ["gotenberg"], None, [], None),
                ("paper-tika", "tika", ["tika"], None, [], None),
                ("paper-web", "paperless", ["webserver"], ["paperless"], [
                    ("volume", "data", "/usr/src/paperless/data", False), ("volume", "media", "/usr/src/paperless/media", False),
                    ("volume", "consume", "/usr/src/paperless/consume", False), ("volume", "export", "/usr/src/paperless/export", False),
                    ("bind", ".", "/fixture", False)], (18000, 8000))]),
            "immich": ("immich", "immich", "immich-edge", False, ["library", "model-cache", "pgdata", "redisdata"], [
                ("immich-database", "postgres", ["database"], None, [("volume", "pgdata", "/var/lib/postgresql/data", False)], None),
                ("immich-redis", "valkey", ["redis"], None, [("volume", "redisdata", "/data", False)], None),
                ("immich-machine-learning", "machine-learning", ["immich-machine-learning"], None,
                    [("volume", "model-cache", "/cache", False), ("bind", ".", "/fixture", False)], None),
                ("immich-server", "server", [], ["immich-server"],
                    [("volume", "library", "/data", False)], (18283, 2283))]),
            "observability": ("observability", "observability", "observability-edge", True,
                              ["alloy-data", "grafana-data", "loki-data", "prometheus-data", "telemetry-logs"], [
                ("observability-metrics-producer", "producer", ["metrics-producer"], None, [("bind", ".", "/fixture", True)], None),
                ("observability-log-producer", "producer", [], None,
                    [("bind", ".", "/fixture", True), ("volume", "telemetry-logs", "/var/log/boxferry", False)], None),
                ("observability-prometheus", "prometheus", ["prometheus"], None,
                    [("bind", "prometheus.yml", "/etc/prometheus/prometheus.yml", True), ("volume", "prometheus-data", "/prometheus", False)], None),
                ("observability-loki", "loki", ["loki"], None,
                    [("bind", "loki.yaml", "/etc/loki/loki.yaml", True), ("volume", "loki-data", "/loki", False)], None),
                ("observability-alloy", "alloy", [], None,
                    [("bind", "config.alloy", "/etc/alloy/config.alloy", True), ("volume", "alloy-data", "/var/lib/alloy/data", False),
                     ("volume", "telemetry-logs", "/var/log/boxferry", True)], None),
                ("observability-grafana", "grafana", [], ["grafana"], [
                    ("bind", "grafana-datasources.yaml", "/etc/grafana/provisioning/datasources/boxferry.yaml", True),
                    ("bind", "grafana-dashboards.yaml", "/etc/grafana/provisioning/dashboards/boxferry.yaml", True),
                    ("bind", "dashboard.json", "/etc/grafana/provisioning/boxferry-dashboards/boxferry.json", True),
                    ("volume", "grafana-data", "/var/lib/grafana", False)], (13000, 3000))])}
        for app, (owner, stem, edge, shared, volumes, services) in topologies.items():
            with self.subTest(application=app):
                self.application = app
                self.images = {row[1]: f"registry.invalid/offline/{row[1]}:reviewed" for row in services}
                labels = {"io.boxferry.live-run": "offline-run", "io.boxferry.application": "unit-" + owner}
                backend, edge_name = f"unit-{stem}-backend", "unit-" + edge
                requests = [request("networks/create", {"Name": backend, "Driver": "bridge", "Internal": True, "Labels": labels})]
                if not shared:
                    requests.append(request("networks/create", {"Name": edge_name, "Driver": "bridge", "Labels": labels}))
                requests.extend(request("volumes/create", {"Name": f"unit-{stem}-{volume}", "Labels": labels}) for volume in volumes)
                for suffix, image, aliases, edge_aliases, mounts, port in services:
                    name = "unit-" + suffix
                    body = {"Image": self.images[image], "Labels": labels, "HostConfig": {"NetworkMode": backend},
                            "NetworkingConfig": {"EndpointsConfig": {backend: {"Aliases": aliases} if aliases else {}}}}
                    if mounts:
                        body["HostConfig"]["Mounts"] = [{"Type": kind,
                            "Source": f"unit-{stem}-{source}" if kind == "volume" else (
                                "/tmp/offline-fixture" if source == "." else "/tmp/offline-fixture/" + source),
                            "Target": target, "ReadOnly": ro} for kind, source, target, ro in mounts]
                    if port:
                        host, container = port
                        body["ExposedPorts"] = {f"{container}/tcp": {}}
                        body["HostConfig"]["PortBindings"] = {f"{container}/tcp": [{"HostIp": "127.0.0.1", "HostPort": str(host)}]}
                    requests.append(request("containers/create?name=" + name, body))
                    if edge_aliases is not None:
                        requests.append(request(f"networks/{edge_name}/connect", {"Container": name, "EndpointConfig": {"Aliases": edge_aliases}}))
                self.plan = {"schema_version": 1, "context": copy.deepcopy(self.profile), "requests": requests,
                             "prerequisites": [{"kind": "network", "reference": "0", "identity": edge_name, "expected_driver": "bridge"}] if shared else []}
                self.assertEqual(len(self.validate()["services"]), len(services))
                if app == "observability":
                    self.assert_superseded_admission_bindings_reject()
                alias_free = {"immich": ["immich-server"], "observability": [
                    "observability-log-producer", "observability-alloy", "observability-grafana"]}.get(app, [])
                for suffix in alias_free:
                    with self.subTest(invented_alias=suffix):
                        plan = copy.deepcopy(self.plan)
                        body = next(row["body"] for row in plan["requests"] if row["path"].endswith("name=unit-" + suffix))
                        body["NetworkingConfig"]["EndpointsConfig"][backend]["Aliases"] = [suffix]
                        with self.assertRaisesRegex(contract.ExpectationError, "alias graph"):
                            self.validate(plan)
                if app == "immich":
                    plan = copy.deepcopy(self.plan)
                    body = next(row["body"] for row in plan["requests"] if row["path"].endswith("name=unit-immich-server"))
                    body["HostConfig"]["Mounts"].append({"Type": "bind", "Source": "/tmp/offline-fixture", "Target": "/fixture", "ReadOnly": False})
                    with self.assertRaisesRegex(contract.ExpectationError, "mount inventory"):
                        self.validate(plan)
                for suffix, *_ in services:
                    plan = copy.deepcopy(self.plan)
                    plan["requests"] = [row for row in plan["requests"] if row["path"] != f"/v1.56/containers/create?name=unit-{suffix}"]
                    with self.assertRaises(contract.ExpectationError):
                        self.validate(plan)

    def test_resource_topology_mutations_reject_with_fresh_digest(self):
        def mount(plan):
            return plan["requests"][4]["body"]["HostConfig"]["Mounts"][0]
        mutations = {
            "missing service": lambda p: p["requests"].pop(3),
            "missing volume": lambda p: p["requests"].pop(1),
            "extra volume": lambda p: p["requests"].insert(1, request("volumes/create", {"Name": "unit-extra", "Labels": self.labels})),
            "missing network": lambda p: p["requests"].pop(0),
            "external network create": lambda p: p["requests"].insert(0, request("networks/create", {"Name": "unit-shared-edge", "Driver": "bridge", "Labels": self.labels})),
            "backend public": lambda p: p["requests"][0]["body"].update(Internal=False),
            "integer internal": lambda p: p["requests"][0]["body"].update(Internal=1),
            "wrong owner": lambda p: p["requests"][0]["body"].update(Labels={"io.boxferry.live-run": "offline-run"}),
            "wrong service owner": lambda p: p["requests"][3]["body"].update(Labels={}),
            "swapped images": lambda p: p["requests"][3]["body"].update(Image=self.images["forgejo"]),
            "swapped mount": lambda p: mount(p).update(Source="unit-forge-db"),
            "host socket": lambda p: mount(p).update(Type="bind", Source="/var/run/docker.sock"),
            "wrong mount mode": lambda p: mount(p).update(ReadOnly=True),
            "integer mount mode": lambda p: mount(p).update(ReadOnly=0),
            "bad alias": lambda p: p["requests"][-1]["body"]["EndpointConfig"].update(Aliases=["db"]),
            "extra alias": lambda p: p["requests"][-1]["body"]["EndpointConfig"].update(Aliases=["forgejo", "peer"]),
            "missing edge attachment": lambda p: p["requests"].pop(),
            "duplicate attachment": lambda p: p["requests"].append(copy.deepcopy(p["requests"][-1])),
            "unknown endpoint field": lambda p: p["requests"][-1]["body"]["EndpointConfig"].update(IPAMConfig={}),
            "ingress wildcard": lambda p: p["requests"][4]["body"]["HostConfig"]["PortBindings"]["3000/tcp"][0].update(HostIp="0.0.0.0"),
            "extra exposed port": lambda p: p["requests"][3]["body"].update(ExposedPorts={"5432/tcp": {}}),
            "privileged field": lambda p: p["requests"][3]["body"]["HostConfig"].update(Privileged=True),
            "host network": lambda p: p["requests"][3]["body"]["HostConfig"].update(NetworkMode="host"),
            "start operation": lambda p: p["requests"].append(request("containers/unit-forge-app/start", {})),
            "peer create": lambda p: p["requests"].append(request("containers/create?name=unit-peer-app", {})),
            "wrong request API": lambda p: p["requests"][0].update(path="/v1.41/networks/create"),
            "wrong method": lambda p: p["requests"][0].update(method="GET"),
            "missing prerequisite": lambda p: p.update(prerequisites=[]),
            "wrong prerequisite": lambda p: p["prerequisites"][0].update(identity="unit-other-edge"),
            "wrong external driver": lambda p: p["prerequisites"][0].update(expected_driver="host"),
            "numeric reference": lambda p: p["prerequisites"][0].update(reference=2),
            "oversized u64 reference": lambda p: p["prerequisites"][0].update(reference="18446744073709551616"),
            "external volume": lambda p: p["prerequisites"].append({"kind": "volume", "reference": "4", "identity": "data"}),
            "wrong context": lambda p: p["context"].update(daemon_mode="rootful"),
            "unknown plan field": lambda p: p.update(success=True),
            "boolean schema": lambda p: p.update(schema_version=True),
        }
        for name, mutate in mutations.items():
            with self.subTest(mutation=name):
                plan = copy.deepcopy(self.plan)
                mutate(plan)
                with self.assertRaises(contract.ExpectationError):
                    self.validate(plan)

    def test_extra_native_fields_cannot_bypass_closed_topology(self):
        body_fields = {"Volumes": {"/unexpected-data": {}}, "Hostname": "db-redirect",
                       "Domainname": "example.invalid", "MacAddress": "02:00:00:00:00:01",
                       "unexpected": True}
        host_fields = {"PublishAllPorts": True, "ExtraHosts": ["db:192.0.2.1"],
                       "Dns": ["192.0.2.2"], "DnsSearch": ["example.invalid"],
                       "Links": ["unit-peer-app:db"], "VolumesFrom": ["unit-peer-app"],
                       "Binds": ["/var/run/docker.sock:/socket"], "Tmpfs": {"/extra": "rw"},
                       "RestartPolicy": {"Name": "always"}, "ReadonlyRootfs": False,
                       "unexpected": False}
        for field, value in body_fields.items():
            with self.subTest(body_field=field):
                plan = copy.deepcopy(self.plan)
                plan["requests"][3]["body"][field] = value
                with self.assertRaisesRegex(contract.ExpectationError, "native fields"):
                    self.validate(plan)
        for field, value in host_fields.items():
            with self.subTest(host_field=field):
                plan = copy.deepcopy(self.plan)
                plan["requests"][3]["body"]["HostConfig"][field] = value
                with self.assertRaisesRegex(contract.ExpectationError, "host fields"):
                    self.validate(plan)

    def test_known_deferred_semantic_fields_do_not_claim_fidelity(self):
        plan = copy.deepcopy(self.plan)
        plan["requests"][3]["body"].update(Env=["DEFERRED=fixture"], Cmd=["deferred-command"],
            Entrypoint=[], Healthcheck={"Test": ["CMD", "deferred-health"], "Interval": 2_000_000_000},
            User="deferred-user", WorkingDir="/deferred", StopSignal="SIGTERM", StopTimeout=5)
        self.assertEqual(self.validate(plan)["runtime_evidence"], "unmeasured")
        for field, value in [("Cmd", "not-an-array"), ("Env", [False]), ("Entrypoint", [None]),
                             ("Healthcheck", {"Test": ["CMD", "probe"], "unknown": True}),
                             ("Healthcheck", {"Test": ["CMD"], "Interval": True}), ("StopTimeout", True)]:
            with self.subTest(field=field):
                changed = copy.deepcopy(plan)
                changed["requests"][3]["body"][field] = value
                with self.assertRaises(contract.ExpectationError):
                    self.validate(changed)

    def test_null_and_short_network_syntax_never_invent_aliases_or_mounts(self):
        def block(application, service):
            source = (ROOT / f"fixtures/conformance/{application}-application/compose.yaml").read_text()
            matched = re.search(r"^  " + re.escape(service) + r":\n((?:    .*\n|[ \t]*\n)+)", source, re.MULTILINE)
            self.assertIsNotNone(matched, service)
            return matched.group(1)

        server = block("immich", "immich-server")
        self.assertIn("    networks:\n      backend:\n      edge:\n        aliases: [immich-server]\n", server)
        self.assertIn("    volumes:\n      - library:/data\n", server)
        self.assertNotIn("/fixture", server)
        expected_server = EXPECTED["applications"]["immich"]["services"]["immich-server"]
        self.assertEqual(expected_server["networks"], {"backend": [], "edge": ["immich-server"]})
        self.assertEqual(expected_server["mounts"], [{"kind": "volume", "source": "library", "target": "/data", "read_only": False}])
        ml = block("immich", "immich-machine-learning")
        self.assertIn("      - ${BF_FIXTURE_ROOT:?required}:/fixture:rw\n", ml)
        for service in ("log-producer", "alloy"):
            self.assertIn("    networks: [backend]\n", block("observability", service))
            self.assertEqual(EXPECTED["applications"]["observability"]["services"][service]["networks"], {"backend": []})
        self.assertIn("    networks:\n      backend:\n      edge:\n        aliases: [grafana]\n", block("observability", "grafana"))
        self.assertEqual(EXPECTED["applications"]["observability"]["services"]["grafana"]["networks"],
                         {"backend": [], "edge": ["grafana"]})
        for service in ("init", "cron"):
            self.assertIn("    networks: [backend]\n", block("nextcloud", service))
            self.assertEqual(EXPECTED["applications"]["nextcloud"]["services"][service]["networks"], {"backend": []})

    def test_creation_order_must_not_allow_implicit_resources(self):
        plan = copy.deepcopy(self.plan)
        plan["requests"][1], plan["requests"][3] = plan["requests"][3], plan["requests"][1]
        with self.assertRaises(contract.ExpectationError):
            self.validate(plan)
        plan = copy.deepcopy(self.plan)
        plan["requests"].insert(0, plan["requests"].pop())
        with self.assertRaises(contract.ExpectationError):
            self.validate(plan)

    def test_admission_may_not_claim_runtime_proof_or_weaken_requirements(self):
        raw = encode(self.plan)
        for key, value in [("native_execution", True), ("schema_version", True), ("runtime_evidence", "passed"),
                           ("budget_measurements", {"wall_seconds": 1}), ("docker_plan_sha256", "0" * 64),
                           ("expectations_sha256", "0" * 64), ("source_sha256", "0" * 64), ("run_id", "other"),
                           ("prefix", "other"), ("fixture_root", "/other"), ("dependencies", []),
                           ("source_kind", "acquired-podman"),
                           ("required_checks", {}), ("excluded_peers", []), ("extra", True)]:
            with self.subTest(field=key):
                admission = self.admission(raw)
                admission[key] = value
                with self.assertRaises(contract.ExpectationError):
                    self.validate(admission=admission)
        with self.assertRaises(contract.ExpectationError):
            self.validate(raw=raw + b" ", admission=self.admission(raw))

    def test_finite_lane_and_version_boundaries(self):
        for lane in ["debian11-rootless", "debian11-rootful", "macos", "swarm", "unknown"]:
            with self.subTest(lane=lane), self.assertRaises(contract.ExpectationError):
                self.validate(lane=lane)
        for mode in ["rootless", "rootful"]:
            self.profile["daemon_mode"] = mode
            self.plan["context"] = copy.deepcopy(self.profile)
            self.lane = "upstream-" + mode
            self.validate()
        self.application = "immich"
        with self.assertRaisesRegex(contract.ExpectationError, "finite approved scope"):
            self.validate()
        self.application = "forgejo"
        for key, value in [("rendering_api_version", "1.57"), ("acquisition_api_version", "1.57"),
                           ("rendering_api_version", "v1.56"), ("build", {"kind": "debian_package"}),
                           ("evidence_sha256", ""), ("kind", "observed")]:
            profile = copy.deepcopy(self.profile)
            profile[key] = value
            with self.subTest(field=key), self.assertRaises(contract.ExpectationError):
                self.validate(profile=profile)

    def test_malformed_and_bounded_json(self):
        for raw in [b"{}", b"\xff", b"[]", b'{"a":1,"a":2}', b'{"a":NaN}', b'{"a":Infinity}',
                    b'{"a":1e1000}', b"x" * (contract.LIMIT + 1), b"[" * 40 + b"0" + b"]" * 40]:
            with self.subTest(raw=raw[:20]), self.assertRaises(contract.ExpectationError):
                self.validate(raw=raw)

    def test_reviewed_source_binding_matches_actual_canonical_bytes(self):
        self.assertEqual(set(contract.check_sources(ROOT, RAW)), contract.APPLICATIONS)
        changed = copy.deepcopy(EXPECTED)
        changed["applications"]["forgejo"]["sources"][0]["sha256"] = "0" * 64
        with self.assertRaisesRegex(contract.ExpectationError, "source bytes differ"):
            contract.check_sources(ROOT, encode(changed))

    def test_protected_alias_semantic_sources_are_required_and_mutation_bound(self):
        sources = [
            ("observability", "scripts/lib/protected-service-alias-contract.py",
             b"os.path.lexists(destination)", b"False"),
            ("supabase", "scripts/lib/protected-service-alias-contract.py",
             b"read_regular(report, 8388608)", b"read_regular(report, 16777216)"),
            ("supabase", "scripts/lib/supabase-application.sh",
             b'supabase_assert_protected_service_alias_refusal "${report}"', b': "${report}"'),
        ]
        for app, path, original_policy, replacement in sources:
            with self.subTest(application=app, source=path):
                records = [row for row in EXPECTED["applications"][app]["sources"] if row["path"] == path]
                self.assertEqual(len(records), 1)
                source_path = ROOT / path
                original = source_path.read_bytes()
                self.assertEqual(records[0]["sha256"], hashlib.sha256(original).hexdigest())
                changed = original.replace(original_policy, replacement, 1)
                self.assertNotEqual(original, changed)
                read_bytes = pathlib.Path.read_bytes
                with mock.patch.object(pathlib.Path, "read_bytes", autospec=True,
                                       side_effect=lambda candidate: changed if candidate == source_path else read_bytes(candidate)):
                    with self.assertRaisesRegex(contract.ExpectationError, "source bytes differ"):
                        contract.check_sources(ROOT, RAW)
                missing = copy.deepcopy(EXPECTED)
                missing["applications"][app]["sources"] = [row for row in missing["applications"][app]["sources"]
                                                         if row["path"] != path]
                with self.assertRaisesRegex(contract.ExpectationError, "semantic source binding is absent"):
                    contract.catalogue(encode(missing))

    def test_observability_boundary_peer_and_probe_source_mutation_invalidates_binding(self):
        path = "scripts/lib/observability-application.sh"
        records = [row for row in EXPECTED["applications"]["observability"]["sources"] if row["path"] == path]
        self.assertEqual(len(records), 1, "boundary-peer/probe defining source must be bound exactly once")
        source_path = ROOT / path
        original = source_path.read_bytes()
        self.assertEqual(records[0]["sha256"], hashlib.sha256(original).hexdigest())
        changed = original.replace(b'--label "io.boxferry.application=${prefix}-boundary"',
                                   b'--label "io.boxferry.application=${prefix}-observability"')
        self.assertNotEqual(original, changed, "mutation must change real boundary-peer ownership")
        read_bytes = pathlib.Path.read_bytes
        # Substitute independently mutated source bytes without writing or
        # executing the harness. Every other source still uses its actual bytes.
        with mock.patch.object(pathlib.Path, "read_bytes", autospec=True,
                               side_effect=lambda candidate: changed if candidate == source_path else read_bytes(candidate)):
            with self.assertRaisesRegex(contract.ExpectationError, "source bytes differ"):
                contract.check_sources(ROOT, RAW)

    def test_supabase_diagnostic_source_retains_independent_migration_tuples(self):
        path = "fixtures/conformance/supabase-application/routes.tsv"
        records = [row for row in EXPECTED["applications"]["supabase"]["sources"] if row["path"] == path]
        self.assertEqual(len(records), 1, "migration route source must be bound exactly once")
        source_path = ROOT / path
        original = source_path.read_bytes()
        self.assertEqual(records[0]["sha256"], hashlib.sha256(original).hexdigest())
        # Independently reviewed migration semantics: reconstruction notes do not
        # authorize live success, change a route, or weaken exact tuple comparison.
        routes = [
            ("podman", "compose", "BFP0002,BFP0003,BFP0009,BFC0007"),
            ("podman", "quadlet", "BFP0002,BFP0003,BFP0009,BFQ0003"),
            ("podman", "podman", "BFP0002,BFP0003,BFP0009,BFP0007"),
            ("compose", "compose", "-"),
            ("compose", "quadlet", "BFQ0003"),
            ("compose", "podman", "BFP0007"),
            ("quadlet", "compose", "BFC0007"),
            ("quadlet", "quadlet", "-"),
            ("quadlet", "podman", "BFP0007"),
        ]
        expected = [
            (source, target, "known-migration-gap" if (source, target) == ("quadlet", "compose") else "migration-success",
             "live-unperformed", codes,
             "protected-service-alias-refusal-or-zero-alias-positive-v1" if (source, target) == ("quadlet", "compose") else
             "zero-loss-zero-diagnostic-reimport" if codes == "-" else
             "exact-diagnostic-tuple-multiset-plus-loss-fidelity-v1")
            for source, target, codes in routes
        ]
        actual = [tuple(line.split("\t")) for line in original.decode().splitlines()
                  if line and not line.startswith("#")]
        self.assertEqual(actual, expected)
        self.assertEqual(sum(row[2] == "migration-success" for row in actual), 8)
        self.assertEqual(sum(row[2] == "known-migration-gap" for row in actual), 1)
        obsolete_success = original.replace(
            b"quadlet\tcompose\tknown-migration-gap", b"quadlet\tcompose\tmigration-success", 1)
        self.assertNotEqual(original, obsolete_success)
        read_bytes = pathlib.Path.read_bytes
        with mock.patch.object(pathlib.Path, "read_bytes", autospec=True,
                               side_effect=lambda candidate: obsolete_success if candidate == source_path else read_bytes(candidate)):
            with self.assertRaisesRegex(contract.ExpectationError, "source bytes differ"):
                contract.check_sources(ROOT, RAW)
        changed = original.replace(b"live-unperformed", b"live-success", 1)
        self.assertNotEqual(original, changed, "mutation must change the actual evidence claim")
        read_bytes = pathlib.Path.read_bytes
        with mock.patch.object(pathlib.Path, "read_bytes", autospec=True,
                               side_effect=lambda candidate: changed if candidate == source_path else read_bytes(candidate)):
            with self.assertRaisesRegex(contract.ExpectationError, "source bytes differ"):
                contract.check_sources(ROOT, RAW)

    def test_catalogue_schema_and_required_supabase_services_fail_closed(self):
        mutations = [lambda c: c.update(schema_version=2), lambda c: c.update(native_execution=True),
                     lambda c: c["applications"].pop("immich"),
                     lambda c: c["applications"]["supabase"]["services"].pop("supavisor"),
                     lambda c: c["applications"]["forgejo"]["networks"]["backend"].update(internal=False),
                     lambda c: c["applications"]["forgejo"]["sources"][0].update(path="../outside"),
                     lambda c: c["applications"]["forgejo"]["dependencies"].append(
                         {"service": "db", "dependency": "forgejo", "condition": "started", "required": True, "restart": False})]
        for mutate in mutations:
            catalogue = copy.deepcopy(EXPECTED)
            mutate(catalogue)
            with self.assertRaises(contract.ExpectationError):
                contract.catalogue(encode(catalogue))

    def test_read_only_cli_source_check_and_failure(self):
        script = ROOT / "scripts/lib/docker-application-expectations.py"
        completed = subprocess.run([sys.executable, str(script), "check-sources"], capture_output=True, timeout=20)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(json.loads(completed.stdout)["evidence_kind"], "offline-contract-prerequisite")
        completed = subprocess.run([sys.executable, str(script), "validate"], capture_output=True, timeout=20)
        self.assertEqual(completed.returncode, 1)
        self.assertNotIn(str(ROOT).encode(), completed.stderr)


if __name__ == "__main__":
    unittest.main()
