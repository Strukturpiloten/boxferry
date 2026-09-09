#!/usr/bin/env python3
"""Resolve Podman limitation candidates and manage privacy-safe evidence.

This helper deliberately uses only the Python standard library.  It validates
candidate provenance against the accepted matrix and limitation ledger before
the privileged shell runner receives an image reference.  Evidence changes
state only through the commands below; arbitrary JSON fragments are never
evaluated or merged.
"""

from __future__ import annotations

import argparse
import copy
import datetime as dt
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import sys
import tempfile
import tomllib
from typing import Any, NoReturn


SCHEMA_VERSION = 1
EVIDENCE_KIND = "podman-limitation-revalidation"
INITIALIZATION_EVIDENCE_KIND = "podman-limitation-revalidation-initialization-failure"
INITIALIZATION_FAILURE_KIND = "podman-limitation-revalidation-initialization-failure"
SOURCE_REPOSITORY = "https://github.com/Strukturpiloten/containers"
SOURCE_LICENSE = "AGPL-3.0-only"
REDISTRIBUTION = "transient-test-pull"
EXPECTED_LIMITATION = "helper-privilege-collision"
EXPECTED_ROLES = {"image-definition", "platform-recipe", "runtime-config"}
EXPECTED_CANDIDATE_KEYS = {
    "id",
    "baseline-image",
    "candidate-image",
    "expected-podman-version",
    "expected-distribution",
    "expected-baseline-observed-distribution",
    "expected-replacement-observed-distribution",
    "expected-mode",
    "expected-lane",
    "expected-architecture",
    "expected-limitation",
    "published-at",
    "source-repository",
    "source-revision",
    "source-license",
    "redistribution",
    "source-files",
}
EXPECTED_SOURCE_FILE_KEYS = {"role", "path", "sha256"}
IMAGE_RE = re.compile(
    r"^ghcr\.io/strukturpiloten/(?P<name>[a-z0-9.-]+):"
    r"(?P<tag>v[0-9]+\.[0-9]+\.[0-9]+)@sha256:(?P<digest>[0-9a-f]{64})$"
)
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
GIT_SHA_RE = re.compile(r"^[0-9a-f]{40}$")
CANDIDATE_PATTERN = (
    r"^podman-(opensuse-(leap-16\.0|tumbleweed)|ubi-(8|9|10))-rootless$"
)
CANDIDATE_RE = re.compile(CANDIDATE_PATTERN)
TIMESTAMP_RE = re.compile(
    r"^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"
)
PODMAN_VERSION_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")
API_VERSION_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")
PACKAGE_REVISION_RE = re.compile(r"^[0-9A-Za-z][0-9A-Za-z.+:~_-]{0,159}$")
OBSERVED_DISTRIBUTION_RE = re.compile(
    r"^(?:opensuse-leap-16\.0|opensuse-tumbleweed-[0-9]{8}|ubi-(?:8\.10|9\.8|10\.2))$"
)
MAX_JSON_BYTES = 128 * 1024
BIDI_CONTROLS = {
    "\u061c",
    "\u200e",
    "\u200f",
    "\u202a",
    "\u202b",
    "\u202c",
    "\u202d",
    "\u202e",
    "\u2066",
    "\u2067",
    "\u2068",
    "\u2069",
}
FAILURE_PHASES = {
    "preflight",
    "catalogue",
    "baseline-pull",
    "baseline-metadata",
    "baseline-collision",
    "baseline-cleanup",
    "replacement-pull",
    "replacement-provenance",
    "replacement-runtime",
    "resource-suite",
    "external-apply",
    "cleanup",
    "evidence",
}
FAILURE_CODES = {
    "invalid-invocation",
    "prerequisite-unavailable",
    "invalid-catalogue",
    "pull-failed",
    "digest-mismatch",
    "baseline-metadata-mismatch",
    "historical-collision-not-reproduced",
    "cleanup-failed",
    "source-proof-mismatch",
    "runtime-metadata-mismatch",
    "resource-contract-failed",
    "external-apply-failed",
    "evidence-invalid",
    "interrupted",
}
FAILURE_CODES_BY_PHASE = {
    "preflight": {"invalid-invocation", "prerequisite-unavailable", "interrupted"},
    "catalogue": {"invalid-catalogue", "interrupted"},
    "baseline-pull": {"pull-failed", "digest-mismatch", "interrupted"},
    "baseline-metadata": {"baseline-metadata-mismatch", "interrupted"},
    "baseline-collision": {"historical-collision-not-reproduced", "interrupted"},
    "baseline-cleanup": {"cleanup-failed", "interrupted"},
    "replacement-pull": {"pull-failed", "digest-mismatch", "interrupted"},
    "replacement-provenance": {"source-proof-mismatch", "interrupted"},
    "replacement-runtime": {"runtime-metadata-mismatch", "interrupted"},
    "resource-suite": {"resource-contract-failed", "interrupted"},
    "external-apply": {"external-apply-failed", "interrupted"},
    "cleanup": {"cleanup-failed", "interrupted"},
    "evidence": {"evidence-invalid", "interrupted"},
}
INITIALIZATION_FAILURE_CODES_BY_PHASE = {
    phase: FAILURE_CODES_BY_PHASE[phase] for phase in ("preflight", "catalogue", "evidence")
}
SELECTORS = ("exact", "prefix", "label", "all", "network")
EXPORTERS = ("compose", "quadlet", "podman")
RUNTIME_CHECKS = (
    "resource_creation",
    "runtime_semantics",
    "deterministic_export",
    "strict_policy",
    "literal_glob_rejection",
    "support_bundle",
    "malformed_response",
    "disappeared_resource",
    "partial_inventory",
    "selinux_intent",
)
PRIVACY_CHECKS = (
    "redaction_passed",
    "raw_outputs_absent",
    "environment_values_absent",
    "host_paths_absent",
    "runtime_identifiers_absent",
)
HISTORICAL_CHECKS = (
    "podman_info_failed",
    "newuidmap_reported",
    "permission_denied_reported",
    "newuidmap_setuid",
    "newgidmap_setuid",
    "newuidmap_capability",
    "newgidmap_capability",
)
FRESH_STORE_CHECKS = (
    "distinct_runtime",
    "distinct_socket",
    "distinct_graph_root",
    "distinct_name_prefix",
)
REIMPORT_CHECKS = ("compose", "quadlet")
EXTERNAL_APPLY_CHECKS = ("performed", "plan_applied", "reacquired")
CLEANUP_CHECKS = ("baseline_removed", "replacement_removed", "apply_target_removed")
EXPECTED_RESULTS = {
    *(f"baseline.historical_collision.{name}" for name in HISTORICAL_CHECKS),
    *(f"fresh_store.{name}" for name in FRESH_STORE_CHECKS),
    *(f"runtime_results.{name}" for name in RUNTIME_CHECKS),
    *(
        f"selector_exporters.{selector}.{exporter}"
        for selector in SELECTORS
        for exporter in EXPORTERS
    ),
    *(f"diagnostic_privacy.{name}" for name in PRIVACY_CHECKS),
    *(f"reimports.{name}" for name in REIMPORT_CHECKS),
    *(f"external_apply.{name}" for name in EXTERNAL_APPLY_CHECKS),
    *(f"cleanup.{name}" for name in CLEANUP_CHECKS),
}
OBSERVATION_FIELDS = {
    "baseline.observed_digest",
    "baseline.observed.podman_version",
    "baseline.observed.package_revision",
    "baseline.observed.distribution",
    "baseline.observed.architecture",
    "baseline.observed.uid",
    "baseline.observed.rootless",
    "replacement.observed_digest",
    "replacement.observed.podman_version",
    "replacement.observed.api_version",
    "replacement.observed.package_revision",
    "replacement.observed.distribution",
    "replacement.observed.architecture",
    "replacement.observed.rootless",
    "replacement.observed.source_revision",
}


class ContractError(ValueError):
    """A bounded catalogue or evidence contract was violated."""


def fail(message: str) -> NoReturn:
    print(f"podman-revalidation: {message}", file=sys.stderr)
    raise SystemExit(2)


def exact_keys(value: dict[str, Any], expected: set[str], context: str) -> None:
    actual = set(value)
    if actual != expected:
        unknown = sorted(actual - expected)
        missing = sorted(expected - actual)
        raise ContractError(
            f"{context} keys differ: unknown={unknown!r}, missing={missing!r}"
        )


def require_string(
    value: Any, context: str, *, pattern: re.Pattern[str] | None = None, limit: int = 240
) -> str:
    if not isinstance(value, str) or not value or len(value.encode("utf-8")) > limit:
        raise ContractError(f"{context} must be a non-empty string of at most {limit} bytes")
    if any(ord(character) < 0x20 or character in BIDI_CONTROLS for character in value):
        raise ContractError(f"{context} contains control or bidirectional formatting characters")
    if pattern is not None and pattern.fullmatch(value) is None:
        raise ContractError(f"{context} has an invalid value")
    return value


def require_bool(value: Any, context: str) -> bool:
    if not isinstance(value, bool):
        raise ContractError(f"{context} must be a boolean")
    return value


def require_optional_string(
    value: Any, context: str, *, pattern: re.Pattern[str] | None = None, limit: int = 240
) -> str | None:
    if value is None:
        return None
    return require_string(value, context, pattern=pattern, limit=limit)


def require_timestamp(value: Any, context: str) -> str:
    timestamp = require_string(value, context, pattern=TIMESTAMP_RE, limit=20)
    try:
        dt.datetime.strptime(timestamp, "%Y-%m-%dT%H:%M:%SZ")
    except ValueError as error:
        raise ContractError(f"{context} is not a real UTC timestamp") from error
    return timestamp


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def catalogue_hashes(
    catalogue_path: Path, matrix_path: Path, limitations_path: Path
) -> dict[str, str]:
    return {
        "candidates_sha256": sha256_file(catalogue_path),
        "matrix_sha256": sha256_file(matrix_path),
        "limitations_sha256": sha256_file(limitations_path),
    }


def available_catalogue_hashes(
    catalogue_path: Path | None,
    matrix_path: Path | None,
    limitations_path: Path | None,
) -> dict[str, str] | None:
    paths = (catalogue_path, matrix_path, limitations_path)
    if any(path is None for path in paths):
        return None
    assert catalogue_path is not None
    assert matrix_path is not None
    assert limitations_path is not None
    try:
        return catalogue_hashes(catalogue_path, matrix_path, limitations_path)
    except OSError:
        return None


def read_tsv(path: Path, columns: int, context: str) -> list[list[str]]:
    rows: list[list[str]] = []
    with path.open("r", encoding="utf-8") as handle:
        for number, raw_line in enumerate(handle, 1):
            line = raw_line.rstrip("\n")
            if not line or line.startswith("#"):
                continue
            fields = line.split("\t")
            if len(fields) != columns or any(not field for field in fields):
                raise ContractError(
                    f"{context} line {number} must contain {columns} non-empty columns"
                )
            rows.append(fields)
    return rows


def load_matrix(path: Path) -> dict[str, dict[str, str]]:
    result: dict[str, dict[str, str]] = {}
    digests: set[str] = set()
    for fields in read_tsv(path, 7, "matrix"):
        identifier, image, version, distribution, mode, lane, architecture = fields
        if identifier in result:
            raise ContractError(f"matrix contains duplicate id {identifier!r}")
        match = IMAGE_RE.fullmatch(image)
        if match is None:
            raise ContractError(f"matrix image for {identifier!r} is not immutable")
        digest = match.group("digest")
        if digest in digests:
            raise ContractError(f"matrix reuses image digest {digest!r}")
        digests.add(digest)
        result[identifier] = {
            "image": image,
            "version": version,
            "distribution": distribution,
            "mode": mode,
            "lane": lane,
            "architecture": architecture,
        }
    if not result:
        raise ContractError("matrix contains no rows")
    return result


def load_limitations(path: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for identifier, reason in read_tsv(path, 2, "limitations"):
        if identifier in result:
            raise ContractError(f"limitations contains duplicate id {identifier!r}")
        result[identifier] = reason
    return result


def validate_source_path(value: Any, context: str) -> str:
    path = require_string(value, context, limit=180)
    pure_path = PurePosixPath(path)
    if (
        pure_path.is_absolute()
        or ".." in pure_path.parts
        or path != pure_path.as_posix()
        or not path.startswith("images/podman/")
    ):
        raise ContractError(f"{context} must be a normalized repository-relative Podman path")
    return path


def load_catalogue(
    catalogue_path: Path, matrix_path: Path, limitations_path: Path
) -> list[dict[str, Any]]:
    with catalogue_path.open("rb") as handle:
        document = tomllib.load(handle)
    if not isinstance(document, dict):
        raise ContractError("candidate catalogue must be a TOML table")
    exact_keys(document, {"schema", "candidates"}, "candidate catalogue")
    if isinstance(document["schema"], bool) or document["schema"] != SCHEMA_VERSION:
        raise ContractError(f"candidate catalogue schema must be {SCHEMA_VERSION}")
    candidates = document["candidates"]
    if not isinstance(candidates, list) or not candidates:
        raise ContractError("candidate catalogue must contain at least one candidate")

    matrix = load_matrix(matrix_path)
    limitations = load_limitations(limitations_path)
    seen_ids: set[str] = set()
    seen_candidate_digests: set[str] = set()
    normalized: list[dict[str, Any]] = []
    matrix_digests = {
        IMAGE_RE.fullmatch(row["image"]).group("digest")  # type: ignore[union-attr]
        for row in matrix.values()
    }

    declared_ids = [candidate.get("id") for candidate in candidates if isinstance(candidate, dict)]
    if len(declared_ids) != len(candidates) or not all(
        isinstance(identifier, str) for identifier in declared_ids
    ):
        raise ContractError("every candidate must be a table with a string id")
    if declared_ids != sorted(declared_ids):
        raise ContractError("candidate catalogue must use deterministic id order")

    for index, raw_candidate in enumerate(candidates):
        context = f"candidate[{index}]"
        if not isinstance(raw_candidate, dict):
            raise ContractError(f"{context} must be a table")
        exact_keys(raw_candidate, EXPECTED_CANDIDATE_KEYS, context)
        candidate = copy.deepcopy(raw_candidate)
        identifier = require_string(candidate["id"], f"{context}.id", pattern=CANDIDATE_RE)
        if identifier in seen_ids:
            raise ContractError(f"candidate catalogue contains duplicate id {identifier!r}")
        seen_ids.add(identifier)
        if identifier not in matrix:
            raise ContractError(f"candidate {identifier!r} is absent from the matrix")
        if limitations.get(identifier) != EXPECTED_LIMITATION:
            raise ContractError(
                f"candidate {identifier!r} is not an exact {EXPECTED_LIMITATION!r} limitation"
            )

        baseline = require_string(candidate["baseline-image"], f"{context}.baseline-image")
        replacement = require_string(candidate["candidate-image"], f"{context}.candidate-image")
        baseline_match = IMAGE_RE.fullmatch(baseline)
        replacement_match = IMAGE_RE.fullmatch(replacement)
        if baseline_match is None or replacement_match is None:
            raise ContractError(f"{context} images must contain an exact tag and digest")
        matrix_row = matrix[identifier]
        if baseline != matrix_row["image"]:
            raise ContractError(f"{context}.baseline-image does not equal the accepted matrix row")
        if (
            baseline_match.group("name") != replacement_match.group("name")
            or baseline_match.group("tag") != replacement_match.group("tag")
        ):
            raise ContractError(f"{context}.candidate-image changes repository or release tag")
        replacement_digest = replacement_match.group("digest")
        if replacement_digest == baseline_match.group("digest"):
            raise ContractError(f"{context}.candidate-image reuses the baseline digest")
        if replacement_digest in matrix_digests or replacement_digest in seen_candidate_digests:
            raise ContractError(f"{context}.candidate-image reuses an accepted or candidate digest")
        seen_candidate_digests.add(replacement_digest)

        expected_pairs = {
            "expected-podman-version": matrix_row["version"],
            "expected-distribution": matrix_row["distribution"],
            "expected-mode": "rootless",
            "expected-lane": "container",
            "expected-architecture": "amd64",
            "expected-limitation": EXPECTED_LIMITATION,
            "source-repository": SOURCE_REPOSITORY,
            "source-license": SOURCE_LICENSE,
            "redistribution": REDISTRIBUTION,
        }
        for key, expected in expected_pairs.items():
            actual = require_string(candidate[key], f"{context}.{key}")
            if actual != expected or (
                key.startswith("expected-")
                and key not in {"expected-limitation"}
                and actual != matrix_row.get(key.removeprefix("expected-").replace("podman-", ""), actual)
            ):
                raise ContractError(f"{context}.{key} must be {expected!r}")
        for key in (
            "expected-baseline-observed-distribution",
            "expected-replacement-observed-distribution",
        ):
            observed_distribution = require_string(
                candidate[key], f"{context}.{key}", pattern=OBSERVED_DISTRIBUTION_RE
            )
            if not observed_distribution_matches(
                candidate["expected-distribution"], observed_distribution
            ):
                raise ContractError(
                    f"{context}.{key} does not belong to expected distribution family "
                    f"{candidate['expected-distribution']!r}"
                )
        if (
            candidate["expected-distribution"] == "opensuse-tumbleweed"
            and candidate["expected-baseline-observed-distribution"]
            == candidate["expected-replacement-observed-distribution"]
        ):
            raise ContractError(
                f"{context} must distinguish baseline and replacement Tumbleweed snapshots"
            )
        require_timestamp(candidate["published-at"], f"{context}.published-at")
        require_string(candidate["source-revision"], f"{context}.source-revision", pattern=GIT_SHA_RE)

        source_files = candidate["source-files"]
        if not isinstance(source_files, list) or len(source_files) != 3:
            raise ContractError(f"{context}.source-files must contain exactly three entries")
        roles: set[str] = set()
        paths: set[str] = set()
        for source_index, source_file in enumerate(source_files):
            source_context = f"{context}.source-files[{source_index}]"
            if not isinstance(source_file, dict):
                raise ContractError(f"{source_context} must be a table")
            exact_keys(source_file, EXPECTED_SOURCE_FILE_KEYS, source_context)
            role = require_string(source_file["role"], f"{source_context}.role")
            if role not in EXPECTED_ROLES or role in roles:
                raise ContractError(f"{source_context}.role is unknown or duplicated")
            roles.add(role)
            source_path = validate_source_path(source_file["path"], f"{source_context}.path")
            if source_path in paths:
                raise ContractError(f"{source_context}.path is duplicated")
            paths.add(source_path)
            require_string(source_file["sha256"], f"{source_context}.sha256", pattern=SHA256_RE)
        if roles != EXPECTED_ROLES:
            raise ContractError(f"{context}.source-files does not cover the required roles")
        expected_source_paths = {
            "image-definition": f"images/podman/{identifier}/container.yaml",
            "platform-recipe": (
                f"images/podman/platforms/{matrix_row['distribution']}/Containerfile"
            ),
            "runtime-config": (
                f"images/podman/platforms/{matrix_row['distribution']}/containers.conf"
            ),
        }
        actual_source_paths = {item["role"]: item["path"] for item in source_files}
        if actual_source_paths != expected_source_paths:
            raise ContractError(f"{context}.source-files are cross-wired to another image or platform")
        normalized.append(candidate)
    expected_ids = {
        identifier
        for identifier, reason in limitations.items()
        if reason == EXPECTED_LIMITATION
    }
    if seen_ids != expected_ids:
        raise ContractError(
            "candidate ids must exactly equal the helper-privilege-collision limitation ids"
        )
    return normalized


def candidate_by_id(candidates: list[dict[str, Any]], identifier: str) -> dict[str, Any]:
    matches = [candidate for candidate in candidates if candidate["id"] == identifier]
    if len(matches) != 1:
        raise ContractError(f"candidate-cell must resolve exactly once: {identifier!r}")
    return matches[0]


def candidate_tsv(candidate: dict[str, Any]) -> str:
    fields = [
        candidate["id"],
        candidate["baseline-image"],
        candidate["candidate-image"],
        candidate["expected-podman-version"],
        candidate["expected-distribution"],
        candidate["expected-baseline-observed-distribution"],
        candidate["expected-replacement-observed-distribution"],
        candidate["expected-mode"],
        candidate["expected-lane"],
        candidate["expected-architecture"],
        candidate["expected-limitation"],
        candidate["published-at"],
        candidate["source-repository"],
        candidate["source-revision"],
        candidate["source-license"],
        candidate["redistribution"],
    ]
    if any("\t" in field or "\n" in field for field in fields):
        raise ContractError("resolved candidate contains a control delimiter")
    return "\t".join(fields)


def false_map(names: tuple[str, ...]) -> dict[str, bool]:
    return {name: False for name in names}


def initialize_document(
    candidate: dict[str, Any],
    *,
    catalogue_path: Path,
    matrix_path: Path,
    limitations_path: Path,
    repository_commit: str,
    clean_tree: bool,
    provider: str,
    started_at: str,
) -> dict[str, Any]:
    baseline_digest = IMAGE_RE.fullmatch(candidate["baseline-image"]).group("digest")  # type: ignore[union-attr]
    del baseline_digest  # The declared reference, not an observation, remains in declared_image.
    return {
        "schema_version": SCHEMA_VERSION,
        "evidence_kind": EVIDENCE_KIND,
        "status": "running",
        "eligibility": False,
        "candidate": {
            "id": candidate["id"],
            "source_repository": candidate["source-repository"],
            "source_revision": candidate["source-revision"],
        },
        "repository": {"commit": repository_commit, "clean_tree": clean_tree},
        "execution": {
            "provider": provider,
            "architecture": "amd64",
            "outer_root_mode": "root",
        },
        "timestamps": {"started_at": started_at, "finished_at": None},
        "catalogues": catalogue_hashes(catalogue_path, matrix_path, limitations_path),
        "baseline": {
            "declared_image": candidate["baseline-image"],
            "expected_observed_distribution": candidate[
                "expected-baseline-observed-distribution"
            ],
            "observed_digest": None,
            "observed": {
                "podman_version": None,
                "package_revision": None,
                "distribution": None,
                "architecture": None,
                "uid": None,
                "rootless": None,
            },
            "historical_collision": false_map(
                (
                    "podman_info_failed",
                    "newuidmap_reported",
                    "permission_denied_reported",
                    "newuidmap_setuid",
                    "newgidmap_setuid",
                    "newuidmap_capability",
                    "newgidmap_capability",
                )
            ),
        },
        "replacement": {
            "declared_image": candidate["candidate-image"],
            "observed_digest": None,
            "expected_podman_version": candidate["expected-podman-version"],
            "expected_distribution": candidate["expected-distribution"],
            "expected_observed_distribution": candidate[
                "expected-replacement-observed-distribution"
            ],
            "expected_mode": candidate["expected-mode"],
            "expected_lane": candidate["expected-lane"],
            "expected_architecture": candidate["expected-architecture"],
            "observed": {
                "podman_version": None,
                "api_version": None,
                "package_revision": None,
                "distribution": None,
                "architecture": None,
                "rootless": None,
                "source_revision": None,
            },
        },
        "source_proof": {
            "published_at": candidate["published-at"],
            "source_license": candidate["source-license"],
            "redistribution": candidate["redistribution"],
            "files": [
                {"role": item["role"], "path": item["path"], "sha256": item["sha256"]}
                for item in candidate["source-files"]
            ],
        },
        "fresh_store": false_map(
            ("distinct_runtime", "distinct_socket", "distinct_graph_root", "distinct_name_prefix")
        ),
        "runtime_results": false_map(RUNTIME_CHECKS),
        "selector_exporters": {
            selector: false_map(EXPORTERS) for selector in SELECTORS
        },
        "diagnostic_privacy": false_map(PRIVACY_CHECKS),
        "reimports": {"compose": False, "quadlet": False},
        "external_apply": {"performed": False, "plan_applied": False, "reacquired": False},
        "cleanup": {
            "baseline_removed": False,
            "replacement_removed": False,
            "apply_target_removed": False,
        },
        "environment_boundary": {
            "kernel_capabilities": "shared-host",
            "cgroup_delegation": "shared-host",
            "selinux_enforcing": "unperformed",
            "systemd_execution": "unperformed",
        },
        "failure": None,
    }


def initialization_failure_document(
    started_at: str,
    finished_at: str,
    *,
    phase: str = "catalogue",
    code: str = "invalid-catalogue",
    candidate_cell: str | None = None,
    repository_commit: str | None = None,
    catalogues: dict[str, str] | None = None,
) -> dict[str, Any]:
    bounded_candidate = (
        candidate_cell
        if isinstance(candidate_cell, str)
        and CANDIDATE_RE.fullmatch(candidate_cell)
        else None
    )
    bounded_commit = (
        repository_commit
        if isinstance(repository_commit, str) and GIT_SHA_RE.fullmatch(repository_commit)
        else None
    )
    return {
        "schema_version": SCHEMA_VERSION,
        "evidence_kind": INITIALIZATION_FAILURE_KIND,
        "status": "failed",
        "eligibility": False,
        "invocation": {
            "candidate_cell": bounded_candidate,
            "repository_commit": bounded_commit,
            "catalogues": copy.deepcopy(catalogues),
        },
        "timestamps": {"started_at": started_at, "finished_at": finished_at},
        "failure": {"phase": phase, "code": code},
        "initialization_failure": True,
    }


def validate_initialization_failure(
    document: dict[str, Any],
    *,
    expected_candidate_cell: str | None = None,
    expected_repository_commit: str | None = None,
    expected_catalogues: dict[str, str] | None = None,
) -> None:
    exact_keys(
        document,
        {
            "schema_version",
            "evidence_kind",
            "status",
            "eligibility",
            "invocation",
            "timestamps",
            "failure",
            "initialization_failure",
        },
        "initialization failure evidence",
    )
    if (
        isinstance(document["schema_version"], bool)
        or document["schema_version"] != SCHEMA_VERSION
        or document["evidence_kind"] != INITIALIZATION_FAILURE_KIND
        or document["status"] != "failed"
        or document["eligibility"] is not False
        or document["initialization_failure"] is not True
    ):
        raise ContractError("initialization failure evidence has an invalid bounded state")
    invocation = document["invocation"]
    if not isinstance(invocation, dict):
        raise ContractError("initialization failure invocation must be an object")
    exact_keys(
        invocation,
        {"candidate_cell", "repository_commit", "catalogues"},
        "initialization failure invocation",
    )
    candidate_cell = invocation["candidate_cell"]
    if candidate_cell is not None:
        require_string(
            candidate_cell,
            "initialization failure invocation candidate",
            pattern=CANDIDATE_RE,
        )
    repository_commit = invocation["repository_commit"]
    if repository_commit is not None:
        require_string(
            repository_commit,
            "initialization failure invocation repository commit",
            pattern=GIT_SHA_RE,
        )
    bound_catalogues = invocation["catalogues"]
    if bound_catalogues is not None:
        if not isinstance(bound_catalogues, dict):
            raise ContractError("initialization failure catalogues must be an object or null")
        exact_keys(
            bound_catalogues,
            {"candidates_sha256", "matrix_sha256", "limitations_sha256"},
            "initialization failure catalogues",
        )
        for name, digest in bound_catalogues.items():
            require_string(
                digest,
                f"initialization failure catalogues.{name}",
                pattern=SHA256_RE,
            )
    if expected_candidate_cell is not None:
        expected_candidate = require_string(
            expected_candidate_cell,
            "expected initialization failure candidate",
            pattern=CANDIDATE_RE,
        )
        if candidate_cell != expected_candidate:
            raise ContractError(
                "initialization failure candidate differs requested candidate-cell"
            )
    if expected_repository_commit is not None:
        expected_commit = require_string(
            expected_repository_commit,
            "expected repository commit",
            pattern=GIT_SHA_RE,
        )
        if repository_commit != expected_commit:
            raise ContractError(
                "initialization failure repository commit differs expected commit"
            )
    if expected_catalogues is not None and bound_catalogues != expected_catalogues:
        raise ContractError("initialization failure catalogue hashes differ current inputs")
    failure = document["failure"]
    if (
        not isinstance(failure, dict)
        or set(failure) != {"phase", "code"}
        or failure["phase"] not in {"preflight", "catalogue", "evidence"}
        or failure["code"] not in FAILURE_CODES_BY_PHASE[failure["phase"]]
    ):
        raise ContractError("initialization failure classification is invalid")
    timestamps = document["timestamps"]
    if not isinstance(timestamps, dict):
        raise ContractError("initialization failure timestamps must be an object")
    exact_keys(timestamps, {"started_at", "finished_at"}, "initialization failure timestamps")
    started_at = require_timestamp(timestamps["started_at"], "initialization failure start")
    finished_at = require_timestamp(timestamps["finished_at"], "initialization failure finish")
    if dt.datetime.strptime(finished_at, "%Y-%m-%dT%H:%M:%SZ") < dt.datetime.strptime(
        started_at, "%Y-%m-%dT%H:%M:%SZ"
    ):
        raise ContractError("initialization failure finish precedes its start")
    privacy_scan(document)


def atomic_write_json(path: Path, document: dict[str, Any]) -> None:
    if path.is_symlink():
        raise ContractError(f"refusing to replace symbolic-link evidence path {path}")
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    payload = json.dumps(document, indent=2, sort_keys=True) + "\n"
    temporary_path: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w", encoding="utf-8", dir=path.parent, prefix=f".{path.name}.", delete=False
        ) as temporary:
            temporary_path = Path(temporary.name)
            os.chmod(temporary_path, 0o600)
            temporary.write(payload)
            temporary.flush()
            os.fsync(temporary.fileno())
        os.replace(temporary_path, path)
        temporary_path = None
        directory_descriptor = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory_descriptor)
        finally:
            os.close(directory_descriptor)
    finally:
        if temporary_path is not None:
            temporary_path.unlink(missing_ok=True)


def load_json_object(path: Path) -> dict[str, Any]:
    if path.stat().st_size > MAX_JSON_BYTES:
        raise ContractError(f"evidence exceeds the {MAX_JSON_BYTES}-byte bound")

    def reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            if key in result:
                raise ContractError(f"evidence contains duplicate JSON key {key!r}")
            result[key] = value
        return result

    payload = path.read_text(encoding="utf-8")
    document = json.loads(payload, object_pairs_hook=reject_duplicate_keys)
    if not isinstance(document, dict):
        raise ContractError("evidence must be a JSON object")
    return document


def validate_failure_schema_contract(
    schema: Any, expected: dict[str, set[str]], context: str
) -> None:
    if not isinstance(schema, dict):
        raise ContractError(f"{context} must be an object")
    properties = schema.get("properties")
    if not isinstance(properties, dict):
        raise ContractError(f"{context} must define phase and code properties")

    def enum_values(name: str) -> set[str]:
        definition = properties.get(name)
        values = definition.get("enum") if isinstance(definition, dict) else None
        if (
            not isinstance(values, list)
            or not values
            or not all(isinstance(value, str) for value in values)
            or len(values) != len(set(values))
        ):
            raise ContractError(f"{context}.{name} must be a unique nonempty string enum")
        return set(values)

    expected_phases = set(expected)
    expected_codes = set().union(*expected.values())
    if enum_values("phase") != expected_phases or enum_values("code") != expected_codes:
        raise ContractError(f"{context} phase or code enum differs from the executable contract")

    rules = schema.get("allOf")
    if not isinstance(rules, list) or len(rules) != len(expected):
        raise ContractError(f"{context} is missing phase-specific failure-code constraints")
    actual: dict[str, set[str]] = {}
    for rule in rules:
        if not isinstance(rule, dict):
            raise ContractError(f"{context} contains a non-object phase constraint")
        phase = (
            rule.get("if", {})
            .get("properties", {})
            .get("phase", {})
            .get("const")
        )
        code_rule = rule.get("then", {}).get("properties", {}).get("code", {})
        codes = code_rule.get("enum") if isinstance(code_rule, dict) else None
        if (
            not isinstance(phase, str)
            or phase in actual
            or not isinstance(codes, list)
            or not codes
            or not all(isinstance(code, str) for code in codes)
            or len(codes) != len(set(codes))
        ):
            raise ContractError(f"{context} contains an invalid or duplicate phase constraint")
        actual[phase] = set(codes)
    if actual != expected:
        raise ContractError(f"{context} phase-specific codes differ from the executable contract")


def validate_schema_document(path: Path) -> None:
    document = load_json_object(path)
    if document.get("$schema") != "https://json-schema.org/draft/2020-12/schema":
        raise ContractError("evidence schema must use JSON Schema draft 2020-12")
    if document.get("type") != "object" or document.get("additionalProperties") is not False:
        raise ContractError("evidence schema root must reject additional properties")
    schema_version = document.get("properties", {}).get("schema_version", {})
    if schema_version != {"type": "integer", "const": SCHEMA_VERSION}:
        raise ContractError("evidence schema must distinguish integer schema version from boolean")
    if len(document.get("oneOf", [])) != 2 or len(document.get("allOf", [])) != 3:
        raise ContractError("evidence schema is missing its full/failure variants or state machine")
    definitions = document.get("$defs", {})
    if not isinstance(definitions, dict):
        raise ContractError("evidence schema must define its reusable contracts")
    validate_failure_schema_contract(
        definitions.get("failure"), FAILURE_CODES_BY_PHASE, "evidence schema failure"
    )
    if FAILURE_PHASES != set(FAILURE_CODES_BY_PHASE) or FAILURE_CODES != set().union(
        *FAILURE_CODES_BY_PHASE.values()
    ):
        raise ContractError("executable failure phase or code sets differ from their phase mapping")
    validate_failure_schema_contract(
        definitions.get("initializationFailure"),
        INITIALIZATION_FAILURE_CODES_BY_PHASE,
        "evidence schema initialization failure",
    )
    candidate_identity = definitions.get("candidateIdentity")
    if not isinstance(candidate_identity, dict):
        raise ContractError("evidence schema must define candidate identity")
    if (
        candidate_identity.get("type") != "object"
        or candidate_identity.get("additionalProperties") is not False
        or set(candidate_identity.get("required", []))
        != {"id", "source_repository", "source_revision"}
        or candidate_identity.get("properties", {})
        != {
            "id": {"type": "string", "pattern": CANDIDATE_PATTERN},
            "source_repository": {"const": SOURCE_REPOSITORY},
            "source_revision": {"$ref": "#/$defs/gitSha"},
        }
    ):
        raise ContractError("evidence schema candidate identity differs executable contract")
    initialization_invocation = definitions.get("initializationInvocation")
    if not isinstance(initialization_invocation, dict):
        raise ContractError("evidence schema must define initialization invocation")
    invocation_properties = initialization_invocation.get("properties", {})
    if (
        initialization_invocation.get("type") != "object"
        or initialization_invocation.get("additionalProperties") is not False
        or set(initialization_invocation.get("required", []))
        != {"candidate_cell", "repository_commit", "catalogues"}
        or invocation_properties.get("candidate_cell", {}).get("oneOf")
        != [
            {"type": "string", "pattern": CANDIDATE_PATTERN},
            {"type": "null"},
        ]
        or invocation_properties.get("repository_commit", {}).get("oneOf")
        != [{"$ref": "#/$defs/gitSha"}, {"type": "null"}]
        or invocation_properties.get("catalogues", {}).get("oneOf")
        != [{"$ref": "#/$defs/catalogues"}, {"type": "null"}]
    ):
        raise ContractError(
            "evidence schema initialization invocation differs executable contract"
        )
    initialization_variants = [
        variant
        for variant in document.get("oneOf", [])
        if isinstance(variant, dict)
        and variant.get("properties", {}).get("evidence_kind", {}).get("const")
        == INITIALIZATION_EVIDENCE_KIND
    ]
    if (
        len(initialization_variants) != 1
        or set(initialization_variants[0].get("required", []))
        != {"invocation", "initialization_failure"}
        or initialization_variants[0].get("properties", {}).get("failure")
        != {"$ref": "#/$defs/initializationFailure"}
        or document.get("properties", {}).get("invocation")
        != {"$ref": "#/$defs/initializationInvocation"}
    ):
        raise ContractError(
            "initialization-failure evidence must use binding and the restricted failure contract"
        )

    def visit(value: Any, location: str) -> None:
        if isinstance(value, dict):
            if value.get("type") == "object" and value.get("additionalProperties") is not False:
                raise ContractError(f"schema object {location} must reject additional properties")
            for key, child in value.items():
                visit(child, f"{location}/{key}")
        elif isinstance(value, list):
            for index, child in enumerate(value):
                visit(child, f"{location}/{index}")

    visit(document, "#")


def privacy_scan(document: Any) -> None:
    forbidden_keys = {
        "stdout",
        "stderr",
        "environment",
        "environment_values",
        "host_path",
        "socket_path",
        "container_id",
        "run_id",
        "command",
        "labels",
        "health_json",
        "runtime_json",
    }
    forbidden_fragments = (
        "/tmp/",
        "/home/",
        "/workspaces/",
        "unix://",
        "bf65-",
        "password",
        "passwd",
        "secret",
        "token",
        "private_key",
        "authorization",
        "bearer ",
    )
    address_pattern = re.compile(r"(?<![0-9])(?:[0-9]{1,3}\.){3}[0-9]{1,3}(?![0-9])")
    runtime_id_pattern = re.compile(
        r"(?:\b[0-9a-f]{12}\b|\b[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-"
        r"[89ab][0-9a-f]{3}-[0-9a-f]{12}\b)",
        re.IGNORECASE,
    )
    credential_pattern = re.compile(
        r"(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|"
        r"(?:AKIA|ASIA)[0-9A-Z]{16}|"
        r"[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,})"
    )
    free_form_runtime_id_pattern = re.compile(r"[0-9a-fA-F]{12,64}")
    allowed_hash_locations = {
        "$/candidate/source_revision",
        "$/repository/commit",
        "$/catalogues/candidates_sha256",
        "$/catalogues/matrix_sha256",
        "$/catalogues/limitations_sha256",
        "$/invocation/repository_commit",
        "$/invocation/catalogues/candidates_sha256",
        "$/invocation/catalogues/matrix_sha256",
        "$/invocation/catalogues/limitations_sha256",
        "$/baseline/observed_digest",
        "$/replacement/observed_digest",
        "$/replacement/observed/source_revision",
    }
    source_file_hash_location = re.compile(r"\$/source_proof/files/[0-9]+/sha256")

    def visit(value: Any, location: str) -> None:
        if isinstance(value, dict):
            for key, child in value.items():
                if key in forbidden_keys:
                    raise ContractError(f"privacy-forbidden evidence key at {location}/{key}")
                visit(child, f"{location}/{key}")
        elif isinstance(value, list):
            for index, child in enumerate(value):
                visit(child, f"{location}/{index}")
        elif isinstance(value, str):
            if len(value.encode("utf-8")) > 500 or any(
                ord(character) < 0x20 or character in BIDI_CONTROLS for character in value
            ):
                raise ContractError(f"unbounded or control-bearing evidence string at {location}")
            normalized = value.casefold()
            if any(fragment in normalized for fragment in forbidden_fragments):
                raise ContractError(f"privacy-forbidden evidence value at {location}")
            if (
                "=" in value
                or address_pattern.search(value)
                or runtime_id_pattern.search(value)
                or credential_pattern.search(value)
                or (
                    free_form_runtime_id_pattern.fullmatch(value)
                    and location not in allowed_hash_locations
                    and source_file_hash_location.fullmatch(location) is None
                )
            ):
                raise ContractError(f"assignment, address, or runtime identifier at {location}")

    visit(document, "$")


def validate_boolean_map(value: Any, names: tuple[str, ...], context: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ContractError(f"{context} must be an object")
    exact_keys(value, set(names), context)
    for name in names:
        require_bool(value[name], f"{context}.{name}")
    return value


def observed_distribution_matches(expected: str, observed: str | None) -> bool:
    if OBSERVED_DISTRIBUTION_RE.fullmatch(expected) is not None:
        return observed == expected
    if expected == "opensuse-tumbleweed":
        return observed is not None and re.fullmatch(
            r"opensuse-tumbleweed-[0-9]{8}", observed
        ) is not None
    expected_values = {
        "opensuse-leap-16.0": {"opensuse-leap-16.0"},
        "ubi-8": {"ubi-8.10"},
        "ubi-9": {"ubi-9.8"},
        "ubi-10": {"ubi-10.2"},
    }
    return observed in expected_values.get(expected, set())


def validate_evidence(document: dict[str, Any]) -> None:
    root_keys = {
        "schema_version",
        "evidence_kind",
        "status",
        "eligibility",
        "candidate",
        "repository",
        "execution",
        "timestamps",
        "catalogues",
        "baseline",
        "replacement",
        "source_proof",
        "fresh_store",
        "runtime_results",
        "selector_exporters",
        "diagnostic_privacy",
        "reimports",
        "external_apply",
        "cleanup",
        "environment_boundary",
        "failure",
    }
    exact_keys(document, root_keys, "evidence")
    if (
        isinstance(document["schema_version"], bool)
        or document["schema_version"] != SCHEMA_VERSION
        or document["evidence_kind"] != EVIDENCE_KIND
    ):
        raise ContractError("evidence version or kind is unsupported")
    if document["status"] not in {"running", "passed", "failed"}:
        raise ContractError("evidence status is invalid")
    require_bool(document["eligibility"], "evidence.eligibility")

    candidate = document["candidate"]
    if not isinstance(candidate, dict):
        raise ContractError("evidence.candidate must be an object")
    exact_keys(candidate, {"id", "source_repository", "source_revision"}, "evidence.candidate")
    require_string(candidate["id"], "evidence.candidate.id", pattern=CANDIDATE_RE)
    if candidate["source_repository"] != SOURCE_REPOSITORY:
        raise ContractError("evidence candidate source repository is invalid")
    require_string(candidate["source_revision"], "evidence.candidate.source_revision", pattern=GIT_SHA_RE)

    repository = document["repository"]
    if not isinstance(repository, dict):
        raise ContractError("evidence.repository must be an object")
    exact_keys(repository, {"commit", "clean_tree"}, "evidence.repository")
    require_string(repository["commit"], "evidence.repository.commit", pattern=GIT_SHA_RE)
    require_bool(repository["clean_tree"], "evidence.repository.clean_tree")

    execution = document["execution"]
    if not isinstance(execution, dict):
        raise ContractError("evidence.execution must be an object")
    exact_keys(execution, {"provider", "architecture", "outer_root_mode"}, "evidence.execution")
    if execution["provider"] not in {"github-actions", "local"}:
        raise ContractError("evidence execution provider is invalid")
    if execution["architecture"] != "amd64" or execution["outer_root_mode"] != "root":
        raise ContractError("evidence execution boundary is invalid")

    timestamps = document["timestamps"]
    if not isinstance(timestamps, dict):
        raise ContractError("evidence.timestamps must be an object")
    exact_keys(timestamps, {"started_at", "finished_at"}, "evidence.timestamps")
    require_timestamp(timestamps["started_at"], "evidence.timestamps.started_at")
    if timestamps["finished_at"] is not None:
        require_timestamp(timestamps["finished_at"], "evidence.timestamps.finished_at")

    catalogues = document["catalogues"]
    if not isinstance(catalogues, dict):
        raise ContractError("evidence.catalogues must be an object")
    exact_keys(
        catalogues,
        {"candidates_sha256", "matrix_sha256", "limitations_sha256"},
        "evidence.catalogues",
    )
    for name, value in catalogues.items():
        require_string(value, f"evidence.catalogues.{name}", pattern=SHA256_RE)

    baseline = document["baseline"]
    if not isinstance(baseline, dict):
        raise ContractError("evidence.baseline must be an object")
    exact_keys(
        baseline,
        {
            "declared_image",
            "expected_observed_distribution",
            "observed_digest",
            "observed",
            "historical_collision",
        },
        "evidence.baseline",
    )
    if IMAGE_RE.fullmatch(require_string(baseline["declared_image"], "evidence.baseline.declared_image")) is None:
        raise ContractError("evidence baseline image is not immutable")
    require_string(
        baseline["expected_observed_distribution"],
        "evidence.baseline.expected_observed_distribution",
        pattern=OBSERVED_DISTRIBUTION_RE,
        limit=80,
    )
    require_optional_string(baseline["observed_digest"], "evidence.baseline.observed_digest", pattern=SHA256_RE)
    baseline_observed = baseline["observed"]
    if not isinstance(baseline_observed, dict):
        raise ContractError("evidence.baseline.observed must be an object")
    exact_keys(
        baseline_observed,
        {
            "podman_version",
            "package_revision",
            "distribution",
            "architecture",
            "uid",
            "rootless",
        },
        "evidence.baseline.observed",
    )
    require_optional_string(
        baseline_observed["podman_version"],
        "evidence.baseline.observed.podman_version",
        pattern=PODMAN_VERSION_RE,
        limit=80,
    )
    require_optional_string(
        baseline_observed["package_revision"],
        "evidence.baseline.observed.package_revision",
        pattern=PACKAGE_REVISION_RE,
        limit=160,
    )
    require_optional_string(
        baseline_observed["distribution"],
        "evidence.baseline.observed.distribution",
        pattern=OBSERVED_DISTRIBUTION_RE,
        limit=80,
    )
    baseline_architecture = require_optional_string(
        baseline_observed["architecture"],
        "evidence.baseline.observed.architecture",
        limit=20,
    )
    if baseline_architecture not in {None, "amd64", "x86_64"}:
        raise ContractError("evidence baseline observed architecture is invalid")
    baseline_uid = baseline_observed["uid"]
    if baseline_uid is not None and (
        isinstance(baseline_uid, bool)
        or not isinstance(baseline_uid, int)
        or baseline_uid != 1000
    ):
        raise ContractError("evidence baseline observed uid must be 1000")
    if baseline_observed["rootless"] is not None:
        require_bool(baseline_observed["rootless"], "evidence.baseline.observed.rootless")
    historical = validate_boolean_map(
        baseline["historical_collision"],
        (
            "podman_info_failed",
            "newuidmap_reported",
            "permission_denied_reported",
            "newuidmap_setuid",
            "newgidmap_setuid",
            "newuidmap_capability",
            "newgidmap_capability",
        ),
        "evidence.baseline.historical_collision",
    )

    replacement = document["replacement"]
    if not isinstance(replacement, dict):
        raise ContractError("evidence.replacement must be an object")
    exact_keys(
        replacement,
        {
            "declared_image",
            "observed_digest",
            "expected_podman_version",
            "expected_distribution",
            "expected_observed_distribution",
            "expected_mode",
            "expected_lane",
            "expected_architecture",
            "observed",
        },
        "evidence.replacement",
    )
    replacement_match = IMAGE_RE.fullmatch(
        require_string(replacement["declared_image"], "evidence.replacement.declared_image")
    )
    if replacement_match is None:
        raise ContractError("evidence replacement image is not immutable")
    require_optional_string(replacement["observed_digest"], "evidence.replacement.observed_digest", pattern=SHA256_RE)
    require_string(replacement["expected_podman_version"], "evidence.replacement.expected_podman_version", limit=80)
    require_string(replacement["expected_distribution"], "evidence.replacement.expected_distribution", limit=80)
    require_string(
        replacement["expected_observed_distribution"],
        "evidence.replacement.expected_observed_distribution",
        pattern=OBSERVED_DISTRIBUTION_RE,
        limit=80,
    )
    if (
        replacement["expected_mode"] != "rootless"
        or replacement["expected_lane"] != "container"
        or replacement["expected_architecture"] != "amd64"
    ):
        raise ContractError("evidence replacement expected boundary is invalid")
    if not observed_distribution_matches(
        replacement["expected_distribution"], baseline["expected_observed_distribution"]
    ) or not observed_distribution_matches(
        replacement["expected_distribution"], replacement["expected_observed_distribution"]
    ):
        raise ContractError("evidence observed-distribution expectations are outside the family")
    baseline_observed = document["baseline"]["observed"]
    if baseline_observed["podman_version"] is not None and (
        baseline_observed["podman_version"] != replacement["expected_podman_version"]
    ):
        raise ContractError("baseline Podman version differs from the candidate contract")
    if baseline_observed["distribution"] is not None and not observed_distribution_matches(
        baseline["expected_observed_distribution"], baseline_observed["distribution"]
    ):
        raise ContractError("baseline distribution differs from the candidate contract")

    observed = replacement["observed"]
    if not isinstance(observed, dict):
        raise ContractError("evidence.replacement.observed must be an object")
    exact_keys(
        observed,
        {
            "podman_version",
            "api_version",
            "package_revision",
            "distribution",
            "architecture",
            "rootless",
            "source_revision",
        },
        "evidence.replacement.observed",
    )
    require_optional_string(
        observed["podman_version"],
        "evidence.replacement.observed.podman_version",
        pattern=PODMAN_VERSION_RE,
        limit=80,
    )
    require_optional_string(
        observed["api_version"],
        "evidence.replacement.observed.api_version",
        pattern=API_VERSION_RE,
        limit=80,
    )
    require_optional_string(
        observed["package_revision"],
        "evidence.replacement.observed.package_revision",
        pattern=PACKAGE_REVISION_RE,
        limit=160,
    )
    require_optional_string(
        observed["distribution"],
        "evidence.replacement.observed.distribution",
        pattern=OBSERVED_DISTRIBUTION_RE,
        limit=80,
    )
    architecture_value = require_optional_string(
        observed["architecture"], "evidence.replacement.observed.architecture", limit=20
    )
    if architecture_value is not None and architecture_value not in {"amd64", "x86_64"}:
        raise ContractError("evidence replacement observed architecture is invalid")
    if observed["rootless"] is not None:
        require_bool(observed["rootless"], "evidence.replacement.observed.rootless")
    require_optional_string(
        observed["source_revision"],
        "evidence.replacement.observed.source_revision",
        pattern=GIT_SHA_RE,
    )

    source_proof = document["source_proof"]
    if not isinstance(source_proof, dict):
        raise ContractError("evidence.source_proof must be an object")
    exact_keys(source_proof, {"published_at", "source_license", "redistribution", "files"}, "evidence.source_proof")
    require_timestamp(source_proof["published_at"], "evidence.source_proof.published_at")
    if source_proof["source_license"] != SOURCE_LICENSE or source_proof["redistribution"] != REDISTRIBUTION:
        raise ContractError("evidence source license or redistribution decision is invalid")
    files = source_proof["files"]
    if not isinstance(files, list) or len(files) != 3:
        raise ContractError("evidence source proof must contain three files")
    roles: set[str] = set()
    for index, source_file in enumerate(files):
        if not isinstance(source_file, dict):
            raise ContractError("evidence source file must be an object")
        exact_keys(source_file, EXPECTED_SOURCE_FILE_KEYS, f"evidence.source_proof.files[{index}]")
        role = require_string(source_file["role"], f"evidence.source_proof.files[{index}].role")
        if role not in EXPECTED_ROLES or role in roles:
            raise ContractError("evidence source proof roles are invalid")
        roles.add(role)
        validate_source_path(source_file["path"], f"evidence.source_proof.files[{index}].path")
        require_string(source_file["sha256"], f"evidence.source_proof.files[{index}].sha256", pattern=SHA256_RE)

    fresh = validate_boolean_map(
        document["fresh_store"],
        ("distinct_runtime", "distinct_socket", "distinct_graph_root", "distinct_name_prefix"),
        "evidence.fresh_store",
    )
    runtime = validate_boolean_map(document["runtime_results"], RUNTIME_CHECKS, "evidence.runtime_results")
    selectors = document["selector_exporters"]
    if not isinstance(selectors, dict):
        raise ContractError("evidence.selector_exporters must be an object")
    exact_keys(selectors, set(SELECTORS), "evidence.selector_exporters")
    for selector in SELECTORS:
        validate_boolean_map(selectors[selector], EXPORTERS, f"evidence.selector_exporters.{selector}")
    diagnostic_privacy = validate_boolean_map(
        document["diagnostic_privacy"], PRIVACY_CHECKS, "evidence.diagnostic_privacy"
    )
    reimports = validate_boolean_map(document["reimports"], ("compose", "quadlet"), "evidence.reimports")
    external_apply = validate_boolean_map(
        document["external_apply"],
        ("performed", "plan_applied", "reacquired"),
        "evidence.external_apply",
    )
    cleanup = validate_boolean_map(
        document["cleanup"],
        ("baseline_removed", "replacement_removed", "apply_target_removed"),
        "evidence.cleanup",
    )
    boundary = document["environment_boundary"]
    if not isinstance(boundary, dict):
        raise ContractError("evidence.environment_boundary must be an object")
    exact_keys(
        boundary,
        {
            "kernel_capabilities",
            "cgroup_delegation",
            "selinux_enforcing",
            "systemd_execution",
        },
        "evidence.environment_boundary",
    )
    if boundary != {
        "kernel_capabilities": "shared-host",
        "cgroup_delegation": "shared-host",
        "selinux_enforcing": "unperformed",
        "systemd_execution": "unperformed",
    }:
        raise ContractError("evidence environment boundary is invalid")

    failure_value = document["failure"]
    if failure_value is not None:
        if not isinstance(failure_value, dict):
            raise ContractError("evidence.failure must be null or an object")
        exact_keys(failure_value, {"phase", "code"}, "evidence.failure")
        if (
            failure_value["phase"] not in FAILURE_PHASES
            or failure_value["code"] not in FAILURE_CODES
            or failure_value["code"] not in FAILURE_CODES_BY_PHASE[failure_value["phase"]]
        ):
            raise ContractError("evidence failure classification is invalid")

    if timestamps["finished_at"] is not None:
        started = dt.datetime.strptime(timestamps["started_at"], "%Y-%m-%dT%H:%M:%SZ")
        finished = dt.datetime.strptime(timestamps["finished_at"], "%Y-%m-%dT%H:%M:%SZ")
        if finished < started:
            raise ContractError("evidence finish timestamp precedes its start timestamp")

    baseline_match = IMAGE_RE.fullmatch(baseline["declared_image"])
    mandatory = [
        repository["clean_tree"],
        baseline["observed_digest"] == baseline_match.group("digest"),  # type: ignore[union-attr]
        baseline_observed["podman_version"] == replacement["expected_podman_version"],
        observed_distribution_matches(
            baseline["expected_observed_distribution"], baseline_observed["distribution"]
        ),
        baseline_observed["architecture"] in {"amd64", "x86_64"},
        baseline_observed["uid"] == 1000,
        baseline_observed["rootless"] is True,
        isinstance(baseline_observed["package_revision"], str)
        and bool(baseline_observed["package_revision"]),
        all(historical.values()),
        replacement["observed_digest"] == replacement_match.group("digest"),
        observed["podman_version"] == replacement["expected_podman_version"],
        observed_distribution_matches(
            replacement["expected_observed_distribution"], observed["distribution"]
        ),
        observed["architecture"] in {"amd64", "x86_64"},
        observed["rootless"] is True,
        observed["source_revision"] == candidate["source_revision"],
        isinstance(observed["api_version"], str) and bool(observed["api_version"]),
        isinstance(observed["package_revision"], str) and bool(observed["package_revision"]),
        all(fresh.values()),
        all(runtime.values()),
        all(all(selectors[name].values()) for name in SELECTORS),
        all(diagnostic_privacy.values()),
        all(reimports.values()),
        all(external_apply.values()),
        all(cleanup.values()),
    ]
    eligible = all(mandatory)
    if document["status"] == "running":
        if document["eligibility"] or failure_value is not None or timestamps["finished_at"] is not None:
            raise ContractError("running evidence cannot be eligible, finished, or failed")
    elif document["status"] == "failed":
        if document["eligibility"] or failure_value is None or timestamps["finished_at"] is None:
            raise ContractError("failed evidence must be ineligible, classified, and finished")
    else:
        if failure_value is not None or timestamps["finished_at"] is None:
            raise ContractError("passed evidence cannot contain a failure and must be finished")
        if not eligible or document["eligibility"] is not True:
            raise ContractError("passed evidence is missing one or more mandatory proofs")
    privacy_scan(document)


def validate_evidence_binding(
    document: dict[str, Any],
    *,
    catalogue_path: Path,
    matrix_path: Path,
    limitations_path: Path,
    candidate_cell: str,
    expected_repository_commit: str | None,
) -> dict[str, Any]:
    candidates = load_catalogue(catalogue_path, matrix_path, limitations_path)
    candidate = candidate_by_id(candidates, candidate_cell)
    if document["candidate"]["id"] != candidate_cell:
        raise ContractError("evidence candidate id differs from the requested candidate-cell")
    expected_identity = {
        "id": candidate["id"],
        "source_repository": candidate["source-repository"],
        "source_revision": candidate["source-revision"],
    }
    if document["candidate"] != expected_identity:
        raise ContractError("evidence candidate identity differs from the current catalogue")
    expected_catalogue_hashes = {
        "candidates_sha256": sha256_file(catalogue_path),
        "matrix_sha256": sha256_file(matrix_path),
        "limitations_sha256": sha256_file(limitations_path),
    }
    if document["catalogues"] != expected_catalogue_hashes:
        raise ContractError("evidence catalogue hashes differ from the current inputs")
    if document["baseline"]["declared_image"] != candidate["baseline-image"]:
        raise ContractError("evidence baseline reference differs from the current catalogue")
    if document["baseline"]["expected_observed_distribution"] != candidate[
        "expected-baseline-observed-distribution"
    ]:
        raise ContractError(
            "evidence baseline observed-distribution expectation differs from the current catalogue"
        )
    replacement = document["replacement"]
    expected_replacement = {
        "declared_image": candidate["candidate-image"],
        "expected_podman_version": candidate["expected-podman-version"],
        "expected_distribution": candidate["expected-distribution"],
        "expected_observed_distribution": candidate[
            "expected-replacement-observed-distribution"
        ],
        "expected_mode": candidate["expected-mode"],
        "expected_lane": candidate["expected-lane"],
        "expected_architecture": candidate["expected-architecture"],
    }
    for key, expected in expected_replacement.items():
        if replacement[key] != expected:
            raise ContractError(f"evidence replacement {key} differs from the current catalogue")
    expected_source_proof = {
        "published_at": candidate["published-at"],
        "source_license": candidate["source-license"],
        "redistribution": candidate["redistribution"],
        "files": [
            {"role": item["role"], "path": item["path"], "sha256": item["sha256"]}
            for item in candidate["source-files"]
        ],
    }
    if document["source_proof"] != expected_source_proof:
        raise ContractError("evidence source proof differs from the current catalogue")
    if expected_repository_commit is not None:
        require_string(
            expected_repository_commit, "expected repository commit", pattern=GIT_SHA_RE
        )
        if document["repository"]["commit"] != expected_repository_commit:
            raise ContractError("evidence repository commit differs from the expected commit")
    observed = replacement["observed"]
    if observed["podman_version"] is not None and (
        observed["podman_version"] != candidate["expected-podman-version"]
    ):
        raise ContractError("observed Podman version differs from the candidate contract")
    if observed["distribution"] is not None and not observed_distribution_matches(
        candidate["expected-replacement-observed-distribution"], observed["distribution"]
    ):
        raise ContractError("observed distribution differs from the candidate contract")
    if observed["source_revision"] is not None and (
        observed["source_revision"] != candidate["source-revision"]
    ):
        raise ContractError("observed source revision differs from the candidate contract")
    return candidate


def validate_bound_evidence(
    document: dict[str, Any],
    *,
    schema_path: Path,
    catalogue_path: Path,
    matrix_path: Path,
    limitations_path: Path,
    candidate_cell: str,
    expected_repository_commit: str | None,
) -> dict[str, Any]:
    validate_schema_document(schema_path)
    validate_evidence(document)
    return validate_evidence_binding(
        document,
        catalogue_path=catalogue_path,
        matrix_path=matrix_path,
        limitations_path=limitations_path,
        candidate_cell=candidate_cell,
        expected_repository_commit=expected_repository_commit,
    )


def nested_result(document: dict[str, Any], result: str) -> tuple[dict[str, Any], str]:
    if result not in EXPECTED_RESULTS:
        raise ContractError(f"result name is outside the bounded contract: {result!r}")
    parts = result.split(".")
    parent: dict[str, Any] = document
    for part in parts[:-1]:
        child = parent.get(part)
        if not isinstance(child, dict):
            raise ContractError(f"result path is not an evidence boolean: {result!r}")
        parent = child
    return parent, parts[-1]


def mark_result_document(document: dict[str, Any], result: str) -> None:
    if document.get("status") != "running":
        raise ContractError("only running evidence can record a result")
    parent, key = nested_result(document, result)
    if parent.get(key) is not False:
        raise ContractError(f"result was already recorded or is not boolean: {result!r}")
    parent[key] = True


def record_observation_document(
    document: dict[str, Any], field: str, value: str, candidate: dict[str, Any]
) -> None:
    if document.get("status") != "running":
        raise ContractError("only running evidence can record an observation")
    if field not in OBSERVATION_FIELDS:
        raise ContractError(f"observation field is outside the bounded contract: {field!r}")
    expected_values: dict[str, Any] = {
        "baseline.observed_digest": IMAGE_RE.fullmatch(candidate["baseline-image"]).group("digest"),  # type: ignore[union-attr]
        "baseline.observed.podman_version": candidate["expected-podman-version"],
        "replacement.observed_digest": IMAGE_RE.fullmatch(candidate["candidate-image"]).group("digest"),  # type: ignore[union-attr]
        "replacement.observed.podman_version": candidate["expected-podman-version"],
        "replacement.observed.source_revision": candidate["source-revision"],
    }
    if field in {"baseline.observed.rootless", "replacement.observed.rootless"}:
        parsed_value: Any = parse_bool(value)
        if parsed_value is not True:
            raise ContractError("runtime observation must report rootless=true")
    elif field == "baseline.observed.uid":
        if value != "1000":
            raise ContractError("baseline observation must report uid=1000")
        parsed_value = 1000
    else:
        parsed_value = value
    if field in expected_values and parsed_value != expected_values[field]:
        raise ContractError(f"observation {field!r} differs from the current candidate contract")
    if field in {
        "baseline.observed.distribution",
        "replacement.observed.distribution",
    } and not observed_distribution_matches(
        candidate[
            "expected-baseline-observed-distribution"
            if field.startswith("baseline.")
            else "expected-replacement-observed-distribution"
        ],
        value,
    ):
        raise ContractError("observed distribution differs from the current candidate contract")
    parts = field.split(".")
    parent: dict[str, Any] = document
    for part in parts[:-1]:
        child = parent.get(part)
        if not isinstance(child, dict):
            raise ContractError(f"observation path is invalid: {field!r}")
        parent = child
    if parent.get(parts[-1]) is not None:
        raise ContractError(f"observation was already recorded: {field!r}")
    parent[parts[-1]] = parsed_value


def finalize_document(document: dict[str, Any], *, finished_at: str) -> None:
    if document.get("status") != "running":
        raise ContractError("only running evidence can be finalized")
    document["timestamps"]["finished_at"] = finished_at
    document["status"] = "passed"
    document["eligibility"] = True
    document["failure"] = None
    validate_evidence(document)


def ensure_failure_document(
    document: dict[str, Any], *, phase: str, code: str, finished_at: str
) -> None:
    if (
        phase not in FAILURE_PHASES
        or code not in FAILURE_CODES
        or code not in FAILURE_CODES_BY_PHASE[phase]
    ):
        raise ContractError("failure phase or code is outside the bounded classification")
    if document.get("status") == "passed":
        raise ContractError("passed evidence cannot be rewritten as a failure")
    document["status"] = "failed"
    document["eligibility"] = False
    document["timestamps"]["finished_at"] = finished_at
    document["failure"] = {"phase": phase, "code": code}
    validate_evidence(document)


def add_catalogue_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--catalogue", required=True, type=Path)
    parser.add_argument("--matrix", required=True, type=Path)
    parser.add_argument("--limitations", required=True, type=Path)


def add_bound_evidence_arguments(parser: argparse.ArgumentParser) -> None:
    add_catalogue_arguments(parser)
    parser.add_argument("--schema", required=True, type=Path)
    parser.add_argument("--candidate-cell", required=True)
    parser.add_argument("--expected-repository-commit")


def parse_bool(value: str) -> bool:
    if value == "true":
        return True
    if value == "false":
        return False
    raise argparse.ArgumentTypeError("expected true or false")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    list_parser = subparsers.add_parser("list-candidates")
    add_catalogue_arguments(list_parser)

    resolve = subparsers.add_parser("resolve-candidate")
    add_catalogue_arguments(resolve)
    resolve.add_argument("--candidate-cell", required=True)
    resolve.add_argument("--format", choices=("json", "tsv"), default="json")

    initialize = subparsers.add_parser("initialize-evidence")
    add_catalogue_arguments(initialize)
    initialize.add_argument("--schema", required=True, type=Path)
    initialize.add_argument("--candidate-cell", required=True)
    initialize.add_argument("--output", required=True, type=Path)
    initialize.add_argument("--repository-commit", required=True)
    initialize.add_argument("--repository-clean-tree", required=True, type=parse_bool)
    initialize.add_argument("--provider", choices=("github-actions", "local"), required=True)
    initialize.add_argument("--started-at")

    initialize_failure = subparsers.add_parser("initialize-failure-evidence")
    initialize_failure.add_argument("--output", required=True, type=Path)
    initialize_failure.add_argument(
        "--phase", choices=("preflight", "catalogue", "evidence"), required=True
    )
    initialize_failure.add_argument("--code", choices=sorted(FAILURE_CODES), required=True)
    initialize_failure.add_argument("--started-at")
    initialize_failure.add_argument("--finished-at")
    initialize_failure.add_argument("--candidate-cell")
    initialize_failure.add_argument("--repository-commit")
    initialize_failure.add_argument("--catalogue", type=Path)
    initialize_failure.add_argument("--matrix", type=Path)
    initialize_failure.add_argument("--limitations", type=Path)

    finalize = subparsers.add_parser("finalize-evidence")
    finalize.add_argument("--evidence", required=True, type=Path)
    add_bound_evidence_arguments(finalize)
    finalize.add_argument("--finished-at")

    mark = subparsers.add_parser("mark-result")
    mark.add_argument("--evidence", required=True, type=Path)
    add_bound_evidence_arguments(mark)
    mark.add_argument("--result", required=True, choices=sorted(EXPECTED_RESULTS))

    record = subparsers.add_parser("record-observation")
    record.add_argument("--evidence", required=True, type=Path)
    add_bound_evidence_arguments(record)
    record.add_argument("--field", required=True, choices=sorted(OBSERVATION_FIELDS))
    record.add_argument("--value", required=True)

    validate = subparsers.add_parser("validate-evidence")
    validate.add_argument("--evidence", required=True, type=Path)
    add_bound_evidence_arguments(validate)

    ensure = subparsers.add_parser("ensure-failure-evidence")
    ensure.add_argument("--evidence", required=True, type=Path)
    add_bound_evidence_arguments(ensure)
    ensure.add_argument("--phase", choices=sorted(FAILURE_PHASES), required=True)
    ensure.add_argument("--code", choices=sorted(FAILURE_CODES), required=True)
    ensure.add_argument("--finished-at")
    return parser


def run_command(arguments: argparse.Namespace) -> None:
    if arguments.command in {"list-candidates", "resolve-candidate"}:
        candidates = load_catalogue(arguments.catalogue, arguments.matrix, arguments.limitations)
    if arguments.command == "list-candidates":
        for candidate in candidates:
            print(candidate["id"])
    elif arguments.command == "resolve-candidate":
        candidate = candidate_by_id(candidates, arguments.candidate_cell)
        if arguments.format == "tsv":
            print(candidate_tsv(candidate))
        else:
            print(json.dumps(candidate, indent=2, sort_keys=True))
    elif arguments.command == "initialize-failure-evidence":
        started_at = require_timestamp(arguments.started_at or utc_now(), "started-at")
        finished_at = require_timestamp(arguments.finished_at or utc_now(), "finished-at")
        catalogue_arguments = (
            arguments.catalogue,
            arguments.matrix,
            arguments.limitations,
        )
        if any(path is not None for path in catalogue_arguments) and any(
            path is None for path in catalogue_arguments
        ):
            raise ContractError(
                "initialization-failure catalogue paths must be supplied together"
            )
        failure = initialization_failure_document(
            started_at,
            finished_at,
            phase=arguments.phase,
            code=arguments.code,
            candidate_cell=arguments.candidate_cell,
            repository_commit=arguments.repository_commit,
            catalogues=available_catalogue_hashes(*catalogue_arguments),
        )
        validate_initialization_failure(failure)
        atomic_write_json(arguments.output, failure)
    elif arguments.command == "initialize-evidence":
        started_at = utc_now()
        failure_phase = "evidence"
        failure_code = "evidence-invalid"
        try:
            started_at = require_timestamp(arguments.started_at or started_at, "started-at")
            validate_schema_document(arguments.schema)
            failure_phase = "catalogue"
            failure_code = "invalid-catalogue"
            candidates = load_catalogue(
                arguments.catalogue, arguments.matrix, arguments.limitations
            )
            repository_commit = require_string(
                arguments.repository_commit, "repository commit", pattern=GIT_SHA_RE
            )
            candidate = candidate_by_id(candidates, arguments.candidate_cell)
        except (
            ContractError,
            FileNotFoundError,
            OSError,
            json.JSONDecodeError,
            tomllib.TOMLDecodeError,
        ) as error:
            failure = initialization_failure_document(
                started_at,
                utc_now(),
                phase=failure_phase,
                code=failure_code,
                candidate_cell=arguments.candidate_cell,
                repository_commit=arguments.repository_commit,
                catalogues=available_catalogue_hashes(
                    arguments.catalogue,
                    arguments.matrix,
                    arguments.limitations,
                ),
            )
            validate_initialization_failure(failure)
            atomic_write_json(arguments.output, failure)
            raise ContractError(f"initialization failed; bounded failure evidence written: {error}") from error
        document = initialize_document(
            candidate,
            catalogue_path=arguments.catalogue,
            matrix_path=arguments.matrix,
            limitations_path=arguments.limitations,
            repository_commit=repository_commit,
            clean_tree=arguments.repository_clean_tree,
            provider=arguments.provider,
            started_at=started_at,
        )
        validate_bound_evidence(
            document,
            schema_path=arguments.schema,
            catalogue_path=arguments.catalogue,
            matrix_path=arguments.matrix,
            limitations_path=arguments.limitations,
            candidate_cell=arguments.candidate_cell,
            expected_repository_commit=repository_commit,
        )
        atomic_write_json(arguments.output, document)
    elif arguments.command in {
        "finalize-evidence",
        "mark-result",
        "record-observation",
        "validate-evidence",
        "ensure-failure-evidence",
    }:
        document = load_json_object(arguments.evidence)
        if document.get("evidence_kind") == INITIALIZATION_FAILURE_KIND:
            validate_schema_document(arguments.schema)
            validate_initialization_failure(
                document,
                expected_candidate_cell=arguments.candidate_cell,
                expected_repository_commit=arguments.expected_repository_commit,
                expected_catalogues=catalogue_hashes(
                    arguments.catalogue,
                    arguments.matrix,
                    arguments.limitations,
                ),
            )
            if arguments.command == "validate-evidence":
                return
            raise ContractError("bounded initialization-failure evidence is terminal")
        candidate = validate_bound_evidence(
            document,
            schema_path=arguments.schema,
            catalogue_path=arguments.catalogue,
            matrix_path=arguments.matrix,
            limitations_path=arguments.limitations,
            candidate_cell=arguments.candidate_cell,
            expected_repository_commit=arguments.expected_repository_commit,
        )
        if arguments.command == "mark-result":
            mark_result_document(document, arguments.result)
        elif arguments.command == "record-observation":
            record_observation_document(document, arguments.field, arguments.value, candidate)
        elif arguments.command == "finalize-evidence":
            finalize_document(
                document,
                finished_at=require_timestamp(arguments.finished_at or utc_now(), "finished-at"),
            )
        elif arguments.command == "ensure-failure-evidence":
            ensure_failure_document(
                document,
                phase=arguments.phase,
                code=arguments.code,
                finished_at=require_timestamp(arguments.finished_at or utc_now(), "finished-at"),
            )
        if arguments.command != "validate-evidence":
            validate_bound_evidence(
                document,
                schema_path=arguments.schema,
                catalogue_path=arguments.catalogue,
                matrix_path=arguments.matrix,
                limitations_path=arguments.limitations,
                candidate_cell=arguments.candidate_cell,
                expected_repository_commit=arguments.expected_repository_commit,
            )
            atomic_write_json(arguments.evidence, document)
    else:  # pragma: no cover - argparse makes this unreachable.
        raise AssertionError(arguments.command)


def expect_contract_error(operation: Any, description: str) -> None:
    try:
        operation()
    except ContractError:
        return
    raise AssertionError(f"counterfactual unexpectedly passed: {description}")


def self_test() -> None:
    repository_root = Path(__file__).resolve().parents[2]
    catalogue = repository_root / "fixtures/conformance/podman-live/candidates.toml"
    matrix = repository_root / "fixtures/conformance/podman-live/matrix.tsv"
    limitations = repository_root / "fixtures/conformance/podman-live/limitations.tsv"
    schema = repository_root / "docs/schemas/podman-limitation-revalidation-v1.schema.json"
    validate_schema_document(schema)
    candidates = load_catalogue(catalogue, matrix, limitations)
    assert len(candidates) == 5
    assert [candidate["id"] for candidate in candidates] == sorted(
        candidate["id"] for candidate in candidates
    )
    candidate = candidates[0]
    assert PACKAGE_REVISION_RE.fullmatch("upstream-source-build:5.4.2")
    expect_contract_error(
        lambda: require_string(
            "upstream-source-build:podman version 5.4.2",
            "synthetic package revision",
            pattern=PACKAGE_REVISION_RE,
        ),
        "package revision containing spaces",
    )

    with tempfile.TemporaryDirectory(prefix="podman-revalidation-hardened-self-test.") as name:
        directory = Path(name)
        catalogue_copy = directory / "candidates.toml"
        matrix_copy = directory / "matrix.tsv"
        limitations_copy = directory / "limitations.tsv"
        catalogue_copy.write_bytes(catalogue.read_bytes())
        matrix_copy.write_bytes(matrix.read_bytes())
        limitations_copy.write_bytes(limitations.read_bytes())
        original_catalogue = catalogue_copy.read_text(encoding="utf-8")

        def rejected_catalogue(contents: str, description: str) -> None:
            catalogue_copy.write_text(contents, encoding="utf-8")
            expect_contract_error(
                lambda: load_catalogue(catalogue_copy, matrix_copy, limitations_copy),
                description,
            )
            catalogue_copy.write_text(original_catalogue, encoding="utf-8")

        rejected_catalogue(
            original_catalogue.replace("schema = 1", "schema = true", 1),
            "boolean catalogue schema",
        )
        first_end = original_catalogue.find("\n[[candidates]]", original_catalogue.find("[[candidates]]") + 1)
        rejected_catalogue(
            original_catalogue[first_end + 1 :],
            "deleted candidate",
        )
        candidate_path = f"images/podman/{candidate['id']}/container.yaml"
        wrong_path = f"images/podman/{candidates[1]['id']}/container.yaml"
        rejected_catalogue(
            original_catalogue.replace(candidate_path, wrong_path, 1),
            "cross-wired image definition",
        )
        platform_path = f"images/podman/platforms/{candidate['expected-distribution']}/Containerfile"
        wrong_platform = (
            f"images/podman/platforms/{candidates[1]['expected-distribution']}/Containerfile"
        )
        rejected_catalogue(
            original_catalogue.replace(platform_path, wrong_platform, 1),
            "cross-wired platform recipe",
        )
        rejected_catalogue(
            original_catalogue.replace(candidate["id"], "podman-private-rootless", 1),
            "schema-shaped but unreviewed candidate id",
        )

        document = initialize_document(
            candidate,
            catalogue_path=catalogue,
            matrix_path=matrix,
            limitations_path=limitations,
            repository_commit="1" * 40,
            clean_tree=True,
            provider="local",
            started_at="2026-09-09T00:00:00Z",
        )
        validate_bound_evidence(
            document,
            schema_path=schema,
            catalogue_path=catalogue,
            matrix_path=matrix,
            limitations_path=limitations,
            candidate_cell=candidate["id"],
            expected_repository_commit="1" * 40,
        )
        tumbleweed = candidates[1]
        tumbleweed_document = initialize_document(
            tumbleweed,
            catalogue_path=catalogue,
            matrix_path=matrix,
            limitations_path=limitations,
            repository_commit="1" * 40,
            clean_tree=True,
            provider="local",
            started_at="2026-09-09T00:00:00Z",
        )
        expect_contract_error(
            lambda: record_observation_document(
                tumbleweed_document,
                "baseline.observed.distribution",
                tumbleweed["expected-replacement-observed-distribution"],
                tumbleweed,
            ),
            "replacement snapshot recorded as Tumbleweed baseline",
        )
        record_observation_document(
            tumbleweed_document,
            "baseline.observed.distribution",
            tumbleweed["expected-baseline-observed-distribution"],
            tumbleweed,
        )
        expect_contract_error(
            lambda: record_observation_document(
                tumbleweed_document,
                "replacement.observed.distribution",
                tumbleweed["expected-baseline-observed-distribution"],
                tumbleweed,
            ),
            "baseline snapshot recorded as Tumbleweed replacement",
        )
        unreviewed_full_evidence = copy.deepcopy(document)
        unreviewed_full_evidence["candidate"]["id"] = "podman-private-rootless"
        expect_contract_error(
            lambda: validate_evidence(unreviewed_full_evidence),
            "schema-shaped but unreviewed full-evidence candidate id",
        )

        def rejected_binding(mutator: Any, description: str) -> None:
            changed = copy.deepcopy(document)
            mutator(changed)
            expect_contract_error(
                lambda: validate_bound_evidence(
                    changed,
                    schema_path=schema,
                    catalogue_path=catalogue,
                    matrix_path=matrix,
                    limitations_path=limitations,
                    candidate_cell=candidate["id"],
                    expected_repository_commit="1" * 40,
                ),
                description,
            )

        rejected_binding(
            lambda value: value["candidate"].update({"id": candidates[1]["id"]}),
            "changed candidate id",
        )
        rejected_binding(
            lambda value: value["candidate"].update(
                {"source_revision": candidates[1]["source-revision"]}
            ),
            "changed candidate source revision",
        )
        rejected_binding(
            lambda value: value["source_proof"]["files"][0].update({"sha256": "2" * 64}),
            "changed source proof hash",
        )
        rejected_binding(
            lambda value: value["catalogues"].update({"matrix_sha256": "2" * 64}),
            "changed catalogue hash",
        )

        def change_versions(value: dict[str, Any]) -> None:
            value["replacement"]["expected_podman_version"] = "9.9.9"
            value["replacement"]["observed"]["podman_version"] = "9.9.9"

        rejected_binding(change_versions, "changed expected and observed Podman version")
        rejected_binding(
            lambda value: value["repository"].update({"commit": "2" * 40}),
            "changed repository commit",
        )

        baseline_digest = IMAGE_RE.fullmatch(candidate["baseline-image"]).group("digest")  # type: ignore[union-attr]
        replacement_digest = IMAGE_RE.fullmatch(candidate["candidate-image"]).group("digest")  # type: ignore[union-attr]
        observations = {
            "baseline.observed_digest": baseline_digest,
            "baseline.observed.podman_version": candidate["expected-podman-version"],
            "baseline.observed.package_revision": "5.4.2-160000.5.1.x86_64",
            "baseline.observed.distribution": candidate[
                "expected-baseline-observed-distribution"
            ],
            "baseline.observed.architecture": "x86_64",
            "baseline.observed.uid": "1000",
            "baseline.observed.rootless": "true",
            "replacement.observed_digest": replacement_digest,
            "replacement.observed.podman_version": candidate["expected-podman-version"],
            "replacement.observed.api_version": "5.4.0",
            "replacement.observed.package_revision": "5.4.2-160000.5.1.x86_64",
            "replacement.observed.distribution": candidate[
                "expected-replacement-observed-distribution"
            ],
            "replacement.observed.architecture": "x86_64",
            "replacement.observed.rootless": "true",
            "replacement.observed.source_revision": candidate["source-revision"],
        }
        completed = copy.deepcopy(document)
        for field, value in observations.items():
            record_observation_document(completed, field, value, candidate)
            validate_evidence(completed)
        for result in sorted(EXPECTED_RESULTS):
            mark_result_document(completed, result)
            validate_evidence(completed)
        finalize_document(completed, finished_at="2026-09-09T00:10:00Z")
        validate_bound_evidence(
            completed,
            schema_path=schema,
            catalogue_path=catalogue,
            matrix_path=matrix,
            limitations_path=limitations,
            candidate_cell=candidate["id"],
            expected_repository_commit="1" * 40,
        )

        baseline_version_mismatch = copy.deepcopy(completed)
        baseline_version_mismatch["baseline"]["observed"]["podman_version"] = "9.9.9"
        expect_contract_error(
            lambda: validate_evidence(baseline_version_mismatch),
            "baseline Podman version drift",
        )
        baseline_distribution_mismatch = copy.deepcopy(completed)
        baseline_distribution_mismatch["baseline"]["observed"]["distribution"] = "ubi-9.8"
        expect_contract_error(
            lambda: validate_evidence(baseline_distribution_mismatch),
            "baseline distribution drift",
        )
        dirty_pass = copy.deepcopy(completed)
        dirty_pass["repository"]["clean_tree"] = False
        expect_contract_error(lambda: validate_evidence(dirty_pass), "dirty passed evidence")

        missing = copy.deepcopy(completed)
        missing["status"] = "running"
        missing["eligibility"] = False
        missing["timestamps"]["finished_at"] = None
        missing["runtime_results"]["resource_creation"] = False
        expect_contract_error(
            lambda: finalize_document(missing, finished_at="2026-09-09T00:10:00Z"),
            "one missing granular result",
        )
        missing_cleanup = copy.deepcopy(completed)
        missing_cleanup["status"] = "running"
        missing_cleanup["eligibility"] = False
        missing_cleanup["timestamps"]["finished_at"] = None
        missing_cleanup["cleanup"]["replacement_removed"] = False
        expect_contract_error(
            lambda: finalize_document(
                missing_cleanup, finished_at="2026-09-09T00:10:00Z"
            ),
            "missing replacement cleanup proof",
        )

        for unsafe, description in (
            ("PASSWORD=hunter2", "environment assignment"),
            ("/tmp/private/value", "private path"),
            ("Bearer token-value", "protected token"),
            ("ghp_" + "A" * 36, "GitHub classic token"),
            ("github_pat_" + "A" * 40, "GitHub fine-grained token"),
            ("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.signaturevalue", "JWT"),
            ("AKIA" + "A" * 16, "AWS access key"),
            ("abcdef123456", "runtime identifier"),
            ("a" * 64, "long runtime identifier"),
            ("bad\nvalue", "control character"),
            ("safe\u202esecret", "bidirectional control"),
        ):
            unsafe_document = copy.deepcopy(document)
            unsafe_document["replacement"]["observed"]["package_revision"] = unsafe
            expect_contract_error(
                lambda value=unsafe_document: validate_evidence(value), description
            )

        free_form_runtime_identifier = copy.deepcopy(document)
        free_form_runtime_identifier["replacement"]["observed"]["api_version"] = (
            "abcdef1234567"
        )
        expect_contract_error(
            lambda: validate_evidence(free_form_runtime_identifier),
            "runtime identifier outside the package revision field",
        )

        boolean_version = copy.deepcopy(document)
        boolean_version["schema_version"] = True
        expect_contract_error(
            lambda: validate_evidence(boolean_version), "boolean evidence schema version"
        )

        early_failure = copy.deepcopy(document)
        mark_result_document(
            early_failure, "baseline.historical_collision.podman_info_failed"
        )
        ensure_failure_document(
            early_failure,
            phase="baseline-collision",
            code="historical-collision-not-reproduced",
            finished_at="2026-09-09T00:02:00Z",
        )
        assert early_failure["baseline"]["historical_collision"]["podman_info_failed"] is True
        metadata_failure = copy.deepcopy(document)
        ensure_failure_document(
            metadata_failure,
            phase="baseline-metadata",
            code="baseline-metadata-mismatch",
            finished_at="2026-09-09T00:02:00Z",
        )
        expect_contract_error(
            lambda: ensure_failure_document(
                copy.deepcopy(document),
                phase="baseline-metadata",
                code="historical-collision-not-reproduced",
                finished_at="2026-09-09T00:02:00Z",
            ),
            "baseline metadata failure misclassified as historical collision",
        )
        expect_contract_error(
            lambda: ensure_failure_document(
                copy.deepcopy(document),
                phase="baseline-collision",
                code="pull-failed",
                finished_at="2026-09-09T00:02:00Z",
            ),
            "invalid phase-code pair",
        )
        expect_contract_error(
            lambda: ensure_failure_document(
                copy.deepcopy(document),
                phase="resource-suite",
                code="resource-contract-failed",
                finished_at="2026-08-01T00:00:00Z",
            ),
            "finish before start",
        )

        initialization_failure = initialization_failure_document(
            "2026-09-09T00:00:00Z",
            "2026-09-09T00:00:01Z",
            candidate_cell=candidate["id"],
            repository_commit="1" * 40,
            catalogues=catalogue_hashes(
                catalogue_copy,
                matrix_copy,
                limitations_copy,
            ),
        )
        validate_initialization_failure(
            initialization_failure,
            expected_candidate_cell=candidate["id"],
            expected_repository_commit="1" * 40,
            expected_catalogues=catalogue_hashes(
                catalogue_copy,
                matrix_copy,
                limitations_copy,
            ),
        )
        initialization_failure_path = directory / "initialization-failure.json"
        atomic_write_json(initialization_failure_path, initialization_failure)
        validation_arguments = build_parser().parse_args(
            [
                "validate-evidence",
                "--evidence",
                str(initialization_failure_path),
                "--catalogue",
                str(catalogue_copy),
                "--matrix",
                str(matrix_copy),
                "--limitations",
                str(limitations_copy),
                "--schema",
                str(schema),
                "--candidate-cell",
                candidate["id"],
                "--expected-repository-commit",
                "1" * 40,
            ]
        )
        run_command(validation_arguments)
        unbound_initialization_failure = initialization_failure_document(
            "2026-09-09T00:00:00Z", "2026-09-09T00:00:01Z"
        )
        validate_initialization_failure(unbound_initialization_failure)
        for unreviewed_candidate in (
            "podman-private-rootless",
            "podman-password-rootless",
        ):
            unreviewed_initialization_failure = initialization_failure_document(
                "2026-09-09T00:00:00Z",
                "2026-09-09T00:00:01Z",
                candidate_cell=unreviewed_candidate,
            )
            assert (
                unreviewed_initialization_failure["invocation"]["candidate_cell"]
                is None
            )
            validate_initialization_failure(unreviewed_initialization_failure)
        expect_contract_error(
            lambda: validate_initialization_failure(
                unbound_initialization_failure,
                expected_candidate_cell="not-one-invocation",
            ),
            "invalid expected initialization failure candidate",
        )
        expect_contract_error(
            lambda: validate_initialization_failure(
                unbound_initialization_failure,
                expected_candidate_cell=candidate["id"],
                expected_repository_commit="1" * 40,
                expected_catalogues=catalogue_hashes(
                    catalogue_copy,
                    matrix_copy,
                    limitations_copy,
                ),
            ),
            "unbound initialization failure against valid invocation",
        )
        changed_initialization_candidate = copy.deepcopy(initialization_failure)
        changed_initialization_candidate["invocation"]["candidate_cell"] = candidates[1]["id"]
        expect_contract_error(
            lambda: validate_initialization_failure(
                changed_initialization_candidate,
                expected_candidate_cell=candidate["id"],
            ),
            "changed initialization failure candidate",
        )
        changed_initialization_commit = copy.deepcopy(initialization_failure)
        changed_initialization_commit["invocation"]["repository_commit"] = "2" * 40
        expect_contract_error(
            lambda: validate_initialization_failure(
                changed_initialization_commit,
                expected_repository_commit="1" * 40,
            ),
            "changed initialization failure commit",
        )
        changed_initialization_catalogue = copy.deepcopy(initialization_failure)
        changed_initialization_catalogue["invocation"]["catalogues"][
            "matrix_sha256"
        ] = "2" * 64
        expect_contract_error(
            lambda: validate_initialization_failure(
                changed_initialization_catalogue,
                expected_catalogues=catalogue_hashes(
                    catalogue_copy,
                    matrix_copy,
                    limitations_copy,
                ),
            ),
            "changed initialization failure catalogue hash",
        )

        duplicate_json = directory / "duplicate.json"
        duplicate_json.write_text('{"status":"failed","status":"passed"}', encoding="utf-8")
        expect_contract_error(lambda: load_json_object(duplicate_json), "duplicate JSON key")
        oversized_json = directory / "oversized.json"
        oversized_json.write_text("x" * (MAX_JSON_BYTES + 1), encoding="utf-8")
        expect_contract_error(lambda: load_json_object(oversized_json), "oversized JSON")

        evidence_path = directory / "atomic" / "evidence-v1.json"
        atomic_write_json(evidence_path, early_failure)
        assert (evidence_path.stat().st_mode & 0o777) == 0o600
        original_replace = os.replace

        def fail_replace(_source: Any, _target: Any) -> None:
            raise OSError("synthetic replace failure")

        os.replace = fail_replace
        try:
            try:
                atomic_write_json(evidence_path, early_failure)
            except OSError:
                pass
            else:
                raise AssertionError("synthetic atomic replacement unexpectedly passed")
        finally:
            os.replace = original_replace
        assert not list(evidence_path.parent.glob(f".{evidence_path.name}.*"))
    print("podman-revalidation hardened self-test: PASS")


def main() -> None:
    if sys.argv[1:] == ["--self-test"]:
        self_test()
        return
    parser = build_parser()
    try:
        run_command(parser.parse_args())
    except (ContractError, FileNotFoundError, json.JSONDecodeError, tomllib.TOMLDecodeError) as error:
        fail(str(error))


if __name__ == "__main__":
    main()
