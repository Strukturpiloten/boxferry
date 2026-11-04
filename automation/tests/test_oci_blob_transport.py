"""Observed GHCR blob redirects are a narrow content-only transport exception."""

from __future__ import annotations

import hashlib
import io
import signal
import time
import unittest
import urllib.request
from collections.abc import Mapping
from email.message import Message
from unittest.mock import patch

from boxferry_ci import dependencies
from boxferry_ci.dependencies import (
    DependencyError,
    OciBlobRedirect,
    _NoMetadataRedirect,
    fetch_oci_blob,
)

BODY = b"{}"
DIGEST = "sha256:" + hashlib.sha256(BODY).hexdigest()
ORIGIN = "https://ghcr.io/v2/devcontainers/features/github-cli/blobs/" + DIGEST
CDN = "https://pkg-containers.githubusercontent.com/ghcr1/blobs/" + DIGEST
SIGNED = "?se=synthetic&sig=synthetic-private-signed-query"


class OciBlobTransport(unittest.TestCase):
    def test_realistic_config_redirect_preserves_budget_and_drops_all_origin_headers(self) -> None:
        calls: list[tuple[str, dict[str, str]]] = []

        def request(url: str, headers: Mapping[str, str]) -> bytes:
            calls.append((url, dict(headers)))
            if url == ORIGIN:
                raise OciBlobRedirect(CDN + SIGNED)
            return BODY

        self.assertEqual(
            fetch_oci_blob(
                ORIGIN,
                {
                    "Authorization": "Bearer anonymous-origin-only",
                    "Accept": "application/octet-stream",
                },
                request,
            ),
            BODY,
        )
        self.assertEqual(len(calls), 2)
        self.assertIn("Authorization", calls[0][1])
        self.assertEqual(calls[1], (CDN + SIGNED, {}))

    def test_redirect_target_cannot_change_origin_or_requested_digest(self) -> None:
        for target in (
            "http://pkg-containers.githubusercontent.com/ghcr1/blobs/" + DIGEST,
            CDN.replace("pkg-containers.githubusercontent.com", "untrusted.invalid"),
            CDN.replace(
                "pkg-containers.githubusercontent.com", "user@pkg-containers.githubusercontent.com"
            ),
            CDN.replace(
                "pkg-containers.githubusercontent.com", "pkg-containers.githubusercontent.com:443"
            ),
            CDN + "#fragment",
            CDN.replace(DIGEST, "sha256:" + "a" * 64),
            CDN.replace("/ghcr1/", "/ghcr1/../ghcr1/"),
            CDN.replace("sha256:", "sha256%3A"),
        ):
            calls: list[str] = []

            def request(
                url: str,
                _headers: Mapping[str, str],
                calls: list[str] = calls,
                target: str = target,
            ) -> bytes:
                calls.append(url)
                raise OciBlobRedirect(target + SIGNED)

            with self.subTest(target=target), self.assertRaises(DependencyError) as caught:
                fetch_oci_blob(ORIGIN, {"Authorization": "Bearer hidden"}, request)
            self.assertEqual(calls, [ORIGIN])
            self.assertNotIn(SIGNED, str(caught.exception))
            self.assertNotIn(SIGNED, repr(caught.exception))

    def test_second_redirect_is_refused_and_every_attempt_is_counted(self) -> None:
        calls: list[str] = []

        def request(url: str, _headers: Mapping[str, str]) -> bytes:
            calls.append(url)
            raise OciBlobRedirect(CDN + SIGNED)

        with self.assertRaises(DependencyError):
            fetch_oci_blob(ORIGIN, {}, request)
        self.assertEqual(calls, [ORIGIN, CDN + SIGNED])

    def test_content_digest_size_and_aggregate_deadline_fail_closed(self) -> None:
        for body in (b"wrong content", b"x" * (dependencies.MAX_METADATA_BYTES + 1)):
            with self.subTest(size=len(body)), self.assertRaises(DependencyError):
                fetch_oci_blob(ORIGIN, {}, lambda _url, _headers, body=body: body)

        def blocked(_url: str, _headers: Mapping[str, str]) -> bytes:
            time.sleep(2)
            return BODY

        previous_handler = signal.getsignal(signal.SIGALRM)
        with (
            patch.object(dependencies, "OCI_BLOB_SECONDS", 0.02),
            self.assertRaisesRegex(TimeoutError, "aggregate deadline"),
        ):
            fetch_oci_blob(ORIGIN, {}, blocked)
        self.assertEqual(signal.getsignal(signal.SIGALRM), previous_handler)
        self.assertEqual(signal.getitimer(signal.ITIMER_REAL)[0], 0)

    def test_default_metadata_reader_still_refuses_all_ordinary_redirects(self) -> None:
        for origin in (
            "https://registry.npmjs.org/package",
            "https://ghcr.io/v2/devcontainers/features/github-cli/manifests/" + DIGEST,
            CDN,
        ):
            response = io.BytesIO()
            with self.assertRaisesRegex(DependencyError, "redirects are not allowed"):
                _NoMetadataRedirect().redirect_request(
                    urllib.request.Request(origin),
                    response,
                    307,
                    "redirect",
                    Message(),
                    CDN + SIGNED,
                )
            self.assertTrue(response.closed)
        response = io.BytesIO()
        with self.assertRaises(OciBlobRedirect) as caught:
            _NoMetadataRedirect().redirect_request(
                urllib.request.Request(ORIGIN), response, 307, "redirect", Message(), CDN + SIGNED
            )
        self.assertTrue(response.closed)
        self.assertNotIn(SIGNED, str(caught.exception))


if __name__ == "__main__":
    unittest.main()
