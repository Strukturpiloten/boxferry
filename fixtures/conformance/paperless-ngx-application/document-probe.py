#!/usr/bin/env python3
"""Deterministic Paperless-ngx document generator and API acceptance probe."""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import time
from typing import Any
import urllib.error
import urllib.parse
import urllib.request
import zipfile


API_ROOT = os.environ.get("BF_PAPERLESS_URL", "http://127.0.0.1:8000").rstrip("/")
USERNAME = os.environ.get("BF_PAPERLESS_USER", "boxferry-admin")
PASSWORD = os.environ.get("BF_PAPERLESS_PASSWORD", "boxferry-public-admin-canary")
REQUEST_TIMEOUT = 20
PROCESSING_DEADLINE = 480
ZIP_TIMESTAMP = (2001, 1, 1, 0, 0, 0)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def zip_bytes(entries: list[tuple[str, bytes, int]]) -> bytes:
    output = io.BytesIO()
    with zipfile.ZipFile(output, "w") as archive:
        for name, payload, compression in entries:
            info = zipfile.ZipInfo(name, ZIP_TIMESTAMP)
            info.compress_type = compression
            info.create_system = 3
            info.external_attr = 0o100644 << 16
            archive.writestr(info, payload)
    return output.getvalue()


def pdf_bytes(text: str) -> bytes:
    escaped = text.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")
    stream = f"BT /F1 12 Tf 72 720 Td ({escaped}) Tj ET\n".encode("ascii")
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
        b"<< /Length " + str(len(stream)).encode("ascii") + b" >>\nstream\n" + stream + b"endstream",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ]
    document = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    offsets = [0]
    for number, body in enumerate(objects, 1):
        offsets.append(len(document))
        document.extend(f"{number} 0 obj\n".encode("ascii"))
        document.extend(body)
        document.extend(b"\nendobj\n")
    xref = len(document)
    document.extend(f"xref\n0 {len(objects) + 1}\n".encode("ascii"))
    document.extend(b"0000000000 65535 f \n")
    for offset in offsets[1:]:
        document.extend(f"{offset:010d} 00000 n \n".encode("ascii"))
    document.extend(
        f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode(
            "ascii"
        )
    )
    return bytes(document)


def docx_bytes(text: str) -> bytes:
    escaped = text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
    content_types = b"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
 <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
 <Default Extension="xml" ContentType="application/xml"/>
 <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"""
    relationships = b"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
 <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"""
    document = (
        """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
 <w:p><w:r><w:t>"""
        + escaped
        + """</w:t></w:r></w:p><w:sectPr/></w:body></w:document>"""
    ).encode("utf-8")
    return zip_bytes(
        [
            ("[Content_Types].xml", content_types, zipfile.ZIP_DEFLATED),
            ("_rels/.rels", relationships, zipfile.ZIP_DEFLATED),
            ("word/document.xml", document, zipfile.ZIP_DEFLATED),
        ]
    )


def odt_bytes(text: str) -> bytes:
    escaped = text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
    content = (
        """<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.3">
 <office:body><office:text><text:p>"""
        + escaped
        + """</text:p></office:text></office:body></office:document-content>"""
    ).encode("utf-8")
    manifest = b"""<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3">
 <manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.text"/>
 <manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
</manifest:manifest>"""
    return zip_bytes(
        [
            ("mimetype", b"application/vnd.oasis.opendocument.text", zipfile.ZIP_STORED),
            ("META-INF/manifest.xml", manifest, zipfile.ZIP_DEFLATED),
            ("content.xml", content, zipfile.ZIP_DEFLATED),
        ]
    )


def generated_documents(phase: str) -> dict[str, tuple[str, bytes]]:
    documents: dict[str, tuple[str, bytes]] = {}
    for extension, builder in (("pdf", pdf_bytes), ("docx", docx_bytes), ("odt", odt_bytes)):
        marker = f"bfpaperlessbodytoken{phase}{extension}x9q7"
        documents[extension] = (marker, builder(marker))
    return documents


def verify_determinism(work_dir: Path, phase: str) -> dict[str, tuple[str, bytes]]:
    first = generated_documents(phase)
    second = generated_documents(phase)
    if {key: sha256(value[1]) for key, value in first.items()} != {
        key: sha256(value[1]) for key, value in second.items()
    }:
        raise RuntimeError("document generation is not deterministic")
    work_dir.mkdir(parents=True, exist_ok=True)
    for extension, (_, payload) in first.items():
        (work_dir / f"{phase}.{extension}").write_bytes(payload)
    return first


def request(
    path: str,
    *,
    token: str | None = None,
    data: bytes | None = None,
    content_type: str | None = None,
) -> bytes:
    headers = {"Accept": "application/json"}
    if token:
        headers["Authorization"] = f"Token {token}"
    if content_type:
        headers["Content-Type"] = content_type
    operation = urllib.request.Request(f"{API_ROOT}{path}", data=data, headers=headers)
    with urllib.request.urlopen(operation, timeout=REQUEST_TIMEOUT) as response:
        return response.read()


def wait_for_token() -> str:
    body = urllib.parse.urlencode({"username": USERNAME, "password": PASSWORD}).encode("ascii")
    deadline = time.monotonic() + PROCESSING_DEADLINE
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            payload = json.loads(
                request(
                    "/api/token/",
                    data=body,
                    content_type="application/x-www-form-urlencoded",
                )
            )
            token = payload.get("token")
            if isinstance(token, str) and token:
                return token
        except (OSError, ValueError, urllib.error.HTTPError) as error:
            last_error = error
        time.sleep(2)
    raise RuntimeError(f"Paperless API did not become ready: {last_error}")


def multipart_document(filename: str, title: str, payload: bytes) -> tuple[str, bytes]:
    boundary = f"boxferry-{sha256(filename.encode('utf-8'))[:24]}"
    chunks = [
        f"--{boundary}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\n{title}\r\n".encode(),
        (
            f"--{boundary}\r\nContent-Disposition: form-data; name=\"document\"; "
            f"filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        ).encode(),
        payload,
        f"\r\n--{boundary}--\r\n".encode(),
    ]
    return f"multipart/form-data; boundary={boundary}", b"".join(chunks)


def task_id(response: bytes) -> str:
    payload = json.loads(response)
    if isinstance(payload, str):
        return payload
    if isinstance(payload, dict):
        for key in ("task_id", "id"):
            value = payload.get(key)
            if isinstance(value, str) and value:
                return value
    raise RuntimeError(f"upload response did not contain a task ID: {payload!r}")


def related_document(value: Any) -> int | None:
    if isinstance(value, dict):
        candidate = value.get("related_document") or value.get("document_id")
        if isinstance(candidate, int):
            return candidate
        if isinstance(candidate, str) and candidate.isdigit():
            return int(candidate)
        candidates = value.get("related_document_ids")
        if isinstance(candidates, list):
            for item in candidates:
                if isinstance(item, int):
                    return item
                if isinstance(item, str) and item.isdigit():
                    return int(item)
        for key in ("result", "task_result", "result_data"):
            nested = value.get(key)
            if isinstance(nested, str):
                try:
                    nested = json.loads(nested)
                except ValueError:
                    continue
            result = related_document(nested)
            if result is not None:
                return result
    if isinstance(value, list):
        for item in value:
            result = related_document(item)
            if result is not None:
                return result
    return None


def wait_for_task(token: str, identifier: str) -> int:
    deadline = time.monotonic() + PROCESSING_DEADLINE
    while time.monotonic() < deadline:
        payload = json.loads(
            request(f"/api/tasks/?task_id={urllib.parse.quote(identifier)}", token=token)
        )
        rows = payload.get("results", []) if isinstance(payload, dict) else payload
        if rows:
            row = rows[0]
            status = str(row.get("status", "")).upper() if isinstance(row, dict) else ""
            if status in {"FAILURE", "FAILED"}:
                raise RuntimeError(f"Paperless ingestion task failed: {row!r}")
            if status in {"SUCCESS", "SUCCEEDED"}:
                document = related_document(row)
                if document is not None:
                    return document
        time.sleep(2)
    raise RuntimeError(f"Paperless ingestion task {identifier} exceeded {PROCESSING_DEADLINE}s")


def search_document(token: str, marker: str, expected_id: int) -> None:
    deadline = time.monotonic() + PROCESSING_DEADLINE
    query = urllib.parse.quote(marker)
    while time.monotonic() < deadline:
        payload = json.loads(request(f"/api/documents/?query={query}", token=token))
        rows = payload.get("results", []) if isinstance(payload, dict) else payload
        if any(isinstance(row, dict) and row.get("id") == expected_id for row in rows):
            return
        time.sleep(2)
    raise RuntimeError(f"search did not return document {expected_id} for marker {marker}")


def prove_document(token: str, record: dict[str, Any]) -> None:
    identifier = int(record["id"])
    marker = str(record["marker"])
    search_document(token, marker, identifier)
    original = request(f"/api/documents/{identifier}/download/?original=true", token=token)
    if sha256(original) != record["original_sha256"]:
        raise RuntimeError(f"original bytes changed for document {identifier}")
    if record["extension"] in {"docx", "odt"}:
        archived = request(f"/api/documents/{identifier}/download/", token=token)
        if not archived.startswith(b"%PDF-"):
            raise RuntimeError(f"converter archive for document {identifier} is not PDF")
        observed_archive = sha256(archived)
        expected_archive = record.get("archive_sha256")
        if expected_archive is not None and expected_archive != observed_archive:
            raise RuntimeError(f"converter archive changed for document {identifier}")
        record["archive_sha256"] = observed_archive


def ingest(phase: str, state_path: Path, work_dir: Path) -> None:
    documents = verify_determinism(work_dir, phase)
    token = wait_for_token()
    records: list[dict[str, Any]] = []
    for extension, (marker, payload) in documents.items():
        filename = f"boxferry-{phase}.{extension}"
        title = f"BoxFerry migration {phase} {extension.upper()}"
        content_type, body = multipart_document(filename, title, payload)
        identifier = task_id(
            request(
                "/api/documents/post_document/",
                token=token,
                data=body,
                content_type=content_type,
            )
        )
        record: dict[str, Any] = {
            "phase": phase,
            "extension": extension,
            "title": title,
            "marker": marker,
            "original_sha256": sha256(payload),
            "id": wait_for_task(token, identifier),
        }
        prove_document(token, record)
        records.append(record)
    state: dict[str, Any] = {"schema": 1, "documents": []}
    if state_path.exists():
        state = json.loads(state_path.read_text(encoding="utf-8"))
    state["documents"].extend(records)
    state_path.write_text(json.dumps(state, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def verify(state_path: Path) -> None:
    state = json.loads(state_path.read_text(encoding="utf-8"))
    if state.get("schema") != 1 or not state.get("documents"):
        raise RuntimeError("probe state is missing its schema or document evidence")
    token = wait_for_token()
    for record in state["documents"]:
        prove_document(token, record)


def self_test(work_dir: Path) -> None:
    first = verify_determinism(work_dir / "first", "selftest")
    second = verify_determinism(work_dir / "second", "selftest")
    if {key: sha256(value[1]) for key, value in first.items()} != {
        key: sha256(value[1]) for key, value in second.items()
    }:
        raise RuntimeError("cross-directory generation differs")
    if not first["pdf"][1].startswith(b"%PDF-"):
        raise RuntimeError("generated PDF signature is invalid")
    for extension, (marker, _) in first.items():
        title = f"BoxFerry migration selftest {extension.upper()}"
        if not marker.isalnum() or marker.casefold() in title.casefold():
            raise RuntimeError("search marker must be one body-only token")
    with zipfile.ZipFile(io.BytesIO(first["docx"][1])) as archive:
        if "word/document.xml" not in archive.namelist():
            raise RuntimeError("generated DOCX package is incomplete")
    with zipfile.ZipFile(io.BytesIO(first["odt"][1])) as archive:
        if archive.read("mimetype") != b"application/vnd.oasis.opendocument.text":
            raise RuntimeError("generated ODT package is incomplete")
    if related_document({"related_document_ids": [7]}) != 7:
        raise RuntimeError("Paperless API v10 task document IDs are not decoded")
    if related_document({"result_data": {"related_document_ids": ["8"]}}) != 8:
        raise RuntimeError("nested Paperless API v10 task results are not decoded")


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    self_parser = subparsers.add_parser("self-test")
    self_parser.add_argument("--work-dir", type=Path, required=True)
    ingest_parser = subparsers.add_parser("ingest")
    ingest_parser.add_argument("--phase", choices=("baseline", "second"), required=True)
    ingest_parser.add_argument("--state", type=Path, required=True)
    ingest_parser.add_argument("--work-dir", type=Path, required=True)
    subparsers.add_parser("ready")
    verify_parser = subparsers.add_parser("verify")
    verify_parser.add_argument("--state", type=Path, required=True)
    return parser.parse_args()


def main() -> None:
    arguments = parse_arguments()
    if arguments.command == "self-test":
        if arguments.work_dir.exists():
            shutil.rmtree(arguments.work_dir)
        self_test(arguments.work_dir)
    elif arguments.command == "ingest":
        ingest(arguments.phase, arguments.state, arguments.work_dir)
    elif arguments.command == "ready":
        wait_for_token()
    else:
        verify(arguments.state)


if __name__ == "__main__":
    main()
