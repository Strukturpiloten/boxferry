#!/usr/bin/env python3
"""Deterministic Immich image generator and bounded API acceptance probe."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import time
from typing import Any
import urllib.error
import urllib.parse
import urllib.request
import zlib


API_ROOT = os.environ.get("BF_IMMICH_URL", "http://immich-server:2283").rstrip("/")
EMAIL = os.environ.get("BF_IMMICH_EMAIL", "boxferry@example.invalid")
PASSWORD = os.environ.get("BF_IMMICH_PASSWORD", "boxferry-public-admin-canary")
REQUEST_TIMEOUT = 20
PROCESSING_DEADLINE = 480


def sha256(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def png_chunk(kind: bytes, data: bytes) -> bytes:
    body = kind + data
    return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))


def png_bytes() -> bytes:
    """Return a deterministic, nontrivial RGB PNG without third-party libraries."""
    width = 96
    height = 64
    rows = bytearray()
    for y in range(height):
        rows.append(0)
        for x in range(width):
            rows.extend(((x * 13 + y * 3) % 256, (x * 5 + y * 17) % 256, (x ^ y) * 2))
    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + png_chunk(b"IHDR", header)
        + png_chunk(b"IDAT", zlib.compress(bytes(rows), level=9))
        + png_chunk(b"IEND", b"")
    )


def generate_twice(work_dir: Path) -> bytes:
    first = png_bytes()
    second = png_bytes()
    if first != second or sha256(first) != sha256(second):
        raise RuntimeError("PNG generation is not byte deterministic")
    work_dir.mkdir(parents=True, exist_ok=True)
    (work_dir / "boxferry-media.png").write_bytes(first)
    return first


def request(
    path: str,
    *,
    token: str | None = None,
    data: bytes | None = None,
    content_type: str | None = None,
    method: str | None = None,
) -> bytes:
    headers = {"Accept": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    if content_type:
        headers["Content-Type"] = content_type
    operation = urllib.request.Request(
        f"{API_ROOT}{path}", data=data, headers=headers, method=method
    )
    with urllib.request.urlopen(operation, timeout=REQUEST_TIMEOUT) as response:
        return response.read()


def json_request(
    path: str,
    payload: dict[str, Any],
    *,
    token: str | None = None,
    method: str = "POST",
) -> Any:
    body = json.dumps(payload, separators=(",", ":"), sort_keys=True).encode("utf-8")
    return json.loads(
        request(
            path,
            token=token,
            data=body,
            content_type="application/json",
            method=method,
        )
    )


def login() -> str:
    payload = json_request("/api/auth/login", {"email": EMAIL, "password": PASSWORD})
    token = payload.get("accessToken") if isinstance(payload, dict) else None
    if not isinstance(token, str) or not token:
        raise RuntimeError("Immich login response did not contain an access token")
    return token


def ensure_admin() -> str:
    try:
        json_request(
            "/api/auth/admin-sign-up",
            {"email": EMAIL, "name": "BoxFerry Admin", "password": PASSWORD},
        )
    except urllib.error.HTTPError as error:
        if error.code not in {400, 409}:
            raise
    return login()


def wait_for_api() -> str:
    deadline = time.monotonic() + PROCESSING_DEADLINE
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            request("/api/server/ping")
            return ensure_admin()
        except (OSError, ValueError, urllib.error.HTTPError) as error:
            last_error = error
        time.sleep(2)
    raise RuntimeError(f"Immich API did not become ready: {last_error}")


def disable_machine_learning(token: str) -> None:
    config = json.loads(request("/api/system-config", token=token))
    if not isinstance(config, dict) or not isinstance(config.get("machineLearning"), dict):
        raise RuntimeError("Immich system configuration omitted machineLearning settings")
    machine_learning = config["machineLearning"]
    if "enabled" in machine_learning:
        machine_learning["enabled"] = False
    disabled = 0
    for key in ("clip", "duplicateDetection", "facialRecognition"):
        section = machine_learning.get(key)
        if isinstance(section, dict) and "enabled" in section:
            section["enabled"] = False
            disabled += 1
    if disabled == 0 and "enabled" not in machine_learning:
        raise RuntimeError("Immich ML configuration has no reviewed enable switches")
    json_request("/api/system-config", config, token=token, method="PUT")


def multipart_asset(payload: bytes, phase: str) -> tuple[str, bytes]:
    boundary = f"boxferry-{sha256(payload)[:24]}"
    fields = {
        "deviceAssetId": f"boxferry-{phase}-{sha256(payload)[:20]}",
        "deviceId": "boxferry-conformance",
        "fileCreatedAt": "2001-01-01T00:00:00.000Z",
        "fileModifiedAt": "2001-01-01T00:00:00.000Z",
        "isFavorite": "false",
    }
    chunks: list[bytes] = []
    for name, value in fields.items():
        chunks.append(
            (
                f"--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\""
                f"\r\n\r\n{value}\r\n"
            ).encode("ascii")
        )
    chunks.extend(
        [
            (
                f"--{boundary}\r\nContent-Disposition: form-data; name=\"assetData\"; "
                f"filename=\"boxferry-{phase}.png\"\r\nContent-Type: image/png\r\n\r\n"
            ).encode("ascii"),
            payload,
            f"\r\n--{boundary}--\r\n".encode("ascii"),
        ]
    )
    return f"multipart/form-data; boundary={boundary}", b"".join(chunks)


def upload_asset(token: str, payload: bytes, phase: str) -> str:
    content_type, body = multipart_asset(payload, phase)
    response = json.loads(
        request("/api/assets", token=token, data=body, content_type=content_type)
    )
    identifier = response.get("id") if isinstance(response, dict) else None
    if not isinstance(identifier, str) or not identifier:
        raise RuntimeError(f"Immich upload response omitted asset ID: {response!r}")
    return identifier


def get_derivative(token: str, identifier: str, size: str) -> bytes:
    query = urllib.parse.urlencode({"size": size})
    return request(f"/api/assets/{identifier}/thumbnail?{query}", token=token)


def wait_for_processing(token: str, identifier: str) -> tuple[bytes, bytes]:
    deadline = time.monotonic() + PROCESSING_DEADLINE
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            asset = json.loads(request(f"/api/assets/{identifier}", token=token))
            exif = asset.get("exifInfo") if isinstance(asset, dict) else None
            if not isinstance(exif, dict):
                raise RuntimeError("metadata extraction has not created exifInfo")
            preview = get_derivative(token, identifier, "preview")
            thumbnail = get_derivative(token, identifier, "thumbnail")
            if preview and thumbnail:
                return preview, thumbnail
        except (OSError, ValueError, RuntimeError, urllib.error.HTTPError) as error:
            last_error = error
        time.sleep(2)
    raise RuntimeError(f"Immich media processing exceeded {PROCESSING_DEADLINE}s: {last_error}")


def prove_asset(token: str, record: dict[str, Any]) -> None:
    identifier = str(record["id"])
    original = request(f"/api/assets/{identifier}/original", token=token)
    if sha256(original) != record["original_sha256"]:
        raise RuntimeError(f"original bytes changed for Immich asset {identifier}")
    preview, thumbnail = wait_for_processing(token, identifier)
    original_digest = record["original_sha256"]
    if not preview or sha256(preview) == original_digest:
        raise RuntimeError("Immich preview is empty or identical to the original")
    if not thumbnail or sha256(thumbnail) == original_digest:
        raise RuntimeError("Immich thumbnail is empty or identical to the original")
    expected_preview = record.get("preview_sha256")
    expected_thumbnail = record.get("thumbnail_sha256")
    if expected_preview is not None and expected_preview != sha256(preview):
        raise RuntimeError("Immich preview changed after recreation")
    if expected_thumbnail is not None and expected_thumbnail != sha256(thumbnail):
        raise RuntimeError("Immich thumbnail changed after recreation")
    record["preview_sha256"] = sha256(preview)
    record["thumbnail_sha256"] = sha256(thumbnail)


def ingest(phase: str, state_path: Path, work_dir: Path) -> None:
    payload = generate_twice(work_dir)
    token = wait_for_api()
    disable_machine_learning(token)
    record: dict[str, Any] = {
        "phase": phase,
        "original_sha256": sha256(payload),
        "id": upload_asset(token, payload, phase),
    }
    prove_asset(token, record)
    state_path.write_text(
        json.dumps({"schema": 1, "assets": [record]}, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def verify(state_path: Path) -> None:
    state = json.loads(state_path.read_text(encoding="utf-8"))
    if state.get("schema") != 1 or len(state.get("assets", [])) != 1:
        raise RuntimeError("probe state must contain exactly one asset")
    token = wait_for_api()
    disable_machine_learning(token)
    prove_asset(token, state["assets"][0])


def self_test(work_dir: Path) -> None:
    first = generate_twice(work_dir / "first")
    second = generate_twice(work_dir / "second")
    if first != second or not first.startswith(b"\x89PNG\r\n\x1a\n"):
        raise RuntimeError("cross-directory PNG generation differs")
    content_type, body = multipart_asset(first, "selftest")
    if "multipart/form-data" not in content_type or body.count(first) != 1:
        raise RuntimeError("multipart upload body is not bounded or complete")


def main() -> None:
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest="command", required=True)
    self_parser = commands.add_parser("self-test")
    self_parser.add_argument("--work-dir", type=Path, required=True)
    commands.add_parser("ready")
    ingest_parser = commands.add_parser("ingest")
    ingest_parser.add_argument("--phase", required=True)
    ingest_parser.add_argument("--state", type=Path, required=True)
    ingest_parser.add_argument("--work-dir", type=Path, required=True)
    verify_parser = commands.add_parser("verify")
    verify_parser.add_argument("--state", type=Path, required=True)
    arguments = parser.parse_args()
    if arguments.command == "self-test":
        if arguments.work_dir.exists():
            shutil.rmtree(arguments.work_dir)
        self_test(arguments.work_dir)
    elif arguments.command == "ready":
        token = wait_for_api()
        disable_machine_learning(token)
    elif arguments.command == "ingest":
        ingest(arguments.phase, arguments.state, arguments.work_dir)
    elif arguments.command == "verify":
        verify(arguments.state)


if __name__ == "__main__":
    main()
