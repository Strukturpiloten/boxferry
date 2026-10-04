#!/usr/bin/env python3
"""Narrow, test-only Docker application catalogue and request boundary.

This is not a BoxFerry transport. The live runner owns an isolated daemon and
must explicitly opt in before this helper can replay one reviewed core request.
"""

from __future__ import annotations

import argparse
import hashlib
import http.client
import importlib.util
import json
import os
import pathlib
import re
import resource
import select
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import time
from typing import Any


CORE_SPEC = importlib.util.spec_from_file_location(
    "docker_core_artifact", pathlib.Path(__file__).with_name("docker-core-artifact.py"))
assert CORE_SPEC is not None and CORE_SPEC.loader is not None
core_artifact = importlib.util.module_from_spec(CORE_SPEC)
CORE_SPEC.loader.exec_module(core_artifact)
REPOSITORY_ROOT = pathlib.Path(__file__).resolve().parents[2]


IMAGE_KEYS = {
    "debian11-rootful": "DEBIAN_ROOTFUL_IMAGE",
    "debian11-rootless": "DEBIAN_ROOTLESS_IMAGE",
    "upstream-rootful": "UPSTREAM_ROOTFUL_IMAGE",
    "upstream-rootless": "UPSTREAM_ROOTLESS_IMAGE",
    "fixture": "FIXTURE_IMAGE",
}
IMAGE_PATTERN = re.compile(r"^[a-z0-9.-]+(?:/[a-z0-9._-]+)+:[A-Za-z0-9._-]+@sha256:[0-9a-f]{64}$")
SHA_PATTERN = re.compile(r"^[0-9a-f]{40}$")
DIGEST_PATTERN = re.compile(r"^[0-9a-f]{64}$")
API_PATTERN = re.compile(r"^1\.(?:[4-9][0-9])$")
CONTAINER_NAME = "bf-docker-core"
REACQUIRED_SERVICE = "docker-service-1"
CORE_IMAGE_ALIAS_PATTERN = re.compile(r"^registry\.invalid/boxferry-core/busybox:[A-Za-z0-9._-]{1,128}$")
# Historical expected native image config, not an operational image pin. The
# canonical pull remains DockerLens's FIXTURE_IMAGE assignment; a changed pin
# requires fresh independent review before this baseline can accept it.
CORE_BUSYBOX_FIXTURE = (
    "docker.io/library/busybox:1.37.0@sha256:"
    "bdf57e528e45e4433820e045b29b4597825a1c9e38353532d90a01445013f82e"
)
CORE_BUSYBOX_ENV = ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"]
CORE_COMMAND = [
    "sh",
    "-c",
    "printf boxferry-core-ready > /tmp/boxferry-core-marker; exec sleep 3600",
]
CORE_PRESETS = {
    "debian11-rootful": "debian11-20.10.5-rootful",
    "debian11-rootless": "debian11-20.10.5-rootless",
    "upstream-rootful": "upstream-29.8.1-rootful",
    "upstream-rootless": "upstream-29.8.1-rootless",
}
# No Docker-to-Compose lane is admitted until every native default field and
# resulting per-field decision has an independently reviewed exact expectation.
CORE_REACQUIRE_REPORT_EXPECTATIONS: dict[str, dict[str, Any]] = {}

# Independent authored fixture bindings, not values learned from producer output.
VOLUME_TOKEN = re.compile(r"^[a-z][a-z0-9-]{0,63}$")
VOLUME_FIXTURES = (
    ("forgejo", "forgejo", "2d7e81eeefc7060812900791db0a3a9fef08b748779f8697fef12f0cced4d5be",
     ("forge-db", "forge-data")),
    ("nextcloud", "cloud", "ca714ddfe9b64620faf47c714db6be2907f7ec6428529c3e041bfd123bdd4761",
     ("cloud-database", "cloud-redis", "cloud-nextcloud")),
    ("paperless-ngx", "paperless", "f22f0e4194db3b907cdb0845045339f8561ad830407acf66f6f1063fcca6f762",
     ("paper-data", "paper-media", "paper-consume", "paper-export", "paper-pgdata", "paper-redisdata")),
    ("immich", "immich", "a1ca57ad8d5342aafa2e947c9ed657b6006e89288c97d1d12166ddc86b55755d",
     ("immich-library", "immich-model-cache", "immich-pgdata", "immich-redisdata")),
    ("observability", "observability", "d66a018d9c3cfe804ab594ad6a1bcbe89e366ac1780422efa90ef3fca3143cbc",
     ("observability-alloy-data", "observability-grafana-data", "observability-loki-data",
      "observability-prometheus-data", "observability-telemetry-logs")),
    ("supabase", "supabase", "9dd2e33343e8cf00836d783fb25d6488dcc0b958f68472dafa4b6d7b24c8a42b",
     ("supabase-deno-cache", "supabase-pgdata", "supabase-storage")),
)


def candidate_source_digest(root: pathlib.Path) -> str:
    """Hash actual candidate bytes, including uncommitted and untracked source."""
    require(root.is_absolute() and root.is_dir() and not root.is_symlink(),
            "BoxFerry source root must be an absolute real directory")
    require(pathlib.Path(git(root, "rev-parse", "--show-toplevel")) == root,
            "BoxFerry source root differs from checkout")
    result = subprocess.run(
        ["git", "-c", f"safe.directory={root}", "-c", "core.fsmonitor=false",
         "-C", str(root), "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        capture_output=True, check=False, timeout=15,
        env={**os.environ, "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
             "GIT_OPTIONAL_LOCKS": "0"},
    )
    require(result.returncode == 0 and len(result.stdout) <= 1_048_576,
            "BoxFerry source inventory cannot be bounded")
    names = sorted(set(result.stdout.split(b"\0")[:-1]))
    require(0 < len(names) <= 10_000, "BoxFerry source inventory is empty or too large")
    digest = hashlib.sha256()
    total = 0
    for name in names:
        require(name and b"\n" not in name and not name.startswith(b"/") and b".." not in name.split(b"/"),
                "BoxFerry source path is unsafe")
        path = root / os.fsdecode(name)
        raw = bounded_regular_bytes(path, 4 * 1024 * 1024)
        total += len(raw)
        require(total <= 128 * 1024 * 1024, "BoxFerry source content exceeds digest budget")
        digest.update(len(name).to_bytes(4, "big") + name)
        digest.update(len(raw).to_bytes(8, "big") + raw)
    return digest.hexdigest()


def expected_build_command(lens_root: pathlib.Path, profile: str = "core-journey") -> list[str]:
    require(profile in {"core-journey", "volume-fixtures"}, "candidate profile is unreviewed")
    if profile == "volume-fixtures":
        return ["cargo", "build", "--locked", "--package", "boxferry", "--example",
                "docker-volume-fixture-rehearsal", "--no-default-features", "--features", "compose,docker",
                "--jobs", "2", "--config", f'patch.crates-io.docker-lens.path="{lens_root}"']
    return ["cargo", "build", "--locked", "--package", "boxferry", "--bin", "boxferry",
            "--jobs", "2", "--config", f'patch.crates-io.docker-lens.path="{lens_root}"']


def verify_candidate(root: pathlib.Path, binary: pathlib.Path, receipt: pathlib.Path,
                     lens_root: pathlib.Path, lens_revision: str, *, profile: str = "core-journey") -> dict[str, Any]:
    require(root.is_absolute() and lens_root.is_absolute()
            and root.resolve(strict=True) == root and lens_root.resolve(strict=True) == lens_root
            and re.fullmatch(r"[A-Za-z0-9_./-]+", str(lens_root)) is not None,
            "candidate source roots must be canonical, path-safe checkout directories")
    require(bool(SHA_PATTERN.fullmatch(lens_revision))
            and git(lens_root, "rev-parse", "HEAD") == lens_revision
            and git(lens_root, "status", "--porcelain", "--untracked-files=all") == "",
            "DockerLens source checkout must be the selected clean revision")
    require(binary.is_absolute() and binary.resolve(strict=True) == binary
            and binary.is_relative_to(root / "target")
            and not binary.is_symlink() and binary.is_file(),
            "BoxFerry binary must be a regular worktree-local target file")
    require(root.is_absolute() and not root.is_symlink()
            and all(not parent.is_symlink() for parent in binary.parents if parent.is_relative_to(root)),
            "BoxFerry binary path must have no symlink parent")
    descriptor = os.open(binary, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as source:
        metadata = os.fstat(source.fileno())
        require(stat.S_ISREG(metadata.st_mode) and bool(metadata.st_mode & stat.S_IXUSR)
                and 0 < metadata.st_size <= 512 * 1024 * 1024,
                "BoxFerry binary is not a bounded executable regular file")
        binary_digest = hashlib.sha256()
        while chunk := source.read(1024 * 1024):
            binary_digest.update(chunk)
        require(source.tell() == metadata.st_size, "BoxFerry binary changed during hashing")
    record = json.loads(read_private_artifact(receipt), object_pairs_hook=no_duplicate_keys)
    require(isinstance(record, dict) and set(record) == {
            "schema_version", "boxferry_revision", "boxferry_source_sha256", "boxferry_lock_sha256",
            "docker_lens_revision", "docker_lens_source_sha256", "docker_lens_lock_sha256",
            "build_command", "override_identity", "binary_sha256",
            } and type(record["schema_version"]) is int and record["schema_version"] == 2,
            "BoxFerry candidate receipt has wrong shape")
    require(isinstance(record["boxferry_revision"], str) and SHA_PATTERN.fullmatch(record["boxferry_revision"])
            and record["boxferry_revision"] == git(root, "rev-parse", "HEAD"),
            "BoxFerry build revision differs from checkout")
    require(isinstance(record["boxferry_source_sha256"], str)
            and DIGEST_PATTERN.fullmatch(record["boxferry_source_sha256"])
            and record["boxferry_source_sha256"] == candidate_source_digest(root),
            "BoxFerry candidate source content differs from receipt")
    require(isinstance(record["docker_lens_revision"], str)
            and record["docker_lens_revision"] == lens_revision
            and isinstance(record["docker_lens_source_sha256"], str)
            and DIGEST_PATTERN.fullmatch(record["docker_lens_source_sha256"])
            and record["docker_lens_source_sha256"] == candidate_source_digest(lens_root),
            "DockerLens source content differs from receipt")
    for key, checkout in (("boxferry_lock_sha256", root), ("docker_lens_lock_sha256", lens_root)):
        value = record[key]
        require(isinstance(value, str) and DIGEST_PATTERN.fullmatch(value)
                and value == hashlib.sha256(bounded_regular_bytes(checkout / "Cargo.lock", 1024 * 1024)).hexdigest(),
                "candidate lockfile differs from receipt")
    require(record["build_command"] == expected_build_command(lens_root, profile)
            and record["override_identity"] == {
                "kind": "cargo-crates-io-patch", "package": "docker-lens", "path": str(lens_root),
                "revision": lens_revision, "source_sha256": record["docker_lens_source_sha256"],
            }, "candidate build recipe or DockerLens override identity differs")
    require(isinstance(record["binary_sha256"], str) and DIGEST_PATTERN.fullmatch(record["binary_sha256"])
            and record["binary_sha256"] == binary_digest.hexdigest(),
            "BoxFerry binary differs from candidate receipt")
    return record


def capture_candidate(root: pathlib.Path, binary: pathlib.Path, receipt: pathlib.Path,
                      lens_root: pathlib.Path, lens_revision: str,
                      destination: pathlib.Path, *, profile: str = "core-journey") -> dict[str, Any]:
    """Execute only an owner-private snapshot of the attested input bytes."""
    record = verify_candidate(root, binary, receipt, lens_root, lens_revision, profile=profile)
    require(destination.is_absolute() and destination.name == "boxferry-candidate"
            and not destination.exists() and destination.parent.is_dir()
            and not destination.parent.is_symlink()
            and destination.parent.stat().st_uid == os.geteuid()
            and stat.S_IMODE(destination.parent.stat().st_mode) == 0o700,
            "candidate snapshot destination must be new in a private run directory")
    source_fd = os.open(binary, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        before = os.fstat(source_fd)
        require(stat.S_ISREG(before.st_mode) and bool(before.st_mode & stat.S_IXUSR)
                and 0 < before.st_size <= 512 * 1024 * 1024,
                "candidate input is not a bounded regular binary")
        target_fd = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        try:
            digest = hashlib.sha256()
            copied = 0
            while chunk := os.read(source_fd, 1024 * 1024):
                copied += len(chunk)
                require(copied <= before.st_size, "candidate input changed while copying")
                digest.update(chunk)
                remaining = memoryview(chunk)
                while remaining:
                    remaining = remaining[os.write(target_fd, remaining):]
            after = os.fstat(source_fd)
            require(copied == before.st_size and (after.st_dev, after.st_ino, after.st_size,
                    after.st_mtime_ns, after.st_ctime_ns) == (before.st_dev, before.st_ino,
                    before.st_size, before.st_mtime_ns, before.st_ctime_ns),
                    "candidate input changed while copying")
            require(digest.hexdigest() == record["binary_sha256"],
                    "candidate snapshot differs from attested binary")
            os.fsync(target_fd)
            os.fchmod(target_fd, 0o500)
            target = os.fstat(target_fd)
            require(stat.S_ISREG(target.st_mode) and target.st_size == copied
                    and target.st_uid == os.geteuid() and stat.S_IMODE(target.st_mode) == 0o500,
                    "candidate snapshot is not owner-only executable")
        finally:
            os.close(target_fd)
    finally:
        os.close(source_fd)
    readback_fd = os.open(destination, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(readback_fd, "rb") as snapshot:
        readback = hashlib.sha256()
        while chunk := snapshot.read(1024 * 1024):
            readback.update(chunk)
    require(readback.hexdigest() == record["binary_sha256"],
            "candidate snapshot readback differs")
    verify_candidate(root, binary, receipt, lens_root, lens_revision, profile=profile)
    return record


def core_prerequisite_inputs(template: pathlib.Path | None = None) -> tuple[bytes, bytes, dict[str, Any]]:
    """Reuse authored source bindings; neither their hashes nor this check grant replay authority."""
    source_bytes = bounded_regular_bytes(template if template is not None else
                                         REPOSITORY_ROOT / core_artifact.SOURCE_PATH, 4096)
    expectations_bytes = bounded_regular_bytes(REPOSITORY_ROOT / core_artifact.EXPECTATIONS_PATH, 16_384)
    try:
        expected = core_artifact.check_source(source_bytes, expectations_bytes)
    except core_artifact.topology.ExpectationError as error:
        raise ContractError(str(error)) from error
    return source_bytes, expectations_bytes, expected


def render_core_source(template: pathlib.Path, destination: pathlib.Path, image: str) -> None:
    require(bool(CORE_IMAGE_ALIAS_PATTERN.fullmatch(image)), "core image alias is not allowlisted")
    source_bytes, _, expected = core_prerequisite_inputs(template)
    raw = source_bytes.decode("utf-8")
    placeholder = expected["image_placeholder"]
    require(raw.count(placeholder) == 1 and "${" not in raw,
            "core source differs from the closed authored template")
    require(destination.is_absolute() and not destination.exists() and destination.parent.is_dir()
            and not destination.parent.is_symlink()
            and stat.S_IMODE(destination.parent.stat().st_mode) == 0o700,
            "core source destination must be new in private directory")
    descriptor = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as output:
        output.write(raw.replace(placeholder, image))


def core_generated_compose(raw: bytes, image: str) -> None:
    """Closed check of reviewed image, environment, and external-bridge intent."""
    require(0 < len(raw) <= 16_384, "reacquired Compose document exceeds core limit")
    lines = raw.decode("utf-8").splitlines()
    require(lines[:4] == ["---", f"name: {CONTAINER_NAME}", "services:", f"  {REACQUIRED_SERVICE}:"],
            "reacquired Compose root or service identity differs")
    require(lines.count("networks:") == 1, "reacquired Compose network root is missing or repeats")
    network_root = lines.index("networks:")
    require(network_root > 4, "reacquired Compose network root is misplaced")
    def scalar(value: str) -> str:
        if value.startswith('"'):
            parsed = json.loads(value)
            require(isinstance(parsed, str), "reacquired Compose scalar is not a string")
            return parsed
        require(bool(re.fullmatch(r"[A-Za-z0-9_./:@; =-]+", value)),
                "reacquired Compose scalar is outside checked subset")
        return value
    fields: dict[str, Any] = {}
    index = 4
    while index < network_root:
        line = lines[index]
        if line in ("    command:", "    environment:"):
            key = line.strip().removesuffix(":")
            require(key not in fields, "reacquired Compose list field repeats")
            values = []
            index += 1
            while index < network_root and lines[index].startswith("      - "):
                values.append(scalar(lines[index][8:]))
                index += 1
            fields[key] = values
            continue
        if line == "    networks:":
            require("networks" not in fields and index + 1 < network_root
                    and lines[index + 1] == "      docker-network-1: {}",
                    "reacquired Compose bridge attachment differs")
            fields["networks"] = ["docker-network-1"]
            index += 2
            continue
        match = re.fullmatch(r"    (image|container_name): (.+)", line)
        require(match is not None, "reacquired Compose has an unreviewed root or service field")
        key, value = match.groups()
        require(key not in fields, "reacquired Compose field repeats")
        fields[key] = scalar(value)
        index += 1
    require(fields == {"image": image, "container_name": CONTAINER_NAME,
                       "command": CORE_COMMAND, "environment": CORE_BUSYBOX_ENV,
                       "networks": ["docker-network-1"]},
            "reacquired Compose core semantics differ")
    require(lines[network_root + 1:network_root + 2] == ["  docker-network-1:"],
            "reacquired Compose external bridge identity differs")
    network_fields: dict[str, str] = {}
    for line in lines[network_root + 2:]:
        match = re.fullmatch(r"    (name|external): (.+)", line)
        require(match is not None, "reacquired Compose network has an unreviewed field")
        key, value = match.groups()
        require(key not in network_fields, "reacquired Compose network field repeats")
        network_fields[key] = scalar(value) if key == "name" else value
    require(network_fields == {"name": "bridge", "external": "true"},
            "reacquired Compose external bridge semantics differ")


def core_report(raw: bytes, source: str, target: str, lane: str,
                profiles: dict[str, dict[str, Any]]) -> None:
    require(0 < len(raw) <= 65_536, "BoxFerry CLI report exceeds core limit")
    report = json.loads(raw, object_pairs_hook=no_duplicate_keys)
    require(isinstance(report, dict) and report.get("schema_version") == 1
            and report.get("status") == "success" and report.get("exit_category") == "success"
            and report.get("failed_stage") is None and report.get("failure_summary") is None
            and report.get("fix_first") is None
            and report.get("source_type") == source and report.get("target_type") == target,
            "BoxFerry CLI did not close expected conversion")
    require(report.get("primary_diagnostic_code") in (None, ""),
            "BoxFerry CLI reported primary diagnostic")
    require(report.get("application") == CONTAINER_NAME
            and report.get("truncations") == []
            and isinstance(report.get("invocation"), dict)
            and report["invocation"].get("command_kind") == "convert",
            "BoxFerry CLI application or invocation differs")
    require(lane in CORE_PRESETS and lane in profiles,
            "core report has no independently selected exact native lane")
    choice_list = report.get("choices")
    require(isinstance(choice_list, list) and all(isinstance(choice, dict)
            and set(choice) == {"name", "value"} for choice in choice_list),
            "BoxFerry CLI choices are malformed")
    choices = {choice["name"]: choice["value"] for choice in choice_list}
    require(len(choices) == len(choice_list)
            and choices.get("loss_policy") == ("exact" if target == "docker" else "partial")
            and choices.get("environment_values") == ("withhold" if target == "docker" else "include"),
            "BoxFerry CLI policy choices differ")
    fidelity = report.get("fidelity")
    require(isinstance(fidelity, dict)
            and set(fidelity) == {"exact", "approximate", "unsupported", "invalid", "other"}
            and all(type(value) is int and 0 <= value <= 64 for value in fidelity.values())
            and fidelity["exact"] > 0 and fidelity["invalid"] == 0 and fidelity["other"] == 0,
            "BoxFerry CLI fidelity envelope exceeds core limits")
    diagnostics = report.get("diagnostics")
    require(isinstance(diagnostics, list), "BoxFerry CLI diagnostics are malformed")
    if target == "docker":
        profile = profiles[lane]
        requested = {"minimum": CORE_PRESETS[lane], "maximum": CORE_PRESETS[lane]}
        resolved = {"minimum": profile["engine_release"], "maximum": profile["engine_release"]}
        build = profile["build"]
        expected_choices = {
            "docker_target": CORE_PRESETS[lane],
            "docker_target_build": "upstream" if build["kind"] == "upstream"
            else f'debian-package:{build["revision"]}',
            "docker_target_engine_release": profile["engine_release"],
            "docker_target_advertised_api": profile["advertised_api_version"],
            "docker_target_acquisition_api": profile["acquisition_api_version"],
            "docker_target_rendering_api": profile["rendering_api_version"],
            "docker_target_mode": profile["daemon_mode"],
            "docker_target_evidence_sha256": profile["evidence_sha256"],
        }
        require(all(choices.get(key) == value for key, value in expected_choices.items())
                and fidelity["approximate"] == 0 and fidelity["unsupported"] == 0
                and diagnostics == [],
                "Compose-to-Docker plan contains unreviewed profile or loss")
    else:
        requested = resolved = {"minimum": "rolling", "maximum": "rolling"}
        require(choices.get("docker_acquisition") == "read-only-local-unix"
                and choices.get("docker_import_policy") == "portable"
                and choices.get("promote_docker_protected_environment_values") == "true"
                and choices.get("promote_docker_same_host_bind_mounts") == "false",
                "Docker source promotion or acquisition choices differ")
        require(CORE_BUSYBOX_ENV[0].encode() not in raw,
                "Docker source report exposed the protected environment value")
        expected = CORE_REACQUIRE_REPORT_EXPECTATIONS.get(lane)
        require(expected is not None,
                "exact per-lane Docker-to-Compose losses and diagnostics remain pending independent native review")
        require(fidelity == expected["fidelity"] and diagnostics == expected["diagnostics"],
                "Docker-to-Compose per-field decisions differ from independently reviewed lane")
    require(report.get("requested_versions") == requested
            and report.get("resolved_versions") == resolved,
            "BoxFerry CLI route version bounds differ")
    expected = "docker-plan.json" if target == "docker" else "compose.yaml"
    artifacts = report.get("output_artifacts")
    require(isinstance(artifacts, list) and len(artifacts) == 1
            and isinstance(artifacts[0], dict) and set(artifacts[0]) == {"name", "size"}
            and artifacts[0]["name"] == expected
            and type(artifacts[0]["size"]) is int and 0 < artifacts[0]["size"] <= 16_384,
            "BoxFerry CLI reported unexpected core output artifacts")


def core_inspect(raw: bytes, container_id: str, image: str,
                 fixture_image: str) -> tuple[str, str]:
    require(0 < len(raw) <= 65_536 and re.fullmatch(r"[0-9a-f]{64}", container_id) is not None,
            "native inspect exceeds core boundary")
    reviewed_tag = CORE_BUSYBOX_FIXTURE.split("@", 1)[0].rsplit(":", 1)[1]
    require(fixture_image == CORE_BUSYBOX_FIXTURE
            and image == f"registry.invalid/boxferry-core/busybox:{reviewed_tag}",
            "BusyBox fixture pin has no independently reviewed environment baseline")
    value = json.loads(raw, object_pairs_hook=no_duplicate_keys)
    require(isinstance(value, dict) and value.get("Id") == container_id
            and value.get("Name") == f"/{CONTAINER_NAME}",
            "native inspect identity differs")
    config = value.get("Config")
    require(isinstance(config, dict) and config.get("Image") == image
            and config.get("Cmd") == CORE_COMMAND and config.get("Env") == CORE_BUSYBOX_ENV,
            "native inspect image, command, or inherited environment differs")
    require(config.get("ExposedPorts") in (None, {}) and config.get("Volumes") in (None, {}),
            "native container declares unexpected ports or volumes")
    host = value.get("HostConfig")
    require(isinstance(host, dict) and host.get("NetworkMode") in ("default", "bridge")
            and host.get("Privileged") is False and host.get("ReadonlyRootfs") is False,
            "native host network or privilege settings differ")
    for field in ("Binds", "Mounts", "VolumesFrom", "Devices", "DeviceRequests",
                  "SecurityOpt", "CapAdd", "CapDrop"):
        require(host.get(field) in (None, []), f"native host {field} is not empty")
    require(host.get("Tmpfs") in (None, {}) and host.get("PortBindings") in (None, {}),
            "native host mount or port bindings are not empty")
    require(value.get("Mounts") == [], "native container has unexpected mounts")
    network_settings = value.get("NetworkSettings")
    require(isinstance(network_settings, dict) and network_settings.get("Ports") in (None, {}),
            "native container has unexpected published ports")
    networks = network_settings.get("Networks")
    require(isinstance(networks, dict) and set(networks) == {"bridge"},
            "native container is not solely attached to the default bridge")
    endpoint = networks["bridge"]
    require(isinstance(endpoint, dict), "native bridge endpoint is missing")
    require(endpoint.get("Aliases") in (None, [])
            and endpoint.get("IPAMConfig") in (None, {})
            and endpoint.get("Links") in (None, []),
            "native bridge endpoint has unreviewed alias, static address, or link intent")
    network_id = endpoint.get("NetworkID")
    require(isinstance(network_id, str) and re.fullmatch(r"[0-9a-f]{64}", network_id) is not None,
            "native bridge network ID is not canonical")
    require(isinstance(endpoint.get("EndpointID"), str)
            and re.fullmatch(r"[0-9a-f]{64}", endpoint["EndpointID"]) is not None,
            "native bridge endpoint ID is not canonical")
    return network_id, endpoint["EndpointID"]


def core_network_inspect(raw: bytes, network_id: str, container_id: str,
                         endpoint_id: str) -> None:
    require(0 < len(raw) <= 65_536 and re.fullmatch(r"[0-9a-f]{64}", network_id) is not None
            and re.fullmatch(r"[0-9a-f]{64}", container_id) is not None,
            "native network inspect exceeds core boundary")
    value = json.loads(raw, object_pairs_hook=no_duplicate_keys)
    require(isinstance(value, dict) and value.get("Id") == network_id
            and value.get("Name") == "bridge" and value.get("Driver") == "bridge"
            and value.get("Internal") is False,
            "native default bridge identity or topology differs")
    members = value.get("Containers")
    require(isinstance(members, dict) and set(members) == {container_id},
            "native default bridge membership differs")
    member = members[container_id]
    require(isinstance(member, dict) and member.get("Name") == CONTAINER_NAME,
            "native default bridge member identity differs")
    require(member.get("EndpointID") == endpoint_id,
            "native default bridge endpoint ID differs")


def core_reacquired_privacy(raw: bytes, container_file: pathlib.Path,
                            network_file: pathlib.Path, container_id: str,
                            network_id: str, image: str, fixture_image: str,
                            socket_path: str) -> None:
    require(len(raw) <= 65_536, "reacquired report or console exceeds privacy scan limit")
    require(container_file.name == "native-inspect.json"
            and network_file == container_file.with_name("native-network-inspect.json")
            and socket_path == str(container_file.parent / "socket/docker.sock"),
            "reacquired privacy evidence does not bind the selected native resources")
    container_raw = core_output_file(container_file, 65_536)
    network_raw = core_output_file(network_file, 65_536)
    selected_network_id, endpoint_id = core_inspect(container_raw, container_id,
                                                     image, fixture_image)
    require(selected_network_id == network_id,
            "reacquired privacy network selector differs from native endpoint")
    core_network_inspect(network_raw, network_id, container_id, endpoint_id)
    protected = (CORE_BUSYBOX_ENV[0], CORE_BUSYBOX_ENV[0].partition("=")[2],
                 container_id, network_id, endpoint_id, socket_path)
    require(all(value.encode() not in raw for value in protected),
            "reacquired report or console exposed a protected native value")
    # Console diagnostics may prefix a quoted JSON value with ordinary text.
    # Scan only bounded string literals, never evaluate or interpret that text.
    decoded_text = raw.decode("utf-8")
    decoder = json.JSONDecoder()
    position = 0
    while (start := decoded_text.find('"', position)) != -1:
        try:
            literal, position = decoder.raw_decode(decoded_text, start)
        except ValueError as error:
            raise ContractError("reacquired report or console has a malformed quoted fragment") from error
        require(isinstance(literal, str)
                and all(protected_value not in literal for protected_value in protected),
                "reacquired report or console exposed a protected native value")


def core_output_file(path: pathlib.Path, limit: int, expected_entries: set[str] | None = None,
                     *, allow_empty: bool = False) -> bytes:
    require(path.is_absolute() and not path.is_symlink() and path.is_file()
            and stat.S_IMODE(path.stat().st_mode) == 0o600,
            "core output must be a private regular file")
    parent = path.parent
    require(parent.is_dir() and not parent.is_symlink()
            and stat.S_IMODE(parent.stat().st_mode) == 0o700,
            "core output parent must be private")
    if expected_entries is not None:
        require({entry.name for entry in parent.iterdir()} == expected_entries,
                "core conversion output contains unexpected files")
    return bounded_regular_bytes(path, limit, allow_empty=allow_empty)


def bounded_measurement(command: list[str], temporary_root: pathlib.Path) -> tuple[int, bytes, bytes]:
    with tempfile.TemporaryFile(dir=temporary_root) as output, tempfile.TemporaryFile(dir=temporary_root) as errors:
        def limit_file() -> None:
            resource.setrlimit(resource.RLIMIT_FSIZE, (8192, 8192))
        result = subprocess.run(command, stdout=output, stderr=errors, check=False, timeout=10,
                                env={**os.environ, "LC_ALL": "C"}, preexec_fn=limit_file)
        output.seek(0)
        errors.seek(0)
        return result.returncode, output.read(8193), errors.read(8193)


def transient_owned_du_error(root: pathlib.Path, raw: bytes) -> bool:
    if not 0 < len(raw) <= 8192 or not root.is_dir() or root.is_symlink():
        return False
    for line in raw.decode("utf-8", "strict").splitlines():
        match = re.fullmatch(r"du: cannot access '([^'\n]+)': No such file or directory", line)
        if not match or not match.group(1).startswith(f"{root}/"):
            return False
        descendant = match.group(1)[len(str(root)) + 1:]
        if not descendant or any(part in ("", ".", "..") for part in descendant.split("/")):
            return False
    return True


def sample_owned_storage(volume: pathlib.Path, run_dir: pathlib.Path, graph: pathlib.Path,
                         baseline_free: int) -> dict[str, int]:
    require(baseline_free >= 8 * 1024 * 1024, "storage baseline is insufficient")
    for root in (volume, run_dir, graph):
        require(root.is_absolute() and root != pathlib.Path("/") and root.is_dir() and not root.is_symlink(),
                "storage measurement root is invalid")
    minimum_free = baseline_free
    for attempt in range(3):
        available = []
        for root in (graph, run_dir):
            status, output, errors = bounded_measurement(["df", "-Pk", "--", str(root)], run_dir)
            require(status == 0 and not errors and len(output) <= 8192,
                    "storage free-space measurement failed")
            lines = output.decode("ascii").splitlines()
            require(bool(lines), "storage free-space value is missing")
            fields = lines[-1].split()
            require(len(fields) >= 4 and fields[3].isdigit(), "storage free-space value is malformed")
            available.append(int(fields[3]))
        free, temporary_free = available
        minimum_free = min(minimum_free, free)
        require(free >= 2 * 1024 * 1024 and temporary_free >= 1024 * 1024
                and baseline_free - minimum_free <= 6 * 1024 * 1024,
                "storage free-space or growth budget exceeded")
        used = []
        transient = False
        for root in (volume, run_dir):
            require(root.is_dir() and not root.is_symlink(), "run-owned measurement root disappeared")
            status, output, errors = bounded_measurement(["du", "-sk", "--", str(root)], run_dir)
            if status != 0:
                require(transient_owned_du_error(root, errors), "storage usage measurement failed")
                transient = True
                break
            require(not errors and len(output) <= 8192,
                    "storage usage measurement emitted unexpected output")
            match = re.fullmatch(rb"([0-9]+)\t" + re.escape(os.fsencode(root)) + rb"\n?", output)
            require(match is not None, "storage usage value is malformed")
            used.append(int(match.group(1)))
        if not transient:
            require(used[0] <= 4 * 1024 * 1024 and used[1] <= 512 * 1024,
                    "run-owned storage usage budget exceeded")
            return {"used": used[0], "temporary_used": used[1],
                    "free": free, "temporary_free": temporary_free, "minimum_free": minimum_free}
        if attempt < 2:
            time.sleep(0.1)
    raise ContractError("run-owned storage remained in transient churn")


class ContractError(Exception):
    """A value cannot cross the test-only safety boundary."""


class ParentGone(ContractError):
    """The original process exited or its PID was recycled."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def process_start(pid: int) -> int:
    require(pid > 0, "parent PID must be positive")
    stat_line = pathlib.Path(f"/proc/{pid}/stat").read_text(encoding="ascii")
    closing = stat_line.rfind(")")
    require(closing >= 0, "parent process identity is unavailable")
    fields = stat_line[closing + 2:].split()
    require(len(fields) >= 20 and fields[19].isdigit(), "parent process identity is malformed")
    return int(fields[19])


def verified_parent_fd(pid: int, start: int) -> int:
    require(start > 0, "parent start time must be positive")
    try:
        fd = os.pidfd_open(pid, 0)
    except ProcessLookupError as error:
        raise ParentGone("parent process has exited") from error
    try:
        try:
            observed_start = process_start(pid)
        except FileNotFoundError as error:
            raise ParentGone("parent process has exited") from error
        if observed_start != start:
            raise ParentGone("parent process identity changed")
        return fd
    except BaseException:
        os.close(fd)
        raise


def parent_alive(fd: int) -> bool:
    monitor = select.poll()
    monitor.register(fd, select.POLLIN)
    return not monitor.poll(0)


def signal_parent(pid: int, start: int) -> None:
    fd = verified_parent_fd(pid, start)
    try:
        if not parent_alive(fd):
            raise ParentGone("parent process has exited")
        try:
            signal.pidfd_send_signal(fd, signal.SIGTERM)
        except ProcessLookupError as error:
            raise ParentGone("parent process has exited") from error
    finally:
        os.close(fd)


def guard_parent(pid: int, start: int, execution_seconds: int, cleanup_seconds: int,
                 ready_file: str | None = None) -> None:
    require(0 < execution_seconds <= 780 and 0 < cleanup_seconds <= 120,
            "parent execution budget is invalid")
    fd = verified_parent_fd(pid, start)
    try:
        monitor = select.poll()
        monitor.register(fd, select.POLLIN)
        if not parent_alive(fd):
            raise ParentGone("parent process has exited")
        if ready_file is not None:
            ready_path = pathlib.Path(ready_file)
            require(ready_path.is_absolute(), "guard readiness path must be absolute")
            marker = os.open(ready_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
            try:
                identity = f"{os.getpid()}:{process_start(os.getpid())}"
                os.write(marker, identity.encode("ascii"))
            finally:
                os.close(marker)
        if monitor.poll(execution_seconds * 1000):
            return
        print("Docker replay probe exceeded its 13-minute execution budget; cleanup has two minutes",
              file=sys.stderr)
        try:
            signal.pidfd_send_signal(fd, signal.SIGTERM)
        except ProcessLookupError:
            return
        if monitor.poll(cleanup_seconds * 1000):
            return
        try:
            signal.pidfd_send_signal(fd, signal.SIGKILL)
        except ProcessLookupError:
            pass
    finally:
        os.close(fd)


def git(root: pathlib.Path, *arguments: str) -> str:
    environment = dict(os.environ)
    environment["GIT_CONFIG_NOSYSTEM"] = "1"
    environment["GIT_CONFIG_GLOBAL"] = os.devnull
    environment["GIT_OPTIONAL_LOCKS"] = "0"
    result = subprocess.run(
        ["git", "-c", f"safe.directory={root}", "-c", "core.fsmonitor=false",
         "-c", f"core.hooksPath={os.devnull}", "-c", "protocol.file.allow=never",
         "-C", str(root), *arguments],
        check=False,
        capture_output=True,
        text=True,
        timeout=15,
        env=environment,
    )
    require(result.returncode == 0, "DockerLens checkout cannot be verified")
    return result.stdout.strip()


def canonical_images(root_text: str, revision: str, script_digest: str) -> dict[str, str]:
    require(bool(SHA_PATTERN.fullmatch(revision)), "DockerLens revision must be a full Git SHA")
    require(bool(DIGEST_PATTERN.fullmatch(script_digest)), "native script digest must be SHA-256")
    root = pathlib.Path(root_text)
    require(root.is_absolute() and not root.is_symlink() and root.is_dir(), "DockerLens root must be a real absolute directory")
    root = root.resolve(strict=True)
    require(git(root, "rev-parse", "--show-toplevel") == str(root), "DockerLens root is not the checkout root")
    require(git(root, "rev-parse", "HEAD") == revision, "DockerLens checkout revision differs")
    require(git(root, "status", "--porcelain", "--untracked-files=all") == "", "DockerLens checkout is dirty")
    script = root / "scripts/native-conformance.sh"
    require(not script.is_symlink() and script.is_file(), "canonical native script must be a regular file")
    raw = bounded_regular_bytes(script, 512 * 1024)
    require(hashlib.sha256(raw).hexdigest() == script_digest, "canonical native script digest differs")
    assignments: dict[str, str] = {}
    pattern = re.compile(r"^([A-Z_]+)='([^'\n]+)'$")
    for line in raw.decode("utf-8").splitlines():
        match = pattern.fullmatch(line)
        if match and match.group(1) in IMAGE_KEYS.values():
            key, value = match.groups()
            require(key not in assignments, f"duplicate canonical image assignment: {key}")
            require(bool(IMAGE_PATTERN.fullmatch(value)), f"invalid immutable image reference: {key}")
            assignments[key] = value
    require(set(assignments) == set(IMAGE_KEYS.values()), "canonical image assignments are missing")
    require(len(set(assignments.values())) == len(assignments), "canonical image references repeat")
    return {lane: assignments[key] for lane, key in IMAGE_KEYS.items()}


def no_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        require(key not in value, "request JSON repeats a key")
        value[key] = item
    return value


def bounded_regular_bytes(path: pathlib.Path, limit: int, *, allow_empty: bool = False) -> bytes:
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as source:
        metadata = os.fstat(source.fileno())
        require(stat.S_ISREG(metadata.st_mode) and (allow_empty or metadata.st_size > 0)
                and metadata.st_size <= limit,
                "reviewed source must be a bounded regular file")
        data = source.read(limit + 1)
    require(len(data) == metadata.st_size, "reviewed source changed while reading")
    return data


def reviewed_profiles(root_text: str) -> dict[str, dict[str, Any]]:
    """Read active, digest-bound records only from the already verified checkout."""
    root = pathlib.Path(root_text)
    source = bounded_regular_bytes(root / "src/reviewed_catalog.rs", 512 * 1024).decode("utf-8")
    declarations = re.findall(
        r"(?m)^const RECORDS: \[\(NativeEvidenceLane, &str, &str\); 4\] = \[(.*?)^\];",
        source, re.DOTALL,
    )
    require(len(declarations) == 1, "active reviewed catalogue declaration changed")
    pattern = re.compile(
        r'\(\s*NativeEvidenceLane::(?P<lane>Debian11Rootful|Debian11Rootless|UpstreamRootful|UpstreamRootless),\s*'
        r'"(?P<digest>[0-9a-f]{64})",\s*include_str!\(\s*'
        r'"\.\./docs/evidence/reviewed/sha256/(?P=digest)\.json"\s*\),\s*\),',
    )
    entries = declarations[0]
    require(not pattern.sub("", entries).strip(), "active reviewed catalogue entries changed")
    lanes = {"Debian11Rootful": "debian11-rootful", "Debian11Rootless": "debian11-rootless",
             "UpstreamRootful": "upstream-rootful", "UpstreamRootless": "upstream-rootless"}
    profiles: dict[str, dict[str, Any]] = {}
    for match in pattern.finditer(entries):
        lane = lanes[match["lane"]]
        require(lane not in profiles, "active reviewed profile is duplicated")
        digest = match["digest"]
        raw = bounded_regular_bytes(root / f"docs/evidence/reviewed/sha256/{digest}.json", 128 * 1024)
        require(hashlib.sha256(raw).hexdigest() == digest, "reviewed profile digest differs")
        record = json.loads(raw, object_pairs_hook=no_duplicate_keys)
        require(record.get("schema_version") == 1 and record.get("lane") == lane,
                "reviewed profile lane or schema differs")
        identity = record["identity"]
        build = identity["build"]
        if lane.startswith("debian11-"):
            require(build["kind"] == "debian-package" and build["distribution"] == "debian11"
                    and build["package_name"] == "docker.io", "reviewed Debian build differs")
            artifact_build = {"kind": "debian_package", "revision": build["package_revision"]}
        else:
            require(build == {"kind": "upstream"}, "reviewed upstream build differs")
            artifact_build = {"kind": "upstream"}
        profiles[lane] = {
            "kind": "target", "build": artifact_build,
            "engine_release": identity["engine_release"],
            "advertised_api_version": identity["advertised_api"],
            "acquisition_api_version": identity["acquisition_api"],
            "rendering_api_version": identity["rendering_api"],
            "daemon_mode": identity["mode"], "evidence_sha256": digest,
        }
    require(set(profiles) == set(lanes.values()), "active reviewed profile extraction changed")
    return profiles


def verify_daemon_mode(lane: str, marker: str, uid_report: str) -> None:
    require(lane in IMAGE_KEYS and lane != "fixture", "unknown daemon lane")
    require(marker in ("true", "false"), "rootless marker is not a Boolean")
    match = re.fullmatch(r"1:([0-9]+)\n?", uid_report)
    require(match is not None, "expected exactly one dockerd effective UID")
    rootless = lane.endswith("-rootless")
    require((int(match[1]) != 0) == rootless and (marker == "true") == rootless,
            "daemon process UID and rootless marker disagree with lane")


def verify_storage_mount(mountinfo: bytes, destination: str) -> None:
    """Admit only the historical rootless data-root mount's effective safe flags."""
    require(destination == "/home/docker/.local/share/docker", "unexpected historical storage destination")
    require(0 < len(mountinfo) <= 1_048_576, "mountinfo exceeds the bounded input size")
    lines = mountinfo.decode("utf-8").splitlines()
    require(len(lines) <= 4096, "mountinfo has too many entries")
    matching = []
    for line in lines:
        fields = line.split()
        if len(fields) < 5 or fields[4] != destination:
            continue
        matching.append(fields)
    require(len(matching) == 1, "historical data-root must have exactly one mount entry")
    fields = matching[0]
    require(len(fields) >= 10, "historical data-root mount entry is malformed")
    try:
        separator = fields.index("-", 6)
    except ValueError as error:
        raise ContractError("historical data-root mount separator is missing") from error
    require(len(fields) == separator + 4, "historical data-root mount entry is malformed")
    options = set(fields[5].split(","))
    require("rw" in options and options.isdisjoint({"ro", "nosuid", "nodev"}),
            "historical data-root must be writable with suid and device access")


def verify_docker_root(lane: str, observed: str) -> None:
    require(lane in IMAGE_KEYS and lane != "fixture", "unknown daemon lane")
    expected = "/home/docker/.local/share/docker" if lane.endswith("-rootless") else "/var/lib/docker"
    require(observed == expected, "observed DockerRootDir differs from isolated storage destination")


def verify_outer_mounts(raw: bytes, volume: str, destination: str, socket_dir: str) -> None:
    require(0 < len(raw) <= 16_384, "outer mount inventory exceeds the bounded input size")
    require(destination in ("/var/lib/docker", "/home/docker/.local/share/docker"),
            "unexpected outer storage destination")
    require(bool(re.fullmatch(r"bf-docker-core-data-[A-Za-z0-9]+", volume)),
            "unexpected run-owned storage volume name")
    require(socket_dir.startswith("/tmp/boxferry-docker-core.") and socket_dir.endswith("/socket"),
            "unexpected private socket directory")
    mounts = json.loads(raw, object_pairs_hook=no_duplicate_keys)
    require(isinstance(mounts, list) and len(mounts) == 2, "outer daemon must have exactly two mounts")
    require(all(isinstance(mount, dict) for mount in mounts), "outer mount entries are malformed")
    volume_mounts = [mount for mount in mounts if mount.get("Type") == "volume"
                     and mount.get("Name") == volume and mount.get("Destination") == destination]
    socket_mounts = [mount for mount in mounts if mount.get("Type") == "bind"
                     and mount.get("Source") == socket_dir and mount.get("Destination") == "/boxferry-core"]
    require(len(volume_mounts) == 1 and len(socket_mounts) == 1,
            "outer Docker daemon mount identity or destination differs")


def read_private_artifact(path: pathlib.Path) -> str:
    require(path.is_absolute() and not path.is_symlink() and path.is_file(), "artifact must be an absolute regular file")
    require(stat.S_IMODE(path.stat().st_mode) == 0o600, "artifact must remain mode 0600")
    require(
        path.parent.is_dir() and not path.parent.is_symlink()
        and stat.S_IMODE(path.parent.stat().st_mode) == 0o700,
        "artifact parent must remain mode 0700",
    )
    return bounded_regular_bytes(path, 16_384).decode("utf-8")


def dependency_schedule(
    plan_bytes: bytes,
    sidecar_bytes: bytes | None,
    *,
    profile: dict[str, Any],
    services: dict[str, str],
    expected_edges: list[tuple[str, str, str, bool, bool]],
) -> tuple[list[list[str]], list[dict[str, Any]]]:
    """Validate an inert pair against independent expectations; execute nothing.

    The returned logical-service layers and original, validated decisions are
    inputs to a future isolated application runner. They do not grant permission
    to apply native requests or claim readiness/restart behavior. In particular,
    the caller must not rederive the choreography from the source definition.
    """
    def document(raw: bytes) -> Any:
        require(isinstance(raw, bytes) and 0 < len(raw) <= 1_048_576,
                "dependency document exceeds its bounded size")
        def reject_nonfinite(_value: str) -> None:
            raise ContractError("dependency document contains a non-JSON numeric constant")
        try:
            return json.loads(raw, object_pairs_hook=no_duplicate_keys, parse_constant=reject_nonfinite)
        except (ValueError, UnicodeError, RecursionError) as error:
            raise ContractError("dependency document is not unambiguous JSON") from error

    identity = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}")
    require(isinstance(services, dict) and 0 < len(services) <= 256,
            "independent service inventory is invalid")
    require(all(isinstance(key, str) and identity.fullmatch(key)
                and isinstance(value, str) and identity.fullmatch(value)
                for key, value in services.items()), "independent service identity is invalid")
    require(len(set(services.values())) == len(services), "independent runtime identities repeat")
    require(isinstance(expected_edges, list) and len(expected_edges) <= 4096,
            "independent dependency count is invalid")
    conditions = {"started", "healthy", "completed_successfully"}
    for edge in expected_edges:
        require(isinstance(edge, tuple) and len(edge) == 5
                and isinstance(edge[0], str) and edge[0] in services
                and isinstance(edge[1], str) and edge[1] in services and edge[0] != edge[1]
                and isinstance(edge[2], str) and edge[2] in conditions
                and type(edge[3]) is bool and type(edge[4]) is bool,
                "independent dependency expectation is invalid")
    require(len({edge[:2] for edge in expected_edges}) == len(expected_edges),
            "independent dependency edges repeat")
    plan = document(plan_bytes)
    require(isinstance(plan, dict)
            and set(plan) == {"schema_version", "context", "requests", "prerequisites"}
            and type(plan["schema_version"]) is int and plan["schema_version"] == 1,
            "dependency plan envelope differs")
    require(isinstance(profile, dict) and plan["context"] == profile,
            "dependency plan differs from independently reviewed profile")
    require(plan["prerequisites"] == [], "dependency schedule does not yet admit external prerequisites")
    api = profile.get("rendering_api_version")
    require(isinstance(api, str) and API_PATTERN.fullmatch(api) is not None,
            "dependency plan rendering API is invalid")
    requests = plan["requests"]
    require(isinstance(requests, list) and len(requests) <= 4096,
            "dependency plan request count is invalid")
    create_prefix = f"/v{api}/containers/create?name="
    runtime_names: set[str] = set()
    for request in requests:
        require(isinstance(request, dict) and set(request) == {"method", "path", "body"}
                and isinstance(request["path"], str) and request["method"] == "POST"
                and isinstance(request["body"], dict), "dependency native request envelope differs")
        path = request["path"]
        # This first pure checkpoint admits container creates only. Network,
        # volume and attachment requests need their own independent expected
        # inventory before a multi-resource runner can use this validator.
        require(path.startswith(create_prefix), "dependency container create path differs")
        name = path.removeprefix(create_prefix)
        require(identity.fullmatch(name) is not None and name not in runtime_names,
                "dependency native container identity is invalid or repeated")
        runtime_names.add(name)
    require(runtime_names == set(services.values()),
            "dependency native targets differ from independent service inventory")
    if sidecar_bytes is None:
        require(not expected_edges, "required dependency sidecar is missing")
        return [sorted(services)], []
    sidecar = document(sidecar_bytes)
    require(isinstance(sidecar, dict)
            and set(sidecar) == {"schema_version", "kind", "docker_plan_sha256", "native_execution", "decisions"}
            and type(sidecar["schema_version"]) is int and sidecar["schema_version"] == 1
            and sidecar["kind"] == "boxferry-docker-dependency-decisions"
            and sidecar["native_execution"] is False, "dependency sidecar envelope differs")
    require(sidecar["docker_plan_sha256"] == hashlib.sha256(plan_bytes).hexdigest(),
            "dependency sidecar is not bound to exact native plan bytes")
    decisions = sidecar["decisions"]
    require(isinstance(decisions, list) and 0 < len(decisions) <= 4096,
            "dependency decisions are empty or exceed the bounded count")
    fields = {"service", "service_runtime_name", "dependency", "dependency_runtime_name",
              "condition", "condition_explicit", "required", "required_explicit", "restart",
              "restart_explicit", "fidelity", "native_engine_field", "provenance"}
    provenance_fields = {"reference", "condition", "required", "restart"}
    categories = {"source_document", "runtime_observation", "user_override",
                  "implementation_default", "conversion_decision"}
    edges: set[tuple[str, str, str, bool, bool]] = set()
    dependencies: dict[str, set[str]] = {service: set() for service in services}
    for decision in decisions:
        require(isinstance(decision, dict) and set(decision) == fields,
                "dependency decision fields differ")
        source, target = decision["service"], decision["dependency"]
        require(isinstance(source, str) and source in services
                and isinstance(target, str) and target in services and source != target,
                "dependency decision contains an unknown or self reference")
        require(decision["service_runtime_name"] == services[source]
                and decision["dependency_runtime_name"] == services[target],
                "dependency runtime reference differs from native target")
        require(target not in dependencies[source], "dependency edges repeat")
        condition = decision["condition"]
        require(isinstance(condition, str) and condition in conditions,
                "dependency readiness condition is unreviewed")
        for key in ("condition_explicit", "required", "required_explicit", "restart", "restart_explicit"):
            require(type(decision[key]) is bool, "dependency decision flag is not boolean")
        require((decision["condition_explicit"] or condition == "started")
                and (decision["required_explicit"] or decision["required"])
                and (decision["restart_explicit"] or not decision["restart"]),
                "dependency implicit flag differs from its documented default")
        require(decision["fidelity"] == "approximate" and decision["native_engine_field"] is False,
                "dependency decision incorrectly claims native Engine semantics")
        provenance = decision["provenance"]
        require(isinstance(provenance, dict) and set(provenance) == provenance_fields,
                "dependency provenance fields differ")
        require(all(isinstance(values, list) and len(values) <= 32
                    and all(isinstance(value, str) and value in categories for value in values)
                    for values in provenance.values()), "dependency provenance contains an unreviewed category")
        dependencies[source].add(target)
        edges.add((source, target, condition, decision["required"], decision["restart"]))
    require(edges == set(expected_edges), "dependency edges differ from independent application expectations")
    layers: list[list[str]] = []
    remaining = set(services)
    while remaining:
        layer = sorted(service for service in remaining if not dependencies[service] & remaining)
        require(bool(layer), "dependency decisions contain a cycle")
        layers.append(layer)
        remaining.difference_update(layer)
    return layers, decisions


def core_request(
    artifact_text: str,
    image: str,
    api_version: str,
    lane: str,
    observed_release: str | None = None,
    observed_api: str | None = None,
    *, profiles: dict[str, dict[str, Any]], expected_plan_sha256: str,
    observed_package: str | None = None,
) -> dict[str, Any]:
    # The offline archive is verified against the canonical digest before this
    # no-registry alias is installed in the disposable inner daemon.
    require(bool(CORE_IMAGE_ALIAS_PATTERN.fullmatch(image)), "core request must use the isolated preloaded image alias")
    require(bool(API_PATTERN.fullmatch(api_version)), "rendering API version must be explicit")
    require(lane in IMAGE_KEYS and lane != "fixture", "native lane is not one of the four reviewed identities")
    encoded = artifact_text.encode("utf-8")
    require(0 < len(encoded) <= 16_384, "core artifact must be nonempty and at most 16 KiB")
    lines = artifact_text.splitlines()
    require(len(lines) == 1 and lines[0].strip() == lines[0], "core artifact must contain exactly one JSON request")
    require(lane in profiles, "native lane has no active reviewed profile")
    source_bytes, expectations_bytes, _ = core_prerequisite_inputs()
    # The caller-selected digest binds a reviewed snapshot only. Independent
    # authored intent and selected catalogue context supply semantic authority;
    # the offline helper deliberately supplies no runtime/replay authority.
    try:
        core_artifact.validate_core(encoded, expectations_bytes=expectations_bytes, source_bytes=source_bytes,
                                    expected_plan_sha256=expected_plan_sha256, lane=lane,
                                    profile=profiles[lane], image_alias=image)
        record = core_artifact.topology.document(encoded)
    except core_artifact.topology.ExpectationError as error:
        raise ContractError(str(error)) from error
    context = profiles[lane]
    build = context["build"]
    if lane.startswith("debian11-"):
        require(build["revision"].startswith("20.10.5+"), "Debian native package revision differs")
    require(context["rendering_api_version"] == api_version, "requested rendering API differs from target context")
    if observed_release is not None:
        allowed_releases = {context["engine_release"]}
        if lane.startswith("debian11-") and context["engine_release"].endswith("+dfsg1"):
            allowed_releases.add(context["engine_release"].removesuffix("+dfsg1"))
        require(
            observed_release in allowed_releases,
            "target Engine release differs from observed daemon",
        )
    if observed_api is not None:
        require(context["advertised_api_version"] == observed_api, "target advertised API differs from observed daemon")
    if observed_package is not None:
        expected_package = context["build"].get("revision", "")
        require(observed_package == expected_package, "installed Docker package differs from reviewed profile")
    return record["requests"][0]


class UnixConnection(http.client.HTTPConnection):
    def __init__(self, socket_path: pathlib.Path) -> None:
        super().__init__("localhost", timeout=10)
        self.socket_path = socket_path

    def connect(self) -> None:
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(self.timeout)
        self.sock.connect(str(self.socket_path))


def engine_request(socket_path: pathlib.Path, method: str, path: str, body: bytes | None = None,
                   *, timeout: float = 10) -> tuple[int, bytes]:
    connection = UnixConnection(socket_path)
    connection.timeout = timeout
    try:
        headers = {"Content-Type": "application/json"} if body is not None else {}
        connection.request(method, path, body=body, headers=headers)
        response = connection.getresponse()
        data = response.read(16_385)
        require(len(data) <= 16_384, "Engine response exceeded the core limit")
        return response.status, data
    finally:
        connection.close()


def volume_document(raw: bytes) -> dict[str, Any]:
    try:
        value = core_artifact.topology.document(raw)
    except core_artifact.topology.ExpectationError as error:
        raise ContractError("volume document is not bounded closed JSON") from error
    require(isinstance(value, dict), "volume document must be an object")
    return value


def expected_volumes(run: str, prefix: str) -> list[dict[str, Any]]:
    require(isinstance(run, str) and VOLUME_TOKEN.fullmatch(run) is not None
            and isinstance(prefix, str) and VOLUME_TOKEN.fullmatch(prefix) is not None,
            "volume run and prefix must be closed tokens")
    return [{"Name": f"{prefix}-{suffix}",
             "Labels": {"io.boxferry.live-run": run, "io.boxferry.application": f"{prefix}-{owner}"}}
            for _, owner, _, suffixes in VOLUME_FIXTURES for suffix in suffixes]


def validate_volume_fixtures(directory: pathlib.Path, root: pathlib.Path, *, lane: str, run: str,
                             prefix: str, receipt_sha256: str, api_version: str,
                             profiles: dict[str, dict[str, Any]], observed_release: str | None = None,
                             observed_api: str | None = None, observed_package: str | None = None) -> list[dict[str, Any]]:
    """Validate all six complete artifacts/23 requests before authorizing any POST."""
    expected = expected_volumes(run, prefix)
    require(lane in CORE_PRESETS and lane in profiles, "volume lane has no reviewed profile")
    require(isinstance(receipt_sha256, str) and DIGEST_PATTERN.fullmatch(receipt_sha256) is not None,
            "volume candidate receipt binding is invalid")
    profile = profiles[lane]
    require(profile["rendering_api_version"] == api_version, "volume rendering API differs")
    releases = {profile["engine_release"]}
    if lane.startswith("debian11-") and profile["engine_release"].endswith("+dfsg1"):
        releases.add(profile["engine_release"].removesuffix("+dfsg1"))
    require(observed_release is None or observed_release in releases, "volume Engine release differs")
    require(observed_api is None or observed_api == profile["advertised_api_version"],
            "volume advertised API differs")
    require(observed_package is None or observed_package == profile["build"].get("revision", ""),
            "volume installed package differs")
    require(root.is_absolute() and root.resolve(strict=True) == root and root.is_dir(),
            "volume source checkout must be canonical")
    require(directory.is_absolute() and directory.resolve(strict=True) == directory
            and directory.is_dir() and directory.stat().st_uid == os.geteuid()
            and stat.S_IMODE(directory.stat().st_mode) == 0o700,
            "volume output must be an owner-private canonical directory")
    names = {"manifest.json", *(f"{identity}-volumes.json" for identity, _, _, _ in VOLUME_FIXTURES)}
    require(set(os.listdir(directory)) == names, "volume output files differ from closed inventory")
    require(all((directory / name).stat().st_uid == os.geteuid() for name in names),
            "volume output file ownership differs")
    manifest = volume_document(read_private_artifact(directory / "manifest.json").encode())
    require(set(manifest) == {"schema", "scope", "lane", "run", "prefix", "candidate_receipt_sha256", "fixtures"}
            and type(manifest["schema"]) is int and manifest["schema"] == 1
            and manifest["scope"] == "volume-only" and manifest["lane"] == lane
            and manifest["run"] == run and manifest["prefix"] == prefix
            and manifest["candidate_receipt_sha256"] == receipt_sha256,
            "volume manifest envelope or candidate binding differs")
    rows = manifest["fixtures"]
    require(isinstance(rows, list) and len(rows) == 6, "volume manifest fixture inventory differs")
    validated = []
    offset = 0
    for row, (identity, _owner, source_sha, suffixes) in zip(rows, VOLUME_FIXTURES, strict=True):
        require(isinstance(row, dict) and set(row) == {"id", "source_sha256", "artifact", "artifact_sha256", "volume_count"}
                and row["id"] == identity and row["source_sha256"] == source_sha
                and row["artifact"] == f"{identity}-volumes.json"
                and type(row["volume_count"]) is int and row["volume_count"] == len(suffixes),
                "volume manifest fixture fields differ")
        source = bounded_regular_bytes(root / f"fixtures/conformance/{identity}-application/compose.yaml", 65_536)
        require(hashlib.sha256(source).hexdigest() == source_sha, "volume original fixture source differs")
        raw = read_private_artifact(directory / row["artifact"]).encode()
        require(hashlib.sha256(raw).hexdigest() == row["artifact_sha256"], "volume artifact hash differs")
        artifact = volume_document(raw)
        require(set(artifact) == {"schema_version", "context", "requests", "prerequisites"}
                and type(artifact["schema_version"]) is int and artifact["schema_version"] == 1
                and artifact["context"] == profile and artifact["prerequisites"] == [],
                "volume complete artifact context or envelope differs")
        requests = artifact["requests"]
        bodies = expected[offset:offset + len(suffixes)]
        offset += len(suffixes)
        require(isinstance(requests, list) and len(requests) == len(bodies), "volume request count differs")
        remaining = {body["Name"]: body for body in bodies}
        for request in requests:
            require(isinstance(request, dict) and set(request) == {"method", "path", "body"}
                    and request["method"] == "POST" and request["path"] == f"/v{api_version}/volumes/create"
                    and isinstance(request["body"], dict), "volume native request envelope differs")
            body = request["body"]
            require(isinstance(body.get("Name"), str) and body["Name"] in remaining
                    and body == remaining[body["Name"]], "volume literal name or complete labels differ")
            remaining.pop(body["Name"])
            validated.append(request)
        require(not remaining, "volume literal inventory is incomplete")
    require(len(validated) == 23, "volume total inventory differs")
    return validated


def volume_socket(path: pathlib.Path) -> None:
    require(path.is_absolute() and not path.is_symlink() and stat.S_ISSOCK(path.stat().st_mode)
            and not path.parent.is_symlink() and stat.S_IMODE(path.parent.parent.stat().st_mode) == 0o700,
            "volume Engine socket must remain in the private run directory")


def volume_native_identity(raw: bytes, body: dict[str, Any]) -> None:
    value = volume_document(raw)
    require(value.get("Name") == body["Name"] and value.get("Labels") == body["Labels"],
            "native volume identity or complete ownership differs")


def volume_ledger(path: pathlib.Path, *, run: str, prefix: str, lane: str, api_version: str) -> dict[str, Any]:
    require(path.stat().st_uid == os.geteuid(), "volume ledger owner differs")
    value = volume_document(read_private_artifact(path).encode())
    require(set(value) == {"schema", "scope", "lane", "run", "prefix", "api_version", "volumes"}
            and type(value["schema"]) is int and value["schema"] == 1 and value["scope"] == "volume-only"
            and value["lane"] == lane and value["run"] == run and value["prefix"] == prefix
            and value["api_version"] == api_version and lane in CORE_PRESETS
            and API_PATTERN.fullmatch(api_version) is not None,
            "volume cleanup ledger envelope differs")
    expected = {body["Name"]: body for body in expected_volumes(run, prefix)}
    volumes = value["volumes"]
    require(isinstance(volumes, list) and len(volumes) <= 23, "volume ledger count differs")
    for body in volumes:
        require(isinstance(body, dict) and isinstance(body.get("Name"), str)
                and body["Name"] in expected and body == expected.pop(body["Name"]),
                "volume ledger ownership or inventory differs")
    return value


def volume_call(socket_path: pathlib.Path, method: str, path: str, body: bytes | None, deadline: float) -> tuple[int, bytes]:
    remaining = deadline - time.monotonic()
    require(remaining > 0, "volume request deadline exhausted")
    return engine_request(socket_path, method, path, body, timeout=min(10, remaining))


def apply_volume_fixtures(socket_path: pathlib.Path, state: pathlib.Path, requests: list[dict[str, Any]],
                          *, run: str, prefix: str, lane: str, api_version: str) -> None:
    expected = {body["Name"]: body for body in expected_volumes(run, prefix)}
    require(lane in CORE_PRESETS and API_PATTERN.fullmatch(api_version) is not None
            and isinstance(requests, list) and len(requests) == 23, "volume apply inventory differs")
    for request in requests:
        require(isinstance(request, dict) and set(request) == {"method", "path", "body"}
                and request["method"] == "POST" and request["path"] == f"/v{api_version}/volumes/create"
                and isinstance(request["body"], dict) and isinstance(request["body"].get("Name"), str)
                and request["body"]["Name"] in expected
                and request["body"] == expected.pop(request["body"]["Name"]),
                "volume apply native request differs")
    volume_socket(socket_path)
    require(state.is_absolute() and not state.exists() and not state.is_symlink()
            and state.parent.is_dir() and not state.parent.is_symlink()
            and stat.S_IMODE(state.parent.stat().st_mode) == 0o700, "volume ledger must be new and private")
    ledger = {"schema": 1, "scope": "volume-only", "lane": lane, "run": run, "prefix": prefix,
              "api_version": api_version, "volumes": []}
    deadline = time.monotonic() + 40
    descriptor = os.open(state, os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        identity = os.fstat(descriptor)
        def save() -> None:
            nonlocal identity
            raw = json.dumps(ledger, separators=(",", ":"), sort_keys=True).encode()
            require(len(raw) <= 16_384, "volume ledger exceeds bound")
            current = state.lstat()
            require((current.st_dev, current.st_ino) == (identity.st_dev, identity.st_ino)
                    and stat.S_ISREG(current.st_mode), "volume ledger identity changed")
            temporary_fd, temporary_name = tempfile.mkstemp(prefix=".volume-ledger-", dir=state.parent)
            try:
                view = memoryview(raw)
                while view:
                    written = os.write(temporary_fd, view)
                    require(written > 0, "volume ledger write failed")
                    view = view[written:]
                os.fsync(temporary_fd)
                identity = os.fstat(temporary_fd)
                os.replace(temporary_name, state)
            finally:
                os.close(temporary_fd)
                if os.path.lexists(temporary_name):
                    os.unlink(temporary_name)
        save()
        for request in requests:
            body = request["body"]
            path = f"/v{api_version}/volumes/{body['Name']}"
            status, _ = volume_call(socket_path, "GET", path, None, deadline)
            require(status == 404, "volume name is occupied or absence inspection failed")
            ledger["volumes"].append(body)
            save()  # Registration survives a partially successful/failed/timed-out POST.
            raw = json.dumps(body, separators=(",", ":")).encode()
            status, response = volume_call(socket_path, "POST", request["path"], raw, deadline)
            require(status == 201, "volume create failed")
            volume_native_identity(response, body)
            status, response = volume_call(socket_path, "GET", path, None, deadline)
            require(status == 200, "created volume inspection failed")
            volume_native_identity(response, body)
    finally:
        os.close(descriptor)


def cleanup_volume_fixtures(socket_path: pathlib.Path, state: pathlib.Path, *, run: str, prefix: str,
                            lane: str, api_version: str) -> None:
    ledger = volume_ledger(state, run=run, prefix=prefix, lane=lane, api_version=api_version)
    volume_socket(socket_path)
    deadline = time.monotonic() + 25
    failed = False
    for body in reversed(ledger["volumes"]):
        path = f"/v{api_version}/volumes/{body['Name']}"
        try:
            status, response = volume_call(socket_path, "GET", path, None, deadline)
            if status == 404:
                continue
            require(status == 200, "volume cleanup inspection failed")
            volume_native_identity(response, body)
            status, _ = volume_call(socket_path, "DELETE", path, None, deadline)
            require(status == 204, "owned volume removal failed")
            status, _ = volume_call(socket_path, "GET", path, None, deadline)
            require(status == 404, "owned volume absence is unverified")
        except (ContractError, OSError, ValueError, http.client.HTTPException):
            failed = True
    require(not failed, "volume cleanup is incomplete or ownership unavailable")


def replay(
    socket_text: str,
    artifact_text: str,
    image: str,
    api_version: str,
    lane: str,
    observed_release: str,
    observed_api: str,
    state_text: str,
    *, profiles: dict[str, dict[str, Any]], observed_package: str, expected_plan_sha256: str,
) -> dict[str, str]:
    record = core_request(artifact_text, image, api_version, lane, observed_release, observed_api,
                          profiles=profiles, observed_package=observed_package,
                          expected_plan_sha256=expected_plan_sha256)
    socket_path = pathlib.Path(socket_text)
    require(socket_path.is_absolute() and not socket_path.is_symlink(), "Engine socket must be an explicit regular path")
    require(stat.S_ISSOCK(socket_path.stat().st_mode), "explicit Engine socket is absent")
    require(
        socket_path.parent.is_dir() and not socket_path.parent.is_symlink()
        and stat.S_IMODE(socket_path.parent.parent.stat().st_mode) == 0o700,
        "Engine socket must remain below a private run directory",
    )
    state_path = pathlib.Path(state_text)
    require(state_path.is_absolute() and not state_path.exists(), "state path must be absolute and new")
    require(state_path.parent.is_dir() and not state_path.parent.is_symlink(), "state parent must be a real directory")
    require(stat.S_IMODE(state_path.parent.stat().st_mode) == 0o700, "state parent must be private")
    status, _ = engine_request(socket_path, "GET", f"/v{api_version}/containers/{CONTAINER_NAME}/json")
    require(status == 404, "core container name is already occupied or inspection failed")
    body = json.dumps(record["body"], separators=(",", ":"), ensure_ascii=False).encode("utf-8")
    status, data = engine_request(socket_path, "POST", record["path"], body)
    require(status == 201, f"Engine rejected the core create request with status {status}")
    try:
        container_id = json.loads(data)["Id"]
    except (ValueError, KeyError, TypeError) as error:
        raise ContractError("Engine create response lacks a container ID") from error
    require(isinstance(container_id, str) and re.fullmatch(r"[0-9a-f]{64}", container_id) is not None, "Engine container ID is invalid")
    state = {"container_id": container_id, "name": CONTAINER_NAME, "image": image, "api_version": api_version}
    descriptor = os.open(state_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as output:
        json.dump(state, output, sort_keys=True)
        output.write("\n")
    status, _ = engine_request(socket_path, "POST", f"/v{api_version}/containers/{container_id}/start")
    require(status == 204, f"Engine rejected the harness-owned core start with status {status}")
    return state


READINESS_INSPECT_FORMAT = (
    '{"id":{{json .ID}},"name":{{json .Name}},"labels":{{json .Config.Labels}},'
    '"state":{{json .State.Status}},"running":{{json .State.Running}}}'
)


def readiness_read(arguments: list[str], deadline: float, *, merge_output: bool = False) -> tuple[str, bytes]:
    """Bound one read-only subprocess; neither stderr nor failed output escapes."""
    process = None
    outcome, raw = "read-failed", b""
    interrupted = False
    termination_failed = False
    completed = False
    status = None
    expires = min(deadline, time.monotonic() + 3)
    try:
        if expires <= time.monotonic():
            return "timed-out", b""
        process = subprocess.Popen(arguments, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                   stderr=subprocess.STDOUT if merge_output else subprocess.DEVNULL,
                                   start_new_session=True)
        assert process.stdout is not None
        descriptor = process.stdout.fileno()
        os.set_blocking(descriptor, False)
        output = bytearray()
        while True:
            remaining = expires - time.monotonic()
            if remaining <= 0:
                outcome = "timed-out"
                break
            if not select.select([descriptor], [], [], remaining)[0]:
                outcome = "timed-out"
                break
            chunk = os.read(descriptor, min(4096, 16_385 - len(output)))
            if not chunk:
                # WNOWAIT keeps the leader PID reserved until its owned group
                # has been terminated. Never reap and then signal a numeric PGID.
                while True:
                    observed = os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
                    if observed is not None:
                        completed, raw = True, bytes(output)
                        break
                    remaining = expires - time.monotonic()
                    if remaining <= 0:
                        outcome = "timed-out"
                        break
                    time.sleep(min(0.01, remaining))
                break
            output.extend(chunk)
            if len(output) > 16_384:
                outcome = "oversized"
                break
    except subprocess.TimeoutExpired:
        outcome = "timed-out"
    except OSError:
        pass
    except KeyboardInterrupt:
        interrupted = True
    finally:
        if process is not None:
            # This session belongs only to this diagnostic read, not the daemon.
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            except (OSError, KeyboardInterrupt):
                termination_failed = True
                try:
                    process.kill()
                except (OSError, KeyboardInterrupt):
                    pass
            try:
                status = process.wait(timeout=max(0.001, min(1, deadline - time.monotonic())))
            except (OSError, subprocess.TimeoutExpired, KeyboardInterrupt):
                termination_failed = True
            finally:
                if process.stdout is not None:
                    try:
                        process.stdout.close()
                    except OSError:
                        termination_failed = True
    if termination_failed:
        return "termination-unverified", b""
    if interrupted:
        raise KeyboardInterrupt
    if completed:
        return ("read", raw) if status == 0 else ("read-failed", b"")
    return outcome, raw


def readiness_log_category(raw: bytes) -> str:
    if not raw.strip():
        return "empty"
    lowered = raw.lower()
    observations = {
        "permission-error-observed": (b"permission denied",),
        "storage-error-observed": (b"failed to mount overlay", b"error initializing graphdriver"),
        "network-error-observed": (b"failed to create nat chain", b"iptables failed"),
        "socket-error-observed": (b"address already in use",),
        "startup-error-observed": (b"failed to start daemon",),
    }
    matched = [category for category, phrases in observations.items() if any(phrase in lowered for phrase in phrases)]
    return matched[0] if len(matched) == 1 else "multiple-errors-observed" if matched else "content-present"


def readiness_diagnostics(outer: str, run: str, socket_path: pathlib.Path, *, registered: bool) -> dict[str, str]:
    """Private observations only; this cannot establish startup cause or readiness."""
    result = {"socket": "unavailable", "outer": "not-registered", "state": "unverified", "logs": "not-read"}
    if not registered or re.fullmatch(r"[A-Za-z0-9_]{1,64}", run) is None or outer != f"bf-docker-core-{run}":
        return result
    if socket_path != pathlib.Path(f"/tmp/boxferry-docker-core.{run}/socket/docker.sock"):
        result["outer"] = "invalid-boundary"
        return result
    deadline = time.monotonic() + 8
    try:
        root = socket_path.parent.parent.lstat()
        require(stat.S_ISDIR(root.st_mode) and stat.S_IMODE(root.st_mode) == 0o700 and root.st_uid == os.geteuid(),
                "readiness private root differs")
        require(stat.S_ISDIR(socket_path.parent.lstat().st_mode), "readiness socket directory differs")
    except (OSError, ContractError):
        result["outer"] = "invalid-boundary"
        return result
    try:
        mode = socket_path.lstat().st_mode
        result["socket"] = "socket" if stat.S_ISSOCK(mode) else "not-socket"
    except FileNotFoundError:
        result["socket"] = "absent"
    except OSError:
        pass
    previous = {}
    def cancelled(_signum, _frame):
        raise KeyboardInterrupt
    try:
        for signum in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
            previous[signum] = signal.signal(signum, cancelled)
        outcome, raw = readiness_read(["podman", "inspect", "--format", READINESS_INSPECT_FORMAT, outer], deadline)
        if outcome != "read":
            result["outer"] = outcome
            return result
        try:
            record = volume_document(raw)
            require(set(record) == {"id", "name", "labels", "state", "running"}
                    and isinstance(record["id"], str) and DIGEST_PATTERN.fullmatch(record["id"]) is not None
                    and record["name"] == outer and isinstance(record["labels"], dict)
                    and all(isinstance(key, str) and isinstance(value, str) for key, value in record["labels"].items())
                    and isinstance(record["state"], str) and type(record["running"]) is bool,
                    "readiness inspect shape differs")
        except ContractError:
            result["outer"] = "malformed"
            return result
        if record["labels"].get("io.boxferry.docker-core-run") != run:
            result["outer"] = "wrong-owner"
            return result
        result["outer"] = "verified"
        state = record["state"]
        if state in {"configured", "created", "running", "stopped", "exited", "paused", "restarting", "removing", "stopping"}:
            result["state"] = state if record["running"] == (state == "running") else "inconsistent"
        else:
            result["state"] = "unknown"
        outcome, raw = readiness_read(["podman", "logs", "--tail", "80", record["id"]], deadline, merge_output=True)
        result["logs"] = readiness_log_category(raw) if outcome == "read" else outcome
    except KeyboardInterrupt:
        result["logs"] = "cancelled"
        if result["outer"] != "verified":
            result["outer"] = "cancelled"
    finally:
        for signum, handler in previous.items():
            signal.signal(signum, handler)
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    readiness = commands.add_parser("readiness-diagnostics")
    readiness.add_argument("--outer", required=True)
    readiness.add_argument("--run", required=True)
    readiness.add_argument("--socket", type=pathlib.Path, required=True)
    readiness.add_argument("--registered", action="store_true")
    catalogue = commands.add_parser("catalogue")
    catalogue.add_argument("--docker-lens-root", required=True)
    catalogue.add_argument("--docker-lens-revision", required=True)
    catalogue.add_argument("--native-script-sha256", required=True)
    artifact_identity = commands.add_parser("artifact-identity", help="bind bounded private artifact bytes, not semantic authority")
    artifact_identity.add_argument("--artifact", type=pathlib.Path, required=True)
    candidate = commands.add_parser("verify-candidate")
    candidate.add_argument("--boxferry-root", required=True)
    candidate.add_argument("--binary", required=True)
    candidate.add_argument("--receipt", required=True)
    candidate.add_argument("--docker-lens-root", required=True)
    candidate.add_argument("--docker-lens-revision", required=True)
    candidate.add_argument("--profile", choices=("core-journey", "volume-fixtures"), default="core-journey")
    snapshot = commands.add_parser("capture-candidate")
    snapshot.add_argument("--boxferry-root", required=True)
    snapshot.add_argument("--binary", required=True)
    snapshot.add_argument("--receipt", required=True)
    snapshot.add_argument("--docker-lens-root", required=True)
    snapshot.add_argument("--docker-lens-revision", required=True)
    snapshot.add_argument("--destination", required=True)
    snapshot.add_argument("--profile", choices=("core-journey", "volume-fixtures"), default="core-journey")
    for name in ("validate-volume-fixtures", "apply-volume-fixtures"):
        volumes = commands.add_parser(name)
        volumes.add_argument("--directory", type=pathlib.Path, required=True)
        volumes.add_argument("--boxferry-root", type=pathlib.Path, required=True)
        volumes.add_argument("--lane", required=True)
        volumes.add_argument("--run", required=True)
        volumes.add_argument("--prefix", required=True)
        volumes.add_argument("--receipt-sha256", required=True)
        volumes.add_argument("--api-version", required=True)
        volumes.add_argument("--catalogue-json", required=True)
        volumes.add_argument("--observed-release")
        volumes.add_argument("--observed-api")
        volumes.add_argument("--observed-package")
        if name == "apply-volume-fixtures":
            volumes.add_argument("--socket", type=pathlib.Path, required=True)
            volumes.add_argument("--state", type=pathlib.Path, required=True)
            volumes.add_argument("--allow-isolated-apply", action="store_true")
    volumes_cleanup = commands.add_parser("cleanup-volume-fixtures")
    volumes_cleanup.add_argument("--socket", type=pathlib.Path, required=True)
    volumes_cleanup.add_argument("--state", type=pathlib.Path, required=True)
    volumes_cleanup.add_argument("--lane", required=True)
    volumes_cleanup.add_argument("--run", required=True)
    volumes_cleanup.add_argument("--prefix", required=True)
    volumes_cleanup.add_argument("--api-version", required=True)
    volumes_cleanup.add_argument("--allow-isolated-apply", action="store_true")
    source_digest = commands.add_parser("candidate-source-digest")
    source_digest.add_argument("--boxferry-root", required=True)
    sample = commands.add_parser("sample-owned-storage")
    sample.add_argument("--volume", required=True)
    sample.add_argument("--run-dir", required=True)
    sample.add_argument("--graph-root", required=True)
    sample.add_argument("--baseline-free", required=True, type=int)
    source = commands.add_parser("render-core-source")
    source.add_argument("--template", required=True)
    source.add_argument("--destination", required=True)
    source.add_argument("--image", required=True)
    checked = commands.add_parser("check-core-output")
    checked.add_argument("--kind", required=True,
                         choices=("plan-report", "plan-artifact", "compose-report", "compose-console", "compose",
                                  "inspect", "network-inspect", "state-id"))
    checked.add_argument("--file", required=True)
    checked.add_argument("--image")
    checked.add_argument("--fixture-image")
    checked.add_argument("--container-id")
    checked.add_argument("--container-file")
    checked.add_argument("--network-file")
    checked.add_argument("--network-id")
    checked.add_argument("--socket-path")
    checked.add_argument("--lane")
    checked.add_argument("--catalogue-json")
    artifact = commands.add_parser("validate-artifact")
    artifact.add_argument("--artifact", type=pathlib.Path, required=True)
    artifact.add_argument("--artifact-sha256", required=True,
                          help="Caller-selected reviewed snapshot digest; identity only, not semantic authority.")
    artifact.add_argument("--image", required=True)
    artifact.add_argument("--api-version", required=True)
    artifact.add_argument("--lane", required=True)
    artifact.add_argument("--observed-release")
    artifact.add_argument("--observed-api")
    artifact.add_argument("--observed-package")
    artifact.add_argument("--catalogue-json", required=True)
    apply = commands.add_parser("replay-test-only")
    apply.add_argument("--artifact", type=pathlib.Path, required=True)
    apply.add_argument("--artifact-sha256", required=True,
                       help="Reuse the reviewed snapshot digest; never recompute it to authorize changed bytes.")
    apply.add_argument("--image", required=True)
    apply.add_argument("--api-version", required=True)
    apply.add_argument("--lane", required=True)
    apply.add_argument("--observed-release", required=True)
    apply.add_argument("--observed-api", required=True)
    apply.add_argument("--observed-package", required=True)
    apply.add_argument("--catalogue-json", required=True)
    apply.add_argument("--socket", required=True)
    apply.add_argument("--state", required=True)
    apply.add_argument("--allow-isolated-apply", action="store_true")
    mode = commands.add_parser("verify-mode")
    mode.add_argument("--lane", required=True)
    mode.add_argument("--rootless-marker", required=True)
    mode.add_argument("--uid-report", required=True)
    mount = commands.add_parser("verify-storage-mount")
    mount.add_argument("--destination", required=True)
    docker_root = commands.add_parser("verify-docker-root")
    docker_root.add_argument("--lane", required=True)
    docker_root.add_argument("--observed", required=True)
    outer_mounts = commands.add_parser("verify-outer-mounts")
    outer_mounts.add_argument("--volume", required=True)
    outer_mounts.add_argument("--destination", required=True)
    outer_mounts.add_argument("--socket-dir", required=True)
    parent_start = commands.add_parser("parent-start")
    parent_start.add_argument("--parent-pid", type=int, required=True)
    parent_state = commands.add_parser("parent-alive")
    parent_state.add_argument("--parent-pid", type=int, required=True)
    parent_state.add_argument("--parent-start", type=int, required=True)
    parent_signal = commands.add_parser("signal-parent")
    parent_signal.add_argument("--parent-pid", type=int, required=True)
    parent_signal.add_argument("--parent-start", type=int, required=True)
    deadline = commands.add_parser("deadline-guard")
    deadline.add_argument("--parent-pid", type=int, required=True)
    deadline.add_argument("--parent-start", type=int, required=True)
    deadline.add_argument("--execution-seconds", type=int, required=True)
    deadline.add_argument("--cleanup-seconds", type=int, required=True)
    deadline.add_argument("--ready-file")
    args = parser.parse_args()
    try:
        if args.command == "readiness-diagnostics":
            result = readiness_diagnostics(args.outer, args.run, args.socket, registered=args.registered)
            print("readiness observations: " + " ".join(f"{key}={value}" for key, value in result.items())
                  + "; startup-cause=unestablished")
        elif args.command == "parent-start":
            print(process_start(args.parent_pid))
        elif args.command == "parent-alive":
            fd = verified_parent_fd(args.parent_pid, args.parent_start)
            try:
                if not parent_alive(fd):
                    raise ParentGone("parent process has exited")
            finally:
                os.close(fd)
        elif args.command == "signal-parent":
            signal_parent(args.parent_pid, args.parent_start)
        elif args.command == "deadline-guard":
            guard_parent(args.parent_pid, args.parent_start,
                         args.execution_seconds, args.cleanup_seconds, args.ready_file)
        elif args.command == "artifact-identity":
            print(hashlib.sha256(read_private_artifact(args.artifact).encode("utf-8")).hexdigest())
        elif args.command == "catalogue":
            result = canonical_images(args.docker_lens_root, args.docker_lens_revision, args.native_script_sha256)
            result["profiles"] = reviewed_profiles(args.docker_lens_root)
            print(json.dumps(result, sort_keys=True))
        elif args.command == "verify-candidate":
            print(json.dumps(verify_candidate(pathlib.Path(args.boxferry_root),
                                              pathlib.Path(args.binary), pathlib.Path(args.receipt),
                                              pathlib.Path(args.docker_lens_root), args.docker_lens_revision,
                                              profile=args.profile),
                             sort_keys=True))
        elif args.command == "capture-candidate":
            print(json.dumps(capture_candidate(pathlib.Path(args.boxferry_root),
                                               pathlib.Path(args.binary), pathlib.Path(args.receipt),
                                               pathlib.Path(args.docker_lens_root), args.docker_lens_revision,
                                               pathlib.Path(args.destination), profile=args.profile), sort_keys=True))
        elif args.command in {"validate-volume-fixtures", "apply-volume-fixtures"}:
            catalogue = json.loads(args.catalogue_json, object_pairs_hook=no_duplicate_keys)
            requests = validate_volume_fixtures(
                args.directory, args.boxferry_root, lane=args.lane, run=args.run, prefix=args.prefix,
                receipt_sha256=args.receipt_sha256, api_version=args.api_version,
                profiles=catalogue["profiles"], observed_release=args.observed_release,
                observed_api=args.observed_api, observed_package=args.observed_package)
            if args.command == "apply-volume-fixtures":
                require(args.allow_isolated_apply and all(value is not None for value in
                        (args.observed_release, args.observed_api, args.observed_package)),
                        "volume apply requires explicit isolated permission and observed identity")
                apply_volume_fixtures(args.socket, args.state, requests, run=args.run, prefix=args.prefix,
                                      lane=args.lane, api_version=args.api_version)
            print("volume-only fixture contract verified; no application acceptance")
        elif args.command == "cleanup-volume-fixtures":
            require(args.allow_isolated_apply, "volume cleanup requires explicit isolated permission")
            cleanup_volume_fixtures(args.socket, args.state, run=args.run, prefix=args.prefix,
                                    lane=args.lane, api_version=args.api_version)
            print("volume-only owned cleanup and absence verified")
        elif args.command == "candidate-source-digest":
            root = pathlib.Path(args.boxferry_root)
            print(json.dumps({"revision": git(root, "rev-parse", "HEAD"),
                              "source_sha256": candidate_source_digest(root)}, sort_keys=True))
        elif args.command == "sample-owned-storage":
            print(json.dumps(sample_owned_storage(pathlib.Path(args.volume), pathlib.Path(args.run_dir),
                                                  pathlib.Path(args.graph_root), args.baseline_free),
                             sort_keys=True))
        elif args.command == "render-core-source":
            render_core_source(pathlib.Path(args.template), pathlib.Path(args.destination), args.image)
        elif args.command == "check-core-output":
            path = pathlib.Path(args.file)
            entries = ({"docker-plan.json"} if args.kind == "plan-artifact" else
                       {"compose.yaml"} if args.kind == "compose" else None)
            raw = core_output_file(path, 65_536 if args.kind in ("inspect", "network-inspect",
                                                               "plan-report", "compose-report",
                                                               "compose-console")
                                   else 16_384, entries, allow_empty=args.kind == "compose-console")
            if args.kind == "plan-report":
                catalogue = json.loads(args.catalogue_json, object_pairs_hook=no_duplicate_keys)
                core_report(raw, "compose", "docker", args.lane, catalogue["profiles"])
            elif args.kind == "plan-artifact":
                require(path.name == "docker-plan.json", "core Docker plan filename differs")
            elif args.kind in ("compose-report", "compose-console"):
                require(all(value is not None for value in
                            (args.container_file, args.network_file, args.container_id,
                             args.network_id, args.image, args.fixture_image, args.socket_path)),
                        "reacquired privacy selectors are incomplete")
                core_reacquired_privacy(raw, pathlib.Path(args.container_file),
                                         pathlib.Path(args.network_file), args.container_id,
                                         args.network_id, args.image, args.fixture_image,
                                         args.socket_path)
                if args.kind == "compose-report":
                    catalogue = json.loads(args.catalogue_json, object_pairs_hook=no_duplicate_keys)
                    core_report(raw, "docker", "compose", args.lane, catalogue["profiles"])
            elif args.kind == "compose":
                require(args.container_file is not None and args.network_file is not None,
                        "independent native topology evidence is required for Compose output")
                container_raw = core_output_file(pathlib.Path(args.container_file), 65_536)
                network_raw = core_output_file(pathlib.Path(args.network_file), 65_536)
                network_id, endpoint_id = core_inspect(container_raw, args.container_id, args.image,
                                                       args.fixture_image)
                require(network_id == args.network_id,
                        "Compose bridge selector differs from validated native endpoint")
                core_network_inspect(network_raw, network_id, args.container_id, endpoint_id)
                core_generated_compose(raw, args.image)
            elif args.kind == "inspect":
                network_id, _ = core_inspect(raw, args.container_id, args.image,
                                             args.fixture_image)
                print(network_id)
            elif args.kind == "network-inspect":
                require(args.container_file is not None, "native container evidence is missing")
                container_raw = core_output_file(pathlib.Path(args.container_file), 65_536)
                network_id, endpoint_id = core_inspect(container_raw, args.container_id, args.image,
                                                       args.fixture_image)
                require(network_id == args.network_id,
                        "native network selector differs from container endpoint")
                core_network_inspect(raw, network_id, args.container_id, endpoint_id)
            else:
                state = json.loads(raw, object_pairs_hook=no_duplicate_keys)
                require(isinstance(state, dict) and state.get("name") == CONTAINER_NAME
                        and state.get("image") == args.image
                        and isinstance(state.get("container_id"), str)
                        and re.fullmatch(r"[0-9a-f]{64}", state["container_id"]) is not None,
                        "core apply state identity differs")
                print(state["container_id"])
        elif args.command == "verify-mode":
            verify_daemon_mode(args.lane, args.rootless_marker, args.uid_report)
            print("verified exact daemon mode from process and marker")
        elif args.command == "verify-storage-mount":
            verify_storage_mount(sys.stdin.buffer.read(1_048_577), args.destination)
            print("verified bounded historical data-root mount flags")
        elif args.command == "verify-docker-root":
            verify_docker_root(args.lane, args.observed)
            print("verified isolated Docker data-root destination")
        elif args.command == "verify-outer-mounts":
            verify_outer_mounts(sys.stdin.buffer.read(16_385), args.volume, args.destination, args.socket_dir)
            print("verified two exact outer mount destinations")
        else:
            value = read_private_artifact(args.artifact)
            catalogue = json.loads(args.catalogue_json, object_pairs_hook=no_duplicate_keys)
            profiles = catalogue["profiles"]
            if args.command == "validate-artifact":
                core_request(value, args.image, args.api_version, args.lane, args.observed_release, args.observed_api,
                             profiles=profiles, observed_package=args.observed_package,
                             expected_plan_sha256=args.artifact_sha256)
                print("validated one inert core request; no runtime operation performed")
            else:
                require(args.allow_isolated_apply, "test-only replay requires explicit isolated-apply flag")
                replay(
                    args.socket, value, args.image, args.api_version, args.lane,
                    args.observed_release, args.observed_api, args.state,
                    profiles=profiles, observed_package=args.observed_package,
                    expected_plan_sha256=args.artifact_sha256,
                )
                print("test-only core request created and started; harness owns cleanup")
        return 0
    except ParentGone as error:
        print(f"docker application contract: {error}", file=sys.stderr)
        return 3
    except ContractError as error:
        print(f"docker application contract failed: {error}", file=sys.stderr)
        return 1
    except (OSError, UnicodeError, subprocess.TimeoutExpired, KeyError, TypeError, ValueError,
            http.client.HTTPException):
        # Native or caller-provided values must not escape via exception text.
        print("docker application contract failed: invalid or unavailable bounded input", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
