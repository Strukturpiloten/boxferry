"""Build a held candidate and execute client probes in its exact native image.

This producer performs no publication or application/native-runtime qualification.
Two hosted lanes share one immutable seven-repository source resolution; local
execution requires real clean sibling checkouts and an explicit Podman backend.
Budgets are conservative qualification targets, not measured acceptance evidence.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shlex
import shutil
import signal
import stat
import sys
import tempfile
import time
import tomllib
import uuid
from collections.abc import Callable, Mapping
from dataclasses import asdict, dataclass
from enum import StrEnum
from pathlib import Path
from typing import Any, cast

from .contracts import ContractError, exact_sha
from .dependencies import fetch_metadata
from .operational import _json
from .runner import (
    AbortContext,
    OpaqueAbortContext,
    OpaqueMetrics,
    OpaqueObservation,
    OpaqueParseError,
    OpaqueReason,
    OpaqueRunError,
    OpaqueRunResult,
    RunCancelled,
    _argument_secrets,
    _redacted_argv,
    protected_values,
    redact,
    run,
)

OWNERS = frozenset({"boxferry", "compose-lens", "podman-lens", "quadlet-lens"})
SIBLINGS = (
    "compose-lens",
    "docker-lens",
    "kubernetes-lens",
    "podman-lens",
    "quadlet-lens",
    "boxferry-website",
)
ALL_REPOSITORIES = ("boxferry", *SIBLINGS)
ARCHITECTURES = {"x86_64": "amd64", "aarch64": "arm64"}
CLI_PROJECT = "automation/devcontainer"
CONFIG = ".devcontainer/devcontainer.json"
LOCK = ".devcontainer/devcontainer-lock.json"
GIB = 1024**3
BUILD_SECONDS, PROBE_SECONDS, CLEANUP_SECONDS = 3600, 300, 300
MAX_TREE_FILES, MAX_TREE_BYTES = 25000, 256 * 1024**2
MAX_REPORT = 64 * 1024
MAX_FAILURE_STREAM = 2048
_CAUSAL_LINE = re.compile(r"\b(error|failed|failure|fatal|denied|cannot|unable|not found)\b", re.I)


def _bounded_text(value: str, limit: int) -> str:
    return value.encode("utf-8")[:limit].decode("utf-8", "ignore")


def failure_evidence(value: str) -> dict[str, object]:
    """Keep initial context, early causal lines and final context within a byte bound."""
    causal = "\n".join(
        _bounded_text(line[max(0, match.start() - 64) :], 384)
        for line in value.splitlines()
        if (match := _CAUSAL_LINE.search(line))
    )
    if len(value.encode("utf-8")) <= MAX_FAILURE_STREAM:
        excerpt = value
    else:
        excerpt = (
            _bounded_text(value, 512)
            + "\n[... causal lines ...]\n"
            + _bounded_text(causal, 896)
            + "\n[... final context ...]\n"
            + value.encode("utf-8")[-512:].decode("utf-8", "ignore")
        )
    return {
        "excerpt": excerpt,
        "truncated": excerpt != value,
        "causal": _bounded_text(causal or value, 768),
    }


# Observation is disabled until a fresh exact-owned opaque capture and source review.
class LivePidSchema(StrEnum):
    PODMAN_STATE_FIELDS = "podman-state-pids"


LIVE_PID_SCHEMA: LivePidSchema | None = None
LIVE_PROCESS_OBSERVER_PROGRAM = r"""
import json, os, re, select, sys


class ObserveFailure(Exception):
    def __init__(self, code):
        self.code = code


def number(value, maximum=2147483647, minimum=1):
    if type(value) is not int or not minimum <= value <= maximum:
        raise ObserveFailure(2)
    return value


def namespace(value, kind):
    if (
        type(value) is not str
        or len(value.encode()) > 64
        or re.fullmatch(kind + r":\[[0-9]{1,20}\]", value) is None
    ):
        raise ObserveFailure(2)
    return value


def read_file(directory, name, limit):
    fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=directory)
    try:
        data = os.read(fd, limit + 1)
        if len(data) > limit:
            raise ObserveFailure(7)
        return data
    finally:
        os.close(fd)


def ticks(directory, pid):
    data = read_file(directory, "stat", 4096)
    try:
        first, rest = data.rsplit(b")", 1)
        native_pid, _comm = first.split(b" (", 1)
        fields = rest.split()
        value = fields[19]
        if (
            native_pid != str(pid).encode()
            or not value.isdigit()
            or len(value) > 20
            or int(value) <= 0
        ):
            raise ObserveFailure(7)
        return value.decode("ascii")
    except (ValueError, IndexError):
        raise ObserveFailure(7) from None


def status(directory):
    data = read_file(directory, "status", 16384)
    result = {}
    for line in data.splitlines():
        name, sep, value = line.partition(b":")
        if name not in (b"Uid", b"NSpid"):
            continue
        if not sep or name in result:
            raise ObserveFailure(7)
        fields = value.split()
        if (
            not fields
            or len(fields) > 32
            or any(not item.isdigit() or len(item) > 10 for item in fields)
        ):
            raise ObserveFailure(7)
        result[name] = [int(item) for item in fields]
    if set(result) != {b"Uid", b"NSpid"} or len(result[b"Uid"]) != 4:
        raise ObserveFailure(7)
    if any(not 0 <= item <= 4294967295 for item in result[b"Uid"]):
        raise ObserveFailure(7)
    if any(not 1 <= item <= 2147483647 for item in result[b"NSpid"]):
        raise ObserveFailure(7)
    return result[b"Uid"], result[b"NSpid"]


def links(directory):
    result = []
    for name, kind in (("ns/pid", "pid"), ("ns/mnt", "mnt")):
        value = os.readlink(name, dir_fd=directory)
        if (
            len(value.encode()) > 128
            or re.fullmatch(kind + r":\[[0-9]{1,20}\]", value) is None
        ):
            raise ObserveFailure(7)
        result.append(value)
    return result


def sample(proc, pid):
    directory = os.open(
        str(pid),
        os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
        dir_fd=proc,
    )
    try:
        before = ticks(directory, pid)
        uids, nspid = status(directory)
        pidns, mntns = links(directory)
        if ticks(directory, pid) != before:
            raise ObserveFailure(4)
        return {
            "pid": pid,
            "uids": uids,
            "nspid": nspid,
            "pid_namespace": pidns,
            "mount_namespace": mntns,
            "start_ticks": before,
        }
    finally:
        os.close(directory)


def observe(request):
    keys = {
        "schema",
        "container_pid",
        "conmon_pid",
        "expected_container_uid",
        "pid_namespace",
        "mount_namespace",
        "host_pid_namespace",
        "host_mount_namespace",
    }
    if (
        type(request) is not dict
        or set(request) != keys
        or type(request["schema"]) is not int
        or request["schema"] != 1
    ):
        raise ObserveFailure(2)
    own = os.getpid()
    container = number(request["container_pid"], minimum=2)
    conmon = number(request["conmon_pid"], minimum=2)
    uid = number(request["expected_container_uid"], maximum=4294967295)
    if container == conmon or own in (container, conmon):
        raise ObserveFailure(2)
    pidns = namespace(request["pid_namespace"], "pid")
    mntns = namespace(request["mount_namespace"], "mnt")
    hostpid = namespace(request["host_pid_namespace"], "pid")
    hostmnt = namespace(request["host_mount_namespace"], "mnt")
    if pidns == hostpid or mntns == hostmnt:
        raise ObserveFailure(5)
    if os.getuid() != 0 or os.geteuid() != 0 or os.getresuid() != (0, 0, 0):
        raise ObserveFailure(6)
    proc = os.open("/proc", os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    held = []
    try:
        me = sample(proc, own)
        if me["uids"] != [0] * 4:
            raise ObserveFailure(6)
        if (
            me["nspid"] != [own]
            or me["pid_namespace"] != pidns
            or me["mount_namespace"] != mntns
        ):
            raise ObserveFailure(5)
        for pid in (container, conmon):
            fd = os.pidfd_open(pid, 0)
            held.append(fd)
            os.set_inheritable(fd, False)
        if select.select(held, [], [], 0)[0]:
            raise ObserveFailure(4)
        records = []
        for role, pid, fd in (
            ("container-init", container, held[0]),
            ("conmon", conmon, held[1]),
        ):
            if select.select([fd], [], [], 0)[0]:
                raise ObserveFailure(4)
            record = sample(proc, pid)
            if select.select([fd], [], [], 0)[0]:
                raise ObserveFailure(4)
            if role == "conmon":
                if record["uids"] != [0] * 4:
                    raise ObserveFailure(6)
                if record["nspid"] != [pid] or record["pid_namespace"] != pidns:
                    raise ObserveFailure(5)
            else:
                if record["uids"] != [uid] * 4:
                    raise ObserveFailure(6)
                if record["nspid"] != [pid, 1] or record["pid_namespace"] in (
                    pidns,
                    hostpid,
                ):
                    raise ObserveFailure(5)
            record.update(role=role, pidfd_alive_before=True, pidfd_alive_after=True)
            records.append(record)
        if select.select(held, [], [], 0)[0]:
            raise ObserveFailure(4)
        return {
            "schema": 1,
            "status": "observed",
            "observer": {
                "pid": own,
                "uids": [0] * 4,
                "pid_namespace": pidns,
                "mount_namespace": mntns,
                "proc_pid_view": "lifetime",
            },
            "processes": records,
        }
    finally:
        for fd in held:
            os.close(fd)
        os.close(proc)


def main():
    if len(sys.argv) != 2 or len(sys.argv[1].encode()) > 2048:
        raise ObserveFailure(2)
    try:
        request = json.loads(sys.argv[1])
    except (ValueError, TypeError):
        raise ObserveFailure(2) from None
    value = observe(request)
    encoded = json.dumps(value, sort_keys=True, separators=(",", ":")).encode() + b"\n"
    if len(encoded) > 8192:
        raise ObserveFailure(7)
    if os.write(1, encoded) != len(encoded):
        raise ObserveFailure(7)


if __name__ == "__main__":
    code = 0
    try:
        main()
    except ObserveFailure as error:
        code = error.code
    except (FileNotFoundError, ProcessLookupError, PermissionError):
        code = 3
    except BaseException:
        code = 7
    raise SystemExit(code)
"""
LIVE_PROCESS_OBSERVER_SHA256 = hashlib.sha256(LIVE_PROCESS_OBSERVER_PROGRAM.encode()).hexdigest()


PROBES: tuple[tuple[str, ...], ...] = (
    ("actionlint", "--version"),
    ("cargo", "--version"),
    ("cargo-clippy", "--version"),
    ("cargo-deny", "--version"),
    ("cargo-llvm-cov", "--version"),
    ("cargo-semver-checks", "--version"),
    ("curl", "--version"),
    ("gh", "--version"),
    ("git", "--version"),
    ("hadolint", "--version"),
    ("helm", "version", "--short"),
    ("jq", "--version"),
    ("kind", "version"),
    ("kubectl", "version", "--client=true"),
    ("kustomize", "version"),
    ("lychee", "--version"),
    ("markdownlint-cli2", "--help"),
    ("node", "--version"),
    ("npm", "--version"),
    ("prettier", "--version"),
    ("rustc", "--version"),
    ("rustfmt", "--version"),
    ("rustup", "--version"),
    ("shellcheck", "--version"),
    ("shfmt", "--version"),
    ("tombi", "--version"),
    ("uv", "--version"),
    ("ruff", "--version"),
    ("ty", "--version"),
    ("zizmor", "--version"),
)


class ValidationError(ContractError):
    """Candidate, native backend, resource or cleanup evidence failed."""


def manifest_digest(document: Mapping[str, object]) -> str:
    return hashlib.sha256(
        json.dumps(document, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()


def parse_sources(document: object, repository: str, candidate: str) -> dict[str, str]:
    if (
        not isinstance(document, dict)
        or set(document) != {"schema", "repositories"}
        or type(document["schema"]) is not int
        or document["schema"] != 1
    ):
        raise ValidationError("workspace source manifest has an unsupported schema")
    sources = document["repositories"]
    expected = set(ALL_REPOSITORIES) if repository == "boxferry" else {repository}
    if not isinstance(sources, dict) or set(sources) != expected:
        raise ValidationError(
            "workspace source manifest must identify every required real checkout"
        )
    result = {name: exact_sha(value, "workspace revision") for name, value in sources.items()}
    if result[repository] != candidate:
        raise ValidationError("workspace source manifest differs from the tested candidate")
    return result


def resolve_sources(
    repository: str, candidate: str, request: Callable[[str], bytes] = fetch_metadata
) -> dict[str, object]:
    """Read each permitted public default branch once; never resolve inside a worker."""
    if repository not in OWNERS:
        raise ValidationError("repository does not own a Dev Container")
    revisions = {repository: exact_sha(candidate, "candidate SHA")}
    if repository == "boxferry":
        for name in SIBLINGS:
            metadata = _json(request(f"https://api.github.com/repos/Strukturpiloten/{name}"))
            branch = metadata.get("default_branch")
            if not isinstance(branch, str) or re.fullmatch(r"[A-Za-z0-9_.-]+", branch) is None:
                raise ValidationError("public repository has an unsupported default branch")
            commit = _json(
                request(f"https://api.github.com/repos/Strukturpiloten/{name}/commits/{branch}")
            )
            revisions[name] = exact_sha(commit.get("sha"), "resolved sibling SHA")
    return {"schema": 1, "repositories": revisions}


def environment(private: Path) -> dict[str, str]:
    """Finite trusted host environment: no tokens, personal configs or runtime sockets."""
    return {
        "PATH": "/usr/local/bin:/usr/bin:/bin",
        "HOME": str(private / "home"),
        "DOCKER_CONFIG": str(private / "docker-config"),
        "XDG_CONFIG_HOME": str(private / "config"),
        "XDG_CACHE_HOME": str(private / "cache"),
        "XDG_RUNTIME_DIR": str(private / "runtime"),
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": "/dev/null",
        "GIT_TERMINAL_PROMPT": "0",
        "GIT_ASKPASS": "/bin/false",
        "LC_ALL": "C.UTF-8",
        "CI": "true",
    }


@dataclass
class Budget:
    path: Path
    initial_free: int
    deadline: float
    last_checked: float = 0
    check_disk: bool = True

    @classmethod
    def start(cls, path: Path, seconds: int) -> Budget:
        free = shutil.disk_usage(path).free
        if free < 12 * GIB:
            raise ValidationError("image qualification requires at least 12 GiB free initially")
        return cls(path, free, time.monotonic() + seconds)

    def check(self) -> None:
        now = time.monotonic()
        if now > self.deadline:
            raise ValidationError("image qualification exhausted its stage deadline")
        if self.check_disk and now - self.last_checked >= 1:
            self.last_checked = now
            free = shutil.disk_usage(self.path).free
            # Counts all filesystem growth conservatively, including external users;
            # never treats shared daemon layers as free run-owned disk capacity.
            if free < 2 * GIB or self.initial_free - free > 10 * GIB:
                raise ValidationError("image qualification exceeded disk growth or reserve budget")


@dataclass(frozen=True)
class LiveHeldIdentity:
    container_id: str
    image_id: str
    run_nonce: str
    container_uid: int
    native_binary_sha256: str
    native_version: str


@dataclass(frozen=True)
class LiveInspection:
    held: LiveHeldIdentity
    running: bool
    status: str
    container_pid: int | None
    conmon_pid: int | None
    container_pid_kind: str
    conmon_pid_kind: str
    started_at: str

    def schema_capture(self) -> dict[str, object]:
        return {
            "schema": 1,
            "status": "unsupported-selector-schema-capture-only",
            "container_id": self.held.container_id,
            "image_id": self.held.image_id,
            "run_nonce": self.held.run_nonce,
            "running": self.running,
            "state_pid_kind": self.container_pid_kind,
            "candidate_state_conmon_pid_kind": self.conmon_pid_kind,
            "state_pid_candidate": self.container_pid,
            "candidate_state_conmon_pid_candidate": self.conmon_pid,
            "started_at_valid": bool(self.started_at),
            "native_binary_sha256": self.held.native_binary_sha256,
            "native_version": self.held.native_version,
            "expected_container_uid": self.held.container_uid,
            "selector_approved": False,
            "observer_executed": False,
        }


@dataclass(frozen=True)
class LiveProcessRequest:
    container_pid: int
    conmon_pid: int
    expected_container_uid: int
    pid_namespace: str
    mount_namespace: str
    host_pid_namespace: str
    host_mount_namespace: str
    schema: int = 1


@dataclass(frozen=True)
class LiveKnownProcess:
    role: str
    pid: int
    uids: tuple[int, ...]
    nspid: tuple[int, ...]
    start_ticks: str
    pid_namespace: str
    mount_namespace: str
    pidfd_alive_before: bool = True
    pidfd_alive_after: bool = True


@dataclass(frozen=True)
class LiveProcessSample:
    observer_pid: int
    pid_namespace: str
    mount_namespace: str
    processes: tuple[LiveKnownProcess, ...]


def _live_invalid(reason: OpaqueReason = OpaqueReason.INVALID_NATIVE_SCHEMA) -> None:
    raise OpaqueParseError(reason)


def _live_dict(
    value: object, reason: OpaqueReason = OpaqueReason.INVALID_NATIVE_SCHEMA
) -> dict[str, Any]:
    if type(value) is not dict:
        raise OpaqueParseError(reason)
    return cast(dict[str, Any], value)


def _live_uint(value: object, *, maximum: int = 2147483647, minimum: int = 0) -> int:
    if type(value) is not int or not minimum <= value <= maximum:
        raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
    return value


def _live_ns(value: object, kind: str) -> str:
    if type(value) is not str or re.fullmatch(kind + r":\[[0-9]{1,20}\]", value) is None:
        raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
    return value


def _live_json(content: bytes) -> object:
    def pairs(items: list[tuple[str, object]]) -> dict[str, object]:
        result: dict[str, object] = {}
        for name, value in items:
            if name in result:
                raise OpaqueParseError(OpaqueReason.INVALID_JSON)
            result[name] = value
        return result

    def constant(_value: str) -> object:
        raise OpaqueParseError(OpaqueReason.INVALID_JSON)

    try:
        result = json.loads(content, object_pairs_hook=pairs, parse_constant=constant)
    except (ValueError, UnicodeError):
        raise OpaqueParseError(OpaqueReason.INVALID_JSON) from None
    pending = [(result, 0)]
    count = 0
    while pending:
        value, depth = pending.pop()
        count += 1
        if count > 4096 or depth > 16:
            raise OpaqueParseError(OpaqueReason.INVALID_JSON)
        if isinstance(value, dict):
            pending.extend((item, depth + 1) for item in value.values())
        elif isinstance(value, list):
            pending.extend((item, depth + 1) for item in value)
    return result


def _live_held(held: LiveHeldIdentity) -> None:
    if (
        type(held) is not LiveHeldIdentity
        or re.fullmatch(r"[0-9a-f]{64}", held.container_id) is None
        or re.fullmatch(r"sha256:[0-9a-f]{64}", held.image_id) is None
        or re.fullmatch(r"[0-9a-f]{32}", held.run_nonce) is None
        or re.fullmatch(r"[0-9a-f]{64}", held.native_binary_sha256) is None
        or re.fullmatch(
            r"podman version [0-9]+\.[0-9]+\.[0-9]+(?:[-+][A-Za-z0-9.-]+)?", held.native_version
        )
        is None
    ):
        raise OpaqueParseError(OpaqueReason.INVALID_NATIVE_SCHEMA)
    _live_uint(held.container_uid, minimum=1, maximum=4294967295)


def _live_pid_candidate(state: Mapping[str, object], name: str) -> tuple[int | None, str]:
    if name not in state:
        return None, "missing"
    value = state[name]
    if type(value) is int:
        if 0 <= value <= 2147483647:
            return value, "integer"
        return None, "invalid-integer"
    kinds = {
        bool: "boolean",
        str: "string",
        float: "float",
        list: "array",
        dict: "object",
        type(None): "null",
    }
    return None, kinds.get(type(value), "other")


def _live_timestamp(value: object) -> str:
    if (
        type(value) is not str
        or len(value) > 40
        or re.fullmatch(
            r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}"
            r"(?:\.[0-9]{1,9})?(?:Z|[+-][0-9]{2}:[0-9]{2})",
            value,
        )
        is None
        or value.startswith("0001-")
    ):
        raise OpaqueParseError(OpaqueReason.INVALID_NATIVE_SCHEMA)
    from datetime import datetime

    try:
        datetime.fromisoformat(value)
    except ValueError:
        raise OpaqueParseError(OpaqueReason.INVALID_NATIVE_SCHEMA) from None
    return value


def _parse_live_inspect(content: bytes, held: LiveHeldIdentity) -> LiveInspection:
    _live_held(held)
    value = _live_json(content)
    if type(value) is not list or len(value) != 1:
        raise OpaqueParseError(OpaqueReason.INVALID_NATIVE_SCHEMA)
    record = _live_dict(value[0])
    config, state = _live_dict(record.get("Config")), _live_dict(record.get("State"))
    labels = _live_dict(config.get("Labels"))
    image = record.get("Image")
    if type(image) is str and re.fullmatch(r"[0-9a-f]{64}", image):
        image = "sha256:" + image
    if (
        record.get("Id") != held.container_id
        or image != held.image_id
        or labels.get("io.boxferry.devcontainer.run") != held.run_nonce
        or type(state.get("Running")) is not bool
        or state.get("Status") not in ("running", "created", "configured")
    ):
        raise OpaqueParseError(OpaqueReason.INVALID_NATIVE_SCHEMA)
    container, container_kind = _live_pid_candidate(state, "Pid")
    conmon, conmon_kind = _live_pid_candidate(state, "ConmonPid")
    running = state["Running"]
    started = _live_timestamp(state.get("StartedAt")) if running else ""
    if running and state["Status"] != "running":
        raise OpaqueParseError(OpaqueReason.INVALID_NATIVE_SCHEMA)
    if not running and (
        state["Status"] not in ("created", "configured") or container != 0 or conmon != 0
    ):
        raise OpaqueParseError(OpaqueReason.INVALID_NATIVE_SCHEMA)
    if LIVE_PID_SCHEMA is not None:
        if LIVE_PID_SCHEMA is not LivePidSchema.PODMAN_STATE_FIELDS:
            raise OpaqueParseError(OpaqueReason.INVALID_NATIVE_SCHEMA)
        if running and (
            container is None
            or conmon is None
            or container < 2
            or conmon < 2
            or container == conmon
        ):
            raise OpaqueParseError(OpaqueReason.INVALID_NATIVE_SCHEMA)
    return LiveInspection(
        held, running, state["Status"], container, conmon, container_kind, conmon_kind, started
    )


def _live_request(inspection: LiveInspection, context: Mapping[str, object]) -> LiveProcessRequest:
    if (
        not inspection.running
        or LIVE_PID_SCHEMA is not LivePidSchema.PODMAN_STATE_FIELDS
        or inspection.container_pid is None
        or inspection.conmon_pid is None
    ):
        raise OpaqueParseError(OpaqueReason.INVALID_NATIVE_SCHEMA)
    request = LiveProcessRequest(
        inspection.container_pid,
        inspection.conmon_pid,
        inspection.held.container_uid,
        _live_ns(context.get("pid_namespace"), "pid"),
        _live_ns(context.get("mount_namespace"), "mnt"),
        _live_ns(context.get("host_pid_namespace"), "pid"),
        _live_ns(context.get("host_mount_namespace"), "mnt"),
    )
    _validate_live_request(request)
    return request


def _validate_live_request(request: LiveProcessRequest) -> None:
    if (
        type(request) is not LiveProcessRequest
        or type(request.schema) is not int
        or request.schema != 1
    ):
        raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
    for pid in (request.container_pid, request.conmon_pid):
        _live_uint(pid, minimum=2)
    if request.container_pid == request.conmon_pid:
        raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
    _live_uint(request.expected_container_uid, minimum=1, maximum=4294967295)
    for prefix in ("", "host_"):
        _live_ns(getattr(request, prefix + "pid_namespace"), "pid")
        _live_ns(getattr(request, prefix + "mount_namespace"), "mnt")
    if (
        request.pid_namespace == request.host_pid_namespace
        or request.mount_namespace == request.host_mount_namespace
    ):
        raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)


def _parse_live_processes(content: bytes, request: LiveProcessRequest) -> LiveProcessSample:
    _validate_live_request(request)
    value = _live_dict(_live_json(content), OpaqueReason.INVALID_PROCESS_OBSERVATION)
    if (
        set(value) != {"schema", "status", "observer", "processes"}
        or type(value["schema"]) is not int
        or value["schema"] != 1
        or value["status"] != "observed"
    ):
        raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
    observer = _live_dict(value["observer"], OpaqueReason.INVALID_PROCESS_OBSERVATION)
    if (
        set(observer) != {"pid", "uids", "pid_namespace", "mount_namespace", "proc_pid_view"}
        or observer["uids"] != [0, 0, 0, 0]
        or observer["proc_pid_view"] != "lifetime"
        or observer["pid_namespace"] != request.pid_namespace
        or observer["mount_namespace"] != request.mount_namespace
    ):
        raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
    if type(observer["uids"]) is not list or len(observer["uids"]) != 4:
        raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
    for uid in observer["uids"]:
        _live_uint(uid, maximum=4294967295)
    own = _live_uint(observer["pid"], minimum=2)
    records = value["processes"]
    if type(records) is not list or len(records) != 2:
        raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
    result = []
    roles = set()
    for item in records:
        record = _live_dict(item, OpaqueReason.INVALID_PROCESS_OBSERVATION)
        if set(record) != {
            "role",
            "pid",
            "uids",
            "nspid",
            "start_ticks",
            "pid_namespace",
            "mount_namespace",
            "pidfd_alive_before",
            "pidfd_alive_after",
        }:
            raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
        role = record["role"]
        if role not in ("container-init", "conmon") or role in roles:
            raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
        roles.add(role)
        pid = _live_uint(record["pid"], minimum=2)
        expected = request.container_pid if role == "container-init" else request.conmon_pid
        if pid != expected or own in (request.container_pid, request.conmon_pid):
            raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
        uids, nspid = record["uids"], record["nspid"]
        if (
            type(uids) is not list
            or len(uids) != 4
            or type(nspid) is not list
            or not 1 <= len(nspid) <= 32
        ):
            raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
        for uid in uids:
            _live_uint(uid, maximum=4294967295)
        for native_pid in nspid:
            _live_uint(native_pid, minimum=1)
        expected_uid = request.expected_container_uid if role == "container-init" else 0
        if uids != [expected_uid] * 4 or nspid != ([pid, 1] if role == "container-init" else [pid]):
            raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
        pidns, mntns = (
            _live_ns(record["pid_namespace"], "pid"),
            _live_ns(record["mount_namespace"], "mnt"),
        )
        if (role == "conmon" and pidns != request.pid_namespace) or (
            role == "container-init"
            and pidns in (request.pid_namespace, request.host_pid_namespace)
        ):
            raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
        ticks = record["start_ticks"]
        if (
            type(ticks) is not str
            or re.fullmatch(r"[0-9]{1,20}", ticks) is None
            or int(ticks) <= 0
            or record["pidfd_alive_before"] is not True
            or record["pidfd_alive_after"] is not True
        ):
            raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
        result.append(LiveKnownProcess(role, pid, tuple(uids), tuple(nspid), ticks, pidns, mntns))
    return LiveProcessSample(own, request.pid_namespace, request.mount_namespace, tuple(result))


class Commands:
    _last_live_metrics: OpaqueMetrics | None = None
    lifetime_context: dict[str, Any] | None = None
    lifetime_report: dict[str, object] | None = None
    evidence_paths: tuple[Path, ...] = ()

    def __init__(self, private: Path) -> None:
        self.private = private
        self.env = environment(private)
        for key in (
            "HOME",
            "DOCKER_CONFIG",
            "XDG_CONFIG_HOME",
            "XDG_CACHE_HOME",
            "XDG_RUNTIME_DIR",
        ):
            Path(self.env[key]).mkdir(mode=0o700, parents=True, exist_ok=True)
        self.budget: Budget | None = None
        self.outcomes: list[dict[str, object]] = []

    def call(
        self,
        argv: tuple[str, ...],
        stage: str,
        timeout: int = 30,
        *,
        cwd: Path | None = None,
        while_running: Callable[[], None] | None = None,
        max_output_bytes: int = 2_000_000,
    ) -> str:
        started = time.monotonic()
        hidden = (*protected_values(os.environ), *_argument_secrets(argv))
        home = os.environ.get("HOME", "")
        evidence_hidden = (
            *hidden,
            str(self.private.resolve()),
            str((cwd or self.private).resolve()),
            *(str(path.resolve()) for path in self.evidence_paths),
            *((home,) if home.startswith("/") and home != "/" else ()),
        )
        safe = shlex.join(_redacted_argv(argv, evidence_hidden, cwd or self.private))
        outcome: dict[str, object] = {
            "stage": stage,
            "command": safe[:2048],
            "command_truncated": len(safe) > 2048,
            "status": "infrastructure",
            "cause": "command precheck not completed",
            "next_action": (
                "Correct the named command or resource cause; rerun the same candidate."
            ),
        }
        self.outcomes.append(outcome)
        try:
            if self.budget is not None:
                self.budget.check()
            abort_options: dict[str, Any] = {}
            if self.lifetime_context is not None:
                from . import devcontainer_lifetime

                context = self.lifetime_context
                abort_options["on_abort"] = lambda failure: devcontainer_lifetime.notify_abort(
                    context, failure, self.lifetime_report or {}
                )
            if while_running is not None and stage != "tooling-workspace-probe":
                raise ValidationError("live monitor requires the held attached probe")

            def combined_monitor() -> None:
                if self.budget is not None:
                    self.budget.check()
                if while_running is not None:
                    while_running()

            result = run(
                argv,
                cwd=cwd or self.private,
                stage=stage,
                timeout_seconds=timeout,
                environment=self.env,
                secrets=hidden,
                monitor=combined_monitor
                if while_running is not None
                else self.budget.check
                if self.budget
                else None,
                max_output_bytes=max_output_bytes,
                **abort_options,
            )
            outcome.update(status=result.status, cause=result.diagnostic.cause)
            if result.abort_hook_failure:
                outcome["abort_hook_failure"] = result.abort_hook_failure
            if result.status != "success":
                evidence = {
                    name: failure_evidence(redact(value, evidence_hidden))
                    for name, value in (("stderr", result.stderr), ("stdout", result.stdout))
                }
                outcome["failure_evidence"] = evidence
                detail = "; ".join(
                    f"{name}: {item['causal']}" for name, item in evidence.items() if item["causal"]
                )
                raise ValidationError(f"{result.diagnostic.cause}; {detail}")
            return result.stdout
        except (OSError, ValueError, RunCancelled, KeyboardInterrupt) as error:
            cause = redact(str(error), evidence_hidden)[:2048]
            if isinstance(error, (RunCancelled, KeyboardInterrupt)):
                outcome["status"] = "cancelled"
            outcome["cause"] = cause
            if secondary := getattr(error, "abort_hook_failure", ""):
                outcome["abort_hook_failure"] = secondary
            if isinstance(error, RunCancelled):
                raise RunCancelled(f"{stage}: {cause}") from None
            if isinstance(error, KeyboardInterrupt):
                raise KeyboardInterrupt(f"{stage}: {cause}") from None
            raise ValidationError(f"{stage}: {cause}") from None
        finally:
            outcome["duration_seconds"] = time.monotonic() - started

    def _live_inspect(self, held: LiveHeldIdentity, phase: OpaqueObservation) -> LiveInspection:
        if phase not in (OpaqueObservation.INSPECT_BEFORE, OpaqueObservation.INSPECT_AFTER):
            raise OpaqueRunError(OpaqueReason.INVOCATION_UNAVAILABLE)
        result = self._invoke_live_observation(phase, held=held)
        assert isinstance(result, LiveInspection)
        return result

    def _live_processes(self, request: LiveProcessRequest) -> LiveProcessSample:
        result = self._invoke_live_observation(OpaqueObservation.PROCESSES, request=request)
        assert isinstance(result, LiveProcessSample)
        return result

    def _live_stop(
        self, kind: OpaqueObservation, reason: OpaqueReason, current: OpaqueMetrics | None = None
    ) -> None:
        from dataclasses import replace

        if (
            current is None
            and self._last_live_metrics is not None
            and self._last_live_metrics.mode is kind
        ):
            current = self._last_live_metrics
        if current is None:
            current = OpaqueMetrics(
                kind,
                reason,
                "cancelled" if reason is OpaqueReason.CANCELLED else "infrastructure",
                "",
                None,
                0,
                0,
                hashlib.sha256(b"").hexdigest(),
                hashlib.sha256(b"").hexdigest(),
            )
        # A completed sample is provisional until all producer checks finish.
        # A previous failure, including its first cancellation, remains primary.
        if current.status != "success":
            reason = current.reason
            safe = current
        else:
            safe = replace(
                current,
                mode=kind,
                reason=reason,
                status="cancelled" if reason is OpaqueReason.CANCELLED else "infrastructure",
            )
        self._last_live_metrics = safe
        if self.outcomes and self.outcomes[-1].get("stage") == kind.stage:
            self.outcomes[-1].update(
                status=safe.status, cause=safe.reason.value, opaque_observation=asdict(safe)
            )
        if self.lifetime_context is not None and not self.lifetime_context.get("abort_notified"):
            from .devcontainer_lifetime import notify_abort

            failed = False
            try:
                notify_abort(
                    self.lifetime_context,
                    OpaqueAbortContext(kind.stage, safe.status, reason.value, safe),
                    self.lifetime_report or {},
                )
            except BaseException:
                failed = True
            if failed and self.outcomes:
                self.outcomes[-1]["abort_hook_failure"] = OpaqueReason.ABORT_HANDOFF_FAILED.value
        raise OpaqueRunError(reason, safe) from None

    def _invoke_live_observation(
        self,
        kind: OpaqueObservation,
        *,
        held: LiveHeldIdentity | None = None,
        request: LiveProcessRequest | None = None,
    ) -> LiveInspection | LiveProcessSample:
        """Closed private producer; real argv, streams and parser exceptions never become logs."""
        if type(kind) is not OpaqueObservation:
            raise OpaqueRunError(OpaqueReason.INVOCATION_UNAVAILABLE)
        outcome: dict[str, object] = {
            "stage": kind.stage,
            "command": "opaque-observation " + kind.value,
            "status": "infrastructure",
            "cause": OpaqueReason.INVOCATION_UNAVAILABLE.value,
            "next_action": "Review the fixed observation reason and exact owned schema.",
        }
        self.outcomes.append(outcome)
        self._last_live_metrics = None
        holder: list[LiveInspection | LiveProcessSample] = []
        reason = OpaqueReason.INVOCATION_UNAVAILABLE
        status = "infrastructure"
        current: OpaqueMetrics | None = None
        cancelled, interrupted = False, False
        try:
            if (
                self.lifetime_context is None
                or self.lifetime_context.get("abort_notified")
                or self.lifetime_context.get("private") != str(self.private)
            ):
                raise OpaqueRunError(reason)
            if self.budget is not None:
                self.budget.check()
            if kind in (OpaqueObservation.INSPECT_BEFORE, OpaqueObservation.INSPECT_AFTER):
                if held is None or request is not None:
                    raise OpaqueRunError(reason)
                _live_held(held)
                argv = (str(self.private / "podman"), "container", "inspect", held.container_id)
            else:
                if LIVE_PID_SCHEMA is not LivePidSchema.PODMAN_STATE_FIELDS:
                    raise OpaqueParseError(OpaqueReason.INVALID_NATIVE_SCHEMA)
                if request is None or held is not None:
                    raise OpaqueRunError(reason)
                _validate_live_request(request)
                encoded = json.dumps(asdict(request), sort_keys=True, separators=(",", ":"))
                if len(encoded.encode()) > 2048:
                    raise OpaqueRunError(reason)
                privilege, clean = _rootful_bridge(self.private)
                argv = (
                    *privilege,
                    "2s",
                    *clean,
                    "/usr/bin/python3",
                    "-I",
                    "-S",
                    "-c",
                    LIVE_PROCESS_OBSERVER_PROGRAM,
                    encoded,
                )
            outcome["argv_sha256"] = hashlib.sha256(
                json.dumps(argv, ensure_ascii=True, separators=(",", ":")).encode("ascii")
            ).hexdigest()
            if kind is OpaqueObservation.PROCESSES:
                outcome["program_sha256"] = LIVE_PROCESS_OBSERVER_SHA256

            def consume(content: bytes) -> None:
                # Pure bounded parsing only. Stage/disk checks remain outside the consumer.
                if kind is OpaqueObservation.PROCESSES:
                    assert request is not None
                    value: LiveInspection | LiveProcessSample = _parse_live_processes(
                        content, request
                    )
                else:
                    assert held is not None
                    value = _parse_live_inspect(content, held)
                holder.append(value)

            def abort(failure: AbortContext) -> None:
                assert self.lifetime_context is not None
                from .devcontainer_lifetime import notify_abort

                notify_abort(self.lifetime_context, failure, self.lifetime_report or {})

            result = run(
                argv,
                cwd=self.private,
                stage=kind.stage,
                timeout_seconds=kind.timeout,
                max_output_bytes=kind.output_cap,
                environment=self.env,
                monitor=self.budget.check if self.budget else None,
                on_abort=abort,
                opaque_observation=kind,
                on_private_stdout=consume,
            )
            if not isinstance(result, OpaqueRunResult):
                raise OpaqueRunError(reason)
            current = result.metrics
            self._last_live_metrics = current
            reason, status = current.reason, result.status
            outcome.update(status=status, cause=reason.value, opaque_observation=asdict(current))
            if result.abort_hook_failure:
                outcome["abort_hook_failure"] = result.abort_hook_failure
            if status != "success" or current.cleanup_failed or len(holder) != 1:
                raise OpaqueRunError(reason, current)
            if self.budget is not None:
                self.budget.check()
            return holder[0]
        except BaseException as error:
            if type(error) in (OpaqueRunError, OpaqueParseError):
                assert isinstance(error, OpaqueRunError)
                reason = error.reason
                current = error.metrics or current
            elif isinstance(error, RunCancelled):
                reason, status, cancelled = OpaqueReason.CANCELLED, "cancelled", True
            elif isinstance(error, KeyboardInterrupt):
                reason, status, interrupted = OpaqueReason.CANCELLED, "cancelled", True
            else:
                reason = OpaqueReason.INVOCATION_UNAVAILABLE
            holder.clear()
            outcome.update(
                status=status if status != "success" else "infrastructure", cause=reason.value
            )
        # Outside the original except block: no original exception, attrs or chain survive.
        if cancelled or interrupted:
            try:
                self._live_stop(kind, reason, current)
            except OpaqueRunError:
                pass
            if interrupted:
                raise KeyboardInterrupt(OpaqueReason.CANCELLED.value) from None
            raise RunCancelled(OpaqueReason.CANCELLED.value) from None
        self._live_stop(kind, reason, current)
        raise AssertionError("unreachable")

    def git(self, root: Path, *argv: str) -> str:
        return self.call(
            (
                "git",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "http.followRedirects=false",
                "-C",
                str(root),
                *argv,
            ),
            "source",
            120,
        )


class LiveProcessObservation:
    """One bounded known-process sandwich on the existing attached probe monitor."""

    def __init__(self, commands: Commands, held: LiveHeldIdentity) -> None:
        self.commands, self.held = commands, held
        self.first_tick: float | None = None
        self.last_attempt: float | None = None
        self.attempts = 0
        self.completed: dict[str, object] | None = None

    def tick(self) -> None:
        reason, kind, current = None, OpaqueObservation.INSPECT_BEFORE, None
        cancelled, interrupted = False, False
        try:
            self._tick()
        except BaseException as error:
            self.completed = None
            if type(error) in (OpaqueRunError, OpaqueParseError):
                assert isinstance(error, OpaqueRunError)
                reason, current = error.reason, error.metrics
                if current is not None:
                    kind = current.mode
            elif isinstance(error, RunCancelled):
                reason, cancelled = OpaqueReason.CANCELLED, True
            elif isinstance(error, KeyboardInterrupt):
                reason, interrupted = OpaqueReason.CANCELLED, True
            else:
                reason = OpaqueReason.INVOCATION_UNAVAILABLE
        if reason is not None:
            if cancelled or interrupted:
                try:
                    self.commands._live_stop(kind, reason, current)
                except OpaqueRunError:
                    pass
                if interrupted:
                    raise KeyboardInterrupt(OpaqueReason.CANCELLED.value) from None
                raise RunCancelled(OpaqueReason.CANCELLED.value) from None
            self.commands._live_stop(kind, reason, current)

    def _tick(self) -> None:
        if self.completed is not None:
            return
        commands = self.commands
        now = time.monotonic()
        if self.first_tick is None:
            self.first_tick = now
        if commands.budget is None or commands.budget.deadline - now < 10:
            commands._live_stop(OpaqueObservation.INSPECT_BEFORE, OpaqueReason.TIMEOUT)
        assert commands.budget is not None
        if now > min(commands.budget.deadline, self.first_tick + 2) or self.attempts >= 4:
            commands._live_stop(OpaqueObservation.INSPECT_BEFORE, OpaqueReason.PROCESS_UNAVAILABLE)
        if self.last_attempt is not None and now - self.last_attempt < 0.1:
            return
        self.last_attempt = now
        self.attempts += 1
        before = commands._live_inspect(self.held, OpaqueObservation.INSPECT_BEFORE)
        if time.monotonic() > min(commands.budget.deadline, self.first_tick + 2):
            commands._live_stop(OpaqueObservation.INSPECT_BEFORE, OpaqueReason.TIMEOUT)
        if not before.running:
            return
        if LIVE_PID_SCHEMA is None:
            if commands.lifetime_report is not None:
                commands.lifetime_report["live_process_schema_capture"] = before.schema_capture()
            commands._live_stop(
                OpaqueObservation.INSPECT_BEFORE, OpaqueReason.INVALID_NATIVE_SCHEMA
            )
        assert commands.lifetime_context is not None
        request = _live_request(before, commands.lifetime_context)
        sample = commands._live_processes(request)
        after = commands._live_inspect(self.held, OpaqueObservation.INSPECT_AFTER)
        if before != after:
            commands._live_stop(
                OpaqueObservation.INSPECT_AFTER, OpaqueReason.INVALID_PROCESS_OBSERVATION
            )
        self.completed = {
            "schema": 1,
            "status": "observed",
            "scope": "known-two-pid-snapshot",
            "container_id": self.held.container_id,
            "image_id": self.held.image_id,
            "run_nonce": self.held.run_nonce,
            "started_at": before.started_at,
            **asdict(sample),
        }
        if commands.lifetime_report is not None:
            commands.lifetime_report["live_process_observation"] = self.completed

    def require_complete(self) -> None:
        if self.completed is None:
            self.commands._live_stop(
                OpaqueObservation.INSPECT_BEFORE, OpaqueReason.PROCESS_UNAVAILABLE
            )


def fingerprint(root: Path, commands: Commands) -> str:
    """Bind all nonignored build context, modes and bytes, including dirty local edits."""
    paths = commands.git(
        root, "ls-files", "--cached", "--others", "--exclude-standard", "-z"
    ).split("\0")
    paths = sorted(set(path for path in paths if path))
    if len(paths) > MAX_TREE_FILES:
        raise ValidationError("candidate input closure exceeds its file bound")
    digest, total = hashlib.sha256(), 0
    consumed = 0
    for path in paths:
        relative = Path(path)
        target = root / relative
        if relative.is_absolute() or ".." in relative.parts or ".git" in relative.parts:
            raise ValidationError("candidate input path escapes its build context")
        if not target.exists() and not target.is_symlink():
            digest.update(json.dumps([path, "deleted"]).encode())
            continue
        info = target.lstat()
        if not stat.S_ISREG(info.st_mode) or any(
            parent.is_symlink() for parent in target.parents if parent != root
        ):
            raise ValidationError(
                "candidate input closure requires regular files without symbolic links"
            )
        total += info.st_size
        if total > MAX_TREE_BYTES:
            raise ValidationError("candidate input closure exceeds its byte bound")
        digest.update(json.dumps([path, stat.S_IMODE(info.st_mode), info.st_size]).encode())
        with target.open("rb") as stream:
            while data := stream.read(65536):
                consumed += len(data)
                if consumed > MAX_TREE_BYTES:
                    raise ValidationError(
                        "candidate input changed beyond its byte bound while reading"
                    )
                if commands.budget:
                    commands.budget.check()
                digest.update(data)
    return digest.hexdigest()


def _held_revision(root: Path, expected: str, commands: Commands, *, clean: bool = True) -> None:
    if Path(commands.git(root, "rev-parse", "--show-toplevel").strip()).resolve() != root.resolve():
        raise ValidationError("workspace source must be an actual independent Git checkout")
    if commands.git(root, "rev-parse", "HEAD").strip() != expected:
        raise ValidationError("source checkout differs from its immutable revision")
    if clean and commands.git(root, "status", "--porcelain", "--untracked-files=all").strip():
        raise ValidationError("immutable source checkout has working-tree changes")


def _checkout(destination: Path, revision: str, commands: Commands, source: str) -> None:
    destination.mkdir()
    commands.git(destination, "init", "--quiet")
    commands.git(destination, "fetch", "--quiet", "--no-tags", "--depth=1", source, revision)
    commands.git(destination, "checkout", "--quiet", "--detach", "FETCH_HEAD")
    _held_revision(destination, revision, commands)


def _overlay(source: Path, target: Path, commands: Commands) -> None:
    paths = commands.git(
        source, "ls-files", "--cached", "--others", "--exclude-standard", "-z"
    ).split("\0")
    for path in filter(None, paths):
        original, destination = source / path, target / path
        if original.exists():
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(original, destination)
            destination.chmod(stat.S_IMODE(original.stat().st_mode))
        elif destination.exists():
            destination.unlink()


def validate_configuration(root: Path, repository: str) -> dict[str, Any]:
    path = root / CONFIG
    if path.stat().st_size > 32768:
        raise ValidationError("Dev Container configuration exceeds its byte bound")
    document = _json(path.read_bytes())
    allowed = {
        "name",
        "build",
        "workspaceMount",
        "workspaceFolder",
        "remoteUser",
        "updateRemoteUserUID",
        "features",
        "containerEnv",
        "mounts",
        "postCreateCommand",
        "waitFor",
        "hostRequirements",
        "customizations",
    }
    if set(document) - allowed:
        raise ValidationError(
            "Dev Container configuration includes unsupported hooks or runtime options"
        )
    workspace = f"/workspaces/{repository}"
    if document.get("build") != {"dockerfile": "Dockerfile", "context": ".."}:
        raise ValidationError(
            "only the reviewed Dockerfile and repository build context are supported"
        )
    if (
        document.get("workspaceFolder") != workspace
        or document.get("workspaceMount")
        != f"source=${{localWorkspaceFolder}},target={workspace},type=bind"
    ):
        raise ValidationError("Dev Container workspace differs from its reviewed repository target")
    if document.get("remoteUser") != "vscode" or document.get("updateRemoteUserUID") is not True:
        raise ValidationError("Dev Container must configure its reviewed nonroot user")
    lifecycle = "bash .devcontainer/verify-tools.sh"
    if repository != "boxferry":
        lifecycle += " && cargo fetch --locked"
    if (
        document.get("postCreateCommand") != lifecycle
        or document.get("waitFor") != "postCreateCommand"
    ):
        raise ValidationError("Dev Container lifecycle command differs from its reviewed verifier")
    env = document.get("containerEnv")
    paths = {"CARGO_HOME": f"/workspaces/.{repository}-cargo"}
    if repository != "boxferry":
        paths["CARGO_TARGET_DIR"] = f"/workspaces/.{repository}-target"
    if repository in {"boxferry", "podman-lens"}:
        paths["GH_CONFIG_DIR"] = f"/workspaces/.{repository}-gh"
    expected_env = dict(paths)
    if repository != "podman-lens":
        expected_env["PATH"] = (
            "/home/vscode/.local/bin:/usr/local/cargo/bin:/usr/local/sbin:"
            "/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
        )
    if env != expected_env:
        raise ValidationError("Dev Container environment differs from its finite reviewed contract")
    volume_names = {"CARGO_HOME": "cargo", "CARGO_TARGET_DIR": "target", "GH_CONFIG_DIR": "gh"}
    expected = {
        f"source={repository}-{volume_names[key]}-${{devcontainerId}},target={target},type=volume"
        for key, target in paths.items()
    }
    if repository == "boxferry":
        expected.update(
            f"source=${{localWorkspaceFolder}}/../{name},target={workspace}/.boxferry-workspace/{name},type=bind"
            for name in SIBLINGS
        )
    mounts = document.get("mounts")
    if (
        not isinstance(mounts, list)
        or any(not isinstance(item, str) for item in mounts)
        or len(mounts) != len(expected)
        or set(mounts) != expected
    ):
        raise ValidationError(
            "Dev Container mounts differ from the finite reviewed workspace contract"
        )
    # Configuration substitutions can read host environment even during build.
    for key in ("features", "build"):
        if "${" in json.dumps(document.get(key)):
            raise ValidationError("Dev Container build inputs cannot substitute host environment")
    if not (root / LOCK).is_file() or not (root / ".devcontainer/verify-tools.sh").is_file():
        raise ValidationError(
            "candidate requires a committed feature lock and executable verifier source"
        )
    return document


def canonical_versions(automation: Path) -> dict[str, str]:
    project = tomllib.loads((automation / "automation/pyproject.toml").read_text())
    versions = {"uv": project["tool"]["uv"]["required-version"].removeprefix("==")}
    for name in ("ruff", "ty"):
        matches = [
            value.split("==", 1)[1]
            for value in project["dependency-groups"]["dev"]
            if value.startswith(name + "==")
        ]
        if len(matches) != 1:
            raise ValidationError("shared Python authority has no unique exact tool version")
        versions[name] = matches[0]
    source = (automation / ".github/workflows/ci.yml").read_text()
    nodes = set(re.findall(r"^\s+node-version: ([0-9]+\.[0-9]+\.[0-9]+)$", source, re.MULTILINE))
    if len(nodes) != 1:
        raise ValidationError("shared Node authority has no unique exact version")
    versions["node"] = nodes.pop()
    if any(re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", value) is None for value in versions.values()):
        raise ValidationError("canonical tool versions must be exact numeric releases")
    return versions


def _cli(automation: Path) -> tuple[Path, str]:
    project = automation / CLI_PROJECT
    package = _json((project / "package.json").read_bytes())
    lock = _json((project / "package-lock.json").read_bytes())
    dependencies = package.get("devDependencies")
    if not isinstance(dependencies, dict) or set(dependencies) != {"@devcontainers/cli"}:
        raise ValidationError(
            "official CLI project must declare only its exact canonical dependency"
        )
    version = dependencies["@devcontainers/cli"]
    packages = lock.get("packages")
    if lock.get("lockfileVersion") != 3 or not isinstance(packages, dict):
        raise ValidationError("official CLI requires a bounded npm v3 integrity lock")
    root, record = packages.get(""), packages.get("node_modules/@devcontainers/cli")
    if (
        not isinstance(root, dict)
        or root.get("devDependencies") != dependencies
        or not isinstance(record, dict)
    ):
        raise ValidationError("official CLI root declaration and locked package disagree")
    if (
        not isinstance(version, str)
        or re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version) is None
        or record.get("version") != version
        or not isinstance(record.get("integrity"), str)
        or re.fullmatch(r"sha512-[A-Za-z0-9+/]{86}==", record["integrity"]) is None
        or record.get("resolved")
        != f"https://registry.npmjs.org/@devcontainers/cli/-/cli-{version}.tgz"
    ):
        raise ValidationError("official CLI declaration and integrity-locked package disagree")
    executable = project / "node_modules/@devcontainers/cli/devcontainer.js"
    if not executable.is_file():
        raise ValidationError(
            "install the canonical CLI with npm ci --ignore-scripts before qualification"
        )
    return executable, version


def backend(repository: str, mode: str, selected: str) -> str:
    expected = (
        "podman" if mode == "local" or repository in {"podman-lens", "quadlet-lens"} else "docker"
    )
    if selected != expected:
        raise ValidationError(
            "backend differs from reviewed hosted/local runtime policy; no fallback"
        )
    return expected


def validate_hosted_podman_host(environment_values: Mapping[str, str], release: str) -> None:
    fields = dict(line.split("=", 1) for line in release.splitlines() if "=" in line)
    if (
        environment_values.get("GITHUB_ACTIONS") != "true"
        or environment_values.get("RUNNER_OS") != "Linux"
        or environment_values.get("GITHUB_REPOSITORY")
        not in {"Strukturpiloten/podman-lens", "Strukturpiloten/quadlet-lens"}
        or fields.get("ID", "").strip('"') != "ubuntu"
        or re.fullmatch(r"[0-9]{2}\.[0-9]{2}", fields.get("VERSION_ID", "").strip('"')) is None
    ):
        raise ValidationError(
            "rootful Podman prerequisite requires its reviewed Ubuntu hosted caller"
        )


def prepare_hosted_podman(commands: Commands) -> dict[str, object]:
    """Reuse the existing Ubuntu signed-APT OUTER prerequisite exception.

    Quadlet's hosted-generator-podman.py and Podman's native workflow own this
    host-package prerequisite, distinct from immutable INNER native runtime pins.
    There is no new floating download or invented exact outer version pin.
    """
    validate_hosted_podman_host(os.environ, Path("/etc/os-release").read_text())
    if not Path("/usr/bin/podman").is_file():
        commands.call(
            (
                "/usr/bin/sudo",
                "-n",
                "/usr/bin/timeout",
                "--signal=TERM",
                "--kill-after=2s",
                "80s",
                "/bin/sh",
                "-ceu",
                "apt-get update && apt-get install --yes --no-install-recommends podman",
            ),
            "podman-signed-apt-prerequisite",
            85,
        )
    owner = commands.call(
        ("/usr/bin/dpkg-query", "--search", "/usr/bin/podman"), "podman-package-owner"
    ).strip()
    if owner != "podman: /usr/bin/podman":
        raise ValidationError("hosted Podman binary is outside the reviewed signed-APT package")
    package = commands.call(
        ("/usr/bin/dpkg-query", "--show", "--showformat=${Version}", "podman"),
        "podman-package-version",
    ).strip()
    if re.fullmatch(r"[A-Za-z0-9.+:~_-]{1,128}", package) is None:
        raise ValidationError("hosted Podman package version is unavailable")
    uid = commands.call(
        (
            "/usr/bin/sudo",
            "-n",
            "/usr/bin/timeout",
            "--signal=TERM",
            "--kill-after=2s",
            "5s",
            "/usr/bin/id",
            "-u",
        ),
        "podman-rootful-prerequisite",
        8,
    ).strip()
    if uid != "0":
        raise ValidationError("reviewed hosted Podman requires actual root privileges")
    return {
        "installation": "ubuntu-signed-apt-prerequisite",
        "package": package,
        "rootless": False,
        "remote": False,
        "root_uid": 0,
        "version_policy": "reviewed-host-package-prerequisite-not-inner-runtime-pin",
    }


def effective_outer_mode(mode: str, engine: str, requested: str | None = None) -> str:
    """Privilege is explicit local operator input, never inferred from EUID or policy."""
    if mode not in {"local", "hosted"}:
        raise ValidationError("unsupported qualification orchestration mode")
    if mode == "hosted":
        if requested is not None:
            raise ValidationError("hosted qualification cannot accept a local outer-mode override")
        return "rootful" if engine == "podman" else "daemon"
    if engine != "podman" or requested not in {None, "rootless", "rootful"}:
        raise ValidationError(
            "local qualification requires an explicit supported Podman outer mode"
        )
    if os.getuid() == 0 or os.geteuid() != os.getuid():
        raise ValidationError(
            "local qualifier must remain the original nonroot operator; do not sudo it"
        )
    return requested or "rootless"


def _rootful_bridge(private: Path) -> tuple[tuple[str, ...], tuple[str, ...]]:
    """One fixed hosted/local rootful deadline and clean-environment bridge."""
    return (
        ("/usr/bin/sudo", "-n", "/usr/bin/timeout", "--signal=TERM", "--kill-after=2s"),
        (
            "/usr/bin/env",
            "-i",
            *(f"{key}={value}" for key, value in environment(private).items()),
            "CONTAINERS_CONF_OVERRIDE=" + str(private / "containers.conf"),
        ),
    )


def _safe_live_public(value: object) -> dict[str, object]:
    """One closed whitelist for normal and compact live-observation receipts."""
    record = _live_dict(value)
    capture_keys = {
        "schema",
        "status",
        "container_id",
        "image_id",
        "run_nonce",
        "running",
        "state_pid_kind",
        "candidate_state_conmon_pid_kind",
        "state_pid_candidate",
        "candidate_state_conmon_pid_candidate",
        "started_at_valid",
        "native_binary_sha256",
        "native_version",
        "selector_approved",
        "observer_executed",
        "expected_container_uid",
    }
    observed_keys = {
        "schema",
        "status",
        "scope",
        "container_id",
        "image_id",
        "run_nonce",
        "started_at",
        "observer_pid",
        "pid_namespace",
        "mount_namespace",
        "processes",
    }
    if type(record.get("schema")) is not int or record["schema"] != 1:
        raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
    for key, pattern in (
        ("container_id", r"[0-9a-f]{64}"),
        ("image_id", r"sha256:[0-9a-f]{64}"),
        ("run_nonce", r"[0-9a-f]{32}"),
    ):
        field = record.get(key)
        if type(field) is not str or re.fullmatch(pattern, field) is None:
            raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
    if record.get("status") == "unsupported-selector-schema-capture-only":
        if set(record) != capture_keys:
            raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
        if record["selector_approved"] is not False or record["observer_executed"] is not False:
            raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
        for key in ("running", "started_at_valid"):
            if type(record[key]) is not bool:
                raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
        for kind, pid in (
            ("state_pid_kind", "state_pid_candidate"),
            ("candidate_state_conmon_pid_kind", "candidate_state_conmon_pid_candidate"),
        ):
            if record[kind] not in (
                "missing",
                "integer",
                "invalid-integer",
                "boolean",
                "string",
                "float",
                "array",
                "object",
                "null",
                "other",
            ):
                raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
            if record[kind] == "integer":
                _live_uint(record[pid])
            elif record[pid] is not None:
                raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
        _live_uint(record["expected_container_uid"], minimum=1, maximum=4294967295)
        if (
            type(record["native_binary_sha256"]) is not str
            or re.fullmatch(r"[0-9a-f]{64}", record["native_binary_sha256"]) is None
            or type(record["native_version"]) is not str
            or len(record["native_version"]) > 512
            or re.fullmatch(
                r"podman version [0-9]+\.[0-9]+\.[0-9]+(?:[-+][A-Za-z0-9.-]+)?",
                record["native_version"],
            )
            is None
        ):
            raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
    elif record.get("status") == "observed":
        if set(record) != observed_keys or record["scope"] != "known-two-pid-snapshot":
            raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
        _live_timestamp(record["started_at"])
        _live_uint(record["observer_pid"], minimum=2)
        _live_ns(record["pid_namespace"], "pid")
        _live_ns(record["mount_namespace"], "mnt")
        rows = record["processes"]
        if type(rows) not in (list, tuple) or len(rows) != 2:
            raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
        roles = set()
        for item in rows:
            row = _live_dict(item)
            if set(row) != {
                "role",
                "pid",
                "uids",
                "nspid",
                "start_ticks",
                "pid_namespace",
                "mount_namespace",
                "pidfd_alive_before",
                "pidfd_alive_after",
            }:
                raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
            if row["role"] not in ("container-init", "conmon") or row["role"] in roles:
                raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
            roles.add(row["role"])
            _live_uint(row["pid"], minimum=2)
            for key, limit in (("uids", 4), ("nspid", 32)):
                numbers = row[key]
                if type(numbers) not in (list, tuple) or not 1 <= len(numbers) <= limit:
                    raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
                if key == "uids" and len(numbers) != 4:
                    raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
                for number in numbers:
                    _live_uint(
                        number,
                        minimum=0 if key == "uids" else 1,
                        maximum=4294967295 if key == "uids" else 2147483647,
                    )
            _live_ns(row["pid_namespace"], "pid")
            _live_ns(row["mount_namespace"], "mnt")
            if (
                type(row["start_ticks"]) is not str
                or re.fullmatch(r"[0-9]{1,20}", row["start_ticks"]) is None
                or int(row["start_ticks"]) <= 0
                or row["pidfd_alive_before"] is not True
                or row["pidfd_alive_after"] is not True
            ):
                raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
    else:
        raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
    if len(json.dumps(record, sort_keys=True).encode()) > 8192:
        raise OpaqueParseError(OpaqueReason.INVALID_PROCESS_OBSERVATION)
    return cast(dict[str, object], record)


def serialize_report(report: Mapping[str, object]) -> str:
    """Retain bounded identities, primary and cleanup causes when detail overflows."""
    live_keys = ("live_process_schema_capture", "live_process_observation")
    if any(key in report for key in live_keys):
        normalized = dict(report)
        for key in live_keys:
            if key in report:
                try:
                    normalized[key] = _safe_live_public(report[key])
                except BaseException:
                    normalized[key] = {
                        "schema": 1,
                        "status": "unavailable",
                        "reason": OpaqueReason.INVALID_PROCESS_OBSERVATION.value,
                    }
                    normalized["status"] = "failure"
                    normalized.setdefault("cause", OpaqueReason.INVALID_PROCESS_OBSERVATION.value)
        report = normalized
    encoded = json.dumps(report, sort_keys=True)
    if len(encoded.encode()) <= MAX_REPORT:
        return encoded
    compact: dict[str, object] = {
        "schema": 1,
        "status": "failure",
        "cause": "qualification report exceeds its byte bound",
    }
    for key in (
        "mode",
        "engine",
        "requested_outer_mode",
        "outer_mode",
        "caller_uid",
        "engine_uid",
        "lifetime_bootstrap_sha256",
        "lifetime_init_sha256",
        "lifetime_module_sha256",
        "retained_lifetime_preparation",
        "retained_scope_owner_uid",
        "retained_scope_mode",
        "retained_control_owner_uid",
        "retained_control_mode",
        "empty_private_directory",
        "empty_lifetime_control_directory",
        "opaque_failure_mode",
        "opaque_failure_reason",
        "opaque_failure_status",
        "opaque_failure_argv_sha256",
        "opaque_failure_program_sha256",
        "opaque_failure_stdout_prefix_bytes",
        "opaque_failure_stderr_prefix_bytes",
        "opaque_failure_stdout_prefix_sha256",
        "opaque_failure_stderr_prefix_sha256",
        "command_failure_stage",
        "command_failure_status",
        "command_failure_cause",
        "abort_hook_failure",
        "outer_abort_stage",
        "outer_abort_status",
        "outer_abort_cause",
        "outer_abort_signal_number",
        "outer_abort_signal_name",
        "outer_abort_persisted",
        "outer_abort_receipt",
        "outer_abort_receipt_sha256",
        "outer_abort_receipt_owner_uid",
        "outer_abort_receipt_mode",
        "outer_abort_receipt_disposition",
        "outer_abort_persistence_failure",
        "outer_abort_close_failure",
        "namespace_closed",
        "directory_cleanup",
        "engine_resource_cleanup",
        "engine_client_version",
        "engine_storage_driver",
        "engine_graph_root",
        "engine_run_root",
        "engine_remote",
        "engine_cgroup_manager",
        "engine_runtime",
        "engine_runtime_name_representation",
        "engine_runtime_path_representation",
        "candidate_sha",
        "policy_sha",
        "automation_sha",
        "input_sha256",
        "source_manifest_sha256",
        "run_nonce",
        "lifetime_nonce",
        "private_directory_identity",
        "preparation_nonce",
        "preparation_directory_identity",
        "cleanup",
        "cleanup_cause",
        "preparation_cleanup",
        "preparation_cleanup_cause",
        "retained_private_directory",
        "retained_preparation_directory",
        "next_action",
    ):
        value = report.get(key)
        if isinstance(value, (str, int, bool)):
            compact[key] = (
                _bounded_text(redact(str(value), protected_values(os.environ)), 1024)
                if isinstance(value, str)
                else value
            )
    for key in live_keys:
        if key in report:
            compact[key] = report[key]
    if report.get("cause"):
        compact["primary_cause"] = _bounded_text(
            redact(str(report["cause"]), protected_values(os.environ)), 2048
        )
    return json.dumps(compact, sort_keys=True)


def _root_owned_executable(selected: Path) -> tuple[Path, os.stat_result, list[dict[str, object]]]:
    """Check every traversed parent and symlink hop, including intermediate targets."""
    if not selected.is_absolute():
        raise ValidationError("installed executable observation requires an absolute path")
    pending = list(selected.parts[1:])
    current = Path("/")
    custody: list[dict[str, object]] = []
    links = 0
    while True:
        facts = current.lstat()
        if facts.st_uid != 0 or (
            not stat.S_ISLNK(facts.st_mode) and stat.S_IMODE(facts.st_mode) & 0o022
        ):
            raise ValidationError("installed executable path or parent custody is unsafe")
        custody.append(
            {
                "path": str(current),
                "uid": facts.st_uid,
                "mode": stat.S_IMODE(facts.st_mode),
                "identity": [facts.st_dev, facts.st_ino],
                "mtime_ns": facts.st_mtime_ns,
                "ctime_ns": facts.st_ctime_ns,
            }
        )
        if stat.S_ISLNK(facts.st_mode):
            links += 1
            if links > 40:
                raise ValidationError("installed executable symlink chain exceeds bound")
            target = Path(os.readlink(current))
            current = Path("/") if target.is_absolute() else current.parent
            pending = list(target.parts[1:] if target.is_absolute() else target.parts) + pending
            continue
        if not pending:
            if (
                not stat.S_ISREG(facts.st_mode)
                or not facts.st_mode & 0o111
                or facts.st_mode & (stat.S_ISUID | stat.S_ISGID)
            ):
                raise ValidationError("installed executable must resolve to a regular executable")
            return current, facts, custody
        if not stat.S_ISDIR(facts.st_mode):
            raise ValidationError("installed executable parent is not a directory")
        part = pending.pop(0)
        current = current.parent if part == ".." else current / part


def _podman_path_custody() -> tuple[Path, os.stat_result, list[dict[str, object]]]:
    return _root_owned_executable(Path("/usr/bin/podman"))


def installed_podman() -> dict[str, object]:
    """Observe the literal installed binary as the user before any local root bridge."""
    path, facts, custody = _podman_path_custody()
    if facts.st_size > 64 * 1024**2:
        raise ValidationError("installed /usr/bin/podman exceeds executable observation bound")
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    with os.fdopen(fd, "rb") as stream:
        opened = os.fstat(stream.fileno())
        if (opened.st_dev, opened.st_ino) != (facts.st_dev, facts.st_ino):
            raise ValidationError("installed /usr/bin/podman inode changed before hashing")
        content = stream.read(64 * 1024**2 + 1)
    if len(content) != facts.st_size or len(content) > 64 * 1024**2:
        raise ValidationError("installed /usr/bin/podman size changed or exceeds bound")
    outcome = run(
        (str(path), "--version"),
        cwd=Path("/"),
        stage="installed-podman-version",
        timeout_seconds=5,
        environment={"PATH": "/usr/bin:/bin", "LC_ALL": "C.UTF-8"},
    )
    version = outcome.stdout.strip()
    if (
        outcome.status != "success"
        or re.fullmatch(r"podman version [0-9]+\.[0-9]+\.[0-9]+(?:[-+][A-Za-z0-9.-]+)?", version)
        is None
        or len(version.encode()) > 512
    ):
        raise ValidationError("installed /usr/bin/podman version observation failed")
    current_path, current, current_custody = _podman_path_custody()

    def identity(item: os.stat_result) -> tuple[int, int, int, int, int]:
        return item.st_dev, item.st_ino, item.st_size, item.st_mtime_ns, item.st_ctime_ns

    if current_path != path or identity(current) != identity(facts) or current_custody != custody:
        raise ValidationError("installed /usr/bin/podman changed during observation")
    return {
        "path": "/usr/bin/podman",
        "resolved_path": str(path),
        "uid": facts.st_uid,
        "mode": stat.S_IMODE(facts.st_mode),
        "version": version,
        "sha256": hashlib.sha256(content).hexdigest(),
        "path_custody": custody,
        "integrity_scope": "observed-installed-host-not-download-or-cross-architecture-pin",
    }


def installed_runc(commands: Commands) -> dict[str, object]:
    """Observe the explicitly selected installed runtime, never invent a download pin."""
    path, facts, custody = _root_owned_executable(Path("/usr/bin/runc"))
    version = commands.call((str(path), "--version"), "selected-runc-version").strip()
    if not re.match(r"runc version [0-9]+\.[0-9]+\.[0-9]+(?:\s|$)", version):
        raise ValidationError("selected installed runc has unsupported version evidence")
    if facts.st_size > 64 * 1024**2:
        raise ValidationError("selected installed runc exceeds executable observation bound")
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    with os.fdopen(fd, "rb") as stream:
        opened = os.fstat(stream.fileno())
        if (opened.st_dev, opened.st_ino) != (facts.st_dev, facts.st_ino):
            raise ValidationError("selected installed runc inode changed")
        content = stream.read(64 * 1024**2 + 1)
    if len(content) != facts.st_size or len(content) > 64 * 1024**2:
        raise ValidationError("selected installed runc size changed or exceeds bound")
    digest = hashlib.sha256(content).hexdigest()
    current_path, current, current_custody = _root_owned_executable(Path("/usr/bin/runc"))

    def identity(item: os.stat_result) -> tuple[int, int, int, int, int]:
        return item.st_dev, item.st_ino, item.st_size, item.st_mtime_ns, item.st_ctime_ns

    if current_path != path or identity(current) != identity(facts) or current_custody != custody:
        raise ValidationError("selected installed runc changed during observation")
    return {
        "path": "/usr/bin/runc",
        "resolved_path": str(path),
        "uid": facts.st_uid,
        "mode": stat.S_IMODE(facts.st_mode),
        "sha256": digest,
        "version": version[:512],
        "path_custody": custody,
        "integrity_scope": "observed-installed-host-not-download-or-cross-architecture-pin",
    }


def _runtime_representation(runtime: object, field: str, receipt: Mapping[str, object]) -> str:
    """Classify native evidence without retaining untrusted strings or normalizing paths."""
    if not isinstance(runtime, dict):
        return "invalid-type"
    if field not in runtime:
        return "missing"
    value = runtime[field]
    if not isinstance(value, str):
        return "invalid-type"
    if field == "name" and value == "runc":
        return "alias-runc"
    # The selected spelling wins deterministically when the trusted paths coincide.
    if value == receipt["path"]:
        return "selected-path"
    if value == receipt["resolved_path"]:
        return "resolved-path"
    return "other"


def _engine(
    private: Path,
    engine: str,
    mode: str = "local",
    *,
    rootful: bool | None = None,
    local_profile: bool = False,
) -> str:
    if engine == "docker":
        return "/usr/bin/docker"
    launcher = private / "podman"
    override = private / "containers.conf"
    override.write_text('[engine]\nlock_type = "file"\n')
    base = (
        "/usr/bin/podman",
        "--remote=false",
        "--root",
        str(private / "store"),
        "--runroot",
        str(private / "runroot"),
        "--tmpdir",
        str(private / "podman-tmp"),
        "--storage-driver",
        "overlay",
    )
    if local_profile:
        if mode != "local" or rootful is not True:
            raise ValidationError("direct profile is local rootful only")
        base += (
            "--cgroup-manager",
            "cgroupfs",
            "--events-backend",
            "file",
            "--runtime",
            "/usr/bin/runc",
        )
    if rootful if rootful is not None else mode == "hosted":
        # The timeout itself is root-owned, matching the reviewed hosted launcher;
        # killing an unprivileged sudo client is not a privileged-child deadline.
        privilege, clean = _rootful_bridge(private)
        prefix = (
            '#!/bin/sh\ncase "$1" in\n'
            "  build) seconds=3600 ;;\n"
            '  buildx) if [ "${2:-}" = build ]; then seconds=3600; else seconds=30; fi ;;\n'
            "  start) seconds=300 ;;\n"
            "  *) seconds=30 ;;\nesac\n"
            "exec " + shlex.join(privilege) + ' "${seconds}s" '
        )
        launcher.write_text(prefix + shlex.join((*clean, *base)) + ' "$@"\n')
    else:
        launcher.write_text(
            "#!/bin/sh\nexec "
            + shlex.join(("/usr/bin/env", "CONTAINERS_CONF_OVERRIDE=" + str(override), *base))
            + ' "$@"\n'
        )
    launcher.chmod(0o700)
    return str(launcher)


def exclude_git_build_metadata(snapshot: Path) -> dict[str, dict[str, str | int]]:
    """Bind effective ignores while retaining real Git metadata for runtime smoke."""
    result = {}
    paths = [snapshot / ".dockerignore"]
    # Podman gives this root-level file precedence over .dockerignore.
    container_ignore = snapshot / ".containerignore"
    if container_ignore.exists() or container_ignore.is_symlink():
        paths.append(container_ignore)
    specific = snapshot / ".devcontainer/Dockerfile.dockerignore"
    if specific.exists():
        paths.append(specific)
    for path in paths:
        if path.is_symlink() or (path.exists() and not path.is_file()):
            raise ValidationError("build-context ignore must be a regular candidate file")
        content = path.read_bytes() if path.exists() else b""
        if len(content) > 128 * 1024:
            raise ValidationError("build-context ignore exceeds its byte bound")
        content = (
            content.rstrip(b"\r\n")
            + b"\n# BoxFerry qualification: Git metadata is runtime-only.\n.git\n**/.git\n"
        )
        path.write_bytes(content)
        result[path.relative_to(snapshot).as_posix()] = {
            "sha256": hashlib.sha256(content).hexdigest(),
            "mode": stat.S_IMODE(path.stat().st_mode),
        }
    return result


def build_context_fingerprint(
    snapshot: Path, commands: Commands, ignores: Mapping[str, Mapping[str, str | int]]
) -> str:
    # Candidate .gitignore can hide these files; bind generated bytes and modes
    # explicitly in addition to the ordinary complete tracked input fingerprint.
    actual = {}
    for name in ignores:
        path = snapshot / name
        if path.is_symlink() or not path.is_file():
            raise ValidationError("held build-context ignore was substituted")
        actual[name] = {
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "mode": stat.S_IMODE(path.stat().st_mode),
        }
    if actual != ignores:
        raise ValidationError("held build-context ignore changed during qualification")
    return manifest_digest({"files": fingerprint(snapshot, commands), "ignores": actual})


def _image_identity(value: object) -> str:
    if not isinstance(value, str) or re.fullmatch(r"(?:sha256:)?[a-f0-9]{64}", value) is None:
        raise ValidationError("built image has no immutable SHA-256 image ID")
    return value if value.startswith("sha256:") else "sha256:" + value


def validate_engine(
    document: object,
    selected: str,
    architecture: str,
    private: Path,
    *,
    rootless: bool | None = None,
    storage_driver: str | None = None,
) -> None:
    if not isinstance(document, dict):
        raise ValidationError("outer engine returned malformed native evidence")
    if selected == "docker":
        if (
            document.get("OSType") != "linux"
            or document.get("Architecture")
            not in {architecture, {"amd64": "x86_64", "arm64": "aarch64"}[architecture]}
            or not isinstance(document.get("DockerRootDir"), str)
            or not document["DockerRootDir"].startswith("/")
        ):
            raise ValidationError("reviewed Docker outer backend or architecture was not observed")
    else:
        host, store = document.get("host"), document.get("store")
        if not isinstance(host, dict) or not isinstance(store, dict):
            raise ValidationError("outer Podman store and host facts are missing")
        if (
            host.get("serviceIsRemote") is not False
            or host.get("arch") != architecture
            or store.get("graphRoot") != str(private / "store")
            or store.get("runRoot") != str(private / "runroot")
            or not isinstance(host.get("security"), dict)
            or host["security"].get("rootless") is not rootless
            or (storage_driver is not None and store.get("graphDriverName") != storage_driver)
        ):
            raise ValidationError(
                "Podman must use the exact private local store and native architecture"
            )


def validate_docker_budget_filesystem(
    private: Path, docker_root: str, report: dict[str, object]
) -> None:
    """Require the monitored directory and resolved hosted Docker store to share st_dev.

    This checks coverage of the existing Budget without rebaselining free space;
    it establishes no cold-build budget or native qualification evidence.
    """
    facts: dict[str, object] = {
        "reported_docker_root_dir": docker_root,
        "docker_root_dir": None,
        "budget_path": None,
        "budget_device": None,
        "docker_device": None,
        "same_filesystem": None,
    }
    report["docker_storage"] = facts
    try:
        budget = private.resolve(strict=True)
        facts["budget_path"] = str(budget)
        budget_stat = budget.stat()
        facts["budget_device"] = budget_stat.st_dev
        docker = Path(docker_root).resolve(strict=True)
        facts["docker_root_dir"] = str(docker)
        docker_stat = docker.stat()
        facts["docker_device"] = docker_stat.st_dev
    except (OSError, ValueError) as error:
        raise ValidationError(
            "Hosted Docker budget coverage needs existing observable DockerRootDir and "
            f"private budget directories on the same filesystem; verify runner storage: {error}"
        ) from error
    if not stat.S_ISDIR(budget_stat.st_mode) or not stat.S_ISDIR(docker_stat.st_mode):
        raise ValidationError(
            "DockerRootDir and private budget path must be existing directories; "
            "verify the hosted runner storage layout"
        )
    facts["same_filesystem"] = budget_stat.st_dev == docker_stat.st_dev
    if not facts["same_filesystem"]:
        raise ValidationError(
            "DockerRootDir and private budget path use different filesystems; "
            "use a hosted runner where both existing directories share a filesystem"
        )


def passwd_identity(path: Path, user: str) -> tuple[str, str]:
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 65536:
        raise ValidationError("built image passwd content exceeds its regular-file bound")
    entries = [
        line.split(":") for line in path.read_text().splitlines() if line.split(":", 1)[0] == user
    ]
    if len(entries) != 1 or len(entries[0]) != 7:
        raise ValidationError("configured nonroot user has no unique built-image passwd entry")
    uid, gid = entries[0][2:4]
    if re.fullmatch(r"[1-9][0-9]{0,8}", uid) is None or re.fullmatch(r"[0-9]{1,9}", gid) is None:
        raise ValidationError(
            "configured development user needs an actual positive UID and numeric GID"
        )
    return uid, gid


def _inspection(output: str) -> dict[str, Any]:
    document = json.loads(output)
    if not isinstance(document, list) or len(document) != 1 or not isinstance(document[0], dict):
        raise ValidationError("engine inspection must identify one held resource")
    return document[0]


def validate_engine_version(document: dict[str, Any], selected: str) -> None:
    if selected == "docker":
        server = document.get("Server")
        if not isinstance(server, dict):
            raise ValidationError("outer Docker daemon version was not observed")
        platform = server.get("Platform")
        components = server.get("Components")
        if (
            not isinstance(platform, dict)
            or not isinstance(platform.get("Name"), str)
            or not platform["Name"].startswith("Docker Engine")
            or not isinstance(components, list)
            or not any(
                isinstance(item, dict)
                and item.get("Name") == "Engine"
                and item.get("Version") == server.get("Version")
                for item in components
            )
        ):
            raise ValidationError("outer daemon is not the reviewed actual Docker Engine")
    else:
        client = document.get("Client")
        if (
            not isinstance(client, dict)
            or not isinstance(client.get("Version"), str)
            or not isinstance(client.get("APIVersion"), str)
            or re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", client["Version"]) is None
            or re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", client["APIVersion"]) is None
        ):
            raise ValidationError("outer Podman client version was not observed")


def remove_owned_container(
    commands: Commands, engine: tuple[str, ...], identity: str, token: str
) -> None:
    inspected = _inspection(
        commands.call((*engine, "container", "inspect", identity), "cleanup-identity")
    )
    held_id = inspected.get("Id")
    config = inspected.get("Config")
    labels = config.get("Labels") if isinstance(config, dict) else None
    if (
        not isinstance(labels, dict)
        or labels.get("io.boxferry.devcontainer.run") != token
        or not isinstance(held_id, str)
        or re.fullmatch(r"[a-f0-9]{64}", held_id) is None
        or (re.fullmatch(r"[a-f0-9]{64}", identity) and held_id != identity)
    ):
        raise ValidationError("cleanup refuses a container without exact run ownership")
    commands.call((*engine, "rm", "--force", held_id), "cleanup-container")
    remaining = commands.call(
        (
            *engine,
            "container",
            "ls",
            "--all",
            "--no-trunc",
            "--filter",
            "id=" + held_id,
            "--format",
            "{{.ID}}",
        ),
        "cleanup-container-absence",
    )
    if remaining.strip():
        raise ValidationError("exact owned container remains after cleanup")


def cleanup_image_tag(
    commands: Commands,
    engine: tuple[str, ...],
    tag: str,
    image_id: str | None,
    build_completed: bool,
    report: dict[str, object],
) -> None:
    # A failed build may leave layers without ever creating the requested tag.
    # A successful listing, rather than an arbitrary inspect failure, proves absence.
    listed = commands.call(
        (
            *engine,
            "image",
            "ls",
            "--all",
            "--no-trunc",
            "--filter",
            "reference=" + tag,
            "--format",
            "{{.ID}}",
        ),
        "cleanup-image-list",
    ).strip()
    if not listed:
        if build_completed:
            raise ValidationError("completed build is missing its exact image tag")
        report["cleanup_image_tag"] = "absent-after-failed-build"
        return
    listed_id = _image_identity(listed)
    inspected_id = _inspection(
        commands.call((*engine, "image", "inspect", tag), "cleanup-image-identity")
    ).get("Id")
    cleanup_image_id = _image_identity(inspected_id)
    report["cleanup_image_id"] = cleanup_image_id
    if cleanup_image_id != listed_id or (image_id is not None and cleanup_image_id != image_id):
        raise ValidationError("cleanup refuses substituted image tag")
    # Remove only our tag. Never force-remove a shared image ID or prune.
    commands.call((*engine, "image", "rm", tag), "cleanup-image-tag")
    remaining = commands.call(
        (
            *engine,
            "image",
            "ls",
            "--all",
            "--filter",
            "reference=" + tag,
            "--format",
            "{{.Repository}}:{{.Tag}}",
        ),
        "cleanup-image-absence",
    )
    if remaining.strip():
        raise ValidationError("owned image tag remains after cleanup")


def verify_private_store_idle(
    commands: Commands,
    engine: tuple[str, ...],
    private: Path,
    architecture: str,
    rootless: bool,
    identity: tuple[int, int],
    storage_driver: str | None = None,
    owner_uid: int | None = None,
) -> None:
    """Authorize deletion only for the exact run's local, container-free unmounted store."""
    facts = private.lstat()
    if (
        not stat.S_ISDIR(facts.st_mode)
        or (facts.st_dev, facts.st_ino) != identity
        or (
            owner_uid is not None
            and (facts.st_uid != owner_uid or stat.S_IMODE(facts.st_mode) != 0o700)
        )
        or any((private / name).is_symlink() for name in ("store", "runroot"))
    ):
        raise ValidationError("cleanup refuses substituted private store paths")
    info = _json(
        commands.call((*engine, "info", "--format", "{{json .}}"), "cleanup-store-identity")
    )
    validate_engine(
        info, "podman", architecture, private, rootless=rootless, storage_driver=storage_driver
    )
    mount_engine = (*engine, "unshare", *engine) if rootless else engine
    for argv, stage in (
        ((*engine, "ps", "--all", "--external", "--format", "json"), "cleanup-store-containers"),
        ((*mount_engine, "image", "mount", "--format", "json"), "cleanup-store-image-mounts"),
        (
            (*mount_engine, "container", "mount", "--format", "json"),
            "cleanup-store-container-mounts",
        ),
    ):
        if json.loads(commands.call(argv, stage)) != []:
            raise ValidationError(f"{stage}: private store is not empty or evidence is malformed")


def inspect_image(document: object, architecture: str) -> str:
    if not isinstance(document, list) or len(document) != 1 or not isinstance(document[0], dict):
        raise ValidationError("engine returned malformed built-image inspection")
    image = document[0]
    identity = _image_identity(image.get("Id"))
    if image.get("Architecture") != architecture or image.get("Os") != "linux":
        raise ValidationError("built image differs from the actual native host architecture")
    config = image.get("Config")
    if not isinstance(config, dict) or config.get("Volumes") or config.get("Env") is None:
        raise ValidationError("built image declares unsupported anonymous volumes or environment")
    return identity


def _runtime_environment(
    config: dict[str, Any], sources: Mapping[str, str], repository: str
) -> list[str]:
    values = {
        **config["containerEnv"],
        "HOME": "/tmp/qualification-home",
        "CI": "true",
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": "/dev/null",
        "GIT_TERMINAL_PROMPT": "0",
        "LC_ALL": "C.UTF-8",
        "RUSTUP_HOME": "/usr/local/rustup",
        "PATH": "/home/vscode/.local/bin:/usr/local/cargo/bin:/usr/local/sbin:"
        "/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        "GIT_CONFIG_COUNT": str(len(sources)),
    }
    for index, name in enumerate(sources):
        target = (
            f"/workspaces/{repository}"
            if name == repository
            else f"/workspaces/boxferry/.boxferry-workspace/{name}"
        )
        values[f"GIT_CONFIG_KEY_{index}"] = "safe.directory"
        values[f"GIT_CONFIG_VALUE_{index}"] = target
    return [f"{name}={value}" for name, value in values.items()]


def probe_script(versions: Mapping[str, str], sources: Mapping[str, str], repository: str) -> str:
    lines = [
        "set -euo pipefail",
        'test "$(id -u)" -gt 0',
        "mkdir -p /tmp/qualification-home",
        'test "$(uname -m)" = "$1"',
    ]
    for name, revision in sources.items():
        destination = (
            f"/workspaces/{repository}"
            if name == repository
            else f"/workspaces/boxferry/.boxferry-workspace/{name}"
        )
        lines.append(
            f'test "$(git -C {shlex.quote(destination)} rev-parse HEAD)" = {shlex.quote(revision)}'
        )
    for argv in PROBES:
        if repository != "boxferry" and argv[0] in {"helm", "kind", "kubectl", "kustomize"}:
            continue
        command = shlex.join(argv)
        name = argv[0]
        lines.append("printf '%s\\n' " + shlex.quote("probe: " + name))
        if name in versions:
            lines.append(f"observed=$({command})")
            if name == "node":
                expected = "v" + versions[name]
                lines.append('actual="$observed"')
            else:
                expected = name + " " + versions[name]
                lines.append('read -r observed_name observed_version _ <<< "$observed"')
                lines.append('actual="$observed_name $observed_version"')
            lines.extend(
                (
                    "expected=" + shlex.quote(expected),
                    'if [[ "$actual" != "$expected" ]]; then',
                    "  printf 'Version mismatch for %s: expected %.160s; observed %.160s\\n' "
                    + shlex.quote(name)
                    + ' "$expected" "$actual" >&2',
                    "  exit 1",
                    "fi",
                )
            )
        else:
            lines.append(command + " >/dev/null")
    lines.append("bash .devcontainer/verify-tools.sh")
    return "\n".join(lines)


def validate_binding(candidate: str, policy: str, event: str, payload: Mapping[str, Any]) -> None:
    if event == "pull_request":
        expected = payload.get("pull_request", {}).get("base", {}).get("sha")
    elif event == "merge_group":
        expected = payload.get("merge_group", {}).get("base_sha")
    elif event in {"push", "workflow_dispatch"}:
        expected = candidate
    else:
        raise ValidationError("unsupported hosted qualification event")
    if exact_sha(expected, "trusted event policy SHA") != policy:
        raise ValidationError(
            "qualification policy differs from the independently trusted event base"
        )


def _verify_origin(root: Path, repository: str, commands: Commands) -> None:
    origin = commands.git(root, "remote", "get-url", "origin").strip()
    if origin not in {
        f"https://github.com/Strukturpiloten/{repository}.git",
        f"git@github.com:Strukturpiloten/{repository}.git",
    }:
        raise ValidationError("source origin differs from its scoped workspace repository")


def _local_sibling_source(
    name: str, source: Path, paths: Mapping[str, Path] | None, revision: str, commands: Commands
) -> Path:
    if name not in SIBLINGS:
        raise ValidationError("local sibling is outside the scoped workspace repositories")
    local = paths[name] if paths is not None else source.parent / name
    _held_revision(local, revision, commands)
    _verify_origin(local, name, commands)
    return local.resolve()


def qualify(
    *,
    repository: str,
    candidate: str,
    policy_sha: str,
    automation_sha: str,
    source: Path,
    policy: Path,
    automation: Path,
    manifest: Mapping[str, object],
    engine: str,
    mode: str,
    work: Path,
    expected_architecture: str | None = None,
    local_sources: Mapping[str, Path] | None = None,
    outer_mode: str | None = None,
    lifetime_context: dict[str, Any] | None = None,
) -> dict[str, object]:
    """Execute a fresh build and verifier; every error including cleanup blocks success."""
    if repository not in OWNERS:
        raise ValidationError("repository does not own a Dev Container")
    candidate = exact_sha(candidate, "candidate SHA")
    policy_sha = exact_sha(policy_sha, "policy SHA")
    automation_sha = exact_sha(automation_sha, "automation SHA")
    sources = parse_sources(manifest, repository, candidate)
    if local_sources is not None and (
        mode != "local"
        or repository != "boxferry"
        or set(local_sources) != set(SIBLINGS)
        or any(not isinstance(path, Path) for path in local_sources.values())
    ):
        raise ValidationError(
            "explicit local Box sources require exactly the six scoped sibling paths"
        )
    selected = backend(repository, mode, engine)
    effective = effective_outer_mode(mode, selected, outer_mode)
    caller_uid = os.getuid()
    machine = platform.machine()
    if platform.system() != "Linux" or machine not in ARCHITECTURES:
        raise ValidationError("qualification requires an actual supported native Linux host")
    if mode == "hosted" and os.environ.get("GITHUB_SHA") != candidate:
        raise ValidationError("hosted tested candidate differs from the current workflow SHA")
    if expected_architecture is not None and expected_architecture != ARCHITECTURES[machine]:
        raise ValidationError("native host differs from the required architecture lane")
    if mode == "hosted":
        if os.environ.get("GITHUB_REPOSITORY") != f"Strukturpiloten/{repository}":
            raise ValidationError("hosted repository differs from the scoped candidate owner")
        event_path = Path(os.environ.get("GITHUB_EVENT_PATH", "/nonexistent"))
        if event_path.stat().st_size > 2 * 1024**2:
            raise ValidationError("hosted event evidence exceeds its byte bound")
        validate_binding(
            candidate,
            policy_sha,
            os.environ.get("GITHUB_EVENT_NAME", ""),
            _json(event_path.read_bytes()),
        )
    if lifetime_context is not None and not (
        mode == "local" and selected == "podman" and effective == "rootful"
    ):
        raise ValidationError("private lifetime context is local rootful only")
    if mode == "local" and selected == "podman" and effective == "rootful":
        from . import devcontainer_lifetime

        if lifetime_context is None:
            installed_podman()  # Before even the outer namespace's first sudo invocation.
            return devcontainer_lifetime.execute(
                {
                    "repository": repository,
                    "candidate": candidate,
                    "policy_sha": policy_sha,
                    "automation_sha": automation_sha,
                    "source": source,
                    "policy": policy,
                    "automation": automation,
                    "manifest": dict(manifest),
                    "engine": engine,
                    "mode": mode,
                    "work": work,
                    "expected_architecture": expected_architecture,
                    "local_sources": local_sources,
                    "outer_mode": outer_mode,
                }
            )
        private = devcontainer_lifetime.validate_worker_context(lifetime_context)
    else:
        private = Path(
            tempfile.mkdtemp(
                prefix="bfv-" if selected == "podman" else "devcontainer-",
                dir="/tmp" if selected == "podman" else work,
            )
        )
    private.chmod(0o700)
    private_facts = private.lstat()
    private_identity = (private_facts.st_dev, private_facts.st_ino)
    commands = Commands(private)
    commands.lifetime_context = lifetime_context
    commands.evidence_paths = (source, policy, automation, work)
    report: dict[str, object] = {
        "schema": 1,
        "status": "failure",
        "candidate_sha": candidate,
        "policy_sha": policy_sha,
        "automation_sha": automation_sha,
        "architecture": ARCHITECTURES[machine],
        "engine": selected,
        "workspace_sources": dict(sources),
        "source_manifest_sha256": manifest_digest(manifest),
        "scope": "image-tooling-and-workspace-smoke",
        "cleanup": "pending",
        "mode": mode,
        "caller_uid": caller_uid,
        "requested_outer_mode": effective,
        "private_directory_identity": (
            f"{private_facts.st_dev}:{private_facts.st_ino}:{private_facts.st_uid}:"
            f"{stat.S_IMODE(private_facts.st_mode):o}"
        ),
    }
    commands.lifetime_report = report
    token = uuid.uuid4().hex
    name, tag = f"boxferry-devcontainer-{token}", f"localhost/boxferry-devcontainer:{token}"
    report["run_nonce"] = token
    executable = _engine(
        private,
        selected,
        mode,
        rootful=effective == "rootful",
        local_profile=lifetime_context is not None,
    )
    engine_argv = (executable,)
    container_id: str | None = None
    image_id: str | None = None
    created = False
    build_started = False
    build_completed = False
    engine_verified = False
    try:
        commands.budget = Budget.start(private, BUILD_SECONDS)
        _held_revision(source, candidate, commands, clean=mode != "local")
        _verify_origin(source, repository, commands)
        _held_revision(policy, policy_sha, commands)
        _verify_origin(policy, repository, commands)
        _held_revision(automation, automation_sha, commands)
        _verify_origin(automation, "boxferry", commands)
        held = fingerprint(source, commands)
        report["input_sha256"] = held
        prerequisite = None
        if selected == "podman" and mode == "hosted":
            prerequisite = prepare_hosted_podman(commands)
            report["outer_prerequisite"] = prerequisite
        if selected == "podman" and mode == "local":
            if not Path("/usr/bin/podman").is_file():
                raise ValidationError(
                    "local qualification requires installed /usr/bin/podman; no installer fallback"
                )
            if effective == "rootful":
                report["installed_podman"] = installed_podman()
            privilege, clean = _rootful_bridge(private)
            observer = (
                (*privilege, "5s", *clean, "/usr/bin/id", "-u")
                if effective == "rootful"
                else ("/usr/bin/id", "-u")
            )
            observed_uid = commands.call(observer, "engine-process-uid", 8).strip()
            report["engine_uid"] = (
                int(observed_uid)
                if re.fullmatch(r"[0-9]{1,10}", observed_uid)
                else _bounded_text(observed_uid, 64)
            )
            if observed_uid != str(0 if effective == "rootful" else caller_uid):
                raise ValidationError(
                    "actual engine bridge UID differs from the requested outer mode"
                )
        version = _json(
            commands.call((*engine_argv, "version", "--format", "{{json .}}"), "engine-version")
        )
        validate_engine_version(version, selected)
        report["engine_version"] = version
        info = _json(commands.call((*engine_argv, "info", "--format", "{{json .}}"), "engine-info"))
        if selected == "podman":
            host = info.get("host", {})
            security = host.get("security", {}) if isinstance(host, dict) else {}
            observed_rootless = security.get("rootless") if isinstance(security, dict) else None
            report["outer_mode"] = (
                "rootless"
                if observed_rootless is True
                else "rootful"
                if observed_rootless is False
                else "unknown"
            )
        expected_rootless = effective == "rootless"
        private_driver = "overlay" if mode == "local" and effective == "rootful" else None
        validate_engine(
            info,
            selected,
            ARCHITECTURES[machine],
            private,
            rootless=expected_rootless,
            storage_driver=private_driver,
        )
        engine_verified = True
        if selected == "podman" and mode == "local" and effective == "rootful":
            host, store = info["host"], info["store"]
            if host.get("cgroupManager") != "cgroupfs" or host.get("eventLogger") != "file":
                raise ValidationError(
                    "explicit local cgroupfs/file-events profile was not observed"
                )
            runtime_facts = host.get("ociRuntime", {})
            runtime_receipt = installed_runc(commands)
            name_representation = _runtime_representation(runtime_facts, "name", runtime_receipt)
            path_representation = _runtime_representation(runtime_facts, "path", runtime_receipt)
            report.update(
                engine_runtime_name_representation=name_representation,
                engine_runtime_path_representation=path_representation,
            )
            if name_representation not in {"alias-runc", "selected-path", "resolved-path"}:
                raise ValidationError("explicit local runc runtime was not observed")
            if path_representation not in {"selected-path", "resolved-path"}:
                raise ValidationError("explicit installed /usr/bin/runc path was not observed")
            report["installed_runtime"] = runtime_receipt
            # Accepted aliases/paths describe the same custody-checked binary. Retain the
            # canonical identity, never an unchecked native name in normal or abort reports.
            manager = host.get("cgroupManager")
            report.update(
                engine_client_version=version["Client"]["Version"],
                engine_storage_driver=store["graphDriverName"],
                engine_graph_root=store["graphRoot"],
                engine_run_root=store["runRoot"],
                engine_remote=host["serviceIsRemote"],
                engine_cgroup_manager=_bounded_text(manager, 128)
                if isinstance(manager, str)
                else "unknown",
                engine_runtime="runc",
            )
        if selected == "docker" and mode == "hosted":
            validate_docker_budget_filesystem(private, info["DockerRootDir"], report)
        report["runner"] = {"os": "linux", "architecture": ARCHITECTURES[machine], "mode": mode}
        if selected == "podman":
            observed = info.get("version")
            if (
                not isinstance(observed, dict)
                or observed.get("Version") != version["Client"]["Version"]
            ):
                raise ValidationError("Podman info and client versions disagree")
            report["outer_mode"] = "rootless" if expected_rootless else "rootful"
            if prerequisite:
                prerequisite["version"] = observed["Version"]
        snapshots: dict[str, Path] = {}
        for name_in_manifest, revision in sources.items():
            destination = private / name_in_manifest
            if name_in_manifest == repository:
                remote = str(source.resolve())
            elif mode == "local":
                remote = str(
                    _local_sibling_source(
                        name_in_manifest, source, local_sources, revision, commands
                    )
                )
            else:
                remote = f"https://github.com/Strukturpiloten/{name_in_manifest}.git"
            _checkout(destination, revision, commands, remote)
            snapshots[name_in_manifest] = destination
        snapshot = snapshots[repository]
        if mode == "local":
            _overlay(source, snapshot, commands)
        if fingerprint(snapshot, commands) != held:
            raise ValidationError("private candidate snapshot differs from held input closure")
        config = validate_configuration(snapshot, repository)
        if repository == "boxferry":
            for sibling in SIBLINGS:
                (snapshot / ".boxferry-workspace" / sibling).mkdir(parents=True, exist_ok=True)
        ignores = exclude_git_build_metadata(snapshot)
        context_held = build_context_fingerprint(snapshot, commands, ignores)
        report["build_context_sha256"] = context_held
        report["build_context_ignore_sha256"] = {
            name: binding["sha256"] for name, binding in ignores.items()
        }
        report["build_context_ignore_modes"] = {
            name: binding["mode"] for name, binding in ignores.items()
        }
        report["git_metadata_excluded_from_build"] = True
        cli, cli_version = _cli(automation)
        node = shutil.which("node")
        if node is None or not Path(node).is_file():
            raise ValidationError("trusted Node executable is unavailable")
        node_version = commands.call((node, "--version"), "node-version").strip()
        if node_version != "v" + canonical_versions(automation)["node"]:
            raise ValidationError("host Node differs from selected canonical automation authority")
        actual_cli = commands.call((node, str(cli), "--version"), "cli-version").strip()
        if actual_cli != cli_version:
            raise ValidationError("installed official CLI differs from its canonical lock")
        report["cli_version"] = cli_version
        # CLI build does not run lifecycle hooks. Runtime comes from our finite argv,
        # never candidate runArgs, mounts, host hooks, devcontainer up or a socket.
        if commands.call(
            (*engine_argv, "image", "ls", tag, "--format", "{{.ID}}"), "owned-tag-absence"
        ).strip():
            raise ValidationError("unique qualification image tag already exists")
        if commands.call(
            (
                *engine_argv,
                "container",
                "ls",
                "--all",
                "--filter",
                "name=" + name,
                "--format",
                "{{.ID}}",
            ),
            "owned-container-absence",
        ).strip():
            raise ValidationError("unique qualification container name already exists")
        build_started = True
        output = commands.call(
            (
                node,
                str(cli),
                "build",
                "--workspace-folder",
                str(snapshot),
                "--config",
                str(snapshot / CONFIG),
                "--docker-path",
                executable,
                "--platform",
                "linux/" + ARCHITECTURES[machine],
                "--image-name",
                tag,
                "--experimental-frozen-lockfile",
            ),
            "build",
            BUILD_SECONDS,
        )
        build_completed = True
        lines = output.strip().splitlines()
        if not lines:
            raise ValidationError("official CLI returned no build outcome")
        result = _json(lines[-1])
        if result.get("outcome") != "success" or result.get("imageName") != [tag]:
            raise ValidationError(
                "official CLI did not return the exact requested built-image identity"
            )
        image_id = inspect_image(
            json.loads(commands.call((*engine_argv, "image", "inspect", tag), "image-inspect")),
            ARCHITECTURES[machine],
        )
        report["image_id"] = image_id
        commands.budget = Budget(
            private, commands.budget.initial_free, time.monotonic() + PROBE_SECONDS
        )
        # Copy inert passwd bytes from a held stopped container. No untracked
        # `run --rm` discovery process can outlive cancellation or escape cleanup.
        created = True
        container_id = commands.call(
            (
                *engine_argv,
                "create",
                "--name",
                name,
                "--label",
                f"io.boxferry.devcontainer.run={token}",
                "--network",
                "none",
                "--read-only",
                "--user",
                config["remoteUser"],
                "--entrypoint",
                "/bin/true",
                image_id,
            ),
            "create-user-inspection",
        ).strip()
        if re.fullmatch(r"[a-f0-9]{64}", container_id) is None:
            raise ValidationError("engine returned an invalid stopped-container identity")
        passwd = private / "image-passwd"
        commands.call(
            (*engine_argv, "cp", f"{container_id}:/etc/passwd", str(passwd)), "read-image-user"
        )
        identity, group = passwd_identity(passwd, config["remoteUser"])
        remove_owned_container(commands, engine_argv, container_id, token)
        created, container_id = False, None
        report["uid"], report["gid"] = int(identity), int(group)
        mounts: list[str] = []
        for item, path in snapshots.items():
            target = (
                config["workspaceFolder"]
                if item == repository
                else f"/workspaces/boxferry/.boxferry-workspace/{item}"
            )
            mounts.extend(("--mount", f"type=bind,source={path},target={target},readonly"))
        for key in ("CARGO_HOME", "GH_CONFIG_DIR", "CARGO_TARGET_DIR"):
            if key not in config["containerEnv"]:
                continue
            mounts.extend(
                (
                    "--tmpfs",
                    f"{config['containerEnv'][key]}:rw,nosuid,nodev,uid={identity},gid={group},mode=0700,size=256m",
                )
            )
        argv = (
            *engine_argv,
            "create",
            "--name",
            name,
            "--label",
            f"io.boxferry.devcontainer.run={token}",
            "--network",
            "none",
            "--read-only",
            "--cap-drop",
            "ALL",
            "--security-opt",
            "no-new-privileges",
            "--pids-limit",
            "512",
            "--memory",
            "4g",
            "--cpus",
            "2",
            "--tmpfs",
            "/tmp:rw,nosuid,nodev,size=256m",
            "--user",
            f"{identity}:{group}",
            "--workdir",
            config["workspaceFolder"],
            *mounts,
            "--entrypoint",
            "/usr/bin/env",
            image_id,
            "-i",
            *_runtime_environment(config, sources, repository),
            "/bin/bash",
            "-c",
            probe_script(canonical_versions(automation), sources, repository),
            "qualification",
            machine,
        )
        created = True  # Track interrupted creates by unique name plus held owner label.
        container_id = commands.call(argv, "create-probe").strip()
        if re.fullmatch(r"[a-f0-9]{64}", container_id) is None:
            raise ValidationError("engine did not return one exact owned container ID")
        live: LiveProcessObservation | None = None
        attach_options: dict[str, Any] = {}
        if lifetime_context is not None:
            installed = report["installed_podman"]
            assert isinstance(installed, dict)
            live = LiveProcessObservation(
                commands,
                LiveHeldIdentity(
                    container_id,
                    image_id,
                    token,
                    int(identity),
                    str(installed["sha256"]),
                    str(installed["version"]),
                ),
            )
            attach_options["while_running"] = live.tick
        commands.call(
            (*engine_argv, "start", "--attach", container_id),
            "tooling-workspace-probe",
            PROBE_SECONDS,
            **attach_options,
        )
        if live is not None:
            live.require_complete()
        state = _inspection(
            commands.call((*engine_argv, "container", "inspect", container_id), "probe-state")
        )
        if (
            not isinstance(state.get("State"), dict)
            or state["State"].get("ExitCode") != 0
            or _image_identity(state.get("Image")) != image_id
        ):
            raise ValidationError(
                "exact-image verifier exited unsuccessfully or changed image identity"
            )
        _held_revision(source, candidate, commands, clean=mode != "local")
        _held_revision(policy, policy_sha, commands)
        _held_revision(automation, automation_sha, commands)
        if (
            fingerprint(source, commands) != held
            or build_context_fingerprint(snapshot, commands, ignores) != context_held
        ):
            raise ValidationError("candidate input closure changed during image qualification")
        report["status"] = "success"
    except (OSError, ValueError, RunCancelled, KeyboardInterrupt) as error:
        report["cause"] = redact(str(error), protected_values(os.environ))[:4096]
        if lifetime_context is not None and not lifetime_context.get("abort_notified"):
            from .devcontainer_lifetime import notify_abort
            from .runner import AbortContext

            try:
                notify_abort(
                    lifetime_context,
                    AbortContext("qualification", "infrastructure", str(report["cause"])),
                    report,
                )
            except (OSError, ValueError) as secondary:
                report["abort_hook_failure"] = redact(str(secondary), protected_values(os.environ))[
                    :512
                ]
    finally:
        if lifetime_context is not None and lifetime_context.get("abort_notified"):
            report.update(
                status="failure",
                cleanup="pending-outer-closure",
                directory_cleanup="deferred-to-outer",
            )
            report["outcomes"] = commands.outcomes
        else:
            commands.budget = Budget(
                private,
                shutil.disk_usage(private).free,
                time.monotonic() + CLEANUP_SECONDS,
                check_disk=False,
            )
            try:
                if mode == "local" and effective == "rootful":
                    current = private.lstat()
                    if private.is_symlink() or (
                        current.st_dev,
                        current.st_ino,
                        current.st_uid,
                        current.st_mode,
                    ) != (
                        private_facts.st_dev,
                        private_facts.st_ino,
                        private_facts.st_uid,
                        private_facts.st_mode,
                    ):
                        raise ValidationError(
                            "rootful cleanup refuses changed private directory custody"
                        )
                    if (private / "store").exists() and not engine_verified:
                        raise ValidationError(
                            "unverified rootful engine cannot authorize private-store cleanup"
                        )
                if created:
                    remove_owned_container(commands, engine_argv, container_id or name, token)
                if build_started:
                    cleanup_image_tag(commands, engine_argv, tag, image_id, build_completed, report)
                report["cleanup"] = "success"
            except (OSError, ValueError, RunCancelled, KeyboardInterrupt) as error:
                report["status"], report["cleanup"] = "failure", "failure"
                report["cleanup_cause"] = redact(str(error), protected_values(os.environ))[:4096]
            report["outcomes"] = commands.outcomes
            # Private Podman stores must be container-free/unmounted before directory removal;
            # normal shutdown never deletes another store or global Docker resources.
            if (
                selected == "podman"
                and (private / "store").exists()
                and report["cleanup"] == "success"
            ):
                try:
                    verify_private_store_idle(
                        commands,
                        engine_argv,
                        private,
                        ARCHITECTURES[machine],
                        rootless=effective == "rootless",
                        identity=private_identity,
                        storage_driver="overlay"
                        if mode == "local" and effective == "rootful"
                        else None,
                        owner_uid=caller_uid
                        if mode == "local" and effective == "rootful"
                        else None,
                    )
                    if effective == "rootful" and lifetime_context is None:
                        owned = tuple(
                            str(private / name)
                            for name in (
                                "store",
                                "runroot",
                                "podman-tmp",
                                "home",
                                "config",
                                "cache",
                                "runtime",
                                "docker-config",
                            )
                        )
                        commands.call(
                            (
                                "/usr/bin/sudo",
                                "-n",
                                "/usr/bin/timeout",
                                "--signal=TERM",
                                "--kill-after=2s",
                                "60s",
                                "/bin/rm",
                                "-rf",
                                "--",
                                *owned,
                            ),
                            "cleanup-private-rootful-store",
                            65,
                        )
                    elif effective != "rootful":
                        commands.call(
                            (*engine_argv, "unshare", "rm", "-rf", str(private / "store")),
                            "cleanup-private-store",
                            60,
                        )
                except (OSError, ValueError, RunCancelled, KeyboardInterrupt) as error:
                    report["status"], report["cleanup"] = "failure", "failure"
                    report["cleanup_cause"] = str(error)[:4096]
            try:
                if lifetime_context is not None:
                    report["directory_cleanup"] = "deferred-to-outer"
                    report["engine_resource_cleanup"] = report["cleanup"]
                elif report["cleanup"] == "success":
                    shutil.rmtree(private)
                else:
                    report["retained_private_directory"] = str(private)
            except OSError as error:
                report["status"], report["cleanup"] = "failure", "failure"
                report["cleanup_cause"] = str(error)[:4096]
    return report


def local_sources(
    repository: str, candidate: str, source: Path, commands: Commands
) -> dict[str, object]:
    revisions = {repository: candidate}
    if repository == "boxferry":
        for name in SIBLINGS:
            path = source.parent / name
            revision = exact_sha(
                commands.git(path, "rev-parse", "HEAD").strip(), "local sibling SHA"
            )
            _held_revision(path, revision, commands)
            _verify_origin(path, name, commands)
            revisions[name] = revision
    return {"schema": 1, "repositories": revisions}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("resolve", "qualify", "prepare-podman"))
    parser.add_argument("--repository", required=True, choices=sorted(OWNERS))
    parser.add_argument("--candidate-sha", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--github-output", type=Path)
    parser.add_argument("--source", type=Path)
    parser.add_argument("--policy", type=Path)
    parser.add_argument("--automation", type=Path)
    parser.add_argument("--policy-sha")
    parser.add_argument("--automation-sha")
    parser.add_argument("--sources-json")
    parser.add_argument("--engine", choices=("podman", "docker"))
    parser.add_argument("--architecture", choices=("amd64", "arm64"))
    parser.add_argument("--mode", choices=("local", "hosted"), default="hosted")
    args = parser.parse_args(argv)
    result: dict[str, object]
    previous = signal.signal(
        signal.SIGTERM,
        lambda _signum, _frame: (_ for _ in ()).throw(RunCancelled("workflow cancellation")),
    )
    try:
        if args.command == "prepare-podman":
            with tempfile.TemporaryDirectory(prefix="bfv-prerequisite-") as temporary:
                commands = Commands(Path(temporary))
                result = {
                    "schema": 1,
                    "status": "success",
                    "prerequisite": prepare_hosted_podman(commands),
                    "outcomes": commands.outcomes,
                }
        elif args.command == "resolve":
            result = resolve_sources(args.repository, args.candidate_sha)
        else:
            if not all(
                (
                    args.source,
                    args.policy,
                    args.automation,
                    args.policy_sha,
                    args.automation_sha,
                    args.engine,
                )
            ):
                raise ValidationError(
                    "qualification requires explicit candidate, policy, automation "
                    "and backend identities"
                )
            if args.mode == "local":
                effective_outer_mode("local", args.engine)
            args.output.parent.mkdir(parents=True, exist_ok=True)
            if args.sources_json:
                manifest = _json(args.sources_json)
            elif args.mode == "local":
                with tempfile.TemporaryDirectory(prefix="devcontainer-local-sources-") as temporary:
                    manifest = local_sources(
                        args.repository, args.candidate_sha, args.source, Commands(Path(temporary))
                    )
            else:
                raise ValidationError(
                    "hosted qualification needs the once-resolved shared source manifest"
                )
            result = qualify(
                repository=args.repository,
                candidate=args.candidate_sha,
                policy_sha=args.policy_sha,
                automation_sha=args.automation_sha,
                source=args.source.resolve(),
                policy=args.policy.resolve(),
                automation=args.automation.resolve(),
                manifest=manifest,
                engine=args.engine,
                mode=args.mode,
                work=args.output.parent.resolve(),
                expected_architecture=args.architecture,
            )
    except (OSError, ValueError, RunCancelled, KeyboardInterrupt) as error:
        result = {
            "schema": 1,
            "status": "failure",
            "cause": redact(str(error), protected_values(os.environ))[:4096],
        }
    finally:
        signal.signal(signal.SIGTERM, previous)
    encoded = serialize_report(result)
    result = json.loads(encoded)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(encoded + "\n")
    if args.github_output and args.command == "resolve" and "repositories" in result:
        with args.github_output.open("a") as stream:
            stream.write(f"sources={encoded}\nsource_digest={manifest_digest(result)}\n")
            if args.automation:
                stream.write(f"node_version={canonical_versions(args.automation)['node']}\n")
    if args.github_output and args.command == "qualify" and result.get("status") == "success":
        with args.github_output.open("a") as stream:
            for name in (
                "candidate_sha",
                "source_manifest_sha256",
                "input_sha256",
                "build_context_sha256",
                "architecture",
            ):
                stream.write(f"{name}={result[name]}\n")
    return (
        0
        if (args.command == "resolve" and "repositories" in result)
        or result.get("status") == "success"
        else 1
    )


if __name__ == "__main__":
    sys.exit(main())
