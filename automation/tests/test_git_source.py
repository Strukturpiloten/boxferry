"""Independent synthetic Git source records; no fixture is an operational pin."""

from __future__ import annotations

import ast
import json
import unittest
from copy import deepcopy
from pathlib import Path
from typing import Any

from boxferry_ci import git_source as subject


def record() -> dict[str, Any]:
    return {
        "tool": "git",
        "provider": "kernel.org",
        "currentVersion": "2.51.7",
        "filename": "git-2.51.7.tar.xz",
        "url": "https://www.kernel.org/pub/software/scm/git/git-2.51.7.tar.xz",
        "sha256": "7" * 64,
    }


def signed(body: str) -> bytes:
    # Syntactic envelope only: the reader must never claim PGP verification.
    return (
        "-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA256\n\n"
        + body
        + "\n-----BEGIN PGP SIGNATURE-----\n\nZmFrZQ==\n=abcd\n"
        "-----END PGP SIGNATURE-----\n"
    ).encode()


class GitSources(unittest.TestCase):
    def test_pure_reader_has_no_acquisition_installer_or_persistence_imports(self) -> None:
        tree = ast.parse(Path(subject.__file__).read_text())
        imports = {node.module for node in ast.walk(tree) if isinstance(node, ast.ImportFrom)} | {
            alias.name
            for node in ast.walk(tree)
            if isinstance(node, ast.Import)
            for alias in node.names
        }
        self.assertEqual(imports, {"__future__", "json", "re", "dataclasses", "typing"})

    def test_independent_exact_source_and_checksum_identity(self) -> None:
        source = subject.source_record(record())
        self.assertEqual(
            (source.version, source.filename, source.sha256),
            ("2.51.7", "git-2.51.7.tar.xz", "7" * 64),
        )
        body = "\n".join(
            [
                "3" * 64 + "  git-manpages-2.51.7.tar.xz",
                "4" * 64 + "  git-2.51.7-rc1.tar.xz",
                "7" * 64 + "  git-2.51.7.tar.xz",
                "5" * 64 + "  git-2.51.7.tar.gz",
            ]
        )
        self.assertEqual(subject.match_checksums(source, signed(body)), "7" * 64)
        self.assertEqual(
            subject.match_checksums(source, signed(body).replace(b"\n", b"\r\n")), "7" * 64
        )

    def test_source_refuses_every_noncanonical_field_and_identity(self) -> None:
        for key, values in {
            "tool": ["boxferry", "Git", None],
            "provider": ["github", "www.kernel.org", None],
            "currentVersion": ["v2.51.7", "2.51.7-rc1", "2.51", "2.51.7.1", "02.51.7", 2],
            "filename": ["git-2.51.8.tar.xz", "git-2.51.7.tar.gz", "../git-2.51.7.tar.xz"],
            "sha256": ["7" * 63, "G" * 64, "A" * 64, None],
            "url": [
                "http://www.kernel.org/pub/software/scm/git/git-2.51.7.tar.xz",
                "https://kernel.org/pub/software/scm/git/git-2.51.7.tar.xz",
                "https://www.kernel.org:443/pub/software/scm/git/git-2.51.7.tar.xz",
                "https://www.kernel.org/pub/software/scm/git/git-2.51.7.tar.xz?q=1",
                "https://user@www.kernel.org/pub/software/scm/git/git-2.51.7.tar.xz",
                "https://www.kernel.org/pub/software/scm/git/git-2.51.7.tar.xz#fragment",
                "https://evil.invalid/git-2.51.7.tar.xz",
            ],
        }.items():
            for value in values:
                row = record() | {key: value}
                with self.subTest(key=key, value=value), self.assertRaises(subject.SourceError):
                    subject.source_record(row)
        for row in (
            None,
            {},
            record() | {"architecture": "x86_64"},
            {key: value for key, value in record().items() if key != "url"},
        ):
            with self.subTest(row=row), self.assertRaises(subject.SourceError):
                subject.source_record(row)

    def test_metadata_refuses_missing_duplicate_conflicting_malformed_and_truncated(self) -> None:
        source = subject.source_record(record())
        row = "7" * 64 + "  git-2.51.7.tar.xz"
        cases = [
            signed("8" * 64 + "  git-2.51.7.tar.xz"),
            signed("7" * 64 + "  git-2.51.8.tar.xz"),
            signed(row + "\n" + row),
            signed(row + "\n" + "8" * 64 + "  git-2.51.7.tar.xz"),
            signed(row.replace("7", "z", 1)),
            signed(row + " trailing"),
            signed(row + "\nmalformed"),
            signed(row).split(b"-----BEGIN PGP SIGNATURE-----")[0],
            signed(row)[:-20],
            signed(row) + b"extra",
            row.encode(),
            b"\xff",
            b"x" * (2 * 1024 * 1024 + 1),
        ]
        for index, value in enumerate(cases):
            with self.subTest(index=index), self.assertRaises(subject.SourceError):
                subject.match_checksums(source, value)

    def test_versioned_envelope_is_strict_without_architecture_source_rows(self) -> None:
        legacy = {"schema": 1, "provenance": {}, "assets": []}
        self.assertEqual(subject.inventory(json.dumps(legacy).encode()), (legacy, ()))
        future: dict[str, Any] = legacy | {"schema": 2, "sources": []}
        self.assertEqual(subject.inventory(json.dumps(future).encode()), (future, ()))
        future["sources"] = [record()]
        self.assertEqual(
            subject.inventory(json.dumps(future).encode())[1], (subject.source_record(record()),)
        )
        for document in (
            legacy | {"sources": []},
            future | {"schema": 3},
            future | {"schema": True},
            future | {"sources": [record(), record()]},
            future | {"sources": {}},
            future | {"unknown": 1},
            future | {"assets": {}},
            {key: value for key, value in future.items() if key != "sources"},
        ):
            with self.subTest(document=document), self.assertRaises(subject.SourceError):
                subject.inventory(json.dumps(document).encode())
        for raw in (
            b'{"schema":1,"schema":1}',
            b"{",
            b"\xff",
            b" " * (32 * 1024 + 1),
            b"[" * 1500 + b"]" * 1500,
        ):
            with self.subTest(size=len(raw)), self.assertRaises(subject.SourceError):
                subject.inventory(raw)
        duplicate_row = json.dumps(future).replace('"tool": "git"', '"tool":"git","tool":"git"')
        with self.assertRaises(subject.SourceError):
            subject.inventory(duplicate_row.encode())
        changed = deepcopy(future)
        changed["sources"][0]["sha256"] = "8" * 64
        self.assertNotEqual(
            subject.inventory(json.dumps(changed).encode())[1],
            subject.inventory(json.dumps(future).encode())[1],
        )
