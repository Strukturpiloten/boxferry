"""End-to-end inert Git/API snapshot tests; Git and HTTP transports are synthetic."""

from __future__ import annotations

import base64
import contextlib
import hashlib
import io
import json
import tarfile
import tempfile
import unittest
from collections.abc import Callable, Mapping
from copy import deepcopy
from datetime import UTC, datetime
from pathlib import Path
from typing import Any
from unittest.mock import patch

from boxferry_ci import __main__ as cli
from boxferry_ci import dependencies, eligibility
from boxferry_ci.operational import OperationalError, SnapshotCache

BASE, HEAD = "a" * 40, "b" * 40
REPOSITORY = "Strukturpiloten/boxferry"
NOW = datetime(2026, 11, 3, 12, tzinfo=UTC)
OLD = ".github/workflows/old.yml"
NEW = ".github/workflows/new.yml"
FEATURE = ".devcontainer/devcontainer.json"
LOCK = ".devcontainer/devcontainer-lock.json"
IMAGE = "automation/renovate-runtime.json"
MANIFEST = json.dumps(
    {
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {
            "mediaType": "application/vnd.oci.image.config.v1+json",
            "size": 1,
            "digest": "sha256:" + "d" * 64,
        },
        "layers": [],
    },
    separators=(",", ":"),
).encode()
DIGEST = "sha256:" + hashlib.sha256(MANIFEST).hexdigest()


def blob_sha(data: bytes) -> str:
    return hashlib.sha1(
        b"blob " + str(len(data)).encode() + b"\0" + data, usedforsecurity=False
    ).hexdigest()


class SnapshotAPI(eligibility.GitHub):
    def __init__(self, files: dict[str, dict[str, bytes]]) -> None:
        super().__init__(REPOSITORY, "synthetic-token")
        self.files = files
        self.seen: list[str] = []
        self.truncated = False
        self.symlink = False
        self.omit = False

    def get(self, suffix: str) -> dict[str, Any]:
        self.requests += 1
        self.seen.append(suffix)
        if suffix.startswith("compare/"):
            changes = [{"filename": NEW, "previous_filename": OLD}]
            for name, data in self.files[HEAD].items():
                if name != NEW and self.files[BASE].get(name) != data:
                    changes.append({"filename": name})
            return {"merge_base_commit": {"sha": BASE}, "files": changes}
        if suffix.startswith("git/trees/"):
            revision = suffix.split("/", 2)[2].split("?", 1)[0]
            entries = [
                {
                    "path": name,
                    "mode": "120000" if self.symlink and name == IMAGE else "100644",
                    "type": "blob",
                    "sha": blob_sha(data),
                }
                for name, data in self.files[revision].items()
            ]
            return {"tree": entries, "truncated": self.truncated}
        if suffix.startswith("contents/"):
            name, revision = suffix.removeprefix("contents/").split("?ref=", 1)
            data = self.files[revision].get(name)
            if self.omit and name == IMAGE:
                raise eligibility.MissingResource(suffix)
            if data is not None:
                return {
                    "type": "file",
                    "size": len(data),
                    "encoding": "base64",
                    "content": base64.b64encode(data).decode(),
                }
        raise eligibility.MissingResource(suffix)


class OperationalSnapshots(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.repository = self.root / "candidate"
        self.repository.mkdir()
        metadata = {"id": "github-cli", "version": "1.1.0"}
        encoded_metadata = json.dumps(metadata).encode()
        archive = io.BytesIO()
        with tarfile.open(fileobj=archive, mode="w") as stream:
            member = tarfile.TarInfo("devcontainer-feature.json")
            member.size = len(encoded_metadata)
            stream.addfile(member, io.BytesIO(encoded_metadata))
        config, layer = b"{}", archive.getvalue()

        def digest(content: bytes) -> str:
            return "sha256:" + hashlib.sha256(content).hexdigest()

        manifest = json.dumps(
            {
                "schemaVersion": 2,
                "mediaType": "application/vnd.oci.image.manifest.v1+json",
                "config": {
                    "mediaType": "application/vnd.devcontainers",
                    "size": len(config),
                    "digest": digest(config),
                },
                "layers": [
                    {
                        "mediaType": "application/vnd.devcontainers.layer.v1+tar",
                        "size": len(layer),
                        "digest": digest(layer),
                    }
                ],
                "annotations": {"dev.containers.metadata": json.dumps(metadata)},
            }
        ).encode()
        self.feature_blobs = {digest(body): body for body in (manifest, config, layer)}
        feature_digest = digest(manifest)
        feature = "ghcr.io/devcontainers/features/github-cli:1.1.0"
        declared = json.dumps({"features": {feature: {"version": "2.103.2"}}}).encode()
        lock = json.dumps(
            {
                "features": {
                    feature: {
                        "version": "1.1.0",
                        "resolved": "ghcr.io/devcontainers/features/github-cli@" + feature_digest,
                        "integrity": feature_digest,
                    }
                }
            }
        ).encode()
        workflow = b"---\njobs:\n  check:\n    runs-on: ubuntu-24.04\n"
        self.files = {
            BASE: {
                OLD: workflow,
                FEATURE: declared,
                LOCK: lock,
                "scripts/exploit.py": b"raise RuntimeError('never execute candidate')\n",
            }
        }
        self.files[HEAD] = deepcopy(self.files[BASE])
        del self.files[HEAD][OLD]
        self.files[HEAD][NEW] = workflow.replace(b"24.04", b"26.04")
        self.files[HEAD][IMAGE] = json.dumps(
            {"image": "ghcr.io/example/tool:1.0.0@" + DIGEST}
        ).encode()
        self.files[HEAD][FEATURE] = declared.replace(b"2.103.2", b"2.104.0")
        self.git_calls: list[tuple[str, ...]] = []
        self.http_calls: list[tuple[str, dict[str, str]]] = []

    def http(self, url: str, headers: Mapping[str, str] | None = None) -> bytes:
        import urllib.error
        from email.message import Message

        values = dict(headers or {})
        self.http_calls.append((url, values))
        if "/token?" in url:
            return b'{"token":"synthetic-pull-token"}'
        if "Authorization" not in values:
            raise urllib.error.HTTPError(url, 401, "anonymous pull", Message(), None)
        if url.rsplit("/", 1)[1] in self.feature_blobs:
            return self.feature_blobs[url.rsplit("/", 1)[1]]
        if url.endswith("/manifests/" + DIGEST):
            return MANIFEST
        raise AssertionError("snapshot test attempted an unexpected metadata endpoint")

    def git(self, root: Path, *args: str) -> bytes | None:
        self.assertEqual(root, self.repository)
        self.git_calls.append(args)
        if args[:3] == ("ls-tree", "-r", "-z"):
            return b"".join(
                b"100644 blob " + blob_sha(data).encode() + b"\t" + name.encode() + b"\0"
                for name, data in sorted(self.files[args[3]].items())
            )
        if args[0] == "show":
            revision, name = args[1].split(":", 1)
            return self.files[revision].get(name)
        raise AssertionError("unexpected Git command")

    def candidate(self) -> eligibility.Candidate:
        return eligibility.Candidate(BASE, HEAD, 17, HEAD, "maintainer", "User", "pull_request")

    def test_cli_git_inventory_reaches_guard_without_candidate_execution(self) -> None:
        output = self.root / "report.json"
        original = dependencies.check_dependencies

        def evaluate(
            base: Path, head: Path, *, state: dict[str, Any] | None = None
        ) -> dict[str, Any]:
            return original(base, head, now=NOW, state=state, fetch_request=self.http)

        with (
            patch.object(cli, "git", side_effect=self.git),
            patch("boxferry_ci.dependencies.check_dependencies", side_effect=evaluate),
            contextlib.redirect_stdout(io.StringIO()),
        ):
            code = cli.main(
                [
                    "dependencies",
                    "--repository-root",
                    str(self.repository),
                    "--base",
                    BASE,
                    "--head",
                    HEAD,
                    "--output",
                    str(output),
                ]
            )
        self.assertEqual(code, 1)
        report = json.loads(output.read_text())
        self.assertEqual(report["status"], "waiting")
        self.assertEqual(
            {item["ecosystem"] for item in report["results"]}, {"oci", "github-runners"}
        )
        self.assertTrue(any(args == ("show", BASE + ":" + OLD) for args in self.git_calls))
        self.assertTrue(any(args == ("show", HEAD + ":" + LOCK) for args in self.git_calls))
        self.assertFalse(any("exploit.py" in " ".join(args) for args in self.git_calls))
        self.assertEqual(len(self.http_calls), 8)

    def test_cli_directory_mode_has_no_author_exemption_or_git_trust_flag(self) -> None:
        roots = {BASE: self.root / "base", HEAD: self.root / "head"}
        for revision, root in roots.items():
            root.mkdir()
            for name, body in self.files[revision].items():
                target = root / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(body)
        output = self.root / "directory-report.json"
        original = dependencies.check_dependencies

        def evaluate(
            base: Path, head: Path, *, state: dict[str, Any] | None = None
        ) -> dict[str, Any]:
            return original(base, head, now=NOW, state=state, fetch_request=self.http)

        with (
            patch("boxferry_ci.dependencies.check_dependencies", side_effect=evaluate),
            patch.object(
                cli, "git", side_effect=AssertionError("directory mode must not invoke Git")
            ),
            contextlib.redirect_stdout(io.StringIO()),
        ):
            code = cli.main(
                [
                    "dependencies",
                    "--base",
                    str(roots[BASE]),
                    "--head",
                    str(roots[HEAD]),
                    "--output",
                    str(output),
                ]
            )
        self.assertEqual(code, 1)
        report = json.loads(output.read_text())
        self.assertEqual(report["status"], "waiting")
        self.assertFalse(report["first_party_exempt"])

    def test_api_inventory_shares_transport_budget_and_feature_counterparts(self) -> None:
        api = SnapshotAPI(self.files)
        with patch("boxferry_ci.dependencies.fetch_metadata", side_effect=self.http):
            report = eligibility.evaluate(
                api, self.candidate(), {"schema_version": 1, "first_seen": {}}, NOW
            )
        self.assertEqual(report["status"], "waiting")
        self.assertEqual(report["retry_at"], "2026-11-06T12:00:00Z")
        self.assertFalse(report["first_party_exempt"])
        self.assertTrue(any(path.startswith("contents/" + LOCK) for path in api.seen))
        self.assertFalse(any(path.startswith("contents/scripts/exploit.py") for path in api.seen))
        self.assertEqual(api.requests, len(api.seen) + len(self.http_calls))
        self.assertEqual(len(self.http_calls), 8)

    def test_invalid_comparison_never_reaches_dependency_policy_or_registry(self) -> None:
        for document in (
            {"merge_base_commit": {"sha": HEAD}, "files": []},
            {"merge_base_commit": {"sha": BASE}, "files": [{}] * 300},
            {"merge_base_commit": {"sha": BASE}, "files": [{"previous_filename": "../escape"}]},
            {"merge_base_commit": {"sha": BASE}, "files": [{}]},
            {"merge_base_commit": {"sha": BASE}, "files": [{"filename": None}]},
            {"merge_base_commit": {"sha": BASE}, "files": [{"filename": ""}]},
            {"merge_base_commit": {"sha": BASE}, "files": [{"previous_filename": OLD}]},
            {
                "merge_base_commit": {"sha": BASE},
                "files": [
                    {
                        "filename": NEW,
                        "previous_filename": None,
                    }
                ],
            },
        ):
            api = SnapshotAPI(self.files)
            with (
                self.subTest(document=document),
                patch.object(api, "get", return_value=document),
                patch.object(dependencies, "check_dependencies") as policy,
                patch.object(dependencies, "fetch_metadata") as registry,
            ):
                with self.assertRaises(eligibility.EligibilityError):
                    eligibility.evaluate(
                        api, self.candidate(), {"schema_version": 1, "first_seen": {}}, NOW
                    )
                policy.assert_not_called()
                registry.assert_not_called()

    def test_api_existing_selector_reused_and_renamed_is_not_new(self) -> None:
        self.files[HEAD][NEW] = self.files[BASE][OLD]
        self.files[HEAD][".github/workflows/another.yml"] = self.files[BASE][OLD]
        self.files[HEAD].pop(IMAGE)
        api = SnapshotAPI(self.files)
        with patch("boxferry_ci.dependencies.fetch_metadata", side_effect=self.http):
            report = eligibility.evaluate(
                api, self.candidate(), {"schema_version": 1, "first_seen": {}}, NOW
            )
        self.assertEqual(report["status"], "pass")
        self.assertEqual(report["results"], [])
        self.assertEqual(len(self.http_calls), 5)
        self.assertFalse(any(path.startswith("contents/" + NEW) for path in api.seen))
        self.assertEqual(sum(path.startswith("git/trees/") for path in api.seen), 2)

    def test_api_tree_truncation_and_owned_symlinks_fail_before_network(self) -> None:
        for field in ("truncated", "symlink", "omit"):
            api = SnapshotAPI(self.files)
            setattr(api, field, True)
            with self.assertRaises(ValueError):
                eligibility.evaluate(
                    api, self.candidate(), {"schema_version": 1, "first_seen": {}}, NOW
                )
        self.assertEqual(self.http_calls, [])

    def test_cli_omitted_owned_blob_is_not_a_false_green(self) -> None:
        real_git = self.git

        def incomplete(root: Path, *args: str) -> bytes | None:
            if args == ("show", HEAD + ":" + IMAGE):
                return None
            return real_git(root, *args)

        with (
            patch.object(cli, "git", side_effect=incomplete),
            contextlib.redirect_stderr(io.StringIO()),
        ):
            code = cli.main(
                [
                    "dependencies",
                    "--repository-root",
                    str(self.repository),
                    "--base",
                    BASE,
                    "--head",
                    HEAD,
                    "--output",
                    str(self.root / "omitted.json"),
                ]
            )
        self.assertEqual(code, 1)
        self.assertEqual(self.http_calls, [])

    def test_snapshot_content_cache_has_immutable_identity_and_memory_bound(self) -> None:
        cache = SnapshotCache()
        data = b"one immutable Git blob"
        with self.assertRaises(OperationalError):
            cache.put("a" * 40, data)
        cache.put(blob_sha(data), data)
        cache.put(blob_sha(data), data)
        self.assertEqual(cache.size, len(data))
        with patch("boxferry_ci.operational.MAX_CACHE_BYTES", len(data)):
            with self.assertRaises(OperationalError):
                cache.put(blob_sha(b"different"), b"different")


if __name__ == "__main__":
    unittest.main()


class InventoryAPI(SnapshotAPI):
    def get(self, suffix: str) -> dict[str, Any]:
        if suffix.startswith("compare/"):
            self.requests += 1
            self.seen.append(suffix)
            return {
                "merge_base_commit": {"sha": BASE},
                "files": [
                    {"filename": path}
                    for path in sorted(self.files[BASE].keys() | self.files[HEAD].keys())
                    if self.files[BASE].get(path) != self.files[HEAD].get(path)
                ],
            }
        return super().get(suffix)


class GitInventorySnapshots(unittest.TestCase):
    """Exact Git inventory acquisition precedes every metadata/observation effect."""

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.path = "automation/tool-artifacts.json"
        self.row = {
            "tool": "git",
            "provider": "kernel.org",
            "currentVersion": "2.49.8",
            "filename": "git-2.49.8.tar.xz",
            "url": "https://www.kernel.org/pub/software/scm/git/git-2.49.8.tar.xz",
            "sha256": "6" * 64,
        }
        self.legacy = json.dumps({"schema": 1, "provenance": {}, "assets": []}).encode()
        self.future = json.dumps(
            {
                "schema": 2,
                "provenance": {},
                "assets": [],
                "sources": [self.row],
            }
        ).encode()
        self.files = {BASE: {self.path: self.legacy}, HEAD: {self.path: self.future}}
        self.metadata = (
            "-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA256\n\n"
            + "6" * 64
            + "  git-2.49.8.tar.xz\n"
            "-----BEGIN PGP SIGNATURE-----\n\nZmFrZQ==\n=abcd\n"
            "-----END PGP SIGNATURE-----\n"
        ).encode()

    @staticmethod
    def candidate() -> eligibility.Candidate:
        return eligibility.Candidate(BASE, HEAD, 17, HEAD, "maintainer", "User", "pull_request")

    def evaluate(self, api: SnapshotAPI) -> dict[str, Any]:
        return eligibility.evaluate(
            api, self.candidate(), {"schema_version": 1, "first_seen": {}}, NOW
        )

    def test_initial_introduction_materializes_both_exact_revisions_before_checksum(self) -> None:
        api = InventoryAPI(self.files)
        original = dependencies.check_dependencies

        def inspect(
            base: Path,
            head: Path,
            *,
            now: datetime,
            state: dict[str, Any],
            context: dependencies.DependencyContext,
            fetch: dependencies.Fetch,
            fetch_request: dependencies.FetchRequest,
        ) -> dict[str, Any]:
            self.assertEqual((base / self.path).read_bytes(), self.legacy)
            self.assertEqual((head / self.path).read_bytes(), self.future)
            self.assertEqual(sum(path.startswith("git/trees/") for path in api.seen), 2)
            return original(
                base,
                head,
                now=now,
                state=state,
                context=context,
                fetch=fetch,
                fetch_request=fetch_request,
            )

        with (
            patch.object(dependencies, "check_dependencies", side_effect=inspect),
            patch.object(eligibility, "registry_fetch", return_value=self.metadata) as metadata,
        ):
            result = self.evaluate(api)
        self.assertEqual(result["status"], "waiting")
        self.assertEqual(result["results"][0]["age_evidence"], "artifact-first-seen")
        metadata.assert_called_once_with(
            api, "https://www.kernel.org/pub/software/scm/git/sha256sums.asc"
        )
        for revision in (BASE, HEAD):
            self.assertIn(f"contents/{self.path}?ref={revision}", api.seen)

    def test_unchanged_inventory_is_authenticated_and_reuses_one_content_blob(self) -> None:
        for revision in (BASE, HEAD):
            self.files[revision][self.path] = self.future
            self.files[revision][OLD] = b"---\njobs:\n  check:\n    runs-on: ubuntu-24.04\n"
        self.files[HEAD]["README.md"] = b"irrelevant change\n"
        api = InventoryAPI(self.files)
        original = dependencies.check_dependencies

        def inspect(
            base: Path,
            head: Path,
            *,
            now: datetime,
            state: dict[str, Any],
            context: dependencies.DependencyContext,
            fetch: dependencies.Fetch,
            fetch_request: dependencies.FetchRequest,
        ) -> dict[str, Any]:
            for root in (base, head):
                self.assertEqual((root / self.path).read_bytes(), self.future)
                self.assertEqual((root / OLD).read_bytes(), self.files[BASE][OLD])
                self.assertFalse((root / "README.md").exists())
            return original(
                base,
                head,
                now=now,
                state=state,
                context=context,
                fetch=fetch,
                fetch_request=fetch_request,
            )

        with (
            patch.object(dependencies, "check_dependencies", side_effect=inspect),
            patch.object(eligibility, "registry_fetch") as metadata,
        ):
            self.assertEqual(self.evaluate(api)["status"], "pass")
        metadata.assert_not_called()
        self.assertEqual(sum(path.startswith(f"contents/{self.path}?") for path in api.seen), 1)
        self.assertEqual(sum(path.startswith("git/trees/") for path in api.seen), 2)

    def test_historical_absence_on_both_sides_and_initial_file_addition_are_supported(self) -> None:
        self.files[BASE] = {}
        self.files[HEAD] = {}
        with patch.object(eligibility, "registry_fetch") as metadata:
            self.assertEqual(self.evaluate(InventoryAPI(self.files))["status"], "pass")
        metadata.assert_not_called()
        self.files[HEAD][self.path] = self.future
        with patch.object(eligibility, "registry_fetch", return_value=self.metadata):
            self.assertEqual(self.evaluate(InventoryAPI(self.files))["status"], "waiting")

    def test_removed_or_renamed_canonical_file_and_contradictory_comparison_refuse(self) -> None:
        for renamed in (None, "automation/renamed-tools.json"):
            self.files[HEAD] = {} if renamed is None else {renamed: self.future}
            with (
                self.subTest(renamed=renamed),
                patch.object(eligibility, "registry_fetch") as metadata,
                self.assertRaisesRegex(eligibility.EligibilityError, "canonical tool inventory"),
            ):
                self.evaluate(InventoryAPI(self.files))
            metadata.assert_not_called()
        self.files[BASE] = {}
        self.files[HEAD] = {}
        api = InventoryAPI(self.files)
        original = api.get

        def contradictory(suffix: str) -> dict[str, Any]:
            if suffix.startswith("compare/"):
                return {"merge_base_commit": {"sha": BASE}, "files": [{"filename": self.path}]}
            return original(suffix)

        with (
            patch.object(api, "get", side_effect=contradictory),
            patch.object(eligibility, "registry_fetch") as metadata,
            self.assertRaisesRegex(eligibility.EligibilityError, "canonical tool inventory"),
        ):
            self.evaluate(api)
        metadata.assert_not_called()

    def test_acquisition_failures_never_contact_metadata_or_create_first_seen_state(self) -> None:
        for fault in (
            "missing",
            "tampered",
            "symlink",
            "wrong-sha",
            "truncated",
            "oversized",
            "short-content",
            "encoding",
            "budget",
        ):
            api = InventoryAPI(self.files)
            original = api.get

            def broken(
                suffix: str,
                fault: str = fault,
                read: Callable[[str], dict[str, Any]] = original,
            ) -> dict[str, Any]:
                if fault == "budget":
                    raise eligibility.EligibilityError("existing request/deadline budget exhausted")
                value = read(suffix)
                if suffix == f"git/trees/{HEAD}?recursive=1":
                    if fault == "truncated":
                        value["truncated"] = True
                    entry = next(item for item in value["tree"] if item["path"] == self.path)
                    if fault == "symlink":
                        entry["mode"] = "120000"
                    if fault == "wrong-sha":
                        entry["sha"] = "f" * 40
                if suffix == f"contents/{self.path}?ref={HEAD}":
                    if fault == "missing":
                        raise eligibility.MissingResource(suffix)
                    if fault == "tampered":
                        data = self.future + b" "
                        value.update(size=len(data), content=base64.b64encode(data).decode())
                    if fault == "oversized":
                        value["size"] = 2 * 1024 * 1024 + 1
                    if fault == "short-content":
                        value["size"] += 1
                    if fault == "encoding":
                        value["encoding"] = "none"
                return value

            with (
                self.subTest(fault=fault),
                patch.object(api, "get", side_effect=broken),
                patch.object(eligibility, "registry_fetch") as metadata,
                self.assertRaises(ValueError),
            ):
                self.evaluate(api)
            metadata.assert_not_called()
        api = InventoryAPI(self.files)
        with (
            patch.object(eligibility, "MAX_SNAPSHOT_BYTES", 1),
            patch.object(eligibility, "registry_fetch") as metadata,
            self.assertRaisesRegex(ValueError, "size budget"),
        ):
            self.evaluate(api)
        metadata.assert_not_called()
