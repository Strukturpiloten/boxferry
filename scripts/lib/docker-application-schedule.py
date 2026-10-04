#!/usr/bin/env python3
"""Reviewed Forgejo/Nextcloud offline intent; never replay or native admission."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import pathlib
import stat
import sys
from typing import Any


SPEC = importlib.util.spec_from_file_location(
    "docker_application_expectations", pathlib.Path(__file__).with_name("docker-application-expectations.py"))
assert SPEC is not None and SPEC.loader is not None
topology = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(topology)

EXPECTED_EDGE = {"service": "forgejo", "dependency": "db", "condition": "healthy",
                 "required": True, "restart": False}
NEXTCLOUD_EDGES = [
    {"service": "app", "dependency": "db", "condition": "healthy", "required": True, "restart": False},
    {"service": "app", "dependency": "cache", "condition": "healthy", "required": True, "restart": False},
    {"service": "init", "dependency": "app", "condition": "healthy", "required": True, "restart": False},
    {"service": "cron", "dependency": "init", "condition": "completed_successfully", "required": True, "restart": False},
    {"service": "frontend", "dependency": "app", "condition": "healthy", "required": True, "restart": False},
]
PROVENANCE = {"source_document", "runtime_observation", "user_override",
              "implementation_default", "conversion_decision"}


def dependency_layers(decisions: list[dict[str, Any]], services: dict[str, str]) -> list[list[str]]:
    """Review prerequisites before dependents, without executing or measuring them."""
    dependencies: dict[str, set[str]] = {service: set() for service in services}
    for decision in decisions:
        source, target = decision["service"], decision["dependency"]
        topology.require(source in services and target in services and source != target,
                         "dependency contains an unknown or self reference")
        topology.require(target not in dependencies[source], "dependency edges repeat")
        dependencies[source].add(target)
    layers: list[list[str]] = []
    remaining = set(services)
    while remaining:
        layer = sorted(service for service in remaining if not dependencies[service] & remaining)
        topology.require(bool(layer), "dependency decisions contain a cycle")
        layers.append(layer)
        remaining.difference_update(layer)
    return layers


def validate_schedule(
    plan_bytes: bytes, admission_bytes: bytes, sidecar_bytes: bytes | None, *,
    catalogue_bytes: bytes, application: str, lane: str, profile: dict[str, Any],
    image_aliases: dict[str, str], prefix: str, run_id: str, fixture_root: str,
) -> dict[str, Any]:
    """Pure, bounded review of two independently authored application contracts.

    The CLI checks source bytes first; embedded callers must separately call
    topology.check_sources. Caller-selected profile/catalogue authority is not
    established by the artifact or this result. No native request is rendered,
    applied or authorized. Only Nextcloud's authored init/cron fields are checked;
    environment, image defaults, health and actual completion remain unproved.
    """
    topology.require(isinstance(application, str) and application in {"forgejo", "nextcloud"},
                     "schedule application is unreviewed")
    topology.validate_application(plan_bytes, admission_bytes, catalogue_bytes=catalogue_bytes,
        application=application, lane=lane, profile=profile, image_aliases=image_aliases,
        prefix=prefix, run_id=run_id, fixture_root=fixture_root)
    app = topology.catalogue(catalogue_bytes)["applications"][application]
    expected_edges = [EXPECTED_EDGE] if application == "forgejo" else NEXTCLOUD_EDGES
    expected_services = {"db", "forgejo"} if application == "forgejo" else {"db", "cache", "app", "init", "cron", "frontend"}
    topology.require(set(app["services"]) == expected_services
                     and topology.same(app["dependencies"], expected_edges),
                     "authored service or dependency contract differs")
    services = {key: f"{prefix}-{value['runtime_suffix']}" for key, value in app["services"].items()}
    topology.require(sidecar_bytes is not None, "required dependency sidecar is missing")
    sidecar = topology.document(sidecar_bytes)
    topology.fields(sidecar, {"schema_version", "kind", "docker_plan_sha256", "native_execution", "decisions"},
                    "dependency sidecar fields differ")
    topology.require(type(sidecar["schema_version"]) is int and sidecar["schema_version"] == 1
                     and sidecar["kind"] == "boxferry-docker-dependency-decisions"
                     and sidecar["native_execution"] is False, "dependency sidecar schema differs")
    plan_digest = hashlib.sha256(plan_bytes).hexdigest()
    topology.require(sidecar["docker_plan_sha256"] == plan_digest,
                     "dependency sidecar is not bound to exact native plan bytes")
    decisions = sidecar["decisions"]
    topology.unique_list(decisions, "dependency decisions repeat or exceed bound", 4096)
    topology.require(bool(decisions), "required dependency decision is missing")
    for decision in decisions:
        topology.fields(decision, {"service", "service_runtime_name", "dependency", "dependency_runtime_name",
            "condition", "condition_explicit", "required", "required_explicit", "restart", "restart_explicit",
            "fidelity", "native_engine_field", "provenance"}, "dependency decision fields differ")
        source, target = decision["service"], decision["dependency"]
        topology.require(isinstance(source, str) and isinstance(target, str)
                         and source in services and target in services,
                         "dependency contains an unknown service")
        topology.require(decision["service_runtime_name"] == services[source]
                         and decision["dependency_runtime_name"] == services[target],
                         "dependency runtime identity differs")
        topology.require(isinstance(decision["condition"], str)
                         and decision["condition"] in {"started", "healthy", "completed_successfully"},
                         "dependency readiness condition is unreviewed")
        topology.require(all(type(decision[key]) is bool for key in
            ("condition_explicit", "required", "required_explicit", "restart", "restart_explicit")),
            "dependency decision flag is not boolean")
        topology.require((decision["condition_explicit"] or decision["condition"] == "started")
                         and (decision["required_explicit"] or decision["required"])
                         and (decision["restart_explicit"] or not decision["restart"]),
                         "dependency implicit flag differs from its default")
        topology.require(decision["fidelity"] == "approximate" and decision["native_engine_field"] is False,
                         "dependency cannot claim native Engine semantics")
        provenance = decision["provenance"]
        topology.fields(provenance, {"reference", "condition", "required", "restart"},
                        "dependency provenance fields differ")
        for values in provenance.values():
            topology.require(isinstance(values, list) and len(values) <= 32
                             and all(isinstance(value, str) and value in PROVENANCE for value in values),
                             "dependency provenance contains an unreviewed category")
        if application == "nextcloud":
            topology.require(decision["condition_explicit"] and not decision["required_explicit"]
                             and not decision["restart_explicit"], "authored dependency explicitness differs")
    layers = dependency_layers(decisions, services)
    projected_edges = [{key: decision[key] for key in EXPECTED_EDGE} for decision in decisions]
    topology.require(len(decisions) == len(expected_edges)
                     and all(edge in projected_edges for edge in expected_edges),
                     "dependency edges differ from authored expectations")

    # Topology has already admitted the closed native operations. These are
    # references to existing requests for review, never new Engine requests.
    plan = topology.document(plan_bytes)
    api_prefix = f"/v{profile['rendering_api_version']}/"
    if application == "nextcloud":
        containers = {row["path"][len(api_prefix):].removeprefix("containers/create?name="): row["body"]
                      for row in plan["requests"] if row["path"][len(api_prefix):].startswith("containers/create?name=")}
        initialization, cron = containers[services["init"]], containers[services["cron"]]
        topology.require(topology.same(initialization.get("Cmd"), ["php", "/var/www/html/occ", "status"]),
                         "authored initialization command missing or differs")
        topology.require(initialization.get("User") == "www-data", "authored initialization user missing or differs")
        topology.require(topology.same(cron.get("Cmd"), ["/cron.sh"]), "authored cron command missing or differs")
    runtime_roles = {value: key for key, value in services.items()}
    creates: dict[str, list[dict[str, Any]]] = {kind: [] for kind in ("network", "volume", "container", "attachment")}
    for index, request in enumerate(plan["requests"]):
        path, body = request["path"][len(api_prefix):], request["body"]
        if path == "networks/create":
            kind, identity = "network", body["Name"]
        elif path == "volumes/create":
            kind, identity = "volume", body["Name"]
        elif path.startswith("containers/create?name="):
            kind, identity = "container", path.removeprefix("containers/create?name=")
        else:
            topology.require(path.startswith("networks/") and path.endswith("/connect"),
                             "unreviewed schedule operation")
            kind, identity = "attachment", path[len("networks/"):-len("/connect")]
        entry = {"kind": kind, "request_index": index, "identity": identity}
        if kind == "container":
            entry["service"] = runtime_roles[identity]
        elif kind == "attachment":
            entry["service"] = runtime_roles[body["Container"]]
        creates[kind].append(entry)
    rank = {service: index for index, layer in enumerate(layers) for service in layer}
    review_order = [*sorted(creates["network"], key=lambda row: row["identity"]),
                    *sorted(creates["volume"], key=lambda row: row["identity"]),
                    *sorted(creates["container"], key=lambda row: (rank[row["service"]], row["service"])),
                    *sorted(creates["attachment"], key=lambda row: (rank[row["service"]], row["identity"]))]
    result = {"schema_version": 1, "kind": f"boxferry-docker-{application}-offline-schedule",
            "evidence_kind": "offline-contract-prerequisite", "application": application, "lane": lane,
            "native_execution": False, "replay_authority": False, "native_admission": False,
            "runtime_evidence": "unmeasured", "budget_measurements": None,
            "docker_plan_sha256": plan_digest, "admission_sha256": hashlib.sha256(admission_bytes).hexdigest(),
            "dependency_sidecar_sha256": hashlib.sha256(sidecar_bytes).hexdigest(),
            "expectations_sha256": hashlib.sha256(catalogue_bytes).hexdigest(),
            "source_sha256": topology.source_digest(app["sources"]),
            "external_prerequisites": sorted(plan["prerequisites"], key=lambda row: row["identity"]),
            "native_request_order": list(range(len(plan["requests"]))), "review_operations": review_order,
            "service_review_layers": layers, "dependency_decisions": decisions}
    if application == "nextcloud":
        result["authored_checks"] = ["init.command", "init.user", "cron.command"]
        result["shared_service_expectations"] = [
            {"identity": f"{prefix}-{row['runtime_suffix']}", "ownership": "shared", "runtime_evidence": "unmeasured"}
            for row in app["shared_services"]]
    return result


def read_document(path: pathlib.Path) -> bytes:
    """Bound a regular-file read; a FIFO or final-component symlink cannot block."""
    descriptor = os.open(path, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW)
    with os.fdopen(descriptor, "rb") as source:
        topology.require(stat.S_ISREG(os.fstat(source.fileno()).st_mode), "input is not a regular file")
        raw = source.read(topology.LIMIT + 1)
    topology.require(len(raw) <= topology.LIMIT, "document size differs")
    return raw


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", type=pathlib.Path, default=pathlib.Path(__file__).resolve().parents[2])
    parser.add_argument("--application", default="forgejo")
    for name in ("plan", "admission", "sidecar", "profile", "image-aliases"):
        parser.add_argument(f"--{name}", type=pathlib.Path, required=True)
    for name in ("lane", "prefix", "run-id", "fixture-root"):
        parser.add_argument(f"--{name}", required=True)
    args = parser.parse_args()

    try:
        catalogue_bytes = read_document(args.repository / topology.CATALOGUE_PATH)
        topology.check_sources(args.repository, catalogue_bytes)
        result = validate_schedule(read_document(args.plan), read_document(args.admission), read_document(args.sidecar),
            catalogue_bytes=catalogue_bytes, application=args.application, lane=args.lane,
            profile=topology.document(read_document(args.profile)), image_aliases=topology.document(read_document(args.image_aliases)),
            prefix=args.prefix, run_id=args.run_id, fixture_root=args.fixture_root)
        print(json.dumps(result, sort_keys=True))
        return 0
    except (topology.ExpectationError, OSError) as error:
        category = str(error) if isinstance(error, topology.ExpectationError) else "input unavailable"
        application_label = {"forgejo": "Forgejo", "nextcloud": "Nextcloud"}.get(args.application, "application")
        print(f"offline {application_label} schedule rejected: {category}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
