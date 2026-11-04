"""Fail-closed aggregate validation of exact-candidate job outcomes."""

from __future__ import annotations

import re

from boxferry_ci.contracts import ContractError, exact_sha, object_value, strings
from boxferry_ci.devcontainer_seed import DEVCONTAINER_JOB, require_devcontainer_policy
from boxferry_ci.planner import PLAN_SCHEMA, Comparison
from boxferry_ci.policy import Policy


class GateFailure(ContractError):
    """The contract is valid, but one or more required checks did not pass."""


def verify_gate(
    plan_value: object,
    needs_value: object,
    policy: Policy,
    comparison: Comparison,
    *,
    expected_plan: dict[str, object] | None = None,
) -> list[tuple[str, str]]:
    plan = object_value(plan_value, "validation plan")
    keys = {
        "schema",
        "repository",
        "event",
        "base_sha",
        "head_sha",
        "tested_sha",
        "profile",
        "profiles",
        "jobs",
        "reasons",
        "local_fingerprint",
    }
    if set(plan) != keys or type(plan["schema"]) is not int or plan["schema"] != PLAN_SCHEMA:
        raise ContractError("missing or unsupported validation plan; regenerate with schema 3")
    if plan["repository"] != policy.repository or plan["event"] != comparison.event:
        raise ContractError("validation plan repository/event mismatch")
    for key, expected in (
        ("base_sha", comparison.base_sha),
        ("head_sha", comparison.head_sha),
        ("tested_sha", comparison.tested_sha),
    ):
        if plan[key] != exact_sha(expected, key):
            raise ContractError(f"validation plan is not bound to the exact {key}")
    profiles = strings(plan["profiles"], "plan profiles")
    if tuple(sorted(profiles)) != profiles or ("full" in profiles and profiles != ("full",)):
        raise ContractError("plan profiles must be canonical; full cannot be mixed")
    if plan["profile"] != (profiles[0] if len(profiles) == 1 else "mixed"):
        raise ContractError("plan profile does not match its constituent profiles")
    if comparison.event not in {"pull_request", "merge_group", "local"}:
        if profiles != ("full",) or comparison.base_sha != comparison.tested_sha:
            raise ContractError("non-PR/queue validation must be complete at the tested revision")
        if comparison.head_sha != comparison.tested_sha:
            raise ContractError("non-PR/queue head must equal the tested revision")
    strings(plan["reasons"], "plan reasons")
    fingerprint = plan["local_fingerprint"]
    if fingerprint is not None and (
        comparison.event != "local"
        or not isinstance(fingerprint, str)
        or re.fullmatch(r"[0-9a-f]{64}", fingerprint) is None
    ):
        raise ContractError("invalid local fingerprint")
    jobs = object_value(plan["jobs"], "plan jobs")
    if set(jobs) != set(policy.jobs) or any(
        type(selected) is not bool for selected in jobs.values()
    ):
        raise ContractError("plan has unknown, missing, or non-boolean job selections")
    selected = policy.selected_jobs(profiles)
    if {job for job, enabled in jobs.items() if enabled} != selected:
        raise ContractError("plan jobs do not equal the trusted union of constituent profiles")
    if expected_plan is not None and any(
        plan[key] != expected_plan[key] for key in ("profiles", "profile", "jobs")
    ):
        raise ContractError("plan selections differ from an independent exact-candidate comparison")
    needs = object_value(needs_value, "aggregate job dependencies")
    if set(needs) != {"validation-plan", *policy.jobs}:
        raise ContractError("aggregate job dependencies do not match policy")
    results: list[tuple[str, str]] = []
    failures: list[str] = []
    for job in ("validation-plan", *policy.jobs):
        outcome = object_value(needs[job], f"{job} outcome")
        result = outcome.get("result")
        if not isinstance(result, str) or result not in {
            "success",
            "failure",
            "cancelled",
            "skipped",
        }:
            raise ContractError(f"{job} has a missing or unknown result")
        required = job == "validation-plan" or jobs[job] is True
        if required and result != "success":
            failures.append(f"selected {job} concluded {result}")
        elif not required and result not in {"success", "skipped"}:
            failures.append(f"unselected {job} concluded {result}")
        results.append((job, result))
    if failures:
        raise GateFailure("; ".join(failures))
    return results


def _devcontainer_receipt(outcome: dict[str, object], tested_sha: str) -> None:
    result = outcome.get("result")
    if not isinstance(result, str) or result not in {"success", "failure", "cancelled", "skipped"}:
        raise ContractError(f"{DEVCONTAINER_JOB} has a missing or unknown result")
    if result != "success":
        raise GateFailure(f"selected {DEVCONTAINER_JOB} concluded {result}")
    outputs = object_value(outcome.get("outputs"), f"{DEVCONTAINER_JOB} outputs")
    candidate = exact_sha(outputs.get("candidate_sha"), "Dev Container candidate receipt")
    if candidate != tested_sha:
        raise ContractError("Dev Container receipt does not identify the exact tested candidate")


def verify_devcontainer_gate(
    plan_value: object,
    needs_value: object,
    policy: Policy,
    comparison: Comparison,
    *,
    expected_plan: dict[str, object],
) -> list[tuple[str, str]]:
    """Keep the trusted gate intact and bind its image prerequisite to this candidate.

    An approved pre-seed policy permits precisely one supplemental dependency;
    it never changes the plan or any original job selection or outcome rule.
    """
    seed = DEVCONTAINER_JOB not in policy.jobs
    require_devcontainer_policy(policy, seed=seed)
    needs = object_value(needs_value, "aggregate job dependencies")
    original = dict(needs)
    if seed:
        image = original.pop(DEVCONTAINER_JOB, None)
    else:
        image = original.get(DEVCONTAINER_JOB)
    results = verify_gate(plan_value, original, policy, comparison, expected_plan=expected_plan)
    outcome = object_value(image, f"{DEVCONTAINER_JOB} outcome")
    if seed or outcome.get("result") == "success":
        _devcontainer_receipt(outcome, comparison.tested_sha)
    if seed:
        results.append((DEVCONTAINER_JOB, "success"))
    return results
