"""Pure readers for one canonical Git source and its official checksum metadata.

The clear-signed envelope is parsed for completeness, never PGP-verified. Matching
its checksum proves published metadata identity, not downloaded archive bytes or
a release date. There is deliberately no acquisition, installer or state here.
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from typing import Any

INVENTORY = "automation/tool-artifacts.json"
ORIGIN = "https://www.kernel.org/pub/software/scm/git/"
CHECKSUMS = ORIGIN + "sha256sums.asc"
MAX_INVENTORY_BYTES = 32 * 1024
MAX_CHECKSUM_BYTES = 2 * 1024 * 1024


class SourceError(ValueError):
    """Missing, unsupported or inconsistent canonical source evidence."""


@dataclass(frozen=True)
class GitSource:
    version: str
    filename: str
    url: str
    sha256: str


def source_record(row: object) -> GitSource:
    """Accept only the single architecture-independent stable Git representation."""
    if not isinstance(row, dict) or set(row) != {
        "tool",
        "provider",
        "currentVersion",
        "filename",
        "url",
        "sha256",
    }:
        raise SourceError("Git source row has missing or unknown fields")
    version = row["currentVersion"]
    if (
        row["tool"] != "git"
        or row["provider"] != "kernel.org"
        or not isinstance(version, str)
        or re.fullmatch(r"[1-9][0-9]*\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)", version) is None
    ):
        raise SourceError("Git source requires the canonical provider and exact stable version")
    filename = f"git-{version}.tar.xz"
    if row["filename"] != filename or row["url"] != ORIGIN + filename:
        raise SourceError("Git source URL/filename differs from its canonical stable version")
    digest = row["sha256"]
    if not isinstance(digest, str) or re.fullmatch(r"[a-f0-9]{64}", digest) is None:
        raise SourceError("Git source requires exact lowercase SHA-256 integrity")
    return GitSource(version, filename, ORIGIN + filename, digest)


def _unique(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise SourceError("canonical tool metadata repeats a key")
        value[key] = item
    return value


def inventory(raw: bytes) -> tuple[dict[str, Any], tuple[GitSource, ...]]:
    """Read legacy schema 1 or schema 2 with zero or one Git source row.

    Binary asset validation remains with its existing reader; neither schema
    allows source fields to be silently added to the legacy representation.
    """
    if not isinstance(raw, bytes) or len(raw) > MAX_INVENTORY_BYTES:
        raise SourceError("canonical tool inventory exceeds its byte bound")
    try:
        document = json.loads(raw, object_pairs_hook=_unique)
    except (UnicodeDecodeError, json.JSONDecodeError, RecursionError) as error:
        raise SourceError("canonical tool inventory is not complete UTF-8 JSON") from error
    if not isinstance(document, dict) or type(document.get("schema")) is not int:
        raise SourceError("canonical tool inventory requires a supported integer schema")
    schema = document["schema"]
    keys = {"schema", "provenance", "assets"} | ({"sources"} if schema == 2 else set())
    if (
        schema not in {1, 2}
        or set(document) != keys
        or not isinstance(document["provenance"], dict)
        or not isinstance(document["assets"], list)
    ):
        raise SourceError("canonical tool inventory has unsupported schema or fields")
    rows = document.get("sources", [])
    if not isinstance(rows, list) or len(rows) > 1:
        raise SourceError("canonical tool inventory permits at most one Git source")
    return document, tuple(source_record(row) for row in rows)


def match_checksums(source: GitSource, raw: bytes) -> str:
    """Require one exact official checksum row before an age observation may exist."""
    # Revalidate even typed values: callers cannot construct an alternate origin.
    source_record(
        {
            "tool": "git",
            "provider": "kernel.org",
            "currentVersion": source.version,
            "filename": source.filename,
            "url": source.url,
            "sha256": source.sha256,
        }
    )
    if not isinstance(raw, bytes) or len(raw) > MAX_CHECKSUM_BYTES:
        raise SourceError("Git checksum metadata exceeds its byte bound")
    try:
        text = raw.decode("ascii").replace("\r\n", "\n")
    except UnicodeDecodeError as error:
        raise SourceError("Git checksum metadata is not ASCII") from error
    envelope = re.fullmatch(
        r"-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA256\n\n"
        r"(?P<body>.+)\n-----BEGIN PGP SIGNATURE-----\n\n"
        r"[A-Za-z0-9+/=\n]+\n-----END PGP SIGNATURE-----\n?",
        text,
        re.DOTALL,
    )
    if envelope is None:
        raise SourceError("Git checksum metadata has a malformed or incomplete signed envelope")
    selected: list[str] = []
    for line in envelope["body"].split("\n"):
        match = re.fullmatch(r"([a-f0-9]{64}) {1,2}\*?([A-Za-z0-9._+-]+)", line)
        if match is None:
            raise SourceError("Git checksum metadata contains a malformed checksum row")
        if match[2] == source.filename:
            selected.append(match[1])
    if len(selected) != 1:
        raise SourceError("Git checksum metadata requires exactly one selected source row")
    if selected[0] != source.sha256:
        raise SourceError("Git source checksum differs from official checksum metadata")
    return selected[0]
