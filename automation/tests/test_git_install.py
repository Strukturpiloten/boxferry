"""Authored source archives and inert command controls; no upstream Git build."""

from __future__ import annotations

import contextlib
import hashlib
import http.client
import io
import json
import lzma
import os
import signal
import socket
import subprocess
import sys
import tarfile
import tempfile
import textwrap
import threading
import time
import unittest
from collections.abc import Buffer
from dataclasses import replace
from http.client import IncompleteRead
from pathlib import Path
from typing import Any, cast
from unittest.mock import Mock, patch

from boxferry_ci import git_install as subject
from boxferry_ci import tooling
from boxferry_ci.git_source import ORIGIN, GitSource, SourceError
from boxferry_ci.runner import Diagnostic, RunResult

VERSION = "2.56.0"
ROOT = f"git-{VERSION}"


def source(payload: bytes) -> GitSource:
    filename = f"git-{VERSION}.tar.xz"
    return GitSource(VERSION, filename, ORIGIN + filename, hashlib.sha256(payload).hexdigest())


def archive(
    extra: list[tuple[str, bytes | str, bytes]] | None = None,
    *,
    makefile: bytes = b"# Authored; must never execute.\n",
) -> bytes:
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w") as output:
        for path, payload, kind in [
            (ROOT + "/Makefile", makefile, tarfile.REGTYPE),
            (
                ROOT + "/COPYING",
                b"Authored license fixture, not upstream content.\n",
                tarfile.REGTYPE,
            ),
            (ROOT + "/GIT-VERSION-GEN", b"# Authored version fixture.\n", tarfile.REGTYPE),
            *(extra or []),
        ]:
            member = tarfile.TarInfo(path)
            member.type = kind
            if isinstance(payload, str):
                member.linkname = payload
            else:
                member.size = len(payload)
            output.addfile(member, io.BytesIO(payload) if isinstance(payload, bytes) else None)
    return lzma.compress(raw.getvalue())


def result(stdout: str = "", status: str = "success", stderr: str = "") -> RunResult:
    return RunResult(
        status,
        0 if status == "success" else 1,
        stdout,
        stderr,
        0,
        Diagnostic("fixture", "authored command failure", "fixture", "none", "fix"),
        (),
    )


class Response(io.BytesIO):
    def __init__(self, data: bytes, url: str, length: str | None = None) -> None:
        super().__init__(data)
        self.url = url
        self.headers = {} if length is None else {"Content-Length": length}

    def geturl(self) -> str:
        return self.url


class SourceDownloadTests(unittest.TestCase):
    def test_exact_bytes_and_url_then_independent_integrity(self) -> None:
        data = archive()
        selected = source(data)
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "download"
            with patch.object(subject, "build_opener") as opener:
                opener.return_value.open.return_value = Response(data, selected.url, str(len(data)))
                subject.download(selected, path)
            self.assertEqual(path.read_bytes(), data)
            self.assertEqual(opener.return_value.open.call_args.args[0].full_url, selected.url)

    def test_wrong_empty_partial_or_oversized_bytes_never_survive(self) -> None:
        data = archive()
        selected = source(data)
        for body, length in (
            (b"", None),
            (data[:-1], str(len(data))),
            (b"wrong", None),
            (data, str(subject.MAX_ARCHIVE + 1)),
        ):
            with self.subTest(length=length, size=len(body)), tempfile.TemporaryDirectory() as tmp:
                path = Path(tmp) / "download"
                with patch.object(subject, "build_opener") as opener:
                    opener.return_value.open.return_value = Response(body, selected.url, length)
                    with self.assertRaises(tooling.ToolError):
                        subject.download(selected, path)
                self.assertFalse(path.exists())

    def test_download_limits_are_enforced_without_trusting_content_length(self) -> None:
        data = archive()
        with tempfile.TemporaryDirectory() as tmp, patch.object(subject, "MAX_ARCHIVE", 1):
            path = Path(tmp) / "download"
            with patch.object(subject, "build_opener") as opener:
                opener.return_value.open.return_value = Response(data, source(data).url)
                with self.assertRaises(tooling.ToolError):
                    subject.download(source(data), path)
            self.assertFalse(path.exists())

    def test_redirect_alternate_origin_or_bad_typed_source_refuses(self) -> None:
        data = archive()
        with tempfile.TemporaryDirectory() as tmp:
            for url in ("http://www.kernel.org/a", "https://evil.invalid/a"):
                with self.subTest(url=url), patch.object(subject, "build_opener") as opener:
                    opener.return_value.open.return_value = Response(data, url)
                    with self.assertRaises(tooling.ToolError):
                        subject.download(source(data), Path(tmp) / "download")
            with self.assertRaises(tooling.ToolError):
                subject._NoRedirect().redirect_request(
                    Mock(), Mock(), 302, "moved", Mock(), "other"
                )
            with patch.object(subject, "build_opener") as opener:
                with self.assertRaises(SourceError):
                    subject.download(
                        replace(source(data), url="https://evil.invalid/a"), Path(tmp) / "x"
                    )
                opener.assert_not_called()

    def test_interrupted_response_cleans_partial_download(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "download"
            with patch.object(subject, "build_opener") as opener:
                response = Mock()
                response.__enter__ = Mock(return_value=response)
                response.__exit__ = Mock(return_value=False)
                response.geturl.return_value = source(archive()).url
                response.headers = {}
                response.read.side_effect = [b"partial", IncompleteRead(b"truncated")]
                opener.return_value.open.return_value = response
                with self.assertRaises(tooling.ToolError):
                    subject.download(source(archive()), path)
            self.assertFalse(path.exists())

    def test_missing_partial_and_noncanonical_digest_refuses_before_transport(self) -> None:
        for digest in ("", "a" * 32, "A" * 64, "g" * 64):
            with self.subTest(digest=digest), patch.object(subject, "build_opener") as opener:
                with self.assertRaises(SourceError):
                    subject.download(replace(source(archive()), sha256=digest), Path("unused"))
                opener.assert_not_called()

    def test_preexisting_download_is_never_replaced(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "existing"
            path.write_bytes(b"prior")
            with self.assertRaises(tooling.ToolError):
                subject.download(source(archive()), path)
            self.assertEqual(path.read_bytes(), b"prior")


class DownloadDeadlineTests(unittest.TestCase):
    def test_real_stdlib_buffered_slow_drip_is_interrupted_and_partial_removed(self) -> None:
        # Actual HTTPResponse/BufferedReader buffering: every body byte arrives
        # within the socket inactivity timeout, but the requested read never
        # finishes inside the whole acquisition budget. No socket is created.
        body = b"x" * 64
        headers = b"HTTP/1.1 200 OK\r\nContent-Length: 64\r\n\r\n"

        class Drip(io.RawIOBase):
            def __init__(self) -> None:
                super().__init__()
                self.header_offset = 0
                self.body_offset = 0

            def readable(self) -> bool:
                return True

            def readinto(self, buffer: Buffer) -> int:
                output = memoryview(buffer)
                if self.header_offset < len(headers):
                    count = min(len(output), len(headers) - self.header_offset)
                    output[:count] = headers[self.header_offset : self.header_offset + count]
                    self.header_offset += count
                    return count
                if self.body_offset == len(body):
                    return 0
                time.sleep(0.01)
                output[0] = body[self.body_offset]
                self.body_offset += 1
                return 1

        raw = Drip()
        buffered = io.BufferedReader(raw)

        class FakeSocket:
            def makefile(self, mode: str) -> io.BufferedIOBase:
                assert mode == "rb"
                return buffered

        response = http.client.HTTPResponse(cast(socket.socket, FakeSocket()))
        response.begin()
        selected = source(body)
        previous = signal.getsignal(signal.SIGALRM)
        with tempfile.TemporaryDirectory() as tmp:
            destination = Path(tmp) / "source.tar.xz"
            with (
                patch.object(response, "geturl", return_value=selected.url),
                patch.object(subject, "build_opener") as opener,
                patch.object(tooling, "DOWNLOAD_SECONDS", 0.03),
                patch.object(socket, "socket", side_effect=AssertionError("network forbidden")),
                patch.object(socket, "getaddrinfo", side_effect=AssertionError("DNS forbidden")),
                patch.object(subprocess, "Popen", side_effect=AssertionError("process forbidden")),
                patch.object(tooling, "_expand", side_effect=AssertionError("expansion forbidden")),
                patch.object(tooling, "_remove_partial", wraps=tooling._remove_partial) as cleanup,
            ):
                opener.return_value.open.return_value = response
                started = time.monotonic()
                with self.assertRaisesRegex(tooling.ToolError, "absolute download deadline"):
                    subject.download(selected, destination)
                elapsed = time.monotonic() - started
                self.assertLess(elapsed, 0.2, "must interrupt before the 640ms body completes")
                self.assertLess(raw.body_offset, len(body))
                self.assertFalse(destination.exists())
                self.assertIsNotNone(cleanup.call_args.args[1], "created partial inode was owned")
                opener.return_value.open.assert_called_once()
            self.assertTrue(response.isclosed())
            self.assertTrue(raw.closed)
            self.assertEqual(signal.getsignal(signal.SIGALRM), previous)
            self.assertEqual(signal.getitimer(signal.ITIMER_REAL), (0.0, 0.0))

    def test_opening_is_inside_the_same_absolute_budget(self) -> None:
        def blocked_open(*args: object, **kwargs: object) -> None:
            time.sleep(0.4)
            self.fail("opening was not interrupted")

        with tempfile.TemporaryDirectory() as tmp:
            with (
                patch.object(subject, "build_opener") as opener,
                patch.object(tooling, "DOWNLOAD_SECONDS", 0.02),
            ):
                opener.return_value.open.side_effect = blocked_open
                started = time.monotonic()
                with self.assertRaisesRegex(tooling.ToolError, "absolute download deadline"):
                    subject.download(source(archive()), Path(tmp) / "download")
                self.assertLess(time.monotonic() - started, 0.2)
                self.assertFalse((Path(tmp) / "download").exists())
        self.assertEqual(signal.getitimer(signal.ITIMER_REAL), (0.0, 0.0))

    def test_original_read_exception_and_handler_restored_before_owned_cleanup(self) -> None:
        original_handler = signal.getsignal(signal.SIGALRM)
        events: list[str] = []

        def unrelated_alarm(_number: int, _frame: object) -> None:
            events.append("prior handler")

        signal.signal(signal.SIGALRM, unrelated_alarm)
        failure = RuntimeError("original authored read exception")
        original_cleanup = tooling._remove_partial

        def cleanup(path: Path, identity: tuple[int, int] | None) -> None:
            self.assertEqual(signal.getsignal(signal.SIGALRM), unrelated_alarm)
            self.assertEqual(signal.getitimer(signal.ITIMER_REAL), (0.0, 0.0))
            self.assertIsNotNone(identity)
            events.append("owned cleanup")
            original_cleanup(path, identity)

        try:
            with tempfile.TemporaryDirectory() as tmp:
                selected = source(archive())
                response = Response(b"", selected.url)
                with (
                    patch.object(subject, "build_opener") as opener,
                    patch.object(response, "read", side_effect=[b"partial", failure]),
                    patch.object(tooling, "_remove_partial", side_effect=cleanup),
                ):
                    opener.return_value.open.return_value = response
                    with self.assertRaises(RuntimeError) as caught:
                        subject.download(selected, Path(tmp) / "download")
                self.assertIs(caught.exception, failure)
                self.assertTrue(response.closed)
                self.assertFalse((Path(tmp) / "download").exists())
            self.assertEqual(events, ["owned cleanup"])
            self.assertEqual(signal.getsignal(signal.SIGALRM), unrelated_alarm)
        finally:
            signal.setitimer(signal.ITIMER_REAL, 0)
            signal.signal(signal.SIGALRM, original_handler)

    def test_existing_alarm_is_refused_without_replacing_or_resetting_it(self) -> None:
        previous = signal.getsignal(signal.SIGALRM)
        signal.setitimer(signal.ITIMER_REAL, 5, 1)
        try:
            with tempfile.TemporaryDirectory() as tmp:
                with patch.object(subject, "build_opener") as opener:
                    with self.assertRaisesRegex(tooling.ToolError, "existing alarm"):
                        subject.download(source(archive()), Path(tmp) / "download")
                    opener.assert_not_called()
            remaining, interval = signal.getitimer(signal.ITIMER_REAL)
            self.assertGreater(remaining, 4)
            self.assertEqual(interval, 1)
            self.assertEqual(signal.getsignal(signal.SIGALRM), previous)
        finally:
            signal.setitimer(signal.ITIMER_REAL, 0)

    def test_blocked_or_pending_alarm_refuses_before_acquisition(self) -> None:
        previous_mask = signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGALRM})
        try:
            with tempfile.TemporaryDirectory() as tmp:
                with patch.object(subject, "build_opener") as opener:
                    with self.assertRaisesRegex(tooling.ToolError, "unblocked, non-pending"):
                        subject.download(source(archive()), Path(tmp) / "download")
                    opener.assert_not_called()
            self.assertIn(signal.SIGALRM, signal.pthread_sigmask(signal.SIG_BLOCK, ()))
        finally:
            signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
        with tempfile.TemporaryDirectory() as tmp:
            with (
                patch.object(signal, "sigpending", return_value={signal.SIGALRM}),
                patch.object(subject, "build_opener") as opener,
            ):
                with self.assertRaisesRegex(tooling.ToolError, "unblocked, non-pending"):
                    subject.download(source(archive()), Path(tmp) / "download")
                opener.assert_not_called()
        self.assertEqual(signal.getitimer(signal.ITIMER_REAL), (0.0, 0.0))

    def test_non_main_thread_refuses_without_signal_or_transport_mutation(self) -> None:
        errors: list[BaseException] = []
        with tempfile.TemporaryDirectory() as tmp:

            def attempt() -> None:
                try:
                    subject.download(source(archive()), Path(tmp) / "download")
                except BaseException as error:
                    errors.append(error)

            previous = signal.getsignal(signal.SIGALRM)
            with patch.object(subject, "build_opener") as opener:
                thread = threading.Thread(target=attempt)
                thread.start()
                thread.join(timeout=2)
                self.assertFalse(thread.is_alive())
                opener.assert_not_called()
            self.assertEqual(len(errors), 1)
            self.assertIsInstance(errors[0], tooling.ToolError)
            self.assertIn("Linux main-thread", str(errors[0]))
            self.assertEqual(signal.getsignal(signal.SIGALRM), previous)
            self.assertEqual(signal.getitimer(signal.ITIMER_REAL), (0.0, 0.0))


class SourceExtractionTests(unittest.TestCase):
    def unpack(self, data: bytes, root: Path) -> Path:
        path = root / "download.tar.xz"
        path.write_bytes(data)
        return subject.extract(source(data), path, root / "source")

    def test_safe_tree_and_exact_inert_symlink_omissions(self) -> None:
        data = archive(
            [
                (ROOT + "/RelNotes", f"Documentation/RelNotes/{VERSION}.adoc", tarfile.SYMTYPE),
                (ROOT + "/subprojects/git-gui", "../git-gui", tarfile.SYMTYPE),
                (ROOT + "/subprojects/gitk", "../gitk-git", tarfile.SYMTYPE),
                (ROOT + "/src/code.c", b"authored fixture\n", tarfile.REGTYPE),
            ]
        )
        with tempfile.TemporaryDirectory() as tmp:
            tree = self.unpack(data, Path(tmp))
            self.assertEqual((tree / "src/code.c").read_bytes(), b"authored fixture\n")
            self.assertFalse((tree / "RelNotes").exists())
            self.assertFalse((tree / "subprojects").exists())
            self.assertFalse(list(Path(tmp).glob("*.expanded.tar")))

    def test_checksum_precedes_even_decompression(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "bad"
            path.write_bytes(b"not a tar archive")
            with patch.object(tooling, "_expand") as expand:
                with self.assertRaises(tooling.ToolError):
                    subject.extract(source(archive()), path, Path(tmp) / "source")
                expand.assert_not_called()

    def test_traversal_absolute_foreign_duplicate_and_links_devices_refuse(self) -> None:
        cases = [
            ("../escape", b"bad", tarfile.REGTYPE),
            ("/absolute", b"bad", tarfile.REGTYPE),
            (ROOT + "/../escape", b"bad", tarfile.REGTYPE),
            ("other-root/file", b"bad", tarfile.REGTYPE),
            (ROOT + "/Makefile", b"duplicate", tarfile.REGTYPE),
            (ROOT + "/evil", "../../escape", tarfile.SYMTYPE),
            (ROOT + "/RelNotes", "../../escape", tarfile.SYMTYPE),
            (ROOT + "/hard", ROOT + "/Makefile", tarfile.LNKTYPE),
            (ROOT + "/device", b"", tarfile.CHRTYPE),
            (ROOT + "/fifo", b"", tarfile.FIFOTYPE),
        ]
        for row in cases:
            with self.subTest(row=row), tempfile.TemporaryDirectory() as tmp:
                with self.assertRaises(tooling.ToolError):
                    self.unpack(archive([row]), Path(tmp))
                self.assertFalse((Path(tmp) / "escape").exists())

    def test_member_file_decompression_and_time_bounds(self) -> None:
        for name, limit in (("MAX_MEMBERS", 2), ("MAX_SOURCE_FILE", 1)):
            with self.subTest(limit=name), tempfile.TemporaryDirectory() as tmp:
                with patch.object(subject, name, limit), self.assertRaises(tooling.ToolError):
                    self.unpack(archive(), Path(tmp))
        with tempfile.TemporaryDirectory() as tmp, patch.object(tooling, "MAX_UNPACKED", 128):
            with self.assertRaises(tooling.ToolError):
                self.unpack(archive(), Path(tmp))
        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(subject.time, "monotonic", side_effect=(0, 0, 0, 61)):
                with self.assertRaises(tooling.ToolError):
                    self.unpack(archive(), Path(tmp))

    def test_truncated_and_trailing_xz_and_missing_build_closure(self) -> None:
        for data in (archive()[:-3], archive() + b"trailing"):
            with self.subTest(size=len(data)), tempfile.TemporaryDirectory() as tmp:
                with self.assertRaises(tooling.ToolError):
                    self.unpack(data, Path(tmp))
        raw = io.BytesIO()
        with tarfile.open(fileobj=raw, mode="w"):
            pass
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(tooling.ToolError, "build/license/version closure"):
                self.unpack(lzma.compress(raw.getvalue()), Path(tmp))


class BuildFixture:
    """No subprocess: imitate installed layout, inspect exact bounded argv/env."""

    def __init__(self, fail: str = "", missing: str = "") -> None:
        self.calls: list[tuple[tuple[str, ...], dict[str, object]]] = []
        self.prefix: Path | None = None
        self.fail = fail
        self.missing = missing

    def __call__(self, wrapped: tuple[str, ...], **options: object) -> RunResult:
        self.calls.append((wrapped, options))
        argv = wrapped[4:]
        if argv[0] == "/usr/bin/make":
            if argv[-1] == self.fail:
                return result(status="failed")
            if argv[-1] == "install":
                final = Path(
                    next(arg.removeprefix("prefix=") for arg in argv if arg.startswith("prefix="))
                )
                stage = Path(
                    next(arg.removeprefix("DESTDIR=") for arg in argv if arg.startswith("DESTDIR="))
                )
                self.prefix = stage / final.relative_to("/")
                for name in (
                    "bin/git",
                    "libexec/git-core/git-remote-http",
                    "libexec/git-core/git-remote-https",
                    "libexec/git-core/git-http-fetch",
                    "libexec/git-core/git-http-push",
                    "libexec/git-core/git-send-email",
                    "share/perl5/Git.pm",
                    "share/perl5/Git/I18N.pm",
                    "share/git-core/templates/description",
                    "share/git-core/templates/info/exclude",
                    "share/git-core/templates/hooks/applypatch-msg.sample",
                ):
                    if name == self.missing:
                        continue
                    path = self.prefix / name
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text("authored runtime fixture\n")
                    path.chmod(0o755 if name.startswith(("bin/", "libexec/")) else 0o644)
        elif argv[0] == "/usr/bin/perl":
            return result("git-perl-ok\n" if self.fail != "perl" else "wrong library\n")
        elif "--version" in argv:
            return result(
                f"git version {VERSION}\n" if self.fail != "version" else "git version 0.0.0\n"
            )
        elif "--exec-path" in argv:
            assert self.prefix is not None
            return result(
                str(self.prefix / "libexec/git-core") + "\n"
                if self.fail != "helper"
                else "/wrong/prefix\n"
            )
        elif "GIT_TEMPLATE_DIR" in argv:
            assert self.prefix is not None
            return result(
                str(self.prefix / "share/git-core/templates") + "\n"
                if self.fail != "templates"
                else "/wrong/templates\n"
            )
        return result()


class SourceInstallTests(unittest.TestCase):
    def install(self, root: Path, execute: BuildFixture) -> dict[str, object]:
        data = archive()

        def write(_source: GitSource, path: Path) -> None:
            path.write_bytes(data)

        return subject.install(
            root / "git",
            source(data),
            downloader=write,
            execute=execute,
        )

    def test_common_fixture_reaches_build_runtime_and_failure_on_both_native_hosts(self) -> None:
        identities: list[object] = []
        for architecture in ("x86_64", "aarch64"):
            with self.subTest(host=architecture, outcome="complete"):
                fixture = BuildFixture()
                with (
                    tempfile.TemporaryDirectory() as tmp,
                    patch.object(tooling, "host_architecture", return_value=architecture),
                    patch.object(subject, "_https_probe") as https,
                ):
                    evidence = self.install(Path(tmp), fixture)
                    self.assertEqual(evidence["architecture"], architecture)
                    self.assertTrue(
                        (Path(tmp) / "git/share/doc/git/boxferry-source.json").is_file()
                    )
                    makes = [row[0] for row in fixture.calls if row[0][4] == "/usr/bin/make"]
                    self.assertEqual([row[-1] for row in makes], ["all", "install"])
                    self.assertTrue(any(row[0][4] == "/usr/bin/perl" for row in fixture.calls))
                    https.assert_called_once()
                    identities.append(evidence["source"])
            with self.subTest(host=architecture, outcome="install-failure"):
                fixture = BuildFixture(fail="install")
                with (
                    tempfile.TemporaryDirectory() as tmp,
                    patch.object(tooling, "host_architecture", return_value=architecture),
                    patch.object(subject, "_https_probe") as https,
                ):
                    with self.assertRaisesRegex(tooling.ToolError, "authored command failure"):
                        self.install(Path(tmp), fixture)
                    self.assertEqual([row[0][-1] for row in fixture.calls], ["all", "install"])
                    self.assertFalse((Path(tmp) / "git").exists())
                    self.assertFalse(list(Path(tmp).glob(".boxferry-git-*")))
                    https.assert_not_called()
        self.assertEqual(identities[0], identities[1])

    def test_staged_complete_prefix_and_provenance_without_build_or_inherited_overrides(
        self,
    ) -> None:
        fixture = BuildFixture()
        with tempfile.TemporaryDirectory() as tmp, patch.object(subject, "_https_probe") as https:
            root = Path(tmp)
            with patch.dict(
                os.environ, {"MAKEFLAGS": "-j99", "NO_CURL": "1", "GIT_EXEC_PATH": "/bad"}
            ):
                evidence = self.install(root, fixture)
            self.assertEqual(evidence["prefix"], str(root / "git"))
            self.assertTrue((root / "git/share/doc/git/COPYING").is_file())
            recorded = json.loads((root / "git/share/doc/git/boxferry-source.json").read_text())
            self.assertEqual(recorded["source"]["sha256"], source(archive()).sha256)
            self.assertFalse(list(root.glob(".boxferry-git-*")))
            https.assert_called_once()
            makes = [row for row in fixture.calls if row[0][4] == "/usr/bin/make"]
            self.assertEqual([row[0][-1] for row in makes], ["all", "install"])
            self.assertEqual(makes[0][0][:-1], makes[1][0][:-1])
            for wrapped, options in fixture.calls:
                self.assertEqual(wrapped[:3], (subject.sys.executable, "-I", "-c"))
                self.assertIn("resource.RLIMIT_AS", wrapped[3])
                self.assertNotIn("resource.RLIMIT_NPROC", wrapped[3])
                env = cast(dict[str, str], options["environment"])
                self.assertNotIn("MAKEFLAGS", env)
                self.assertNotIn("NO_CURL", env)
                self.assertNotIn("GIT_EXEC_PATH", env)
                self.assertLessEqual(cast(float, options["timeout_seconds"]), subject.BUILD_SECONDS)
            self.assertIn("-j2", makes[0][0])
            self.assertIn(f"prefix={root}/git", makes[0][0])
            self.assertIn(f"bindir={root}/git/bin", makes[0][0])
            self.assertIn(f"perllibdir={root}/git/share/perl5", makes[0][0])
            self.assertNotIn("NO_PERL=YesPlease", makes[0][0])
            self.assertNotIn("NO_CURL=YesPlease", makes[0][0])

    def test_build_install_version_and_missing_runtime_fail_without_publication(self) -> None:
        for fail in ("all", "install", "version", "helper", "templates", "perl"):
            with self.subTest(fail=fail), tempfile.TemporaryDirectory() as tmp:
                with patch.object(subject, "_https_probe"):
                    with self.assertRaises(tooling.ToolError):
                        self.install(Path(tmp), BuildFixture(fail))
                self.assertFalse((Path(tmp) / "git").exists())
                self.assertFalse(list(Path(tmp).glob(".boxferry-git-*")))
        for missing in (
            "bin/git",
            "libexec/git-core/git-remote-https",
            "share/perl5/Git.pm",
            "share/git-core/templates/info/exclude",
        ):
            with self.subTest(missing=missing), tempfile.TemporaryDirectory() as tmp:
                with self.assertRaises(tooling.ToolError):
                    self.install(Path(tmp), BuildFixture(missing=missing))
                self.assertFalse((Path(tmp) / "git").exists())

    def test_corrupt_downloader_never_runs_archive_or_build(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            execute = Mock()

            def corrupt(_source: GitSource, path: Path) -> None:
                path.write_bytes(b"corrupt")

            with self.assertRaisesRegex(tooling.ToolError, "SHA256 mismatch before extraction"):
                subject.install(
                    Path(tmp) / "git",
                    source(archive()),
                    downloader=corrupt,
                    execute=execute,
                )
            execute.assert_not_called()
            self.assertFalse((Path(tmp) / "git").exists())

    def test_unsupported_architecture_version_or_existing_linked_prefix_is_untouched(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with self.assertRaises(tooling.ToolError):
                subject.install(root / "git", source(archive()), architecture="mips")
            legacy = GitSource(
                "2.47.0", "git-2.47.0.tar.xz", ORIGIN + "git-2.47.0.tar.xz", "a" * 64
            )
            with self.assertRaises(tooling.ToolError):
                subject.install(root / "git", legacy)
            existing = root / "git"
            existing.mkdir()
            (existing / "sentinel").write_text("preserve")
            with self.assertRaises(tooling.ToolError):
                self.install(root, BuildFixture())
            self.assertEqual((existing / "sentinel").read_text(), "preserve")
            link = root / "link"
            link.symlink_to(root, target_is_directory=True)
            with self.assertRaises(tooling.ToolError):
                self.install(link, BuildFixture())

    def test_minimum_supported_version_is_an_explicit_boundary(self) -> None:
        minimum = GitSource("2.48.0", "git-2.48.0.tar.xz", ORIGIN + "git-2.48.0.tar.xz", "a" * 64)
        self.assertEqual(subject.validated(minimum), minimum)

    def test_aarch64_uses_the_same_source_only_on_a_native_aarch64_host(self) -> None:
        payload = archive()

        def write(_source: GitSource, path: Path) -> None:
            path.write_bytes(payload)

        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(tooling, "host_architecture", return_value="aarch64"):
                with patch.object(subject, "_https_probe"):
                    evidence = subject.install(
                        Path(tmp) / "git",
                        source(payload),
                        architecture="aarch64",
                        downloader=write,
                        execute=BuildFixture(),
                    )
            self.assertEqual(evidence["architecture"], "aarch64")
            self.assertEqual(evidence["source"]["sha256"], source(payload).sha256)

    def test_native_architecture_and_no_replace_publication(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with patch.object(tooling, "host_architecture", return_value="x86_64"):
                with self.assertRaises(tooling.ToolError):
                    subject.install(root / "git", source(archive()), architecture="aarch64")
            staging, target = root / "staged", root / "target"
            staging.mkdir()
            target.mkdir()
            (target / "sentinel").write_text("preserved")
            with self.assertRaises(OSError):
                subject._publish(staging, target)
            self.assertEqual((target / "sentinel").read_text(), "preserved")
            self.assertTrue(staging.is_dir())

    def test_inventory_requires_one_source_and_rejects_partial_or_linked_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "inventory.json"
            path.write_text(json.dumps({"schema": 1, "provenance": {}, "assets": []}))
            with self.assertRaises(tooling.ToolError):
                subject.load_source(path)
            path.write_text("{partial")
            with self.assertRaises(tooling.ToolError):
                subject.load_source(path)
            path.unlink()
            path.symlink_to("absent")
            with self.assertRaises(tooling.ToolError):
                subject.load_source(path)

    def test_runtime_tls_failure_or_deadline_failure_is_not_published(self) -> None:
        with (
            tempfile.TemporaryDirectory() as tmp,
            patch.object(subject, "_https_probe", side_effect=tooling.ToolError("TLS failure")),
        ):
            with self.assertRaises(tooling.ToolError):
                self.install(Path(tmp), BuildFixture())
            self.assertFalse((Path(tmp) / "git").exists())
        with tempfile.TemporaryDirectory() as tmp, patch.object(subject, "INSTALL_SECONDS", -1):
            with self.assertRaises(tooling.ToolError):
                self.install(Path(tmp), BuildFixture())
            self.assertFalse((Path(tmp) / "git").exists())

    def test_filesystem_budget_and_runtime_escape_refusal(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "file").write_bytes(b"xx")
            for max_bytes, max_files in ((1, 10), (10, 0)):
                with self.assertRaises(tooling.ToolError):
                    subject._files(root, max_bytes, max_files)
            (root / "escape").symlink_to("/etc/passwd")
            with self.assertRaises(tooling.ToolError):
                subject._runtime(root, source(archive()), root, {}, time.monotonic() + 30, Mock())


class HTTPSContractTests(unittest.TestCase):
    def test_local_tls_probe_requires_verified_trust_and_untrusted_refusal(self) -> None:
        for reject_status in ("failed", "success", "timeout", "wrong-failure"):
            calls: list[tuple[str, ...]] = []

            def execute(
                wrapped: tuple[str, ...],
                *,
                call_log: list[tuple[str, ...]] = calls,
                rejected: str = reject_status,
                **options: object,
            ) -> RunResult:
                argv = wrapped[4:]
                call_log.append(argv)
                if len(call_log) == 3:
                    if rejected == "wrong-failure":
                        return result(status="failed", stderr="unexpected unrelated failure")
                    return result(status=rejected, stderr="SSL certificate: self-signed")
                return result()

            with (
                tempfile.TemporaryDirectory() as tmp,
                patch.object(subject.ssl, "SSLContext") as tls,
                patch.object(subject, "CA_BUNDLE", Path(tmp) / "base-ca.pem"),
            ):
                (Path(tmp) / "base-ca.pem").write_text("authored inert CA fixture\n")
                tls.return_value.wrap_socket.side_effect = lambda sock, **_kw: sock
                if reject_status == "failed":
                    manager = contextlib.nullcontext()
                else:
                    manager = self.assertRaises(tooling.ToolError)
                with manager:
                    subject._https_probe(
                        Path("/synthetic/git"), Path(tmp), {}, time.monotonic() + 30, execute
                    )
                self.assertEqual(calls[0][:2], ("/usr/bin/openssl", "req"))
                for command in calls[1:]:
                    self.assertIn("http.sslVerify=true", command)
                    self.assertIn("http.proxy=", command)
                    self.assertTrue(command[-1].startswith("https://localhost:"))
                self.assertIn("probe-cert.pem", calls[1][-3])
                self.assertNotEqual(calls[1][-3], calls[2][-3])

    def test_missing_base_ca_fails_before_running_commands(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            execute = Mock()
            with patch.object(subject, "CA_BUNDLE", Path(tmp) / "absent-ca.pem"):
                with self.assertRaises(tooling.ToolError):
                    subject._https_probe(
                        Path("/synthetic/git"), Path(tmp), {}, time.monotonic() + 30, execute
                    )
            execute.assert_not_called()


# Authored executable fixture: never dispatches host Git or an upstream build.
# Its local TLS client exercises real certificate verification in both directions.
FIXTURE_GIT = r"""
import http.client
import pathlib
import ssl
import sys
from urllib.parse import urlsplit
prefix = pathlib.Path(__file__).resolve().parents[1]
args = sys.argv[1:]
if args == ['--version']:
    print('git version 2.56.0')
elif args == ['--exec-path']:
    print(prefix / 'libexec/git-core')
elif args == ['var', 'GIT_TEMPLATE_DIR']:
    print(prefix / 'share/git-core/templates')
elif 'ls-remote' in args:
    assert 'http.sslVerify=true' in args and 'http.proxy=' in args
    ca = next(value.split('=', 1)[1] for value in args if value.startswith('http.sslCAInfo='))
    url = urlsplit(args[-1])
    assert url.scheme == 'https' and url.hostname == 'localhost'
    context = ssl.create_default_context(cafile=ca)
    client = http.client.HTTPSConnection(url.hostname, url.port, context=context, timeout=5)
    try:
        client.request('GET', '/info/refs')
        response = client.getresponse()
        assert response.status == 200 and response.read() == b''
    except ssl.SSLCertVerificationError:
        print('SSL certificate: verification failed', file=sys.stderr)
        sys.exit(1)
    finally:
        client.close()
else:
    raise AssertionError('unexpected fixture argv')
"""


class ActualRunnerTests(unittest.TestCase):
    def base_ca(self, root: Path) -> Path:
        cert = root / "base-ca.pem"
        completed = subprocess.run(
            [
                "/usr/bin/openssl",
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-keyout",
                str(root / "base-key.pem"),
                "-out",
                str(cert),
                "-days",
                "1",
                "-subj",
                "/CN=authored-fixture-base",
            ],
            capture_output=True,
            timeout=20,
            check=False,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr.decode())
        return cert

    def payload(self, root: Path, mode: str = "success") -> bytes:
        runtime = {
            "bin/git": "#!" + sys.executable + "\n" + textwrap.dedent(FIXTURE_GIT),
            **{
                "libexec/git-core/" + name: "#!/bin/sh\nexit 99\n"
                for name in (
                    "git-remote-http",
                    "git-remote-https",
                    "git-http-fetch",
                    "git-http-push",
                    "git-send-email",
                )
            },
            "share/perl5/Git.pm": "package Git; 1;\n",
            "share/perl5/Git/I18N.pm": "package Git::I18N; 1;\n",
            "share/git-core/templates/description": "Authored template\n",
            "share/git-core/templates/info/exclude": "# Authored exclusions\n",
            "share/git-core/templates/hooks/applypatch-msg.sample": "#!/bin/sh\nexit 0\n",
        }
        builder = (
            "import json,os,pathlib,subprocess,sys,time\n"
            f"mode={mode!r}\nmarker=pathlib.Path({str(root / 'owned-pids.json')!r})\n"
            "if sys.argv[1] == 'all' and mode in ('timeout','budget','output'):\n"
            " child=subprocess.Popen([sys.executable,'-I','-c','import time;time.sleep(30)'])\n"
            " marker.write_text(json.dumps([os.getpid(),child.pid]))\n"
            " if mode == 'budget': pathlib.Path('overflow').write_bytes(b'x'*65536)\n"
            " if mode == 'output': print('x'*1048577,flush=True)\n"
            " time.sleep(30)\n"
            "if mode == sys.argv[1] + '-failure': sys.exit(9)\n"
            "if sys.argv[1] == 'install':\n"
            " prefix=pathlib.Path(sys.argv[2])/pathlib.Path(sys.argv[3]).relative_to('/')\n"
            f" for name,content in {runtime!r}.items():\n"
            "  path=prefix/name;path.parent.mkdir(parents=True,exist_ok=True)\n"
            "  path.write_text(content)\n"
            "  path.chmod(0o755 if name.startswith(('bin/','libexec/')) else 0o644)\n"
        )
        makefile = (
            f"all:\n\t{sys.executable} -I build-fixture.py all\n"
            f'install:\n\t{sys.executable} -I build-fixture.py install "$(DESTDIR)" "$(prefix)"\n'
        ).encode()
        return archive(
            [(ROOT + "/build-fixture.py", builder.encode(), tarfile.REGTYPE)], makefile=makefile
        )

    def install(self, root: Path, mode: str = "success") -> dict[str, Any]:
        payload = self.payload(root, mode)

        def write(_source: GitSource, path: Path) -> None:
            path.write_bytes(payload)

        return subject.install(root / "git", source(payload), downloader=write)

    def test_real_make_wrapper_perl_runtime_and_tls_without_host_git(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ca = self.base_ca(root)
            with (
                patch.object(subject, "CA_BUNDLE", ca),
                patch.dict(
                    os.environ,
                    {"SSL_CERT_FILE": "/absent-caller-trust", "GIT_EXEC_PATH": "/absent-host-git"},
                ),
            ):
                evidence = self.install(root)
            prefix = root / "git"
            self.assertEqual(evidence["source"]["sha256"], source(self.payload(root)).sha256)
            self.assertEqual(evidence["prefix"], str(prefix))
            self.assertEqual(
                evidence["license"]["sha256"],
                hashlib.sha256((prefix / "share/doc/git/COPYING").read_bytes()).hexdigest(),
            )
            recorded = json.loads((prefix / "share/doc/git/boxferry-source.json").read_text())
            self.assertEqual(
                recorded["validated"][-2:], ["HTTPS verified local CA", "untrusted TLS CA refused"]
            )
            self.assertEqual((prefix / "share/perl5/Git.pm").read_text(), "package Git; 1;\n")
            self.assertTrue((prefix / "libexec/git-core/git-remote-https").is_file())
            self.assertFalse(list(root.glob(".boxferry-git-*")))

    def test_real_build_failures_limits_and_owned_child_cleanup(self) -> None:
        for mode in ("all-failure", "install-failure", "timeout", "budget", "output"):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                sentinel = root / "unrelated"
                sentinel.write_text("preserve")
                with (
                    patch.object(subject, "BUILD_SECONDS", 0.5 if mode == "timeout" else 10),
                    patch.object(
                        subject,
                        "MAX_BUILD_BYTES",
                        32768 if mode == "budget" else subject.MAX_BUILD_BYTES,
                    ),
                ):
                    expected = (
                        "file/byte budget" if mode == "budget" else "Git source command failed"
                    )
                    with self.assertRaisesRegex(tooling.ToolError, expected):
                        self.install(root, mode)
                self.assertFalse((root / "git").exists())
                self.assertFalse(list(root.glob(".boxferry-git-*")))
                self.assertEqual(sentinel.read_text(), "preserve")
                marker = root / "owned-pids.json"
                if mode in ("timeout", "budget", "output"):
                    self.assertTrue(marker.is_file(), "actual owned children must have started")
                    for pid in json.loads(marker.read_text()):
                        status = Path(f"/proc/{pid}/status")
                        if status.exists():
                            self.assertRegex(
                                status.read_text(), r"State:\s+Z", "owned process still running"
                            )


if __name__ == "__main__":
    unittest.main()
