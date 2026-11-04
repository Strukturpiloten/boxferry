"""Verify exact candidate Compose provider pins against inert release-asset bytes.

Hosted callers read only immutable Git objects. Local complete gates explicitly select
working-tree mode. Neither mode sources the installer, writes the artifact, or executes it.
"""

from __future__ import annotations

import argparse
import os
import re
import shlex
import stat
import subprocess
import sys
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

from boxferry_ci.kubernetes_tool_checksum_refresh import (
    RefreshError,
    fetch_artifact_sha256,
)

PROVIDER = "scripts/lib/compose-provider.sh"
APPLICATIONS = (
    "forgejo",
    "immich",
    "nextcloud",
    "observability",
    "paperless-ngx",
    "supabase",
)
CATALOGUES = tuple(f"fixtures/conformance/{app}-application/providers.tsv" for app in APPLICATIONS)
HOSTS = frozenset(
    {
        "github.com",
        "release-assets.githubusercontent.com",
        "objects.githubusercontent.com",
    }
)
MAX_SOURCE = 128 * 1024
MAX_ARTIFACT = 128 * 1024 * 1024


class ProviderError(ValueError):
    """Invalid candidate provider metadata or artifact integrity."""


@dataclass(frozen=True)
class Pin:
    release: str
    digest: str

    @property
    def version(self) -> str:
        return self.release.removeprefix("v")

    @property
    def url(self) -> str:
        return (
            f"https://github.com/docker/compose/releases/download/{self.release}/"
            "docker-compose-linux-x86_64"
        )


def parse_provider(source: str) -> Pin:
    """Read the finite literal pin without evaluating candidate shell code."""
    values: list[str] = []
    for name, pattern in (
        ("release", r"v[0-9]+\.[0-9]+\.[0-9]+"),
        ("sha256", r"[a-f0-9]{64}"),
    ):
        matches = re.findall(rf'^  local -r expected_{name}="({pattern})"$', source, re.MULTILINE)
        if len(matches) != 1:
            raise ProviderError(f"canonical provider must contain one literal expected_{name}")
        values.append(matches[0])
    # A prior differently quoted readonly local can silently win at runtime. Require
    # exactly the finite literal declarations, and reject every extra assignment.
    expected = {
        "expected_release": f'  local -r expected_release="{values[0]}"',
        "expected_sha256": f'  local -r expected_sha256="{values[1]}"',
        "expected_version": '  local -r expected_version="${expected_release#v}"',
        "expected_url": (
            '  local -r expected_url="https://github.com/docker/compose/releases/download/'
            'v${expected_version}/docker-compose-linux-x86_64"'
        ),
    }
    declarations: dict[str, list[str]] = {name: [] for name in expected}
    for line in source.splitlines():
        if not re.match(r"^\s*(?:local|readonly|declare|typeset|export)\b", line):
            continue
        try:
            tokens = shlex.split(line, comments=True)
        except ValueError as error:
            raise ProviderError(
                "canonical metadata declaration is not a finite shell literal"
            ) from error
        for token in tokens[1:]:
            protected = re.match(r"^(expected_(?:release|sha256|version|url))\b", token)
            if protected is not None:
                declarations[protected.group(1)].append(line)
    for name, line in expected.items():
        assignments = re.findall(rf"\b{re.escape(name)}(?:\[[^\]\n]*\])?[ \t]*\+?=", source)
        if declarations[name] != [line] or len(assignments) != 1:
            raise ProviderError(f"canonical {name} must have one exact declaration and assignment")

    return Pin(*values)


def validate_catalogue(source: str, path: str, pin: Pin) -> None:
    rows = [line.split("\t") for line in source.splitlines() if line.startswith("docker-compose\t")]
    expected = (
        ["docker-compose", pin.version, "linux-amd64", pin.digest, pin.url]
        if "supabase-application" in path
        else ["docker-compose", pin.version, pin.url, pin.digest]
    )
    if len(rows) != 1 or rows[0][: len(expected)] != expected:
        raise ProviderError(
            f"{path}: provider version, URL and digest differ from the canonical pin"
        )


def read_source(root: Path, path: str, *, candidate_sha: str | None) -> str:
    """Read bounded UTF-8 from a commit, or explicitly authorized local worktree."""
    if candidate_sha is not None:
        if not re.fullmatch(r"[a-f0-9]{40}", candidate_sha):
            raise ProviderError("candidate identity must be an exact lowercase commit SHA")
        try:
            entry = subprocess.run(
                ["git", "--no-replace-objects", "ls-tree", "-z", candidate_sha, "--", path],
                cwd=root,
                capture_output=True,
                check=True,
                timeout=10,
            ).stdout
            record, separator, trailing = entry.partition(b"\0")
            metadata, tab, entry_path = record.partition(b"\t")
            fields = metadata.split()
            if (
                separator != b"\0"
                or trailing
                or tab != b"\t"
                or entry_path != path.encode()
                or len(fields) != 3
                or fields[0] not in {b"100644", b"100755"}
                or fields[1] != b"blob"
                or re.fullmatch(rb"[a-f0-9]{40}", fields[2]) is None
            ):
                raise ProviderError(f"{path}: candidate source must be one regular Git blob")
            size = subprocess.run(
                [
                    "git",
                    "--no-replace-objects",
                    "cat-file",
                    "-s",
                    f"{candidate_sha}:{path}",
                ],
                cwd=root,
                capture_output=True,
                check=True,
                timeout=10,
            ).stdout
            if not size.strip().isdigit() or not 0 < int(size) <= MAX_SOURCE:
                raise ProviderError(f"{path}: candidate source exceeds its size budget")
            raw = subprocess.run(
                ["git", "--no-replace-objects", "show", f"{candidate_sha}:{path}"],
                cwd=root,
                capture_output=True,
                check=True,
                timeout=10,
            ).stdout
        except (OSError, subprocess.SubprocessError) as error:
            raise ProviderError(f"{path}: cannot read exact candidate Git object") from error
    else:
        source = root / path
        if any(part.is_symlink() for part in (source, *source.parents)):
            raise ProviderError(f"{path}: working-tree source must not follow symbolic links")
        try:
            descriptor = os.open(source, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
            with os.fdopen(descriptor, "rb") as incoming:
                if not stat.S_ISREG(os.fstat(incoming.fileno()).st_mode):
                    raise ProviderError(f"{path}: working-tree source must be a regular file")
                raw = incoming.read(MAX_SOURCE + 1)
        except OSError as error:
            raise ProviderError(f"{path}: cannot read working-tree source") from error
    if not 0 < len(raw) <= MAX_SOURCE:
        raise ProviderError(f"{path}: candidate source exceeds its size budget")
    try:
        return raw.decode("utf-8")
    except UnicodeDecodeError as error:
        raise ProviderError(f"{path}: source must be UTF-8") from error


def verify(
    root: Path,
    *,
    candidate_sha: str | None = None,
    verify_working_tree: bool = False,
    artifact_fetcher: Callable[..., str] = fetch_artifact_sha256,
) -> Pin:
    if (candidate_sha is None) == (not verify_working_tree):
        raise ProviderError(
            "select exactly one immutable candidate SHA or explicit working-tree mode"
        )
    if candidate_sha is not None:
        if not re.fullmatch(r"[a-f0-9]{40}", candidate_sha):
            raise ProviderError("candidate identity must be an exact lowercase commit SHA")
        try:
            kind = subprocess.run(
                ["git", "--no-replace-objects", "cat-file", "-t", candidate_sha],
                cwd=root,
                capture_output=True,
                check=True,
                timeout=10,
            ).stdout
        except (OSError, subprocess.SubprocessError) as error:
            raise ProviderError("candidate commit object is unavailable") from error
        if kind.strip() != b"commit":
            raise ProviderError("candidate identity must name a commit object")
    pin = parse_provider(read_source(root, PROVIDER, candidate_sha=candidate_sha))
    for path in CATALOGUES:
        validate_catalogue(read_source(root, path, candidate_sha=candidate_sha), path, pin)
    actual = artifact_fetcher(pin.url, allowed_hosts=set(HOSTS), max_bytes=MAX_ARTIFACT)
    if actual != pin.digest:
        raise ProviderError(
            f"Compose {pin.release}: downloaded artifact SHA-256 differs from candidate pin"
        )
    return pin


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository-root", required=True, type=Path)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--candidate-sha")
    mode.add_argument("--verify-working-tree", action="store_true")
    options = parser.parse_args(argv)
    try:
        pin = verify(
            options.repository_root,
            candidate_sha=options.candidate_sha,
            verify_working_tree=options.verify_working_tree,
        )
    except (ProviderError, RefreshError) as error:
        print(f"Compose provider integrity failed: {error}", file=sys.stderr)
        return 1
    print(
        f"Compose {pin.release}: canonical pin and six catalogues match verified artifact SHA-256."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
