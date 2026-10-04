#!/usr/bin/env python3
"""Check one inert core artifact offline; never native admission or replay authority."""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import pathlib
import re
import sys
from typing import Any

SPEC = importlib.util.spec_from_file_location(
    "docker_application_schedule", pathlib.Path(__file__).with_name("docker-application-schedule.py"))
assert SPEC is not None and SPEC.loader is not None
schedule = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(schedule)
topology = schedule.topology
EXPECTATIONS_PATH = "fixtures/conformance/docker-application/core-expectations.json"
SOURCE_PATH = "fixtures/conformance/docker-application/core.compose.yaml"
LANES = {"debian11-rootful", "debian11-rootless", "upstream-rootful", "upstream-rootless"}


def expectations(raw: bytes) -> dict[str, Any]:
    """Read authored expectations, not a native capability catalogue."""
    value = topology.document(raw)
    topology.fields(value, {"schema_version", "kind", "evidence_kind", "source_kind", "native_execution",
                            "source", "container_name", "image_placeholder", "command", "lanes"},
                    "core expectation fields differ")
    topology.require(type(value["schema_version"]) is int and value["schema_version"] == 1
                     and value["kind"] == "boxferry-docker-core-expectations"
                     and value["evidence_kind"] == "offline-contract-prerequisite"
                     and value["source_kind"] == "authored-compose" and value["native_execution"] is False,
                     "core expectation schema differs")
    topology.fields(value["source"], {"path", "sha256"}, "core source fields differ")
    topology.require(value["source"]["path"] == SOURCE_PATH
                     and isinstance(value["source"]["sha256"], str)
                     and topology.DIGEST.fullmatch(value["source"]["sha256"]) is not None,
                     "core source binding differs")
    topology.require(value["container_name"] == "bf-docker-core"
                     and value["image_placeholder"] == "BOXFERRY_CORE_IMAGE_LITERAL",
                     "core authored identities differ")
    command = value["command"]
    topology.require(isinstance(command, list) and 0 < len(command) <= 256
                     and all(isinstance(arg, str) and len(arg) <= 4096 and "\x00" not in arg for arg in command),
                     "core literal command differs")
    topology.fields(value["lanes"], LANES, "core lane inventory differs")
    for lane, row in value["lanes"].items():
        topology.fields(row, {"build_kind", "daemon_mode"}, "core lane fields differ")
        topology.require(row["build_kind"] == ("debian_package" if lane.startswith("debian11-") else "upstream")
                         and row["daemon_mode"] == lane.rsplit("-", 1)[1], "core lane identity differs")
    return value


def check_source(source_bytes: bytes, expectations_bytes: bytes) -> dict[str, Any]:
    value = expectations(expectations_bytes)
    topology.require(isinstance(source_bytes, bytes) and 0 < len(source_bytes) <= topology.LIMIT
                     and hashlib.sha256(source_bytes).hexdigest() == value["source"]["sha256"],
                     "core authored source bytes differ")
    return value


def check_sources(repository: pathlib.Path, expectations_bytes: bytes) -> dict[str, Any]:
    return check_source(schedule.read_document(repository / SOURCE_PATH), expectations_bytes)


def validate_core(plan_bytes: bytes, *, expectations_bytes: bytes, source_bytes: bytes,
                  expected_plan_sha256: str, lane: str, profile: dict[str, Any], image_alias: str) -> dict[str, Any]:
    """All trust inputs must be selected independently of the artifact under review.

    The caller supplies exact raw-plan binding, context, and private literal alias.
    Source checking is mandatory here as well as in the CLI. No result authorizes execution.
    """
    value = check_source(source_bytes, expectations_bytes)
    plan = topology.document(plan_bytes)
    topology.require(isinstance(expected_plan_sha256, str)
                     and topology.DIGEST.fullmatch(expected_plan_sha256) is not None
                     and hashlib.sha256(plan_bytes).hexdigest() == expected_plan_sha256,
                     "core exact plan byte binding differs")
    topology.require(isinstance(lane, str) and lane in value["lanes"], "core lane is outside authored scope")
    selected = value["lanes"][lane]
    topology.fields(profile, {"kind", "build", "engine_release", "advertised_api_version", "acquisition_api_version",
                              "rendering_api_version", "daemon_mode", "evidence_sha256"}, "core profile fields differ")
    topology.require(profile["kind"] == "target" and profile["daemon_mode"] == selected["daemon_mode"],
                     "core target identity differs")
    build = profile["build"]
    topology.fields(build, {"kind", "revision"} if selected["build_kind"] == "debian_package" else {"kind"},
                    "core build fields differ")
    topology.require(build["kind"] == selected["build_kind"], "core build identity differs")
    if "revision" in build:
        topology.require(isinstance(build["revision"], str)
                         and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9.+:~_-]{0,255}", build["revision"]) is not None,
                         "core package revision differs")
    topology.require(isinstance(profile["engine_release"], str)
                     and re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:\+[A-Za-z0-9._-]+)?", profile["engine_release"]) is not None
                     and isinstance(profile["evidence_sha256"], str)
                     and topology.DIGEST.fullmatch(profile["evidence_sha256"]) is not None,
                     "core target version evidence differs")
    for key in ("advertised_api_version", "acquisition_api_version", "rendering_api_version"):
        topology.require(isinstance(profile[key], str) and topology.API.fullmatch(profile[key]) is not None,
                         "core API identity differs")
    topology.require(profile["acquisition_api_version"] <= profile["advertised_api_version"]
                     and profile["rendering_api_version"] <= profile["advertised_api_version"],
                     "core API ordering differs")
    topology.require(isinstance(image_alias, str) and re.fullmatch(
        r"registry\.invalid/boxferry-core/busybox:[A-Za-z0-9._-]{1,128}", image_alias) is not None,
        "core private image alias differs")
    topology.fields(plan, {"schema_version", "context", "requests", "prerequisites"}, "core artifact fields differ")
    topology.require(type(plan["schema_version"]) is int and plan["schema_version"] == 1,
                     "core artifact schema differs")
    topology.require(topology.same(plan["context"], profile), "core complete target context differs")
    topology.require(topology.same(plan["prerequisites"], []), "core external prerequisites are forbidden")
    expected_request = {"method": "POST", "path": f"/v{profile['rendering_api_version']}/containers/create?name={value['container_name']}",
                        "body": {"Image": image_alias, "Cmd": value["command"], "HostConfig": {}}}
    topology.require(topology.same(plan["requests"], [expected_request]), "core literal create request differs")
    return {"schema_version": 1, "kind": "boxferry-docker-core-offline-prerequisite",
            "evidence_kind": "offline-contract-prerequisite", "source_kind": "authored-compose", "lane": lane,
            "native_execution": False, "replay_authority": False, "native_admission": False,
            "runtime_evidence": "unmeasured", "budget_measurements": None,
            "docker_plan_sha256": expected_plan_sha256,
            "expectations_sha256": hashlib.sha256(expectations_bytes).hexdigest(),
            "source_sha256": value["source"]["sha256"]}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", type=pathlib.Path, default=pathlib.Path(__file__).resolve().parents[2])
    subcommands = parser.add_subparsers(dest="command", required=True)
    subcommands.add_parser("check-sources")
    validate = subcommands.add_parser("validate")
    for name in ("plan", "profile"):
        validate.add_argument(f"--{name}", type=pathlib.Path, required=True)
    for name in ("plan-sha256", "lane", "image-alias"):
        validate.add_argument(f"--{name}", required=True)
    args = parser.parse_args()
    try:
        raw = schedule.read_document(args.repository / EXPECTATIONS_PATH)
        value = check_sources(args.repository, raw)
        if args.command == "check-sources":
            result = {"schema_version": 1, "kind": "boxferry-docker-core-source-check",
                      "evidence_kind": "offline-contract-prerequisite", "native_execution": False,
                      "replay_authority": False, "native_admission": False,
                      "source_sha256": value["source"]["sha256"], "expectations_sha256": hashlib.sha256(raw).hexdigest()}
        else:
            result = validate_core(schedule.read_document(args.plan), expectations_bytes=raw,
                                   source_bytes=schedule.read_document(args.repository / SOURCE_PATH),
                                   expected_plan_sha256=args.plan_sha256, lane=args.lane,
                                   profile=topology.document(schedule.read_document(args.profile)), image_alias=args.image_alias)
        print(json.dumps(result, sort_keys=True))
        return 0
    except (topology.ExpectationError, OSError) as error:
        print(str(error) if isinstance(error, topology.ExpectationError) else "unable to read core review input", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
