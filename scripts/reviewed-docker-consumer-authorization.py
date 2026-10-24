#!/usr/bin/env python3
"""Trusted-main authorization and closed source copying; receipts are not authority."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess


MANIFEST = Path(".github/reviewed-docker-consumer-authorizations.json")
LANES = ("debian11-rootful", "debian11-rootless", "upstream-rootful", "upstream-rootless")
INERT_TESTS = (
    "full_nextcloud_and_explicit_supabase_bind_mount_only_projection_retain_binds_for_all_profiles",
    "generated_external_constraint_controls_retain_actual_fixture_binds_on_all_four_profiles",
)
FIXTURES = (
    ("forgejo", "forgejo", ("forge-db", "forge-data")),
    ("nextcloud", "cloud", ("cloud-database", "cloud-redis", "cloud-nextcloud")),
    ("paperless-ngx", "paperless", ("paper-data", "paper-media", "paper-consume", "paper-export", "paper-pgdata", "paper-redisdata")),
    ("immich", "immich", ("immich-library", "immich-model-cache", "immich-pgdata", "immich-redisdata")),
    ("observability", "observability", ("observability-alloy-data", "observability-grafana-data", "observability-loki-data",
                                      "observability-prometheus-data", "observability-telemetry-logs")),
    ("supabase", "supabase", ("supabase-deno-cache", "supabase-pgdata", "supabase-storage")),
)
BOX_FILES = frozenset({
    "scripts/docker-application-conformance.sh",
    "scripts/lib/docker-application-contract.py",
    "scripts/lib/docker-core-artifact.py",
    "scripts/lib/docker-application-schedule.py",
    "scripts/lib/docker-application-expectations.py",
    "scripts/lib/bounded-native-read.py",
    "crates/boxferry/examples/docker-volume-fixture-rehearsal.rs",
    "crates/boxferry/tests/support/docker_volume_fixtures.rs",
    "crates/boxferry/examples/docker-host-metadata.rs",
    "crates/boxferry/tests/docker_authored_bind_rehearsal.rs",
    "fixtures/scenarios/nextcloud-application/compose.yaml",
    "fixtures/scenarios/nextcloud-application/compose.acceptance.yaml",
    "fixtures/scenarios/nextcloud-application/nextcloud.env.example",
    "fixtures/conformance/docker-application/expected-applications.json",
    "fixtures/conformance/docker-application/core-expectations.json",
    "fixtures/conformance/docker-application/core-reacquire-expectations.json",
    "fixtures/conformance/docker-application/core.compose.yaml",
    *(f"fixtures/conformance/{name}-application/compose.yaml" for name, _, _ in FIXTURES),
})
OUTER_RUNTIME_FILES = frozenset({
    "scripts/native-outer-command.sh",
    "scripts/native-outer-runtime.sh", "scripts/native-outer-observation.py", "scripts/native-presence.py",
    "scripts/native-runner-apparmor.py", "scripts/native_outer_diagnostic_capture.py",
})
LENS_FILES = frozenset({
    "scripts/native-conformance.sh", "scripts/native-unix-http.py", "src/reviewed_catalog.rs",
}) | OUTER_RUNTIME_FILES
POLICY = {
    "outer_engine": "docker",
    "scope": "volume-fixtures-only", "lanes": list(LANES), "fixtures": 6, "volumes_per_lane": 23,
    "inert_tests": list(INERT_TESTS), "minimum_available_memory_gib": 6, "minimum_free_disk_gib": 8,
    "outer_pids": 512, "outer_memory_gib": 4, "outer_cpus": 2,
    "execution_seconds": 780, "cleanup_seconds": 120, "cargo_jobs": 2,
    "upstream_rootless_prerequisite": "github_runner_userns_v1",
}
SHA = re.compile(r"[0-9a-f]{40}\Z")
DIGEST = re.compile(r"[0-9a-f]{64}\Z")
RECORD = re.compile(r'docs/evidence/reviewed/sha256/([0-9a-f]{64})\.json')


class Rejection(ValueError):
    """Closed trusted admission failure; caller never prints candidate payloads."""


def require(condition: bool) -> None:
    if not condition:
        raise Rejection("reviewed Docker consumer authorization rejected")


def keys(value, expected) -> None:
    require(type(value) is dict and set(value) == set(expected))


def sha(value, pattern=DIGEST) -> None:
    require(type(value) is str and pattern.fullmatch(value) is not None)


def positive(value) -> None:
    require(type(value) is int and 0 < value < 2**63)


def no_duplicates(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result)
        result[key] = value
    return result


def read(path: Path, maximum=1024 * 1024) -> bytes:
    require(path.is_absolute() and path.resolve(strict=True) == path)
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as stream:
        before = os.fstat(stream.fileno())
        require(stat.S_ISREG(before.st_mode) and before.st_nlink == 1 and 0 < before.st_size <= maximum)
        value = stream.read(maximum + 1)
        after = os.fstat(stream.fileno())
        identity = lambda item: (item.st_dev, item.st_ino, item.st_size, item.st_mtime_ns, item.st_ctime_ns)
        require(len(value) == before.st_size and identity(before) == identity(after))
    return value


def document(path: Path, maximum=1024 * 1024):
    return json.loads(read(path, maximum), object_pairs_hook=no_duplicates)


def git(root: Path, *arguments: str) -> str:
    result = subprocess.run(
        ["git", "-c", f"safe.directory={root}", "-c", "core.fsmonitor=false", "-C", str(root), *arguments],
        capture_output=True, timeout=20, check=False,
        env={"PATH": "/usr/bin:/bin", "LC_ALL": "C", "GIT_CONFIG_NOSYSTEM": "1",
             "GIT_CONFIG_GLOBAL": os.devnull, "GIT_OPTIONAL_LOCKS": "0"},
    )
    require(result.returncode == 0 and len(result.stdout) <= 1024 * 1024)
    return result.stdout.decode().strip()


def source_digest(root: Path) -> str:
    """Same bounded source identity algorithm as the reviewed producer; no imports."""
    require(root.is_absolute() and root.resolve(strict=True) == root)
    require(git(root, "rev-parse", "--show-toplevel") == str(root))
    names = git(root, "ls-files", "--cached", "--others", "--exclude-standard", "-z").split("\0")
    names = sorted(set(name for name in names if name))
    require(0 < len(names) <= 10000)
    digest, total = hashlib.sha256(), 0
    for name in names:
        path = Path(name)
        require(not path.is_absolute() and ".." not in path.parts and "\n" not in name)
        raw = read(root / path, 4 * 1024 * 1024)
        total += len(raw)
        require(total <= 128 * 1024 * 1024)
        encoded = os.fsencode(name)
        digest.update(len(encoded).to_bytes(4, "big") + encoded)
        digest.update(len(raw).to_bytes(8, "big") + raw)
    return digest.hexdigest()


def entry(value, repository: str) -> None:
    keys(value, ("repository", "repository_id", "pr", "revision", "source_sha256", "lock_sha256", "files"))
    require(value["repository"] == repository)
    positive(value["repository_id"])
    positive(value["pr"])
    sha(value["revision"], SHA)
    sha(value["source_sha256"])
    sha(value["lock_sha256"])
    files = value["files"]
    require(type(files) is dict and 0 < len(files) <= 32)
    for path, digest in files.items():
        require(type(path) is str and not Path(path).is_absolute() and ".." not in Path(path).parts)
        sha(digest)
    if repository == "Strukturpiloten/boxferry":
        require(set(files) == BOX_FILES)
    else:
        records = set(files) - LENS_FILES
        require(LENS_FILES <= set(files) and len(records) == 4
                and all(RECORD.fullmatch(path) is not None for path in records))


def authorize(trusted: Path, box_pr: int, box_sha: str, lens_pr: int, lens_sha: str):
    """Authority comes only from this trusted-main manifest, never source receipts."""
    positive(box_pr)
    positive(lens_pr)
    sha(box_sha, SHA)
    sha(lens_sha, SHA)
    raw = read(trusted / MANIFEST, 128 * 1024)
    manifest = json.loads(raw, object_pairs_hook=no_duplicates)
    keys(manifest, ("schema", "policy", "authorizations"))
    require(type(manifest["schema"]) is int and manifest["schema"] == 1 and manifest["policy"] == POLICY)
    require(type(manifest["authorizations"]) is list and len(manifest["authorizations"]) == 1)
    selected, seen = [], set()
    for pair in manifest["authorizations"]:
        keys(pair, ("boxferry", "docker_lens"))
        box, lens = pair["boxferry"], pair["docker_lens"]
        entry(box, "Strukturpiloten/boxferry")
        entry(lens, "Strukturpiloten/docker-lens")
        identity = (box["pr"], box["revision"], lens["pr"], lens["revision"])
        require(identity not in seen)
        seen.add(identity)
        if identity == (box_pr, box_sha, lens_pr, lens_sha):
            selected.append(pair)
    require(len(selected) == 1)
    return selected[0], hashlib.sha256(raw).hexdigest()


def verify_source(root: Path, authorization) -> dict[str, bytes]:
    require(root.is_absolute() and root.resolve(strict=True) == root)
    require(git(root, "rev-parse", "HEAD") == authorization["revision"])
    require(git(root, "status", "--porcelain", "--untracked-files=all") == "")
    require(source_digest(root) == authorization["source_sha256"])
    require(hashlib.sha256(read(root / "Cargo.lock")).hexdigest() == authorization["lock_sha256"])
    files = {}
    for path, digest in authorization["files"].items():
        raw = read(root / path)
        require(hashlib.sha256(raw).hexdigest() == digest)
        files[path] = raw
    if authorization["repository"] == "Strukturpiloten/docker-lens":
        referenced = set(RECORD.findall(files["src/reviewed_catalog.rs"].decode()))
        # Only the active four-record declaration is consumed; historical records
        # outside the declaration are not arbitrary dynamic-read authorization.
        declarations = re.findall(r"(?ms)^const RECORDS: .*? = \[(.*?)^\];", files["src/reviewed_catalog.rs"].decode())
        require(len(declarations) == 1)
        active = set(RECORD.findall(declarations[0]))
        paths = {f"docs/evidence/reviewed/sha256/{digest}.json" for digest in active}
        require(len(active) == 4 and paths == set(files) - LENS_FILES and active <= referenced)
    return files


def copy_closure(files: dict[str, bytes], destination: Path) -> None:
    require(destination.is_absolute() and destination.parent.resolve(strict=True) == destination.parent)
    destination.mkdir(mode=0o700)
    for path, raw in files.items():
        target = destination / path
        target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        with target.open("xb") as stream:
            stream.write(raw)
        target.chmod(0o600)
        require(read(target) == raw)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--trusted-root", type=Path, required=True)
    parser.add_argument("--box-pr", type=int, required=True)
    parser.add_argument("--box-sha", required=True)
    parser.add_argument("--lens-pr", type=int, required=True)
    parser.add_argument("--lens-sha", required=True)
    args = parser.parse_args()
    authorize(args.trusted_root, args.box_pr, args.box_sha, args.lens_pr, args.lens_sha)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, UnicodeError, RecursionError, subprocess.SubprocessError):
        raise SystemExit("reviewed Docker consumer authorization rejected") from None
