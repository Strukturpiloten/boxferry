"""Authored inert metadata fixtures for the read-capability acquisition boundary."""

from __future__ import annotations

import ast
import hashlib
import tempfile
import unittest
from pathlib import Path
from typing import Any
from unittest.mock import patch

from boxferry_ci import dependency_snapshots as subject
from boxferry_ci import eligibility
from boxferry_ci.operational import OperationalError, SnapshotCache

BASE, TESTED = "a" * 40, "b" * 40
OLD, NEW = ".github/workflows/old.yml", ".github/workflows/new.yml"
BODY = b"---\njobs: {check: {runs-on: ubuntu-24.04}}\n"
SHA = hashlib.sha1(
    b"blob " + str(len(BODY)).encode() + b"\0" + BODY, usedforsecurity=False
).hexdigest()


class Reads:
    """Only reads: no client, credentials, state, effects or second request budget."""

    def __init__(self) -> None:
        self.calls: list[str] = []
        self.files = {
            BASE: {OLD: BODY, "Cargo.toml": b"[package]\nname='same'\n"},
            TESTED: {NEW: BODY, "Cargo.toml": b"[package]\nname='same'\n"},
        }

    def json(self, suffix: str) -> dict[str, Any]:
        self.calls.append(suffix)
        if suffix == f"compare/{BASE}...{TESTED}?per_page=100":
            return {
                "merge_base_commit": {"sha": BASE},
                "files": [{"filename": NEW, "previous_filename": OLD}],
            }
        revision = suffix.split("/", 2)[2].split("?", 1)[0]
        return {
            "truncated": False,
            "tree": [
                {"path": path, "type": "blob", "mode": "100755", "sha": SHA}
                for path in self.files[revision]
                if path.endswith(".yml")
            ],
        }

    def regular(self, path: str, revision: str) -> bytes | None:
        self.calls.append(f"file/{revision}/{path}")
        return self.files[revision].get(path)


class DependencySnapshots(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.reads = Reads()
        self.cache = SnapshotCache()

    def materialize(
        self, revision: str = BASE, paths: tuple[str, ...] = (OLD, NEW), *, max_bytes: int = 1024
    ) -> subject.Materialization:
        root = self.root / revision
        root.mkdir(exist_ok=True)
        return subject.materialize(
            revision,
            root,
            paths,
            read_json=self.reads.json,
            read_regular_file=self.reads.regular,
            cache=self.cache,
            lock_paths=("Cargo.lock",),
            max_bytes=max_bytes,
        )

    def test_exact_comparison_rename_and_shared_blob_materialize_only_inert_bytes(self) -> None:
        paths = subject.changed_paths(BASE, TESTED, self.reads.json)
        self.assertEqual(paths, (NEW, OLD))
        with patch("subprocess.Popen", side_effect=AssertionError("candidate execution")):
            base = self.materialize(BASE, paths)
            tested = self.materialize(TESTED, paths)
        self.assertEqual(base.paths, (OLD, "Cargo.toml"))
        self.assertEqual(tested.paths, (NEW, "Cargo.toml"))
        for revision, workflow in ((BASE, OLD), (TESTED, NEW)):
            self.assertEqual((self.root / revision / workflow).read_bytes(), BODY)
            self.assertEqual(
                (self.root / revision / "Cargo.toml").read_bytes(), b"[package]\nname='same'\n"
            )
            self.assertFalse((self.root / revision / workflow).stat().st_mode & 0o111)
        self.assertEqual(base.bytes_written, len(BODY) + len(b"[package]\nname='same'\n"))
        self.assertEqual(self.cache.contents, {SHA: BODY})
        self.assertEqual(
            sum(call.startswith("file/") and call.endswith(".yml") for call in self.reads.calls), 1
        )
        self.assertIn(f"git/trees/{BASE}?recursive=1", self.reads.calls)
        self.assertIn(f"git/trees/{TESTED}?recursive=1", self.reads.calls)

    def test_comparison_refuses_truncation_wrong_merge_base_and_unsafe_rename(self) -> None:
        cases = [
            {"merge_base_commit": {"sha": BASE}, "files": [{}] * 300},
            {"merge_base_commit": {"sha": TESTED}, "files": []},
            {"merge_base_commit": {"sha": BASE}},
            {"merge_base_commit": [], "files": []},
        ]
        for row in (
            {},
            {"filename": None},
            {"filename": ""},
            {"previous_filename": OLD},
            {"filename": NEW, "previous_filename": None},
            {"filename": NEW, "previous_filename": ""},
        ):
            cases.append({"merge_base_commit": {"sha": BASE}, "files": [row]})
        for key in ("filename", "previous_filename"):
            for path in ("../outside", "/outside", "crates/../Cargo.toml", "a\\b", "a/./b"):
                cases.append(
                    {"merge_base_commit": {"sha": BASE}, "files": [{"filename": NEW, key: path}]}
                )
        for document in cases:
            with (
                self.subTest(document=document),
                patch.object(self.reads, "json", return_value=document),
                self.assertRaises(subject.AcquisitionError),
            ):
                subject.changed_paths(BASE, TESTED, self.reads.json)
        with self.assertRaises(subject.AcquisitionError):
            subject.changed_paths("main", TESTED, self.reads.json)
        self.assertEqual(self.reads.calls, [])

    def test_missing_mismatched_oversize_and_nonregular_promised_blobs_refuse(self) -> None:
        for body in (None, b"tampered", b"x" * (2 * 1024 * 1024 + 1)):
            with self.subTest(body_size=None if body is None else len(body)):
                self.reads.files[BASE] = {} if body is None else {OLD: body}
                with (
                    patch.object(
                        self.reads,
                        "json",
                        return_value={
                            "truncated": False,
                            "tree": [{"path": OLD, "type": "blob", "mode": "100644", "sha": SHA}],
                        },
                    ),
                    self.assertRaises(ValueError),
                ):
                    self.materialize()
                self.assertEqual(list((self.root / BASE).iterdir()), [])
        for mode in ("120000", "160000"):
            with (
                patch.object(
                    self.reads,
                    "json",
                    return_value={
                        "truncated": False,
                        "tree": [{"path": OLD, "type": "blob", "mode": mode, "sha": SHA}],
                    },
                ),
                self.assertRaises(OperationalError),
            ):
                self.materialize()
        with patch.object(self.reads, "json", return_value={"truncated": True, "tree": []}):
            with self.assertRaises(OperationalError):
                self.materialize()

    def test_budget_and_output_parents_fail_before_any_materialization(self) -> None:
        with self.assertRaisesRegex(subject.AcquisitionError, "size budget"):
            self.materialize(max_bytes=1)
        self.assertEqual(list((self.root / BASE).iterdir()), [])
        outside = self.root / "outside"
        outside.mkdir()
        (self.root / BASE / ".github").symlink_to(outside, target_is_directory=True)
        with self.assertRaisesRegex(subject.AcquisitionError, "unsafe parent"):
            self.materialize()
        self.assertEqual(list(outside.iterdir()), [])
        with self.assertRaises(subject.AcquisitionError):
            self.materialize(paths=("../Cargo.toml",))

    def test_wrapper_preserves_error_category_and_existing_run_budget(self) -> None:
        api = eligibility.GitHub("Strukturpiloten/boxferry", "unused")
        candidate = eligibility.Candidate(BASE, TESTED, 1, "c" * 40, "user", "User", "pull_request")
        with patch.object(api, "get", return_value={"files": []}):
            with self.assertRaisesRegex(eligibility.EligibilityError, "exact expected base"):
                eligibility.changed_paths(api, candidate)
        with patch.object(api, "get", side_effect=eligibility.EligibilityError("budget exhausted")):
            with self.assertRaisesRegex(eligibility.EligibilityError, "budget exhausted"):
                eligibility.changed_paths(api, candidate)
        self.assertEqual(api.requests, 0)

    def test_snapshot_wrapper_maps_acquisition_failure_without_changing_client_identity(
        self,
    ) -> None:
        api = eligibility.GitHub("Strukturpiloten/boxferry", "unused")
        root = self.root / "wrapper"
        root.mkdir()
        with (
            patch.object(api, "get", side_effect=self.reads.json),
            patch.object(eligibility, "file_bytes", return_value=BODY),
            patch.object(eligibility, "MAX_SNAPSHOT_BYTES", 1),
        ):
            with self.assertRaisesRegex(eligibility.EligibilityError, "size budget"):
                eligibility.snapshot(api, BASE, root, [OLD], self.cache)
        self.assertEqual(list(root.iterdir()), [])

    def test_acquisition_module_has_no_service_or_execution_import(self) -> None:
        tree = ast.parse(Path(subject.__file__).read_text())
        imports = {node.module for node in ast.walk(tree) if isinstance(node, ast.ImportFrom)}
        self.assertEqual(
            imports,
            {"__future__", "collections.abc", "dataclasses", "pathlib", "typing", "operational"},
        )
        self.assertEqual(
            {
                alias.name
                for node in ast.walk(tree)
                if isinstance(node, ast.Import)
                for alias in node.names
            },
            {"re"},
        )


if __name__ == "__main__":
    unittest.main()
