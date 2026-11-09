"""Acquire inert dependency snapshots using only caller-supplied read capabilities.

No client, credential, state, registry acquisition or policy result crosses this
boundary. The caller owns the empty output directory and the run-wide budget;
base and tested snapshots share the existing immutable-blob cache.
"""

from __future__ import annotations

import re
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .operational import MAX_FILE_BYTES, SnapshotCache, inventory_blobs, snapshot_paths

ReadJSON = Callable[[str], dict[str, Any]]
ReadRegularFile = Callable[[str, str], bytes | None]


class AcquisitionError(ValueError):
    """Unavailable or inconsistent immutable snapshot evidence."""


@dataclass(frozen=True)
class Materialization:
    """Files actually materialized, without an eligibility or policy decision."""

    paths: tuple[str, ...]
    bytes_written: int


def _revision(value: str) -> None:
    if re.fullmatch(r"[a-f0-9]{40}", value) is None:
        raise AcquisitionError("candidate snapshot requires an exact immutable revision")


def _path(value: object) -> str:
    if (
        not isinstance(value, str)
        or not value
        or value.startswith("/")
        or any(part in {"", ".", ".."} for part in value.split("/"))
        or "\\" in value
        or "\0" in value
    ):
        raise AcquisitionError("candidate metadata contains an unsafe repository path")
    return value


def changed_paths(base: str, tested: str, read_json: ReadJSON) -> tuple[str, ...]:
    """Bind a complete comparison to the independently trusted base and tested SHA."""
    _revision(base)
    _revision(tested)
    comparison = read_json(f"compare/{base}...{tested}?per_page=100")
    merge_base = comparison.get("merge_base_commit")
    if not isinstance(merge_base, dict) or merge_base.get("sha") != base:
        raise AcquisitionError("candidate comparison does not bind the exact expected base")
    files = comparison.get("files")
    if not isinstance(files, list) or len(files) >= 300:
        raise AcquisitionError("candidate changed-file metadata is missing or may be truncated")
    paths: set[str] = set()
    for item in files:
        if not isinstance(item, dict):
            raise AcquisitionError("candidate changed-file metadata is malformed")
        paths.add(_path(item.get("filename")))
        if "previous_filename" in item:
            paths.add(_path(item["previous_filename"]))
    return tuple(sorted(paths))


def materialize(
    head: str,
    destination: Path,
    paths: Sequence[str],
    *,
    read_json: ReadJSON,
    read_regular_file: ReadRegularFile,
    cache: SnapshotCache,
    lock_paths: Sequence[str],
    max_bytes: int,
) -> Materialization:
    """Read bounded regular metadata at one exact revision; never execute its bytes."""
    _revision(head)
    validated = tuple(_path(path) for path in paths)
    manifests = {
        path
        for path in validated
        if path == "Cargo.toml" or re.fullmatch(r"crates/[^/]+/Cargo.toml", path)
    }
    operational = snapshot_paths(validated)
    blobs: dict[str, str] = {}
    if operational:
        # Complete inventory preserves reused identities outside the changed paths.
        blobs = inventory_blobs(read_json(f"git/trees/{head}?recursive=1"))
        operational.update(snapshot_paths(list(blobs)))
    pending: dict[str, bytes] = {}
    total = 0
    selected = {_path(path) for path in lock_paths} | manifests | {"Cargo.toml"} | operational
    for path in sorted(selected):
        sha = blobs.get(path)
        if path in operational and sha is None:
            continue
        data = cache.get(sha) if sha is not None else None
        if data is None:
            data = read_regular_file(path, head)
            if data is None:
                if sha is not None:
                    raise AcquisitionError(
                        "owned operational blob is missing from its exact snapshot"
                    )
                continue
            if not isinstance(data, bytes) or len(data) > MAX_FILE_BYTES:
                raise AcquisitionError("candidate metadata file is not a bounded regular file")
            if sha is not None:
                cache.put(sha, data)
        total += len(data)
        if total > max_bytes:
            raise AcquisitionError("candidate dependency snapshot exceeds its size budget")
        pending[path] = data
    # Validate every output before writing, including preexisting local symlinks.
    if destination.is_symlink() or not destination.is_dir():
        raise AcquisitionError("dependency snapshot requires an owned regular directory")
    for path in pending:
        target = destination / path
        if target.exists() or target.is_symlink():
            raise AcquisitionError("dependency snapshot output already exists")
        for parent in target.parents:
            if parent == destination:
                break
            if parent.is_symlink() or (parent.exists() and not parent.is_dir()):
                raise AcquisitionError("dependency snapshot output contains an unsafe parent")
    for path, data in pending.items():
        target = destination / path
        target.parent.mkdir(parents=True, exist_ok=True)
        with target.open("xb") as output:
            output.write(data)
    return Materialization(tuple(pending), total)
