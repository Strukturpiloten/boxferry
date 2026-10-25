#!/usr/bin/env python3
"""Closed data-only build handoff between fresh hosted runners.

Hashes bind bytes to a successful reviewed build job; they are not compiler
attestations. This helper never imports or executes transferred code.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import stat
import time


SPEC = importlib.util.spec_from_file_location(
    "reviewed_transfer_evidence", Path(__file__).with_name("reviewed-docker-consumer-evidence.py"))
assert SPEC is not None and SPEC.loader is not None
proof = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(proof)
auth = proof.auth
WORKSPACE = Path("/home/runner/work/boxferry/boxferry")
LAYOUT = "github-hosted-boxferry-v1"
MAX_BINARY = 512 * 1024 * 1024
MAX_TOTAL = 1024 * 1024 * 1024
MAX_JSON = 128 * 1024
FRESH_SECONDS = 120
BINARIES = ("volume-fixtures", "host-metadata", "inert-tests")
EXAMPLES = {"volume-fixtures": "docker-volume-fixture-rehearsal", "host-metadata": "docker-host-metadata"}


def artifact_id(value):
    auth.require(type(value) is str and re.fullmatch(r"[1-9][0-9]{0,18}", value) is not None)
    auth.positive(int(value))
    return value


def artifact_name(kind, lane, context):
    auth.require(kind in ("build", "binding") and lane in auth.LANES)
    return (f"reviewed-docker-{kind}-{lane}-{context['box_sha']}-{context['lens_sha']}-"
            f"{context['run_id']}-{context['run_attempt']}")


def layout(args, environment):
    """Caller paths are assertions, never values adopted from uploaded metadata."""
    auth.require(environment.get("GITHUB_WORKSPACE") == str(WORKSPACE))
    for attribute, name in (("trusted_root", "trusted"), ("box_root", "candidate"),
                            ("lens_root", "lens-execution"), ("execution_root", "execution"),
                            ("private", "private")):
        if hasattr(args, attribute):
            auth.require(getattr(args, attribute) == WORKSPACE / name)


def binary(path: Path, destination=None):
    """Bound streaming reads; refuse link/special files and changes during reads."""
    auth.require(path.is_absolute() and path.resolve(strict=True) == path)
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as stream:
        before = os.fstat(stream.fileno())
        auth.require(stat.S_ISREG(before.st_mode) and before.st_nlink == 1
                     and 0 < before.st_size <= MAX_BINARY)
        digest = hashlib.sha256()
        size = 0
        while raw := stream.read(1024 * 1024):
            size += len(raw)
            auth.require(size <= MAX_BINARY)
            digest.update(raw)
            if destination is not None:
                destination.write(raw)
        after = os.fstat(stream.fileno())
        identity = lambda item: (item.st_dev, item.st_ino, item.st_size, item.st_mtime_ns, item.st_ctime_ns)
        auth.require(size == before.st_size and identity(before) == identity(after))
    return {"sha256": digest.hexdigest(), "size": size}


def create(path, value):
    auth.require(path.is_absolute() and path.parent.resolve(strict=True) == path.parent)
    raw = proof.encoded(value)
    auth.require(len(raw) <= MAX_JSON)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "wb") as stream:
        stream.write(raw)


def document(path):
    raw = auth.read(path, MAX_JSON)
    value = json.loads(raw, object_pairs_hook=auth.no_duplicates)
    auth.require(raw == proof.encoded(value))
    return value


def manifest(pair, context, lane, lens, metadata, inert):
    auth.require(lane in auth.LANES)
    auth.keys(metadata, BINARIES)
    for record in metadata.values():
        auth.keys(record, ("sha256", "size"))
        auth.sha(record["sha256"])
        auth.require(type(record["size"]) is int and 0 < record["size"] <= MAX_BINARY)
    auth.require(sum(record["size"] for record in metadata.values()) <= MAX_TOTAL)
    proof.validate_inert(inert)
    auth.require(inert["binary_sha256"] == metadata["inert-tests"]["sha256"])
    return {"schema": 1, "layout": LAYOUT, "context": context, "pair": pair,
            "producer": {"job": "lanes", "mode": "build", "lane": lane},
            "source_binding": proof.runtime_source_binding(pair), "files": metadata,
            "recipes": {profile: proof.build_command(lens, profile) for profile in EXAMPLES},
            "builds": {profile: proof.sanitized_build(pair, lens, profile, metadata[profile]["sha256"])
                       for profile in EXAMPLES}, "inert": inert}


def validate_manifest(value, *, pair, context, lane, lens):
    auth.keys(value, ("schema", "layout", "context", "pair", "producer", "source_binding", "files",
                      "recipes", "builds", "inert"))
    auth.require(type(value["schema"]) is int)
    auth.keys(value["context"], context)
    for field in ("run_id", "run_attempt", "box_pr", "lens_pr"):
        auth.positive(value["context"][field])
    expected = manifest(pair, context, lane, lens, value["files"], value["inert"])
    auth.require(value == expected)
    return expected


def bundle(root, *, pair, context, lane, lens):
    auth.require(root.is_absolute() and root.resolve(strict=True) == root and root.is_dir()
                 and set(path.name for path in root.iterdir()) == {*BINARIES, "manifest.json"})
    value = document(root / "manifest.json")
    validate_manifest(value, pair=pair, context=context, lane=lane, lens=lens)
    auth.require(sum((root / name).lstat().st_size for name in BINARIES) <= MAX_TOTAL)
    metadata = {name: binary(root / name) for name in BINARIES}
    auth.require(value["files"] == metadata)
    return value


def export(args, *, pair, context, state):
    auth.require(args.lane in auth.LANES and args.bundle_root is not None)
    layout(args, os.environ)
    auth.keys(state, ("context", "builds", "inert", "test_binary"))
    auth.require(state["context"] == context)
    sources = {profile: args.box_root / "target/debug/examples" / example for profile, example in EXAMPLES.items()}
    test_binary = Path(state["test_binary"])
    auth.require(test_binary.is_relative_to(args.box_root / "target/debug/deps"))
    sources["inert-tests"] = test_binary
    args.bundle_root.mkdir(mode=0o700)
    metadata = {}
    for name, source in sources.items():
        descriptor = os.open(args.bundle_root / name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(descriptor, "wb") as stream:
            metadata[name] = binary(source, stream)
        auth.require(sum(item["size"] for item in metadata.values()) <= MAX_TOTAL)
    value = manifest(pair, context, args.lane, args.lens_root, metadata, state["inert"])
    auth.require(value["builds"] == state["builds"])
    create(args.bundle_root / "manifest.json", value)


def draft(value, identifier, lane, context):
    return {"schema": 1, "kind": "trusted-reviewed-build-binding", "artifact": {
        "id": artifact_id(identifier), "name": artifact_name("build", lane, context)},
        "manifest": value, "manifest_sha256": hashlib.sha256(proof.encoded(value)).hexdigest()}


def validate_draft(value, *, pair, context, lane, lens, identifier=None):
    auth.keys(value, ("schema", "kind", "artifact", "manifest", "manifest_sha256"))
    auth.require(type(value["schema"]) is int)
    auth.keys(value["artifact"], ("id", "name"))
    selected = artifact_id(value["artifact"]["id"])
    if identifier is not None:
        auth.require(selected == artifact_id(identifier))
    validate_manifest(value["manifest"], pair=pair, context=context, lane=lane, lens=lens)
    auth.require(value == draft(value["manifest"], selected, lane, context))


def issue(value, *, pair, context, lane, lens, now=None):
    validate_draft(value, pair=pair, context=context, lane=lane, lens=lens)
    issued = int(time.time()) if now is None else now
    auth.positive(issued)
    return {**value, "issued_at": issued, "expires_at": issued + FRESH_SECONDS}


def binding(value, *, pair, context, lane, lens, identifier=None, fresh=False, now=None):
    auth.keys(value, ("schema", "kind", "artifact", "manifest", "manifest_sha256", "issued_at", "expires_at"))
    validate_draft({key: item for key, item in value.items() if key not in ("issued_at", "expires_at")},
                   pair=pair, context=context, lane=lane, lens=lens, identifier=identifier)
    auth.positive(value["issued_at"])
    auth.positive(value["expires_at"])
    auth.require(value["expires_at"] == value["issued_at"] + FRESH_SECONDS)
    if fresh:
        current = time.time() if now is None else now
        auth.require(value["issued_at"] <= current < value["expires_at"])
    return value["manifest"]


def restore(args, *, pair, context, box_files):
    layout(args, os.environ)
    auth.require(len(os.environ.get("BUILD_BINDING_JSON", "").encode()) <= MAX_JSON)
    value = json.loads(os.environ.get("BUILD_BINDING_JSON", ""), object_pairs_hook=auth.no_duplicates)
    expected = binding(value, pair=pair, context=context, lane=args.lane, lens=args.lens_root,
                       identifier=args.build_artifact_id, fresh=True)
    actual = bundle(args.bundle_root, pair=pair, context=context, lane=args.lane, lens=args.lens_root)
    auth.require(actual == expected)
    # Nothing from the bundle is executed. Only fixed trusted destination names are used.
    auth.copy_closure(box_files, args.execution_root)
    args.private.mkdir(mode=0o700)
    targets = {profile: args.box_root / "target/debug/examples" / example for profile, example in EXAMPLES.items()}
    targets["inert-tests"] = args.box_root / "target/debug/deps/reviewed-transferred-inert-tests"
    for name, destination in targets.items():
        destination.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        auth.require(destination.parent.resolve(strict=True) == destination.parent)
        descriptor = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(descriptor, "wb") as stream:
            auth.require(binary(args.bundle_root / name, stream) == actual["files"][name])
        destination.chmod(0o700)
    for profile in EXAMPLES:
        create(args.private / (profile + "-receipt.json"), proof.build_receipt(
            pair, args.lens_root, profile, actual["files"][profile]["sha256"]))
    create(args.private / "build-state.json", {"context": context, "builds": actual["builds"],
        "inert": actual["inert"], "test_binary": str(targets["inert-tests"])})
    binding(value, pair=pair, context=context, lane=args.lane, lens=args.lens_root,
            identifier=args.build_artifact_id, fresh=True)
    return value


def validate_results(root, *, expected_context, pair, lens, results):
    auth.require(root.is_absolute() and root.resolve(strict=True) == root and root.is_dir())
    auth.require(set(path.name for path in root.iterdir()) == {
        artifact_name("binding", lane, expected_context) for lane in auth.LANES})
    identifiers = set()
    for lane in auth.LANES:
        directory = root / artifact_name("binding", lane, expected_context)
        auth.require(directory.resolve(strict=True) == directory and directory.is_dir()
                     and set(path.name for path in directory.iterdir()) == {"binding.json"})
        value = document(directory / "binding.json")
        selected = binding(value, pair=pair, context=expected_context, lane=lane, lens=lens)
        auth.require(value["artifact"]["id"] not in identifiers)
        identifiers.add(value["artifact"]["id"])
        consumer = auth.document(results / proof.artifact_name(lane, expected_context) / "consumer.json", MAX_JSON)
        auth.require(consumer["builds"] == selected["builds"] and consumer["inert"] == selected["inert"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("inspect", "issue"))
    for name in ("trusted-root", "box-root", "lens-root", "bundle-root", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    for name in ("box-pr", "lens-pr", "run-id", "attempt"):
        parser.add_argument("--" + name, type=int, required=True)
    for name in ("box-sha", "lens-sha", "trusted-sha", "artifact-id"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--lane", choices=auth.LANES, required=True)
    parser.add_argument("--checked-at", type=int)
    args = parser.parse_args()
    layout(args, os.environ)
    pair, digest = auth.authorize(args.trusted_root, args.box_pr, args.box_sha, args.lens_pr, args.lens_sha)
    context = proof.context(pair, digest, args.trusted_sha, args.run_id, args.attempt)
    if args.command == "inspect":
        auth.require(args.checked_at is None)
        auth.require(auth.git(args.trusted_root, "rev-parse", "HEAD") == args.trusted_sha)
        auth.verify_source(args.box_root, pair["boxferry"])
        auth.verify_source(args.lens_root, pair["docker_lens"])
        value = bundle(args.bundle_root, pair=pair, context=context, lane=args.lane, lens=args.lens_root)
        create(args.output, draft(value, args.artifact_id, args.lane, context))
    else:
        # Draft was independently checked before the final canonical GETs and revocation.
        value = document(args.bundle_root)
        validate_draft(value, pair=pair, context=context, lane=args.lane, lens=args.lens_root,
                       identifier=args.artifact_id)
        auth.positive(args.checked_at)
        value = issue(value, pair=pair, context=context, lane=args.lane, lens=args.lens_root, now=args.checked_at)
        binding(value, pair=pair, context=context, lane=args.lane, lens=args.lens_root, fresh=True)
        args.output.mkdir(mode=0o700)
        create(args.output / "binding.json", value)
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
            stream.write("binding=" + proof.encoded(value).decode().strip() + "\n")


if __name__ == "__main__":
    try:
        main()
    except Exception:
        raise SystemExit("reviewed Docker build transfer rejected") from None
