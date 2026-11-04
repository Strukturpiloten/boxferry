"""Candidate integrity comes from every exact official URL, independent of host."""

from __future__ import annotations

import hashlib
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from dataclasses import replace
from http.client import HTTPMessage
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
from urllib.error import HTTPError
from urllib.request import Request

from boxferry_ci import asset_verification as subject

ROOT = Path(os.environ.get("BOXFERRY_ASSET_FIXTURE_ROOT", Path(__file__).resolve().parents[2]))


def payload(url: str) -> bytes:
    return ("independently authored inert candidate bytes: " + url).encode()


class Response(io.BytesIO):
    def __init__(self, url: str) -> None:
        super().__init__(payload(url))
        self.url = url
        self.headers = HTTPMessage()
        self.headers["Content-Length"] = str(len(payload(url)))

    def geturl(self) -> str:
        return self.url


class CandidateIntegrityTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="candidate-assets-test-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        for relative in subject.SOURCES:
            target = self.root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes((ROOT / relative).read_bytes())
        self.refresh_digests()
        self.good = {path: (self.root / path).read_bytes() for path in subject.SOURCES}
        self.calls: list[str] = []

    def refresh_digests(self) -> None:
        assets = subject.inventory(self.root)
        digests = {
            (asset.tool, asset.architecture): hashlib.sha256(payload(asset.url)).hexdigest()
            for asset in assets
        }
        path = self.root / "automation/tool-artifacts.json"
        document = json.loads(path.read_text())
        for row in document["assets"]:
            row["sha256"] = digests[row["tool"], row["architecture"]]
        path.write_text(json.dumps(document))
        path = self.root / "automation/uv-artifacts.json"
        document = json.loads(path.read_text())
        reverse = {
            filename: architecture for architecture, filename in subject.bootstrap.UV_ASSETS.items()
        }
        for row in document["assets"]:
            row["sha256"] = digests["uv", reverse[row["filename"]]]
        path.write_text(json.dumps(document))

    def open(self, request: Request, *, timeout: float) -> Response:
        self.assertGreater(timeout, 0)
        self.assertTrue(request.full_url.startswith("https://github.com/"))
        self.calls.append(request.full_url)
        return Response(request.full_url)

    def verify(self, candidate_sha: str | None = None) -> tuple[int, int, int]:
        opener = SimpleNamespace(open=self.open)
        with (
            patch.object(subject.tooling, "build_opener", return_value=opener),
            patch.object(subject.bootstrap, "build_opener", return_value=opener),
        ):
            return subject.verify(self.root, candidate_sha)

    def test_all_twelve_tools_and_both_uv_archives_are_hashed_without_execution(self) -> None:
        with (
            patch.object(subject.tooling, "extract") as extract_tool,
            patch.object(subject.bootstrap, "extract_uv_binary") as extract_uv,
            patch.object(Path, "chmod") as chmod,
            patch.object(subject.bootstrap.subprocess, "run") as execute,
        ):
            count, unique, size = self.verify()
        self.assertEqual((count, unique), (14, 14))
        self.assertEqual(len(set(self.calls)), 14)
        self.assertEqual(size, sum(len(payload(url)) for url in self.calls))
        extract_tool.assert_not_called()
        extract_uv.assert_not_called()
        chmod.assert_not_called()
        execute.assert_not_called()

    def test_new_releases_with_stale_nonhost_or_noncore_digests_fail(self) -> None:
        for tool, architecture in (
            ("hadolint", "aarch64"),
            ("actionlint", "x86_64"),
            ("lychee", "aarch64"),
            ("uv", "aarch64"),
        ):
            with self.subTest(tool=tool, architecture=architecture):
                for relative, content in self.good.items():
                    (self.root / relative).write_bytes(content)
                before = next(
                    asset
                    for asset in subject.inventory(self.root)
                    if (asset.tool, asset.architecture) == (tool, architecture)
                )
                if tool == "uv":
                    path = self.root / "automation/uv-artifacts.json"
                    document = json.loads(path.read_text())
                    version = f"{int(before.version.split('.')[0]) + 1}.0.0"
                    for row in document["assets"]:
                        row["currentVersion"] = version
                    path.write_text(json.dumps(document))
                    path = self.root / "automation/pyproject.toml"
                    path.write_text(path.read_text().replace("==" + before.version, "==" + version))
                else:
                    path = self.root / "automation/tool-artifacts.json"
                    document = json.loads(path.read_text())
                    prefix = "lychee-v" if tool == "lychee" else "v"
                    version = (
                        f"{prefix}{int(before.version.removeprefix(prefix).split('.')[0]) + 1}.0.0"
                    )
                    for row in document["assets"]:
                        if row["tool"] == tool:
                            row["currentVersion"] = version
                    path.write_text(json.dumps(document))
                self.refresh_digests()
                path = self.root / (
                    "automation/uv-artifacts.json"
                    if tool == "uv"
                    else "automation/tool-artifacts.json"
                )
                document = json.loads(path.read_text())
                for row in document["assets"]:
                    if (tool == "uv" and row["filename"] == before.filename) or (
                        row.get("tool"),
                        row.get("architecture"),
                    ) == (tool, architecture):
                        row["sha256"] = before.digest
                path.write_text(json.dumps(document))
                with self.assertRaisesRegex(
                    subject.VerificationError, f"{tool} .*{architecture}.*SHA256"
                ):
                    self.verify()

    def test_exact_candidate_blobs_win_over_modified_worktree(self) -> None:
        revision = "c" * 40
        blobs = {hashlib.sha1(content).hexdigest(): content for content in self.good.values()}
        identities = {
            relative: hashlib.sha1(content).hexdigest() for relative, content in self.good.items()
        }
        tree_revisions = []

        def git(_root: Path, args: tuple[str, ...], _limit: int) -> bytes:
            if args[0] == "ls-tree":
                tree_revisions.append(args[2])
                self.assertEqual(args[2], revision)
                relative = args[-1]
                return f"100644 blob {identities[relative]}\t{relative}\0".encode()
            if args[1] == "-s":
                return str(len(blobs[args[2]])).encode()
            return blobs[args[2]]

        path = self.root / "automation/tool-artifacts.json"
        document = json.loads(path.read_text())
        document["assets"][0]["sha256"] = "0" * 64
        path.write_text(json.dumps(document))
        with patch.object(subject.admission, "read_git", side_effect=git):
            self.assertEqual(self.verify(revision)[:2], (14, 14))
        self.assertEqual(tree_revisions, [revision] * 3)
        with self.assertRaisesRegex(subject.VerificationError, "SHA256"):
            self.verify()

    def test_exact_source_rejects_symlinks_and_mutable_revisions(self) -> None:
        with (
            patch.object(
                subject.admission,
                "read_git",
                return_value=b"120000 blob " + b"a" * 40 + b"\tautomation/tool-artifacts.json\0",
            ),
            self.assertRaisesRegex(subject.VerificationError, "regular tracked file"),
        ):
            self.verify("b" * 40)
        for revision in ("main", "HEAD", "b" * 39):
            with self.subTest(revision=revision), self.assertRaises(subject.ContractError):
                self.verify(revision)
        self.assertEqual(self.calls, [])

    def test_missing_extra_foreign_and_invalid_rows_fail_before_network(self) -> None:
        for mutation in ("missing", "extra", "foreign", "uv-missing"):
            with self.subTest(mutation=mutation):
                for relative, content in self.good.items():
                    (self.root / relative).write_bytes(content)
                path = self.root / (
                    "automation/uv-artifacts.json"
                    if mutation == "uv-missing"
                    else "automation/tool-artifacts.json"
                )
                document = json.loads(path.read_text())
                if mutation in {"missing", "uv-missing"}:
                    document["assets"].pop()
                elif mutation == "extra":
                    document["assets"].append(document["assets"][0])
                else:
                    document["assets"][0]["repository"] = "evil.invalid/repository"
                path.write_text(json.dumps(document))
                with self.assertRaises(
                    (subject.tooling.ToolError, subject.bootstrap.BootstrapError)
                ):
                    self.verify()
                self.assertEqual(self.calls, [])

    def test_byte_time_and_request_budgets_fail_with_asset_context(self) -> None:
        with (
            patch.object(subject, "MAX_TOTAL_BYTES", 1),
            self.assertRaisesRegex(subject.VerificationError, r"tombi .*byte budget"),
        ):
            self.verify()
        clock = [0.0]

        def late(asset: subject.tooling.Asset, destination: Path) -> None:
            destination.write_bytes(payload(asset.url))
            clock[0] = subject.TOTAL_SECONDS + 1

        with (
            patch.object(subject.time, "monotonic", side_effect=lambda: clock[0]),
            self.assertRaisesRegex(subject.VerificationError, r"tombi .*time budget"),
        ):
            subject.verify(self.root, None, tool_download=late)
        self.calls.clear()

        def unavailable(request: Request, *, timeout: float) -> Response:
            self.calls.append(request.full_url)
            raise HTTPError(request.full_url, 503, "unavailable", HTTPMessage(), None)

        with (
            patch.object(
                subject.tooling, "build_opener", return_value=SimpleNamespace(open=unavailable)
            ),
            patch.object(subject.tooling.time, "sleep"),
            self.assertRaisesRegex(subject.VerificationError, r"tombi .*HTTP 503"),
        ):
            subject.verify(self.root, None)
        self.assertEqual(len(self.calls), subject.tooling.DOWNLOAD_ATTEMPTS)

    def test_hard_total_deadline_interrupts_and_restores_the_signal_handler(self) -> None:
        previous = subject.signal.getsignal(subject.signal.SIGALRM)

        def blocked(asset: subject.tooling.Asset, destination: Path) -> None:
            destination.write_bytes(payload(asset.url))
            handler = subject.signal.getsignal(subject.signal.SIGALRM)
            assert callable(handler)
            handler(subject.signal.SIGALRM, None)

        with self.assertRaisesRegex(subject.VerificationError, r"tombi .*total time budget"):
            subject.verify(self.root, None, tool_download=blocked)
        self.assertEqual(subject.signal.getsignal(subject.signal.SIGALRM), previous)
        self.assertEqual(subject.signal.getitimer(subject.signal.ITIMER_REAL), (0.0, 0.0))

    def test_real_blocked_read_is_interrupted_without_thread_residue(self) -> None:
        script = r"""
import os, signal, sys, time
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
from boxferry_ci import asset_verification as subject
reader, writer = os.pipe()
entered = False
class Blocked:
    headers = {"Content-Length": "1"}
    def __init__(self, url): self.url = url
    def __enter__(self): return self
    def __exit__(self, *_args): pass
    def geturl(self): return self.url
    def read(self, size):
        global entered
        entered = True
        return os.read(reader, size)
opener = SimpleNamespace(open=lambda request, **_kwargs: Blocked(request.full_url))
previous = signal.getsignal(signal.SIGALRM)
start = time.monotonic()
try:
    with (patch.object(subject, "TOTAL_SECONDS", 0.2),
          patch.object(subject.tooling, "build_opener", return_value=opener)):
        try:
            subject.verify(Path(sys.argv[1]), None)
        except subject.VerificationError as error:
            assert entered, "deadline fired before the blocking stream was reached"
            assert "tombi" in str(error) and "total time budget" in str(error), str(error)
        else:
            raise AssertionError("blocked acquisition escaped its hard deadline")
    assert time.monotonic() - start < 2
    assert signal.getsignal(signal.SIGALRM) == previous
    assert signal.getitimer(signal.ITIMER_REAL) == (0.0, 0.0)
finally:
    os.close(reader)
    os.close(writer)
"""
        result = subprocess.run(
            [sys.executable, "-c", script, str(self.root)],
            capture_output=True,
            text=True,
            check=False,
            timeout=5,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_guard_refuses_existing_timers_and_nonmain_threads_without_extending_them(self) -> None:
        with (
            patch.object(subject.signal, "getitimer", return_value=(5.0, 0.0)),
            patch.object(subject.signal, "setitimer") as timer,
            patch.object(subject.signal, "signal") as handler,
            self.assertRaisesRegex(subject.VerificationError, "own process time budget"),
        ):
            self.verify()
        timer.assert_not_called()
        handler.assert_not_called()
        with (
            patch.object(subject.threading, "current_thread", return_value=object()),
            self.assertRaisesRegex(subject.VerificationError, "main-thread wall-clock guard"),
        ):
            self.verify()

    def test_deduplication_requires_both_exact_url_and_digest(self) -> None:
        values = subject.inventory(self.root)
        with patch.object(subject, "inventory", return_value=[*values, values[0]]):
            self.assertEqual(self.verify()[:2], (15, 14))
        self.assertEqual(len(self.calls), 14)
        original = values[0]
        assert original.native is not None
        prefix = "lychee-v" if original.version.startswith("lychee-v") else "v"
        major = int(original.version.removeprefix(prefix).split(".")[0])
        next_native = replace(original.native, tag=f"{prefix}{major + 1}.0.0")
        changed = replace(
            original, version=next_native.tag, url=next_native.url, native=next_native
        )
        self.calls.clear()
        with (
            patch.object(subject, "inventory", return_value=[original, changed]),
            self.assertRaisesRegex(subject.VerificationError, "SHA256"),
        ):
            self.verify()
        self.assertEqual(self.calls, [original.url, changed.url])

    def test_cli_requires_an_explicit_source_mode(self) -> None:
        for arguments in ([], ["--working-tree", "--candidate-sha", "a" * 40]):
            with (
                self.subTest(arguments=arguments),
                patch("sys.stderr", new_callable=io.StringIO),
                self.assertRaises(SystemExit),
            ):
                subject.main(["--repository-root", str(self.root), *arguments])


if __name__ == "__main__":
    unittest.main()
