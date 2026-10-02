#!/usr/bin/env python3
"""Offline application topology prerequisites, never runtime replay authority.

The caller supplies a reviewed profile and private image aliases independently of
the artifact. Application behavior categories are requirements, not observations.
Commands, environment, default fields, losses, and dependency choreography still
need their own reviewed acceptance before any later isolated runtime rehearsal.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import pathlib
import re
import sys
from typing import Any


APPLICATIONS = {"forgejo", "nextcloud", "paperless-ngx", "immich", "observability", "supabase"}
IDENTITY = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}")
DIGEST = re.compile(r"[0-9a-f]{64}")
API = re.compile(r"1\.[0-9]{2}")
CATALOGUE_PATH = "fixtures/conformance/docker-application/expected-applications.json"
LIMIT = 1_048_576


class ExpectationError(ValueError):
    """An offline prerequisite differs from independently reviewed expectations."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ExpectationError(message)


def document(raw: bytes) -> Any:
    require(isinstance(raw, bytes) and 0 < len(raw) <= LIMIT, "document size differs")

    def pairs(items: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in items:
            require(key not in result, "duplicate JSON key")
            result[key] = value
        return result

    def nonfinite(_value: str) -> None:
        raise ExpectationError("non-JSON numeric constant")

    try:
        result = json.loads(raw, object_pairs_hook=pairs, parse_constant=nonfinite)
    except (ValueError, UnicodeError, RecursionError) as error:
        raise ExpectationError("invalid unambiguous JSON document") from error
    pending = [(result, 0)]
    while pending:
        value, depth = pending.pop()
        require(depth <= 32, "document nesting exceeds bound")
        if isinstance(value, dict):
            pending.extend((child, depth + 1) for child in value.values())
        elif isinstance(value, list):
            pending.extend((child, depth + 1) for child in value)
        elif isinstance(value, float):
            require(math.isfinite(value), "nonfinite JSON number")
    return result


def fields(value: Any, expected: set[str], message: str) -> None:
    require(isinstance(value, dict) and set(value) == expected, message)


def same(left: Any, right: Any) -> bool:
    """JSON equality that does not treat booleans as integer default fields."""
    return json.dumps(left, sort_keys=True) == json.dumps(right, sort_keys=True)


def unique_list(value: Any, message: str, maximum: int = 256) -> None:
    require(isinstance(value, list) and len(value) <= maximum, message)
    require(len({json.dumps(item, sort_keys=True) for item in value}) == len(value), message)


def identity(value: Any) -> bool:
    return isinstance(value, str) and IDENTITY.fullmatch(value) is not None


def container_fields(body: dict[str, Any]) -> None:
    """Close the native field boundary without claiming deferred semantic fidelity."""
    required = {"Image", "Labels", "HostConfig", "NetworkingConfig"}
    deferred = {"Env", "Entrypoint", "Cmd", "Healthcheck", "User", "WorkingDir", "StopSignal", "StopTimeout"}
    require(required <= set(body) and set(body) <= required | deferred | {"ExposedPorts"},
            "container native fields are outside topology prerequisite")
    for key in ("Env", "Entrypoint", "Cmd"):
        if key in body:
            require(isinstance(body[key], list) and len(body[key]) <= 256
                    and all(isinstance(value, str) and "\x00" not in value for value in body[key]),
                    "deferred native argument field is malformed")
    for key in ("User", "WorkingDir", "StopSignal"):
        if key in body:
            require(isinstance(body[key], str) and "\x00" not in body[key], "deferred native setting is malformed")
    if "StopTimeout" in body:
        require(type(body["StopTimeout"]) is int and 0 <= body["StopTimeout"] <= 2147483647,
                "deferred native stop timeout is malformed")
    if "Healthcheck" in body:
        health = body["Healthcheck"]
        require(isinstance(health, dict) and "Test" in health
                and set(health) <= {"Test", "Interval", "Timeout", "Retries", "StartPeriod", "StartInterval"},
                "deferred native health fields are malformed")
        test = health["Test"]
        require(isinstance(test, list) and 0 < len(test) <= 256
                and all(isinstance(value, str) and "\x00" not in value for value in test)
                and test[0] in {"CMD", "CMD-SHELL", "NONE"}, "deferred native health test is malformed")
        require(all(type(value) is int and 0 <= value <= 9223372036854775807
                    for key, value in health.items() if key != "Test"), "deferred native health number is malformed")
    host = body["HostConfig"]
    require(isinstance(host, dict) and "NetworkMode" in host
            and set(host) <= {"NetworkMode", "Mounts", "PortBindings"},
            "container host fields are outside topology prerequisite")


def source_digest(sources: list[dict[str, str]]) -> str:
    """Bind sorted relative source paths and exact reviewed byte SHA-256 values."""
    digest = hashlib.sha256()
    for row in sorted(sources, key=lambda item: item["path"]):
        path = row["path"].encode("utf-8")
        digest.update(len(path).to_bytes(8, "big"))
        digest.update(path)
        digest.update(bytes.fromhex(row["sha256"]))
    return digest.hexdigest()


def catalogue(raw: bytes) -> dict[str, Any]:
    value = document(raw)
    fields(value, {"schema_version", "kind", "evidence_kind", "source_kind", "native_execution", "applications"},
           "expectations envelope differs")
    require(type(value["schema_version"]) is int and value["schema_version"] == 1
            and value["kind"] == "boxferry-docker-application-expectations"
            and value["evidence_kind"] == "offline-contract-prerequisite"
            and value["source_kind"] == "authored-compose"
            and value["native_execution"] is False, "expectations version or evidence kind differs")
    require(isinstance(value["applications"], dict) and set(value["applications"]) == APPLICATIONS,
            "finite application set differs")
    for app, expected in value["applications"].items():
        fields(expected, {"owner_suffix", "sources", "image_catalogue", "provider_catalogue", "networks",
                          "volumes", "services", "dependencies", "excluded_peers", "shared_services",
                          "required_checks"}, "application expectations fields differ")
        require(identity(expected["owner_suffix"]), "application owner differs")
        unique_list(expected["sources"], "source bindings differ", 64)
        paths = set()
        for row in expected["sources"]:
            fields(row, {"path", "sha256"}, "source binding fields differ")
            path = row["path"]
            require(isinstance(path, str) and not pathlib.PurePosixPath(path).is_absolute()
                    and all(part not in {"", ".", ".."} for part in path.split("/"))
                    and path not in paths and isinstance(row["sha256"], str)
                    and DIGEST.fullmatch(row["sha256"]) is not None, "source binding differs")
            paths.add(path)
        require(expected["image_catalogue"] == f"fixtures/conformance/{app}-application/images.tsv"
                and expected["provider_catalogue"] == f"fixtures/conformance/{app}-application/providers.tsv"
                and {expected["image_catalogue"], expected["provider_catalogue"],
                     f"fixtures/conformance/{app}-application/compose.yaml"} <= paths,
                "canonical fixture references differ")
        for group in ("services", "networks", "volumes"):
            records = expected[group]
            require(isinstance(records, dict) and 0 < len(records) <= 256
                    and all(identity(key) for key in records), "resource catalogue differs")
            require(all(isinstance(row, dict) and identity(row.get("runtime_suffix"))
                        for row in records.values()), "resource identities differ")
            require(len({row["runtime_suffix"] for row in records.values()}) == len(records),
                    "resource identities repeat")
        for network in expected["networks"].values():
            fields(network, {"runtime_suffix", "ownership", "driver", "internal"}, "network fields differ")
            require(isinstance(network["ownership"], str) and network["ownership"] in {"application", "shared"} and network["driver"] == "bridge"
                    and type(network["internal"]) is bool, "network isolation differs")
            require(network["ownership"] != "shared" or network["internal"] is False,
                    "shared network isolation differs")
        require("backend" in expected["networks"]
                and expected["networks"]["backend"]["internal"] is True
                and expected["networks"]["backend"]["ownership"] == "application",
                "internal application backend is required")
        for volume in expected["volumes"].values():
            fields(volume, {"runtime_suffix"}, "volume fields differ")
        for service in expected["services"].values():
            fields(service, {"runtime_suffix", "image_key", "networks", "mounts", "ingress"},
                   "service fields differ")
            require(identity(service["image_key"]) and isinstance(service["networks"], dict)
                    and "backend" in service["networks"]
                    and set(service["networks"]) <= set(expected["networks"]), "service network graph differs")
            for aliases in service["networks"].values():
                unique_list(aliases, "network aliases differ")
                require(all(identity(alias) for alias in aliases), "network alias differs")
            unique_list(service["mounts"], "mount inventory differs")
            targets = set()
            for mount in service["mounts"]:
                fields(mount, {"kind", "source", "target", "read_only"}, "mount fields differ")
                require(isinstance(mount["kind"], str) and mount["kind"] in {"bind", "volume"} and type(mount["read_only"]) is bool
                        and isinstance(mount["target"], str) and mount["target"].startswith("/")
                        and mount["target"] not in targets, "mount contract differs")
                targets.add(mount["target"])
                require(isinstance(mount["source"], str), "mount source differs")
                if mount["kind"] == "volume":
                    require(mount["source"] in expected["volumes"], "mount volume differs")
                else:
                    require(mount["source"] == "." or
                            (not pathlib.PurePosixPath(mount["source"]).is_absolute()
                             and all(part not in {"", ".", ".."} for part in mount["source"].split("/"))),
                            "bind source differs")
            unique_list(service["ingress"], "ingress inventory differs")
            for ingress in service["ingress"]:
                fields(ingress, {"host_ip", "host_port", "container_port", "protocol"}, "ingress fields differ")
                require(ingress["host_ip"] == "127.0.0.1" and ingress["protocol"] == "tcp"
                        and all(type(ingress[key]) is int and 0 < ingress[key] <= 65535
                                for key in ("host_port", "container_port")), "loopback ingress differs")
        unique_list(expected["dependencies"], "dependency expectations differ", 4096)
        seen_edges = set()
        for dependency in expected["dependencies"]:
            fields(dependency, {"service", "dependency", "condition", "required", "restart"},
                   "dependency fields differ")
            pair = (dependency["service"], dependency["dependency"])
            require(all(isinstance(service, str) and service in expected["services"] for service in pair) and pair[0] != pair[1]
                    and pair not in seen_edges and isinstance(dependency["condition"], str) and dependency["condition"] in
                    {"started", "healthy", "completed_successfully"}
                    and type(dependency["required"]) is bool and type(dependency["restart"]) is bool,
                    "dependency graph differs")
            seen_edges.add(pair)
        remaining = set(expected["services"])
        while remaining:
            ready = {service for service in remaining if not any(
                row["service"] == service and row["dependency"] in remaining for row in expected["dependencies"])}
            require(bool(ready), "dependency graph is cyclic")
            remaining -= ready
        for group in ("excluded_peers", "shared_services"):
            unique_list(expected[group], "peer inventory differs")
            peers = set()
            for peer in expected[group]:
                peer_fields = {"runtime_suffix", "owner_suffix", "networks"} if group == "excluded_peers" else {
                    "runtime_suffix", "ownership", "networks", "mounts", "ingress"}
                fields(peer, peer_fields, "peer fields differ")
                require(identity(peer["runtime_suffix"]) and peer["runtime_suffix"] not in peers
                        and peer["runtime_suffix"] not in {row["runtime_suffix"] for row in expected["services"].values()}
                        and isinstance(peer["networks"], dict) and set(peer["networks"]) == {"edge"},
                        "peer must remain separately owned and edge-only")
                peers.add(peer["runtime_suffix"])
                for aliases in peer["networks"].values():
                    unique_list(aliases, "peer aliases differ")
                    require(all(identity(alias) for alias in aliases), "peer alias differs")
                if group == "excluded_peers":
                    require(identity(peer["owner_suffix"]) and peer["owner_suffix"] != expected["owner_suffix"],
                            "excluded peer ownership differs")
                else:
                    require(peer["ownership"] == "shared", "shared service ownership differs")
                    unique_list(peer["mounts"], "shared service mounts differ")
                    for mount in peer["mounts"]:
                        fields(mount, {"kind", "source", "target", "read_only"}, "shared mount fields differ")
                        require(mount["kind"] == "bind" and isinstance(mount["source"], str)
                                and mount["source"] in {pathlib.PurePosixPath(path).name for path in paths}
                                and isinstance(mount["target"], str) and mount["target"].startswith("/")
                                and mount["read_only"] is True, "shared service mount differs")
                    unique_list(peer["ingress"], "shared service ingress differs")
                    for ingress in peer["ingress"]:
                        fields(ingress, {"host_ip", "host_port", "container_port", "protocol"}, "shared ingress fields differ")
                        require(ingress["host_ip"] == "127.0.0.1" and ingress["protocol"] == "tcp"
                                and all(type(ingress[key]) is int and 0 < ingress[key] <= 65535
                                        for key in ("host_port", "container_port")), "shared loopback ingress differs")
        fields(expected["required_checks"], {"application", "persistence", "safety"}, "success categories differ")
        for checks in expected["required_checks"].values():
            unique_list(checks, "success category list differs")
            require(bool(checks) and all(identity(check) for check in checks), "success category differs")
    require(len(value["applications"]["supabase"]["services"]) == 11, "Supabase requires eleven services")
    return value


def check_sources(root: pathlib.Path, catalogue_bytes: bytes) -> dict[str, str]:
    """Read-only checksum check; existing provenance and historical evidence stay canonical."""
    expected = catalogue(catalogue_bytes)
    root = root.resolve(strict=True)
    for app in expected["applications"].values():
        for row in app["sources"]:
            path = root / row["path"]
            require(not any(parent.is_symlink() for parent in [path, *path.parents] if parent != root.parent),
                    "source path has symlink")
            require(path.is_file() and path.stat().st_size <= LIMIT, "source file is absent or oversized")
            require(hashlib.sha256(path.read_bytes()).hexdigest() == row["sha256"], "reviewed source bytes differ")
    return {app: source_digest(row["sources"]) for app, row in expected["applications"].items()}


def validate_application(
    plan_bytes: bytes, admission_bytes: bytes, *, catalogue_bytes: bytes,
    application: str, lane: str, profile: dict[str, Any], image_aliases: dict[str, str],
    prefix: str, run_id: str, fixture_root: str,
) -> dict[str, Any]:
    """Check raw native topology against finite expectations; perform no I/O or execution.

    A passing result is only an offline prerequisite. In particular, declarations
    of dependencies and success categories are not evidence that they executed.
    The caller must separately call check_sources before using source bindings.
    """
    expected = catalogue(catalogue_bytes)
    require(isinstance(application, str) and application in APPLICATIONS, "unsupported application")
    require(lane == "upstream-rootless" or
            (lane == "upstream-rootful" and application in {"forgejo", "nextcloud"}),
            "application lane is outside finite approved scope")
    require(identity(prefix) and len(prefix) <= 48 and identity(run_id), "run identity differs")
    require(isinstance(fixture_root, str) and fixture_root.startswith("/")
            and len(fixture_root) <= 4096 and "\x00" not in fixture_root
            and all(part not in {"", ".", ".."} for part in fixture_root.split("/")[1:]),
            "private fixture root differs")
    app = expected["applications"][application]
    fields(profile, {"kind", "build", "engine_release", "advertised_api_version", "acquisition_api_version",
                     "rendering_api_version", "daemon_mode", "evidence_sha256"}, "reviewed profile fields differ")
    require(profile["kind"] == "target" and profile["build"] == {"kind": "upstream"}
            and profile["daemon_mode"] == lane.rsplit("-", 1)[1]
            and isinstance(profile["engine_release"], str)
            and re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", profile["engine_release"]) is not None
            and isinstance(profile["evidence_sha256"], str)
            and DIGEST.fullmatch(profile["evidence_sha256"]) is not None,
            "reviewed native target identity differs")
    for key in ("advertised_api_version", "acquisition_api_version", "rendering_api_version"):
        require(isinstance(profile[key], str) and API.fullmatch(profile[key]) is not None,
                "reviewed API identity differs")
    require(profile["acquisition_api_version"] <= profile["advertised_api_version"]
            and profile["rendering_api_version"] <= profile["advertised_api_version"], "reviewed API ordering differs")
    required_images = {row["image_key"] for row in app["services"].values()}
    require(isinstance(image_aliases, dict) and set(image_aliases) == required_images
            and all(isinstance(alias, str) and re.fullmatch(
                r"registry\.invalid/[a-z0-9][a-z0-9._/-]*:[A-Za-z0-9._-]+", alias) is not None
                    for alias in image_aliases.values())
            and len(set(image_aliases.values())) == len(image_aliases), "independent private image aliases differ")
    admission = document(admission_bytes)
    fields(admission, {"schema_version", "kind", "evidence_kind", "source_kind", "native_execution", "application", "lane",
                       "prefix", "run_id", "fixture_root", "docker_plan_sha256", "expectations_sha256",
                       "source_sha256", "required_checks", "dependencies", "excluded_peers", "shared_services",
                       "runtime_evidence", "budget_measurements"}, "offline admission fields differ")
    require(type(admission["schema_version"]) is int and admission["schema_version"] == 1
            and admission["kind"] == "boxferry-docker-application-offline-admission"
            and admission["evidence_kind"] == "offline-contract-prerequisite"
            and admission["source_kind"] == "authored-compose"
            and admission["native_execution"] is False and admission["runtime_evidence"] == "unmeasured"
            and admission["budget_measurements"] is None, "offline admission cannot assert runtime success")
    for key, value in {"application": application, "lane": lane, "prefix": prefix,
                       "run_id": run_id, "fixture_root": fixture_root,
                       "docker_plan_sha256": hashlib.sha256(plan_bytes).hexdigest(),
                       "expectations_sha256": hashlib.sha256(catalogue_bytes).hexdigest(),
                       "source_sha256": source_digest(app["sources"])}.items():
        require(admission[key] == value, "offline admission identity binding differs")
    for key in ("required_checks", "dependencies", "excluded_peers", "shared_services"):
        require(same(admission[key], app[key]), "offline declared application requirements differ")
    plan = document(plan_bytes)
    fields(plan, {"schema_version", "context", "requests", "prerequisites"}, "complete artifact fields differ")
    require(type(plan["schema_version"]) is int and plan["schema_version"] == 1
            and same(plan["context"], profile), "complete artifact target context differs")
    unique_list(plan["requests"], "native requests repeat or exceed bound", 4096)
    unique_list(plan["prerequisites"], "native prerequisites repeat or exceed bound", 256)
    runtime = lambda suffix: f"{prefix}-{suffix}"
    labels = {"io.boxferry.live-run": run_id, "io.boxferry.application": runtime(app["owner_suffix"])}
    network_names = {key: runtime(row["runtime_suffix"]) for key, row in app["networks"].items()}
    volume_names = {key: runtime(row["runtime_suffix"]) for key, row in app["volumes"].items()}
    service_names = {key: runtime(row["runtime_suffix"]) for key, row in app["services"].items()}
    require(all(identity(name) for name in [*network_names.values(), *volume_names.values(), *service_names.values()]),
            "expanded runtime identity differs")
    expected_prerequisites = {name for key, name in network_names.items()
                              if app["networks"][key]["ownership"] == "shared"}
    references, prerequisites = set(), set()
    for row in plan["prerequisites"]:
        fields(row, {"kind", "reference", "identity", "expected_driver"}, "external prerequisite fields differ")
        reference = row["reference"]
        require(row["kind"] == "network" and row["expected_driver"] == "bridge"
                and isinstance(reference, str) and re.fullmatch(r"0|[1-9][0-9]{0,19}", reference) is not None
                and int(reference) <= 18446744073709551615 and reference not in references
                and isinstance(row["identity"], str) and row["identity"] not in prerequisites,
                "external prerequisite identity differs")
        references.add(reference)
        prerequisites.add(row["identity"])
    require(prerequisites == expected_prerequisites, "external prerequisite inventory differs")
    api_prefix = f"/v{profile['rendering_api_version']}/"
    created_networks, created_volumes, containers, attachments = set(), set(), {}, {}
    available_networks = set(prerequisites)
    for request in plan["requests"]:
        fields(request, {"method", "path", "body"}, "native request envelope differs")
        require(request["method"] == "POST" and isinstance(request["path"], str)
                and request["path"].startswith(api_prefix) and isinstance(request["body"], dict),
                "native method or API path differs")
        path, body = request["path"][len(api_prefix):], request["body"]
        if path == "networks/create":
            name = body.get("Name")
            require(identity(name) and name in network_names.values() and name not in available_networks,
                    "created network inventory differs")
            key = next(key for key, value in network_names.items() if value == name)
            network = app["networks"][key]
            required = {"Name": name, "Driver": "bridge", "Labels": labels}
            if network["internal"]:
                required["Internal"] = True
            require(same(body, required), "created network isolation or ownership differs")
            created_networks.add(name)
            available_networks.add(name)
        elif path == "volumes/create":
            require(body.get("Name") in volume_names.values() and body.get("Name") not in created_volumes
                    and same(body, {"Name": body.get("Name"), "Labels": labels}),
                    "created volume inventory or ownership differs")
            created_volumes.add(body["Name"])
        elif path.startswith("containers/create?name="):
            name = path.removeprefix("containers/create?name=")
            require(name in service_names.values() and name not in containers, "container inventory differs")
            key = next(key for key, value in service_names.items() if value == name)
            service = app["services"][key]
            container_fields(body)
            expected_labels = dict(labels)
            if application == "supabase" and key == "storage":
                expected_labels["io.boxferry.selection"] = "storage"
            require(same(body.get("Labels"), expected_labels), "container ownership differs")
            require(body.get("Image") == image_aliases[service["image_key"]], "container image role differs")
            host = body.get("HostConfig")
            expected_mounts = []
            for mount in service["mounts"]:
                source = volume_names[mount["source"]] if mount["kind"] == "volume" else (
                    fixture_root if mount["source"] == "." else f"{fixture_root}/{mount['source']}")
                if mount["kind"] == "volume":
                    require(source in created_volumes, "named volume was not explicitly created before mount")
                expected_mounts.append({"Type": mount["kind"], "Source": source,
                                        "Target": mount["target"], "ReadOnly": mount["read_only"]})
            require(same(host.get("Mounts", []), expected_mounts), "container mount inventory or access differs")
            ports = {f"{row['container_port']}/tcp": [{"HostIp": row["host_ip"], "HostPort": str(row["host_port"])}]
                     for row in service["ingress"]}
            require(same(host.get("PortBindings", {}), ports)
                    and same(body.get("ExposedPorts", {}), {key: {} for key in ports}), "container ingress differs")
            networking = body.get("NetworkingConfig")
            fields(networking, {"EndpointsConfig"}, "primary network fields differ")
            endpoints = networking["EndpointsConfig"]
            require(isinstance(endpoints, dict) and len(endpoints) == 1, "primary network attachment differs")
            network, endpoint = next(iter(endpoints.items()))
            require(network in available_networks and host.get("NetworkMode") == network,
                    "primary network ownership or creation order differs")
            attachments[name] = {network: endpoint}
            containers[name] = body
        elif path.startswith("networks/") and path.endswith("/connect"):
            network = path[len("networks/"):-len("/connect")]
            fields(body, {"Container", "EndpointConfig"}, "secondary attachment fields differ")
            name = body["Container"]
            require(isinstance(name, str) and name in containers and network in available_networks
                    and network not in attachments[name], "secondary network inventory or order differs")
            attachments[name][network] = body["EndpointConfig"]
        else:
            raise ExpectationError("native operation is outside topology prerequisite")
    require(created_networks == set(network_names.values()) - prerequisites
            and created_volumes == set(volume_names.values()) and set(containers) == set(service_names.values()),
            "complete application resource inventory differs")
    for key, name in service_names.items():
        wanted = {network_names[network]: ({"Aliases": aliases} if aliases else {})
                  for network, aliases in app["services"][key]["networks"].items()}
        require(same(attachments[name], wanted), "complete attachment or alias graph differs")
    return {"application": application, "lane": lane, "evidence_kind": "offline-contract-prerequisite", "source_kind": "authored-compose",
            "native_execution": False, "runtime_evidence": "unmeasured",
            "services": sorted(app["services"]), "docker_plan_sha256": hashlib.sha256(plan_bytes).hexdigest()}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["check-sources", "validate"])
    parser.add_argument("--repository", type=pathlib.Path, default=pathlib.Path(__file__).resolve().parents[2])
    parser.add_argument("--plan", type=pathlib.Path)
    parser.add_argument("--admission", type=pathlib.Path)
    parser.add_argument("--profile", type=pathlib.Path)
    parser.add_argument("--image-aliases", type=pathlib.Path)
    parser.add_argument("--application", choices=sorted(APPLICATIONS))
    parser.add_argument("--lane")
    parser.add_argument("--prefix")
    parser.add_argument("--run-id")
    parser.add_argument("--fixture-root")
    args = parser.parse_args()

    def read(path: pathlib.Path) -> bytes:
        with path.open("rb") as source:
            raw = source.read(LIMIT + 1)
        require(len(raw) <= LIMIT, "document size differs")
        return raw

    try:
        raw = read(args.repository / CATALOGUE_PATH)
        bindings = check_sources(args.repository, raw)
        if args.command == "check-sources":
            print(json.dumps({"evidence_kind": "offline-contract-prerequisite", "source_sha256": bindings}, sort_keys=True))
            return 0
        require(all(getattr(args, name) is not None for name in
                    ("plan", "admission", "profile", "image_aliases", "application", "lane", "prefix", "run_id", "fixture_root")),
                "validate requires all independently supplied identity and artifact arguments")
        result = validate_application(read(args.plan), read(args.admission), catalogue_bytes=raw,
            application=args.application, lane=args.lane, profile=document(read(args.profile)),
            image_aliases=document(read(args.image_aliases)), prefix=args.prefix, run_id=args.run_id,
            fixture_root=args.fixture_root)
        print(json.dumps(result, sort_keys=True))
        return 0
    except (ExpectationError, OSError) as error:
        # Error strings contain categories only; never print artifact fields or private paths.
        print(f"offline Docker application prerequisite rejected: {error if isinstance(error, ExpectationError) else 'input unavailable'}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
