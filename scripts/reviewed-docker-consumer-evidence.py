#!/usr/bin/env python3
"""Read-only trusted validator for sanitized native-volume and inert receipts.

This module never imports a candidate validator, invokes a runtime or turns build
identity receipts into authorization. The trusted-main manifest is mandatory.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import subprocess


SPEC = importlib.util.spec_from_file_location(
    "reviewed_docker_authorization", Path(__file__).with_name("reviewed-docker-consumer-authorization.py"))
assert SPEC is not None and SPEC.loader is not None
auth = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(auth)


def encoded(value) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def identity(value) -> str:
    # Existing native writer's identity convention deliberately has no newline.
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def runtime_source_binding(pair):
    """Sanitized reviewed source identity, never evidence of runtime execution."""
    dependency = pair["docker_lens"]
    files = {name: dependency["files"][name] for name in sorted(auth.OUTER_RUNTIME_FILES)}
    auth.sha(dependency["revision"], auth.SHA)
    auth.sha(dependency["source_sha256"])
    for digest in files.values():
        auth.sha(digest)
    return {"qualification": "reviewed-source-identity-only", "engine": auth.POLICY["outer_engine"],
            "docker_lens_revision": dependency["revision"], "docker_lens_source_sha256": dependency["source_sha256"],
            "files": files, "closure_sha256": identity(files)}


OUTER_PROJECTION_PREFIX = b"OUTER-RUNTIME-OBSERVATION "
OUTER_VERSION = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+(?:[-+][A-Za-z0-9_.-]+)?\Z")


def validate_outer_runtime(value):
    auth.keys(value, ("qualification", "engine", "runner", "runner_image_version", "client_version",
                      "server_version", "rootful", "local", "native_qualified"))
    auth.require(value["qualification"] == "test-only-outer-prerequisite-observation"
                 and value["engine"] == "docker" and value["runner"] == "ubuntu-24.04"
                 and value["rootful"] is True and value["local"] is True and value["native_qualified"] is False)
    auth.require(type(value["runner_image_version"]) is str
                 and re.fullmatch(r"[0-9]{8}\.[0-9]+\.[0-9]+", value["runner_image_version"]) is not None)
    for field in ("client_version", "server_version"):
        auth.require(type(value[field]) is str and len(value[field]) <= 128
                     and OUTER_VERSION.fullmatch(value[field]) is not None)


def outer_runtime_from_log(raw: bytes, lane: str):
    """Whole successful private log, never a first-match or native stdout claim."""
    auth.require(type(raw) is bytes and 0 < len(raw) <= 16 * 1024 * 1024)
    auth.require(lane in auth.LANES)
    lines = [line for line in raw.split(b"\n") if OUTER_PROJECTION_PREFIX.rstrip() in line]
    auth.require(len(lines) == 1 and lines[0].startswith(OUTER_PROJECTION_PREFIX)
                 and len(lines[0]) <= 2048 and raw.endswith(b"\n"))
    # Only the final harness-owned handoff can emit this exact trailing pair.
    auth.require(len(raw.split(b"\n")) >= 3 and raw.split(b"\n")[-3] == lines[0]
                 and re.fullmatch(rb"VOLUME-ONLY REHEARSAL CHECKS-PASSED: lane=" + lane.encode() + rb" fixtures=6 volumes=23 cleanup=verified acceptance=pending-reviewed-native-evidence; no container/image workload or six-application acceptance",
                                  raw.split(b"\n")[-2]) is not None)
    value = json.loads(lines[0][len(OUTER_PROJECTION_PREFIX):], object_pairs_hook=auth.no_duplicates)
    validate_outer_runtime(value)
    auth.require(lines[0] == OUTER_PROJECTION_PREFIX + encoded(value).rstrip(b"\n"))
    return value


def build_command(lens: Path, profile: str) -> list[str]:
    auth.require(profile in {"volume-fixtures", "host-metadata"})
    example, features = ("docker-volume-fixture-rehearsal", "compose,docker") if profile == "volume-fixtures" else (
        "docker-host-metadata", "docker")
    return ["cargo", "build", "--locked", "--package", "boxferry", "--example", example,
            "--no-default-features", "--features", features, "--jobs", "2",
            "--config", f'patch.crates-io.docker-lens.path="{lens}"']


def build_receipt(pair, lens: Path, profile: str, binary_sha: str):
    box, dependency = pair["boxferry"], pair["docker_lens"]
    auth.sha(binary_sha)
    return {
        "schema_version": 2, "boxferry_revision": box["revision"],
        "boxferry_source_sha256": box["source_sha256"], "boxferry_lock_sha256": box["lock_sha256"],
        "docker_lens_revision": dependency["revision"], "docker_lens_source_sha256": dependency["source_sha256"],
        "docker_lens_lock_sha256": dependency["lock_sha256"], "build_command": build_command(lens, profile),
        "override_identity": {"kind": "cargo-crates-io-patch", "package": "docker-lens", "path": str(lens),
                              "revision": dependency["revision"], "source_sha256": dependency["source_sha256"]},
        "binary_sha256": binary_sha,
    }


def sanitized_build(pair, lens: Path, profile: str, binary_sha: str):
    receipt = build_receipt(pair, lens, profile, binary_sha)
    # No host path, command arguments or native payload is uploaded.
    return {"profile": profile, "receipt_schema": 2, "binary_sha256": binary_sha,
            "receipt_sha256": hashlib.sha256(encoded(receipt)).hexdigest(),
            **{key: receipt[key] for key in ("boxferry_revision", "boxferry_source_sha256", "boxferry_lock_sha256",
                                             "docker_lens_revision", "docker_lens_source_sha256", "docker_lens_lock_sha256")}}


def context(pair, manifest_sha: str, trusted_sha: str, run_id: int, attempt: int):
    auth.sha(manifest_sha)
    auth.sha(trusted_sha, auth.SHA)
    auth.positive(run_id)
    auth.positive(attempt)
    return {"repository": "Strukturpiloten/boxferry", "workflow_sha": trusted_sha,
            "run_id": run_id, "run_attempt": attempt, "manifest_sha256": manifest_sha,
            "box_pr": pair["boxferry"]["pr"], "box_sha": pair["boxferry"]["revision"],
            "lens_pr": pair["docker_lens"]["pr"], "lens_sha": pair["docker_lens"]["revision"]}


def native_catalogue(files):
    """Parse only the exact approved image/catalogue/record inputs, without imports."""
    names = {"debian11-rootful": "DEBIAN_ROOTFUL_IMAGE", "debian11-rootless": "DEBIAN_ROOTLESS_IMAGE",
             "upstream-rootful": "UPSTREAM_ROOTFUL_IMAGE", "upstream-rootless": "UPSTREAM_ROOTLESS_IMAGE",
             "fixture": "FIXTURE_IMAGE"}
    assignments = re.findall(r"(?m)^([A-Z_]+)='([^'\n]+)'$", files["scripts/native-conformance.sh"].decode())
    selected = [(key, value) for key, value in assignments if key in names.values()]
    auth.require(len(selected) == 5 and len(dict(selected)) == 5)
    images = dict(selected)
    auth.require(all(re.fullmatch(r"[a-z0-9.-]+(?:/[a-z0-9._-]+)+:[A-Za-z0-9._-]+@sha256:[0-9a-f]{64}", value)
                     for value in images.values()) and len(set(images.values())) == 5)
    catalogue = {lane: images[key] for lane, key in names.items()}
    declarations = re.findall(r"(?ms)^const RECORDS: .*? = \[(.*?)^\];", files["src/reviewed_catalog.rs"].decode())
    auth.require(len(declarations) == 1)
    pattern = re.compile(
        r'\(\s*NativeEvidenceLane::(Debian11Rootful|Debian11Rootless|UpstreamRootful|UpstreamRootless),\s*'
        r'"([0-9a-f]{64})",\s*include_str!\(\s*"\.\./docs/evidence/reviewed/sha256/\2\.json"\s*\),\s*\),')
    matches = list(pattern.finditer(declarations[0]))
    auth.require(len(matches) == 4 and not pattern.sub("", declarations[0]).strip())
    lanes = dict(zip(("Debian11Rootful", "Debian11Rootless", "UpstreamRootful", "UpstreamRootless"), auth.LANES, strict=True))
    profiles = {}
    for match in matches:
        lane, digest = lanes[match[1]], match[2]
        auth.require(lane not in profiles)
        raw = files[f"docs/evidence/reviewed/sha256/{digest}.json"]
        auth.require(hashlib.sha256(raw).hexdigest() == digest)
        record = json.loads(raw, object_pairs_hook=auth.no_duplicates)
        auth.require(type(record.get("schema_version")) is int and record["schema_version"] == 1 and record.get("lane") == lane)
        source = record["identity"]
        if lane.startswith("debian11-"):
            build = source["build"]
            auth.require(build["kind"] == "debian-package" and build["distribution"] == "debian11" and build["package_name"] == "docker.io")
            target_build = {"kind": "debian_package", "revision": build["package_revision"]}
        else:
            auth.require(source["build"] == {"kind": "upstream"})
            target_build = {"kind": "upstream"}
        profiles[lane] = {
            "kind": "target", "build": target_build, "engine_release": source["engine_release"],
            "advertised_api_version": source["advertised_api"], "acquisition_api_version": source["acquisition_api"],
            "rendering_api_version": source["rendering_api"], "daemon_mode": source["mode"], "evidence_sha256": digest,
        }
    auth.require(set(profiles) == set(auth.LANES))
    catalogue["profiles"] = profiles
    return catalogue


def validate_inert(value):
    auth.keys(value, ("kind", "binary_sha256", "tests"))
    auth.require(value["kind"] == "inert-only-no-runtime")
    auth.sha(value["binary_sha256"])
    auth.require(type(value["tests"]) is list and len(value["tests"]) == 2)
    for row, name in zip(value["tests"], auth.INERT_TESTS, strict=True):
        auth.keys(row, ("name", "selected", "passed", "failed", "ignored"))
        auth.require(row["name"] == name)
        for field, expected in (("selected", 1), ("passed", 1), ("failed", 0), ("ignored", 0)):
            auth.require(type(row[field]) is int and row[field] == expected)


def validate_volume(value, *, lane, pair, lens, builds, files, box_files):
    """Validate uploaded completed evidence, never call its runtime writer."""
    auth.keys(value, ("schema", "scope", "lane", "outer_run", "volume_run", "volume_prefix", "api_version",
                      "candidate", "native", "observed", "manifest_sha256", "fixtures", "result", "volumes", "closure"))
    auth.require(type(value["schema"]) is int and value["schema"] == 1 and value["scope"] == "volume-only"
                 and value["lane"] == lane and value["result"] == "checks-passed")
    token = value["outer_run"]
    auth.require(type(token) is str and re.fullmatch(r"[A-Za-z0-9]{8}", token) is not None
                 and value["volume_run"] == f"bf-{token.lower()}" and value["volume_prefix"] == f"bf-volume-{token.lower()}")
    auth.sha(value["manifest_sha256"])
    producer = builds["volume-fixtures"]
    expected_candidate = {"receipt_schema": 2, "receipt_sha256": producer["receipt_sha256"], "build_profile": "volume-fixtures",
                          **{key: producer[key] for key in ("boxferry_revision", "boxferry_source_sha256", "boxferry_lock_sha256",
                                                            "docker_lens_revision", "docker_lens_source_sha256",
                                                            "docker_lens_lock_sha256", "binary_sha256")}}
    auth.require(value["candidate"] == expected_candidate)
    catalogue = native_catalogue(files)
    target = catalogue["profiles"][lane]
    expected_native = {"native_script_sha256": pair["docker_lens"]["files"]["scripts/native-conformance.sh"],
                       "catalogue_sha256": identity(catalogue), "target": target}
    auth.require(value["native"] == expected_native and value["api_version"] == target["rendering_api_version"])
    observed = value["observed"]
    auth.keys(observed, ("release", "api", "package"))
    auth.require(observed["release"] in {target["engine_release"], target["engine_release"].removesuffix("+dfsg1")}
                 and observed["api"] == target["advertised_api_version"]
                 and observed["package"] == target["build"].get("revision", ""))
    auth.require(type(value["fixtures"]) is list and len(value["fixtures"]) == 6)
    expected_outcomes = []
    for row, (name, owner, volumes) in zip(value["fixtures"], auth.FIXTURES, strict=True):
        auth.keys(row, ("id", "source_sha256", "artifact_sha256", "volume_count"))
        path = f"fixtures/conformance/{name}-application/compose.yaml"
        auth.require(row["id"] == name and row["source_sha256"] == hashlib.sha256(box_files[path]).hexdigest()
                     and type(row["volume_count"]) is int and row["volume_count"] == len(volumes))
        auth.sha(row["artifact_sha256"])
        for volume in volumes:
            body = {"Name": f"{value['volume_prefix']}-{volume}", "Labels": {
                "io.boxferry.live-run": value["volume_run"], "io.boxferry.application": f"{value['volume_prefix']}-{owner}"}}
            expected_outcomes.append(identity(body))
    auth.require(len(expected_outcomes) == 23 and type(value["volumes"]) is list and len(value["volumes"]) == 23)
    for row, digest in zip(value["volumes"], expected_outcomes, strict=True):
        auth.keys(row, ("identity_sha256", "cleanup", "absence"))
        auth.require(row["identity_sha256"] == digest and row["cleanup"] in {"removed", "already-absent"}
                     and row["absence"] == "verified")
    closure = value["closure"]
    auth.keys(closure, ("outer", "storage", "disposable", "interruption"))
    auth.require(closure["interruption"] == "none")
    for field, kind, name in (("outer", "container", f"bf-docker-core-{token}"),
                              ("storage", "volume", f"bf-docker-core-data-{token}")):
        auth.require(closure[field] == {"ownership_sha256": identity({"kind": kind, "name": name, "run": token}),
                                      "absence": "verified"})
    auth.keys(closure["disposable"], ("identity_sha256", "absence"))
    auth.sha(closure["disposable"]["identity_sha256"])
    auth.require(closure["disposable"]["absence"] == "verified")


def validate_lane(consumer, volume, *, expected_context, lane, pair, lens, files, box_files):
    auth.keys(consumer, ("schema", "scope", "lane", "result", "context", "builds", "inert", "volume_proof_sha256",
                         "source_binding", "outer_runtime"))
    auth.require(consumer["source_binding"] == runtime_source_binding(pair))
    validate_outer_runtime(consumer["outer_runtime"])
    auth.sha(consumer["volume_proof_sha256"])
    auth.require(consumer["volume_proof_sha256"] == hashlib.sha256(encoded(volume)).hexdigest())
    auth.keys(consumer["context"], expected_context)
    for field in ("run_id", "run_attempt", "box_pr", "lens_pr"):
        auth.positive(consumer["context"][field])
    auth.require(type(consumer["schema"]) is int and consumer["schema"] == 2
                 and consumer["scope"] == auth.POLICY["scope"] and consumer["lane"] == lane
                 and consumer["result"] == "checks-passed" and consumer["context"] == expected_context)
    builds = consumer["builds"]
    auth.keys(builds, ("volume-fixtures", "host-metadata"))
    for profile, build in builds.items():
        auth.sha(build.get("binary_sha256") if type(build) is dict else None)
        auth.require(build == sanitized_build(pair, lens, profile, build["binary_sha256"]))
    validate_inert(consumer["inert"])
    validate_volume(volume, lane=lane, pair=pair, lens=lens, builds=builds, files=files, box_files=box_files)


def artifact_name(lane, expected_context):
    auth.require(lane in auth.LANES)
    return (f"reviewed-docker-consumer-{lane}-{expected_context['box_sha']}-{expected_context['lens_sha']}-"
            f"{expected_context['run_id']}-{expected_context['run_attempt']}")


def aggregate(root: Path, *, expected_context, pair, lens, files, box_files):
    auth.require(root.is_absolute() and root.resolve(strict=True) == root)
    names = {artifact_name(lane, expected_context) for lane in auth.LANES}
    auth.require(set(path.name for path in root.iterdir()) == names)
    seen_runs = set()
    for lane in auth.LANES:
        directory = root / artifact_name(lane, expected_context)
        auth.require(directory.is_dir() and directory.resolve(strict=True) == directory
                     and set(path.name for path in directory.iterdir()) == {"consumer.json", "volume-proof.json"})
        consumer = auth.document(directory / "consumer.json", 128 * 1024)
        raw_volume = auth.read(directory / "volume-proof.json", 16 * 1024)
        volume = json.loads(raw_volume, object_pairs_hook=auth.no_duplicates)
        auth.require(raw_volume == encoded(volume))
        auth.require(type(volume) is dict and type(volume.get("outer_run")) is str
                     and volume["outer_run"] not in seen_runs)
        seen_runs.add(volume["outer_run"])
        validate_lane(consumer, volume, expected_context=expected_context, lane=lane,
                      pair=pair, lens=lens, files=files, box_files=box_files)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("trusted-root", "box-root", "lens-root", "verifier-root", "artifacts"):
        parser.add_argument("--" + name, type=Path, required=True)
    for name in ("box-pr", "lens-pr", "run-id", "attempt"):
        parser.add_argument("--" + name, type=int, required=True)
    for name in ("box-sha", "lens-sha", "trusted-sha"):
        parser.add_argument("--" + name, required=True)
    args = parser.parse_args()
    pair, digest = auth.authorize(args.trusted_root, args.box_pr, args.box_sha, args.lens_pr, args.lens_sha)
    auth.require(auth.git(args.trusted_root, "rev-parse", "HEAD") == args.trusted_sha)
    box_files = auth.verify_source(args.box_root, pair["boxferry"])
    lens_files = auth.verify_source(args.lens_root, pair["docker_lens"])
    # Fresh aggregate tree: no source-side module is imported and no runtime is invoked.
    args.verifier_root.mkdir(mode=0o700)
    auth.copy_closure(box_files, args.verifier_root / "boxferry")
    auth.copy_closure(lens_files, args.verifier_root / "docker-lens")
    expected = context(pair, digest, args.trusted_sha, args.run_id, args.attempt)
    aggregate(args.artifacts, expected_context=expected, pair=pair, lens=args.lens_root,
              files=lens_files, box_files=box_files)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, UnicodeError, RecursionError, subprocess.SubprocessError):
        raise SystemExit("reviewed Docker consumer evidence rejected") from None
