"""Refresh official Kubernetes-tool checksums on an exact Renovate PR head.

This trusted-main helper treats pull-request content as inert text. It streams
bounded official artifacts only to hash them and never stores or executes them.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import http.client
import importlib
import json
import os
import re
import signal
import socket
import ssl
import stat
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Callable, Generator, Mapping
from contextlib import contextmanager
from dataclasses import dataclass, field
from http.client import HTTPMessage
from pathlib import Path
from types import FrameType, TracebackType
from typing import IO, Any, Protocol, Self, cast

REPOSITORY = "Strukturpiloten/boxferry"
INSTALLER_PATH = "scripts/install-kubernetes-tools.sh"
BOT_LOGIN = "renovate[bot]"
APP_EMAIL = "317908760+strukturpiloten-release-plz[bot]@users.noreply.github.com"
SHA256_RE = re.compile(r"^[a-f0-9]{64}$")
SEMVER_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")
GIT_SHA_RE = re.compile(r"^[a-f0-9]{40}$")
MAX_API_BYTES = 1024 * 1024
MAX_CHECKSUM_BYTES = 16 * 1024
MAX_INSTALLER_BYTES = 64 * 1024
MAX_KUBECTL_ARTIFACT_BYTES = 128 * 1024 * 1024
MAX_HELM_ARTIFACT_BYTES = 64 * 1024 * 1024
MAX_KUSTOMIZE_ARTIFACT_BYTES = 32 * 1024 * 1024
MAX_VERIFY_REQUESTS = 32
MAX_VERIFY_BYTES = 512 * 1024 * 1024
VERIFY_SECONDS = 720
MAX_PULLS = 100

CHECKSUM_NAMES = {
    "kind": ("kind_amd64", "kind_arm64"),
    "kubectl": ("kubectl_amd64", "kubectl_arm64"),
    "helm": ("helm_amd64", "helm_arm64"),
    "kustomize": ("kustomize_amd64", "kustomize_arm64"),
}

VERSION_PATTERNS = {
    "kubectl": re.compile(r'(?m)^readonly kubectl_version="(?P<version>[0-9]+\.[0-9]+\.[0-9]+)"$'),
    "helm": re.compile(r'(?m)^readonly helm_version="(?P<version>[0-9]+\.[0-9]+\.[0-9]+)"$'),
    "kustomize": re.compile(
        r'(?m)^readonly kustomize_version="(?P<version>[0-9]+\.[0-9]+\.[0-9]+)"$'
    ),
}
KIND_RE = re.compile(
    r'(?m)^readonly kind_(?P<arch>amd64|arm64)_release="(?P<tag>v[0-9]+\.[0-9]+\.[0-9]+)" '
    r'kind_(?P=arch)_checksum="(?P<digest>[a-f0-9]{64})"$'
)
CHECKSUM_PATTERNS = {
    "kubectl_amd64": re.compile(r'(?m)^\s*readonly kubectl_checksum="([a-f0-9]{64})"$'),
    "helm_amd64": re.compile(r'(?m)^\s*readonly helm_checksum="([a-f0-9]{64})"$'),
    "kustomize_amd64": re.compile(r'(?m)^\s*readonly kustomize_checksum="([a-f0-9]{64})"$'),
}


class RefreshError(RuntimeError):
    """A fail-closed input, metadata, or API error with an actionable message."""


@dataclass
class NetworkBudget:
    """One bounded metadata and artifact acquisition run."""

    deadline: float
    requests: int = 0
    bytes_read: int = 0
    ipv4_hosts: set[str] = field(default_factory=set)

    @classmethod
    def start(cls) -> NetworkBudget:
        return cls(time.monotonic() + VERIFY_SECONDS)

    def begin_request(self) -> float:
        if self.requests >= MAX_VERIFY_REQUESTS:
            raise RefreshError("Kubernetes tool verification exhausted its request budget")
        remaining = self.deadline - time.monotonic()
        if remaining <= 0:
            raise RefreshError("Kubernetes tool verification exceeded its time budget")
        self.requests += 1
        return remaining

    def add_bytes(self, count: int) -> None:
        self.bytes_read += count
        if self.bytes_read > MAX_VERIFY_BYTES:
            raise RefreshError("Kubernetes tool verification exceeded its total byte budget")


_active_alarm_depth = 0
_active_alarm_deadline = 0.0


@contextmanager
def _deadline_guard(deadline: float) -> Generator[None]:
    """Interrupt blocking urllib reads at the shared monotonic deadline on Linux CLI runs."""
    global _active_alarm_depth, _active_alarm_deadline
    if (
        not hasattr(signal, "setitimer")
        or threading.current_thread() is not threading.main_thread()
    ):
        raise RefreshError(
            "bounded Kubernetes artifact acquisition requires the main thread on Linux"
        )
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise RefreshError("Kubernetes tool verification exceeded its time budget")
    if _active_alarm_depth:
        previous_deadline = _active_alarm_deadline
        _active_alarm_depth += 1
        _active_alarm_deadline = min(previous_deadline, deadline)
        signal.setitimer(signal.ITIMER_REAL, max(0.0, _active_alarm_deadline - time.monotonic()))
        try:
            yield
        finally:
            _active_alarm_depth -= 1
            _active_alarm_deadline = previous_deadline
            signal.setitimer(signal.ITIMER_REAL, max(0.0, previous_deadline - time.monotonic()))
        return

    previous_delay, previous_interval = signal.getitimer(signal.ITIMER_REAL)
    if previous_delay > 0 or previous_interval > 0:
        raise RefreshError(
            "bounded Kubernetes artifact acquisition cannot replace an existing alarm"
        )
    previous_handler = signal.getsignal(signal.SIGALRM)

    def expire(_signum: int, _frame: FrameType | None) -> None:
        raise RefreshError("Kubernetes tool verification exceeded its time budget")

    _active_alarm_depth = 1
    _active_alarm_deadline = deadline
    signal.signal(signal.SIGALRM, expire)
    signal.setitimer(signal.ITIMER_REAL, remaining)
    try:
        yield
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous_handler)
        _active_alarm_depth = 0
        _active_alarm_deadline = 0.0


class _Response(Protocol):
    @property
    def headers(self) -> HTTPMessage | Mapping[str, str]: ...

    def __enter__(self) -> Self: ...

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc_val: BaseException | None,
        traceback: TracebackType | None,
        /,
    ) -> bool | None: ...

    def geturl(self) -> str: ...

    def read(self, limit: int) -> bytes: ...


class _Opener(Protocol):
    def open(self, request: urllib.request.Request, /, *, timeout: float) -> _Response: ...


class _Admission(Protocol):
    AdmissionError: type[Exception]
    read_git: Callable[[Path, tuple[str, ...], int], bytes]

    def _tracked_blob(
        self,
        root: Path,
        revision: str,
        path: str,
        reader: Callable[[Path, tuple[str, ...], int], bytes],
        limit: int,
    ) -> bytes: ...


JsonObject = dict[str, Any]


@dataclass(frozen=True)
class RefreshPlan:
    number: int
    head_sha: str
    head_ref: str
    base_sha: str
    base_content: str
    head_content: str
    updated_content: str


def _validate_https_url(url: str, allowed_hosts: set[str]) -> None:
    parsed = urllib.parse.urlsplit(url)
    if (
        parsed.scheme != "https"
        or parsed.username is not None
        or parsed.password is not None
        or parsed.port not in (None, 443)
        or parsed.hostname not in allowed_hosts
    ):
        raise RefreshError(f"refused untrusted metadata URL: {url}")


class _AllowlistedRedirectHandler(urllib.request.HTTPRedirectHandler):
    def __init__(self, allowed_hosts: set[str]) -> None:
        super().__init__()
        self.allowed_hosts = allowed_hosts

    def redirect_request(
        self,
        req: urllib.request.Request,
        fp: IO[bytes],
        code: int,
        msg: str,
        headers: HTTPMessage,
        newurl: str,
    ) -> urllib.request.Request | None:
        _validate_https_url(newurl, self.allowed_hosts)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


class _IPv4HTTPSConnection(http.client.HTTPSConnection):
    """Select IPv4 sockets while preserving stdlib proxy, tunnel, and TLS logic."""

    def __init__(
        self,
        host: str,
        port: int | None = None,
        *,
        timeout: float | None = None,
        source_address: tuple[str, int] | None = None,
        context: ssl.SSLContext | None = None,
        blocksize: int = 8192,
    ) -> None:
        super().__init__(
            host,
            port,
            timeout=timeout,
            source_address=source_address,
            context=context,
            blocksize=blocksize,
        )
        self._create_connection = self._create_ipv4_connection

    def _create_ipv4_connection(
        self,
        address: tuple[str, int],
        timeout: float | None,
        source_address: tuple[str, int] | None,
    ) -> socket.socket:
        host, port = address
        errors: list[OSError] = []
        for family, socket_type, protocol, _canonical_name, sockaddr in socket.getaddrinfo(
            host,
            port,
            family=socket.AF_INET,
            type=socket.SOCK_STREAM,
        ):
            connection = socket.socket(family, socket_type, protocol)
            try:
                if timeout is not None:
                    connection.settimeout(timeout)
                if source_address:
                    connection.bind(source_address)
                connection.connect(sockaddr)
                return connection
            except OSError as error:
                errors.append(error)
                connection.close()
            except BaseException:
                connection.close()
                raise
        if errors:
            raise errors[-1]
        raise OSError(f"no IPv4 address available for {host}")


class _IPv4HTTPSHandler(urllib.request.HTTPSHandler):
    def __init__(self) -> None:
        self._ipv4_context = ssl.create_default_context()
        super().__init__(context=self._ipv4_context)

    def https_open(self, req: urllib.request.Request) -> http.client.HTTPResponse:
        return self.do_open(
            _IPv4HTTPSConnection,
            req,
            context=self._ipv4_context,
        )


def _build_ipv4_opener(allowed_hosts: set[str]) -> urllib.request.OpenerDirector:
    """Build the same HTTPS/redirect policy with IPv4-only connection selection."""
    return urllib.request.build_opener(
        _AllowlistedRedirectHandler(allowed_hosts),
        _IPv4HTTPSHandler(),
    )


def _ipv4_fallback_worthy(error: BaseException) -> bool:
    if isinstance(error, urllib.error.HTTPError):
        return False
    reason = error.reason if isinstance(error, urllib.error.URLError) else error
    if isinstance(reason, (TimeoutError, ConnectionError)):
        return True
    return isinstance(reason, ssl.SSLError) and "timed out" in str(reason).casefold()


def _open_with_ipv4_fallback(
    request: urllib.request.Request,
    *,
    allowed_hosts: set[str],
    timeout: float,
    opener: _Opener,
    budget: NetworkBudget,
) -> _Response:
    """Retry a transient dual-stack connection failure once via HTTPS over IPv4."""
    host = urllib.parse.urlsplit(request.full_url).hostname
    if host in budget.ipv4_hosts:
        remaining = budget.begin_request()
        return _build_ipv4_opener(allowed_hosts).open(request, timeout=min(timeout, remaining))

    remaining = budget.begin_request()
    try:
        return opener.open(request, timeout=min(timeout, remaining))
    except (OSError, urllib.error.URLError) as error:
        if not _ipv4_fallback_worthy(error):
            raise
    remaining = budget.begin_request()
    response = _build_ipv4_opener(allowed_hosts).open(request, timeout=min(timeout, remaining))
    if host is not None:
        budget.ipv4_hosts.add(host)
    return response


def _transient(error: BaseException) -> bool:
    if isinstance(error, urllib.error.HTTPError):
        return error.code in {408, 425, 429} or 500 <= error.code <= 599
    return isinstance(error, (urllib.error.URLError, TimeoutError, ConnectionError, OSError))


def _retry_delay(attempt: int, budget: NetworkBudget) -> None:
    delay = min(0.5 * (2**attempt), 2.0)
    if budget.deadline - time.monotonic() <= delay:
        raise RefreshError("Kubernetes tool verification deadline exhausted during retry wait")
    time.sleep(delay)


def fetch_bounded(
    url: str,
    *,
    allowed_hosts: set[str],
    max_bytes: int,
    headers: dict[str, str] | None = None,
    method: str = "GET",
    body: bytes | None = None,
    timeout: float = 10,
    opener: _Opener | None = None,
    budget: NetworkBudget | None = None,
) -> bytes:
    """Fetch small allowlisted metadata, enforcing redirects and response size."""
    _validate_https_url(url, allowed_hosts)
    request = urllib.request.Request(url, data=body, headers=headers or {}, method=method)
    safe_opener = opener or urllib.request.build_opener(_AllowlistedRedirectHandler(allowed_hosts))
    budget = budget or NetworkBudget.start()
    with _deadline_guard(budget.deadline):
        for attempt in range(4):
            try:
                with _open_with_ipv4_fallback(
                    request,
                    allowed_hosts=allowed_hosts,
                    timeout=timeout,
                    opener=safe_opener,
                    budget=budget,
                ) as response:
                    final_url = response.geturl()
                    _validate_https_url(final_url, allowed_hosts)
                    length = response.headers.get("Content-Length")
                    if length is not None and int(length) > max_bytes:
                        raise RefreshError(f"official metadata exceeded {max_bytes} bytes: {url}")
                    result = response.read(max_bytes + 1)
                if len(result) > max_bytes:
                    raise RefreshError(f"official metadata exceeded {max_bytes} bytes: {url}")
                budget.add_bytes(len(result))
                return result
            except RefreshError:
                raise
            except (OSError, ValueError, urllib.error.URLError) as error:
                if isinstance(error, urllib.error.HTTPError):
                    error.close()
                if attempt < 3 and _transient(error):
                    _retry_delay(attempt, budget)
                    continue
                raise RefreshError(f"could not fetch official metadata {url}: {error}") from error
    raise RefreshError(f"could not fetch official metadata {url} after bounded retries")


def fetch_artifact_sha256(
    url: str,
    *,
    allowed_hosts: set[str],
    max_bytes: int,
    timeout: float = 300,
    opener: _Opener | None = None,
    budget: NetworkBudget | None = None,
) -> str:
    """Hash an inert official artifact stream without storing or executing it."""
    _validate_https_url(url, allowed_hosts)
    request = urllib.request.Request(url, headers={"Accept-Encoding": "identity"})
    safe_opener = opener or urllib.request.build_opener(_AllowlistedRedirectHandler(allowed_hosts))
    budget = budget or NetworkBudget.start()
    with _deadline_guard(budget.deadline):
        for attempt in range(4):
            digest = hashlib.sha256()
            total = 0
            try:
                with _open_with_ipv4_fallback(
                    request,
                    allowed_hosts=allowed_hosts,
                    timeout=timeout,
                    opener=safe_opener,
                    budget=budget,
                ) as response:
                    _validate_https_url(response.geturl(), allowed_hosts)
                    length = response.headers.get("Content-Length")
                    if length is not None and int(length) > max_bytes:
                        raise RefreshError(f"official artifact exceeded {max_bytes} bytes: {url}")
                    while chunk := response.read(64 * 1024):
                        if time.monotonic() > budget.deadline:
                            raise RefreshError(
                                f"official artifact exceeded the total verification deadline: {url}"
                            )
                        total += len(chunk)
                        budget.add_bytes(len(chunk))
                        if total > max_bytes:
                            raise RefreshError(
                                f"official artifact exceeded {max_bytes} bytes: {url}"
                            )
                        digest.update(chunk)
                if total == 0:
                    raise RefreshError(f"official artifact was empty: {url}")
                return digest.hexdigest()
            except RefreshError:
                raise
            except (OSError, ValueError, urllib.error.URLError) as error:
                if isinstance(error, urllib.error.HTTPError):
                    error.close()
                if attempt < 3 and _transient(error):
                    _retry_delay(attempt, budget)
                    continue
                raise RefreshError(f"could not verify official artifact {url}: {error}") from error
    raise RefreshError(f"could not verify official artifact {url} after bounded retries")


def parse_installer(source: str) -> tuple[dict[str, str], dict[str, str]]:
    """Read the three external-source versions and all architecture digests."""
    if len(source.encode("utf-8")) > MAX_INSTALLER_BYTES:
        raise RefreshError("installer content exceeds the reviewed 64 KiB bound")
    versions: dict[str, str] = {}
    for tool, pattern in VERSION_PATTERNS.items():
        matches = list(pattern.finditer(source))
        if len(matches) != 1:
            raise RefreshError(f"expected exactly one {tool} version pin; found {len(matches)}")
        versions[tool] = matches[0].group("version")
    kind_records = list(KIND_RE.finditer(source))
    if len(kind_records) != 2 or {record.group("arch") for record in kind_records} != {
        "amd64",
        "arm64",
    }:
        raise RefreshError(
            "expected one exact kind release-tag and checksum record per architecture"
        )
    for record in kind_records:
        if not SEMVER_RE.fullmatch(record.group("tag")[1:]):
            raise RefreshError(f"invalid kind release tag: {record.group('tag')}")
    if len({record.group("tag") for record in kind_records}) != 1:
        raise RefreshError("kind architecture records must use the same release tag")
    versions["kind"] = kind_records[0].group("tag")
    digests: dict[str, str] = {}
    for record in kind_records:
        digests[f"kind_{record.group('arch')}"] = record.group("digest")
    for name, pattern in CHECKSUM_PATTERNS.items():
        matches = list(pattern.finditer(source))
        if len(matches) != 2:
            raise RefreshError(
                f"expected exactly two {name.split('_')[0]} checksums; found {len(matches)}"
            )
        # Installer order is amd64 then arm64; architecture case labels are checked below.
        digests[name] = matches[0].group(1)
        digests[f"{name.split('_')[0]}_arm64"] = matches[1].group(1)
    return versions, digests


def _parse_checksum_line(payload: bytes, filename: str, source: str) -> str:
    try:
        text = payload.decode("ascii").strip()
    except UnicodeDecodeError as error:
        raise RefreshError(f"checksum metadata from {source} is not ASCII") from error
    lines = [line for line in text.splitlines() if line.strip()]
    matches: list[str] = []
    for line in lines:
        fields = line.split()
        if (len(fields) == 1 and SHA256_RE.fullmatch(fields[0])) or (
            len(fields) == 2 and SHA256_RE.fullmatch(fields[0]) and fields[1] == filename
        ):
            matches.append(fields[0])
        elif len(fields) >= 2 and filename in fields[1:]:
            raise RefreshError(f"checksum metadata filename mismatch in {source}: {line!r}")
    if len(matches) != 1:
        raise RefreshError(
            f"expected one SHA-256 entry for {filename} in {source}; found {len(matches)}"
        )
    return matches[0]


Fetcher = Callable[..., bytes]
ArtifactFetcher = Callable[..., str]


def _verify_artifact(
    name: str,
    url: str,
    expected: str,
    *,
    allowed_hosts: set[str],
    max_bytes: int,
    artifact_fetcher: ArtifactFetcher,
    budget: NetworkBudget,
) -> None:
    try:
        actual = artifact_fetcher(
            url, allowed_hosts=allowed_hosts, max_bytes=max_bytes, budget=budget
        )
    except RefreshError as error:
        raise RefreshError(f"{name} artifact verification failed: {error}") from error
    if not SHA256_RE.fullmatch(actual) or actual != expected:
        raise RefreshError(f"official {name} artifact SHA-256 did not match its checksum metadata")


def obtain_checksums(
    versions: dict[str, str],
    fetcher: Fetcher = fetch_bounded,
    artifact_fetcher: ArtifactFetcher = fetch_artifact_sha256,
    budget: NetworkBudget | None = None,
) -> dict[str, str]:
    """Fetch official checksum metadata and verify both architecture artifacts."""
    budget = budget or NetworkBudget.start()
    checksums: dict[str, str] = {}
    for architecture in ("amd64", "arm64"):
        kubectl_hosts = {"dl.k8s.io", "storage.googleapis.com"}
        kube_url = (
            "https://dl.k8s.io/release/v"
            f"{versions['kubectl']}/bin/linux/{architecture}/kubectl.sha256"
        )
        kube_payload = fetcher(
            kube_url,
            allowed_hosts=kubectl_hosts,
            max_bytes=128,
            budget=budget,
        )
        kube_digest = _parse_checksum_line(kube_payload, "kubectl", kube_url)
        _verify_artifact(
            f"kubectl {architecture}",
            kube_url.removesuffix(".sha256"),
            kube_digest,
            allowed_hosts=kubectl_hosts,
            max_bytes=MAX_KUBECTL_ARTIFACT_BYTES,
            artifact_fetcher=artifact_fetcher,
            budget=budget,
        )
        checksums[f"kubectl_{architecture}"] = kube_digest

        helm_filename = f"helm-v{versions['helm']}-linux-{architecture}.tar.gz"
        helm_url = f"https://get.helm.sh/{helm_filename}.sha256sum"
        helm_payload = fetcher(
            helm_url,
            allowed_hosts={"get.helm.sh"},
            max_bytes=256,
            budget=budget,
        )
        helm_digest = _parse_checksum_line(helm_payload, helm_filename, helm_url)
        _verify_artifact(
            f"Helm {architecture}",
            helm_url.removesuffix(".sha256sum"),
            helm_digest,
            allowed_hosts={"get.helm.sh"},
            max_bytes=MAX_HELM_ARTIFACT_BYTES,
            artifact_fetcher=artifact_fetcher,
            budget=budget,
        )

        kustomize_filename = f"kustomize_v{versions['kustomize']}_linux_{architecture}.tar.gz"
        kustomize_url = (
            "https://github.com/kubernetes-sigs/kustomize/releases/download/"
            f"kustomize/v{versions['kustomize']}/checksums.txt"
        )
        kustomize_hosts = {
            "github.com",
            "release-assets.githubusercontent.com",
            "objects.githubusercontent.com",
        }
        checksums[f"helm_{architecture}"] = helm_digest
        kustomize_payload = fetcher(
            kustomize_url,
            allowed_hosts=kustomize_hosts,
            max_bytes=MAX_CHECKSUM_BYTES,
            budget=budget,
        )
        kustomize_digest = _parse_checksum_line(
            kustomize_payload, kustomize_filename, kustomize_url
        )
        _verify_artifact(
            f"Kustomize {architecture}",
            kustomize_url.removesuffix("checksums.txt") + kustomize_filename,
            kustomize_digest,
            allowed_hosts=kustomize_hosts,
            max_bytes=MAX_KUSTOMIZE_ARTIFACT_BYTES,
            artifact_fetcher=artifact_fetcher,
            budget=budget,
        )
        checksums[f"kustomize_{architecture}"] = kustomize_digest
    return checksums


def verify_kind_artifacts(
    release_tag: str,
    checksums: dict[str, str],
    artifact_fetcher: ArtifactFetcher = fetch_artifact_sha256,
    budget: NetworkBudget | None = None,
) -> None:
    """Verify each candidate kind asset digest against its inert release stream."""
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", release_tag):
        raise RefreshError(f"invalid kind release tag: {release_tag}")
    allowed_hosts = {
        "github.com",
        "release-assets.githubusercontent.com",
        "objects.githubusercontent.com",
    }
    budget = budget or NetworkBudget.start()
    for architecture in ("amd64", "arm64"):
        name = f"kind_{architecture}"
        url = (
            "https://github.com/kubernetes-sigs/kind/releases/download/"
            f"{release_tag}/kind-linux-{architecture}"
        )
        _verify_artifact(
            f"kind {architecture}",
            url,
            checksums.get(name, ""),
            allowed_hosts=allowed_hosts,
            max_bytes=32 * 1024 * 1024,
            artifact_fetcher=artifact_fetcher,
            budget=budget,
        )


def verify_installer_integrity(
    source: str,
    *,
    fetcher: Fetcher = fetch_bounded,
    artifact_fetcher: ArtifactFetcher = fetch_artifact_sha256,
) -> None:
    """Require every declared checksum to match official metadata and both assets."""
    versions, actual_checksums = parse_installer(source)
    budget = NetworkBudget.start()
    expected_checksums = obtain_checksums(versions, fetcher, artifact_fetcher, budget)
    for name, expected in expected_checksums.items():
        if actual_checksums[name] != expected:
            raise RefreshError(f"installer {name} checksum does not match official metadata")
    verify_kind_artifacts(versions["kind"], actual_checksums, artifact_fetcher, budget)


def _installer_from_git_object(reference: str, repository_root: Path) -> str:
    """Read the bounded regular installer blob from one exact Git commit."""
    if not GIT_SHA_RE.fullmatch(reference):
        raise RefreshError("integrity verification requires an exact lowercase Git commit SHA")
    try:
        admission = cast(_Admission, importlib.import_module("boxferry_ci.admission"))
    except ImportError as error:
        raise RefreshError(
            "exact-object verification requires the shared Git admission helper"
        ) from error
    root = repository_root.resolve()
    try:
        content = admission._tracked_blob(
            root,
            reference,
            INSTALLER_PATH,
            admission.read_git,
            MAX_INSTALLER_BYTES,
        )
        return content.decode("utf-8")
    except (OSError, UnicodeDecodeError, admission.AdmissionError) as error:
        raise RefreshError(
            f"could not read bounded installer blob from tested Git object {reference}: {error}"
        ) from error


def verify_git_object(reference: str, repository_root: Path = Path(".")) -> None:
    """Verify installer content read from an exact candidate commit, never the checkout."""
    verify_installer_integrity(_installer_from_git_object(reference, repository_root))


def verify_working_tree(repository_root: Path = Path(".")) -> None:
    """Verify the explicit local installer file as inert text."""
    root = repository_root.resolve()
    source = root / INSTALLER_PATH
    if not hasattr(os, "O_NOFOLLOW"):
        raise RefreshError("working-tree verification requires no-follow file access")
    try:
        for parent in source.parents:
            if parent == root:
                break
            if parent.is_symlink():
                raise RefreshError("working-tree installer path must not contain symlinks")
        flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_NONBLOCK
        descriptor = os.open(source, flags)
        try:
            info = os.fstat(descriptor)
            if not stat.S_ISREG(info.st_mode) or info.st_size > MAX_INSTALLER_BYTES:
                raise RefreshError("working-tree installer must be a bounded regular file")
            with os.fdopen(descriptor, "rb") as stream:
                descriptor = -1
                raw = stream.read(MAX_INSTALLER_BYTES + 1)
            if len(raw) > MAX_INSTALLER_BYTES:
                raise RefreshError("working-tree installer exceeded its read budget")
        finally:
            if descriptor >= 0:
                os.close(descriptor)
        content = raw.decode("utf-8")
    except RefreshError:
        raise
    except (OSError, UnicodeDecodeError) as error:
        raise RefreshError(
            f"could not read working-tree installer under {root}: {error}"
        ) from error
    verify_installer_integrity(content)


def replace_checksums(source: str, checksums: dict[str, str]) -> str:
    """Replace only six external checksum literals, requiring exact coverage."""
    updated = source
    for architecture in ("amd64", "arm64"):
        for tool in ("kubectl", "helm", "kustomize"):
            name = f"{tool}_{architecture}"
            digest = checksums.get(name)
            if digest is None or not SHA256_RE.fullmatch(digest):
                raise RefreshError(f"missing or invalid official checksum for {name}")
            assignment = re.compile(rf'(?m)^(\s*readonly {tool}_checksum=")([a-f0-9]{{64}})("\s*)$')
            matches = list(assignment.finditer(updated))
            if len(matches) != 2:
                raise RefreshError(
                    f"expected exactly two {tool} checksum assignments; found {len(matches)}"
                )
            index = 0 if architecture == "amd64" else 1
            match = matches[index]
            updated = updated[: match.start(2)] + digest + updated[match.end(2) :]
    return updated


def _normalize_allowed_changes(source: str) -> str:
    """Mask only the 4 tool versions and 8 owned digest literals for diff admission."""
    normalized = source
    for pattern in VERSION_PATTERNS.values():
        normalized, count = pattern.subn(
            lambda match: match.group(0).replace(match.group("version"), "VERSION"),
            normalized,
        )
        if count != 1:
            raise RefreshError("installer version fields are not in the reviewed form")
    normalized, count = KIND_RE.subn(
        lambda match: (
            match.group(0)
            .replace(match.group("tag"), "KIND_TAG")
            .replace(match.group("digest"), "KIND_DIGEST")
        ),
        normalized,
    )
    if count != 2:
        raise RefreshError("kind pin fields are not in the reviewed form")
    for pattern in CHECKSUM_PATTERNS.values():
        normalized, count = pattern.subn(
            lambda match: match.group(0).replace(match.group(1), "CHECKSUM"), normalized
        )
        if count != 2:
            raise RefreshError("Kubernetes checksum fields are not in the reviewed form")
    return normalized


def build_refresh_plan(
    *,
    number: int,
    head_sha: str,
    head_ref: str,
    base_sha: str,
    base_content: str,
    head_content: str,
    fetcher: Fetcher = fetch_bounded,
    artifact_fetcher: ArtifactFetcher = fetch_artifact_sha256,
) -> RefreshPlan | None:
    """Validate a Renovate-only installer edit and prepare a metadata-only patch."""
    if not GIT_SHA_RE.fullmatch(head_sha) or not GIT_SHA_RE.fullmatch(base_sha):
        raise RefreshError("PR base and head must be exact lowercase 40-character commits")
    if not re.fullmatch(r"renovate/[A-Za-z0-9._/-]+", head_ref) or ".." in head_ref:
        raise RefreshError("refused to update a non-Renovate or malformed PR branch")
    if _normalize_allowed_changes(base_content) != _normalize_allowed_changes(head_content):
        raise RefreshError(
            "Renovate PR changes files or installer text outside the four Kubernetes tool pins"
        )
    base_versions, base_digests = parse_installer(base_content)
    head_versions, head_digests = parse_installer(head_content)
    external_versions_changed = any(
        head_versions[tool] != base_versions[tool] for tool in VERSION_PATTERNS
    )
    kind_changed = any(
        head_versions["kind"] != base_versions["kind"] or head_digests[name] != base_digests[name]
        for name in CHECKSUM_NAMES["kind"]
    )
    if not external_versions_changed and not kind_changed:
        return None
    budget = NetworkBudget.start()
    if kind_changed:
        verify_kind_artifacts(head_versions["kind"], head_digests, artifact_fetcher, budget)
    expected = {
        name: head_digests[name] for tool in VERSION_PATTERNS for name in CHECKSUM_NAMES[tool]
    }
    if external_versions_changed:
        expected = obtain_checksums(head_versions, fetcher, artifact_fetcher, budget)
    for name, digest in expected.items():
        if head_digests[name] not in {base_digests[name], digest}:
            raise RefreshError(
                f"PR changed {name} checksum to an unverified value; "
                f"expected official source {digest}"
            )
    updated = replace_checksums(head_content, expected)
    if updated == head_content:
        return None
    return RefreshPlan(
        number=number,
        head_sha=head_sha,
        head_ref=head_ref,
        base_sha=base_sha,
        base_content=base_content,
        head_content=head_content,
        updated_content=updated,
    )


def _api_url(path: str) -> str:
    suffix = path.lstrip("/")
    return f"https://api.github.com/repos/{REPOSITORY}" + (f"/{suffix}" if suffix else "")


def _json_request(
    method: str,
    path: str,
    token: str,
    *,
    body: dict[str, Any] | None = None,
    max_bytes: int = MAX_API_BYTES,
) -> JsonObject:
    headers = {
        "Accept": "application/vnd.github+json",
        "Authorization": f"Bearer {token}",
        "X-GitHub-Api-Version": "2026-03-10",
    }
    payload = None if body is None else json.dumps(body, separators=(",", ":")).encode()
    if payload is not None:
        headers["Content-Type"] = "application/json"
    raw = fetch_bounded(
        _api_url(path),
        allowed_hosts={"api.github.com"},
        max_bytes=max_bytes,
        headers=headers,
        method=method,
        body=payload,
    )
    try:
        return json.loads(raw)
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        raise RefreshError(f"GitHub API returned invalid JSON for {path}") from error


def _read_content(token: str, reference: str) -> str:
    encoded_path = urllib.parse.quote(INSTALLER_PATH, safe="/")
    encoded_ref = urllib.parse.quote(reference, safe="")
    response = _json_request(
        "GET",
        f"contents/{encoded_path}?ref={encoded_ref}",
        token,
        max_bytes=MAX_INSTALLER_BYTES * 3,
    )
    if (
        response.get("encoding") != "base64"
        or response.get("size", MAX_INSTALLER_BYTES + 1) > MAX_INSTALLER_BYTES
    ):
        raise RefreshError("GitHub returned an oversized or non-base64 installer file")
    try:
        content = base64.b64decode(response["content"], validate=False).decode("utf-8")
    except (KeyError, ValueError, UnicodeDecodeError) as error:
        raise RefreshError("GitHub returned malformed installer content") from error
    if len(content.encode("utf-8")) > MAX_INSTALLER_BYTES:
        raise RefreshError("installer content exceeds the reviewed 64 KiB bound")
    return content


def _validate_pull_request(pr: dict[str, Any], current_sha: str, default_branch: str) -> None:
    number = pr.get("number")
    if pr.get("state") != "open" or pr.get("merged"):
        raise RefreshError(f"PR #{number} is not open")
    author = pr.get("user", {})
    if author.get("login") != BOT_LOGIN or author.get("type") != "Bot":
        raise RefreshError(f"PR #{number} is not authored by {BOT_LOGIN}")
    head = pr.get("head", {})
    base = pr.get("base", {})
    if (
        head.get("repo", {}).get("full_name") != REPOSITORY
        or base.get("repo", {}).get("full_name") != REPOSITORY
    ):
        raise RefreshError(f"PR #{number} must have same-repository base and head")
    if base.get("ref") != default_branch or base.get("sha") != current_sha:
        raise RefreshError(
            f"PR #{number} is not based on the current {default_branch} head; "
            "wait for Renovate rebase"
        )
    head_sha = head.get("sha", "")
    if not GIT_SHA_RE.fullmatch(head_sha):
        raise RefreshError(f"PR #{number} has an invalid head SHA")
    head_ref = head.get("ref", "")
    if not re.fullmatch(r"renovate/[A-Za-z0-9._/-]+", head_ref) or ".." in head_ref:
        raise RefreshError(f"PR #{number} branch is not a safe Renovate branch")


def _load_pr_plan(number: int, head_sha: str, read_token: str) -> RefreshPlan | None:
    repository = _json_request("GET", "", read_token)
    default_branch = repository.get("default_branch")
    if default_branch != "main":
        raise RefreshError(f"expected protected default branch main, found {default_branch!r}")
    ref = _json_request("GET", "git/ref/heads/main", read_token)
    current_sha = ref.get("object", {}).get("sha", "")
    if not GIT_SHA_RE.fullmatch(current_sha):
        raise RefreshError("could not read exact current main commit")
    pr = _json_request("GET", f"pulls/{number}", read_token)
    _validate_pull_request(pr, current_sha, default_branch)
    if pr.get("head", {}).get("sha") != head_sha:
        raise RefreshError(f"PR #{number} head changed during checksum refresh discovery")
    files = _json_request("GET", f"pulls/{number}/files?per_page=100", read_token)
    if not isinstance(files, list) or len(files) != 1 or files[0].get("filename") != INSTALLER_PATH:
        raise RefreshError(f"PR #{number} must change only {INSTALLER_PATH}")
    base_sha = pr["base"]["sha"]
    base_content = _read_content(read_token, base_sha)
    head_content = _read_content(read_token, head_sha)
    return build_refresh_plan(
        number=number,
        head_sha=head_sha,
        head_ref=pr["head"]["ref"],
        base_sha=base_sha,
        base_content=base_content,
        head_content=head_content,
    )


def _discover(read_token: str) -> list[int]:
    repository = _json_request("GET", "", read_token)
    if repository.get("default_branch") != "main":
        raise RefreshError("checksum refresher is restricted to the main default branch")
    prs = _json_request("GET", "pulls?state=open&per_page=100", read_token)
    if not isinstance(prs, list) or len(prs) > MAX_PULLS:
        raise RefreshError("open Renovate PR inventory exceeded the reviewed scan bound")
    candidates = []
    for pr in prs:
        if (
            pr.get("user", {}).get("login") == BOT_LOGIN
            and pr.get("base", {}).get("ref") == "main"
            and pr.get("base", {}).get("repo", {}).get("full_name") == REPOSITORY
            and pr.get("head", {}).get("repo", {}).get("full_name") == REPOSITORY
        ):
            candidates.append(pr["number"])
    return candidates


def _create_refresh_commit(plan: RefreshPlan, read_token: str, app_token: str) -> str:
    """Create one Git-data commit parented to, and fast-forward only, the exact PR head."""
    branch_path = urllib.parse.quote(f"heads/{plan.head_ref}", safe="/")
    current_pr = _json_request("GET", f"pulls/{plan.number}", read_token)
    _validate_pull_request(current_pr, plan.base_sha, "main")
    if current_pr.get("head", {}).get("sha") != plan.head_sha:
        raise RefreshError(f"PR #{plan.number} head changed before commit construction")
    current_ref = _json_request("GET", f"git/ref/{branch_path}", read_token)
    if current_ref.get("object", {}).get("sha") != plan.head_sha:
        raise RefreshError(f"PR #{plan.number} branch ref no longer matches the admitted head")

    blob = _json_request(
        "POST",
        "git/blobs",
        app_token,
        body={"content": plan.updated_content, "encoding": "utf-8"},
    )
    parent = _json_request("GET", f"git/commits/{plan.head_sha}", read_token)
    base_tree = parent.get("tree", {}).get("sha", "")
    if not GIT_SHA_RE.fullmatch(base_tree):
        raise RefreshError("could not read exact Renovate parent tree")
    tree = _json_request(
        "POST",
        "git/trees",
        app_token,
        body={
            "base_tree": base_tree,
            "tree": [
                {
                    "path": INSTALLER_PATH,
                    "mode": "100755",
                    "type": "blob",
                    "sha": blob.get("sha"),
                }
            ],
        },
    )
    commit = _json_request(
        "POST",
        "git/commits",
        app_token,
        body={
            "message": "ci: refresh Kubernetes tool checksums",
            "tree": tree.get("sha"),
            "parents": [plan.head_sha],
            "author": {"name": "strukturpiloten-release-plz[bot]", "email": APP_EMAIL},
            "committer": {
                "name": "strukturpiloten-release-plz[bot]",
                "email": APP_EMAIL,
            },
        },
    )
    new_sha = commit.get("sha", "")
    if not GIT_SHA_RE.fullmatch(new_sha):
        raise RefreshError("GitHub returned an invalid checksum-refresh commit SHA")

    # Revalidate the immutable PR identity and head immediately before the only ref write.
    current_pr = _json_request("GET", f"pulls/{plan.number}", read_token)
    _validate_pull_request(current_pr, plan.base_sha, "main")
    if current_pr.get("head", {}).get("sha") != plan.head_sha:
        raise RefreshError(f"PR #{plan.number} head changed before the guarded ref update")
    current_ref = _json_request("GET", f"git/ref/{branch_path}", read_token)
    if current_ref.get("object", {}).get("sha") != plan.head_sha:
        raise RefreshError(f"PR #{plan.number} branch moved before the guarded ref update")
    result = _json_request(
        "PATCH",
        f"git/refs/{branch_path}",
        app_token,
        body={"sha": new_sha, "force": False},
    )
    if result.get("object", {}).get("sha") != new_sha:
        raise RefreshError("GitHub did not advance the exact Renovate PR ref")
    return new_sha


def _plan_to_json(plan: RefreshPlan) -> dict[str, Any]:
    return {
        "number": plan.number,
        "head_sha": plan.head_sha,
        "head_ref": plan.head_ref,
        "base_sha": plan.base_sha,
        "base_content": plan.base_content,
        "head_content": plan.head_content,
        "updated_content": plan.updated_content,
    }


def _plan_from_json(value: object) -> RefreshPlan:
    if not isinstance(value, dict):
        raise RefreshError("trusted scan plan is malformed")
    try:
        plan = RefreshPlan(**value)
    except TypeError as error:
        raise RefreshError("trusted scan plan is incomplete") from error
    if not isinstance(plan.number, int) or not GIT_SHA_RE.fullmatch(plan.head_sha):
        raise RefreshError("trusted scan plan contains an invalid PR identity")
    if _normalize_allowed_changes(plan.base_content) != _normalize_allowed_changes(
        plan.head_content
    ):
        raise RefreshError("trusted scan plan candidate changed outside the reviewed pin fields")
    _, planned_checksums = parse_installer(plan.updated_content)
    if replace_checksums(plan.head_content, planned_checksums) != plan.updated_content:
        raise RefreshError("trusted scan plan changes installer text outside checksum literals")
    return plan


def scan(read_token: str) -> RefreshPlan | None:
    """Find one exact current Renovate PR that needs a verified checksum refresh."""
    current_ref = _json_request("GET", "git/ref/heads/main", read_token)
    current_sha = current_ref.get("object", {}).get("sha", "")
    if not GIT_SHA_RE.fullmatch(current_sha):
        raise RefreshError("could not read exact current main commit")
    for number in _discover(read_token):
        pr = _json_request("GET", f"pulls/{number}", read_token)
        if pr.get("base", {}).get("sha") != current_sha:
            print(
                f"Skipping Renovate PR #{number}: waiting for rebase onto current main",
                file=sys.stderr,
            )
            continue
        head_sha = pr.get("head", {}).get("sha", "")
        try:
            plan = _load_pr_plan(number, head_sha, read_token)
        except RefreshError as error:
            files = _json_request("GET", f"pulls/{number}/files?per_page=100", read_token)
            if isinstance(files, list) and any(
                item.get("filename") == INSTALLER_PATH for item in files
            ):
                raise
            print(f"Skipping unrelated Renovate PR #{number}: {error}", file=sys.stderr)
            continue
        if plan is not None:
            return plan
    return None


def apply_plan(plan: RefreshPlan, read_token: str, app_token: str) -> str:
    """Re-read candidate state, then create and guarded-fast-forward one PR commit."""
    current = _load_pr_plan(plan.number, plan.head_sha, read_token)
    if current is None or current != plan:
        raise RefreshError(
            f"PR #{plan.number} metadata changed after trusted scan; refusing stale plan"
        )
    return _create_refresh_commit(plan, read_token, app_token)


def run(
    read_token: str,
    *,
    app_token: str | None = None,
    scan_only: bool = False,
    plan_file: Path | None = None,
    apply_plan_file: Path | None = None,
) -> int:
    if os.environ.get("GITHUB_REPOSITORY") != REPOSITORY:
        raise RefreshError("checksum refresh is enabled only for Strukturpiloten/boxferry")
    if os.environ.get("GITHUB_REF") != "refs/heads/main":
        raise RefreshError("checksum refresh must execute from trusted refs/heads/main")
    if scan_only:
        if plan_file is None:
            raise RefreshError("--scan-only requires --plan-file")
        plan = scan(read_token)
        if plan is None:
            plan_file.write_text("{}\n", encoding="utf-8")
            print("refresh=false")
            print(
                "No eligible Renovate Kubernetes tool PR needs checksum refresh.",
                file=sys.stderr,
            )
            return 0
        plan_file.write_text(
            json.dumps(_plan_to_json(plan), separators=(",", ":")) + "\n",
            encoding="utf-8",
        )
        print("refresh=true")
        print(
            f"Prepared checksum refresh for PR #{plan.number} at {plan.head_sha}",
            file=sys.stderr,
        )
        return 0
    if apply_plan_file is not None:
        if app_token is None:
            raise RefreshError("RELEASE_PLZ_APP_TOKEN is required to update the admitted PR branch")
        try:
            plan = _plan_from_json(json.loads(apply_plan_file.read_text(encoding="utf-8")))
        except (OSError, json.JSONDecodeError) as error:
            raise RefreshError("could not read trusted checksum-refresh scan plan") from error
        new_sha = apply_plan(plan, read_token, app_token)
        print(
            f"Refreshed official checksums on Renovate PR #{plan.number}: "
            f"{plan.head_sha} -> {new_sha}"
        )
        return 0
    raise RefreshError("select --scan-only or --apply-plan")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--scan-only", action="store_true", help="discover and validate; never write")
    mode.add_argument("--apply-plan", type=Path, help="apply a previously scanned trusted plan")
    mode.add_argument(
        "--verify-working-tree",
        action="store_true",
        help="verify all installed pins against official metadata and assets",
    )
    mode.add_argument(
        "--verify-git-ref",
        help="verify installer content from an exact Git commit SHA, such as GITHUB_SHA",
    )
    parser.add_argument("--plan-file", type=Path, help="temporary plan path for scan-only mode")
    parser.add_argument(
        "--repository-root",
        type=Path,
        default=Path("."),
        help="repository root (default: current directory)",
    )
    args = parser.parse_args()
    if args.verify_working_tree:
        verify_working_tree(args.repository_root)
        print("All Kubernetes tool pins match official checksum metadata and artifacts.")
        return 0
    if args.verify_git_ref is not None:
        verify_git_object(args.verify_git_ref, args.repository_root)
        print("Kubernetes pins at the tested Git object match official artifacts.")
        return 0
    read_token = os.environ.get("GITHUB_TOKEN", "")
    app_token = os.environ.get("RELEASE_PLZ_APP_TOKEN")
    if not read_token:
        raise RefreshError("GITHUB_TOKEN is required for trusted repository metadata reads")
    return run(
        read_token,
        app_token=app_token,
        scan_only=args.scan_only,
        plan_file=args.plan_file,
        apply_plan_file=args.apply_plan,
    )


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RefreshError as error:
        print(f"Kubernetes checksum refresh failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
