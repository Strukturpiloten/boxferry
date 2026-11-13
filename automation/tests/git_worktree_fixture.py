"""Inert native-receipt controls, independent of image execution."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any

from boxferry_ci import git_worktree_validation as subject

HEAD = "f88fda6838d28966248fd8422d6fc920c6084f20"
CASES = {
    "primary": "normal/checkout",
    "relative-leaf": "leaf/relative",
    "relative-topology": "topology/relative",
    "legacy": "legacy/checkout",
    "standalone": "standalone/checkout",
}
REFUSALS = {"relative-leaf": "relative-metadata-unmounted", "legacy": "legacy-metadata-unmounted"}
STAGED = {
    "executable.sh": ("100755", b"#!/bin/sh\nexit 0\n"),
    "link": ("120000", b"payload.bin"),
    "payload.bin": ("100644", b"staged\0payload\n"),
    "new.txt": ("100644", b"new staged blob\n"),
    "working-deleted.txt": ("100644", b"working deletion\n"),
}
WORKING = {
    "executable.sh": ("file", 0o755, b"#!/bin/sh\nexit 0\n"),
    "link": ("symlink", 0o777, b"payload.bin"),
    "payload.bin": ("file", 0o644, b"working\0payload\n"),
    "new.txt": ("file", 0o644, b"new staged blob\n"),
    "untracked.txt": ("file", 0o644, b"untracked bytes\n"),
}


def expected_files() -> dict[str, dict[str, object]]:
    result: dict[str, dict[str, object]] = {
        name: {"kind": kind, "mode": mode, "sha256": hashlib.sha256(data).hexdigest()}
        for name, (kind, mode, data) in WORKING.items()
    }
    result["working-deleted.txt"] = {"kind": "deleted"}
    return result


def expected_index() -> bytes:
    result = []
    for name, (mode, data) in sorted(STAGED.items()):
        oid = hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()
        result.append(f"{mode} {oid} 0\t{name}\0".encode())
    return b"".join(result)


def fixtures(root: Path, package: Path) -> subject.Fixtures:
    root.mkdir()
    expected: dict[str, Any] = {
        "schema": 1,
        "spec_sha256": subject.SPEC_SHA256,
        "head": HEAD,
        "files": expected_files(),
        "index_sha256": hashlib.sha256(expected_index()).hexdigest(),
        "host_git": "git version 99.0.0",
        "expected_git_version": "99.0.1",
        "host_repository_formats": {"primary": 1, "relative": 1, "legacy": 0, "standalone": 0},
        "host_index_sha256": {
            name: hashlib.sha256(name.encode()).hexdigest()
            for name in ("primary", "relative", "legacy", "standalone")
        },
        "helper_sha256": {
            name: hashlib.sha256(name.encode()).hexdigest()
            for name in ("__init__.py", "git_worktree.py", "git_worktree_validation.py")
        },
        "candidate_helper_sha256": {
            "scripts/git-worktree.py": hashlib.sha256(b"caller").hexdigest()
        },
    }
    path = root.parent / "expected-fixture.json"
    path.write_text(json.dumps(expected))
    return subject.Fixtures(root, path, package, subject.fingerprint(root), expected)


def receipt(fixture: subject.Fixtures, machine: str = "x86_64") -> dict[str, Any]:
    values = {}
    expected = fixture.expected
    for name, location in CASES.items():
        refusal = REFUSALS.get(name)
        facts = None
        if refusal is None:
            original = "relative" if name == "relative-topology" else name
            root = Path("/workspaces/.boxferry-git-contract") / location
            gitdir = (
                Path("/workspaces/.boxferry-git-contract/topology/primary/.git/worktrees/relative")
                if original == "relative"
                else root / ".git"
            )
            files = expected_files()
            formats = expected["host_repository_formats"]
            indexes = expected["host_index_sha256"]
            assert isinstance(formats, dict) and isinstance(indexes, dict)
            facts = {
                "schema": 1,
                "source": str(root),
                "git_dir": str(gitdir),
                "index": str(gitdir / "index"),
                "git_version": "git version 99.0.1",
                "head": HEAD,
                "index_sha256": hashlib.sha256(expected_index()).hexdigest(),
                "raw_index_sha256": indexes[original],
                "repository_format": formats[original],
                "extensions": {"relativeworktrees": "true"}
                if original in {"primary", "relative"}
                else {},
                "files": files,
                "files_sha256": hashlib.sha256(
                    json.dumps(files, sort_keys=True).encode()
                ).hexdigest(),
                "bytes": sum(len(data) for _, _, data in WORKING.values()),
            }
        values[name] = {
            "exit": 1 if refusal else 0,
            "git_exit": 128 if refusal else 0,
            "category": refusal or "success",
            "candidate_cli_exit": 1 if refusal else 0,
            "inspection": facts,
        }
    return {
        "schema": 1,
        "spec_sha256": subject.SPEC_SHA256,
        "architecture": machine,
        "uid": 1000,
        "gid": 1000,
        "helper_sha256": expected["helper_sha256"],
        "candidate_helper_sha256": expected["candidate_helper_sha256"],
        "cases": values,
        "system_git": "git version 99.0.1",
    }
