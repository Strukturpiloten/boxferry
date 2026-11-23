"""Select the PodmanLens native prerequisite independently of ordinary profiles."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path

from boxferry_ci.contracts import ContractError, object_value
from boxferry_ci.operational import PODMAN_LENS_REPOSITORY, PODMAN_NATIVE_PINS, parse_fedora_base
from boxferry_ci.planner import Comparison, git, git_text, regular_contents
from boxferry_ci.policy import Policy

NATIVE_JOB = "native-conformance"
POLICY_PATH = "scripts/validation-policy.json"
# PodmanLens main 20914f2cce74066fd1ff9474c12b56c3fe874244, before policy opt-in.
BOOTSTRAP_POLICY_SHA256 = "a6107b006e7f617dd423ce2d0936a76e671c62ae9e1cb6a7f6aaa628c9563833"


def _fedora_digest(root: Path, revision: str) -> str:
    contents = regular_contents(root, revision, PODMAN_NATIVE_PINS)
    if contents is None:
        raise ContractError(f"missing owned Fedora image declaration at {revision}")
    return parse_fedora_base(contents.decode("utf-8")).digest


def _policy_active(contents: bytes | None) -> bool:
    if contents is None:
        return False
    config = object_value(json.loads(contents), "PodmanLens validation policy")
    enabled = config.get("podman_native_conformance", False)
    if type(enabled) is not bool:
        raise ContractError("podman_native_conformance must be boolean")
    return enabled


def select_native_conformance(
    root: Path,
    policy: Policy,
    comparison: Comparison,
    *,
    bootstrap: bool,
) -> bool | None:
    """Select from the verified base policy and the exact hosted comparison."""
    event, base, head, tested = (
        comparison.event,
        comparison.base_sha,
        comparison.head_sha,
        comparison.tested_sha,
    )
    hosted = event != "local"
    if os.environ.get("GITHUB_REPOSITORY") == PODMAN_LENS_REPOSITORY and (
        policy.repository != PODMAN_LENS_REPOSITORY
    ):
        raise ContractError("validation policy repository does not match PodmanLens")
    active = bootstrap or policy.podman_native_conformance
    if hosted and policy.repository == PODMAN_LENS_REPOSITORY:
        # Legacy callers may name another trusted config. Once the canonical
        # policy activates, that override cannot hide its prerequisite.
        canonical = regular_contents(root, base, POLICY_PATH)
        if canonical != policy.path.read_bytes() and (active or _policy_active(canonical)):
            raise ContractError(
                "native conformance requires the canonical trusted PodmanLens policy"
            )
    if not active:
        return None
    if policy.repository != PODMAN_LENS_REPOSITORY or NATIVE_JOB in policy.jobs:
        raise ContractError("native conformance is a supplemental PodmanLens prerequisite")
    if not policy.podman_native_conformance and (
        hashlib.sha256(policy.path.read_bytes()).hexdigest() != BOOTSTRAP_POLICY_SHA256
    ):
        raise ContractError("unapproved native conformance bootstrap policy")
    if hosted and git_text(root, "rev-parse", "HEAD") != tested:
        raise ContractError("native conformance checkout does not match the tested SHA")
    if event not in {"pull_request", "merge_group"}:
        return False
    if event == "pull_request":
        parents = git_text(root, "rev-list", "--parents", "-n", "1", tested).split()
        if parents != [tested, base, head]:
            raise ContractError("native PR comparison requires the exact base and head parents")
    elif (
        head != tested
        or base == tested
        or (git(root, "merge-base", "--is-ancestor", base, tested, optional=True) is None)
    ):
        raise ContractError("native group comparison requires the tested group on its exact base")
    if policy.podman_native_conformance:
        if not _policy_active(regular_contents(root, tested, POLICY_PATH)):
            raise ContractError("candidate cannot disable the active native prerequisite")
    return _fedora_digest(root, base) != _fedora_digest(root, tested)
