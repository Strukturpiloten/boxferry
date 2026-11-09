"""Canonical downloaded-tool integrity, archive and native Renovate contracts."""

from __future__ import annotations

import base64
import gzip
import hashlib
import io
import json
import lzma
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
from copy import deepcopy
from dataclasses import replace
from http.client import HTTPMessage
from pathlib import Path
from typing import Any
from unittest.mock import Mock, patch
from urllib.error import HTTPError, URLError
from urllib.request import Request

from boxferry_ci import bootstrap, runner
from boxferry_ci import tooling as subject
from boxferry_ci.runner import Diagnostic, RunResult

ROOT = Path(__file__).resolve().parents[2]
PAYLOAD = b"verified executable bytes\n"
CONSUMERS = Path(os.environ.get("BOXFERRY_TOOL_CONSUMER_FIXTURES", ROOT))


def manifest_fixture() -> dict[str, Any]:
    """Independent synthetic releases; never inherit mutable production pins."""
    rows = []
    for architecture, go_architecture, digest in (
        ("x86_64", "amd64", "a" * 64),
        ("aarch64", "arm64", "b" * 64),
    ):
        for tool, repository, archive, filename, member in (
            (
                "tombi",
                "tombi-toml/tombi",
                "tar.gz",
                f"tombi-cli-{{version}}-{architecture}-unknown-linux-musl.tar.gz",
                f"tombi-cli-{{version}}-{architecture}-unknown-linux-musl/tombi",
            ),
            (
                "shfmt",
                "mvdan/sh",
                "binary",
                f"shfmt_v{{version}}_linux_{go_architecture}",
                None,
            ),
            (
                "shellcheck",
                "koalaman/shellcheck",
                "tar.xz",
                f"shellcheck-v{{version}}.linux.{architecture}.tar.xz",
                "shellcheck-v{version}/shellcheck",
            ),
            (
                "hadolint",
                "hadolint/hadolint",
                "binary",
                f"hadolint-linux-{'x86_64' if architecture == 'x86_64' else 'arm64'}",
                None,
            ),
            (
                "actionlint",
                "rhysd/actionlint",
                "tar.gz",
                f"actionlint_{{version}}_linux_{go_architecture}.tar.gz",
                "actionlint",
            ),
            (
                "lychee",
                "lycheeverse/lychee",
                "tar.gz",
                f"lychee-{architecture}-unknown-linux-gnu.tar.gz",
                f"lychee-{architecture}-unknown-linux-gnu/lychee",
            ),
        ):
            rows.append(
                {
                    "tool": tool,
                    "repository": repository,
                    "architecture": architecture,
                    "filenameTemplate": filename,
                    "memberTemplate": member,
                    "archive": archive,
                    "currentVersion": ("lychee-v" if tool == "lychee" else "v") + "2.4.6",
                    "sha256": digest,
                }
            )
    return {
        "schema": 1,
        "provenance": {"fixture": "independently authored synthetic releases"},
        "assets": rows,
    }


def load_document(document: dict[str, Any]) -> dict[tuple[str, str], subject.Asset]:
    """Parse an authored fixture without altering the production manifest."""
    with tempfile.TemporaryDirectory() as temporary:
        path = Path(temporary) / "tool-artifacts.json"
        path.write_text(json.dumps(document), encoding="utf-8")
        return subject.load_manifest(path)


def archive_bytes(
    member: str = "test/bin",
    *,
    kind: bytes = tarfile.REGTYPE,
    extra: tuple[str, bytes] | None = None,
    compression: str = "gz",
) -> bytes:
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w") as archive:
        entry = tarfile.TarInfo(member)
        entry.type = kind
        if kind == tarfile.REGTYPE:
            entry.size = len(PAYLOAD)
        if kind in {tarfile.SYMTYPE, tarfile.LNKTYPE}:
            entry.linkname = "../../outside"
        archive.addfile(entry, io.BytesIO(PAYLOAD) if entry.size else None)
        if extra:
            duplicate = tarfile.TarInfo(extra[0])
            duplicate.type = extra[1]
            archive.addfile(duplicate)
    return gzip.compress(raw.getvalue()) if compression == "gz" else lzma.compress(raw.getvalue())


def asset(
    content: bytes, *, archive: str = "tar.gz", member: str | None = "test/bin"
) -> subject.Asset:
    return subject.Asset(
        "shfmt",
        "mvdan/sh",
        "x86_64",
        "v2.4.6",
        hashlib.sha256(content).hexdigest(),
        "fixture.tar.gz",
        member,
        archive,
    )


class ManifestTests(unittest.TestCase):
    def document(self) -> dict[str, Any]:
        return manifest_fixture()

    def load(self, document: dict[str, Any]) -> dict[tuple[str, str], subject.Asset]:
        return load_document(document)

    def assert_complete_invariants(self, values: dict[tuple[str, str], subject.Asset]) -> None:
        self.assertEqual(
            set(values),
            {
                (tool, architecture)
                for tool in (
                    "tombi",
                    "shfmt",
                    "shellcheck",
                    "hadolint",
                    "actionlint",
                    "lychee",
                )
                for architecture in ("x86_64", "aarch64")
            },
        )
        for (tool, architecture), value in values.items():
            with self.subTest(tool=tool, architecture=architecture):
                prefix = "lychee-v" if tool == "lychee" else "v"
                self.assertRegex(value.tag, rf"^{prefix}[0-9]+\.[0-9]+\.[0-9]+$")
                self.assertRegex(value.digest, r"^[0-9a-f]{64}$")
                self.assertEqual(
                    value.url,
                    f"https://github.com/{value.repository}/releases/download/{value.tag}/{value.filename}",
                )
                self.assertIn(value.archive, {"binary", "tar.gz", "tar.xz"})
                self.assertEqual(value.member is None, value.archive == "binary")
        for tool in (
            "tombi",
            "shfmt",
            "shellcheck",
            "hadolint",
            "actionlint",
            "lychee",
        ):
            self.assertEqual(values[tool, "x86_64"].tag, values[tool, "aarch64"].tag)

    def test_production_manifest_has_complete_version_independent_coverage(self) -> None:
        self.assert_complete_invariants(subject.load_manifest())

    def test_production_lychee_selects_portable_musl_archives_for_both_native_lanes(self) -> None:
        values = subject.load_manifest()
        for architecture in ("x86_64", "aarch64"):
            with self.subTest(architecture=architecture):
                selected = values["lychee", architecture]
                self.assertEqual(
                    selected.filename, f"lychee-{architecture}-unknown-linux-musl.tar.gz"
                )
                self.assertEqual(
                    selected.member, f"lychee-{architecture}-unknown-linux-musl/lychee"
                )
                self.assertEqual(selected.archive, "tar.gz")

    def test_different_valid_production_release_and_integrity_pairs_are_accepted(self) -> None:
        document = json.loads(subject.MANIFEST.read_text(encoding="utf-8"))
        for row in document["assets"]:
            prefix = "lychee-v" if row["tool"] == "lychee" else "v"
            major = int(row["currentVersion"].removeprefix(prefix).split(".")[0])
            row["currentVersion"] = f"{prefix}{major + 1}.0.0"
            row["sha256"] = hashlib.sha256(
                ("independent synthetic next-release integrity:" + row["sha256"]).encode()
            ).hexdigest()
        values = self.load(document)
        self.assert_complete_invariants(values)
        for row in document["assets"]:
            value = values[row["tool"], row["architecture"]]
            self.assertEqual(value.tag, row["currentVersion"])
            self.assertEqual(value.digest, row["sha256"])

    def test_authored_fixture_preserves_exact_tags_digests_and_asset_names(
        self,
    ) -> None:
        values = self.load(self.document())
        self.assertEqual(len(values), 12)
        self.assertEqual(values["hadolint", "aarch64"].filename, "hadolint-linux-arm64")
        self.assertEqual(values["hadolint", "aarch64"].digest, "b" * 64)
        self.assertEqual(values["actionlint", "x86_64"].digest, "a" * 64)
        self.assertEqual(values["lychee", "aarch64"].tag, "lychee-v2.4.6")
        self.assertEqual(
            values["lychee", "aarch64"].url,
            "https://github.com/lycheeverse/lychee/releases/download/"
            "lychee-v2.4.6/lychee-aarch64-unknown-linux-gnu.tar.gz",
        )

    def test_updated_versions_compute_new_filenames_members_and_keep_exact_tags(
        self,
    ) -> None:
        document = self.document()
        for row in document["assets"]:
            row["currentVersion"] = "lychee-v9.8.7" if row["tool"] == "lychee" else "v9.8.7"
            row["sha256"] = "c" * 64
        values = self.load(document)
        self.assertTrue(all(value.digest == "c" * 64 for value in values.values()))
        self.assertEqual(
            values["tombi", "x86_64"].filename,
            "tombi-cli-9.8.7-x86_64-unknown-linux-musl.tar.gz",
        )
        self.assertEqual(values["shellcheck", "aarch64"].member, "shellcheck-v9.8.7/shellcheck")
        self.assertEqual(
            values["actionlint", "aarch64"].filename,
            "actionlint_9.8.7_linux_arm64.tar.gz",
        )
        self.assertIn(
            "/lychee-v9.8.7/lychee-x86_64-unknown-linux-gnu.tar.gz",
            values["lychee", "x86_64"].url,
        )

    def test_missing_duplicate_partial_foreign_and_invalid_rows_fail(self) -> None:
        for change in (
            "missing",
            "duplicate",
            "partial",
            "origin",
            "digest",
            "bare-tag",
            "schema",
        ):
            document = self.document()
            if change == "missing":
                document["assets"].pop()
            elif change == "duplicate":
                document["assets"][1] = deepcopy(document["assets"][0])
            elif change == "partial":
                document["assets"][0]["currentVersion"] = "v9.8.7"
            elif change == "origin":
                document["assets"][0]["repository"] = "evil/repo"
            elif change == "digest":
                document["assets"][0]["sha256"] = "not-a-checksum"
            elif change == "bare-tag":
                document["assets"][0]["currentVersion"] = "2.4.6"
            else:
                document["schema"] = True
            with self.subTest(change=change), self.assertRaises(subject.ToolError):
                self.load(document)

    def test_future_git_sources_leave_all_six_binary_tools_and_profiles_identical(self) -> None:
        legacy = self.document()
        expected = self.load(legacy)
        row = {
            "tool": "git",
            "provider": "kernel.org",
            "currentVersion": "2.48.9",
            "filename": "git-2.48.9.tar.xz",
            "url": "https://www.kernel.org/pub/software/scm/git/git-2.48.9.tar.xz",
            "sha256": "5" * 64,
        }
        for sources in ([], [row]):
            future = legacy | {"schema": 2, "sources": sources}
            self.assertEqual(self.load(future), expected)
        self.assertEqual(
            set(subject.REPOSITORIES),
            {"tombi", "shfmt", "shellcheck", "hadolint", "actionlint", "lychee"},
        )
        self.assertEqual(subject.CORE, ("tombi", "shfmt", "shellcheck", "hadolint"))
        self.assertEqual(
            subject.WEBSITE, ("tombi", "shfmt", "shellcheck", "hadolint", "actionlint", "lychee")
        )
        for future in (
            legacy | {"sources": [row]},
            legacy | {"schema": 2, "sources": [row | {"architecture": "x86_64"}]},
            legacy | {"schema": 2, "sources": [row | {"provider": "github"}]},
            legacy | {"schema": 2, "sources": [row, row]},
            legacy | {"schema": 2, "sources": [row], "assets": legacy["assets"][:-1]},
        ):
            with self.subTest(future=future), self.assertRaises(subject.ToolError):
                self.load(future)

    def test_template_paths_fields_and_format_access_are_rejected(self) -> None:
        for value in (
            "../outside",
            "/absolute",
            "a/{version.__class__}",
            "a/{version!r}",
            "a/{version:99}",
            "a/{unknown}",
            "a\\b",
            "a\nb",
            "{version",
        ):
            document = self.document()
            document["assets"][0]["filenameTemplate"] = value
            with self.subTest(value=value), self.assertRaises(subject.ToolError):
                self.load(document)

    def test_duplicate_keys_oversized_and_linked_metadata_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / "metadata.json"
            for raw in (
                b'{"schema":1,"schema":1}',
                b" " * (subject.MAX_MANIFEST + 1),
                b"\xff",
            ):
                path.write_bytes(raw)
                with self.assertRaises(subject.ToolError):
                    subject.load_manifest(path)
            link = root / "linked.json"
            link.symlink_to(path)
            with self.assertRaises(subject.ToolError):
                subject.load_manifest(link)

    def test_linux_arch_aliases_and_unsupported_platforms(self) -> None:
        for machine, expected in (
            ("amd64", "x86_64"),
            ("x86_64", "x86_64"),
            ("arm64", "aarch64"),
            ("aarch64", "aarch64"),
        ):
            with (
                patch.object(subject.platform, "system", return_value="Linux"),
                patch.object(subject.platform, "machine", return_value=machine),
            ):
                self.assertEqual(subject.host_architecture(), expected)
        for system, machine in (
            ("Darwin", "arm64"),
            ("Linux", "ppc64le"),
            ("Windows", "AMD64"),
        ):
            with (
                patch.object(subject.platform, "system", return_value=system),
                patch.object(subject.platform, "machine", return_value=machine),
                self.assertRaises(subject.ToolError),
            ):
                subject.host_architecture()


class ExtractionTests(unittest.TestCase):
    def extract(self, content: bytes, value: subject.Asset | None = None) -> bytes:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "archive"
            source.write_bytes(content)
            destination = root / "binary"
            subject.extract(value or asset(content), source, destination)
            return destination.read_bytes()

    def test_regular_selected_member_raw_and_both_compressions(self) -> None:
        for compression, format_name in (("gz", "tar.gz"), ("xz", "tar.xz")):
            content = archive_bytes(compression=compression)
            self.assertEqual(self.extract(content, asset(content, archive=format_name)), PAYLOAD)
        self.assertEqual(
            self.extract(PAYLOAD, asset(PAYLOAD, archive="binary", member=None)), PAYLOAD
        )

    def test_traversal_absolute_symlink_hardlink_device_and_duplicate_rejected(self) -> None:
        for member, kind in (
            ("../escape", tarfile.REGTYPE),
            ("/absolute", tarfile.REGTYPE),
            ("dir/../escape", tarfile.REGTYPE),
            ("test/bin", tarfile.SYMTYPE),
            ("test/bin", tarfile.LNKTYPE),
            ("device", tarfile.CHRTYPE),
        ):
            content = archive_bytes(member, kind=kind)
            with self.subTest(member=member, kind=kind), self.assertRaises(subject.ToolError):
                self.extract(content)
        for extra in (("test/bin", tarfile.REGTYPE), ("unselected/link", tarfile.SYMTYPE)):
            content = archive_bytes(extra=extra)
            with self.subTest(extra=extra), self.assertRaises(subject.ToolError):
                self.extract(content)

    def test_missing_truncated_and_budget_exhausted_archives_fail(self) -> None:
        for content in (archive_bytes("other"), b"not gzip", archive_bytes()[:16]):
            with self.subTest(size=len(content)), self.assertRaises(subject.ToolError):
                self.extract(content)
        for constant, maximum in (("MAX_UNPACKED", 10), ("MAX_BINARY", 1), ("MAX_MEMBERS", 0)):
            with patch.object(subject, constant, maximum), self.assertRaises(subject.ToolError):
                self.extract(archive_bytes())
        packed = archive_bytes(compression="xz")
        for value in (packed[:-8], packed + b"trailing"):
            with self.assertRaises(subject.ToolError):
                self.extract(value, asset(value, archive="tar.xz"))


class Response(io.BytesIO):
    def __init__(
        self, payload: bytes, *, length: str | None = None, url: str = "https://github.com/a"
    ) -> None:
        super().__init__(payload)
        self.headers = HTTPMessage()
        if length is not None:
            self.headers["Content-Length"] = length
        self.url = url

    def geturl(self) -> str:
        return self.url


class DownloadTests(unittest.TestCase):
    def fetch(self, response: Response, value: subject.Asset | None = None) -> bytes:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "download"
            with patch.object(subject, "build_opener") as opener:
                opener.return_value.open.return_value = response
                subject.download(value or asset(PAYLOAD), path)
                call = opener.return_value.open.call_args
                self.assertEqual(call.kwargs["timeout"], 20)
                self.assertNotIn("Authorization", call.args[0].headers)
            return path.read_bytes()

    def test_valid_payload_digest_and_official_final_url(self) -> None:
        self.assertEqual(self.fetch(Response(PAYLOAD, length=str(len(PAYLOAD)))), PAYLOAD)

    def test_digest_length_byte_time_and_redirect_budgets_fail(self) -> None:
        responses = (
            Response(b"changed"),
            Response(b""),
            Response(PAYLOAD, length="999"),
            Response(PAYLOAD, length="invalid"),
            Response(PAYLOAD, url="https://evil.example/asset"),
            Response(PAYLOAD, length=str(subject.MAX_DOWNLOAD + 1)),
        )
        for response in responses:
            with self.subTest(url=response.url), self.assertRaises(subject.ToolError):
                self.fetch(response)
        with patch.object(subject, "MAX_DOWNLOAD", 1), self.assertRaises(subject.ToolError):
            self.fetch(Response(PAYLOAD))
        with (
            patch.object(subject.time, "monotonic", side_effect=(0, subject.DOWNLOAD_SECONDS + 1)),
            self.assertRaises(subject.ToolError),
        ):
            self.fetch(Response(PAYLOAD))

    def test_redirect_handler_rejects_foreign_insecure_and_credential_urls(self) -> None:
        handler = subject.OfficialRedirect()
        self.assertEqual(handler.max_redirections, 4)
        for url in (
            "http://github.com/asset",
            "https://evil.example/asset",
            "https://user:password@github.com/asset",
            "https://github.com:123/asset",
            "https://github.com/asset#fragment",
        ):
            with self.subTest(url=url), self.assertRaises(subject.ToolError):
                handler.redirect_request(
                    Request("https://github.com/original"),
                    io.BytesIO(),
                    302,
                    "Found",
                    HTTPMessage(),
                    url,
                )


class RetryTests(unittest.TestCase):
    def http_error(self, code: int) -> HTTPError:
        return HTTPError(
            "https://github.com/signed?secret=must-not-print",
            code,
            "private response body must not print",
            HTTPMessage(),
            io.BytesIO(b"private body"),
        )

    def test_each_transient_http_status_recovers_with_bounded_backoff(self) -> None:
        for code in (408, 429, 500, 503, 599):
            with self.subTest(code=code), tempfile.TemporaryDirectory() as temporary:
                target = Path(temporary) / "download"
                with (
                    patch.object(subject, "build_opener") as opener,
                    patch.object(subject.time, "sleep") as sleep,
                ):
                    opener.return_value.open.side_effect = [
                        self.http_error(code),
                        Response(PAYLOAD),
                    ]
                    subject.download(asset(PAYLOAD), target)
                    self.assertEqual(opener.return_value.open.call_count, 2)
                    sleep.assert_called_once_with(1)
                self.assertEqual(target.read_bytes(), PAYLOAD)

    def test_network_timeouts_and_interrupted_partial_reads_recover(self) -> None:
        for failure in (
            URLError("sensitive address"),
            TimeoutError("sensitive address"),
            ConnectionResetError("sensitive address"),
        ):
            with (
                self.subTest(kind=type(failure).__name__),
                tempfile.TemporaryDirectory() as temporary,
            ):
                target = Path(temporary) / "download"
                with (
                    patch.object(subject, "build_opener") as opener,
                    patch.object(subject.time, "sleep") as sleep,
                ):
                    opener.return_value.open.side_effect = [failure, Response(PAYLOAD)]
                    subject.download(asset(PAYLOAD), target)
                    sleep.assert_called_once_with(1)
                self.assertEqual(target.read_bytes(), PAYLOAD)
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "download"
            interrupted = Response(PAYLOAD)
            with (
                patch.object(interrupted, "read", side_effect=[b"partial", ConnectionResetError()]),
                patch.object(subject, "build_opener") as opener,
                patch.object(subject.time, "sleep") as sleep,
            ):

                def open_response(*_args: object, **_kwargs: object) -> Response:
                    self.assertFalse(target.exists(), "each retry must clean its own partial file")
                    return (
                        interrupted
                        if opener.return_value.open.call_count == 1
                        else Response(PAYLOAD)
                    )

                opener.return_value.open.side_effect = open_response
                subject.download(asset(PAYLOAD), target)
                self.assertEqual(opener.return_value.open.call_count, 2)
                sleep.assert_called_once_with(1)
            self.assertEqual(target.read_bytes(), PAYLOAD)

    def test_exhaustion_preserves_http_code_and_old_installed_binary_without_private_data(
        self,
    ) -> None:
        for code, attempts in ((403, 1), (503, 4)):
            with self.subTest(code=code), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                (root / "shfmt").write_bytes(b"prior verified binary")
                with (
                    patch.object(
                        subject, "load_manifest", return_value=load_document(manifest_fixture())
                    ),
                    patch.object(subject, "build_opener") as opener,
                    patch.object(subject.time, "sleep") as sleep,
                ):
                    opener.return_value.open.side_effect = [
                        self.http_error(code) for _ in range(attempts)
                    ]
                    with self.assertRaises(subject.ToolError) as raised:
                        subject.install(root, ("shfmt",), architecture="x86_64")
                    self.assertEqual(opener.return_value.open.call_count, attempts)
                    self.assertEqual(
                        [call.args[0] for call in sleep.call_args_list],
                        [] if attempts == 1 else [1, 2, 4],
                    )
                message = str(raised.exception)
                self.assertIn(f"HTTP {code}", message)
                self.assertIn("shfmt v2.4.6 x86_64", message)
                self.assertNotIn("secret", message)
                self.assertNotIn("private", message)
                self.assertNotIn("signed", message)
                self.assertEqual((root / "shfmt").read_bytes(), b"prior verified binary")
                self.assertFalse(list(root.glob(".boxferry-tools-*")))

    def test_nontransient_http_integrity_size_and_unsafe_host_fail_without_retry(self) -> None:
        for response in (
            Response(b"bad digest"),
            Response(PAYLOAD, length="invalid"),
            Response(PAYLOAD, length=str(subject.MAX_DOWNLOAD + 1)),
            Response(PAYLOAD, url="https://evil.example/asset"),
        ):
            with tempfile.TemporaryDirectory() as temporary:
                target = Path(temporary) / "download"
                with (
                    patch.object(subject, "build_opener") as opener,
                    patch.object(subject.time, "sleep") as sleep,
                ):
                    opener.return_value.open.return_value = response
                    with self.assertRaises(subject.ToolError):
                        subject.download(asset(PAYLOAD), target)
                    opener.return_value.open.assert_called_once()
                    sleep.assert_not_called()
                self.assertFalse(target.exists())
        for code in (301, 401, 404, 422):
            with self.subTest(code=code), tempfile.TemporaryDirectory() as temporary:
                with (
                    patch.object(subject, "build_opener") as opener,
                    patch.object(subject.time, "sleep") as sleep,
                ):
                    opener.return_value.open.side_effect = self.http_error(code)
                    with self.assertRaisesRegex(subject.ToolError, f"HTTP {code}"):
                        subject.download(asset(PAYLOAD), Path(temporary) / "download")
                    opener.return_value.open.assert_called_once()
                    sleep.assert_not_called()

    def test_retry_uses_one_deadline_and_caps_socket_timeout_to_remaining_budget(self) -> None:
        now = [0.0]
        with tempfile.TemporaryDirectory() as temporary:
            with (
                patch.object(subject, "build_opener") as opener,
                patch.object(subject.time, "monotonic", side_effect=lambda: now[0]),
                patch.object(
                    subject.time,
                    "sleep",
                    side_effect=lambda delay: now.__setitem__(0, now[0] + delay),
                ) as sleep,
            ):

                def fail(*_args: object, **_kwargs: object) -> Response:
                    now[0] += 118 if opener.return_value.open.call_count == 1 else 1
                    raise self.http_error(503)

                opener.return_value.open.side_effect = fail
                with self.assertRaisesRegex(subject.ToolError, "deadline exhausted.*HTTP 503"):
                    subject.download(asset(PAYLOAD), Path(temporary) / "download")
                self.assertEqual(opener.return_value.open.call_count, 2)
                self.assertEqual(
                    [call.kwargs["timeout"] for call in opener.return_value.open.call_args_list],
                    [20, 1],
                )
                sleep.assert_called_once_with(1)

    def test_deadline_crossed_during_backoff_prevents_another_request(self) -> None:
        now = [0.0]
        with tempfile.TemporaryDirectory() as temporary:
            with (
                patch.object(subject, "build_opener") as opener,
                patch.object(subject.time, "monotonic", side_effect=lambda: now[0]),
                patch.object(
                    subject.time,
                    "sleep",
                    side_effect=lambda _delay: now.__setitem__(0, subject.DOWNLOAD_SECONDS + 1),
                ),
            ):
                opener.return_value.open.side_effect = self.http_error(503)
                with self.assertRaisesRegex(subject.ToolError, "deadline exhausted"):
                    subject.download(asset(PAYLOAD), Path(temporary) / "download")
                opener.return_value.open.assert_called_once()

    def test_redirect_hops_share_the_original_deadline(self) -> None:
        handler = subject.OfficialRedirect(deadline=subject.DOWNLOAD_SECONDS)
        request = Request("https://github.com/original")
        with patch.object(subject.time, "monotonic", return_value=subject.DOWNLOAD_SECONDS - 2):
            redirected = handler.redirect_request(
                request,
                io.BytesIO(),
                302,
                "Found",
                HTTPMessage(),
                "https://release-assets.githubusercontent.com/asset",
            )
        self.assertIsNotNone(redirected)
        self.assertEqual(request.timeout, 2)
        with (
            patch.object(subject.time, "monotonic", return_value=subject.DOWNLOAD_SECONDS),
            self.assertRaisesRegex(subject.ToolError, "redirect exceeded.*deadline"),
        ):
            handler.redirect_request(
                request,
                io.BytesIO(),
                302,
                "Found",
                HTTPMessage(),
                "https://release-assets.githubusercontent.com/asset",
            )

    def test_partial_cleanup_preserves_a_replacement_owned_by_another_writer(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            target = root / "download"
            response = Response(PAYLOAD)
            reads = [0]

            def interrupt(_length: int) -> bytes:
                reads[0] += 1
                if reads[0] == 1:
                    return b"own partial bytes"
                competitor = root / "competitor"
                competitor.write_bytes(b"other writer owns this file")
                os.replace(competitor, target)
                raise ConnectionResetError()

            with (
                patch.object(response, "read", side_effect=interrupt),
                patch.object(subject, "build_opener") as opener,
                patch.object(subject.time, "sleep") as sleep,
            ):
                opener.return_value.open.return_value = response
                with self.assertRaisesRegex(subject.ToolError, "appeared between attempts"):
                    subject.download(asset(PAYLOAD), target)
                opener.return_value.open.assert_called_once()
                sleep.assert_called_once_with(1)
            self.assertEqual(target.read_bytes(), b"other writer owns this file")

    def test_preexisting_and_concurrently_created_destinations_are_never_removed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            target = root / "download"
            target.write_bytes(b"preexisting")
            with (
                patch.object(subject, "build_opener") as opener,
                self.assertRaisesRegex(subject.ToolError, "already exists"),
            ):
                subject.download(asset(PAYLOAD), target)
            opener.assert_not_called()
            self.assertEqual(target.read_bytes(), b"preexisting")
            target.unlink()
            target.symlink_to(root / "absent")
            with self.assertRaisesRegex(subject.ToolError, "already exists"):
                subject.download(asset(PAYLOAD), target)
            self.assertTrue(target.is_symlink())
            target.unlink()
            with (
                patch.object(subject, "build_opener") as opener,
                patch.object(subject.time, "sleep") as sleep,
            ):

                def competing_response(*_args: object, **_kwargs: object) -> Response:
                    target.write_bytes(b"concurrent owner")
                    return Response(PAYLOAD)

                opener.return_value.open.side_effect = competing_response
                with self.assertRaisesRegex(subject.ToolError, "FileExistsError"):
                    subject.download(asset(PAYLOAD), target)
                sleep.assert_not_called()
            self.assertEqual(target.read_bytes(), b"concurrent owner")


class InstallationTests(unittest.TestCase):
    def test_old_gnu_bytes_with_same_version_cannot_replace_verified_musl_asset(self) -> None:
        inventory = subject.load_manifest()
        for architecture in ("x86_64", "aarch64"):
            selected = inventory["lychee", architecture]
            legacy = f"old GNU payload reporting the same version {selected.tag}\n".encode()
            self.assertNotEqual(hashlib.sha256(legacy).hexdigest(), selected.digest)

            def stale_download(_value: subject.Asset, path: Path, payload: bytes = legacy) -> None:
                path.write_bytes(payload)

            with (
                self.subTest(architecture=architecture),
                tempfile.TemporaryDirectory() as temporary,
            ):
                root = Path(temporary)
                target = root / "lychee"
                target.write_bytes(legacy)
                with (
                    patch.object(subject, "extract") as extract,
                    self.assertRaisesRegex(subject.ToolError, "SHA256 differs"),
                ):
                    subject.install(
                        root, ("lychee",), architecture=architecture, downloader=stale_download
                    )
                extract.assert_not_called()
                self.assertEqual(target.read_bytes(), legacy)

    def install_fixture(
        self, root: Path, tools: tuple[str, ...], *, fail: bool = False
    ) -> list[subject.Asset]:
        values = load_document(manifest_fixture())
        selected = {
            key: replace(
                value, archive="binary", member=None, digest=hashlib.sha256(PAYLOAD).hexdigest()
            )
            for key, value in values.items()
        }

        def acquire(value: subject.Asset, path: Path) -> None:
            path.write_bytes(b"corrupt" if fail and value.tool == tools[-1] else PAYLOAD)

        with patch.object(subject, "load_manifest", return_value=selected):
            return subject.install(root, tools, architecture="x86_64", downloader=acquire)

    def test_selected_subset_atomic_replacement_permissions_and_cleanup(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "bin"
            root.mkdir()
            (root / "shfmt").write_bytes(b"old")
            (root / "unrelated").write_bytes(b"preserved")
            installed = self.install_fixture(root, ("shfmt", "lychee"))
            self.assertEqual([value.tool for value in installed], ["shfmt", "lychee"])
            self.assertEqual((root / "shfmt").read_bytes(), PAYLOAD)
            self.assertEqual((root / "lychee").stat().st_mode & 0o777, 0o755)
            self.assertEqual((root / "unrelated").read_bytes(), b"preserved")
            self.assertFalse(list(root.glob(".boxferry-tools-*")))

    def test_failed_later_download_keeps_every_prior_installed_tool(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "shfmt").write_bytes(b"old-shfmt")
            (root / "lychee").write_bytes(b"old-lychee")
            with self.assertRaisesRegex(subject.ToolError, "lychee lychee-v2.4.6 x86_64"):
                self.install_fixture(root, ("shfmt", "lychee"), fail=True)
            self.assertEqual((root / "shfmt").read_bytes(), b"old-shfmt")
            self.assertEqual((root / "lychee").read_bytes(), b"old-lychee")
            self.assertFalse(list(root.glob(".boxferry-tools-*")))

    def test_total_deadline_after_acquisition_and_extraction_prevents_replacement(self) -> None:
        for moments in (
            (0, 0, subject.INSTALL_SECONDS + 1),
            (0, 0, 0, subject.INSTALL_SECONDS + 1),
            (0, 0, 0, 0, subject.INSTALL_SECONDS + 1),
        ):
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                (root / "shfmt").write_bytes(b"prior verified binary")
                with (
                    patch.object(subject.time, "monotonic", side_effect=moments),
                    self.assertRaisesRegex(subject.ToolError, "total time budget"),
                ):
                    self.install_fixture(root, ("shfmt",))
                self.assertEqual((root / "shfmt").read_bytes(), b"prior verified binary")
                self.assertFalse(list(root.glob(".boxferry-tools-*")))

    def test_invalid_selection_architecture_and_link_targets_fail(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for tools in ((), ("shfmt", "shfmt"), ("unreviewed",)):
                with self.subTest(tools=tools), self.assertRaises(subject.ToolError):
                    subject.install(root / "bin", tools, architecture="x86_64")
            with self.assertRaises(subject.ToolError):
                subject.install(root / "bin", ("shfmt",), architecture="ppc64")
            linked = root / "linked"
            linked.symlink_to(root, target_is_directory=True)
            with self.assertRaises(subject.ToolError):
                self.install_fixture(linked, ("shfmt",))
            target = root / "shfmt"
            target.symlink_to(root / "outside")
            with self.assertRaises(subject.ToolError):
                self.install_fixture(root, ("shfmt",))
            self.assertFalse((root / "outside").exists())

    def test_cli_requires_explicit_selection_and_uses_canonical_profiles(self) -> None:
        with patch.object(subject, "install", return_value=[]) as install:
            self.assertEqual(subject.main(["--destination", "/tmp/tools", "--profile", "core"]), 0)
            self.assertEqual(install.call_args.args[1], subject.CORE)
            self.assertEqual(
                subject.main(["--destination", "/tmp/tools", "--profile", "website"]), 0
            )
            self.assertEqual(install.call_args.args[1], subject.WEBSITE)
            self.assertEqual(subject.main(["--destination", "/tmp/tools", "--tools", "lychee"]), 0)
            self.assertEqual(install.call_args.args[1], ("lychee",))
        with patch.object(subject, "install", side_effect=subject.ToolError("fixture failure")):
            self.assertEqual(subject.main(["--destination", "/tmp/tools", "--tools", "lychee"]), 2)

    def test_shared_cli_dispatches_tools_without_candidate_policy_arguments(self) -> None:
        from boxferry_ci.__main__ import main as shared_main

        with patch.object(subject, "install", return_value=[]) as install:
            code = shared_main(["tools", "--destination", "/tmp/tools", "--tools", "actionlint"])
        self.assertEqual(code, 0)
        self.assertEqual(install.call_args.args, (Path("/tmp/tools"), ("actionlint",)))


class PythonToolExportTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.project = self.root / "automation"
        self.project.mkdir()
        for name in ("pyproject.toml", "uv.lock", "uv-artifacts.json"):
            shutil.copyfile(ROOT / "automation" / name, self.project / name)
        self.versions = subject._python_versions(self.project)
        self.environment = self.project / ".venv"
        (self.environment / "bin").mkdir(parents=True)
        site = self.environment / "lib/site-packages"
        site.mkdir(parents=True)
        self.distributions: dict[str, Mock] = {}
        for name in ("ruff", "ty"):
            binary = self.environment / "bin" / name
            binary.write_bytes(b"\x7fELF" + name.encode() + b" fixture native bytes")
            binary.chmod(0o700)
            digest = (
                base64.urlsafe_b64encode(hashlib.sha256(binary.read_bytes()).digest())
                .decode()
                .rstrip("=")
            )
            record = Mock()
            record.hash = Mock(mode="sha256", value=digest)
            distribution = Mock(version=self.versions[name], files=[record])
            distribution.locate_file.side_effect = lambda item, name=name: str(
                site if item == "" else site / "../../bin" / name
            )
            self.distributions[name] = distribution
        self.uv = self.root / "private-uv-cache/uv"
        self.uv.parent.mkdir()
        self.uv.write_bytes(b"\x7fELFfixture uv bytes")
        self.destination = self.root / "public/bin"
        self.prefix = patch.object(subject.sys, "prefix", str(self.environment))
        self.env = patch.dict(os.environ, {"UV_PROJECT_ENVIRONMENT": str(self.environment)})
        self.metadata = patch.object(
            subject.metadata, "distribution", side_effect=self.distributions.__getitem__
        )
        self.bootstrap = patch.object(bootstrap, "ensure_uv", return_value=self.uv)
        self.probes = patch.object(runner, "run", side_effect=self.probe)
        for manager in (self.prefix, self.env, self.metadata, self.bootstrap, self.probes):
            manager.start()
            self.addCleanup(manager.stop)

    def probe(self, argv: tuple[str, ...], **kwargs: object) -> RunResult:
        name = Path(argv[0]).name
        self.assertEqual(argv[1:], ("--version",))
        return RunResult(
            "success",
            0,
            f"{name} {self.versions[name]}\n",
            "",
            0,
            Diagnostic("version", "fixture", name, "local", "none"),
            argv,
        )

    def export(self) -> dict[str, str]:
        return subject.install_python_tools(self.destination, project=self.project)

    def test_copies_exact_locked_binaries_without_private_cache_or_python_links(self) -> None:
        with patch.object(bootstrap, "ensure_uv", return_value=self.uv) as acquire:
            self.assertEqual(self.export(), self.versions)
        acquire.assert_called_once()
        self.assertEqual(acquire.call_args.args[0], self.project)
        cache = acquire.call_args.args[1]
        self.assertTrue(cache.is_relative_to(self.destination))
        self.assertFalse(cache.is_relative_to(self.project))
        self.assertFalse(cache.exists())
        self.assertEqual(acquire.call_args.kwargs, {"prefer_path": False})
        for name in ("uv", "ruff", "ty"):
            target = self.destination / name
            self.assertFalse(target.is_symlink())
            self.assertEqual(target.stat().st_mode & 0o777, 0o755)
            self.assertTrue(target.read_bytes().startswith(b"\x7fELF"))
            source = self.uv if name == "uv" else self.environment / "bin" / name
            self.assertNotEqual(source.stat().st_ino, target.stat().st_ino)
        self.assertFalse((self.destination / "python").exists())
        shutil.rmtree(self.environment)
        self.uv.unlink()
        self.assertTrue(all((self.destination / name).is_file() for name in self.versions))

    def test_readonly_project_is_unchanged_with_real_uv_cache_verification(self) -> None:
        self.bootstrap.stop()
        filename = bootstrap.uv_host_asset()
        raw = io.BytesIO()
        payload = self.uv.read_bytes()
        with tarfile.open(fileobj=raw, mode="w:gz") as archive:
            archive_root = filename.removesuffix(".tar.gz")
            directory = tarfile.TarInfo(archive_root)
            directory.type = tarfile.DIRTYPE
            archive.addfile(directory)
            for name in ("uv", "uvx"):
                entry = tarfile.TarInfo(archive_root + "/" + name)
                entry.size = len(payload)
                archive.addfile(entry, io.BytesIO(payload))
        archive_bytes = raw.getvalue()
        metadata_path = self.project / "uv-artifacts.json"
        metadata = json.loads(metadata_path.read_text())
        for row in metadata["assets"]:
            if row["filename"] == filename:
                row["sha256"] = hashlib.sha256(archive_bytes).hexdigest()
        metadata_path.write_text(json.dumps(metadata))
        inventory = {
            path.relative_to(self.project).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in self.project.rglob("*")
            if path.is_file()
        }
        for path in self.project.rglob("*"):
            path.chmod(0o555 if path.is_dir() else 0o444)
        self.project.chmod(0o555)
        try:
            with (
                patch.object(
                    bootstrap,
                    "download_uv_archive",
                    side_effect=lambda version, filename, destination: destination.write_bytes(
                        archive_bytes
                    ),
                ) as acquire,
                patch.object(bootstrap, "uv_version_matches", return_value=True),
            ):
                self.assertEqual(self.export(), self.versions)
            self.assertEqual(acquire.call_count, 1)
            downloaded = acquire.call_args.args[2]
            self.assertTrue(downloaded.is_relative_to(self.destination))
            self.assertFalse(downloaded.exists())
            self.assertEqual(
                {
                    path.relative_to(self.project).as_posix(): hashlib.sha256(
                        path.read_bytes()
                    ).hexdigest()
                    for path in self.project.rglob("*")
                    if path.is_file()
                },
                inventory,
            )
            self.assertEqual(
                set(path.name for path in self.destination.iterdir()), set(self.versions)
            )
            for name in self.versions:
                self.assertFalse((self.destination / name).is_symlink())
                self.assertEqual((self.destination / name).stat().st_mode & 0o777, 0o755)
        finally:
            self.project.chmod(0o755)
            for path in self.project.rglob("*"):
                path.chmod(0o755 if path.is_dir() else 0o644)

    def test_wrong_version_or_record_digest_never_replaces_existing_binaries(self) -> None:
        for defect in ("version", "record"):
            with self.subTest(defect=defect):
                self.destination.mkdir(parents=True, exist_ok=True)
                for name in self.versions:
                    (self.destination / name).write_bytes(b"unrelated original")
                distribution = self.distributions["ruff"]
                distribution.version = "9.9.9" if defect == "version" else self.versions["ruff"]
                distribution.files[0].hash.value = (
                    "wrong" if defect == "record" else distribution.files[0].hash.value
                )
                with self.assertRaises(subject.ToolError):
                    self.export()
                self.assertTrue(
                    all(
                        (self.destination / name).read_bytes() == b"unrelated original"
                        for name in self.versions
                    )
                )

    def test_failed_probe_leaves_targets_intact(self) -> None:
        self.destination.mkdir(parents=True)
        for name in self.versions:
            (self.destination / name).write_bytes(b"original")
        with patch.object(
            runner,
            "run",
            return_value=RunResult(
                "failed",
                1,
                "",
                "bad",
                0,
                Diagnostic("probe", "failed", "ty", "local", "retry"),
                ("ty", "--version"),
            ),
        ):
            with self.assertRaisesRegex(subject.ToolError, "version probe"):
                self.export()
        self.assertTrue(
            all((self.destination / name).read_bytes() == b"original" for name in self.versions)
        )

    def test_rejects_inactive_environment_symlink_target_and_non_native_wrapper(self) -> None:
        with patch.object(subject.sys, "prefix", str(self.root / "other-environment")):
            with self.assertRaisesRegex(subject.ToolError, "active locked"):
                self.export()
        self.destination.mkdir(parents=True)
        sentinel = self.root / "sentinel"
        sentinel.write_bytes(b"keep")
        (self.destination / "uv").symlink_to(sentinel)
        with self.assertRaisesRegex(subject.ToolError, "linked"):
            self.export()
        self.assertEqual(sentinel.read_bytes(), b"keep")
        (self.destination / "uv").unlink()
        self.uv.write_text("#!/root/private/python\n")
        with self.assertRaisesRegex(subject.ToolError, "standalone native"):
            self.export()

    def test_rejects_changed_lock_integrity_and_unsupported_host(self) -> None:
        text = (self.project / "uv.lock").read_text()
        (self.project / "uv.lock").write_text(text.replace("sha256:", "unverified:", 1))
        # Change a Ruff wheel specifically; unrelated package integrity is owned by uv.
        offset = text.index('name = "ruff"')
        (self.project / "uv.lock").write_text(
            text[:offset] + text[offset:].replace("sha256:", "unverified:")
        )
        with self.assertRaisesRegex(subject.ToolError, "integrity"):
            self.export()
        with patch.object(subject.platform, "system", return_value="Darwin"):
            with self.assertRaisesRegex(subject.ToolError, "Linux"):
                self.export()

    def test_invalid_lock_and_active_environment_destination_fail_actionably(self) -> None:
        with self.assertRaisesRegex(subject.ToolError, "must not modify"):
            subject.install_python_tools(self.environment / "bin", project=self.project)
        (self.project / "uv.lock").write_text("not valid TOML [")
        with self.assertRaisesRegex(subject.ToolError, "restore the selected locked source"):
            self.export()

    def test_cli_can_combine_downloaded_and_python_tools_or_select_python_alone(self) -> None:
        with (
            patch.object(subject, "install", return_value=[]) as install,
            patch.object(subject, "install_python_tools", return_value=self.versions) as export,
        ):
            self.assertEqual(
                subject.main(["--destination", str(self.destination), "--python-tools"]), 0
            )
            install.assert_not_called()
            self.assertEqual(
                subject.main(
                    [
                        "--destination",
                        str(self.destination),
                        "--tools",
                        "actionlint",
                        "lychee",
                        "--python-tools",
                    ]
                ),
                0,
            )
            self.assertEqual(install.call_args.args[1], ("actionlint", "lychee"))
            self.assertEqual(export.call_count, 2)


class ConsumerContractTests(unittest.TestCase):
    def closure(self, root: Path) -> Path:
        wrapper = (CONSUMERS / "scripts/install-file-tools.sh").read_text(encoding="utf-8")
        self.assertIn(
            'exec python3 "${repository_root}/automation/src/boxferry_ci/tooling.py"',
            wrapper,
            "consumer integration fixture must be the stdlib shim before any execution",
        )
        self.assertNotIn(
            "curl ", wrapper, "legacy download scripts must never execute in this test"
        )
        for relative, source in (
            ("scripts/install-file-tools.sh", CONSUMERS / "scripts/install-file-tools.sh"),
            ("automation/src/boxferry_ci/tooling.py", Path(subject.__file__)),
            ("automation/tool-artifacts.json", subject.MANIFEST),
        ):
            target = root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target)
        return root / "scripts/install-file-tools.sh"

    def test_thin_wrapper_preserves_destinations_and_stdlib_only_closure(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            wrapper = self.closure(root)
            fakebin = root / "fakebin"
            fakebin.mkdir()
            interpreter = fakebin / "python3"
            interpreter.write_text(
                f"#!{sys.executable}\nimport json, sys\nprint(json.dumps(sys.argv[1:]))\n",
                encoding="utf-8",
            )
            interpreter.chmod(0o755)
            for name in ("curl", "wget", "install", "tar"):
                blocked = fakebin / name
                blocked.write_text(
                    f"#!{sys.executable}\n"
                    f"raise SystemExit('unexpected legacy command in isolated test: {name}')\n",
                    encoding="utf-8",
                )
                blocked.chmod(0o755)
            environment = dict(
                os.environ, PATH=str(fakebin) + os.pathsep + os.environ.get("PATH", "")
            )
            for arguments, destination in (
                ([], "/usr/local/bin"),
                ([str(root / "directory with spaces")], str(root / "directory with spaces")),
            ):
                result = subprocess.run(
                    ["bash", str(wrapper), *arguments],
                    env=environment,
                    capture_output=True,
                    text=True,
                    check=False,
                    timeout=10,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(
                    json.loads(result.stdout),
                    [
                        str(root / "automation/src/boxferry_ci/tooling.py"),
                        "--destination",
                        destination,
                        "--profile",
                        "core",
                    ],
                )
            result = subprocess.run(
                ["bash", str(wrapper), "one", "two"],
                env=environment,
                capture_output=True,
                text=True,
                check=False,
                timeout=10,
            )
            self.assertEqual(result.returncode, 2)
            source = root / "automation/src/boxferry_ci/tooling.py"
            check = "import runpy,sys; print(len(runpy.run_path(sys.argv[1])['load_manifest']()))"
            result = subprocess.run(
                [sys.executable, "-I", "-c", check, str(source)],
                capture_output=True,
                text=True,
                check=False,
                timeout=10,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.strip(), "12")

    def test_standalone_future_inventory_requires_its_exact_source_reader(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.closure(root)
            path = root / "automation/tool-artifacts.json"
            document: dict[str, Any] = manifest_fixture() | {
                "schema": 2,
                "sources": [
                    {
                        "tool": "git",
                        "provider": "kernel.org",
                        "currentVersion": "2.47.9",
                        "filename": "git-2.47.9.tar.xz",
                        "url": "https://www.kernel.org/pub/software/scm/git/git-2.47.9.tar.xz",
                        "sha256": "4" * 64,
                    }
                ],
            }
            path.write_text(json.dumps(document))
            directory = root / "automation/src/boxferry_ci"
            probe = directory / "probe.py"
            probe.write_text(
                "import runpy\n"
                "reader = runpy.run_path(str(__file__).replace('probe.py', 'tooling.py'))\n"
                "print(len(reader['load_manifest']()))\n"
            )
            environment = dict(os.environ, PYTHONPATH="")

            def run() -> subprocess.CompletedProcess[str]:
                return subprocess.run(
                    [sys.executable, str(probe)],
                    env=environment,
                    capture_output=True,
                    text=True,
                    check=False,
                    timeout=10,
                )

            missing = run()
            self.assertNotEqual(missing.returncode, 0)
            self.assertIn(
                "source-aware inventory requires its valid git_source.py reader", missing.stderr
            )
            wrong = root / "wrong-source"
            wrong.mkdir()
            marker = root / "unexpected-helper-execution"
            (wrong / "git_source.py").write_text(
                "from pathlib import Path\n" + f"Path({str(marker)!r}).touch()\n"
            )
            environment["PYTHONPATH"] = str(wrong)
            refused = run()
            self.assertNotEqual(refused.returncode, 0)
            self.assertFalse(marker.exists())
            environment["PYTHONPATH"] = ""
            shutil.copyfile(
                Path(subject.__file__).with_name("git_source.py"), directory / "git_source.py"
            )
            supported = run()
            self.assertEqual(supported.returncode, 0, supported.stderr)
            self.assertEqual(supported.stdout.strip(), "12")
            selected = directory / "git_source.py"
            selected.unlink()
            selected.symlink_to(Path(subject.__file__).with_name("git_source.py"))
            linked = run()
            self.assertNotEqual(linked.returncode, 0)
            self.assertIn("missing or nonregular", linked.stderr)
            selected.unlink()
            shutil.copyfile(Path(subject.__file__).with_name("git_source.py"), selected)
            document["sources"][0]["provider"] = "github"
            path.write_text(json.dumps(document))
            invalid = run()
            self.assertNotEqual(invalid.returncode, 0)
            self.assertIn("canonical provider", invalid.stderr)

    def test_isolated_runpy_loads_exact_schema2_sibling_outside_search_path(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.closure(root)
            source = root / "automation/src/boxferry_ci/tooling.py"
            sibling = source.with_name("git_source.py")
            shutil.copyfile(Path(subject.__file__).with_name("git_source.py"), sibling)
            elsewhere = root / "unrelated-working-directory"
            elsewhere.mkdir()
            marker = root / "unselected-reader-ran"
            (elsewhere / "git_source.py").write_text(
                "from pathlib import Path\n" + f"Path({str(marker)!r}).touch()\n"
            )
            row = {
                "tool": "git",
                "provider": "kernel.org",
                "currentVersion": "2.45.9",
                "filename": "git-2.45.9.tar.xz",
                "url": "https://www.kernel.org/pub/software/scm/git/git-2.45.9.tar.xz",
                "sha256": "2" * 64,
            }
            check = (
                "import runpy,sys; "
                "sys.path.insert(0, sys.argv[2]); "
                "reader = runpy.run_path(sys.argv[1]); "
                "print(len(reader['load_manifest']())); "
                "assert '_boxferry_tooling_git_source' not in sys.modules"
            )
            for sources in ([], [row]):
                document = manifest_fixture() | {"schema": 2, "sources": sources}
                (root / "automation/tool-artifacts.json").write_text(json.dumps(document))
                isolated = (
                    "import runpy,sys; print(len(runpy.run_path(sys.argv[1])['load_manifest']()))"
                )
                for invocation in (isolated, check):
                    result = subprocess.run(
                        [sys.executable, "-I", "-c", invocation, str(source), str(elsewhere)],
                        cwd=elsewhere,
                        capture_output=True,
                        text=True,
                        check=False,
                        timeout=10,
                    )
                    with self.subTest(sources=sources, invocation=invocation):
                        self.assertEqual(result.returncode, 0, result.stderr)
                        self.assertEqual(result.stdout.strip(), "12")
                        self.assertFalse(marker.exists())
            # A preloaded private module cannot become authority or be replaced.
            poisoned = (
                "import runpy,sys,types\n"
                "poison = types.ModuleType('_boxferry_tooling_git_source')\n"
                "sys.modules[poison.__name__] = poison\n"
                "reader = runpy.run_path(sys.argv[1])\n"
                "try:\n"
                "    reader['load_manifest']()\n"
                "except ValueError as error:\n"
                "    assert 'module name is already occupied' in str(error)\n"
                "    assert sys.modules[poison.__name__] is poison\n"
                "    print('refused poisoned module')\n"
                "else:\n"
                "    raise AssertionError('preloaded reader accepted')\n"
            )
            refused = subprocess.run(
                [sys.executable, "-I", "-c", poisoned, str(source)],
                cwd=elsewhere,
                capture_output=True,
                text=True,
                check=False,
                timeout=10,
            )
            self.assertEqual(refused.returncode, 0, refused.stderr)
            self.assertEqual(refused.stdout.strip(), "refused poisoned module")

    def test_devcontainer_minimal_closure_has_no_duplicate_version_authorities(self) -> None:
        source = (CONSUMERS / ".devcontainer/Dockerfile").read_text(encoding="utf-8")
        for relative, destination in (
            ("scripts/install-file-tools.sh", "/opt/boxferry-tooling/scripts/"),
            (
                "automation/src/boxferry_ci/tooling.py",
                "/opt/boxferry-tooling/automation/src/boxferry_ci/",
            ),
            ("automation/tool-artifacts.json", "/opt/boxferry-tooling/automation/"),
        ):
            self.assertIn(f"COPY {relative} {destination}", source)
        self.assertIn(
            "bash /opt/boxferry-tooling/scripts/install-file-tools.sh /usr/local/bin", source
        )
        self.assertIn("--destination /usr/local/bin --tools actionlint lychee", source)
        self.assertIn("COPY scripts/shared-ci.py /opt/boxferry-tooling/scripts/", source)
        self.assertIn("COPY .github/automation.json /opt/boxferry-tooling/.github/", source)
        self.assertIn("--tools actionlint lychee --python-tools", source)
        self.assertNotIn("rm -r /opt/boxferry-tooling", source)
        self.assertNotIn("ACTIONLINT_VERSION", source)
        self.assertNotIn("LYCHEE_VERSION", source)
        self.assertNotIn("releases/download/v${", source)


class NativeRenovateTests(unittest.TestCase):
    def test_native_extraction_atomic_replacement_and_release_asset_mapping(self) -> None:
        runtime = Path(os.environ.get("BOXFERRY_RENOVATE_PACKAGE", ROOT / "target/tools/renovate"))
        self.assertTrue(runtime.is_dir(), "provision the pinned native Renovate test runtime")
        fragment = os.environ.get("BOXFERRY_TOOL_RENOVATE_FRAGMENT")
        config = Path(fragment) if fragment else ROOT / "renovate-presets/common.json"
        root_config = CONSUMERS / ".github/renovate.json"
        script = r"""
(async () => {
  const fs = require("fs"),
    assert = require("assert/strict"),
    os = require("os"),
    path = require("path");
  const base = process.argv[1] + "/dist/",
    root = process.argv[2],
    config = JSON.parse(fs.readFileSync(process.argv[3]));
  const rootConfig = JSON.parse(fs.readFileSync(process.argv[4]));
  await require(base + "logger/index.js").init();
  assert.equal(require(base + "util/regex.js").regexEngineStatus.type, "available");
  const { GlobalConfig } = require(base + "config/global.js");
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "boxferry-tool-native-"));
  GlobalConfig.set({ localDir: temporary });
  try {
    const { extractPackageFile: extract } = require(
      base + "modules/manager/custom/regex/index.js",
    );
    const { doAutoReplace } = require(
      base + "workers/repository/update/branch/auto-replace.js",
    );
    const { applyPackageRules } = require(base + "util/package-rules/index.js");
    const memory = require(base + "util/cache/memory/index.js");
    memory.init();
    // Resolve the actual inherited rules through the pinned SDK without network access.
    // These fixture bytes are the same canonical preset inputs under test, not defaults.
    for (const reference of rootConfig.extends || []) {
      if (!reference.startsWith("github>Strukturpiloten/boxferry//renovate-presets/")) continue;
      const pattern =
        /^github>Strukturpiloten\/boxferry\/\/renovate-presets\/(common|rust)#([a-f0-9]{40})$/;
      const match = pattern.exec(reference);
      assert.ok(match, "inherited first-party presets must retain immutable scoped references");
      memory.set("preset:" + reference,
        JSON.parse(fs.readFileSync(root + "/renovate-presets/" + match[1] + ".json")));
    }
    const { resolveConfigPresets } = require(base + "config/presets/index.js");
    const { config: resolvedRoot } = await resolveConfigPresets(rootConfig);
    const { get: versioning } = require(base + "modules/versioning/index.js");
    const { GithubReleaseAttachmentsDatasource } = require(
      base + "modules/datasource/github-release-attachments/index.js",
    );
    const manager = config.customManagers.find((x) =>
      x.managerFilePatterns.includes("/^automation/tool-artifacts\\.json$/"),
    );
    assert.ok(manager, "the canonical tool metadata needs exactly one native manager");
    const file = "automation/tool-artifacts.json",
      original = fs.readFileSync(root + "/" + file, "utf8");
    let content = original;
    const rows = JSON.parse(original).assets;
    const owners = config.customManagers.flatMap((m) =>
      m.managerFilePatterns.some((p) => new RegExp(p.slice(1, -1)).test(file))
        ? extract(content, file, m).deps
        : [],
    );
    assert.equal(
      owners.length,
      12,
      "each tool/architecture row has exactly one extraction owner",
    );
    const adopted = (rootConfig.extends || []).some(x =>
      x.startsWith("github>Strukturpiloten/boxferry//renovate-presets/common#"));
    const localManagers = [...(adopted ? config.customManagers : []),
      ...rootConfig.customManagers];
    const rootOwners = localManagers.flatMap(m =>
      m.managerFilePatterns.some(p => new RegExp(p.slice(1,-1)).test(file))
        ? extract(original, file, m).deps : []);
    assert.equal(rootOwners.length, 12, "root bootstrap needs one owner per row");
    assert.deepEqual(rootOwners, owners);
    assert.ok(!rootConfig.customManagers.some(m =>
      m.description === "Signal updates for checksum-pinned file-quality tools"));
    for (let index = 0; index < rows.length; index++) {
      const dep = extract(content, file, manager).deps[index],
        row = rows[index];
      assert.equal(dep.depName, row.repository);
      assert.equal(dep.depType, row.architecture);
      assert.equal(dep.currentValue, row.currentVersion);
      assert.equal(dep.currentDigest, row.sha256);
      const currentVersion = row.currentVersion.replace(/^(?:lychee-v|v)/, ""),
        nextVersion = [Number(currentVersion.split(".")[0]) + 1, 0, 0].join("."),
        nextTag = (row.tool === "lychee" ? "lychee-v" : "v") + nextVersion,
        nextDigest = (row.sha256 === "b".repeat(64) ? "c" : "b").repeat(64);
      const api = versioning(manager.versioningTemplate);
      assert.ok(api.isValid(dep.currentValue));
      assert.ok(api.isGreaterThan(nextTag, dep.currentValue));
      const filename = row.filenameTemplate.replaceAll("{version}", currentVersion);
      const nextFilename = row.filenameTemplate.replaceAll("{version}", nextVersion);
      const datasource = new GithubReleaseAttachmentsDatasource();
      const calls = [];
      datasource.http.getJsonUnchecked = async (url) => {
        calls.push(url);
        if (url.endsWith("/releases/tags/" + row.currentVersion))
          return {
            body: {
              tag_name: row.currentVersion,
              html_url:
                "https://github.com/" +
                row.repository +
                "/releases/tag/" +
                row.currentVersion,
              assets: [
                {
                  name: "checksums.txt",
                  size: 100,
                  browser_download_url: "https://github.com/current-checksums",
                },
              ],
            },
          };
        assert.ok(url.endsWith("/releases/tags/" + nextTag));
        return {
          body: {
            tag_name: nextTag,
            assets: [
              {
                name: "checksums.txt",
                size: 100,
                browser_download_url: "https://github.com/next-checksums",
              },
            ],
          },
        };
      };
      datasource.http.getText = async (url) => ({
        body: url.endsWith("current-checksums")
          ? row.sha256 + "  " + filename
          : nextDigest + "  " + nextFilename,
      });
      const digest = await datasource.getDigest(
        {
          packageName: row.repository,
          currentValue: row.currentVersion,
          currentDigest: row.sha256,
          registryUrl: "https://github.com",
        },
        nextTag,
      );
      assert.equal(digest, nextDigest);
      assert.equal(calls.length, 2);
      const updated = await doAutoReplace(
        {
          ...manager,
          ...dep,
          manager: "regex",
          packageFile: file,
          depIndex: index,
          newValue: nextTag,
          newDigest: digest,
        },
        content,
        false,
      );
      const before = JSON.parse(content),
        after = JSON.parse(updated);
      before.assets[index].currentVersion = nextTag;
      before.assets[index].sha256 = nextDigest;
      assert.deepEqual(
        after,
        before,
        "native replacement must update only this release tag and checksum",
      );
      assert.equal(after.assets[index].filenameTemplate, row.filenameTemplate);
      content = updated;
      const policy = await applyPackageRules({
        ...config,
        ...dep,
        manager: "regex",
        packageName: dep.depName,
        updateType: "major",
      });
      assert.equal(policy.groupName, "Canonical downloaded tools");
      assert.equal(policy.pinDigests, true);
      assert.equal(policy.automerge, true);
      assert.equal(policy.dependencyDashboardApproval, false);
      assert.equal(policy.minimumReleaseAge, "3 days");
      const localPolicy = await applyPackageRules({ ...resolvedRoot, ...dep, manager: "regex",
        packageName:dep.depName, updateType:"major" });
      assert.equal(localPolicy.automerge, true);
      assert.equal(localPolicy.dependencyDashboardApproval, false);
      assert.equal(localPolicy.groupName, "Canonical downloaded tools");
      assert.equal(localPolicy.pinDigests, true);
      assert.equal(localPolicy.minimumReleaseAge, "3 days");
      assert.equal(localPolicy.minimumReleaseAgeBehaviour, "timestamp-required");
      assert.equal(localPolicy.internalChecksFilter, "strict");
    }
    assert.equal(JSON.parse(content).assets.length, 12);
    console.log(
      "12 unique rows: exact tags, architecture grouping, atomic tag+SHA replacement, " +
      "real datasource asset mapping passed",
    );
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
"""
        result = subprocess.run(
            ["node", "-e", script, str(runtime), str(ROOT), str(config), str(root_config)],
            capture_output=True,
            text=True,
            check=False,
            timeout=90,
        )
        self.assertEqual(result.returncode, 0, (result.stdout + result.stderr)[-6000:])
        self.assertIn("12 unique rows", result.stdout)


if __name__ == "__main__":
    unittest.main()
