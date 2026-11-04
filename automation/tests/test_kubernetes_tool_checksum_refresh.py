"""Offline contracts for trusted Kubernetes tool checksum refresh."""

from __future__ import annotations

import importlib
import io
import json
import os
import re
import socket
import ssl
import subprocess
import sys
import tempfile
import time
import types
import unittest
from collections.abc import Callable, Mapping
from contextlib import redirect_stdout
from http.client import HTTPConnection, HTTPMessage
from pathlib import Path
from types import TracebackType
from typing import Any, ClassVar, Self, cast
from unittest.mock import patch
from urllib import error as urllib_error
from urllib.request import Request

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "automation/src"))
module = importlib.import_module("boxferry_ci.kubernetes_tool_checksum_refresh")
_open_with_ipv4_fallback = cast(Any, module)._open_with_ipv4_fallback


def bump_patch(version: str) -> str:
    major, minor, patch_version = map(int, version.split("."))
    return f"{major}.{minor}.{patch_version + 1}"


def next_versions(source: str) -> dict[str, str]:
    versions, _ = module.parse_installer(source)
    return {
        tool: bump_patch(version)
        for tool, version in versions.items()
        if tool in module.VERSION_PATTERNS
    }


def candidate_installer(source: str) -> str:
    updated = source
    for tool, version in next_versions(source).items():
        pattern = rf'(?m)^(readonly {tool}_version=")[^"]+("$)'
        updated, count = re.subn(pattern, rf"\g<1>{version}\g<2>", updated, count=1)
        assert count == 1
    kind_match = module.KIND_RE.search(source)
    assert kind_match is not None
    kind_tag = "v" + bump_patch(kind_match.group("tag")[1:])

    def update_kind(match: re.Match[str]) -> str:
        digest = "a" * 64 if match.group("arch") == "amd64" else "b" * 64
        return (
            match.group(0)
            .replace(match.group("tag"), kind_tag)
            .replace(match.group("digest"), digest)
        )

    return module.KIND_RE.sub(update_kind, updated)


def sample_checksums() -> dict[str, str]:
    names = ("kubectl", "helm", "kustomize")
    return {
        "kind_amd64": "a" * 64,
        "kind_arm64": "b" * 64,
        **{
            f"{tool}_{architecture}": str(index) * 64
            for index, (architecture, tool) in enumerate(
                (
                    pair
                    for architecture in ("amd64", "arm64")
                    for pair in ((architecture, tool) for tool in names)
                ),
                start=1,
            )
        },
    }


def checksum_fixture_fetcher(
    versions: dict[str, str], checksums: dict[str, str]
) -> tuple[Callable[..., bytes], list[tuple[str, set[str], int]]]:
    requested: list[tuple[str, set[str], int]] = []

    def fetch(url: str, *, allowed_hosts: set[str], max_bytes: int, **_kwargs: object) -> bytes:
        requested.append((url, allowed_hosts, max_bytes))
        if "/kubectl.sha256" in url:
            architecture = "arm64" if "/arm64/" in url else "amd64"
            return f"{checksums[f'kubectl_{architecture}']}\n".encode()
        if url.endswith(".sha256sum"):
            architecture = "arm64" if "linux-arm64" in url else "amd64"
            filename = f"helm-v{versions['helm']}-linux-{architecture}.tar.gz"
            return f"{checksums[f'helm_{architecture}']} {filename}\n".encode()
        if url.endswith("/checksums.txt"):
            lines = [
                (
                    f"{checksums['kustomize_amd64']} "
                    f"kustomize_v{versions['kustomize']}_linux_amd64.tar.gz"
                ),
                (
                    f"{checksums['kustomize_arm64']} "
                    f"kustomize_v{versions['kustomize']}_linux_arm64.tar.gz"
                ),
                "c" * 64 + " unrelated.txt",
            ]
            return ("\n".join(lines) + "\n").encode()
        raise AssertionError(f"unexpected checksum metadata URL: {url}")

    return fetch, requested


def artifact_fixture_fetcher(
    checksums: dict[str, str],
    kind_checksums: dict[str, str] | None = None,
) -> tuple[Callable[..., str], list[tuple[str, set[str], int]]]:
    requested: list[tuple[str, set[str], int]] = []
    kind_values = kind_checksums or {"kind_amd64": "a" * 64, "kind_arm64": "b" * 64}

    def fetch(url: str, *, allowed_hosts: set[str], max_bytes: int, **_kwargs: object) -> str:
        requested.append((url, allowed_hosts, max_bytes))
        architecture = "arm64" if "arm64" in url else "amd64"
        if "/kind-linux-" in url:
            return kind_values[f"kind_{architecture}"]
        if url.endswith("/kubectl"):
            return checksums[f"kubectl_{architecture}"]
        if "/helm-v" in url:
            return checksums[f"helm_{architecture}"]
        if "/kustomize_v" in url:
            return checksums[f"kustomize_{architecture}"]
        raise AssertionError(f"unexpected official artifact URL: {url}")

    return fetch, requested


class FakeResponse:
    def __init__(self, body: bytes, url: str, content_length: str | None = None) -> None:
        self.body = body
        self.url = url
        self.headers = {} if content_length is None else {"Content-Length": content_length}
        self.code = 200
        self.msg = "OK"
        self.offset = 0

    def __enter__(self) -> Self:
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc_val: BaseException | None,
        traceback: TracebackType | None,
        /,
    ) -> bool | None:
        return None

    def geturl(self) -> str:
        return self.url

    def info(self) -> Mapping[str, str]:
        return self.headers

    def read(self, limit: int) -> bytes:
        result = self.body[self.offset : self.offset + limit]
        self.offset += len(result)
        return result


class FakeOpener:
    def __init__(self, response: FakeResponse, timeout: float) -> None:
        self.response = response
        self.timeout = timeout

    def open(self, request: Request, timeout: float) -> FakeResponse:
        assert timeout == self.timeout
        return self.response


class FakeOpenerRaises:
    def __init__(self, error: BaseException) -> None:
        self.error = error

    def open(self, request: Request, timeout: float) -> FakeResponse:
        raise self.error


class KubernetesChecksumRefreshTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.installer = (ROOT / "scripts/install-kubernetes-tools.sh").read_text(encoding="utf-8")

    def test_metadata_timeout_falls_back_once_and_remembers_ipv4_host(self) -> None:
        url = "https://get.helm.sh/helm-v4.3.0-linux-amd64.tar.gz.sha256sum"
        request_error = urllib_error.URLError(TimeoutError("TLS handshake timed out"))

        class FailingOpener:
            def __init__(self) -> None:
                self.requests: list[tuple[Request, float]] = []

            def open(self, request: Request, timeout: float) -> module._Response:
                self.requests.append((request, timeout))
                raise request_error

        primary = FailingOpener()
        ipv4_response = FakeResponse(b"a" * 64 + b"  helm.tar.gz\n", url)
        second_response = FakeResponse(b"b" * 64 + b"  helm.tar.gz\n", url)
        budget = module.NetworkBudget.start()
        with patch.object(
            module,
            "_build_ipv4_opener",
            side_effect=[FakeOpener(ipv4_response, 10), FakeOpener(second_response, 10)],
        ) as build_ipv4:
            result = module.fetch_bounded(
                url,
                allowed_hosts={"get.helm.sh"},
                max_bytes=module.MAX_CHECKSUM_BYTES,
                timeout=10,
                opener=primary,
                budget=budget,
            )
            second = module.fetch_bounded(
                url,
                allowed_hosts={"get.helm.sh"},
                max_bytes=module.MAX_CHECKSUM_BYTES,
                timeout=10,
                opener=primary,
                budget=budget,
            )

        self.assertEqual(result, b"a" * 64 + b"  helm.tar.gz\n")
        self.assertEqual(second, b"b" * 64 + b"  helm.tar.gz\n")
        self.assertEqual(budget.requests, 3)
        self.assertEqual(budget.ipv4_hosts, {"get.helm.sh"})
        self.assertEqual(len(primary.requests), 1)
        self.assertEqual(primary.requests[0][0].full_url, url)
        self.assertEqual(build_ipv4.call_count, 2)
        build_ipv4.assert_any_call({"get.helm.sh"})

    def test_ipv4_opener_dispatches_only_the_custom_allowlisted_handler(self) -> None:
        url = "https://get.helm.sh/metadata"
        opener = module._build_ipv4_opener({"get.helm.sh"})
        urllib_request = cast(Any, module).urllib.request
        https_handlers = [
            handler
            for handler in cast(Any, opener).handlers
            if isinstance(handler, urllib_request.HTTPSHandler)
        ]
        redirect_handlers = [
            handler
            for handler in cast(Any, opener).handlers
            if isinstance(handler, module._AllowlistedRedirectHandler)
        ]
        self.assertEqual(len(https_handlers), 1)
        self.assertIsInstance(https_handlers[0], module._IPv4HTTPSHandler)
        self.assertEqual(len(redirect_handlers), 1)
        context = https_handlers[0]._ipv4_context
        self.assertEqual(context.verify_mode, ssl.CERT_REQUIRED)
        self.assertTrue(context.check_hostname)

        response = FakeResponse(b"metadata\n", url)
        request = Request(url)
        with patch.object(
            urllib_request.HTTPSHandler,
            "do_open",
            autospec=True,
            return_value=response,
        ) as dispatch:
            with opener.open(request, timeout=7) as opened:
                self.assertEqual(opened.read(32), b"metadata\n")
        self.assertIs(dispatch.call_args.args[1], cast(Any, module)._IPv4HTTPSConnection)
        self.assertEqual(dispatch.call_args.args[2].full_url, url)

    def test_ipv4_connection_keeps_proxy_connect_and_original_tls_hostname(self) -> None:
        default_context = ssl.create_default_context()
        self.assertEqual(default_context.verify_mode, ssl.CERT_REQUIRED)
        self.assertTrue(default_context.check_hostname)
        observed_sni: list[str] = []
        observed_tunnel: list[tuple[str | None, int | None]] = []

        class RecordingContext:
            def wrap_socket(self, connection: object, *, server_hostname: str) -> object:
                observed_sni.append(server_hostname)
                return connection

        class FakeSocket:
            def __init__(self) -> None:
                self.address: object | None = None

            def settimeout(self, timeout: float) -> None:
                return None

            def connect(self, address: object) -> None:
                self.address = address

            def setsockopt(self, level: int, option: int, value: int) -> None:
                return None

            def close(self) -> None:
                return None

        fake_socket = FakeSocket()
        connection_type = cast(Any, module)._IPv4HTTPSConnection
        connection = connection_type("proxy.example", 8443, timeout=7, context=default_context)
        connection.set_tunnel("get.helm.sh", 443)
        connection._context = RecordingContext()
        ipv4_result = (socket.AF_INET, socket.SOCK_STREAM, 6, "", ("192.0.2.10", 8443))

        def record_tunnel(tunneled: HTTPConnection) -> None:
            observed_tunnel.append(
                (cast(Any, tunneled)._tunnel_host, cast(Any, tunneled)._tunnel_port)
            )

        with (
            patch.object(socket, "getaddrinfo", return_value=[ipv4_result]) as resolve,
            patch.object(socket, "socket", return_value=fake_socket),
            patch.object(HTTPConnection, "_tunnel", autospec=True, side_effect=record_tunnel),
        ):
            connection.connect()

        resolve.assert_called_once_with(
            "proxy.example", 8443, family=socket.AF_INET, type=socket.SOCK_STREAM
        )
        self.assertEqual(fake_socket.address, ("192.0.2.10", 8443))
        self.assertEqual(observed_tunnel, [("get.helm.sh", 443)])
        self.assertEqual(observed_sni, ["get.helm.sh"])

    def test_ipv4_socket_closes_when_deadline_interrupts_connect(self) -> None:
        class InterruptingSocket:
            def __init__(self) -> None:
                self.close_count = 0
                self.connect_count = 0

            def settimeout(self, timeout: float) -> None:
                return None

            def connect(self, address: object) -> None:
                self.connect_count += 1
                raise module.RefreshError("deadline interrupted socket connect")

            def close(self) -> None:
                self.close_count += 1

        connection_type = cast(Any, module)._IPv4HTTPSConnection
        connection = connection_type("get.helm.sh", timeout=7)
        interrupted = InterruptingSocket()
        addresses = [
            (socket.AF_INET, socket.SOCK_STREAM, 6, "", ("192.0.2.10", 443)),
            (socket.AF_INET, socket.SOCK_STREAM, 6, "", ("192.0.2.11", 443)),
        ]
        with (
            patch.object(socket, "getaddrinfo", return_value=addresses),
            patch.object(socket, "socket", return_value=interrupted) as create_socket,
            self.assertRaisesRegex(module.RefreshError, "deadline interrupted"),
        ):
            connection._create_ipv4_connection(("get.helm.sh", 443), 7, None)

        create_socket.assert_called_once_with(socket.AF_INET, socket.SOCK_STREAM, 6)
        self.assertEqual(interrupted.connect_count, 1)
        self.assertEqual(interrupted.close_count, 1)

    def test_ipv4_fallback_does_not_retry_certificate_or_http_errors(self) -> None:
        url = "https://get.helm.sh/metadata"
        certificate_error = urllib_error.URLError(
            ssl.SSLCertVerificationError("certificate verify failed")
        )
        budget = module.NetworkBudget.start()
        opener = FakeOpenerRaises(certificate_error)
        with (
            patch.object(module, "_build_ipv4_opener") as build_ipv4,
            self.assertRaises(urllib_error.URLError),
        ):
            _open_with_ipv4_fallback(
                Request(url),
                allowed_hosts={"get.helm.sh"},
                timeout=3,
                opener=opener,
                budget=budget,
            )
        build_ipv4.assert_not_called()
        self.assertEqual(budget.requests, 1)

        http_error = urllib_error.HTTPError(url, 404, "not found", HTTPMessage(), None)
        budget = module.NetworkBudget.start()
        with (
            patch.object(module, "_build_ipv4_opener") as build_ipv4,
            self.assertRaises(urllib_error.HTTPError),
        ):
            _open_with_ipv4_fallback(
                Request(url),
                allowed_hosts={"get.helm.sh"},
                timeout=3,
                opener=FakeOpenerRaises(http_error),
                budget=budget,
            )
        build_ipv4.assert_not_called()
        self.assertEqual(budget.requests, 1)

    def test_ipv4_fallback_consumes_the_existing_request_budget(self) -> None:
        budget = module.NetworkBudget.start()
        budget.requests = module.MAX_VERIFY_REQUESTS - 1
        with (
            patch.object(module, "_build_ipv4_opener") as build_ipv4,
            self.assertRaisesRegex(module.RefreshError, "request budget"),
        ):
            _open_with_ipv4_fallback(
                Request("https://get.helm.sh/metadata"),
                allowed_hosts={"get.helm.sh"},
                timeout=3,
                opener=FakeOpenerRaises(urllib_error.URLError(TimeoutError("connect timed out"))),
                budget=budget,
            )
        build_ipv4.assert_not_called()
        self.assertEqual(budget.requests, module.MAX_VERIFY_REQUESTS)

    def test_installer_pins_are_well_formed_for_both_architectures(self) -> None:
        versions, checksums = module.parse_installer(self.installer)
        self.assertEqual(set(versions), {"kind", "kubectl", "helm", "kustomize"})
        self.assertTrue(
            all(
                module.SEMVER_RE.fullmatch(version.removeprefix("v"))
                for version in versions.values()
            )
        )
        self.assertEqual(
            set(checksums),
            set(
                module.CHECKSUM_NAMES["kind"]
                + module.CHECKSUM_NAMES["kubectl"]
                + module.CHECKSUM_NAMES["helm"]
                + module.CHECKSUM_NAMES["kustomize"]
            ),
        )
        self.assertTrue(all(module.SHA256_RE.fullmatch(digest) for digest in checksums.values()))
        kind = list(module.KIND_RE.finditer(self.installer))
        self.assertEqual({item.group("arch") for item in kind}, {"amd64", "arm64"})
        self.assertEqual(len({item.group("tag") for item in kind}), 1)

    def test_kind_only_update_is_verified_for_both_architecture_artifacts(self) -> None:
        base = self.installer
        candidate = module.KIND_RE.sub(
            lambda match: (
                match.group(0)
                .replace(match.group("tag"), "v" + bump_patch(match.group("tag")[1:]))
                .replace(
                    match.group("digest"),
                    "a" * 64 if match.group("arch") == "amd64" else "b" * 64,
                )
            ),
            base,
        )
        _, digests = module.parse_installer(candidate)
        artifact_fetcher, artifact_requests = artifact_fixture_fetcher(
            {}, {name: digests[name] for name in module.CHECKSUM_NAMES["kind"]}
        )
        plan = module.build_refresh_plan(
            number=42,
            head_sha="a" * 40,
            head_ref="renovate/kind",
            base_sha="b" * 40,
            base_content=base,
            head_content=candidate,
            artifact_fetcher=artifact_fetcher,
        )
        self.assertIsNone(plan)
        self.assertEqual(len(artifact_requests), 2)
        self.assertTrue(all("kind-linux-" in url for url, _, _ in artifact_requests))

    def test_kind_only_update_fails_if_either_asset_digest_is_stale(self) -> None:
        base = self.installer
        candidate = module.KIND_RE.sub(
            lambda match: match.group(0).replace(
                match.group("tag"), "v" + bump_patch(match.group("tag")[1:])
            ),
            base,
        )
        artifact_fetcher, _ = artifact_fixture_fetcher({})
        with self.assertRaisesRegex(module.RefreshError, "kind amd64 artifact SHA-256"):
            module.build_refresh_plan(
                number=42,
                head_sha="a" * 40,
                head_ref="renovate/kind",
                base_sha="b" * 40,
                base_content=base,
                head_content=candidate,
                artifact_fetcher=artifact_fetcher,
            )

    def test_rejects_partial_kind_architecture_tag_update(self) -> None:
        candidate = self.installer.replace(
            'kind_arm64_release="v0.33.0"', 'kind_arm64_release="v0.34.0"'
        )
        with self.assertRaisesRegex(module.RefreshError, "same release tag"):
            module.parse_installer(candidate)

    def test_renovate_owns_both_kind_architectures_and_exact_tool_scope(self) -> None:
        configuration = json.loads((ROOT / ".github/renovate.json").read_text(encoding="utf-8"))
        managers = [
            item
            for item in configuration["customManagers"]
            if item.get("description", "").startswith("Update kind's exact release tag")
        ]
        self.assertEqual(len(managers), 1)
        extracted = [
            match
            for pattern in managers[0]["matchStrings"]
            for match in re.finditer(pattern.replace("(?<", "(?P<"), self.installer)
        ]
        self.assertEqual(len(extracted), 2)
        self.assertEqual(
            {match.group("depType") for match in extracted},
            {"kind-linux-amd64", "kind-linux-arm64"},
        )
        rules = [
            item
            for item in configuration["packageRules"]
            if item.get("description")
            == "Group Kubernetes development tool versions and integrity refreshes"
        ]
        self.assertEqual(len(rules), 1)
        self.assertEqual(
            set(rules[0]["matchPackageNames"]),
            {
                "kubernetes-sigs/kind",
                "kubernetes/kubernetes",
                "helm/helm",
                "kubernetes-sigs/kustomize",
            },
        )
        self.assertTrue(rules[0]["automerge"])
        self.assertFalse(rules[0]["dependencyDashboardApproval"])
        self.assertEqual(
            configuration["gitIgnoredAuthors"],
            ["317908760+strukturpiloten-release-plz[bot]@users.noreply.github.com"],
        )

    def test_native_renovate_checksum_asset_mapping_and_replacement(self) -> None:
        pin_path = Path(
            os.environ.get("BOXFERRY_RENOVATE_PIN", ROOT / "automation/renovate-runtime.json")
        )
        if not pin_path.is_file():
            self.skipTest("requires the shared automation foundation Renovate runtime pin")
        runtime = Path(os.environ.get("BOXFERRY_RENOVATE_PACKAGE", ROOT / "target/tools/renovate"))
        self.assertTrue(runtime.is_dir(), "provision the pinned native Renovate runtime first")
        marker = runtime / "source-image.txt"
        node = runtime / "node"
        self.assertTrue(marker.is_file(), "native Renovate runtime identity marker is missing")
        self.assertTrue(node.is_file(), "native Renovate runtime Node executable is missing")
        expected_image = json.loads(pin_path.read_text(encoding="utf-8"))["image"]
        self.assertEqual(marker.read_text(encoding="utf-8").strip(), expected_image)
        result = subprocess.run(
            [
                str(node),
                str(ROOT / "scripts/test-kubernetes-tools-renovate.cjs"),
                str(runtime / "dist"),
                str(ROOT),
            ],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
            timeout=60,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_official_metadata_and_matching_artifacts_cover_both_architectures(
        self,
    ) -> None:
        versions = next_versions(self.installer)
        checksums = sample_checksums()
        metadata_fetcher, metadata_requests = checksum_fixture_fetcher(versions, checksums)
        artifact_fetcher, artifact_requests = artifact_fixture_fetcher(checksums)
        self.assertEqual(
            module.obtain_checksums(versions, metadata_fetcher, artifact_fetcher),
            {name: digest for name, digest in checksums.items() if not name.startswith("kind_")},
        )
        self.assertEqual(len(metadata_requests), 6)
        self.assertEqual(len(artifact_requests), 6)
        self.assertTrue(
            all(url.startswith("https://") for url, _, _ in metadata_requests + artifact_requests)
        )
        self.assertTrue(all(size <= module.MAX_CHECKSUM_BYTES for _, _, size in metadata_requests))
        self.assertEqual(
            {max_bytes for _, _, max_bytes in artifact_requests},
            {
                module.MAX_KUBECTL_ARTIFACT_BYTES,
                module.MAX_HELM_ARTIFACT_BYTES,
                module.MAX_KUSTOMIZE_ARTIFACT_BYTES,
            },
        )
        self.assertTrue(any("dl.k8s.io/release" in url for url, _, _ in metadata_requests))
        self.assertTrue(any("get.helm.sh/helm-v" in url for url, _, _ in metadata_requests))
        self.assertTrue(
            any(
                "kustomize/v" in url and url.endswith("checksums.txt")
                for url, _, _ in metadata_requests
            )
        )

    def test_official_checksum_metadata_must_match_downloaded_artifact(self) -> None:
        versions = next_versions(self.installer)
        checksums = {name: "a" * 64 for name in module.CHECKSUM_PATTERNS}
        metadata_fetcher, _ = checksum_fixture_fetcher(versions, checksums)
        mismatch_fetcher, artifact_requests = artifact_fixture_fetcher(
            {name: "b" * 64 for name in checksums}
        )
        with self.assertRaisesRegex(module.RefreshError, "did not match its checksum metadata"):
            module.obtain_checksums(versions, metadata_fetcher, mismatch_fetcher)
        self.assertEqual(len(artifact_requests), 1, "digest mismatch must not be retried")

    def test_artifact_fetch_retries_transient_transport_failure_within_budget(
        self,
    ) -> None:
        class RetryOpener:
            def __init__(self) -> None:
                self.calls = 0
                self.response = FakeResponse(b"artifact", "https://get.helm.sh/file")

            def open(self, request: Request, timeout: float) -> FakeResponse:
                self.calls += 1
                self.assert_timeout(timeout)
                if self.calls == 1:
                    raise module.urllib.error.URLError("temporary network failure")
                return self.response

            @staticmethod
            def assert_timeout(timeout: float) -> None:
                assert timeout > 0

        opener = RetryOpener()
        with patch.object(module.time, "sleep"):
            digest = module.fetch_artifact_sha256(
                "https://get.helm.sh/file",
                allowed_hosts={"get.helm.sh"},
                max_bytes=10,
                opener=opener,
            )
        self.assertEqual(digest, __import__("hashlib").sha256(b"artifact").hexdigest())
        self.assertEqual(opener.calls, 2)

    @unittest.skipUnless(hasattr(module.signal, "setitimer"), "requires Linux interval timers")
    def test_deadline_interrupts_a_real_blocked_artifact_read(self) -> None:
        read_fd, write_fd = os.pipe()

        class BlockingResponse:
            headers: ClassVar[Mapping[str, str]] = {}

            def __enter__(self) -> Self:
                return self

            def __exit__(
                self,
                exc_type: type[BaseException] | None,
                exc_value: BaseException | None,
                traceback: TracebackType | None,
            ) -> bool | None:
                return None

            def geturl(self) -> str:
                return "https://get.helm.sh/file"

            def read(self, limit: int) -> bytes:
                return os.read(read_fd, 1)

        class BlockingOpener:
            def open(self, request: Request, timeout: float) -> BlockingResponse:
                assert timeout > 0
                return BlockingResponse()

        try:
            budget = module.NetworkBudget(time.monotonic() + 0.05)
            with self.assertRaisesRegex(module.RefreshError, "exceeded its time budget"):
                module.fetch_artifact_sha256(
                    "https://get.helm.sh/file",
                    allowed_hosts={"get.helm.sh"},
                    max_bytes=10,
                    opener=BlockingOpener(),
                    budget=budget,
                )
            self.assertEqual(module.signal.getitimer(module.signal.ITIMER_REAL), (0.0, 0.0))
        finally:
            os.close(read_fd)
            os.close(write_fd)

    @unittest.skipUnless(hasattr(module.signal, "setitimer"), "requires Linux interval timers")
    def test_deadline_guard_preserves_an_existing_alarm(self) -> None:
        module.signal.setitimer(module.signal.ITIMER_REAL, 2.0)
        try:
            with (
                self.assertRaisesRegex(module.RefreshError, "cannot replace an existing alarm"),
                module._deadline_guard(time.monotonic() + 0.1),
            ):
                self.fail("guard should reject an already armed timer")
            remaining, interval = module.signal.getitimer(module.signal.ITIMER_REAL)
            self.assertGreater(remaining, 1.5)
            self.assertEqual(interval, 0.0)
        finally:
            module.signal.setitimer(module.signal.ITIMER_REAL, 0)

    def test_git_object_verification_reads_only_the_exact_installer_object(
        self,
    ) -> None:
        root = Path("/candidate")
        sha = "a" * 40
        with (
            patch.object(
                module, "_installer_from_git_object", return_value="installer text"
            ) as read,
            patch.object(module, "verify_installer_integrity") as verify,
        ):
            module.verify_git_object(sha, root)
        read.assert_called_once_with(sha, root)
        verify.assert_called_once_with("installer text")

    def test_exact_git_reader_delegates_to_bounded_regular_blob_admission(self) -> None:
        root = Path("/candidate")
        sha = "a" * 40
        calls: list[tuple[object, ...]] = []
        admission = cast(Any, types.ModuleType("boxferry_ci.admission"))
        admission_error = type("AdmissionError", (RuntimeError,), {})

        def read_git(*_args: object) -> bytes:
            return b"unused"

        admission.AdmissionError = admission_error
        admission.read_git = read_git

        def tracked_blob(*args: object) -> bytes:
            calls.append(args)
            return b"installer text"

        admission._tracked_blob = tracked_blob
        with patch.dict(sys.modules, {"boxferry_ci.admission": admission}):
            result = module._installer_from_git_object(sha, root)
        self.assertEqual(result, "installer text")
        self.assertEqual(
            calls,
            [
                (
                    root.resolve(),
                    sha,
                    module.INSTALLER_PATH,
                    read_git,
                    module.MAX_INSTALLER_BYTES,
                )
            ],
        )

    def test_exact_git_reader_rejects_admission_failures(self) -> None:
        sha = "a" * 40
        admission = cast(Any, types.ModuleType("boxferry_ci.admission"))
        admission_error = type("AdmissionError", (RuntimeError,), {})
        admission.AdmissionError = admission_error
        admission.read_git = lambda *_args: b"unused"
        admission._tracked_blob = lambda *_args: (_ for _ in ()).throw(
            admission_error("installer is oversized or not a regular blob")
        )
        with (
            patch.dict(sys.modules, {"boxferry_ci.admission": admission}),
            self.assertRaisesRegex(module.RefreshError, "oversized or not a regular blob"),
        ):
            module._installer_from_git_object(sha, Path("/candidate"))

    def test_git_object_verification_rejects_non_exact_sha_before_read(self) -> None:
        with (
            patch.object(module, "verify_installer_integrity") as verify,
            self.assertRaisesRegex(module.RefreshError, "exact lowercase Git commit SHA"),
        ):
            module.verify_git_object("HEAD")
        verify.assert_not_called()

    def test_working_tree_mode_reads_the_explicit_repository_root(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            installer = root / module.INSTALLER_PATH
            installer.parent.mkdir(parents=True)
            installer.write_text("candidate installer", encoding="utf-8")
            with patch.object(module, "verify_installer_integrity") as verify:
                module.verify_working_tree(root)
            verify.assert_called_once_with("candidate installer")

    def test_working_tree_mode_rejects_symlinks_and_oversized_files(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            installer = root / module.INSTALLER_PATH
            installer.parent.mkdir(parents=True)
            target = root / "outside-installer"
            target.write_text("candidate installer", encoding="utf-8")
            installer.symlink_to(target)
            with self.assertRaisesRegex(
                module.RefreshError, "could not read working-tree installer"
            ):
                module.verify_working_tree(root)
            installer.unlink()
            installer.write_bytes(b"x" * (module.MAX_INSTALLER_BYTES + 1))
            with self.assertRaisesRegex(module.RefreshError, "bounded regular file"):
                module.verify_working_tree(root)
            installer.unlink()
            os.mkfifo(installer)
            with self.assertRaisesRegex(module.RefreshError, "bounded regular file"):
                module.verify_working_tree(root)

    def test_scan_cli_stdout_is_one_machine_readable_status_line(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            plan_path = Path(temporary) / "plan.json"
            output = io.StringIO()
            with (
                patch.dict(
                    "os.environ",
                    {
                        "GITHUB_REPOSITORY": module.REPOSITORY,
                        "GITHUB_REF": "refs/heads/main",
                    },
                ),
                patch.object(module, "scan", return_value=None),
                redirect_stdout(output),
            ):
                module.run("read-token", scan_only=True, plan_file=plan_path)
            self.assertEqual(output.getvalue(), "refresh=false\n")
            self.assertEqual(plan_path.read_text(encoding="utf-8"), "{}\n")

    def test_artifact_stream_is_bounded_redirect_checked_and_not_materialized(
        self,
    ) -> None:
        allowed = {"get.helm.sh"}
        too_large = FakeOpener(FakeResponse(b"x" * 11, "https://get.helm.sh/file", "11"), 300)
        with self.assertRaisesRegex(module.RefreshError, "exceeded 10 bytes"):
            module.fetch_artifact_sha256(
                "https://get.helm.sh/file",
                allowed_hosts=allowed,
                max_bytes=10,
                opener=too_large,
            )
        bad_redirect = FakeOpener(FakeResponse(b"x", "https://evil.invalid/file"), 300)
        with self.assertRaisesRegex(module.RefreshError, "untrusted metadata URL"):
            module.fetch_artifact_sha256(
                "https://get.helm.sh/file",
                allowed_hosts=allowed,
                max_bytes=10,
                opener=bad_redirect,
            )
        good = FakeOpener(FakeResponse(b"artifact", "https://get.helm.sh/file"), 300)
        self.assertEqual(
            module.fetch_artifact_sha256(
                "https://get.helm.sh/file",
                allowed_hosts=allowed,
                max_bytes=10,
                opener=good,
            ),
            "".join(f"{byte:02x}" for byte in __import__("hashlib").sha256(b"artifact").digest()),
        )

    def test_refresh_plan_only_changes_external_checksum_fields(self) -> None:
        base = self.installer
        candidate = candidate_installer(base)
        versions = next_versions(base)
        values = sample_checksums()
        fetcher, _ = checksum_fixture_fetcher(versions, values)
        artifact_fetcher, _ = artifact_fixture_fetcher(values)
        plan = module.build_refresh_plan(
            number=42,
            head_sha="a" * 40,
            head_ref="renovate/kubernetes-toolchain",
            base_sha="b" * 40,
            base_content=base,
            head_content=candidate,
            fetcher=fetcher,
            artifact_fetcher=artifact_fetcher,
        )
        self.assertIsNotNone(plan)
        if plan is None:
            self.fail("candidate refresh plan was not constructed")
        updated = module.replace_checksums(candidate, values)
        self.assertEqual(plan.updated_content, updated)
        self.assertEqual(
            module._normalize_allowed_changes(updated),
            module._normalize_allowed_changes(base),
        )
        actual_versions, actual_checksums = module.parse_installer(updated)
        self.assertEqual(
            actual_versions,
            {
                **versions,
                "kind": "v" + bump_patch(module.parse_installer(base)[0]["kind"][1:]),
            },
        )
        self.assertEqual(actual_checksums, values)

    def test_installer_integrity_rejects_stale_kind_digests_after_tag_advance(
        self,
    ) -> None:
        candidate = module.KIND_RE.sub(
            lambda match: match.group(0).replace(
                match.group("tag"), "v" + bump_patch(match.group("tag")[1:])
            ),
            self.installer,
        )
        versions, actual_checksums = module.parse_installer(candidate)
        expected_checksums = {
            name: digest
            for name, digest in actual_checksums.items()
            if name not in module.CHECKSUM_NAMES["kind"]
        }
        metadata_fetcher, _ = checksum_fixture_fetcher(versions, expected_checksums)
        artifact_fetcher, _ = artifact_fixture_fetcher(
            expected_checksums,
            {"kind_amd64": "c" * 64, "kind_arm64": "d" * 64},
        )
        with self.assertRaisesRegex(module.RefreshError, "kind amd64 artifact SHA-256"):
            module.verify_installer_integrity(
                candidate,
                fetcher=metadata_fetcher,
                artifact_fetcher=artifact_fetcher,
            )
        self.assertNotEqual(actual_checksums["kind_amd64"], "c" * 64)

    def test_refuses_non_pin_changes_and_unverified_checksums(self) -> None:
        base = self.installer
        candidate = candidate_installer(base)
        versions = next_versions(base)
        values = sample_checksums()
        fetcher, _ = checksum_fixture_fetcher(versions, values)
        artifact_fetcher, _ = artifact_fixture_fetcher(values)
        changed_tool = candidate.replace("Installed kind", "Unexpected kind", 1)
        with self.assertRaisesRegex(module.RefreshError, "outside the four Kubernetes tool pins"):
            module.build_refresh_plan(
                number=42,
                head_sha="a" * 40,
                head_ref="renovate/kubernetes-toolchain",
                base_sha="b" * 40,
                base_content=base,
                head_content=changed_tool,
                fetcher=fetcher,
                artifact_fetcher=artifact_fetcher,
            )
        altered = re.sub(
            r'(?m)^([ \t]*readonly kubectl_checksum=")[a-f0-9]{64}("$)',
            r"\g<1>" + "f" * 64 + r"\g<2>",
            candidate,
            count=1,
        )
        with self.assertRaisesRegex(module.RefreshError, "unverified value"):
            module.build_refresh_plan(
                number=42,
                head_sha="a" * 40,
                head_ref="renovate/kubernetes-toolchain",
                base_sha="b" * 40,
                base_content=base,
                head_content=altered,
                fetcher=fetcher,
                artifact_fetcher=artifact_fetcher,
            )

    def test_checksum_parser_rejects_missing_duplicate_and_mismatched_entries(
        self,
    ) -> None:
        with self.assertRaisesRegex(module.RefreshError, "found 0"):
            module._parse_checksum_line(b"not-a-checksum\n", "kubectl", "fixture")
        with self.assertRaisesRegex(module.RefreshError, "found 2"):
            module._parse_checksum_line(("a" * 64 + "\n" + "b" * 64).encode(), "kubectl", "fixture")
        with self.assertRaisesRegex(module.RefreshError, "found 0"):
            module._parse_checksum_line(
                ("a" * 64 + " wrong.tar.gz\n").encode(), "right.tar.gz", "fixture"
            )

    def test_pr_admission_matches_github_bot_and_same_repository_shape(self) -> None:
        sha = "a" * 40
        pr = {
            "number": 42,
            "state": "open",
            "merged": False,
            "user": {"login": "renovate[bot]", "type": "Bot"},
            "head": {
                "sha": sha,
                "ref": "renovate/kubernetes-tools",
                "user": {"login": "Strukturpiloten", "type": "Organization"},
                "repo": {"full_name": module.REPOSITORY},
            },
            "base": {
                "ref": "main",
                "sha": "b" * 40,
                "repo": {"full_name": module.REPOSITORY},
            },
        }
        module._validate_pull_request(pr, "b" * 40, "main")
        bad_cases = [
            ({**pr, "state": "closed"}, "not open"),
            ({**pr, "user": {"login": "someone", "type": "Bot"}}, "not authored"),
            (
                {**pr, "user": {"login": "renovate[bot]", "type": "User"}},
                "not authored",
            ),
            (
                {**pr, "head": {**pr["head"], "repo": {"full_name": "fork/boxferry"}}},
                "same-repository",
            ),
            ({**pr, "base": {**pr["base"], "sha": "c" * 40}}, "current main head"),
            ({**pr, "head": {**pr["head"], "ref": "main"}}, "safe Renovate branch"),
        ]
        for changed, message in bad_cases:
            with (
                self.subTest(message=message),
                self.assertRaisesRegex(module.RefreshError, message),
            ):
                module._validate_pull_request(changed, "b" * 40, "main")

    def test_git_data_commit_preserves_executable_mode_and_updates_only_pr_ref(
        self,
    ) -> None:
        plan = module.RefreshPlan(
            42,
            "a" * 40,
            "renovate/kubernetes-toolchain",
            "b" * 40,
            "base",
            "head",
            "updated",
        )
        pr = {
            "number": 42,
            "state": "open",
            "merged": False,
            "user": {"login": "renovate[bot]", "type": "Bot"},
            "head": {
                "sha": "a" * 40,
                "ref": plan.head_ref,
                "user": {"login": "Strukturpiloten", "type": "Organization"},
                "repo": {"full_name": module.REPOSITORY},
            },
            "base": {
                "ref": "main",
                "sha": "b" * 40,
                "repo": {"full_name": module.REPOSITORY},
            },
        }
        replies = [
            pr,
            {"object": {"sha": "a" * 40}},
            {"sha": "c" * 40},
            {"tree": {"sha": "d" * 40}},
            {"sha": "e" * 40},
            {"sha": "f" * 40},
            pr,
            {"object": {"sha": "a" * 40}},
            {"object": {"sha": "f" * 40}},
        ]
        calls: list[tuple[str, str, str, dict[str, object]]] = []

        def request(method: str, path: str, token: str, **kwargs: object) -> dict[str, object]:
            calls.append((method, path, token, kwargs))
            return cast(dict[str, object], replies.pop(0))

        with patch.object(module, "_json_request", side_effect=request):
            self.assertEqual(module._create_refresh_commit(plan, "read", "app"), "f" * 40)
        writes = [call for call in calls if call[0] in {"POST", "PATCH"}]
        self.assertEqual(
            [call[1] for call in writes],
            [
                "git/blobs",
                "git/trees",
                "git/commits",
                "git/refs/heads/renovate/kubernetes-toolchain",
            ],
        )
        tree_body = cast(dict[str, object], writes[1][3]["body"])
        tree_entries = cast(list[dict[str, object]], tree_body["tree"])
        self.assertEqual(tree_entries[0]["mode"], "100755")
        commit_body = cast(dict[str, object], writes[2][3]["body"])
        self.assertEqual(commit_body["parents"], [plan.head_sha])
        self.assertEqual(writes[-1][3]["body"], {"sha": "f" * 40, "force": False})
        self.assertTrue(all(call[2] == "read" for call in calls if call[0] == "GET"))

    def test_pr_head_race_fails_before_any_ref_write(self) -> None:
        plan = module.RefreshPlan(
            42,
            "a" * 40,
            "renovate/kubernetes-toolchain",
            "b" * 40,
            "base",
            "head",
            "updated",
        )
        pr = {
            "number": 42,
            "state": "open",
            "merged": False,
            "user": {"login": "renovate[bot]", "type": "Bot"},
            "head": {
                "sha": "a" * 40,
                "ref": plan.head_ref,
                "user": {"login": "Strukturpiloten", "type": "Organization"},
                "repo": {"full_name": module.REPOSITORY},
            },
            "base": {
                "ref": "main",
                "sha": "b" * 40,
                "repo": {"full_name": module.REPOSITORY},
            },
        }
        newer_pr = json.loads(json.dumps(pr))
        newer_pr["head"]["sha"] = "c" * 40
        replies = [
            pr,
            {"object": {"sha": "a" * 40}},
            {"sha": "d" * 40},
            {"tree": {"sha": "e" * 40}},
            {"sha": "f" * 40},
            {"sha": "b" * 40},
            newer_pr,
        ]
        methods: list[tuple[str, str]] = []

        def request(method: str, path: str, _token: str, **_kwargs: object) -> dict[str, object]:
            methods.append((method, path))
            return cast(dict[str, object], replies.pop(0))

        with (
            patch.object(module, "_json_request", side_effect=request),
            self.assertRaisesRegex(
                module.RefreshError, "head changed before the guarded ref update"
            ),
        ):
            module._create_refresh_commit(plan, "read", "app")
        self.assertFalse(any(method == "PATCH" for method, _ in methods))


if __name__ == "__main__":
    unittest.main()
