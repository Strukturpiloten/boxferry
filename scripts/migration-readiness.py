#!/usr/bin/env python3
"""Run and validate BoxFerry's bounded migration-readiness tiers."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import math
import os
import pathlib
import pwd
import re
import resource
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import tomllib
import uuid
from collections.abc import Callable
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parent.parent
CATALOGUE = ROOT / "fixtures/conformance/migration-readiness/tiers.toml"
PODMAN_MATRIX = ROOT / "fixtures/conformance/podman-live/matrix.tsv"
PODMAN_LIMITATIONS = ROOT / "fixtures/conformance/podman-live/limitations.tsv"
DEFAULT_EVIDENCE = ROOT / "target/migration-readiness/evidence-v2.json"
SHA_RE = re.compile(r"^[0-9a-f]{40}$")
FINAL_STATES = {"passed", "failed", "unavailable", "not-run"}
EVIDENCE_SCHEMA = ROOT / "docs/schemas/migration-readiness-evidence-v2.schema.json"
SAMPLE_INTERVAL_SECONDS = 0.25
SAMPLE_INTERVAL_MILLISECONDS = 250
CATALOGUE_KEYS = {"schema", "evidence-schema", "gaps", "tiers", "tasks"}
GAP_IDS = {
    "gpu",
    "virtual-machine",
    "selinux-enforcing-runtime",
    "booted-systemd",
}
TIER_IDS = {"offline", "trusted-live", "pre-release"}
TASK_IDS = {
    "offline-application-contracts",
    "podman-api-5.4-rootless",
    "podman-api-6.1-rootful",
    "podman-api-6.1-rootless",
    "podman-complete-matrix-shard-1",
    "podman-complete-matrix-shard-2",
    "podman-complete-matrix-shard-3",
    "podman-complete-matrix-shard-4",
    "nextcloud-application",
    "forgejo-root-modes",
    "paperless-application",
    "immich-application",
    "observability-application",
    "supabase-application",
    "compose-lens-candidate",
    "quadlet-lens-candidate",
}
LENS_TASKS = {
    "compose-lens-candidate": "compose-lens",
    "quadlet-lens-candidate": "quadlet-lens",
}
MATRIX_SHARD_RE = re.compile(r"^podman-complete-matrix-shard-([1-4])$")
TIER_KEYS = {
    "id",
    "description",
    "max-concurrency",
    "tier-deadline-seconds",
    "manual-prerequisites",
    "tasks",
}
TASK_COMMON_KEYS = {
    "id",
    "kind",
    "privileged",
    "workload",
    "sources",
    "targets",
    "approved-losses",
    "runtime-claim",
    "deadline-seconds",
    "minimum-memory-mib",
    "minimum-disk-mib",
    "maximum-rss-mib",
    "maximum-disk-growth-mib",
    "required-tools",
}
COMMAND_TASK_KEYS = TASK_COMMON_KEYS | {"command"}
LENS_TASK_KEYS = TASK_COMMON_KEYS | {
    "lens",
    "repository",
    "revision",
    "commands",
}
SUPPORTED_SCHEMA_KEYWORDS = {
    "$defs",
    "$id",
    "$ref",
    "$schema",
    "additionalProperties",
    "const",
    "enum",
    "format",
    "items",
    "maxItems",
    "maximum",
    "minItems",
    "minimum",
    "minLength",
    "pattern",
    "properties",
    "required",
    "title",
    "type",
    "uniqueItems",
}


class ContractError(RuntimeError):
    """A catalogue, invocation, or evidence contract was violated."""


def catalogue_label(path: pathlib.Path) -> str:
    resolved = path.resolve()
    try:
        return str(resolved.relative_to(ROOT))
    except ValueError:
        return str(resolved)


def now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat(timespec="milliseconds").replace(
        "+00:00", "Z"
    )


def catalogue_digest(path: pathlib.Path) -> str:
    """Bind evidence to the exact reviewed catalogue bytes."""
    return hashlib.sha256(path.read_bytes()).hexdigest()


def configured_boxferry_binary_digest() -> str | None:
    """Return the digest of the exact configured binary, when one is in scope."""
    candidate = os.environ.get("BOXFERRY_BIN")
    if not candidate:
        return None
    path = pathlib.Path(candidate)
    if not path.is_file():
        return None
    return hashlib.sha256(path.read_bytes()).hexdigest()


def tabular_ids(path: pathlib.Path) -> list[str]:
    """Read unique first-column IDs from one reviewed TSV catalogue."""
    ids: list[str] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line or line.startswith("#"):
            continue
        identifier = line.split("\t", 1)[0]
        if not identifier or identifier in ids:
            raise ContractError(f"{path.relative_to(ROOT)} has an empty or duplicate id")
        ids.append(identifier)
    if not ids:
        raise ContractError(f"{path.relative_to(ROOT)} has no reviewed rows")
    return ids


def expected_matrix_evidence(task_id: str) -> dict[str, Any] | None:
    """Bind one complete-matrix worker to its exact deterministic row shard."""
    match = MATRIX_SHARD_RE.fullmatch(task_id)
    if match is None:
        return None
    shard = int(match.group(1))
    rows = tabular_ids(PODMAN_MATRIX)
    if len(rows) != 48:
        raise ContractError(f"reviewed Podman matrix must contain 48 rows, found {len(rows)}")
    limitations = set(tabular_ids(PODMAN_LIMITATIONS))
    if not limitations.issubset(rows):
        raise ContractError("Podman limitation catalogue names rows outside the reviewed matrix")
    selected = rows[shard - 1 :: 4]
    selected_limitations = [identifier for identifier in selected if identifier in limitations]
    return {
        "shard": f"{shard}/4",
        "matrix_sha256": catalogue_digest(PODMAN_MATRIX),
        "limitations_sha256": catalogue_digest(PODMAN_LIMITATIONS),
        "row_ids": selected,
        "limitation_row_ids": selected_limitations,
    }


def maximum_interval_overlap(tasks: list[dict[str, Any]]) -> int:
    """Measure maximum worker overlap from half-open task time intervals."""
    events: list[tuple[dt.datetime, int]] = []
    for task in tasks:
        started = parse_date_time(task["started_at"], f"$.tasks.{task['id']}.started_at")
        finished = parse_date_time(task["finished_at"], f"$.tasks.{task['id']}.finished_at")
        if finished <= started:
            raise ContractError(f"evidence task {task['id']} has an empty or reversed interval")
        events.extend(((started, 1), (finished, -1)))
    active = 0
    maximum = 0
    for _when, delta in sorted(events, key=lambda event: (event[0], event[1])):
        active += delta
        maximum = max(maximum, active)
    if active != 0 or maximum < 1:
        raise ContractError("evidence worker intervals are inconsistent")
    return maximum


def validate_complete_matrix_coverage(tasks: list[dict[str, Any]]) -> None:
    """Require all four exact shards to cover every reviewed row once."""
    shard_tasks = [task for task in tasks if MATRIX_SHARD_RE.fullmatch(task["id"])]
    expected_ids = [f"podman-complete-matrix-shard-{index}" for index in range(1, 5)]
    if [task["id"] for task in shard_tasks] != expected_ids:
        raise ContractError("aggregate evidence lacks the four ordered Podman matrix shards")
    row_ids = [
        identifier
        for task in shard_tasks
        for identifier in task["matrix_evidence"]["row_ids"]
    ]
    limitation_ids = [
        identifier
        for task in shard_tasks
        for identifier in task["matrix_evidence"]["limitation_row_ids"]
    ]
    expected_rows = tabular_ids(PODMAN_MATRIX)
    expected_limitations = tabular_ids(PODMAN_LIMITATIONS)
    if len(row_ids) != len(set(row_ids)) or set(row_ids) != set(expected_rows):
        raise ContractError("Podman shard evidence must cover all 48 matrix rows exactly once")
    if len(limitation_ids) != len(set(limitation_ids)) or set(limitation_ids) != set(
        expected_limitations
    ):
        raise ContractError("Podman shard evidence must cover all five limitation rows exactly once")


def json_type_matches(value: Any, expected: str) -> bool:
    """Match JSON types without Python's bool-is-int ambiguity."""
    return {
        "null": value is None,
        "boolean": isinstance(value, bool),
        "integer": isinstance(value, int) and not isinstance(value, bool),
        "number": isinstance(value, (int, float)) and not isinstance(value, bool),
        "string": isinstance(value, str),
        "array": isinstance(value, list),
        "object": isinstance(value, dict),
    }.get(expected, False)


def json_equal(left: Any, right: Any) -> bool:
    """Compare JSON values with type identity, including nested values."""
    return type(left) is type(right) and left == right


def parse_date_time(value: str, location: str) -> dt.datetime:
    if re.fullmatch(
        r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})",
        value,
    ) is None:
        raise ContractError(f"{location} is not a valid RFC 3339 date-time")
    candidate = value[:-1] + "+00:00" if value.endswith("Z") else value
    try:
        parsed = dt.datetime.fromisoformat(candidate)
    except ValueError as error:
        raise ContractError(f"{location} is not a valid date-time") from error
    if parsed.tzinfo is None or parsed.utcoffset() is None:
        raise ContractError(f"{location} date-time must include an offset")
    return parsed.astimezone(dt.timezone.utc)


def validate_schema_definition(schema: Any, location: str = "schema") -> None:
    """Reject schema features outside the deliberately small local evaluator."""
    if not isinstance(schema, dict):
        raise ContractError(f"{location} must be a JSON object")
    unknown = set(schema) - SUPPORTED_SCHEMA_KEYWORDS
    if unknown:
        raise ContractError(f"{location} uses unsupported schema keywords: {sorted(unknown)}")
    properties = schema.get("properties", {})
    definitions = schema.get("$defs", {})
    if not isinstance(properties, dict) or not isinstance(definitions, dict):
        raise ContractError(f"{location} properties and $defs must be objects")
    for name, child in properties.items():
        validate_schema_definition(child, f"{location}.properties.{name}")
    for name, child in definitions.items():
        validate_schema_definition(child, f"{location}.$defs.{name}")
    if "items" in schema:
        validate_schema_definition(schema["items"], f"{location}.items")
    reference = schema.get("$ref")
    if reference is not None and (
        not isinstance(reference, str) or not reference.startswith("#/$defs/") or "/" in reference[8:]
    ):
        raise ContractError(f"{location} has an unsupported $ref")
    expected_type = schema.get("type")
    if expected_type is not None:
        types = expected_type if isinstance(expected_type, list) else [expected_type]
        if not types or any(item not in {"null", "boolean", "integer", "number", "string", "array", "object"} for item in types):
            raise ContractError(f"{location} has an unsupported type")
    if schema.get("additionalProperties", False) not in {False}:
        raise ContractError(f"{location} may only use additionalProperties=false")
    if "format" in schema and schema["format"] not in {"uuid", "date-time"}:
        raise ContractError(f"{location} has an unsupported format")


def resolve_schema_reference(root_schema: dict[str, Any], reference: str) -> dict[str, Any]:
    name = reference.removeprefix("#/$defs/")
    resolved = root_schema.get("$defs", {}).get(name)
    if not isinstance(resolved, dict):
        raise ContractError(f"schema reference {reference} does not resolve")
    return resolved


def validate_json_schema(
    value: Any,
    schema: dict[str, Any],
    root_schema: dict[str, Any],
    location: str = "$",
) -> None:
    """Evaluate only the audited JSON-Schema subset used by readiness evidence."""
    if "$ref" in schema:
        validate_json_schema(value, resolve_schema_reference(root_schema, schema["$ref"]), root_schema, location)
        return
    expected_type = schema.get("type")
    if expected_type is not None:
        types = expected_type if isinstance(expected_type, list) else [expected_type]
        if not any(json_type_matches(value, item) for item in types):
            raise ContractError(f"{location} has the wrong JSON type; expected {types}")
    if "const" in schema and not json_equal(value, schema["const"]):
        raise ContractError(f"{location} does not match schema const")
    if "enum" in schema and not any(json_equal(value, candidate) for candidate in schema["enum"]):
        raise ContractError(f"{location} is not one of the schema enum values")
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        if not math.isfinite(value):
            raise ContractError(f"{location} must be finite")
        if "minimum" in schema and value < schema["minimum"]:
            raise ContractError(f"{location} is below schema minimum")
        if "maximum" in schema and value > schema["maximum"]:
            raise ContractError(f"{location} is above schema maximum")
    if isinstance(value, str):
        if len(value) < schema.get("minLength", 0):
            raise ContractError(f"{location} is shorter than schema minLength")
        if "pattern" in schema and re.search(schema["pattern"], value) is None:
            raise ContractError(f"{location} does not match schema pattern")
        if schema.get("format") == "uuid":
            try:
                parsed_uuid = uuid.UUID(value)
            except ValueError as error:
                raise ContractError(f"{location} is not a UUID") from error
            if str(parsed_uuid) != value:
                raise ContractError(f"{location} is not a canonical UUID")
        elif schema.get("format") == "date-time":
            parse_date_time(value, location)
    if isinstance(value, list):
        if len(value) < schema.get("minItems", 0):
            raise ContractError(f"{location} has fewer than minItems")
        if "maxItems" in schema and len(value) > schema["maxItems"]:
            raise ContractError(f"{location} has more than maxItems")
        if schema.get("uniqueItems"):
            encoded = [json.dumps(item, sort_keys=True, separators=(",", ":")) for item in value]
            if len(encoded) != len(set(encoded)):
                raise ContractError(f"{location} must contain unique items")
        if "items" in schema:
            for index, item in enumerate(value):
                validate_json_schema(item, schema["items"], root_schema, f"{location}[{index}]")
    if isinstance(value, dict):
        properties = schema.get("properties", {})
        for required in schema.get("required", []):
            if required not in value:
                raise ContractError(f"{location} is missing required property {required}")
        if schema.get("additionalProperties") is False:
            unexpected = set(value) - set(properties)
            if unexpected:
                raise ContractError(f"{location} has additional properties: {sorted(unexpected)}")
        for name, item in value.items():
            if name in properties:
                validate_json_schema(item, properties[name], root_schema, f"{location}.{name}")


def load_evidence_schema(catalogue: dict[str, Any]) -> dict[str, Any]:
    schema_label = catalogue.get("evidence-schema")
    expected_label = str(EVIDENCE_SCHEMA.relative_to(ROOT))
    if schema_label != expected_label:
        raise ContractError(f"catalogue evidence-schema must be {expected_label}")
    with EVIDENCE_SCHEMA.open(encoding="utf-8") as stream:
        schema = json.load(stream)
    validate_schema_definition(schema)
    return schema


def require_exact_keys(
    value: Any,
    required: set[str],
    optional: set[str],
    location: str,
) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ContractError(f"{location} must be a table")
    keys = set(value)
    missing = required - keys
    unknown = keys - required - optional
    if missing:
        raise ContractError(f"{location} is missing required fields: {sorted(missing)}")
    if unknown:
        raise ContractError(f"{location} has unknown fields: {sorted(unknown)}")
    return value


def require_nonempty_string(value: Any, location: str) -> str:
    if not isinstance(value, str) or not value:
        raise ContractError(f"{location} must be a nonempty string")
    return value


def require_positive_integer(value: Any, location: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        raise ContractError(f"{location} must be a positive integer")
    return value


def require_string_list(value: Any, location: str, *, allow_empty: bool) -> list[str]:
    if not isinstance(value, list) or (not allow_empty and not value):
        raise ContractError(f"{location} must be a string array")
    if any(not isinstance(item, str) or not item for item in value):
        raise ContractError(f"{location} must contain only nonempty strings")
    if len(value) != len(set(value)):
        raise ContractError(f"{location} must contain unique strings")
    return value


def validate_command(value: Any, location: str) -> None:
    if not isinstance(value, list) or not value:
        raise ContractError(f"{location} must be a nonempty command array")
    if any(not isinstance(argument, str) or not argument for argument in value):
        raise ContractError(f"{location} must contain only nonempty strings")


def validate_catalogue_shape(catalogue: Any) -> None:
    root = require_exact_keys(catalogue, CATALOGUE_KEYS, set(), "catalogue")
    if not isinstance(root["schema"], int) or isinstance(root["schema"], bool):
        raise ContractError("catalogue schema must be an integer")
    require_nonempty_string(root["evidence-schema"], "catalogue evidence-schema")

    gaps = root["gaps"]
    tiers = root["tiers"]
    tasks = root["tasks"]
    for name, items in (("gaps", gaps), ("tiers", tiers), ("tasks", tasks)):
        if not isinstance(items, list) or not items:
            raise ContractError(f"catalogue {name} must be a nonempty table array")

    for index, gap_value in enumerate(gaps):
        gap = require_exact_keys(
            gap_value, {"id", "state", "reason"}, set(), f"catalogue gaps[{index}]"
        )
        gap_id = require_nonempty_string(gap["id"], f"catalogue gaps[{index}].id")
        if gap_id not in GAP_IDS:
            raise ContractError(f"catalogue gaps[{index}] has unknown id {gap_id}")
        state = require_nonempty_string(
            gap["state"], f"catalogue gaps[{index}].state"
        )
        if state not in {"planned", "not-executed"}:
            raise ContractError(f"catalogue gaps[{index}] has a successful state")
        require_nonempty_string(gap["reason"], f"catalogue gaps[{index}].reason")

    for index, tier_value in enumerate(tiers):
        tier = require_exact_keys(
            tier_value, TIER_KEYS, set(), f"catalogue tiers[{index}]"
        )
        tier_id = require_nonempty_string(tier["id"], f"catalogue tiers[{index}].id")
        if tier_id not in TIER_IDS:
            raise ContractError(f"catalogue tiers[{index}] has unknown id {tier_id}")
        require_nonempty_string(
            tier["description"], f"catalogue tiers[{index}].description"
        )
        concurrency = require_positive_integer(
            tier["max-concurrency"], f"catalogue tiers[{index}].max-concurrency"
        )
        if concurrency > 4:
            raise ContractError(f"catalogue tier {tier_id} has unsafe max-concurrency")
        require_positive_integer(
            tier["tier-deadline-seconds"],
            f"catalogue tiers[{index}].tier-deadline-seconds",
        )
        require_string_list(
            tier["manual-prerequisites"],
            f"catalogue tiers[{index}].manual-prerequisites",
            allow_empty=True,
        )
        selected = require_string_list(
            tier["tasks"], f"catalogue tiers[{index}].tasks", allow_empty=False
        )
        unknown_tasks = set(selected) - TASK_IDS
        if unknown_tasks:
            raise ContractError(
                f"catalogue tier {tier_id} selects unknown tasks: {sorted(unknown_tasks)}"
            )

    for index, task_value in enumerate(tasks):
        location = f"catalogue tasks[{index}]"
        if not isinstance(task_value, dict):
            raise ContractError(f"{location} must be a table")
        kind = task_value.get("kind")
        if kind == "command":
            required_keys = COMMAND_TASK_KEYS
        elif kind == "lens-consumer":
            required_keys = LENS_TASK_KEYS
        else:
            raise ContractError(f"{location}.kind must be command or lens-consumer")
        task = require_exact_keys(
            task_value, required_keys, {"required-environment"}, location
        )
        task_id = require_nonempty_string(task["id"], f"{location}.id")
        if task_id not in TASK_IDS:
            raise ContractError(f"{location} has unknown id {task_id}")
        expected_kind = "lens-consumer" if task_id in LENS_TASKS else "command"
        if kind != expected_kind:
            raise ContractError(
                f"{location} id {task_id} must use kind {expected_kind}"
            )
        for name in ("kind", "workload", "approved-losses", "runtime-claim"):
            require_nonempty_string(task[name], f"{location}.{name}")
        if not isinstance(task["privileged"], bool):
            raise ContractError(f"{location}.privileged must be a boolean")
        for name in ("sources", "targets", "required-tools"):
            require_string_list(task[name], f"{location}.{name}", allow_empty=False)
        if "required-environment" in task:
            require_string_list(
                task["required-environment"],
                f"{location}.required-environment",
                allow_empty=False,
            )
        for name in (
            "deadline-seconds",
            "minimum-memory-mib",
            "minimum-disk-mib",
            "maximum-rss-mib",
            "maximum-disk-growth-mib",
        ):
            require_positive_integer(task[name], f"{location}.{name}")
        if kind == "command":
            validate_command(task["command"], f"{location}.command")
        else:
            for name in ("lens", "repository", "revision"):
                require_nonempty_string(task[name], f"{location}.{name}")
            if task["lens"] != LENS_TASKS[task_id]:
                raise ContractError(f"{location}.lens does not match task id {task_id}")
            commands = task["commands"]
            if not isinstance(commands, list) or not commands:
                raise ContractError(f"{location}.commands must be a nonempty array")
            for command_index, command in enumerate(commands):
                validate_command(command, f"{location}.commands[{command_index}]")


def load_catalogue(path: pathlib.Path = CATALOGUE) -> dict[str, Any]:
    with path.open("rb") as stream:
        catalogue = tomllib.load(stream)
    validate_catalogue_shape(catalogue)
    if catalogue.get("schema") != 2:
        raise ContractError("migration-readiness catalogue schema must be 2")
    tasks = catalogue.get("tasks", [])
    tiers = catalogue.get("tiers", [])
    gaps = catalogue.get("gaps", [])
    task_ids = [item.get("id") for item in tasks]
    tier_ids = [item.get("id") for item in tiers]
    gap_ids = [item.get("id") for item in gaps]
    for label, values in (("task", task_ids), ("tier", tier_ids), ("gap", gap_ids)):
        if not values or any(not isinstance(value, str) or not value for value in values):
            raise ContractError(f"catalogue has an invalid {label} id")
        if len(values) != len(set(values)):
            raise ContractError(f"catalogue has duplicate {label} ids")
    for label, actual, expected in (
        ("task", set(task_ids), TASK_IDS),
        ("tier", set(tier_ids), TIER_IDS),
        ("gap", set(gap_ids), GAP_IDS),
    ):
        if actual != expected:
            raise ContractError(
                f"catalogue {label} ids must match the reviewed allowlist: "
                f"missing {sorted(expected - actual)}, unknown {sorted(actual - expected)}"
            )
    known_tasks = set(task_ids)
    for tier in tiers:
        selected = tier.get("tasks", [])
        if not selected or len(selected) != len(set(selected)):
            raise ContractError(f"tier {tier['id']} must select unique tasks")
        unknown = set(selected) - known_tasks
        if unknown:
            raise ContractError(f"tier {tier['id']} selects unknown tasks: {sorted(unknown)}")
        concurrency = tier.get("max-concurrency")
        if not isinstance(concurrency, int) or not 1 <= concurrency <= 4:
            raise ContractError(f"tier {tier['id']} has unsafe max-concurrency")
    for task in tasks:
        validate_task(task)
    required_gaps = {
        "gpu",
        "virtual-machine",
        "selinux-enforcing-runtime",
        "booted-systemd",
    }
    if set(gap_ids) != required_gaps:
        raise ContractError("catalogue explicit gap set changed without a schema decision")
    if any(item.get("state") not in {"planned", "not-executed"} for item in gaps):
        raise ContractError("catalogue gaps must remain explicit non-success states")
    return catalogue


def validate_task(task: dict[str, Any]) -> None:
    required_strings = (
        "id",
        "kind",
        "workload",
        "approved-losses",
        "runtime-claim",
    )
    for name in required_strings:
        if not isinstance(task.get(name), str) or not task[name]:
            raise ContractError(f"task has invalid {name}")
    if task["kind"] not in {"command", "lens-consumer"}:
        raise ContractError(f"task {task['id']} has unknown kind")
    if not isinstance(task.get("privileged"), bool):
        raise ContractError(f"task {task['id']} must declare privilege")
    for name in ("sources", "targets", "required-tools"):
        value = task.get(name)
        if not isinstance(value, list) or not value or not all(isinstance(item, str) and item for item in value):
            raise ContractError(f"task {task['id']} has invalid {name}")
    for name in (
        "deadline-seconds",
        "minimum-memory-mib",
        "minimum-disk-mib",
        "maximum-rss-mib",
        "maximum-disk-growth-mib",
    ):
        value = task.get(name)
        if not isinstance(value, int) or value <= 0:
            raise ContractError(f"task {task['id']} has invalid {name}")
    commands = task.get("commands") if task["kind"] == "lens-consumer" else [task.get("command")]
    if not isinstance(commands, list) or not commands:
        raise ContractError(f"task {task['id']} lacks commands")
    for command in commands:
        if not isinstance(command, list) or not command or not all(isinstance(arg, str) and arg for arg in command):
            raise ContractError(f"task {task['id']} has unsafe command")
    if task["kind"] == "lens-consumer":
        if task.get("lens") not in {"compose-lens", "quadlet-lens"}:
            raise ContractError(f"task {task['id']} has unknown Lens")
        if not SHA_RE.fullmatch(task.get("revision", "")):
            raise ContractError(f"task {task['id']} must pin an exact Lens revision")
        expected_repository = f"https://github.com/Strukturpiloten/{task['lens']}.git"
        if task.get("repository") != expected_repository:
            raise ContractError(f"task {task['id']} uses an unreviewed Lens repository")


def by_id(items: list[dict[str, Any]], item_id: str, kind: str) -> dict[str, Any]:
    for item in items:
        if item["id"] == item_id:
            return item
    raise ContractError(f"unknown {kind}: {item_id}")


def selected_tasks(catalogue: dict[str, Any], tier_id: str, task_id: str | None) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    tier = by_id(catalogue["tiers"], tier_id, "tier")
    ids = tier["tasks"]
    if task_id is not None:
        if task_id not in ids:
            raise ContractError(f"task {task_id} does not belong to tier {tier_id}")
        ids = [task_id]
    tasks = [by_id(catalogue["tasks"], item, "task") for item in ids]
    return tier, tasks


def catalogue_lens_revisions(catalogue: dict[str, Any]) -> dict[str, str]:
    return {
        "compose-lens": by_id(
            catalogue["tasks"], "compose-lens-candidate", "task"
        )["revision"],
        "quadlet-lens": by_id(
            catalogue["tasks"], "quadlet-lens-candidate", "task"
        )["revision"],
    }


def git_revision() -> str:
    result = subprocess.run(
        ["git", "-c", f"safe.directory={ROOT}", "rev-parse", "HEAD"],
        cwd=ROOT,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    )
    revision = result.stdout.strip()
    if not SHA_RE.fullmatch(revision):
        raise ContractError("repository HEAD is not an exact Git commit")
    return revision


def memory_available_mib() -> int | None:
    meminfo = pathlib.Path("/proc/meminfo")
    if meminfo.is_file():
        for line in meminfo.read_text(encoding="utf-8").splitlines():
            if line.startswith("MemAvailable:"):
                return int(line.split()[1]) // 1024
    if shutil.which("sysctl"):
        result = subprocess.run(
            ["sysctl", "-n", "hw.memsize"], check=False, text=True, stdout=subprocess.PIPE
        )
        if result.returncode == 0 and result.stdout.strip().isdigit():
            return int(result.stdout.strip()) // (1024 * 1024)
    return None


def process_tree_rss_kib(root_pid: int) -> int:
    proc = pathlib.Path("/proc")
    if not proc.is_dir():
        return 0
    parents: dict[int, int] = {}
    rss: dict[int, int] = {}
    for entry in proc.iterdir():
        if not entry.name.isdigit():
            continue
        try:
            status = (entry / "status").read_text(encoding="utf-8")
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            continue
        parent = 0
        resident = 0
        for line in status.splitlines():
            if line.startswith("PPid:"):
                parent = int(line.split()[1])
            elif line.startswith("VmRSS:"):
                resident = int(line.split()[1])
        parents[int(entry.name)] = parent
        rss[int(entry.name)] = resident
    selected = {root_pid}
    changed = True
    while changed:
        before = len(selected)
        selected.update(pid for pid, parent in parents.items() if parent in selected)
        changed = len(selected) != before
    return sum(rss.get(pid, 0) for pid in selected)


def existing_ancestor(path: pathlib.Path) -> pathlib.Path:
    """Return the closest existing path used to identify a future filesystem."""
    candidate = path.expanduser().resolve()
    while not candidate.exists():
        parent = candidate.parent
        if parent == candidate:
            raise ContractError(f"no existing ancestor for measurement path {path}")
        candidate = parent
    return candidate


def discover_podman_graph_root(timeout_seconds: float = 10.0) -> pathlib.Path | None:
    """Read Podman's graph root without issuing a mutating runtime request."""
    if shutil.which("podman") is None:
        return None
    try:
        result = subprocess.run(
            ["podman", "info", "--format", "{{.Store.GraphRoot}}"],
            check=False,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout_seconds,
        )
    except subprocess.TimeoutExpired:
        return None
    graph_root = result.stdout.strip()
    if result.returncode != 0 or not graph_root:
        return None
    return pathlib.Path(graph_root).expanduser().resolve()


class ResourceSampler:
    """Retain resource peaks for one task across all of its child commands."""

    def __init__(
        self,
        measurement_paths: list[tuple[str, pathlib.Path]],
        *,
        memory_reader: Callable[[], int | None] = memory_available_mib,
        disk_usage_reader: Callable[[pathlib.Path], Any] = shutil.disk_usage,
        stat_reader: Callable[[pathlib.Path], Any] = os.stat,
    ) -> None:
        self._memory_reader = memory_reader
        self._disk_usage_reader = disk_usage_reader
        self.baseline_memory_mib = memory_reader()
        self._memory_samples_complete = self.baseline_memory_mib is not None
        self.peak_memory_delta_mib: int | None = (
            0 if self.baseline_memory_mib is not None else None
        )
        self.peak_rss_kib = 0
        self._filesystems: dict[str, dict[str, Any]] = {}

        for role, path in measurement_paths:
            resolved = path.expanduser().resolve()
            ancestor = existing_ancestor(resolved)
            device = str(stat_reader(ancestor).st_dev)
            record = self._filesystems.get(device)
            if record is None:
                free_bytes = int(disk_usage_reader(ancestor).free)
                record = {
                    "device": device,
                    "roles": [],
                    "paths": [],
                    "measurement_path": ancestor,
                    "baseline_free_bytes": free_bytes,
                    "minimum_free_bytes": free_bytes,
                }
                self._filesystems[device] = record
            if role not in record["roles"]:
                record["roles"].append(role)
            path_label = str(resolved)
            if path_label not in record["paths"]:
                record["paths"].append(path_label)

    def sample(self, root_pid: int | None = None) -> None:
        available = self._memory_reader()
        if available is None or self.baseline_memory_mib is None:
            self._memory_samples_complete = False
        elif self._memory_samples_complete:
            delta = max(0, self.baseline_memory_mib - available)
            self.peak_memory_delta_mib = max(self.peak_memory_delta_mib or 0, delta)
        if root_pid is not None:
            self.peak_rss_kib = max(self.peak_rss_kib, process_tree_rss_kib(root_pid))
        for record in self._filesystems.values():
            free_bytes = int(self._disk_usage_reader(record["measurement_path"]).free)
            record["minimum_free_bytes"] = min(record["minimum_free_bytes"], free_bytes)

    def apply_rss_fallback(self) -> None:
        if self.peak_rss_kib == 0:
            self.peak_rss_kib = int(
                resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
            )

    def snapshot(self, *, sample: bool = True) -> dict[str, Any]:
        if sample:
            self.sample()
        mebibyte = 1024 * 1024
        filesystems = []
        total_growth_mib = 0
        for record in self._filesystems.values():
            growth_bytes = max(
                0, record["baseline_free_bytes"] - record["minimum_free_bytes"]
            )
            growth_mib = (growth_bytes + mebibyte - 1) // mebibyte
            total_growth_mib += growth_mib
            filesystems.append(
                {
                    "device": record["device"],
                    "roles": sorted(record["roles"]),
                    "paths": sorted(record["paths"]),
                    "baseline_free_mib": record["baseline_free_bytes"] // mebibyte,
                    "minimum_free_mib": record["minimum_free_bytes"] // mebibyte,
                    "peak_growth_mib": growth_mib,
                }
            )
        filesystems.sort(key=lambda item: item["device"])
        available_disk_mib = min(
            (item["baseline_free_mib"] for item in filesystems), default=None
        )
        return {
            "available_memory_mib": self.baseline_memory_mib,
            "available_disk_mib": available_disk_mib,
            "peak_memory_delta_mib": (
                self.peak_memory_delta_mib if self._memory_samples_complete else None
            ),
            "peak_rss_kib": self.peak_rss_kib,
            "disk_growth_mib": total_growth_mib,
            "sample_interval_milliseconds": SAMPLE_INTERVAL_MILLISECONDS,
            "memory_scope": "system-available-memory",
            "rss_scope": "child-process-tree",
            "disk_scope": "deduplicated-filesystems",
            "filesystems": filesystems,
        }


def resource_sampler_for_task(
    task: dict[str, Any], tier_deadline: float | None = None
) -> tuple[ResourceSampler, str | None]:
    paths = [
        ("checkout", ROOT),
        ("temporary-directory", pathlib.Path(tempfile.gettempdir())),
    ]
    discovery_error = None
    if "podman" in task["required-tools"] and shutil.which("podman") is not None:
        timeout_seconds = 10.0
        if tier_deadline is not None:
            timeout_seconds = min(
                timeout_seconds, max(0.001, tier_deadline - time.monotonic())
            )
        graph_root = discover_podman_graph_root(timeout_seconds)
        if graph_root is None:
            discovery_error = "Podman graph root could not be discovered read-only"
        else:
            paths.append(("podman-graph-root", graph_root))
    return ResourceSampler(paths), discovery_error


def emit(step: int, total: int, state: str, label: str, detail: str = "") -> None:
    suffix = f" {detail}" if detail else ""
    print(f"{now()} EVENT {step:02d}/{total:02d} {state} {label}{suffix}", flush=True)


def execution_identity(task: dict[str, Any]) -> tuple[dict[str, Any] | None, str | None]:
    effective_uid = os.geteuid()
    effective_gid = os.getegid()
    if task["privileged"]:
        if effective_uid != 0:
            return None, "task requires root privileges"
        account = pwd.getpwuid(0)
        return {
            "uid": 0,
            "gid": 0,
            "user": account.pw_name,
            "home": account.pw_dir,
            "groups": [0],
        }, None

    if effective_uid != 0:
        try:
            account = pwd.getpwuid(effective_uid)
        except KeyError:
            return None, "current non-root UID does not name a local account"
        groups = os.getgroups()
        if effective_gid == 0 or 0 in groups:
            return None, "non-privileged task cannot retain the root group"
        return {
            "uid": effective_uid,
            "gid": effective_gid,
            "user": account.pw_name,
            "home": account.pw_dir,
            "groups": groups,
        }, None

    sudo_values = {name: os.environ.get(name) for name in ("SUDO_UID", "SUDO_GID", "SUDO_USER")}
    if any(value is None or value == "" for value in sudo_values.values()):
        return None, "non-privileged task requires a complete non-root sudo identity"
    uid_text = sudo_values["SUDO_UID"]
    gid_text = sudo_values["SUDO_GID"]
    user = sudo_values["SUDO_USER"]
    assert uid_text is not None and gid_text is not None and user is not None
    if not uid_text.isdecimal() or not gid_text.isdecimal():
        return None, "sudo identity has a non-numeric UID or GID"
    uid = int(uid_text)
    gid = int(gid_text)
    if uid == 0 or gid == 0 or user == "root":
        return None, "non-privileged task requires a non-root sudo identity"
    try:
        account = pwd.getpwnam(user)
    except KeyError:
        return None, "sudo identity does not name a local account"
    if account.pw_uid != uid or account.pw_gid != gid:
        return None, "sudo UID, GID, and user do not identify the same local account"
    groups = os.getgrouplist(user, gid)
    if 0 in groups:
        return None, "non-privileged task cannot retain the root group"
    return {
        "uid": uid,
        "gid": gid,
        "user": user,
        "home": account.pw_dir,
        "groups": groups,
    }, None


def preflight(
    task: dict[str, Any], sampler: ResourceSampler, discovery_error: str | None
) -> tuple[dict[str, Any], str | None, dict[str, Any] | None]:
    identity, identity_error = execution_identity(task)
    missing_tools = [tool for tool in task["required-tools"] if shutil.which(tool) is None]
    missing_environment = [name for name in task.get("required-environment", []) if not os.environ.get(name)]
    for name in ("BOXFERRY_BIN", "BOXFERRY_COMPOSE_BIN"):
        candidate = os.environ.get(name)
        if (
            name in task.get("required-environment", [])
            and candidate
            and not os.access(candidate, os.X_OK)
        ):
            missing_environment.append(name)
    observed = sampler.snapshot()
    memory_mib = observed["available_memory_mib"]
    filesystems = observed["filesystems"]
    low_filesystems = [
        item
        for item in filesystems
        if item["baseline_free_mib"] < task["minimum-disk-mib"]
    ]
    reason = None
    if missing_tools:
        reason = f"missing required tools: {', '.join(missing_tools)}"
    elif missing_environment:
        reason = f"missing required environment: {', '.join(missing_environment)}"
    elif identity_error is not None:
        reason = identity_error
    elif discovery_error is not None:
        reason = discovery_error
    elif memory_mib is None:
        reason = "available memory could not be measured"
    elif memory_mib < task["minimum-memory-mib"]:
        reason = f"available memory {memory_mib} MiB is below {task['minimum-memory-mib']} MiB"
    elif low_filesystems:
        item = low_filesystems[0]
        reason = (
            f"free disk {item['baseline_free_mib']} MiB on filesystem "
            f"{item['device']} is below {task['minimum-disk-mib']} MiB"
        )
    observed |= {
        "missing_tools": missing_tools,
        "missing_environment": missing_environment,
        "effective_uid": identity["uid"] if identity is not None else os.geteuid(),
    }
    return observed, reason, identity


def run_process(
    command: list[str],
    cwd: pathlib.Path,
    deadline: float,
    sampler: ResourceSampler,
    identity: dict[str, Any],
) -> tuple[int, int, bool]:
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        sampler.sample()
        return 124, sampler.peak_rss_kib, True
    sampler.sample()
    child_environment = os.environ.copy()
    child_environment.update(
        {
            "HOME": identity["home"],
            "USER": identity["user"],
            "LOGNAME": identity["user"],
        }
    )
    for name in ("SUDO_UID", "SUDO_GID", "SUDO_USER"):
        child_environment.pop(name, None)

    demote = None
    if os.geteuid() == 0 and identity["uid"] != 0:
        def demote() -> None:
            os.setgroups(identity["groups"])
            os.setgid(identity["gid"])
            os.setuid(identity["uid"])

    child = subprocess.Popen(
        command,
        cwd=cwd,
        env=child_environment,
        preexec_fn=demote,
        start_new_session=True,
    )
    timed_out = False
    while child.poll() is None:
        sampler.sample(child.pid)
        if time.monotonic() >= deadline:
            timed_out = True
            try:
                os.killpg(child.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                child.wait(timeout=20)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                child.wait()
            break
        time.sleep(SAMPLE_INTERVAL_SECONDS)
    sampler.sample(child.pid)
    return_code = child.returncode
    if return_code is None:
        raise ContractError("child process ended without a return code")
    exit_status = 124 if timed_out else (return_code if return_code >= 0 else 128 - return_code)
    return exit_status, sampler.peak_rss_kib, timed_out


def lens_commands(
    task: dict[str, Any],
    revisions: dict[str, str],
    deadline: float,
    sampler: ResourceSampler,
    identity: dict[str, Any],
) -> tuple[int, int, bool, str]:
    lens = task["lens"]
    revision = revisions[lens]
    with tempfile.TemporaryDirectory(prefix=f"boxferry-{lens}-") as temporary:
        if os.geteuid() == 0 and identity["uid"] != 0:
            os.chown(temporary, identity["uid"], identity["gid"])
        checkout = pathlib.Path(temporary) / lens
        checkout.mkdir()
        if os.geteuid() == 0 and identity["uid"] != 0:
            os.chown(checkout, identity["uid"], identity["gid"])
        setup = [
            ["git", "init", "--quiet"],
            ["git", "remote", "add", "origin", task["repository"]],
            ["git", "fetch", "--quiet", "--depth", "1", "origin", revision],
            ["git", "checkout", "--quiet", "--detach", "FETCH_HEAD"],
        ]
        peak = 0
        for command in setup:
            status, rss, timed_out = run_process(
                command, checkout, deadline, sampler, identity
            )
            peak = max(peak, rss)
            if status != 0:
                return status, peak, timed_out, revision
        actual = (checkout / ".git/HEAD").read_text(encoding="utf-8").strip()
        if actual != revision:
            raise ContractError(f"{lens} checkout resolved {actual}, expected {revision}")
        for command in task["commands"]:
            status, rss, timed_out = run_process(
                command, checkout, deadline, sampler, identity
            )
            peak = max(peak, rss)
            if status != 0:
                return status, peak, timed_out, revision
        return 0, peak, False, revision


def run_task(
    task: dict[str, Any],
    revisions: dict[str, str],
    step: int,
    total_steps: int,
    tier_deadline: float,
) -> tuple[dict[str, Any], int]:
    started_at = now()
    started = time.monotonic()
    emit(step, total_steps, "START", f"{task['id']}:preflight")
    if started >= tier_deadline:
        reason = "tier deadline exhausted before task preflight"
        emit(step, total_steps, "FAIL", f"{task['id']}:preflight", reason)
        step += 1
        emit(step, total_steps, "START", f"{task['id']}:execute")
        emit(step, total_steps, "GAP", f"{task['id']}:execute", "not run")
        step += 1
        return task_evidence(task, started_at, started, "failed", {}, reason), step
    sampler, discovery_error = resource_sampler_for_task(task, tier_deadline)
    observed_preflight, unavailable, identity = preflight(task, sampler, discovery_error)
    step += 1
    if unavailable:
        emit(step - 1, total_steps, "GAP", f"{task['id']}:preflight", unavailable)
        emit(step, total_steps, "START", f"{task['id']}:execute")
        emit(step, total_steps, "GAP", f"{task['id']}:execute", "not run")
        step += 1
        return task_evidence(task, started_at, started, "unavailable", observed_preflight, unavailable), step
    emit(step - 1, total_steps, "PASS", f"{task['id']}:preflight")
    assert identity is not None
    emit(step, total_steps, "START", f"{task['id']}:execute")
    if time.monotonic() >= tier_deadline:
        reason = "tier deadline exhausted during task preflight"
        emit(step, total_steps, "FAIL", f"{task['id']}:execute", reason)
        step += 1
        return task_evidence(
            task, started_at, started, "failed", observed_preflight, reason
        ), step
    deadline = min(started + task["deadline-seconds"], tier_deadline)
    lens_revision = None
    if task["kind"] == "lens-consumer":
        status, peak_rss_kib, timed_out, lens_revision = lens_commands(
            task, revisions, deadline, sampler, identity
        )
    else:
        status, peak_rss_kib, timed_out = run_process(
            task["command"], ROOT, deadline, sampler, identity
        )
    elapsed = time.monotonic() - started
    # ru_maxrss is a useful portable fallback when /proc process-tree sampling is unavailable.
    sampler.apply_rss_fallback()
    observed_resources = sampler.snapshot()
    peak_rss_kib = observed_resources["peak_rss_kib"]
    peak_memory_delta_mib = observed_resources["peak_memory_delta_mib"]
    disk_growth_mib = observed_resources["disk_growth_mib"]
    budget_failure = None
    if time.monotonic() >= tier_deadline:
        budget_failure = "tier deadline exceeded during task execution"
    elif elapsed > task["deadline-seconds"] or timed_out:
        budget_failure = f"deadline exceeded ({elapsed:.3f}s > {task['deadline-seconds']}s)"
    elif peak_rss_kib > task["maximum-rss-mib"] * 1024:
        budget_failure = f"RSS budget exceeded ({peak_rss_kib // 1024} MiB > {task['maximum-rss-mib']} MiB)"
    elif (
        peak_memory_delta_mib is None
        or peak_memory_delta_mib > task["maximum-rss-mib"]
    ):
        budget_failure = (
            "system available-memory delta could not be measured"
            if peak_memory_delta_mib is None
            else "system available-memory delta budget exceeded "
            f"({peak_memory_delta_mib} MiB > {task['maximum-rss-mib']} MiB)"
        )
    elif disk_growth_mib > task["maximum-disk-growth-mib"]:
        budget_failure = (
            f"disk growth budget exceeded ({disk_growth_mib} MiB > "
            f"{task['maximum-disk-growth-mib']} MiB)"
        )
    reason = budget_failure or (None if status == 0 else f"command exited with status {status}")
    state = "passed" if reason is None else "failed"
    emit(step, total_steps, "PASS" if state == "passed" else "FAIL", f"{task['id']}:execute", reason or "")
    step += 1
    observed = observed_preflight | observed_resources | {
        "wall_seconds": round(elapsed, 3),
        "exit_status": status,
        "timed_out": timed_out,
        "concurrency": 1,
    }
    if lens_revision is not None:
        observed["lens_revision"] = lens_revision
    return task_evidence(task, started_at, started, state, observed, reason), step


def task_evidence(
    task: dict[str, Any],
    started_at: str,
    started: float,
    state: str,
    observed: dict[str, Any],
    reason: str | None,
) -> dict[str, Any]:
    normalized_observed = {
        "wall_seconds": round(time.monotonic() - started, 3),
        "concurrency": 1,
        "available_memory_mib": None,
        "available_disk_mib": None,
        "peak_memory_delta_mib": None,
        "peak_rss_kib": 0,
        "disk_growth_mib": 0,
        "exit_status": None,
        "timed_out": None,
        "sample_interval_milliseconds": SAMPLE_INTERVAL_MILLISECONDS,
        "memory_scope": "system-available-memory",
        "rss_scope": "child-process-tree",
        "disk_scope": "deduplicated-filesystems",
        "filesystems": [],
        "missing_tools": [],
        "missing_environment": [],
        "effective_uid": os.geteuid(),
    }
    normalized_observed.update(observed)
    return {
        "id": task["id"],
        "workload": task["workload"],
        "sources": task["sources"],
        "targets": task["targets"],
        "approved_losses": task["approved-losses"],
        "runtime_claim": task["runtime-claim"],
        "matrix_evidence": expected_matrix_evidence(task["id"]),
        "privileged": task["privileged"],
        "state": state,
        "reason": reason,
        "started_at": started_at,
        "finished_at": now(),
        "budgets": {
            "deadline_seconds": task["deadline-seconds"],
            "minimum_memory_mib": task["minimum-memory-mib"],
            "minimum_disk_mib": task["minimum-disk-mib"],
            "maximum_rss_mib": task["maximum-rss-mib"],
            "maximum_disk_growth_mib": task["maximum-disk-growth-mib"],
            "maximum_concurrency": 1,
        },
        "observed": normalized_observed,
    }


def atomic_json(path: pathlib.Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
            json.dump(value, stream, indent=2, sort_keys=True)
            stream.write("\n")
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def validate_evidence_semantics(
    run: dict[str, Any],
    tasks: list[dict[str, Any]],
    expected_tasks: list[dict[str, Any]],
) -> None:
    run_started = parse_date_time(run["started_at"], "$.run.started_at")
    run_finished = parse_date_time(run["finished_at"], "$.run.finished_at")
    if run_finished < run_started:
        raise ContractError("evidence run chronology is reversed")
    if (
        run["evidence_kind"] != "worker"
        and run["wall_seconds"] > run["tier_deadline_seconds"]
        and not run["timed_out"]
    ):
        raise ContractError("evidence run exceeded its tier deadline without timing out")

    previous_finished = run_started
    for actual, expected in zip(tasks, expected_tasks, strict=True):
        task_started = parse_date_time(
            actual["started_at"], f"$.tasks.{actual['id']}.started_at"
        )
        task_finished = parse_date_time(
            actual["finished_at"], f"$.tasks.{actual['id']}.finished_at"
        )
        if (
            task_finished < task_started
            or (run.get("evidence_kind") != "aggregate" and task_started < previous_finished)
        ):
            raise ContractError(f"evidence task {actual['id']} chronology is reversed")
        if task_finished > run_finished:
            raise ContractError(f"evidence task {actual['id']} finishes after its run")
        previous_finished = max(previous_finished, task_finished)

        observed = actual["observed"]
        if actual["state"] != "passed":
            continue
        budgets = actual["budgets"]
        if observed["wall_seconds"] > budgets["deadline_seconds"]:
            raise ContractError(f"successful evidence task {actual['id']} exceeded deadline")
        if observed["available_memory_mib"] is None:
            raise ContractError(
                f"successful evidence task {actual['id']} lacks available memory"
            )
        if observed["available_memory_mib"] < budgets["minimum_memory_mib"]:
            raise ContractError(
                f"successful evidence task {actual['id']} began below its memory minimum"
            )
        if observed["peak_memory_delta_mib"] is None:
            raise ContractError(
                f"successful evidence task {actual['id']} lacks a memory peak"
            )
        if observed["peak_memory_delta_mib"] > budgets["maximum_rss_mib"]:
            raise ContractError(
                f"successful evidence task {actual['id']} exceeded system memory budget"
            )
        if observed["peak_rss_kib"] > budgets["maximum_rss_mib"] * 1024:
            raise ContractError(f"successful evidence task {actual['id']} exceeded RSS budget")
        if observed["disk_growth_mib"] > budgets["maximum_disk_growth_mib"]:
            raise ContractError(f"successful evidence task {actual['id']} exceeded disk budget")
        if observed["concurrency"] != budgets["maximum_concurrency"]:
            raise ContractError(
                f"successful evidence task {actual['id']} has invalid concurrency"
            )
        if observed["exit_status"] != 0 or observed["timed_out"] is not False:
            raise ContractError(
                f"successful evidence task {actual['id']} has unsuccessful process observations"
            )
        if observed["missing_tools"] or observed["missing_environment"]:
            raise ContractError(
                f"successful evidence task {actual['id']} retained missing prerequisites"
            )
        if actual["privileged"] != (observed["effective_uid"] == 0):
            raise ContractError(
                f"successful evidence task {actual['id']} has an inconsistent execution identity"
            )

        filesystems = observed["filesystems"]
        if not filesystems:
            raise ContractError(
                f"successful evidence task {actual['id']} lacks filesystem measurements"
            )
        devices = [item["device"] for item in filesystems]
        if len(devices) != len(set(devices)):
            raise ContractError(
                f"successful evidence task {actual['id']} repeats a filesystem device"
            )
        roles = {role for item in filesystems for role in item["roles"]}
        if not {"checkout", "temporary-directory"}.issubset(roles):
            raise ContractError(
                f"successful evidence task {actual['id']} lacks required filesystem scope"
            )
        if "podman" in expected["required-tools"] and "podman-graph-root" not in roles:
            raise ContractError(
                f"successful evidence task {actual['id']} lacks Podman graph-root scope"
            )
        if any(
            item["baseline_free_mib"] < budgets["minimum_disk_mib"]
            for item in filesystems
        ):
            raise ContractError(
                f"successful evidence task {actual['id']} began below its disk minimum"
            )
        if any(
            item["minimum_free_mib"] > item["baseline_free_mib"]
            for item in filesystems
        ):
            raise ContractError(
                f"successful evidence task {actual['id']} has inconsistent disk samples"
            )
        if any(
            item["peak_growth_mib"]
            not in {
                item["baseline_free_mib"] - item["minimum_free_mib"],
                item["baseline_free_mib"] - item["minimum_free_mib"] + 1,
            }
            for item in filesystems
        ):
            raise ContractError(
                f"successful evidence task {actual['id']} has inconsistent filesystem peaks"
            )
        if sum(item["peak_growth_mib"] for item in filesystems) != observed[
            "disk_growth_mib"
        ]:
            raise ContractError(
                f"successful evidence task {actual['id']} has inconsistent disk growth"
            )
        if observed["available_disk_mib"] != min(
            item["baseline_free_mib"] for item in filesystems
        ):
            raise ContractError(
                f"successful evidence task {actual['id']} has inconsistent available disk"
            )
        if expected["kind"] == "lens-consumer":
            expected_revision = run["lens_revisions"][expected["lens"]]
            if observed.get("lens_revision") != expected_revision:
                raise ContractError(
                    f"successful evidence task {actual['id']} lacks its Lens revision"
                )
    if run["evidence_kind"] == "aggregate":
        aggregate_started = min(
            parse_date_time(task["started_at"], f"$.tasks.{task['id']}.started_at")
            for task in tasks
        )
        aggregate_finished = max(
            parse_date_time(task["finished_at"], f"$.tasks.{task['id']}.finished_at")
            for task in tasks
        )
        if run_started != aggregate_started or run_finished != aggregate_finished:
            raise ContractError("aggregate run boundaries do not match worker task boundaries")
        elapsed = round((aggregate_finished - aggregate_started).total_seconds(), 3)
        if run["wall_seconds"] != elapsed:
            raise ContractError("aggregate wall time does not match worker task intervals")


def validate_evidence(
    value: dict[str, Any],
    tier: str | None,
    revision: str | None,
    task_id: str | None = None,
    catalogue_path: pathlib.Path = CATALOGUE,
) -> None:
    catalogue = load_catalogue(catalogue_path)
    evidence_schema = load_evidence_schema(catalogue)
    validate_json_schema(value, evidence_schema, evidence_schema)
    if value.get("schema_version") != 2:
        raise ContractError("evidence schema_version must be 2")
    run = value.get("run")
    if not isinstance(run, dict) or not SHA_RE.fullmatch(run.get("revision", "")):
        raise ContractError("evidence lacks an exact run revision")
    if run.get("evidence_kind") not in {"serial", "worker", "aggregate"}:
        raise ContractError("evidence has an invalid execution kind")
    if not re.fullmatch(r"[0-9a-f]{64}", run.get("catalogue_sha256", "")):
        raise ContractError("evidence lacks a catalogue digest")
    if run["catalogue_sha256"] != catalogue_digest(catalogue_path):
        raise ContractError("evidence catalogue digest does not match reviewed catalogue")
    if tier is not None and run.get("tier") != tier:
        raise ContractError(f"evidence tier is {run.get('tier')}, expected {tier}")
    if revision is not None and run.get("revision") != revision:
        raise ContractError(f"evidence revision is {run.get('revision')}, expected {revision}")
    tasks = value.get("tasks")
    if not isinstance(tasks, list) or not tasks:
        raise ContractError("evidence has no tasks")
    if len({task.get("id") for task in tasks}) != len(tasks):
        raise ContractError("evidence has duplicate task ids")
    for task in tasks:
        if task.get("state") not in FINAL_STATES:
            raise ContractError("evidence task has an invalid state")
        if task["state"] != "passed" and not task.get("reason"):
            raise ContractError("non-success evidence task lacks a reason")
    gaps = value.get("gaps")
    if not isinstance(gaps, list) or len(gaps) != 4:
        raise ContractError("evidence must retain all four explicit gaps")
    if any(gap.get("state") not in {"planned", "not-executed"} for gap in gaps):
        raise ContractError("evidence converted an explicit gap into success")
    expected_success = (
        all(task["state"] == "passed" for task in tasks)
        and not run["timed_out"]
        and (
            run["evidence_kind"] == "worker"
            or run["wall_seconds"] <= run["tier_deadline_seconds"]
        )
    )
    if value.get("outcome") != ("passed" if expected_success else "failed"):
        raise ContractError("evidence outcome disagrees with task states")

    if tier is None:
        tier = run.get("tier")
    selection = run["selection"]
    selected_task_id = selection["task"]
    if selection["kind"] == "tier" and selected_task_id is not None:
        raise ContractError("full-tier evidence selection must have a null task")
    if selection["kind"] == "task" and selected_task_id is None:
        raise ContractError("partial evidence selection must name its task")
    if task_id != selected_task_id:
        if task_id is None:
            raise ContractError(
                "partial evidence requires an explicit matching --task selection"
            )
        raise ContractError("evidence task selection does not match requested --task")
    evidence_kind = run["evidence_kind"]
    if evidence_kind == "serial":
        if selection["kind"] != "tier" or run.get("worker_id") != "serial":
            raise ContractError("serial evidence must bind a full tier to worker_id serial")
        if run.get("coordinator_id") != run.get("id"):
            raise ContractError("serial evidence must own its coordinator identity")
    elif evidence_kind == "worker":
        if selection["kind"] != "task" or run.get("worker_id") != selected_task_id:
            raise ContractError("worker evidence must bind worker_id to its selected task")
        if len(tasks) != 1:
            raise ContractError("worker evidence must contain exactly one task")
    else:
        if selection["kind"] != "tier" or run.get("worker_id") != "aggregate":
            raise ContractError("aggregate evidence must bind a full tier to worker_id aggregate")
        if run.get("coordinator_id") != run.get("id"):
            raise ContractError("aggregate evidence must own its coordinator identity")
    expected_tier, expected_tasks = selected_tasks(catalogue, tier, selected_task_id)
    expected_task_ids = [task["id"] for task in expected_tasks]
    if [task.get("id") for task in tasks] != expected_task_ids:
        raise ContractError(
            "evidence tasks do not match the complete selected catalogue: "
            f"expected {expected_task_ids}"
        )
    if value.get("manual_prerequisites") != expected_tier["manual-prerequisites"]:
        raise ContractError("evidence manual prerequisites do not match the selected tier")
    if gaps != catalogue["gaps"]:
        raise ContractError("evidence gaps do not match the reviewed catalogue")

    expected_actual_concurrency = (
        maximum_interval_overlap(tasks) if evidence_kind == "aggregate" else 1
    )
    expected_worker_wall = (
        round(sum(task["observed"]["wall_seconds"] for task in tasks), 3)
        if evidence_kind == "aggregate"
        else run["wall_seconds"]
    )
    expected_run_fields = {
        "runner": "scripts/migration-readiness.py",
        "catalogue": catalogue_label(catalogue_path),
        "maximum_concurrency": expected_tier["max-concurrency"],
        "actual_concurrency": expected_actual_concurrency,
        "tier_deadline_seconds": expected_tier["tier-deadline-seconds"],
        "total_worker_wall_seconds": expected_worker_wall,
        "fresh": True,
    }
    for name, expected in expected_run_fields.items():
        if run.get(name) != expected:
            raise ContractError(f"evidence run field {name} does not match the catalogue")
    try:
        uuid.UUID(run.get("id", ""))
    except (ValueError, AttributeError) as error:
        raise ContractError("evidence run id is not a UUID") from error
    lens_revisions = run.get("lens_revisions")
    if not isinstance(lens_revisions, dict) or set(lens_revisions) != {
        "compose-lens",
        "quadlet-lens",
    }:
        raise ContractError("evidence must bind both Lens revisions")
    if any(not SHA_RE.fullmatch(candidate) for candidate in lens_revisions.values()):
        raise ContractError("evidence Lens revisions must be full lowercase Git SHAs")
    if lens_revisions != catalogue_lens_revisions(catalogue):
        raise ContractError("evidence Lens revisions do not match catalogue pins")
    binary_digest = run.get("boxferry_binary_sha256")
    if binary_digest is not None and re.fullmatch(r"[0-9a-f]{64}", binary_digest) is None:
        raise ContractError("evidence BoxFerry binary digest must be lowercase SHA-256")
    if evidence_kind == "aggregate" and binary_digest is None:
        raise ContractError("aggregate evidence must bind the shared BoxFerry binary digest")

    for actual, expected in zip(tasks, expected_tasks, strict=True):
        expected_static = {
            "id": expected["id"],
            "workload": expected["workload"],
            "sources": expected["sources"],
            "targets": expected["targets"],
            "approved_losses": expected["approved-losses"],
            "runtime_claim": expected["runtime-claim"],
            "matrix_evidence": expected_matrix_evidence(expected["id"]),
            "privileged": expected["privileged"],
            "budgets": {
                "deadline_seconds": expected["deadline-seconds"],
                "minimum_memory_mib": expected["minimum-memory-mib"],
                "minimum_disk_mib": expected["minimum-disk-mib"],
                "maximum_rss_mib": expected["maximum-rss-mib"],
                "maximum_disk_growth_mib": expected["maximum-disk-growth-mib"],
                "maximum_concurrency": 1,
            },
        }
        for name, expected_value in expected_static.items():
            if actual.get(name) != expected_value:
                raise ContractError(
                    f"evidence task {actual.get('id')} field {name} does not match the catalogue"
                )
        if actual["state"] == "passed" and actual.get("reason") is not None:
            raise ContractError("successful evidence task must have a null reason")
        observed = actual.get("observed")
        if not isinstance(observed, dict) or observed.get("concurrency") != 1:
            raise ContractError("evidence task lacks sequential observed measurements")
        if actual["state"] == "passed":
            for name in (
                "wall_seconds",
                "peak_rss_kib",
                "disk_growth_mib",
                "exit_status",
                "timed_out",
            ):
                if name not in observed:
                    raise ContractError(
                        f"successful evidence task lacks observed measurement {name}"
                    )
            if observed["exit_status"] != 0 or observed["timed_out"] is not False:
                raise ContractError("successful evidence task has unsuccessful process observations")

    validate_evidence_semantics(run, tasks, expected_tasks)
    if evidence_kind == "aggregate" and tier == "pre-release":
        validate_complete_matrix_coverage(tasks)


def plan(args: argparse.Namespace) -> int:
    catalogue = load_catalogue(pathlib.Path(args.catalogue))
    tier, tasks = selected_tasks(catalogue, args.tier, args.task)
    value = {
        "tier": tier["id"],
        "max_concurrency": tier["max-concurrency"],
        "tier_deadline_seconds": tier["tier-deadline-seconds"],
        "manual_prerequisites": tier["manual-prerequisites"],
        "tasks": [
            {
                "id": task["id"],
                "privileged": task["privileged"],
                "workload": task["workload"],
                "required_tools": task["required-tools"],
                "required_environment": task.get("required-environment", []),
                "deadline_seconds": task["deadline-seconds"],
            }
            for task in tasks
        ],
        "gaps": catalogue["gaps"],
    }
    if args.format == "json":
        print(json.dumps(value, separators=(",", ":"), sort_keys=True))
    else:
        for task in value["tasks"]:
            print(task["id"])
    return 0


def run(args: argparse.Namespace) -> int:
    catalogue = load_catalogue(pathlib.Path(args.catalogue))
    tier, tasks = selected_tasks(catalogue, args.tier, args.task)
    if tier["id"] == "pre-release" and args.task is None:
        raise ContractError(
            "complete pre-release execution requires the parallel GitHub coordinator; "
            "select one --task for focused local reproduction"
        )
    revision = args.revision or git_revision()
    if not SHA_RE.fullmatch(revision):
        raise ContractError("--revision must be a full lowercase 40-character Git SHA")
    if revision != git_revision():
        raise ContractError(f"requested revision {revision} does not match checked-out HEAD {git_revision()}")
    revisions = catalogue_lens_revisions(catalogue)
    run_id = str(uuid.uuid4())
    started_at = now()
    run_started = time.monotonic()
    execution_deadline_seconds = (
        tasks[0]["deadline-seconds"] if args.task is not None else tier["tier-deadline-seconds"]
    )
    tier_deadline = run_started + execution_deadline_seconds
    total_steps = len(tasks) * 2 + 1
    step = 1
    results: list[dict[str, Any]] = []
    failed = False
    for task in tasks:
        if failed:
            skipped_at = now()
            skipped = time.monotonic()
            reason = "an earlier required task failed"
            emit(step, total_steps, "GAP", f"{task['id']}:preflight", reason)
            step += 1
            emit(step, total_steps, "GAP", f"{task['id']}:execute", "not run")
            step += 1
            results.append(
                task_evidence(
                    task,
                    skipped_at,
                    skipped,
                    "not-run",
                    {"concurrency": 1},
                    reason,
                )
            )
            continue
        result, step = run_task(
            task, revisions, step, total_steps, tier_deadline
        )
        results.append(result)
        failed = result["state"] != "passed"
    emit(step, total_steps, "START", "write-evidence")
    run_wall_seconds = time.monotonic() - run_started
    tier_timed_out = run_wall_seconds > execution_deadline_seconds
    failed = failed or tier_timed_out
    evidence = {
        "schema_version": 2,
        "outcome": "failed" if failed else "passed",
        "run": {
            "id": run_id,
            "tier": tier["id"],
            "revision": revision,
            "started_at": started_at,
            "finished_at": now(),
            "runner": "scripts/migration-readiness.py",
            "catalogue": catalogue_label(pathlib.Path(args.catalogue)),
            "maximum_concurrency": tier["max-concurrency"],
            "actual_concurrency": 1,
            "tier_deadline_seconds": tier["tier-deadline-seconds"],
            "wall_seconds": round(run_wall_seconds, 3),
            "total_worker_wall_seconds": round(run_wall_seconds, 3),
            "timed_out": tier_timed_out,
            "fresh": True,
            "selection": {
                "kind": "task" if args.task is not None else "tier",
                "task": args.task,
            },
            "coordinator_id": args.coordinator_id or run_id,
            "catalogue_sha256": catalogue_digest(pathlib.Path(args.catalogue)),
            "boxferry_binary_sha256": configured_boxferry_binary_digest(),
            "worker_id": args.worker_id or (args.task or "serial"),
            "evidence_kind": "worker" if args.task is not None else "serial",
            "lens_revisions": revisions,
        },
        "manual_prerequisites": tier["manual-prerequisites"],
        "tasks": results,
        "gaps": catalogue["gaps"],
    }
    validate_evidence(
        evidence,
        tier["id"],
        revision,
        task_id=args.task,
        catalogue_path=pathlib.Path(args.catalogue),
    )
    output = pathlib.Path(args.evidence)
    atomic_json(output, evidence)
    emit(step, total_steps, "PASS", "write-evidence", str(output))
    return 1 if failed else 0


def validate(args: argparse.Namespace) -> int:
    with pathlib.Path(args.evidence).open(encoding="utf-8") as stream:
        evidence = json.load(stream)
    validate_evidence(
        evidence,
        args.tier,
        args.revision,
        task_id=args.task,
        catalogue_path=pathlib.Path(args.catalogue),
    )
    if args.require_success and evidence["outcome"] != "passed":
        raise ContractError("evidence does not prove a successful tier")
    if args.require_aggregate and evidence["run"]["evidence_kind"] != "aggregate":
        raise ContractError("evidence is not complete aggregate evidence")
    print(
        f"validated migration-readiness evidence tier={evidence['run']['tier']} "
        f"revision={evidence['run']['revision']} outcome={evidence['outcome']}"
    )
    return 0


def collect(args: argparse.Namespace) -> int:
    """Fail closed while combining independently produced pre-release workers."""
    if args.tier != "pre-release":
        raise ContractError("only pre-release evidence may be collected from workers")
    catalogue_path = pathlib.Path(args.catalogue)
    catalogue = load_catalogue(catalogue_path)
    tier, expected = selected_tasks(catalogue, args.tier, None)
    revision = args.revision
    if not SHA_RE.fullmatch(revision):
        raise ContractError("--revision must be a full lowercase 40-character Git SHA")
    try:
        uuid.UUID(args.coordinator_id)
    except ValueError as error:
        raise ContractError("--coordinator-id must be a UUID") from error
    if re.fullmatch(r"[0-9a-f]{64}", args.boxferry_binary_sha256) is None:
        raise ContractError("--boxferry-binary-sha256 must be a lowercase SHA-256")
    documents: list[dict[str, Any]] = []
    seen_workers: set[str] = set()
    for raw_path in args.evidence:
        with pathlib.Path(raw_path).open(encoding="utf-8") as stream:
            document = json.load(stream)
        run = document.get("run", {})
        task_id = run.get("selection", {}).get("task")
        if run.get("evidence_kind") != "worker" or not isinstance(task_id, str):
            raise ContractError(f"{raw_path} is not a single worker evidence document")
        if run.get("coordinator_id") != args.coordinator_id:
            raise ContractError(f"{raw_path} belongs to a different coordinator")
        if run.get("worker_id") != task_id:
            raise ContractError(f"{raw_path} worker identity does not match its selected task")
        if task_id in seen_workers:
            raise ContractError(f"duplicate worker evidence for {task_id}")
        seen_workers.add(task_id)
        validate_evidence(document, args.tier, revision, task_id=task_id, catalogue_path=catalogue_path)
        if run.get("boxferry_binary_sha256") != args.boxferry_binary_sha256:
            raise ContractError(f"{raw_path} belongs to a different BoxFerry binary")
        if run.get("timed_out") or document["tasks"][0]["observed"]["timed_out"]:
            raise ContractError(f"{raw_path} contains timed-out worker evidence")
        if document.get("outcome") != "passed" or document["tasks"][0]["state"] != "passed":
            raise ContractError(f"{raw_path} does not contain successful worker evidence")
        documents.append(document)
    by_task = {document["tasks"][0]["id"]: document for document in documents}
    expected_ids = [task["id"] for task in expected]
    if set(by_task) != set(expected_ids) or len(by_task) != len(documents):
        raise ContractError("worker evidence must contain every selected task exactly once")
    ordered = [by_task[task_id] for task_id in expected_ids]
    tasks = [document["tasks"][0] for document in ordered]
    validate_complete_matrix_coverage(tasks)
    started_task = min(
        tasks,
        key=lambda task: parse_date_time(task["started_at"], f"$.tasks.{task['id']}.started_at"),
    )
    finished_task = max(
        tasks,
        key=lambda task: parse_date_time(task["finished_at"], f"$.tasks.{task['id']}.finished_at"),
    )
    started_at = started_task["started_at"]
    finished_at = finished_task["finished_at"]
    wall_seconds = round(
        (
            parse_date_time(finished_at, "aggregate.finished_at")
            - parse_date_time(started_at, "aggregate.started_at")
        ).total_seconds(),
        3,
    )
    if wall_seconds > tier["tier-deadline-seconds"]:
        raise ContractError("worker evidence exceeded the aggregate tier deadline")
    actual_concurrency = maximum_interval_overlap(tasks)
    if actual_concurrency > tier["max-concurrency"]:
        raise ContractError("worker evidence exceeded the aggregate concurrency limit")
    total_worker_wall_seconds = round(
        sum(task["observed"]["wall_seconds"] for task in tasks), 3
    )
    evidence = {
        "schema_version": 2,
        "outcome": "passed",
        "run": {
            **ordered[0]["run"],
            "id": args.coordinator_id,
            "tier": tier["id"],
            "revision": revision,
            "started_at": started_at,
            "finished_at": finished_at,
            "actual_concurrency": actual_concurrency,
            "wall_seconds": wall_seconds,
            "total_worker_wall_seconds": total_worker_wall_seconds,
            "timed_out": False,
            "selection": {"kind": "tier", "task": None},
            "worker_id": "aggregate",
            "evidence_kind": "aggregate",
            "boxferry_binary_sha256": args.boxferry_binary_sha256,
        },
        "manual_prerequisites": tier["manual-prerequisites"],
        "tasks": tasks,
        "gaps": catalogue["gaps"],
    }
    validate_evidence(evidence, args.tier, revision, catalogue_path=catalogue_path)
    atomic_json(pathlib.Path(args.evidence_output), evidence)
    return 0


def create_coordinator_id(_args: argparse.Namespace) -> int:
    """Print one lowercase coordinator UUID for workflow binding."""
    print(uuid.uuid4())
    return 0


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--catalogue", default=str(CATALOGUE))
    commands = result.add_subparsers(dest="operation", required=True)
    coordinator_parser = commands.add_parser("coordinator-id")
    coordinator_parser.set_defaults(function=create_coordinator_id)
    plan_parser = commands.add_parser("plan")
    plan_parser.add_argument("--tier", required=True, choices=("offline", "trusted-live", "pre-release"))
    plan_parser.add_argument("--task")
    plan_parser.add_argument("--format", choices=("lines", "json"), default="lines")
    plan_parser.set_defaults(function=plan)
    run_parser = commands.add_parser("run")
    run_parser.add_argument("--tier", required=True, choices=("offline", "trusted-live", "pre-release"))
    run_parser.add_argument("--task")
    run_parser.add_argument("--revision")
    run_parser.add_argument("--evidence", default=str(DEFAULT_EVIDENCE))
    run_parser.add_argument("--coordinator-id")
    run_parser.add_argument("--worker-id")
    run_parser.set_defaults(function=run)
    validate_parser = commands.add_parser("validate-evidence")
    validate_parser.add_argument("--evidence", required=True)
    validate_parser.add_argument("--tier", choices=("offline", "trusted-live", "pre-release"))
    validate_parser.add_argument("--task")
    validate_parser.add_argument("--revision")
    validate_parser.add_argument("--require-success", action="store_true")
    validate_parser.add_argument("--require-aggregate", action="store_true")
    validate_parser.set_defaults(function=validate)
    collect_parser = commands.add_parser("collect-evidence")
    collect_parser.add_argument("--tier", required=True, choices=("offline", "trusted-live", "pre-release"))
    collect_parser.add_argument("--revision", required=True)
    collect_parser.add_argument("--coordinator-id", required=True)
    collect_parser.add_argument("--boxferry-binary-sha256", required=True)
    collect_parser.add_argument("--evidence", action="append", required=True)
    collect_parser.add_argument("--evidence-output", required=True)
    collect_parser.set_defaults(function=collect)
    return result


def main() -> int:
    args = parser().parse_args()
    try:
        return args.function(args)
    except (ContractError, OSError, subprocess.SubprocessError, tomllib.TOMLDecodeError, json.JSONDecodeError) as error:
        print(f"migration-readiness: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
