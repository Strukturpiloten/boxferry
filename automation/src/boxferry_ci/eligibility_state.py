"""Validate decoded eligibility artifacts against verified run provenance."""

from __future__ import annotations

import re
from datetime import UTC, datetime
from typing import Any

WORKFLOW_PATH = ".github/workflows/dependency-eligibility.yml"
SHA = re.compile(r"[0-9a-f]{40}\Z")


class EligibilityError(ValueError):
    """Unavailable or inconsistent authoritative eligibility evidence."""


def revision(value: object) -> str:
    """Require a complete immutable Git object identifier."""
    if not isinstance(value, str) or not SHA.fullmatch(value):
        raise EligibilityError("authoritative metadata has an invalid commit revision")
    return value


def utc(value: object) -> datetime:
    """Parse timezone-bearing persisted evidence, never a local wall-clock date."""
    if not isinstance(value, str):
        raise EligibilityError("persisted time must be an RFC3339 string")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise EligibilityError("persisted time is malformed") from error
    if parsed.tzinfo is None:
        raise EligibilityError("persisted time has no timezone")
    return parsed.astimezone(UTC)


def validate_state(
    value: dict[str, Any],
    *,
    repository: str,
    run_id: int,
    implementation: str,
    now: datetime,
) -> dict[str, Any]:
    """Validate persisted provenance and all first-seen times before their reuse."""
    if (
        type(value.get("schema")) is not int
        or value.get("schema") != 1
        or value.get("repository") != repository
        or value.get("run_id") != run_id
        or value.get("implementation_revision") != implementation
        or value.get("workflow_path") != WORKFLOW_PATH
    ):
        raise EligibilityError("trusted artifact provenance does not match the verified run")
    if "run_attempt" in value and (
        type(value["run_attempt"]) is not int or value["run_attempt"] < 1
    ):
        raise EligibilityError("trusted service artifact has an invalid run attempt")
    observed = utc(value.get("observed_at"))
    if observed > now:
        raise EligibilityError("trusted artifact observation is in the future")
    dependency = value.get("dependency_state")
    if (
        not isinstance(dependency, dict)
        or type(dependency.get("schema_version")) is not int
        or dependency.get("schema_version") != 1
    ):
        raise EligibilityError("trusted dependency state schema is invalid")
    first_seen = dependency.get("first_seen")
    if not isinstance(first_seen, dict) or len(first_seen) > 4096:
        raise EligibilityError("trusted first-seen state exceeds its bounds")
    for key, instant in first_seen.items():
        if not isinstance(key, str) or not re.fullmatch(r"[0-9a-f]{64}", key):
            raise EligibilityError("trusted first-seen key is not an immutable artifact identity")
        if utc(instant) > observed:
            raise EligibilityError("trusted first-seen time is later than its observation")
    if not isinstance(value.get("reports"), dict) or len(value["reports"]) > 100:
        raise EligibilityError("trusted candidate reports are malformed")
    for metadata_key in ("deferred_candidates", "no_action_candidates"):
        items = value.get(metadata_key, [])
        if not isinstance(items, list) or len(items) > 1:
            raise EligibilityError("trusted noncandidate metadata exceeds its bound")
        for item in items:
            if (
                not isinstance(item, dict)
                or type(item.get("pull_request")) is not int
                or item["pull_request"] < 1
                or type(item.get("source_run_id")) is not int
                or item["source_run_id"] < 1
                or not isinstance(item.get("reason"), str)
                or not isinstance(item.get("next_action"), str)
            ):
                raise EligibilityError("trusted noncandidate metadata is malformed")
            revision(item.get("source_head_sha"))
            if "queue_movement" in item:
                movement = item["queue_movement"]
                if (
                    metadata_key != "deferred_candidates"
                    or not isinstance(movement, dict)
                    or set(movement)
                    != {
                        "kind",
                        "stage",
                        "target_branch",
                        "before_sha",
                        "after_sha",
                        "tested_sha",
                        "source_run_attempt",
                    }
                    or movement.get("kind") != "queue-target-moved"
                    or movement.get("stage") != "queue-current"
                    or not isinstance(movement.get("target_branch"), str)
                    or not 1 <= len(movement["target_branch"]) <= 512
                    or movement["target_branch"].startswith("/")
                    or any(char in movement["target_branch"] for char in "\r\n\0")
                    or type(movement.get("source_run_attempt")) is not int
                    or movement["source_run_attempt"] < 1
                ):
                    raise EligibilityError("trusted queue movement metadata is malformed")
                for key in ("before_sha", "after_sha", "tested_sha"):
                    revision(movement[key])
                if movement["before_sha"] == movement["after_sha"]:
                    raise EligibilityError("trusted queue movement has no changed target")
    cursor = value.get("scan_cursor", 0)
    if type(cursor) is not int or cursor < 0:
        raise EligibilityError("trusted scheduled scan cursor is malformed")
    reruns = value.get("reruns")
    if not isinstance(reruns, dict) or len(reruns) > 4096:
        raise EligibilityError("trusted rerun ledger is malformed")
    for key, run in reruns.items():
        revision(key)
        if type(run) is not int or run < 1:
            raise EligibilityError("trusted rerun ledger has an invalid run identifier")
    frontier = value.get("recovery_frontier")
    if frontier is not None:
        if (
            not isinstance(frontier, dict)
            or type(frontier.get("protocol")) is not int
            or frontier["protocol"] != 1
            or type(frontier.get("authority")) is not bool
            or not (frontier.get("fence") is None or isinstance(frontier["fence"], str))
            or len(frontier.get("fence") or "") > 2000
            or not isinstance(frontier.get("pending", []), list)
            or len(frontier.get("pending", [])) > 100
        ):
            raise EligibilityError("trusted recovery frontier is malformed")
        utc(frontier.get("service_created_at"))
        if frontier.get("floor") is not None:
            utc(frontier["floor"])
        elif frontier["authority"]:
            raise EligibilityError("trusted recovery frontier has no immutable effect floor")
        predecessor = frontier.get("predecessor_run_id")
        if predecessor is not None and (type(predecessor) is not int or predecessor < 1):
            raise EligibilityError("trusted recovery frontier predecessor is malformed")
        for pending in frontier.get("pending", []):
            if not isinstance(pending, dict) or any(
                type(pending.get(key)) is not int or pending[key] < 1
                for key in ("run_id", "run_attempt")
            ):
                raise EligibilityError("trusted recovery frontier attempt fence is malformed")
    recoveries = value.get("recoveries", {})
    if not isinstance(recoveries, dict) or len(recoveries) > 4096:
        raise EligibilityError("trusted recovery reservation ledger is malformed")
    for key, entry in recoveries.items():
        if (
            not isinstance(key, str)
            or re.fullmatch(r"[0-9a-f]{64}", key) is None
            or not isinstance(entry, dict)
            or entry.get("phase") not in {"prepared", "attempted", "uncertain", "suspended"}
            or entry.get("kind")
            not in {"retained-graph", "handoff", "native-legacy-handoff", "retained-original"}
            or type(entry.get("original_run_id")) is not int
            or entry["original_run_id"] < 1
            or type(entry.get("original_attempt")) is not int
            or entry["original_attempt"] < 1
            or type(entry.get("prepared_service_run")) is not int
            or entry["prepared_service_run"] < 1
        ):
            raise EligibilityError("trusted recovery reservation has invalid bindings")
        for binding_name in ("source_head_sha", "authority"):
            revision(entry.get(binding_name))
        if entry["kind"] == "retained-original" and (
            not isinstance(entry.get("original_identity_sha256"), str)
            or re.fullmatch(r"[0-9a-f]{64}", entry["original_identity_sha256"]) is None
        ):
            raise EligibilityError("retained-original reservation lacks exact PR identity")
        if entry["kind"] == "native-legacy-handoff":
            revision(entry.get("merge_base_sha"))
            if (
                not isinstance(entry.get("transition_fingerprint"), str)
                or re.fullmatch(r"[0-9a-f]{64}", entry["transition_fingerprint"]) is None
                or "hinted_candidate_sha" in entry
                or "hinted_base_sha" in entry
            ):
                raise EligibilityError("native handoff reservation has invalid current-only proof")
        else:
            for binding_name in ("hinted_candidate_sha", "hinted_base_sha"):
                revision(entry.get(binding_name))
        if utc(entry.get("prepared_at")) > observed:
            raise EligibilityError("trusted recovery reservation is from the future")
    return value
