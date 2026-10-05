#!/usr/bin/env python3
"""Verify bounded OCI save metadata without reading or extracting layer payloads."""
from __future__ import annotations

import hashlib
import json
import os
import re
import stat
import sys
import tarfile
import time

MAX_ARCHIVE = 2684354560
MAX_MEMBERS = 8192
MAX_METADATA = 4 * 1024 * 1024
MAX_SECONDS = 60
DIGEST = re.compile(r"sha256:[0-9a-f]{64}\Z")
REFERENCE = re.compile(r"localhost/boxferry-archive/[a-z][a-z0-9-]{0,95}/"
                       r"(?:workload|forgejo|nextcloud|paperless|immich|observability):"
                       r"[a-z0-9][a-z0-9_.-]{0,63}\Z")
MANIFEST = "application/vnd.oci.image.manifest.v1+json"
CONFIG = "application/vnd.oci.image.config.v1+json"
LAYERS = {"application/vnd.oci.image.layer.v1.tar", "application/vnd.oci.image.layer.v1.tar+gzip",
          "application/vnd.oci.image.layer.v1.tar+zstd"}


class InvalidArchive(ValueError):
    """An archive cannot establish an unambiguous serialized identity."""


def require(condition: bool) -> None:
    if not condition:
        raise InvalidArchive("invalid OCI archive metadata")


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result)
        result[key] = value
    return result


def identity(path: str, reference: str) -> str:
    require(REFERENCE.fullmatch(reference) is not None)
    started = time.monotonic()
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as archive:
        before = os.fstat(archive.fileno())
        require(stat.S_ISREG(before.st_mode) and 0 < before.st_size <= MAX_ARCHIVE)
        members = {}
        count = 0
        while True:
            require(time.monotonic() - started < MAX_SECONDS)
            header = archive.read(512)
            require(len(header) == 512)
            if header == bytes(512):
                require(archive.read(512) == bytes(512))
                remaining = before.st_size - archive.tell()
                require(0 <= remaining <= 10240 and archive.read(remaining) == bytes(remaining))
                break
            member = tarfile.TarInfo.frombuf(header, "utf-8", "strict")
            count += 1
            require(count <= MAX_MEMBERS and not member.linkname and not member.sparse)
            name = member.name.removeprefix("./").rstrip("/")
            if not name: name = "."
            require(name not in members)
            offset = archive.tell()
            require(member.size >= 0 and offset + ((member.size + 511) // 512) * 512 <= before.st_size)
            if member.type == tarfile.DIRTYPE:
                require(name in (".", "", "blobs", "blobs/sha256") and member.size == 0)
                require(member.name in (".", "./", "blobs", "blobs/", "./blobs", "./blobs/",
                        "blobs/sha256", "blobs/sha256/", "./blobs/sha256", "./blobs/sha256/"))
            else:
                require(member.type in (tarfile.REGTYPE, tarfile.AREGTYPE) and (name in ("index.json", "oci-layout") or
                        re.fullmatch(r"blobs/sha256/[0-9a-f]{64}", name) is not None))
                require(member.name in (name, "./" + name))
            members[name] = (offset, member.size, member.isreg())
            archive.seek(offset + ((member.size + 511) // 512) * 512)

        def read_json(name: str, expected_size=None, expected_digest=None):
            require(name in members)
            offset, size, regular = members[name]
            require(regular and 0 < size <= MAX_METADATA)
            require(expected_size is None or size == expected_size)
            archive.seek(offset)
            raw = archive.read(size)
            require(len(raw) == size)
            require(expected_digest is None or "sha256:" + hashlib.sha256(raw).hexdigest() == expected_digest)
            document = json.loads(raw, object_pairs_hook=unique_object,
                                  parse_constant=lambda _: require(False))
            require(isinstance(document, dict))
            return document

        def blob_descriptor(value, media_types):
            require(isinstance(value, dict) and value.get("mediaType") in media_types)
            digest, size = value.get("digest"), value.get("size")
            require(isinstance(digest, str) and DIGEST.fullmatch(digest) is not None)
            require(type(size) is int and 0 < size <= MAX_ARCHIVE)
            require(not any(key in value for key in ("urls", "data", "artifactType")))
            annotations = value.get("annotations", {})
            require(isinstance(annotations, dict) and all(isinstance(key, str) and isinstance(item, str)
                    for key, item in annotations.items()))
            name = "blobs/sha256/" + digest[7:]
            require(name in members and members[name][1:] == (size, True))
            return digest, size, name

        require(read_json("oci-layout") == {"imageLayoutVersion": "1.0.0"})
        index = read_json("index.json")
        require(type(index.get("schemaVersion")) is int and index["schemaVersion"] == 2)
        require(index.get("mediaType", "application/vnd.oci.image.index.v1+json") ==
                "application/vnd.oci.image.index.v1+json")
        require(isinstance(index.get("manifests"), list) and len(index["manifests"]) == 1)
        require(not any(key in index for key in ("subject", "artifactType")))
        manifest_descriptor = index["manifests"][0]
        require(isinstance(manifest_descriptor, dict) and
                manifest_descriptor.get("annotations", {}).get("org.opencontainers.image.ref.name") == reference)
        digest, size, name = blob_descriptor(manifest_descriptor, {MANIFEST})
        manifest = read_json(name, size, digest)
        require(type(manifest.get("schemaVersion")) is int and manifest["schemaVersion"] == 2 and
                manifest.get("mediaType", MANIFEST) == MANIFEST)
        require(not any(key in manifest for key in ("subject", "artifactType")))
        config_digest, size, name = blob_descriptor(manifest.get("config"), {CONFIG})
        config = read_json(name, size, config_digest)
        require(isinstance(config.get("architecture"), str) and config["architecture"] and
                isinstance(config.get("os"), str) and config["os"])
        require(isinstance(config.get("config", {}), dict) and isinstance(config.get("history", []), list))
        rootfs = config.get("rootfs")
        require(isinstance(rootfs, dict) and rootfs.get("type") == "layers" and
                isinstance(rootfs.get("diff_ids"), list))
        layers = manifest.get("layers")
        require(isinstance(layers, list) and len(layers) == len(rootfs["diff_ids"]) <= MAX_MEMBERS)
        for layer, diff_id in zip(layers, rootfs["diff_ids"]):
            require(isinstance(diff_id, str) and DIGEST.fullmatch(diff_id) is not None)
            blob_descriptor(layer, LAYERS)
        after = os.fstat(archive.fileno())
        require((before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) ==
                (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns))
        require(time.monotonic() - started < MAX_SECONDS)
        return config_digest[7:]


def main() -> int:
    try:
        require(len(sys.argv) == 3)
        print(identity(sys.argv[1], sys.argv[2]))
        return 0
    except (OSError, ValueError, TypeError, KeyError, AttributeError, RecursionError, tarfile.HeaderError):
        print("OCI archive identity verification failed.", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
