#!/usr/bin/env python3
"""Opt-in, fail-closed Paperless Libpod capture proxy.

Raw requests and responses stay in memory. Only a fully sanitized cassette candidate
and its manifest may be written, and only to a pre-existing private directory outside
the repository. This test-only tool executes one fixed read-only BoxFerry validation.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import socket
import stat
import subprocess
import sys
import tempfile
import threading
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit

MAX_REQUEST_HEADERS = 32 * 1024
MAX_RESPONSE = 64 * 1024 * 1024
SOCKET_TIMEOUT_SECONDS = 30
CASSETTE_NAME = "paperless-ngx-6.1.0-rootless.cassette.json"
MANIFEST_NAME = "capture-manifest-6.1.0-rootless.json"
CHECKSUM_NAME = "SHA256SUMS"
SANITIZER_VERSION = 1
PODMAN_REVISION = "cade97a52ebdf9dbf9e81de8009015776837a074"
MATRIX_SHA256 = "1ed306f4b368c229bca927697156e2314b922c2ec728c55c2820c69a712bad25"
RUNTIME_IMAGE = (
    "ghcr.io/strukturpiloten/podman-6.1-rootless:v6.1.0@"
    "sha256:dd00fadfff6e732728643df565a5db50f6d36dc3ec2d7f23a1fe87e905e08b5e"
)
SOURCE_FILES = (
    "scripts/lib/paperless-application.sh",
    "scripts/podman-live-conformance.sh",
    "fixtures/conformance/paperless-ngx-application/compose.yaml",
    "fixtures/conformance/paperless-ngx-application/images.tsv",
    "fixtures/conformance/podman-live/matrix.tsv",
    "fixtures/conformance/podman-live/capture_proxy.py",
)
ALLOWED_PATH = re.compile(
    r"^/libpod/_ping$|^/v[0-9]+(?:\.[0-9]+){2}/libpod/"
    r"(?:version|(?:containers|images|networks|pods|secrets|volumes)"
    r"(?:/[^/?]+)?/json)(?:\?[A-Za-z0-9._~%=&+-]+)?$"
)
TIMESTAMP = re.compile(
    r"\b\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})\b"
)
MAC = re.compile(r"\b(?:[0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}\b")
IPV4 = re.compile(r"(?<![0-9.])(?:\d{1,3}\.){3}\d{1,3}(?![0-9.])")
IPV6 = re.compile(r"(?<![0-9A-Fa-f:])(?:[0-9A-Fa-f]{1,4}:){2,}[0-9A-Fa-f:]+")
HEX_ID = re.compile(r"(?<![0-9a-f])([0-9a-f]{64})(?![0-9a-f])")
URL = re.compile(r"https?://[^\s\"']+")


class CaptureError(RuntimeError):
    """A capture-policy or protocol violation."""


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def json_bytes(value: Any) -> bytes:
    return (json.dumps(value, indent=2, ensure_ascii=False) + "\n").encode()


class OutputDirectory:
    """An output directory retained by descriptor across the privileged capture."""

    def __init__(
        self,
        path: Path,
        parent_fd: int,
        directory_fd: int,
        owner_uid: int,
        owner_gid: int,
    ) -> None:
        self.path = path
        self.parent_fd = parent_fd
        self.directory_fd = directory_fd
        self.owner_uid = owner_uid
        self.owner_gid = owner_gid

    def close(self) -> None:
        if self.directory_fd >= 0:
            os.close(self.directory_fd)
            self.directory_fd = -1
        if self.parent_fd >= 0:
            os.close(self.parent_fd)
            self.parent_fd = -1

    def __enter__(self) -> OutputDirectory:
        return self

    def __exit__(self, *_error: object) -> None:
        self.close()


def capture_output_owner() -> tuple[int, int]:
    """Return the non-privileged caller that must receive the final artifacts."""
    if os.geteuid() != 0:
        return os.geteuid(), os.getegid()
    try:
        owner_uid = int(os.environ["SUDO_UID"])
        owner_gid = int(os.environ["SUDO_GID"])
    except (KeyError, ValueError) as error:
        raise CaptureError("privileged capture requires numeric SUDO_UID and SUDO_GID") from error
    if owner_uid < 0 or owner_gid < 0:
        raise CaptureError("capture output owner IDs must be non-negative")
    return owner_uid, owner_gid


def open_output_directory(output: Path, repository: Path) -> OutputDirectory:
    """Create and retain a private output directory without raced path traversal."""
    if not output.is_absolute():
        raise CaptureError("capture directory must be absolute")
    if output.name in {"", ".", ".."}:
        raise CaptureError("capture directory must have a safe final path component")

    repository = repository.resolve(strict=True)
    parent = output.parent.resolve(strict=True)
    if parent == repository or repository in parent.parents:
        raise CaptureError("capture directory must be outside the repository")

    owner_uid, owner_gid = capture_output_owner()
    parent_flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
    parent_fd = os.open(parent, parent_flags)
    directory_fd = -1
    try:
        parent_stat = os.fstat(parent_fd)
        if parent_stat.st_uid != owner_uid:
            raise CaptureError("capture parent must be owned by the output recipient")
        if stat.S_IMODE(parent_stat.st_mode) & 0o022:
            raise CaptureError("capture parent must not be group- or world-writable")

        try:
            os.mkdir(output.name, 0o700, dir_fd=parent_fd)
        except FileExistsError as error:
            raise CaptureError("capture directory must not already exist") from error

        directory_flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
        directory_fd = os.open(output.name, directory_flags, dir_fd=parent_fd)
        directory_stat = os.fstat(directory_fd)
        if directory_stat.st_uid != os.geteuid():
            raise CaptureError("capture directory is not owned by the privileged creator")
        if stat.S_IMODE(directory_stat.st_mode) != 0o700:
            raise CaptureError("capture directory mode must be exactly 0700")
        if os.listdir(directory_fd):
            raise CaptureError("new capture directory is unexpectedly nonempty")

        return OutputDirectory(
            parent / output.name,
            parent_fd,
            directory_fd,
            owner_uid,
            owner_gid,
        )
    except Exception:
        if directory_fd >= 0:
            os.close(directory_fd)
        os.close(parent_fd)
        raise


class Sanitizer:
    """Deterministic recursive sanitizer with a closed residual-data policy."""

    def __init__(
        self,
        prefix: str,
        repository: Path,
        upstream_socket: Path,
        image_digests: set[str],
    ) -> None:
        if not prefix or any(character.isspace() for character in prefix):
            raise CaptureError("unsafe empty or whitespace-bearing Paperless prefix")
        self.prefix = prefix
        self.repository = str(repository.resolve())
        self.upstream_socket = str(upstream_socket.resolve())
        self.image_digests = image_digests
        self.ids: dict[str, str] = {}
        self.ipv4: dict[str, str] = {}
        self.ipv6: dict[str, str] = {}
        self.macs: dict[str, str] = {}
        self.secrets = {
            "boxferry-public-admin-canary": "<redacted:PAPERLESS_ADMIN_PASSWORD>",
            "boxferry-public-database-canary": "<redacted:PAPERLESS_DB_PASSWORD>",
            "boxferry-public-broker-canary": "<redacted:PAPERLESS_REDIS_PASSWORD>",
            "boxferry-public-paperless-secret-canary": "<redacted:PAPERLESS_SECRET_KEY>",
        }

    @staticmethod
    def _token(mapping: dict[str, str], value: str, label: str) -> str:
        if value not in mapping:
            mapping[value] = f"<{label}-{len(mapping) + 1:03d}>"
        return mapping[value]

    def _replace_ipv4(self, match: re.Match[str]) -> str:
        value = match.group(0)
        if value in {"0.0.0.0", "127.0.0.1", "255.255.255.255"}:
            return value
        octets = value.split(".")
        if any(int(octet) > 255 for octet in octets):
            return value
        token = self._token(self.ipv4, value, "ipv4")
        return f"192.0.2.{int(token[-4:-1])}"

    def _replace_ipv6(self, match: re.Match[str]) -> str:
        value = match.group(0)
        if value in {"::", "::1"}:
            return value
        token = self._token(self.ipv6, value, "ipv6")
        return f"2001:db8::{int(token[-4:-1])}"

    def _replace_mac(self, match: re.Match[str]) -> str:
        value = match.group(0)
        token = self._token(self.macs, value, "mac")
        return f"02:00:00:00:00:{int(token[-4:-1]):02x}"

    def _replace_hex_id(self, match: re.Match[str]) -> str:
        value = match.group(1)
        if value in self.image_digests:
            return value
        return self._token(self.ids, value, "native-id")

    def string(self, value: str) -> str:
        value = value.replace(self.repository, "<repository-root>")
        value = value.replace(self.upstream_socket, "<podman-socket>")
        value = value.replace(self.prefix, "paperless-captured")
        value = re.sub(
            r"/tmp/boxferry-fixture/[A-Za-z0-9_.-]+",
            "/tmp/boxferry-fixture/paperless-captured",
            value,
        )
        value = re.sub(
            r"/home/[^/]+/(?:\.local/share/containers/storage|\.config/containers)",
            "/sanitized/rootless-storage",
            value,
        )
        value = re.sub(r"/run/user/\d+", "/sanitized/rootless-runtime", value)
        for secret, replacement in self.secrets.items():
            value = value.replace(secret, replacement)
        value = re.sub(
            r"\b(PAPERLESS_(?:ADMIN_PASSWORD|DBPASS|SECRET_KEY)|POSTGRES_PASSWORD)=[^\s\"]+",
            lambda match: f"{match.group(1)}=<redacted>",
            value,
        )
        value = TIMESTAMP.sub("2000-01-01T00:00:00Z", value)
        macs: list[str] = []

        def hold_mac(match: re.Match[str]) -> str:
            macs.append(self._replace_mac(match))
            return f"BOXFERRYMACPLACEHOLDER{len(macs) - 1}"

        value = MAC.sub(hold_mac, value)
        value = IPV4.sub(self._replace_ipv4, value)
        value = IPV6.sub(self._replace_ipv6, value)
        for index, mac in enumerate(macs):
            value = value.replace(f"BOXFERRYMACPLACEHOLDER{index}", mac)
        value = HEX_ID.sub(self._replace_hex_id, value)
        return value

    def value(self, value: Any, key: str = "") -> Any:
        lowered = key.casefold()
        if lowered in {"secretdata", "authorization"}:
            raise CaptureError(f"forbidden field in native evidence: {key}")
        if isinstance(value, dict):
            return {name: self.value(item, name) for name, item in value.items()}
        if isinstance(value, list):
            return [self.value(item, key) for item in value]
        if isinstance(value, str):
            if lowered in {"date", "created", "createdat", "startedat", "finishedat"}:
                return "2000-01-01T00:00:00Z"
            if lowered in {"requestid", "request-id", "referenceid", "reference-id"}:
                return "<request-id>"
            return self.string(value)
        if isinstance(value, (int, float)) and any(
            word in lowered for word in ("created", "timestamp", "started", "finished")
        ):
            return 946684800
        return value

    def verify(self, value: Any) -> None:
        rendered = json.dumps(value, ensure_ascii=False)
        forbidden = (
            "SecretData",
            "Authorization",
            self.repository,
            self.upstream_socket,
            "/home/",
            "/run/user/",
        )
        if any(item and item in rendered for item in forbidden):
            raise CaptureError("sanitized output retains forbidden native data")
        for secret in self.secrets:
            if secret in rendered:
                raise CaptureError("sanitized output retains a protected canary")
        for candidate in URL.findall(rendered):
            parsed = urlsplit(candidate.rstrip(",.;)]}"))
            if parsed.username or parsed.password:
                raise CaptureError("sanitized output retains URL credentials")
            if parsed.hostname not in {
                "127.0.0.1",
                "localhost",
                "broker",
                "db",
                "gotenberg",
                "tika",
            }:
                raise CaptureError(f"unreviewed endpoint in sanitized output: {parsed.hostname}")


def parse_request(raw: bytes) -> tuple[str, list[tuple[str, str]]]:
    if len(raw) > MAX_REQUEST_HEADERS:
        raise CaptureError("request headers exceed capture bound")
    if b"\r\n\r\n" not in raw:
        raise CaptureError("incomplete HTTP request headers")
    header, trailing = raw.split(b"\r\n\r\n", 1)
    if trailing:
        raise CaptureError("request bodies are forbidden")
    try:
        lines = header.decode("iso-8859-1").split("\r\n")
        method, path, version = lines[0].split(" ")
    except (UnicodeDecodeError, ValueError) as error:
        raise CaptureError("malformed HTTP request") from error
    if method != "GET" or version not in {"HTTP/1.0", "HTTP/1.1"}:
        raise CaptureError("capture proxy permits only HTTP GET")
    if not ALLOWED_PATH.fullmatch(path):
        raise CaptureError(f"request path is outside read-only Libpod allowlist: {path}")
    headers = []
    for line in lines[1:]:
        if ":" not in line:
            raise CaptureError("malformed HTTP request header")
        name, value = line.split(":", 1)
        name, value = name.strip(), value.strip()
        lowered = name.casefold()
        if lowered == "authorization":
            raise CaptureError("authorization headers are forbidden")
        if lowered == "transfer-encoding":
            raise CaptureError("request transfer encoding is forbidden")
        if lowered == "content-length" and value != "0":
            raise CaptureError("request bodies are forbidden")
        headers.append((name, value))
    return path, headers


def read_request(connection: socket.socket) -> bytes:
    value = bytearray()
    while b"\r\n\r\n" not in value:
        chunk = connection.recv(4096)
        if not chunk:
            break
        value.extend(chunk)
        if len(value) > MAX_REQUEST_HEADERS:
            raise CaptureError("request headers exceed capture bound")
    return bytes(value)


def dechunk(body: bytes) -> bytes:
    output = bytearray()
    remaining = body
    while True:
        line, marker, remaining = remaining.partition(b"\r\n")
        if not marker:
            raise CaptureError("malformed chunked response")
        try:
            size = int(line.split(b";", 1)[0], 16)
        except ValueError as error:
            raise CaptureError("malformed chunk size") from error
        if size == 0:
            return bytes(output)
        if len(remaining) < size + 2 or remaining[size : size + 2] != b"\r\n":
            raise CaptureError("truncated chunked response")
        output.extend(remaining[:size])
        remaining = remaining[size + 2 :]


def parse_response(raw: bytes) -> tuple[int, list[tuple[str, str]], Any, bytes, bytes]:
    if len(raw) > MAX_RESPONSE:
        raise CaptureError("response exceeds capture bound")
    header, marker, body = raw.partition(b"\r\n\r\n")
    if not marker:
        raise CaptureError("malformed HTTP response")
    try:
        lines = header.decode("iso-8859-1").split("\r\n")
        status = int(lines[0].split(" ", 2)[1])
    except (UnicodeDecodeError, ValueError, IndexError) as error:
        raise CaptureError("malformed HTTP response status") from error
    headers = []
    chunked = False
    for line in lines[1:]:
        if ":" not in line:
            raise CaptureError("malformed HTTP response header")
        name, value = line.split(":", 1)
        name, value = name.strip(), value.strip()
        if name.casefold() in {"authorization", "set-cookie"}:
            raise CaptureError(f"forbidden response header: {name}")
        if name.casefold() == "transfer-encoding" and value.casefold() == "chunked":
            chunked = True
        if name.casefold() == "content-encoding" and value.casefold() != "identity":
            raise CaptureError("compressed responses are forbidden")
        headers.append((name, value))
    decoded = dechunk(body) if chunked else body
    if not decoded or decoded.strip() in {b"", b"OK"}:
        parsed: Any = None
    else:
        try:
            parsed = json.loads(decoded)
        except json.JSONDecodeError as error:
            raise CaptureError("non-JSON Libpod response body") from error
    return status, headers, parsed, header, decoded


def sanitize_interaction(
    sanitizer: Sanitizer, raw_request: bytes, raw_response: bytes
) -> tuple[dict[str, Any], dict[str, str]]:
    path, _ = parse_request(raw_request)
    status, headers, body, response_header, response_body = parse_response(raw_response)
    sanitized_body = sanitizer.value(body)
    sanitized_headers = []
    for name, value in headers:
        lowered = name.casefold()
        if lowered in {"connection", "transfer-encoding", "content-length"}:
            continue
        sanitized_headers.append([lowered, sanitizer.value(value, lowered)])
    body_length = 0
    if sanitized_body is not None:
        body_length = len(
            json.dumps(sanitized_body, ensure_ascii=False, separators=(",", ":")).encode()
        )
    sanitized_headers.append(["content-length", str(body_length)])
    interaction = {
        "request": {"method": "GET", "path": sanitizer.string(path)},
        "response": {
            "status": status,
            "headers": sanitized_headers,
            "body": sanitized_body,
        },
    }
    sanitizer.verify(interaction)
    hashes = {
        "request_headers_sha256": sha256_bytes(raw_request),
        "response_headers_sha256": sha256_bytes(response_header),
        "response_body_sha256": sha256_bytes(response_body),
    }
    return interaction, hashes


def load_images(repository: Path) -> tuple[list[str], set[str]]:
    references = []
    path = repository / "fixtures/conformance/paperless-ngx-application/images.tsv"
    for line in path.read_text().splitlines():
        if not line or line.startswith("#"):
            continue
        columns = line.split("\t")
        if len(columns) != 6 or "@sha256:" not in columns[1]:
            raise CaptureError("invalid Paperless image catalogue")
        references.append(columns[1])
    if len(references) != 5 or len(set(references)) != 5:
        raise CaptureError("Paperless capture requires exactly five reviewed images")
    return references, {reference.rsplit("sha256:", 1)[1] for reference in references}


def source_hashes(repository: Path) -> dict[str, str]:
    result = {}
    for relative in SOURCE_FILES:
        path = repository / relative
        if not path.is_file():
            raise CaptureError(f"capture source is missing: {relative}")
        result[relative] = sha256_bytes(path.read_bytes())
    return result


def validate_matrix(repository: Path) -> str:
    matrix = repository / "fixtures/conformance/podman-live/matrix.tsv"
    observed_hash = sha256_bytes(matrix.read_bytes())
    if observed_hash != MATRIX_SHA256:
        raise CaptureError("live matrix differs from reviewed Paperless capture matrix")
    rows = [
        line.split("\t")
        for line in matrix.read_text().splitlines()
        if line and not line.startswith("#") and line.startswith("podman-6.1-rootless\t")
    ]
    if len(rows) != 1 or len(rows[0]) != 7:
        raise CaptureError("reviewed podman-6.1-rootless matrix row is missing or ambiguous")
    row = rows[0]
    if row[1] != RUNTIME_IMAGE or row[2:] != [
        "6.1.0",
        "upstream-source",
        "rootless",
        "container",
        "amd64",
    ]:
        raise CaptureError("reviewed podman-6.1-rootless matrix row changed")
    return row[1]


def build_artifacts(
    repository: Path,
    prefix: str,
    upstream_socket: Path,
    interactions: list[tuple[bytes, bytes]],
) -> tuple[bytes, bytes]:
    images, digests = load_images(repository)
    runtime_image = validate_matrix(repository)
    sanitizer = Sanitizer(prefix, repository, upstream_socket, digests)
    sanitized = []
    hashes = []
    for raw_request, raw_response in interactions:
        interaction, interaction_hashes = sanitize_interaction(
            sanitizer, raw_request, raw_response
        )
        sanitized.append(interaction)
        hashes.append(interaction_hashes)
    if not sanitized:
        raise CaptureError("capture contained no interactions")
    cassette = {
        "schema_version": 1,
        "fixture_kind": "libpod-cassette",
        "scenario_id": "paperless-ngx-application-podman-6.1.0-rootless-captured",
        "scenario_revision": 1,
        "engine_version": "6.1.0",
        "api_version": "6.1.0",
        "execution_context": "rootless",
        "synthetic": False,
        "provenance": {
            "evidence_kind": "captured-native-sanitized-candidate",
            "release_tag": "v6.1.0",
            "revision": PODMAN_REVISION,
        },
        "sanitization": (
            "Captured through a bodyless-GET-only Unix proxy; identifiers, paths, "
            "addresses, timestamps, request IDs, and protected values are normalized. "
            "Raw bytes were retained only in memory. Human privacy review is required."
        ),
        "sanitizer_version": SANITIZER_VERSION,
        "interactions": sanitized,
    }
    sanitizer.verify(cassette)
    revision = subprocess.run(
        ["git", "-C", str(repository), "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    manifest = {
        "schema_version": 1,
        "candidate": CASSETTE_NAME,
        "admission": "one-off-non-reproducible-candidate-requires-human-privacy-review",
        "engine_version": "6.1.0",
        "api_version": "6.1.0",
        "rootless": True,
        "podman_revision": PODMAN_REVISION,
        "boxferry_revision": revision,
        "matrix_cell": "podman-6.1-rootless",
        "matrix_sha256": MATRIX_SHA256,
        "runtime_image": runtime_image,
        "sanitizer_version": SANITIZER_VERSION,
        "source_sha256": source_hashes(repository),
        "images": images,
        "interaction_hashes": hashes,
        "privacy": (
            "Raw requests and responses were never written. This sanitized candidate "
            "must not enter the repository before independent privacy and provenance review."
        ),
    }
    return json_bytes(cassette), json_bytes(manifest)


def emit_artifacts(output: OutputDirectory, cassette: bytes, manifest: bytes) -> None:
    checksums = (
        f"{sha256_bytes(cassette)}  {CASSETTE_NAME}\n"
        f"{sha256_bytes(manifest)}  {MANIFEST_NAME}\n"
    ).encode()
    names = [CASSETTE_NAME, MANIFEST_NAME, CHECKSUM_NAME]
    created: list[str] = []
    try:
        for name, content in zip(names, (cassette, manifest, checksums), strict=True):
            descriptor = os.open(
                name,
                os.O_WRONLY
                | os.O_CREAT
                | os.O_EXCL
                | os.O_NOFOLLOW
                | os.O_CLOEXEC,
                0o600,
                dir_fd=output.directory_fd,
            )
            created.append(name)
            with os.fdopen(descriptor, "wb") as handle:
                handle.write(content)
                handle.flush()
                os.fsync(handle.fileno())
                os.fchown(handle.fileno(), output.owner_uid, output.owner_gid)

        os.fsync(output.directory_fd)
        os.fchown(output.directory_fd, output.owner_uid, output.owner_gid)
    except Exception:
        for name in created:
            try:
                os.unlink(name, dir_fd=output.directory_fd)
            except FileNotFoundError:
                pass
        raise


class Proxy:
    def __init__(self, listen: Path, upstream: Path) -> None:
        self.listen = listen
        self.upstream = upstream
        self.stop = threading.Event()
        self.interactions: list[tuple[bytes, bytes]] = []
        self.failure: BaseException | None = None

    def _handle(self, downstream: socket.socket) -> None:
        downstream.settimeout(SOCKET_TIMEOUT_SECONDS)
        raw_request = read_request(downstream)
        path, _ = parse_request(raw_request)
        forwarded = (
            f"GET {path} HTTP/1.1\r\nHost: podman\r\nAccept: application/json\r\n"
            "Connection: close\r\n\r\n"
        ).encode()
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as upstream:
            upstream.settimeout(SOCKET_TIMEOUT_SECONDS)
            upstream.connect(str(self.upstream))
            upstream.sendall(forwarded)
            response = bytearray()
            while True:
                chunk = upstream.recv(65536)
                if not chunk:
                    break
                response.extend(chunk)
                if len(response) > MAX_RESPONSE:
                    raise CaptureError("response exceeds capture bound")
        raw_response = bytes(response)
        parse_response(raw_response)
        self.interactions.append((raw_request, raw_response))
        downstream.sendall(raw_response)

    def serve(self) -> None:
        try:
            if self.listen.exists() or self.listen.is_symlink():
                raise CaptureError("capture proxy socket path already exists")
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
                listener.bind(str(self.listen))
                os.chmod(self.listen, 0o600)
                listener.listen(4)
                listener.settimeout(0.25)
                while not self.stop.is_set():
                    try:
                        downstream, _ = listener.accept()
                    except TimeoutError:
                        continue
                    with downstream:
                        self._handle(downstream)
        except BaseException as error:  # propagated to the recording coordinator
            self.failure = error
            self.stop.set()
        finally:
            self.listen.unlink(missing_ok=True)


def record(arguments: argparse.Namespace) -> None:
    repository = arguments.repository.resolve(strict=True)
    upstream = arguments.upstream_socket.resolve(strict=True)
    if not stat.S_ISSOCK(upstream.stat().st_mode):
        raise CaptureError("upstream Podman path is not a Unix socket")
    boxferry = arguments.boxferry_bin.resolve(strict=True)
    if not boxferry.is_file() or not os.access(boxferry, os.X_OK):
        raise CaptureError("BoxFerry capture binary is not executable")
    proxy = Proxy(arguments.proxy_socket, upstream)
    worker = threading.Thread(target=proxy.serve, name="paperless-capture-proxy")
    worker.start()
    command = [
        str(boxferry),
        "validate",
        "podman",
        "compose",
        "--podman-socket",
        str(arguments.proxy_socket),
        "--application-name",
        f"{arguments.prefix}-paperless",
        "--podman-label",
        f"io.boxferry.application={arguments.prefix}-paperless",
        "--promote-podman-effective-named-volumes",
        "--promote-podman-effective-named-networks",
        "--promote-podman-portable-effective-settings",
        "--loss-policy",
        "partial",
        "--console-format",
        "json",
    ]
    try:
        for _ in range(120):
            if arguments.proxy_socket.exists() or proxy.failure:
                break
            proxy.stop.wait(0.05)
        if proxy.failure or not arguments.proxy_socket.exists():
            raise CaptureError("capture proxy failed to start") from proxy.failure
        completed = subprocess.run(
            command,
            cwd=repository,
            capture_output=True,
            timeout=120,
            check=False,
        )
    finally:
        proxy.stop.set()
        worker.join(timeout=SOCKET_TIMEOUT_SECONDS + 1)
    if worker.is_alive():
        raise CaptureError("capture proxy did not stop")
    if proxy.failure:
        raise CaptureError("capture proxy rejected native traffic") from proxy.failure
    if completed.returncode != 0:
        raise CaptureError(
            f"fixed BoxFerry read-only validation failed with status {completed.returncode}"
        )
    cassette, manifest = build_artifacts(
        repository, arguments.prefix, upstream, proxy.interactions
    )
    with open_output_directory(arguments.output_directory, repository) as output:
        emit_artifacts(output, cassette, manifest)


def response(body: Any, headers: list[tuple[str, str]] | None = None) -> bytes:
    encoded = b"" if body is None else json.dumps(body, separators=(",", ":")).encode()
    rows = headers or [("content-type", "application/json")]
    rows.append(("content-length", str(len(encoded))))
    head = "HTTP/1.1 200 OK\r\n" + "".join(f"{key}: {value}\r\n" for key, value in rows)
    return head.encode() + b"\r\n" + encoded


def self_test() -> None:
    request = b"GET /v6.1.0/libpod/containers/json?all=true HTTP/1.1\r\nHost: podman\r\n\r\n"
    assert parse_request(request)[0].endswith("all=true")
    for rejected in (
        b"POST /v6.1.0/libpod/containers/json HTTP/1.1\r\n\r\n",
        b"GET /v6.1.0/libpod/containers/json HTTP/1.1\r\nAuthorization: x\r\n\r\n",
        b"GET /v6.1.0/libpod/containers/json HTTP/1.1\r\nContent-Length: 1\r\n\r\nx",
    ):
        try:
            parse_request(rejected)
        except CaptureError:
            pass
        else:
            raise AssertionError("unsafe request was accepted")
    try:
        parse_request(b"G" * (MAX_REQUEST_HEADERS + 1) + b"\r\n\r\n")
    except CaptureError:
        pass
    else:
        raise AssertionError("oversized request was accepted")

    with tempfile.TemporaryDirectory(prefix="boxferry-capture-self-test-") as root_text:
        root = Path(root_text)
        repository = root / "repository"
        repository.mkdir(mode=0o700)
        (repository / "fixtures/conformance/paperless-ngx-application").mkdir(
            parents=True
        )
        (repository / "fixtures/conformance/podman-live").mkdir(parents=True)
        for relative in SOURCE_FILES:
            path = repository / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("test\n")
        image_table = repository / "fixtures/conformance/paperless-ngx-application/images.tsv"
        image_table.write_text(
            "\n".join(
                f"image-{index}\tregistry.invalid/image-{index}:1@sha256:{index:064x}\t1\tX\thttps://example.invalid\ttransient-test-pull"
                for index in range(1, 6)
            )
            + "\n"
        )
        capture_parent = root / "capture-parent"
        capture_parent.mkdir(mode=0o700)
        output = capture_parent / "output"
        inside = repository / "inside"
        try:
            open_output_directory(inside, repository)
        except CaptureError:
            pass
        else:
            raise AssertionError("inside-repository output was accepted")
        symlink = capture_parent / "output-link"
        symlink.symlink_to(capture_parent, target_is_directory=True)
        try:
            open_output_directory(symlink, repository)
        except CaptureError:
            pass
        else:
            raise AssertionError("symlink output was accepted")

        sanitizer = Sanitizer("run-123-paper", repository, root / "podman.sock", set())
        native = {
            "Id": "a" * 64,
            "Created": "2026-09-09T12:00:00.123Z",
            "Config": {
                "CreateCommand": [
                    "podman",
                    "run",
                    "--env",
                    "PAPERLESS_SECRET_KEY=boxferry-public-paperless-secret-canary",
                    "--name",
                    "run-123-paper-web",
                ],
                "NullIsPreserved": None,
            },
            "Addresses": ["10.88.0.2", "2001:db8:abcd::2", "aa:bb:cc:dd:ee:ff"],
        }
        first = sanitizer.value(native)
        second = Sanitizer(
            "run-123-paper", repository, root / "podman.sock", set()
        ).value(native)
        assert json_bytes(first) == json_bytes(second)
        assert first["Config"]["NullIsPreserved"] is None
        assert list(first["Config"]) == ["CreateCommand", "NullIsPreserved"]
        assert first["Config"]["CreateCommand"][3].endswith("=<redacted>")
        assert first["Addresses"][0].startswith("192.0.2.")
        assert first["Addresses"][1].startswith("2001:db8::")
        assert first["Addresses"][2].startswith("02:00:00:00:00:")
        for forbidden in ({"SecretData": "x"}, {"Authorization": "x"}):
            try:
                sanitizer.value(forbidden)
            except CaptureError:
                pass
            else:
                raise AssertionError("recursive forbidden field was accepted")

        interaction, _ = sanitize_interaction(
            sanitizer,
            request,
            response({"items": [native], "absent": None}, [("Date", "Wed, 09 Sep 2026 12:00:00 GMT")]),
        )
        headers = dict(interaction["response"]["headers"])
        expected_length = len(
            json.dumps(
                interaction["response"]["body"], separators=(",", ":")
            ).encode()
        )
        assert headers["content-length"] == str(expected_length)

        bad = response({"SecretData": "must-not-write"})
        try:
            sanitize_interaction(sanitizer, request, bad)
        except CaptureError:
            pass
        else:
            raise AssertionError("unsafe response was accepted")
        with open_output_directory(output, repository) as retained_output:
            moved_output = capture_parent / "output-moved"
            output.rename(moved_output)
            attacker_target = capture_parent / "attacker-target"
            attacker_target.mkdir(mode=0o700)
            output.symlink_to(attacker_target, target_is_directory=True)
            emit_artifacts(retained_output, b"{}\n", b"{}\n")
            try:
                emit_artifacts(retained_output, b"{}\n", b"{}\n")
            except FileExistsError:
                pass
            else:
                raise AssertionError("capture overwrite was accepted")

        assert not any(attacker_target.iterdir())
        output.unlink()
        output = moved_output
        assert (output / CASSETTE_NAME).stat().st_mode & 0o777 == 0o600

        real_repository = Path(__file__).resolve().parents[3]
        upstream_socket = root / "fake-podman.sock"
        upstream_ready = threading.Event()
        upstream_failure: list[BaseException] = []

        def fake_upstream() -> None:
            try:
                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
                    listener.bind(str(upstream_socket))
                    listener.listen(1)
                    upstream_ready.set()
                    connection, _ = listener.accept()
                    with connection:
                        forwarded = read_request(connection)
                        assert parse_request(forwarded)[0] == "/libpod/_ping"
                        connection.sendall(
                            response(
                                None,
                                [("libpod-api-version", "6.1.0")],
                            )
                        )
            except BaseException as error:
                upstream_failure.append(error)
            finally:
                upstream_socket.unlink(missing_ok=True)

        upstream_thread = threading.Thread(target=fake_upstream)
        upstream_thread.start()
        if not upstream_ready.wait(2):
            raise AssertionError(f"fake upstream failed to start: {upstream_failure!r}")
        validator = root / "boxferry-stub"
        validator.write_text(
            "#!/usr/bin/env python3\n"
            "import socket, sys\n"
            "path = sys.argv[sys.argv.index('--podman-socket') + 1]\n"
            "with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:\n"
            "    client.connect(path)\n"
            "    client.sendall(b'GET /libpod/_ping HTTP/1.1\\r\\nHost: podman\\r\\n\\r\\n')\n"
            "    while client.recv(4096):\n"
            "        pass\n"
        )
        validator.chmod(0o700)
        record_output = root / "record-output"
        proxy_socket = root / "record-proxy.sock"
        record(
            argparse.Namespace(
                repository=real_repository,
                output_directory=record_output,
                upstream_socket=upstream_socket,
                proxy_socket=proxy_socket,
                boxferry_bin=validator,
                prefix="self-test-paper",
            )
        )
        upstream_thread.join(2)
        assert not upstream_thread.is_alive()
        assert not upstream_failure
        assert not proxy_socket.exists()
        assert {path.name for path in record_output.iterdir()} == {
            CASSETTE_NAME,
            MANIFEST_NAME,
            CHECKSUM_NAME,
        }

        rejected_proxy_path = root / "rejected-proxy.sock"
        rejected_proxy = Proxy(rejected_proxy_path, root / "unused-upstream.sock")
        rejected_thread = threading.Thread(target=rejected_proxy.serve)
        rejected_thread.start()
        for _ in range(40):
            if rejected_proxy_path.exists():
                break
            rejected_proxy.stop.wait(0.05)
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
            client.connect(str(rejected_proxy_path))
            client.sendall(b"POST /libpod/_ping HTTP/1.1\r\n\r\n")
        rejected_thread.join(2)
        assert not rejected_thread.is_alive()
        assert isinstance(rejected_proxy.failure, CaptureError)
        assert not rejected_proxy_path.exists()


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--self-test", action="store_true")
    result.add_argument("--repository", type=Path)
    result.add_argument("--output-directory", type=Path)
    result.add_argument("--upstream-socket", type=Path)
    result.add_argument("--proxy-socket", type=Path)
    result.add_argument("--boxferry-bin", type=Path)
    result.add_argument("--prefix")
    return result


def main() -> int:
    arguments = parser().parse_args()
    try:
        if arguments.self_test:
            self_test()
        else:
            required = (
                arguments.repository,
                arguments.output_directory,
                arguments.upstream_socket,
                arguments.proxy_socket,
                arguments.boxferry_bin,
                arguments.prefix,
            )
            if any(value is None for value in required):
                raise CaptureError("recording requires every explicit capture argument")
            record(arguments)
    except (CaptureError, OSError, subprocess.SubprocessError) as error:
        print(f"capture refused: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
