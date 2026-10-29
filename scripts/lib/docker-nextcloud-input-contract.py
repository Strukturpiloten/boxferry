#!/usr/bin/env python3
"""Pure Nextcloud authored-request consistency checks; no custody or admission proof.

Callers supply independently held checkpoint bindings. These functions neither acquire nor
interpret native/image facts, authorize profiles, inspect files/sockets, execute commands, write
receipts, or validate findings/artifacts. Success is None, not runtime or migration acceptance.
Incomplete image/default/finding expectations remain outside this module and blocked.
Environment consents do not authorize protected command or healthcheck values. In particular,
the authored Redis command/healthcheck password remains an exporter blocker; accepting a
request cannot establish that its full application plan is exportable or executable.

JSON loading is bounded to 64 KiB; objects have nesting depth 16, 4096 nodes (including keys),
256 members per collection, 64 KiB of UTF-8 strings, and 256-bit integers. These are harness
record limits, not native Docker semantic constraints. Duplicate keys,
floats, nonfinite values, and invalid Unicode are rejected. A loaded document still
requires the relevant schema validator. File privacy and physical custody belong to the caller.
"""
from __future__ import annotations

import json
import re

LIMIT = 64 * 1024
ROLES = frozenset({"db", "cache", "app", "init", "cron", "frontend"})
LABEL_KEYS = ("io.boxferry.live-run", "io.boxferry.application")
_CODES = frozenset({
    "invalid-input", "invalid-json", "duplicate-field", "budget-exceeded", "invalid-type",
    "invalid-bindings", "request-shape", "request-identity", "source-profile", "target-profile",
    "role-membership", "resource-decisions", "public-labels", "recipe-consents", "recipe-policy",
    "selection-shape", "selection-membership", "selection-selector",
})
_COMMON_BINDINGS = frozenset({
    "lane", "provisioner", "phase", "run_id", "prefix", "application_name", "socket",
    "source_profile", "targets", "role_ids", "resources", "proxy_id", "peer_id",
})
_OUTCOME_FIELDS = frozenset({
    "schema_version", "application_name", "socket", "container_ids", "resource_decisions",
    "public_labels", "promote_protected_environment_values", "include_environment_values",
    "promote_same_host_bind_mounts", "source_profile", "loss_policy", "targets",
})
_SELECTION_FIELDS = frozenset({
    "schema_version", "application_name", "socket", "source_profile", "expected_container_ids", "selector",
})


class InputContractError(ValueError):
    """Value-free fixed-code failure, including repr and normal traceback formatting."""

    def __init__(self, code: str = "invalid-input") -> None:
        if type(code) is not str or code not in _CODES:
            code = "invalid-input"
        self.code = code
        super().__init__(code)


def _require(condition: bool, code: str) -> None:
    if not condition:
        raise InputContractError(code)


def _budget(value: object) -> None:
    pending = [(value, 0)]
    nodes = 0
    string_bytes = 0
    while pending:
        item, depth = pending.pop()
        nodes += 1
        _require(depth <= 16 and nodes <= 4096, "budget-exceeded")
        kind = type(item)
        if kind is str:
            _require(len(item) <= LIMIT - string_bytes, "budget-exceeded")
            try:
                string_bytes += len(item.encode("utf-8"))
            except UnicodeError:
                raise InputContractError("invalid-type") from None
            _require(string_bytes <= LIMIT, "budget-exceeded")
        elif kind is dict:
            _require(len(item) <= 256, "budget-exceeded")
            _require(all(type(key) is str for key in item), "invalid-type")
            pending.extend((part, depth + 1) for pair in item.items() for part in pair)
        elif kind is list:
            _require(len(item) <= 256, "budget-exceeded")
            pending.extend((part, depth + 1) for part in item)
        elif kind is int:
            _require(item.bit_length() <= 256, "budget-exceeded")
        else:
            _require(kind is bool or item is None, "invalid-type")


def _pairs(items: list[tuple[str, object]]) -> dict:
    result = {}
    for key, value in items:
        _require(key not in result, "duplicate-field")
        result[key] = value
    return result


def _nonfinite(_value: str) -> None:
    raise InputContractError("invalid-json")


def _integer(value: str) -> int:
    # Bound decimal work before conversion, independently of Python's global digit limit.
    _require(len(value.lstrip("-")) <= 78, "budget-exceeded")
    result = int(value)
    _require(result.bit_length() <= 256, "budget-exceeded")
    return result


def load_json_document(raw: bytes) -> dict:
    """Load bounded UTF-8 JSON without file access; schema validation is separate."""
    _require(type(raw) is bytes, "invalid-type")
    _require(len(raw) <= LIMIT, "budget-exceeded")
    try:
        result = json.loads(raw.decode("utf-8"), object_pairs_hook=_pairs,
                            parse_constant=_nonfinite, parse_int=_integer)
    except InputContractError:
        raise
    except RecursionError:
        raise InputContractError("budget-exceeded") from None
    except (UnicodeError, ValueError):
        raise InputContractError("invalid-json") from None
    _budget(result)
    _require(type(result) is dict, "invalid-type")
    return result


def _fields(value: object, expected: set | frozenset, code: str) -> None:
    _require(type(value) is dict and set(value) == expected, code)


def _text(value: object, maximum: int, code: str) -> None:
    _require(type(value) is str and 0 < len(value.encode("utf-8")) <= maximum
             and all(ord(char) >= 32 and not 127 <= ord(char) <= 159 for char in value), code)


def _id(value: object, code: str) -> None:
    _require(type(value) is str and re.fullmatch(r"[0-9a-f]{64}", value) is not None, code)


def _role_ids(value: object) -> set[str]:
    _fields(value, ROLES, "invalid-bindings")
    for identity in value.values():
        _id(identity, "invalid-bindings")
    result = set(value.values())
    _require(len(result) == 6, "invalid-bindings")
    return result


def _source_profile(value: object, code: str) -> None:
    _fields(value, {"engine_release", "daemon_mode"}, code)
    _text(value["engine_release"], 64, code)
    _require(type(value["daemon_mode"]) is str and value["daemon_mode"] in {"rootful", "rootless"}, code)


def _targets(value: object, code: str) -> None:
    _fields(value, {"compose", "quadlet", "podman", "docker"}, code)
    for name in ("compose", "quadlet", "podman", "docker"):
        fields = {"implementation", "version"}
        if name in {"podman", "docker"}:
            fields.add("daemon_mode")
        if name == "docker":
            fields.add("engine_build")
        profile = value[name]
        _fields(profile, fields, code)
        _text(profile["implementation"], 128, code)
        _text(profile["version"], 32, code)
        if "daemon_mode" in fields:
            _require(type(profile["daemon_mode"]) is str
                     and profile["daemon_mode"] in {"rootful", "rootless"}, code)
        if "engine_build" in fields:
            _text(profile["engine_build"], 128, code)


def _bindings(value: object) -> dict:
    _budget(value)
    _require(type(value) is dict, "invalid-bindings")
    phase = value.get("phase")
    _require(type(phase) is str and phase in {"source", "reacquired"}, "invalid-bindings")
    fields = _COMMON_BINDINGS | ({"prior_source_role_ids"} if phase == "reacquired" else set())
    _fields(value, fields, "invalid-bindings")
    _require(type(value["lane"]) is str and value["lane"] in {
        "upstream-rootful", "upstream-rootless"}, "invalid-bindings")
    _require(type(value["provisioner"]) is str and value["provisioner"] in {
        "native", "compose"}, "invalid-bindings")
    _require(type(value["run_id"]) is str
             and re.fullmatch(r"bf-[0-9a-f]{8}", value["run_id"]) is not None, "invalid-bindings")
    expected_prefix = f"bf-nextcloud-{value['run_id'][3:]}-{value['provisioner']}"
    _require(value["prefix"] == expected_prefix
             and value["application_name"] == expected_prefix + "-cloud", "invalid-bindings")
    _text(value["socket"], 4096, "invalid-bindings")
    _require(value["socket"].startswith("/"), "invalid-bindings")
    _source_profile(value["source_profile"], "invalid-bindings")
    _targets(value["targets"], "invalid-bindings")
    current = _role_ids(value["role_ids"])
    for name in ("proxy_id", "peer_id"):
        _id(value[name], "invalid-bindings")
    excluded = {value["proxy_id"], value["peer_id"]}
    _require(len(excluded) == 2 and current.isdisjoint(excluded), "invalid-bindings")
    if phase == "reacquired":
        _require((current | excluded).isdisjoint(_role_ids(value["prior_source_role_ids"])), "invalid-bindings")
    resources = value["resources"]
    _fields(resources, {"backend_network_id", "edge_network_id", "database_volume",
                        "redis_volume", "nextcloud_volume"}, "invalid-bindings")
    for name in ("backend_network_id", "edge_network_id"):
        _id(resources[name], "invalid-bindings")
    _require(resources["backend_network_id"] != resources["edge_network_id"], "invalid-bindings")
    generation = "cloud" if phase == "source" else "restore"
    for name in ("database", "redis", "nextcloud"):
        _require(resources[name + "_volume"] == f"{expected_prefix}-{generation}-{name}", "invalid-bindings")
    return value


def _identity(request: dict, bindings: dict) -> None:
    _require(type(request["schema_version"]) is int and request["schema_version"] == 1, "request-shape")
    _text(request["application_name"], 128, "request-identity")
    _text(request["socket"], 4096, "request-identity")
    _require(request["application_name"] == bindings["application_name"]
             and request["socket"] == bindings["socket"], "request-identity")
    _source_profile(request["source_profile"], "source-profile")
    _require(request["source_profile"] == bindings["source_profile"], "source-profile")


def _members(value: object, expected: set[str], code: str) -> None:
    _require(type(value) is list and len(value) == len(expected), code)
    for identity in value:
        _id(identity, code)
    _require(len(set(value)) == len(value) and set(value) == expected, code)


def _resources(request: dict, bindings: dict) -> None:
    rows = request["resource_decisions"]
    _require(type(rows) is list and len(rows) == 5, "resource-decisions")
    resources = bindings["resources"]
    prefix = bindings["prefix"]
    generation = "restore" if bindings["phase"] == "source" else "reexport"
    expected = {
        ("network", resources["backend_network_id"], "create", f"{prefix}-{generation}-backend"),
        ("network", resources["edge_network_id"], "external", f"{prefix}-shared-edge"),
        *(("volume", resources[name + "_volume"], "create", f"{prefix}-{generation}-{name}")
          for name in ("database", "redis", "nextcloud")),
    }
    actual = []
    for row in rows:
        _fields(row, {"kind", "source", "action", "target"}, "resource-decisions")
        for name in ("kind", "source", "action", "target"):
            _text(row[name], 256 if name == "source" else 128, "resource-decisions")
        actual.append((row["kind"], row["source"], row["action"], row["target"]))
    _require(len(set(actual)) == 5 and set(actual) == expected, "resource-decisions")


def _labels(request: dict, bindings: dict) -> None:
    rows = request["public_labels"]
    _require(type(rows) is list and len(rows) == 20, "public-labels")
    resources = bindings["resources"]
    subjects = [("container", identity) for identity in bindings["role_ids"].values()]
    subjects.append(("network", resources["backend_network_id"]))
    subjects.extend(("volume", resources[name + "_volume"]) for name in ("database", "redis", "nextcloud"))
    expected = {(kind, source, key) for kind, source in subjects for key in LABEL_KEYS}
    actual = []
    for row in rows:
        _fields(row, {"kind", "source", "key"}, "public-labels")
        for name in ("kind", "source", "key"):
            _text(row[name], 256, "public-labels")
        actual.append((row["kind"], row["source"], row["key"]))
    _require(len(set(actual)) == 20 and set(actual) == expected, "public-labels")


def validate_outcome_request(request: object, bindings: object) -> None:
    """Check only fixed authored-request consistency against independently supplied bindings.

    Bindings have closed fields lane/provisioner/phase/run_id/prefix/application_name/socket,
    source_profile/targets, six-role role_ids, resources (two network IDs/three volume names),
    proxy_id/peer_id. Only reacquired requires prior_source_role_ids; source forbids it.
    Profile objects are compared exactly, never selected, interpreted or admitted here.
    Socket/ID/label bindings do not prove a socket exists, physical ownership, actual metadata,
    or source context. The caller owns those proofs and native/producer profile acceptance.
    """
    context = _bindings(bindings)
    _budget(request)
    _fields(request, _OUTCOME_FIELDS, "request-shape")
    _identity(request, context)
    _members(request["container_ids"], set(context["role_ids"].values()), "role-membership")
    _targets(request["targets"], "target-profile")
    _require(request["targets"] == context["targets"], "target-profile")
    _require(type(request["loss_policy"]) is str and request["loss_policy"] == "allow-partial", "recipe-policy")
    for name in ("promote_protected_environment_values", "include_environment_values", "promote_same_host_bind_mounts"):
        _require(type(request[name]) is bool and request[name] is True, "recipe-consents")
    _resources(request, context)
    _labels(request, context)


def validate_selection_request(request: object, bindings: object) -> None:
    """Check six application-label IDs or all eight held IDs; no conversion additions."""
    context = _bindings(bindings)
    _budget(request)
    _fields(request, _SELECTION_FIELDS, "selection-shape")
    _identity(request, context)
    selector = request["selector"]
    _require(type(selector) is dict, "selection-selector")
    kind = selector.get("kind")
    _require(type(kind) is str and kind in {"label", "all"}, "selection-selector")
    expected = set(context["role_ids"].values())
    if kind == "label":
        _fields(selector, {"kind", "key", "value"}, "selection-selector")
        _require(selector["key"] == "io.boxferry.application"
                 and selector["value"] == context["application_name"], "selection-selector")
    else:
        _fields(selector, {"kind"}, "selection-selector")
        expected.update((context["proxy_id"], context["peer_id"]))
    _members(request["expected_container_ids"], expected, "selection-membership")
