#!/usr/bin/env python3
"""Admit reviewed PR catalogue metadata using trusted migration-readiness code.

Run this script from the trusted dispatcher checkout, never from the candidate.
The verifier tree contains only trusted validation code and the admitted candidate
catalogue, so its evidence digest and metadata agree with the executed candidate.
"""

from __future__ import annotations

import argparse
import copy
import importlib.util
import os
import pathlib
import re
import stat
import sys
import tempfile
import tomllib


CATALOGUE = pathlib.Path("fixtures/conformance/migration-readiness/tiers.toml")
RUNNER = pathlib.Path("scripts/migration-readiness.py")
SCHEMA = pathlib.Path("docs/schemas/migration-readiness-evidence-v2.schema.json")
APPLICATION_TASKS = {
    "nextcloud-application",
    "paperless-application",
    "immich-application",
}
LENS_TASKS = {"compose-lens-candidate", "quadlet-lens-candidate"}
FULL_SHA = re.compile(r"[0-9a-f]{40}\Z")
MAX_CONTRACT_BYTES = 131072


class AdmissionError(RuntimeError):
    """Candidate execution policy differs from the trusted dispatcher."""


def regular_file(path: pathlib.Path) -> bytes:
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    except OSError as error:
        raise AdmissionError(f"{path} must be an accessible, regular, non-symlink file") from error
    with os.fdopen(descriptor, "rb") as stream:
        metadata = os.fstat(stream.fileno())
        if not stat.S_ISREG(metadata.st_mode):
            raise AdmissionError(f"{path} must be a regular, non-symlink file")
        if metadata.st_size > MAX_CONTRACT_BYTES:
            raise AdmissionError(f"{path} exceeds the admission size limit")
        data = stream.read(MAX_CONTRACT_BYTES + 1)
    if len(data) > MAX_CONTRACT_BYTES:
        raise AdmissionError(f"{path} exceeds the admission size limit")
    return data


def trusted_runner(root: pathlib.Path):
    runner_path = root / RUNNER
    regular_file(runner_path)
    spec = importlib.util.spec_from_file_location("trusted_migration_readiness", runner_path)
    if spec is None or spec.loader is None:
        raise AdmissionError("could not load trusted migration-readiness runner")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def admit(
    trusted_root: pathlib.Path,
    candidate_root: pathlib.Path,
    verifier_root: pathlib.Path,
    task_id: str,
) -> None:
    if task_id not in APPLICATION_TASKS:
        raise AdmissionError("task is outside the reviewed application allowlist")

    # The candidate cannot change either parser/runner or the evidence contract.
    for label in (RUNNER, SCHEMA):
        if regular_file(trusted_root / label) != regular_file(candidate_root / label):
            raise AdmissionError(f"candidate changed trusted execution boundary {label}")

    runner = trusted_runner(trusted_root)
    trusted_path = trusted_root / CATALOGUE
    candidate_path = candidate_root / CATALOGUE
    trusted_bytes = regular_file(trusted_path)
    candidate_bytes = regular_file(candidate_path)
    # Shape and closed-ID validation are always performed by code from trusted main.
    try:
        with tempfile.TemporaryDirectory(prefix="reviewed-catalogue-") as temporary:
            trusted_copy = pathlib.Path(temporary) / "trusted.toml"
            candidate_copy = pathlib.Path(temporary) / "candidate.toml"
            trusted_copy.write_bytes(trusted_bytes)
            candidate_copy.write_bytes(candidate_bytes)
            trusted = runner.load_catalogue(trusted_copy)
            candidate = runner.load_catalogue(candidate_copy)
    except runner.ContractError as error:
        raise AdmissionError(f"catalogue shape is invalid: {error}") from error
    if task_id not in runner.by_id(candidate["tiers"], "pre-release", "tier")["tasks"]:
        raise AdmissionError("selected task is outside the pre-release tier")

    normalized = copy.deepcopy(candidate)
    trusted_tasks = {task["id"]: task for task in trusted["tasks"]}
    for task in normalized["tasks"]:
        original = trusted_tasks[task["id"]]
        # These fields describe evidence. They do not select a command or alter a budget.
        task["sources"] = original["sources"]
        task["targets"] = original["targets"]
        if task["id"] in LENS_TASKS:
            if not FULL_SHA.fullmatch(task["revision"]):
                raise AdmissionError(f"{task['id']} revision must be a full lowercase Git SHA")
            task["revision"] = original["revision"]
    if normalized != trusted:
        raise AdmissionError("candidate changed catalogue execution policy or protected metadata")

    # Preserve exactly the candidate bytes that the live runner hashes. The trusted
    # verifier resolves both the schema and catalogue relative to its own tree.
    for label, contents in (
        (RUNNER, regular_file(trusted_root / RUNNER)),
        (SCHEMA, regular_file(trusted_root / SCHEMA)),
        (CATALOGUE, candidate_bytes),
    ):
        destination = verifier_root / label
        destination.parent.mkdir(parents=True, exist_ok=True)
        if destination.exists() or destination.is_symlink():
            raise AdmissionError(f"verifier destination already exists: {destination}")
        destination.write_bytes(contents)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--trusted-root", required=True, type=pathlib.Path)
    parser.add_argument("--candidate-root", required=True, type=pathlib.Path)
    parser.add_argument("--verifier-root", required=True, type=pathlib.Path)
    parser.add_argument("--task", required=True)
    args = parser.parse_args()
    try:
        admit(args.trusted_root, args.candidate_root, args.verifier_root, args.task)
    except (AdmissionError, OSError, tomllib.TOMLDecodeError, ValueError) as error:
        print(f"reviewed-application-catalogue: {error}", file=sys.stderr)
        return 2
    print(f"admitted reviewed application catalogue for {args.task}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
