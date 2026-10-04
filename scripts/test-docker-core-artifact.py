#!/usr/bin/env python3
"""Independent literal core expectations; all fixtures remain inert and offline."""
from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import os
import pathlib
import socket
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[1]
HELPER = ROOT / "scripts/lib/docker-core-artifact.py"
SPEC = importlib.util.spec_from_file_location("docker_core_artifact", HELPER)
assert SPEC is not None and SPEC.loader is not None
core = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(core)


def encode(value: object) -> bytes:
    return json.dumps(value, indent=2).encode()


def profile_for(lane: str) -> dict:
    debian = lane.startswith("debian11-")
    return {"kind": "target", "build": {"kind": "debian_package", "revision": "20.10.5+dfsg1-1+deb11u2"} if debian else {"kind": "upstream"},
            "engine_release": "20.10.5+dfsg1" if debian else "29.8.1", "advertised_api_version": "1.41" if debian else "1.56",
            "acquisition_api_version": "1.41" if debian else "1.56", "rendering_api_version": "1.41" if debian else "1.56",
            "daemon_mode": lane.rsplit("-", 1)[1], "evidence_sha256": "a" * 64}


class CoreArtifactTests(unittest.TestCase):
    def setUp(self) -> None:
        self.catalogue = (ROOT / "fixtures/conformance/docker-application/core-expectations.json").read_bytes()
        self.source = (ROOT / "fixtures/conformance/docker-application/core.compose.yaml").read_bytes()
        self.lane = "upstream-rootless"
        self.profile = profile_for(self.lane)
        self.alias = "registry.invalid/boxferry-core/busybox:independent-review"
        self.plan = self.artifact(self.profile)

    def artifact(self, profile: dict) -> dict:
        # Literal oracle: no implementation constants or rendered request supply expectations.
        return {"schema_version": 1, "context": copy.deepcopy(profile), "prerequisites": [], "requests": [
            {"method": "POST", "path": f"/v{profile['rendering_api_version']}/containers/create?name=bf-docker-core",
             "body": {"Image": "registry.invalid/boxferry-core/busybox:independent-review",
                      "Cmd": ["sh", "-c", "printf boxferry-core-ready > /tmp/boxferry-core-marker; exec sleep 3600"],
                      "HostConfig": {}}}]}

    def validate(self, raw: bytes | None = None, **changes: object) -> dict:
        raw = encode(self.plan) if raw is None else raw
        arguments = {"expectations_bytes": self.catalogue, "source_bytes": self.source,
                     "expected_plan_sha256": hashlib.sha256(raw).hexdigest(), "lane": self.lane,
                     "profile": self.profile, "image_alias": self.alias}
        arguments.update(changes)
        return core.validate_core(raw, **arguments)

    def reject(self, raw: bytes | None = None, **changes: object) -> None:
        with self.assertRaises(core.topology.ExpectationError):
            self.validate(raw, **changes)

    def test_four_authored_unadmitted_lanes_and_closed_result(self) -> None:
        for lane in ("debian11-rootful", "debian11-rootless", "upstream-rootful", "upstream-rootless"):
            with self.subTest(lane=lane):
                profile = profile_for(lane)
                raw = encode(self.artifact(profile))
                result = self.validate(raw, lane=lane, profile=profile)
                self.assertEqual(result, {
                    "schema_version": 1, "kind": "boxferry-docker-core-offline-prerequisite",
                    "evidence_kind": "offline-contract-prerequisite", "source_kind": "authored-compose", "lane": lane,
                    "native_execution": False, "replay_authority": False, "native_admission": False,
                    "runtime_evidence": "unmeasured", "budget_measurements": None,
                    "docker_plan_sha256": hashlib.sha256(raw).hexdigest(),
                    "expectations_sha256": hashlib.sha256(self.catalogue).hexdigest(),
                    "source_sha256": hashlib.sha256(self.source).hexdigest()})
                serialized = json.dumps(result)
                for protected in (self.alias, "boxferry-core-marker", "evidence_sha256", "requests"):
                    self.assertNotIn(protected, serialized)

    def test_literal_source_correspondence(self) -> None:
        self.assertEqual(self.source, b"---\nname: bf-docker-core\nservices:\n  bf-docker-core:\n    container_name: bf-docker-core\n    image: BOXFERRY_CORE_IMAGE_LITERAL\n    command:\n      - sh\n      - -c\n      - printf boxferry-core-ready > /tmp/boxferry-core-marker; exec sleep 3600\n")
        expected = json.loads(self.catalogue)
        self.assertEqual(expected["command"], ["sh", "-c", "printf boxferry-core-ready > /tmp/boxferry-core-marker; exec sleep 3600"])
        self.assertEqual(expected["source"], {"path": "fixtures/conformance/docker-application/core.compose.yaml",
                                             "sha256": hashlib.sha256(self.source).hexdigest()})

    def test_exact_raw_bindings_and_source_drift(self) -> None:
        raw = encode(self.plan)
        self.reject(raw + b"\n", expected_plan_sha256=hashlib.sha256(raw).hexdigest())
        self.validate(raw + b"\n")
        for digest in (None, True, "a" * 63, "A" * 64, "b" * 64):
            self.reject(expected_plan_sha256=digest)
        self.reject(source_bytes=self.source + b"\n")
        self.reject(source_bytes=b"")
        result = self.validate(expectations_bytes=self.catalogue + b"\n")
        self.assertEqual(result["expectations_sha256"], hashlib.sha256(self.catalogue + b"\n").hexdigest())

    def test_complete_artifact_and_literal_request_mutations(self) -> None:
        mutations = []
        for field in ("schema_version", "context", "requests", "prerequisites"):
            plan = copy.deepcopy(self.plan)
            del plan[field]
            mutations.append(plan)
        for schema in (True, 1.0, 0, 2, "1"):
            mutations.append(dict(self.plan, schema_version=schema))
        mutations.extend([dict(self.plan, extra=False), dict(self.plan, prerequisites=[{"kind": "image", "identity": self.alias}]),
                          dict(self.plan, prerequisites={}), dict(self.plan, requests=[]),
                          dict(self.plan, requests=self.plan["requests"] * 2), self.plan["requests"]])
        request = self.plan["requests"][0]
        request_mutations = [dict(request, method="GET"), dict(request, method="post"), dict(request, extra=None),
                             dict(request, path="/v1.56/containers/bf-docker-core/start"),
                             dict(request, path="/v1.55/containers/create?name=bf-docker-core"),
                             dict(request, path="/v1.56/containers/create?name=other"),
                             dict(request, path=request["path"] + "&extra=1")]
        for body in ({}, dict(request["body"], Image="registry.invalid/boxferry-core/busybox:swapped"),
                     dict(request["body"], Cmd=["sh", "-c", "exit 0"]), dict(request["body"], HostConfig={"NetworkMode": "host"}),
                     dict(request["body"], Env=[]), dict(request["body"], HostConfig=[])):
            request_mutations.append(dict(request, body=body))
        for key in ("method", "path", "body"):
            changed = copy.deepcopy(request)
            del changed[key]
            request_mutations.append(changed)
        mutations.extend(dict(self.plan, requests=[changed]) for changed in request_mutations)
        for index, plan in enumerate(mutations):
            with self.subTest(index=index):
                self.reject(encode(plan))

    def test_exact_independent_context_and_lane_selection(self) -> None:
        for key, changed in {"kind": "source", "build": {"kind": "upstream", "extra": "unknown"},
                             "engine_release": "29.8.2", "advertised_api_version": "1.55",
                             "acquisition_api_version": "1.55", "rendering_api_version": "1.55",
                             "daemon_mode": "rootful", "evidence_sha256": "b" * 64, "extra": None}.items():
            with self.subTest(field=key):
                plan = copy.deepcopy(self.plan)
                plan["context"][key] = changed
                self.reject(encode(plan))
        for lane in ("unknown", "debian11-rootless", "upstream-rootful", None, []):
            self.reject(lane=lane)
        for alias in ("busybox:latest", "BOXFERRY_CORE_IMAGE_LITERAL", self.alias + "/bad", self.alias + "\x00", None):
            self.reject(image_alias=alias)
        debian = profile_for("debian11-rootless")
        plan = self.artifact(debian)
        plan["context"]["build"]["revision"] = "20.10.5+dfsg1-1+deb11u3"
        self.reject(encode(plan), lane="debian11-rootless", profile=debian)

    def test_selected_profile_closedness_and_api_boundary(self) -> None:
        profiles = [None, {}, dict(self.profile, extra=None), dict(self.profile, build={"kind": "unknown"}),
                    dict(self.profile, evidence_sha256="bad"), dict(self.profile, engine_release="latest"),
                    dict(self.profile, rendering_api_version="1.57"), dict(self.profile, acquisition_api_version="1.57"),
                    dict(self.profile, advertised_api_version="1.9"), dict(self.profile, rendering_api_version=1.56)]
        for profile in profiles:
            self.reject(profile=profile)
        for api in ("1.40", "1.41", "1.56"):
            profile = dict(self.profile, advertised_api_version=api, acquisition_api_version=api, rendering_api_version=api)
            self.validate(encode(self.artifact(profile)), profile=profile)
        debian = profile_for("debian11-rootless")
        for revision in (None, "", "bad\nrevision", "a" * 257):
            selected = copy.deepcopy(debian)
            selected["build"]["revision"] = revision
            self.reject(encode(self.artifact(selected)), lane="debian11-rootless", profile=selected)

    def test_bounded_unambiguous_json_in_both_documents(self) -> None:
        malformed = [b"", b"{", b"\xff", b'{"x":1,"x":2}', b'{"x":NaN}', b'{"x":Infinity}', b'{"x":1e999}',
                     b"[" * 34 + b"0" + b"]" * 34, b" " * 1_048_577,
                     b'{"schema_version":1,"schema_version":1}']
        for raw in malformed:
            with self.subTest(raw=raw[:40]):
                with self.assertRaises(core.topology.ExpectationError):
                    core.topology.document(raw)
                self.reject(raw)
                self.reject(expectations_bytes=raw)
        raw = encode(self.plan)
        self.reject(raw.replace(b'"schema_version": 1', b'"schema_version": 1, "schema_version": 1'))
        self.reject(expectations_bytes=self.catalogue.replace(
            b'"schema_version": 1', b'"schema_version": 1, "schema_version": 1'))
        self.reject(raw + b" " * 1_048_576)

    def test_expectation_schema_is_closed(self) -> None:
        original = json.loads(self.catalogue)
        values = [dict(original, extra=False), dict(original, schema_version=True), dict(original, schema_version=2),
                  dict(original, native_execution=True), dict(original, kind="native-catalogue"),
                  dict(original, source_kind="runtime"), dict(original, container_name="other"),
                  dict(original, image_placeholder="other"), dict(original, command=[])]
        for field in original:
            value = copy.deepcopy(original)
            del value[field]
            values.append(value)
        for path in ("/tmp/core.compose.yaml", "../core.compose.yaml"):
            values.append(dict(original, source=dict(original["source"], path=path)))
        for lanes in ({}, dict(original["lanes"], extra={}), dict(original["lanes"], **{
                "debian11-rootless": {"build_kind": "upstream", "daemon_mode": "rootless"}})):
            values.append(dict(original, lanes=lanes))
        for value in values:
            self.reject(expectations_bytes=encode(value))

    def test_expectation_drift_cannot_change_literal_request(self) -> None:
        catalogue = json.loads(self.catalogue)
        catalogue["command"] = ["sh", "-c", "exit 0"]
        self.reject(expectations_bytes=encode(catalogue))

    def test_no_network_or_runtime_calls(self) -> None:
        with mock.patch.object(socket, "socket", side_effect=AssertionError("network forbidden")), \
             mock.patch.object(subprocess, "Popen", side_effect=AssertionError("process forbidden")), \
             mock.patch.object(os, "system", side_effect=AssertionError("runtime forbidden")):
            specification = importlib.util.spec_from_file_location("fresh_inert_core", HELPER)
            assert specification is not None and specification.loader is not None
            fresh = importlib.util.module_from_spec(specification)
            specification.loader.exec_module(fresh)
            self.validate()
            fresh.check_sources(ROOT, self.catalogue)

    def test_bounded_regular_file_reader(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            regular = root / "regular"
            regular.write_bytes(b"{}")
            self.assertEqual(core.schedule.read_document(regular), b"{}")
            (root / "link").symlink_to(regular)
            os.mkfifo(root / "fifo")
            (root / "large").write_bytes(b" " * 1_048_577)
            for path in (root / "link", root / "fifo", root / "large", root, root / "missing"):
                with self.subTest(path=path.name), self.assertRaises((OSError, core.topology.ExpectationError)):
                    core.schedule.read_document(path)

    def test_cli_source_check_and_validation_fail_closed(self) -> None:
        environment = dict(os.environ, PYTHONDONTWRITEBYTECODE="1")
        checked = subprocess.run([sys.executable, str(HELPER), "check-sources"], env=environment,
                                 capture_output=True, check=True, timeout=10)
        result = json.loads(checked.stdout)
        self.assertIs(result["native_admission"], False)
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            plan, profile = root / "plan.json", root / "profile.json"
            raw = encode(self.plan)
            plan.write_bytes(raw)
            profile.write_bytes(encode(self.profile))
            command = [sys.executable, str(HELPER), "validate", "--plan", str(plan), "--profile", str(profile),
                       "--plan-sha256", hashlib.sha256(raw).hexdigest(), "--lane", self.lane, "--image-alias", self.alias]
            checked = subprocess.run(command, env=environment, capture_output=True, check=True, timeout=10)
            self.assertEqual(json.loads(checked.stdout), self.validate())
            plan.write_bytes(raw + b"\n")
            failed = subprocess.run(command, env=environment, capture_output=True, check=False, timeout=10)
            self.assertEqual(failed.returncode, 1)
            self.assertEqual(failed.stdout, b"")
            self.assertNotIn(self.alias.encode(), failed.stderr)
            fixture = root / "fixtures/conformance/docker-application"
            fixture.mkdir(parents=True)
            (fixture / "core-expectations.json").write_bytes(self.catalogue)
            (fixture / "core.compose.yaml").write_bytes(self.source + b"\n")
            failed = subprocess.run([sys.executable, str(HELPER), "--repository", str(root), *command[2:]],
                                    env=environment, capture_output=True, check=False, timeout=10)
            self.assertEqual(failed.returncode, 1)
            self.assertEqual(failed.stdout, b"")
            self.assertIn(b"source bytes differ", failed.stderr)


if __name__ == "__main__":
    unittest.main()
