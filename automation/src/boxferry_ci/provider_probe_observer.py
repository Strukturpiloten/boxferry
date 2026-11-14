#!/usr/bin/env python3
"""Bound manual provider observations, derived from the preserved authored v5 observer.

No automatic poll loop, retry, prefix join or production recovery classifier.
Collection requires separate explicit authorization and fresh exact-source custody.
"""

from __future__ import annotations

import argparse
import contextlib
import datetime as dt
import fcntl
import hashlib
import http.client
import io
import ipaddress
import json
import math
import os
import re
import selectors
import signal
import stat
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from collections.abc import Generator
from pathlib import Path
from typing import Any, Never, cast

REPO = "Strukturpiloten/boxferry"
WORKFLOW = ".github/workflows/recovery-cancellation-provider-probe.yml"
SCHEMA = "boxferry.provider-probe.v1"
OBSERVER_SCHEMA = "boxferry.provider-probe.observer.v1"
CATALOG = {
    "629 preparation call": ("prepare", "outer-wrapper"),
    "629 preparation call / 629 preparation witness": ("prepare", "expanded-leaf"),
    "629 inert effect call": ("effect", "outer-wrapper"),
    "629 inert effect call / 629 inert dependent witness": ("effect", "expanded-leaf"),
}
HOLDS = {"prepare": "Fixed preparation hold", "effect": "Fixed inert dependent hold"}
MAX_REQUESTS, MAX_SECONDS, MAX_JSON, MAX_ARCHIVE = 400, 1800, 1024 * 1024, 256 * 1024
REGISTRATION_ROOT = Path("/tmp")
MAX_CUSTODY = 64 * 1024 * 1024
REDIRECTS = {301, 302, 303, 307, 308}


class Inconclusive(Exception):
    pass


def require(condition: object, reason: str) -> None:
    if not condition:
        raise Inconclusive(reason)


def object_field(parent: object, key: str) -> dict[str, Any]:
    require(
        isinstance(parent, dict) and isinstance(parent.get(key), dict),
        "required " + key + " metadata must be an object",
    )
    return cast(dict[str, Any], parent)[key]


def positive(value: object) -> int:
    require(type(value) is int and 0 < value < 10**20, "invalid positive provider identity")
    return cast(int, value)


def sha(value: object) -> str:
    require(isinstance(value, str) and re.fullmatch("[a-f0-9]{40}", value), "invalid source SHA")
    return cast(str, value)


def digest(value: object) -> str:
    require(isinstance(value, str) and re.fullmatch("[a-f0-9]{64}", value), "invalid SHA256")
    return cast(str, value)


def utc() -> str:
    return dt.datetime.now(dt.UTC).isoformat()


def unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate JSON field")
        result[key] = value
    return result


def invalid_json_constant(value: str) -> Never:
    raise Inconclusive("invalid bounded JSON")


def finite_json_float(value: str) -> float:
    result = float(value)
    require(math.isfinite(result), "invalid bounded JSON")
    return result


def decode(raw: bytes) -> dict[str, Any]:
    try:
        return json.loads(
            raw,
            object_pairs_hook=unique_object,
            parse_constant=invalid_json_constant,
            parse_float=finite_json_float,
        )
    except (ValueError, UnicodeDecodeError, RecursionError):
        raise Inconclusive("invalid bounded JSON") from None


def run_identity_facts(value: object) -> dict[str, Any]:
    """Finite identity observations without provider text or numeric coercion."""
    facts: dict[str, Any] = {
        "observed_source_sha": None,
        "observed_repository_matches": None,
        "observed_created_at": None,
        "event_matches": None,
        "ref_matches": None,
        "workflow_path_matches": None,
    }
    numeric = {
        "id": "observed_run_id",
        "run_attempt": "observed_attempt",
        "workflow_id": "observed_workflow_id",
        "actor": "observed_actor_id",
        "triggering_actor": "observed_triggering_actor_id",
    }
    row = value if isinstance(value, dict) else {}
    for field, target in numeric.items():
        number = row.get(field)
        if field in {"actor", "triggering_actor"}:
            number = number.get("id") if isinstance(number, dict) else None
        facts[target] = number if type(number) is int and 0 < number < 10**20 else None
        facts[target + "_type"] = (
            "boolean"
            if type(number) is bool
            else "integer"
            if type(number) is int
            else "non-integer-number"
            if type(number) is float
            else "missing-or-null"
            if number is None
            else "other"
        )
    if not isinstance(value, dict):
        return facts
    source = row.get("head_sha")
    if isinstance(source, str) and re.fullmatch(r"[a-f0-9]{40}", source):
        facts["observed_source_sha"] = source
    repository = row.get("repository")
    facts["observed_repository_matches"] = (
        repository.get("full_name") == REPO if isinstance(repository, dict) else None
    )
    facts["event_matches"] = row.get("event") == "workflow_dispatch"
    facts["ref_matches"] = row.get("head_branch") == "main"
    facts["workflow_path_matches"] = row.get("path") == WORKFLOW
    created = row.get("created_at")
    if isinstance(created, str) and re.fullmatch(
        r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]{1,6})?(?:Z|\+00:00)",
        created,
    ):
        facts["observed_created_at"] = created
    return facts


@contextlib.contextmanager
def open_directory(path: str | Path) -> Generator[int]:
    """Walk directories through no-follow FDs before any mutation; never follow an ancestor link."""
    absolute = Path(path).absolute()
    descriptor = os.open("/", os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        for component in absolute.parts[1:]:
            following = os.open(
                component, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=descriptor
            )
            os.close(descriptor)
            descriptor = following
        yield descriptor
    finally:
        os.close(descriptor)


def checked_custody(path: str | Path) -> Path:
    value = Path(path).absolute()
    with open_directory(value.parent):
        require(not value.is_symlink(), "custody path contains a symlink")
    return value


def read_regular(path: Path, limit: int) -> bytes:
    """Bounded no-follow read for custody prechecks and immutable evidence."""
    with open_directory(path.parent) as directory:
        descriptor = os.open(
            path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory
        )
        with os.fdopen(descriptor, "rb") as stream:
            info = os.fstat(stream.fileno())
            require(
                stat.S_ISREG(info.st_mode) and info.st_size <= limit,
                "custody read is not a bounded regular file",
            )
            data = stream.read(limit + 1)
    require(len(data) <= limit, "custody read exceeds byte bound")
    return data


def write_json(path: Path, value: dict[str, Any], identity: tuple[int, int] | None = None) -> None:
    data = (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()
    require(len(data) <= MAX_CUSTODY, "custody record exceeds byte bound")
    with open_directory(path.parent) as directory:
        info = os.fstat(directory)
        require(
            identity is None or (info.st_dev, info.st_ino) == identity,
            "custody directory identity changed",
        )
        temporary = path.name + ".new"
        descriptor = os.open(
            temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=directory
        )
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path.name, src_dir_fd=directory, dst_dir_fd=directory)
        os.fsync(directory)


def token_from_gh() -> str:
    """Read only local gh credential state; never return it to stdout or a file."""
    env = {key: value for key, value in os.environ.items() if key not in {"GH_DEBUG", "GH_HOST"}}
    env.update(GH_PROMPT_DISABLED="1", GH_DEBUG="")
    process = subprocess.Popen(
        ["gh", "auth", "token", "--hostname", "github.com"],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        env=env,
    )
    data = bytearray()
    deadline = time.monotonic() + 5
    try:
        with selectors.DefaultSelector() as selector:
            assert process.stdout is not None
            selector.register(process.stdout, selectors.EVENT_READ)
            while selector.get_map():
                require(time.monotonic() < deadline, "local credential read timed out")
                for key, _ in selector.select(min(0.2, deadline - time.monotonic())):
                    chunk = os.read(key.fd, 1024)
                    if not chunk:
                        selector.unregister(key.fileobj)
                    else:
                        data.extend(chunk)
                        require(len(data) <= 16384, "local credential read exceeded bound")
        require(process.wait(timeout=1) == 0, "local gh credential unavailable")
        token = data.decode().strip()
        require(token and not re.search(r"\s", token), "invalid local credential shape")
        return token
    except (OSError, UnicodeError, subprocess.SubprocessError):
        raise Inconclusive("local gh credential unavailable") from None
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        if process.stdout:
            process.stdout.close()


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(
        self, req: object, fp: object, code: object, msg: object, headers: object, newurl: object
    ) -> None:
        return None


def safe_redirect(location: str) -> str:
    """Tighten eligibility.py's reviewed provider-issued HTTPS/no-userinfo policy.

    No caller-supplied URLs. No claim that one vendor host suffix is universal.
    The authenticated provider Location is the authority for a fresh unauthenticated
    HTTPS destination; signed URLs never enter persistent evidence or diagnostics.
    """
    require(isinstance(location, str) and len(location) <= 16384, "invalid redirect size")
    require(
        location.isascii()
        and not any(ord(character) < 33 or ord(character) == 127 for character in location),
        "redirect contains unsafe control or encoding",
    )
    try:
        value = urllib.parse.urlsplit(location)
        require(
            value.scheme == "https"
            and value.hostname
            and not value.username
            and not value.password
            and value.port in (None, 443)
            and not value.fragment,
            "redirect is not safe HTTPS",
        )
        host = cast(str, value.hostname)
        require("." in host and not host.endswith(".") and host.isascii(), "invalid redirect host")
        try:
            ipaddress.ip_address(host)
        except ValueError:
            pass
        else:
            raise Inconclusive("numeric redirect host refused")
        require(not any(character.isspace() for character in location), "invalid redirect encoding")
        require(not host.endswith((".localhost", ".local", ".internal")), "local redirect refused")
    except ValueError:
        raise Inconclusive("invalid redirect URL") from None
    return location


@contextlib.contextmanager
def hard_request_deadline(seconds: float) -> Generator[None]:
    """Bound the entire socket operation, including slow bodies, on this Linux host."""

    def expired(signum: int, frame: object) -> None:
        raise Inconclusive("request wall-clock deadline exhausted")

    previous = signal.signal(signal.SIGALRM, expired)
    signal.setitimer(signal.ITIMER_REAL, seconds)
    try:
        yield
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)


class Evidence:
    def __init__(
        self, directory: str | Path, *, read_only: bool = False, reconcile: bool = True
    ) -> None:
        self.read_only = read_only
        candidate = checked_custody(directory)
        with open_directory(candidate) as descriptor:
            info = os.fstat(descriptor)
            self.directory_identity = (info.st_dev, info.st_ino)
        self.directory = candidate
        self.state_path = self.directory / "state.json"
        require(
            not self.state_path.is_symlink()
            and self.state_path.is_file()
            and self.state_path.stat().st_size <= MAX_CUSTODY,
            "invalid custody file",
        )
        self.state = decode(read_regular(self.state_path, MAX_CUSTODY))
        require(
            isinstance(self.state, dict) and self.state.get("schema") == OBSERVER_SCHEMA,
            "observer custody schema mismatch; do not mix collector versions",
        )
        require(
            self.state.get("observer_sha256")
            == hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            "observer source changed since series initialization",
        )
        require(
            "source_descriptor" in self.state and "discovery_reads" in self.state,
            "legacy custody refused; initialize a new series",
        )
        require(
            isinstance(self.state["discovery_reads"], dict)
            and all(
                key in "ABCDE" and len(key) == 1 and type(value) is int and 0 <= value <= 3
                for key, value in self.state["discovery_reads"].items()
            ),
            "custody discovery ledger mismatch",
        )
        from .provider_probe_source import verify_descriptor

        source_files = [
            item for item in self.state["files"] if item["path"].endswith("-source-manifest")
        ]
        require(len(source_files) == 1, "E exact source manifest unavailable")
        source_raw = e_indexed(self, source_files[0], 65536)
        verify_descriptor(
            self.state["source_descriptor"], source_raw, self.state["source_manifest_sha256"]
        )
        require(
            type(self.state.get("requests")) is int
            and 0 <= self.state["requests"] <= MAX_REQUESTS
            and isinstance(self.state.get("transport"), list)
            and len(self.state["transport"]) <= MAX_REQUESTS
            and all(
                type(row.get("count")) is int and row["count"] == 1
                for row in self.state["transport"]
            )
            and sum(row["count"] for row in self.state["transport"]) == self.state["requests"],
            "custody request ledger mismatch",
        )
        require(
            read_only or self.state.get("custody_directory") == str(self.directory),
            "custody directory identity mismatch",
        )
        pending = self.directory / "incomplete-observation.json"
        if not read_only and (pending.exists() or pending.is_symlink()):
            require(
                not pending.is_symlink() and pending.is_file() and pending.stat().st_size <= 4096,
                "invalid incomplete observer marker",
            )
            marker = decode(read_regular(pending, 4096))
            require(
                marker
                == {
                    "schema": "provider-probe-incomplete-observation.v1",
                    "source_sha": self.state["source_sha"],
                    "series": self.state["series"],
                },
                "incomplete observer marker identity mismatch",
            )
            self.state["normal_aborted"] = True
            if "controller_v5" in self.state:
                self.state["controller_v5"]["aborted"] = True
            if reconcile:
                self.persist()  # Failure leaves the marker; controls cannot resume on later load.
                self.observation_marker(finish=True)
        self.token = None
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())

    def observation_marker(self, *, finish: bool = False) -> None:
        with open_directory(self.directory) as directory:
            info = os.fstat(directory)
            require(
                (info.st_dev, info.st_ino) == self.directory_identity,
                "custody directory identity changed",
            )
            if finish:
                os.unlink("incomplete-observation.json", dir_fd=directory)
            else:
                descriptor = os.open(
                    "incomplete-observation.json",
                    os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                    0o600,
                    dir_fd=directory,
                )
                with os.fdopen(descriptor, "wb") as stream:
                    stream.write(
                        e_bytes(
                            {
                                "schema": "provider-probe-incomplete-observation.v1",
                                "source_sha": self.state["source_sha"],
                                "series": self.state["series"],
                            }
                        )
                    )
                    stream.flush()
                    os.fsync(stream.fileno())
            os.fsync(directory)

    def persist(self) -> None:
        require(not self.read_only, "copied proof is read-only; no shared-ledger fork")
        write_json(self.state_path, self.state, self.directory_identity)

    def elapsed(self) -> float:
        require(
            Path("/proc/sys/kernel/random/boot_id").read_text().strip() == self.state["boot"],
            "host reboot invalidates series clock",
        )
        elapsed = time.monotonic() - self.state["started_monotonic"]
        require(0 <= elapsed < MAX_SECONDS, "series deadline exhausted")
        return elapsed

    def charge(self, count: int, purpose: str) -> None:
        require(not self.read_only, "copied proof is read-only; no shared-ledger fork")
        elapsed = self.elapsed()
        require(type(count) is int and count == 1, "invalid transport charge")
        require(self.state["requests"] + count <= MAX_REQUESTS, "series request budget exhausted")
        before = self.state["requests"]
        self.state["requests"] += count
        self.state["transport"].append(
            {
                "utc": utc(),
                "count": count,
                "purpose": purpose,
                "kind": "transport-reservation-before-attempt",
                "before": before,
                "after": self.state["requests"],
                "elapsed": elapsed,
            }
        )
        self.persist()  # reserve before transport; a crash never restores the allowance

    def save(
        self, label: str, raw: bytes, normalized: dict[str, Any] | None = None
    ) -> dict[str, Any]:
        require(
            re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,79}", label) and ".." not in label,
            "invalid evidence label",
        )
        require(
            sum(item["bytes"] for item in self.state["files"]) + len(raw) <= MAX_CUSTODY,
            "aggregate custody byte budget exhausted",
        )
        index = len(self.state["files"]) + 1
        path = self.directory / f"{index:04d}-{label}"
        require(not path.exists(), "evidence file collision")
        with open_directory(self.directory) as directory:
            info = os.fstat(directory)
            require(
                (info.st_dev, info.st_ino) == self.directory_identity,
                "custody directory identity changed",
            )
            descriptor = os.open(
                path.name,
                os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                0o600,
                dir_fd=directory,
            )
            with os.fdopen(descriptor, "wb") as stream:
                stream.write(raw)
                stream.flush()
                os.fsync(stream.fileno())
            os.fsync(directory)
        item = {
            "path": path.name,
            "bytes": len(raw),
            "sha256": hashlib.sha256(raw).hexdigest(),
            "utc": utc(),
            "elapsed": self.elapsed(),
        }
        self.state["files"].append(item)
        self.persist()
        if normalized is not None:
            self.save(
                label + "-normalized",
                (json.dumps(normalized, sort_keys=True, indent=2) + "\n").encode(),
            )
        return item

    def transport(
        self, url: str, *, authorization: bool, limit: int, purpose: str
    ) -> tuple[int, str | None, bytes]:
        self.charge(1, purpose)
        headers = {
            "User-Agent": "boxferry-provider632-observer",
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
        }
        if authorization:
            require(
                url.startswith("https://api.github.com/repos/" + REPO + "/"),
                "credential origin refused",
            )
            if self.token is None:
                self.token = token_from_gh()
            headers["Authorization"] = "Bearer " + self.token
        try:
            request = urllib.request.Request(url, method="GET", headers=headers)
            timeout = max(0.1, min(15, MAX_SECONDS - self.elapsed()))
            with hard_request_deadline(timeout):
                try:
                    response = self.opener.open(request, timeout=timeout)
                except urllib.error.HTTPError as error:
                    if error.code in REDIRECTS:
                        location = error.headers.get("Location", "")
                        error.close()
                        return error.code, location, b""
                    error.close()
                    raise Inconclusive("provider returned non-success HTTP status") from None
                except (OSError, urllib.error.URLError):
                    raise Inconclusive("bounded GET transport failed") from None
                with response:
                    require(response.status == 200, "unexpected successful HTTP status")
                    length = response.headers.get("Content-Length")
                    if length is not None:
                        require(
                            length.isdecimal() and int(length) <= limit,
                            "response exceeds byte bound",
                        )
                    raw = response.read(limit + 1)
                    require(len(raw) <= limit, "response exceeds byte bound")
                    self.elapsed()
                    return 200, None, raw
        except (
            OSError,
            urllib.error.URLError,
            http.client.HTTPException,
            UnicodeError,
            ValueError,
        ):
            raise Inconclusive("bounded GET request or response refused") from None

    def reserve_read(self, suffix: str, run_scope: int | None = None) -> None:
        previous = self.state["reads"].get(suffix)
        if previous:
            require(
                time.monotonic() - previous["at"] >= 5, "five-second reread cadence not reached"
            )
            require(previous.get("terminal_rereads", 0) < 1, "terminal reread allowance exhausted")
        is_terminal_scope = str(run_scope) in self.state.get("terminal_runs", {})
        terminal = bool((previous and previous.get("terminal")) or is_terminal_scope)
        rereads = (
            previous.get("terminal_rereads", 0) + 1 if previous and previous.get("terminal") else 0
        )
        self.state["reads"][suffix] = {
            "at": time.monotonic(),
            "terminal": terminal,
            "terminal_rereads": rereads,
        }
        self.persist()  # Failed/interrupted requests also consume cadence and reread reservations.

    def reserve_discovery(self, case: str) -> int:
        require(case in "ABCDE" and len(case) == 1, "invalid discovery case")
        self.elapsed()
        require(self.state["requests"] < MAX_REQUESTS, "series request budget exhausted")
        count = self.state["discovery_reads"].get(case, 0)
        require(type(count) is int and 0 <= count < 3, "case discovery read allowance exhausted")
        self.state["discovery_reads"][case] = count + 1
        self.persist()
        return count

    def api(self, suffix: str, label: str) -> tuple[dict[str, Any], dict[str, Any]]:
        require(
            re.fullmatch(
                r"(?:actions/runs/[1-9][0-9]*(?:/attempts/1/jobs\?per_page=100&page=[1-9][0-9]*|/artifacts\?per_page=100&page=[1-9][0-9]*)?"
                r"|actions/jobs/[1-9][0-9]*|actions/artifacts/[1-9][0-9]*|check-runs/[1-9][0-9]*)",
                suffix,
            ),
            "API endpoint outside closed read catalog",
        )
        endpoint = "https://api.github.com/repos/" + REPO + "/" + suffix
        run_scope = None
        run_match = re.match(r"actions/runs/([1-9][0-9]*)(?:/|$)", suffix)
        if run_match:
            run_scope = int(run_match.group(1))
        elif suffix.startswith("check-runs/"):
            run_scope = self.state.get("check_to_run", {}).get(suffix.rsplit("/", 1)[1])
        elif re.fullmatch(r"actions/artifacts/[1-9][0-9]*", suffix):
            artifact_id = suffix.rsplit("/", 1)[1]
            scopes = {
                row["id"]
                for row in self.state.get("runs", {}).values()
                if artifact_id in row.get("artifacts", {})
            }
            if len(scopes) == 1:
                run_scope = scopes.pop()
        elif suffix.startswith("actions/jobs/"):
            job_id = int(suffix.rsplit("/", 1)[1])
            scopes = {
                row["run"]
                for row in self.state.get("observations", {}).values()
                if row.get("job") == job_id
            }
            if len(scopes) == 1:
                run_scope = scopes.pop()
        self.reserve_read(suffix, run_scope)
        status, _, raw = self.transport(
            endpoint, authorization=True, limit=MAX_JSON, purpose=suffix
        )
        require(status == 200, "metadata redirect refused")
        is_run = re.fullmatch(r"actions/runs/[1-9][0-9]*", suffix) is not None
        if is_run:
            summary = {
                "provenance": "exact-run-response",
                "request_run_id": int(suffix.rsplit("/", 1)[1]),
                "bytes": len(raw),
                "sha256": hashlib.sha256(raw).hexdigest(),
            }
            summary_file = self.save(label + "-received-summary", e_bytes(summary))
            self.last_run_response = {
                "provenance": "exact",
                "json_state": "unparsed",
                "response": summary,
                "identity": run_identity_facts(None),
                "evidence_ref": {key: summary_file[key] for key in ("path", "bytes", "sha256")},
            }
        value = decode(raw)
        if is_run:
            self.last_run_response.update(
                json_state="object" if isinstance(value, dict) else "nonobject",
                identity=run_identity_facts(value),
            )
        require(isinstance(value, dict), "provider response is not an object")
        run_scope = value.get("run_id")
        if suffix.startswith("check-runs/"):
            run_scope = self.state.get("check_to_run", {}).get(suffix.split("/")[1])
        terminal = value.get("status") == "completed" and (
            is_run or str(run_scope) in self.state.get("terminal_runs", {})
        )
        self.state["reads"][suffix]["terminal"] |= terminal
        # Authenticate independent identities before persisting a possibly unrelated body.
        try:
            if is_run:
                run_id = int(suffix.rsplit("/", 1)[1])
                cases = [
                    case
                    for case in "ABCDE"
                    if value.get("display_title")
                    == f"629 provider probe {self.state['series']} case {case}"
                ]
                require(len(cases) == 1, "unbound series/case response")
                self.validate_run(value, cases[0], run_id)
            elif suffix.startswith("actions/jobs/"):
                require(
                    value.get("id") == int(suffix.rsplit("/", 1)[1])
                    and value.get("run_id") in [row["id"] for row in self.state["runs"].values()]
                    and value.get("run_attempt") == 1
                    and value.get("head_sha") == self.state["source_sha"],
                    "unbound job response",
                )
            elif suffix.startswith("check-runs/"):
                require(
                    value.get("id") == int(suffix.rsplit("/", 1)[1])
                    and value.get("head_sha") == self.state["source_sha"]
                    and object_field(value, "app").get("id") == 15368
                    and object_field(value, "app").get("slug") == "github-actions",
                    "unbound check response",
                )
            elif re.fullmatch(r"actions/artifacts/[1-9][0-9]*", suffix):
                cases = [
                    case
                    for case, row in self.state["runs"].items()
                    if row["id"] == object_field(value, "workflow_run").get("id")
                ]
                require(len(cases) == 1, "unbound artifact response")
                self.validate_artifact(value, cases[0], self.state["runs"][cases[0]]["id"])
        except Inconclusive:
            self.save(
                label + "-refused-body-hash",
                (
                    json.dumps(
                        {
                            "bytes": len(raw),
                            "sha256": hashlib.sha256(raw).hexdigest(),
                            "reason": "identity-unavailable; unrelated raw body not persisted",
                        }
                    )
                    + "\n"
                ).encode(),
            )
            raise
        item = self.save(label + "-raw", raw)
        return value, item

    def bound(self, case: str, run: int) -> dict[str, Any]:
        positive(run)
        require(case in "ABCDE" and len(case) == 1, "invalid case")
        require(
            self.state["runs"].get(case, {}).get("id") == run, "run/case is not explicitly bound"
        )
        return self.state["runs"][case]

    def validate_run(self, value: dict[str, Any], case: str, run: int) -> dict[str, Any]:
        require(isinstance(value, dict), "run metadata must be an object")
        expected = self.state
        require(
            type(value.get("id")) is int
            and value["id"] == run
            and type(value.get("run_attempt")) is int
            and value["run_attempt"] == 1,
            "run/attempt identity drift",
        )
        require(
            type(value.get("workflow_id")) is int
            and value["workflow_id"] == expected["workflow_id"]
            and value.get("path") == WORKFLOW,
            "workflow identity unavailable or changed",
        )
        require(
            object_field(value, "repository").get("full_name") == REPO
            and object_field(value, "head_repository").get("full_name") == REPO,
            "repository identity unavailable or changed",
        )
        require(
            value.get("event") == "workflow_dispatch"
            and value.get("head_branch") == "main"
            and value.get("head_sha") == expected["source_sha"],
            "event/ref/source drift",
        )
        require(
            value.get("display_title") == f"629 provider probe {expected['series']} case {case}",
            "series/case run correlation unavailable",
        )
        for key in ("actor", "triggering_actor"):
            require(
                object_field(value, key).get("login") == expected[key]["login"]
                and type(object_field(value, key).get("id")) is int
                and object_field(value, key)["id"] == expected[key]["id"],
                "actor identity drift",
            )
        require(
            value.get("html_url") == f"https://github.com/{REPO}/actions/runs/{run}",
            "run details identity drift",
        )
        if value.get("status") == "completed":
            self.state.setdefault("terminal_runs", {})[str(run)] = True
        return value

    def run(self, case: str, run: int, label: str) -> tuple[dict[str, Any], dict[str, Any]]:
        self.bound(case, run)
        value, item = self.api(f"actions/runs/{run}", label)
        return self.validate_run(value, case, run), item

    def inventory(self, case: str, run: int, kind: str, label: str) -> list[dict[str, Any]]:
        self.bound(case, run)
        result, total, page = [], None, 1
        cap = 100 if kind == "jobs" else 2
        while True:
            suffix = (
                f"actions/runs/{run}/attempts/1/jobs"
                if kind == "jobs"
                else f"actions/runs/{run}/artifacts"
            )
            value, _ = self.api(suffix + f"?per_page=100&page={page}", label + f"-{kind}-{page}")
            count, rows = value.get("total_count"), value.get(kind)
            require(
                type(count) is int and 0 <= count <= cap and isinstance(rows, list),
                "inventory exceeds bound or lacks total",
            )
            require(total is None or count == total, "inventory changed during pagination")
            total = count
            require(rows or len(result) == total, "incomplete pagination")
            result += cast(list[dict[str, Any]], rows)
            require(
                len(result) <= cast(int, total) and all(isinstance(row, dict) for row in result),
                "clipped or invalid inventory",
            )
            ids = [positive(row.get("id")) for row in result]
            require(len(ids) == len(set(ids)), "duplicate inventory identity")
            if len(result) == total:
                break
            page += 1
        if kind == "jobs":
            for job in result:
                require(
                    job.get("run_id") == run
                    and job.get("run_attempt") == 1
                    and job.get("head_sha") == self.state["source_sha"],
                    "job inventory identity drift",
                )
        else:
            for artifact in result:
                self.validate_artifact(artifact, case, run)
            require(
                len({row["name"] for row in result}) == len(result),
                "duplicate probe artifact names",
            )
            previous = self.state["runs"][case].get("artifacts", {})
            current = {str(row["id"]): row for row in result}
            require(
                set(previous) <= set(current),
                "previous artifact disappeared from complete inventory",
            )
            for identifier, row in previous.items():
                require(
                    all(
                        row.get(key) == current[identifier].get(key)
                        for key in ("id", "name", "digest", "size_in_bytes", "workflow_run")
                    ),
                    "previous artifact identity/digest changed",
                )
            self.state["runs"][case]["artifacts"] = current
        self.persist()
        return result

    def validate_artifact(self, artifact: dict[str, Any], case: str, run: int) -> int:
        require(isinstance(artifact, dict), "artifact metadata must be an object")
        identifier = positive(artifact.get("id"))
        prefix = f"boxferry-629-provider-probe-v1-{self.state['series']}-{run}-attempt1-"
        require(
            artifact.get("name") in {prefix + "prepare", prefix + "effect-entry"},
            "artifact outside exact probe namespace",
        )
        require(
            object_field(artifact, "workflow_run").get("id") == run
            and object_field(artifact, "workflow_run").get("head_sha") == self.state["source_sha"],
            "artifact run/source attribution unavailable",
        )
        try:
            created = dt.datetime.fromisoformat(artifact["created_at"].replace("Z", "+00:00"))
            expires = dt.datetime.fromisoformat(artifact["expires_at"].replace("Z", "+00:00"))
            require(
                created.tzinfo is not None
                and expires.tzinfo is not None
                and created < expires
                and expires > dt.datetime.now(dt.UTC),
                "artifact expiry binding invalid",
            )
        except (KeyError, AttributeError, ValueError):
            raise Inconclusive("artifact expiry metadata unavailable") from None
        require(
            artifact.get("expired") is False
            and type(artifact.get("size_in_bytes")) is int
            and 0 < artifact["size_in_bytes"] <= MAX_ARCHIVE,
            "artifact unavailable or oversized",
        )
        require(
            isinstance(artifact.get("digest"), str) and artifact["digest"].startswith("sha256:"),
            "artifact server digest unavailable",
        )
        digest(artifact["digest"][7:])
        return identifier

    def capture_terminal(self, case: str, run: int, label: str) -> dict[str, Any]:
        """Reuse the first completed run response; no prediction or extra terminal GET."""
        self.bound(case, run)
        first, stable = case + "-terminal", case + "-terminal-stable"
        require(label not in {first, stable}, "terminal capture requires a unique sample label")
        require(
            stable not in self.state["snapshots"], "terminal capture already stable; no third read"
        )
        previous = self.state["snapshots"].get(first)
        if previous is None:
            require(
                not self.state["reads"].get(f"actions/runs/{run}", {}).get("terminal"),
                "first terminal response cannot be backfilled",
            )
        else:
            require(
                previous.get("case") == case
                and previous.get("run") == run
                and previous.get("run_status") == "completed"
                and self.elapsed() - previous["elapsed"] >= 5,
                "spaced stable terminal capture required",
            )
        value, item = self.run(case, run, label)
        if value.get("status") != "completed":
            require(
                previous is None
                and value.get("status")
                in {"queued", "requested", "waiting", "pending", "in_progress"},
                "terminal capture status contradicted",
            )
            result = {
                "outcome": "NOT_YET_OBSERVED",
                "case": case,
                "run": run,
                "run_status": value["status"],
                "run_file": item,
            }
            self.save(label + "-active-sample", e_bytes(result))
            return result
        snapshot = self.snapshot(case, run, label, run_response=(value, item))
        require(snapshot["outcome"] == "OBSERVED", "terminal capture inventory unsupported")
        canonical = first if previous is None else stable
        require(canonical not in self.state["snapshots"], "terminal capture label already bound")
        self.state["snapshots"][canonical] = snapshot
        self.state.setdefault("terminal_captures", {})[canonical] = {
            "sample_label": label,
            "snapshot_sha256": hashlib.sha256(e_bytes(snapshot)).hexdigest(),
            "run_file": item,
        }
        self.persist()
        return {
            "outcome": "OBSERVED",
            "case": case,
            "run": run,
            "run_status": "completed",
            "terminal_capture": canonical,
        }

    def snapshot(
        self,
        case: str,
        run: int,
        label: str,
        *,
        run_response: tuple[dict[str, Any], dict[str, Any]] | None = None,
    ) -> dict[str, Any]:
        started = self.elapsed()
        if run_response is None:
            value, _ = self.run(case, run, label)
        else:
            value, item = run_response
            self.bound(case, run)
            self.validate_run(value, case, run)
            require(
                item in self.state["files"] and decode(e_indexed(self, item)) == value,
                "terminal run response custody unavailable",
            )
        jobs = self.inventory(case, run, "jobs", label)
        artifacts = self.inventory(case, run, "artifacts", label)
        records = []
        unsupported = []
        for job in jobs:
            name = job.get("name")
            require(
                isinstance(name, str) and 0 < len(name) <= 256,
                "job name metadata must be a bounded string",
            )
            shape = CATALOG.get(name)
            if shape is None:
                unsupported.append(job["id"])
            records.append(
                {
                    "id": job["id"],
                    "name": name,
                    "run_id": job.get("run_id"),
                    "run_attempt": job.get("run_attempt"),
                    "head_sha": job.get("head_sha"),
                    "html_url": job.get("html_url"),
                    "catalog_name_shape": list(shape) if shape else None,
                    "check_run_url": job.get("check_run_url"),
                    "status": job.get("status"),
                    "conclusion": job.get("conclusion"),
                    "runner_id": job.get("runner_id"),
                    "runner_name": job.get("runner_name"),
                    "labels": job.get("labels"),
                    "steps": job.get("steps"),
                    "role_authentication": "unobserved until explicit job/check read",
                }
            )
        normalized = {
            "case": case,
            "run": run,
            "run_status": value.get("status"),
            "run_conclusion": value.get("conclusion"),
            "jobs": records,
            "artifact_ids": [row["id"] for row in artifacts],
            "artifacts": artifacts,
            "outcome": "UNSUPPORTED" if unsupported else "OBSERVED",
            "unsupported_job_ids": unsupported,
            "wrapper_to_leaf_mapping": "UNOBSERVED; no prefix joins",
            "referenced_workflows": value.get("referenced_workflows", "UNOBSERVED"),
        }
        normalized.update(started_elapsed=started, elapsed=self.elapsed())
        self.state["snapshots"][label] = normalized
        self.save(label + "-snapshot", (json.dumps(normalized, sort_keys=True) + "\n").encode())
        require(not unsupported, "unknown job catalog shape; complete raw inventory preserved")
        return normalized

    def job(self, case: str, run: int, identifier: int, check: int, label: str) -> dict[str, Any]:
        started = self.elapsed()
        self.bound(case, run)
        positive(identifier), positive(check)
        job, _ = self.api(f"actions/jobs/{identifier}", label + "-job")
        require(
            job.get("id") == identifier
            and job.get("run_id") == run
            and job.get("run_attempt") == 1
            and job.get("head_sha") == self.state["source_sha"],
            "explicit job binding drift",
        )
        require(
            job.get("check_run_url") == f"https://api.github.com/repos/{REPO}/check-runs/{check}",
            "job/check mapping unavailable or wrong origin",
        )
        self.state.setdefault("check_to_run", {})[str(check)] = run
        self.persist()
        record, _ = self.api(f"check-runs/{check}", label + "-check")
        require(
            record.get("id") == check
            and record.get("head_sha") == self.state["source_sha"]
            and object_field(record, "app").get("id") == 15368
            and object_field(record, "app").get("slug") == "github-actions",
            "check provider/source drift",
        )
        require(
            record.get("details_url")
            == f"https://github.com/{REPO}/actions/runs/{run}/job/{identifier}"
            and record.get("name") == job.get("name"),
            "check job linkage unavailable",
        )
        name = job.get("name")
        require(
            isinstance(name, str) and 0 < len(name) <= 256,
            "job name metadata must be a bounded string",
        )
        shape = CATALOG.get(name)
        normalized = {
            "case": case,
            "run": run,
            "job": identifier,
            "check": check,
            "job_record": job,
            "check_record": record,
            "catalog_name_shape": list(shape) if shape else None,
            "started_elapsed": started,
            "elapsed": self.elapsed(),
            "outcome": "OBSERVED" if shape else "UNSUPPORTED",
            "wrapper_to_leaf_mapping": "UNOBSERVED; explicit IDs preserved",
        }
        self.state["observations"][label] = normalized
        self.save(label + "-bound", (json.dumps(normalized, sort_keys=True) + "\n").encode())
        return normalized

    def download(
        self, case: str, run: int, identifier: int, uploader_digest: str, label: str
    ) -> dict[str, Any]:
        bound = self.bound(case, run)
        e_record = None
        if case == "E":
            artifact = bound.get("artifacts", {}).get(str(identifier))
            require(artifact is not None, "E archive not enumerated")
            phase = E_PRE if artifact["name"].endswith("-prepare") else E_POST
            e_record = e_provenance(self, artifact, phase)
            require(
                uploader_digest == e_record["artifact_digest"],
                "E bare digest cannot replace provenance",
            )
        positive(identifier), digest(uploader_digest)
        require(
            str(identifier) in bound.get("artifacts", {}),
            "artifact was not enumerated for exact run",
        )
        artifact, _ = self.api(f"actions/artifacts/{identifier}", label + "-artifact")
        self.validate_artifact(artifact, case, run)
        previous = bound["artifacts"][str(identifier)]
        for key in ("id", "name", "size_in_bytes", "digest", "workflow_run"):
            require(artifact.get(key) == previous.get(key), "artifact metadata changed")
        raw = self.download_bytes(
            f"actions/artifacts/{identifier}/zip", MAX_ARCHIVE, f"artifact-{identifier}", run
        )
        # Raw bytes are preserved before interpretation, but no private URL/header is persisted.
        archive_item = self.save(label + "-archive.zip", raw)
        witness = verify_archive(raw, artifact, self.state, case, run, uploader_digest)
        normalized = {
            "artifact_id": identifier,
            "archive_sha256": hashlib.sha256(raw).hexdigest(),
            "archive_bytes": len(raw),
            "witness": witness,
            "witness_sha256": hashlib.sha256(witness_bytes(raw)).hexdigest(),
            "run": run,
            "case": case,
            "elapsed": self.elapsed(),
            "outcome": "OBSERVED",
        }
        if case == "E":
            normalized.update(
                phase=cast(dict[str, Any], e_record)["phase"],
                uploader_provenance_file=cast(dict[str, Any], e_record)["record_file"],
                archive_file=archive_item,
            )
        bound.setdefault("verified_artifacts", {})[str(identifier)] = normalized
        self.save(label + "-verified", (json.dumps(normalized, sort_keys=True) + "\n").encode())
        return normalized

    def download_bytes(self, suffix: str, limit: int, purpose: str, run_scope: int) -> bytes:
        require(
            re.fullmatch(r"actions/(?:artifacts/[1-9][0-9]*/zip|jobs/[1-9][0-9]*/logs)", suffix),
            "download path outside closed catalog",
        )
        self.reserve_read(suffix, run_scope)
        url = f"https://api.github.com/repos/{REPO}/" + suffix
        for hop in range(5):
            status, location, body = self.transport(
                url, authorization=hop == 0, limit=limit, purpose=f"{purpose}-hop-{hop}"
            )
            if status == 200:
                return body
            require(hop < 4, "download redirect bound exhausted")
            url = safe_redirect(cast(str, location))
        raise Inconclusive("download unavailable")

    def log(
        self, case: str, run: int, job: int, check: int, label: str, phase: str | None = None
    ) -> dict[str, Any]:
        e_observation = e_phase_log(self, case, run, job, check, phase) if case == "E" else None
        self.bound(case, run)
        require(
            any(
                row["run"] == run and row["job"] == job and row["check"] == check
                for row in self.state["observations"].values()
            ),
            "log job/check not previously authenticated",
        )
        raw = self.download_bytes(
            f"actions/jobs/{positive(job)}/logs", 1024 * 1024, f"job-log-{job}", run
        )
        require(
            self.state.get("log_bytes", 0) + len(raw) <= 5 * 1024 * 1024,
            "aggregate log byte budget exhausted",
        )
        self.state["log_bytes"] = self.state.get("log_bytes", 0) + len(raw)
        self.persist()
        # A privacy refusal preserves only a hash; raw credentials/headers/signed URLs never enter
        # custody.
        suspicious = re.search(
            rb"(?i)(?:authorization\s*:|bearer\s+|gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]+|https?://[^\s<>]+\?|[?&](?:x-amz-signature|x-goog-signature|sig|signature|signed|token|access_token)=)",
            raw,
        )
        if suspicious or (self.token and self.token.encode() in raw):
            self.save(
                label + "-privacy-refused-hash",
                (
                    json.dumps(
                        {
                            "bytes": len(raw),
                            "sha256": hashlib.sha256(raw).hexdigest(),
                            "raw_omitted": "privacy refusal",
                        }
                    )
                    + "\n"
                ).encode(),
            )
            raise Inconclusive("log privacy refused; hash only preserved")
        try:
            text = raw.decode("utf-8")
        except UnicodeDecodeError:
            raise Inconclusive("log encoding unsupported") from None
        raw_item = self.save(label + "-raw.log", raw)
        candidates = []
        for line in text.splitlines():
            if "sha256" in line.lower() and "artifact" in line.lower() and "digest" in line.lower():
                candidates.extend(re.findall(r"(?<![A-Fa-f0-9])[a-f0-9]{64}(?![A-Fa-f0-9])", line))
        result = {
            "run": run,
            "job": job,
            "check": check,
            "bytes": len(raw),
            "sha256": hashlib.sha256(raw).hexdigest(),
            "uploader_digest_candidates": candidates,
            "meaning": (
                "operator must bind digest to exact uploader step/source; no automatic attribution"
            ),
            "outcome": "OBSERVED",
        }
        if case == "E":
            result.update(
                phase=phase,
                case=case,
                attempt=1,
                source_sha=self.state["source_sha"],
                series=self.state["series"],
                raw_file=raw_item,
                observation=e_observation,
                elapsed=self.elapsed(),
                source_catalog_sha256=e_source(self),
            )
            self.state.setdefault("e_v5_logs", {})[label] = result
        self.save(label + "-digests", (json.dumps(result, sort_keys=True) + "\n").encode())
        return result

    def bind_e_uploader(
        self,
        log_label: str,
        identifier: int,
        uploader_digest: str,
        step_number: int,
        line_start: int,
        line_end: int,
    ) -> dict[str, Any]:
        log = self.state.get("e_v5_logs", {}).get(log_label)
        require(isinstance(log, dict), "E authenticated phased uploader log unavailable")
        phase = log["phase"]
        observation = e_phase_log(self, "E", log["run"], log["job"], log["check"], phase)
        require(
            observation == log["observation"], "E uploader observation changed after log collection"
        )
        artifact = self.state["runs"]["E"].get("artifacts", {}).get(str(positive(identifier)))
        require(artifact is not None, "E uploader artifact was not enumerated")
        self.validate_artifact(artifact, "E", log["run"])
        role = "prepare" if phase == E_PRE else "effect"
        require(
            artifact["name"].endswith("-prepare" if role == "prepare" else "-effect-entry")
            and digest(uploader_digest) == artifact["digest"][7:]
            and positive(step_number) == e_steps(observation, role)[E_UPLOADS[role]],
            "E uploader artifact digest or source step mismatch",
        )
        if phase == E_POST:
            require(
                artifact == self.state["e_v5"]["pre_cancel_cut"]["effect_entry_metadata"],
                "E final uploader artifact differs from pre-cancel observed ID",
            )
        require(
            str(identifier) not in self.state.get("e_v5_provenance", {}),
            "E uploader provenance already bound; never replace phase",
        )
        raw = e_indexed(self, log["raw_file"])
        lines = raw.decode("utf-8").splitlines(keepends=True)
        require(
            type(line_start) is int
            and type(line_end) is int
            and 1 <= line_start <= line_end <= len(lines),
            "E uploader exact line span unavailable",
        )
        span = "".join(lines[line_start - 1 : line_end]).encode()
        require(
            span.count(self.state["source_descriptor"]["uploader"].encode()) == 1
            and set(re.findall(rb"(?<![A-Fa-f0-9])[a-f0-9]{64}(?![A-Fa-f0-9])", span))
            == {uploader_digest.encode()}
            and {
                int(value) for value in re.findall(rb"Artifact ID(?: is|:)?\s+([1-9][0-9]*)", span)
            }
            == {identifier},
            "E operator exact uploader span lacks source digest or artifact ID",
        )
        payload = {
            "phase": phase,
            "role": role,
            "case": "E",
            "run": log["run"],
            "attempt": 1,
            "source_sha": self.state["source_sha"],
            "series": self.state["series"],
            "source_catalog_sha256": e_source(self),
            "job": log["job"],
            "check": log["check"],
            "step_name": E_UPLOADS[role],
            "step_number": step_number,
            "uploader": self.state["source_descriptor"]["uploader"],
            "artifact_id": identifier,
            "artifact_digest": uploader_digest,
            "artifact": artifact,
            "log_label": log_label,
            "log": log,
            "line_start": line_start,
            "line_end": line_end,
            "span_sha256": hashlib.sha256(span).hexdigest(),
            "elapsed": self.elapsed(),
            "utc": utc(),
            "attestation": "operator-authenticated-exact-source-uploader-step",
        }
        item = self.save(log_label + "-uploader-provenance", e_bytes(payload))
        record = {**payload, "record_file": item}
        self.state.setdefault("e_v5_provenance", {})[str(identifier)] = record
        self.persist()
        return record


def witness_bytes(raw: bytes) -> bytes:
    require(len(raw) <= MAX_ARCHIVE, "archive exceeds bound")
    try:
        with zipfile.ZipFile(io.BytesIO(raw)) as archive:
            members = archive.infolist()
            require(
                len(members) == 1 and members[0].filename == "witness.json",
                "unexpected archive members",
            )
            member = members[0]
            mode = member.external_attr >> 16
            require(
                member.create_system == 3
                and not member.is_dir()
                and stat.S_ISREG(mode)
                and not member.flag_bits & 1,
                "archive member is not one regular unencrypted file",
            )
            require(
                0 < member.file_size <= 4096 and member.compress_size <= MAX_ARCHIVE,
                "witness archive member exceeds bound",
            )
            return archive.read(member)  # ZIP CRC is checked; never extracts a filesystem path
    except (OSError, ValueError, RuntimeError, zipfile.BadZipFile, NotImplementedError):
        raise Inconclusive("invalid bounded witness archive") from None


def verify_archive(
    raw: bytes,
    artifact: dict[str, Any],
    state: dict[str, Any],
    case: str,
    run: int,
    uploader_digest: str,
) -> dict[str, Any]:
    actual = hashlib.sha256(raw).hexdigest()
    require(
        len(raw) == artifact["size_in_bytes"]
        and actual == artifact["digest"][7:]
        and actual == digest(uploader_digest),
        "archive size/server/uploader digest mismatch",
    )
    witness = decode(witness_bytes(raw))
    role = "prepare" if artifact["name"].endswith("-prepare") else "effect"
    expected = {
        "schema": SCHEMA,
        "repository": REPO,
        "workflow_path": WORKFLOW,
        "source_sha": state["source_sha"],
        "series": state["series"],
        "case": case,
        "run_id": str(run),
        "run_attempt": 1,
        "role": role,
        "marker_phase": "completed" if role == "prepare" else "entry",
    }
    if role == "effect":
        require(isinstance(witness, dict), "witness must be an object")
        identifier = witness.get("preparation_artifact_id")
        require(
            isinstance(identifier, str) and re.fullmatch("[1-9][0-9]{0,19}", identifier),
            "inherited preparation ID unavailable",
        )
        inherited = state["runs"][case].get("artifacts", {}).get(identifier)
        require(
            inherited is not None and inherited["name"].endswith("-prepare"),
            "inherited preparation artifact not enumerated",
        )
        marker = f"{SCHEMA}:{state['series']}:{run}:1:{state['source_sha']}"
        expected.update(
            preparation_artifact_id=identifier,
            preparation_artifact_digest=inherited["digest"][7:],
            preparation_marker=marker,
            caller_prerequisite_result="success",
        )
    require(
        witness == expected and type(witness.get("run_attempt")) is int,
        "witness schema or exact context binding mismatch",
    )
    return witness


def holding(observation: dict[str, Any], role: str, step_number: int | None = None) -> int:
    require(
        observation["catalog_name_shape"] == [role, "expanded-leaf"],
        "expanded role identity unobserved",
    )
    job, check = observation["job_record"], observation["check_record"]
    require(
        job.get("status") == check.get("status") == "in_progress"
        and positive(job.get("runner_id"))
        and isinstance(job.get("steps"), list),
        "positive executing runner evidence unavailable",
    )
    steps = [step for step in job["steps"] if step.get("name") == HOLDS[role]]
    require(
        len(steps) == 1
        and steps[0].get("status") == "in_progress"
        and type(steps[0].get("number")) is int,
        "positive fixed hold step unavailable",
    )
    if step_number is not None:
        require(steps[0]["number"] == step_number, "fixed hold step identity changed")
    return steps[0]["number"]


def successful(observation: dict[str, Any], role: str) -> None:
    require(
        observation["catalog_name_shape"] == [role, "expanded-leaf"],
        "successful expanded role unobserved",
    )
    require(
        all(
            observation[key].get("status") == "completed"
            and observation[key].get("conclusion") == "success"
            for key in ("job_record", "check_record")
        ),
        "positive success job/check evidence unavailable",
    )


def guard_burst(evidence: Evidence, args: argparse.Namespace) -> dict[str, Any]:
    before = evidence.state["observations"][args.before]
    after = evidence.state["observations"][args.after]
    victim = evidence.state["snapshots"][args.victim]
    pending = evidence.state["snapshots"][args.pending]
    number = holding(before, "prepare")
    holding(after, "prepare", number)
    require(
        before["case"] == after["case"] == "A" and victim["case"] == pending["case"] == "B",
        "burst cases do not match A/B",
    )
    require(
        (before["run"], before["job"], before["check"])
        == (after["run"], after["job"], after["check"]),
        "A bound identities changed",
    )
    require(
        pending["run_status"] == "pending"
        and victim["run_status"] == "completed"
        and victim["run_conclusion"] == "cancelled",
        "B pending/terminal sequence unobserved",
    )
    require(
        pending["elapsed"]
        < before["started_elapsed"]
        <= before["elapsed"]
        < victim["elapsed"]
        < after["started_elapsed"]
        <= after["elapsed"],
        "fresh A post-B terminal observation unavailable",
    )
    return {
        "outcome": "OBSERVED",
        "meaning": "controlled pending-replacement sequence only",
        "A_job": before["job"],
        "A_check": before["check"],
        "A_step": number,
        "B_no_effect_classification": (
            "UNSUPPORTED; independently reviewed positive prerequisite evidence still required"
        ),
        "provider_cancellation_cause": "UNOBSERVED",
    }


# Preserve the v5 phase-typed E proof; source is the reviewed fresh-series descriptor.
E_PRE = "E_PRE_CANCEL_PREPARATION"
E_POST = "E_POST_CANCEL_EFFECT"
E_UPLOADS = {
    "prepare": "Upload inert preparation witness",
    "effect": "Upload inert dependent entry witness",
}
E_VALIDATION = "Validate closed dependent identity and write entry witness"


def e_bytes(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def e_indexed(evidence: Evidence, item: dict[str, Any], limit: int = MAX_JSON) -> bytes:
    require(
        isinstance(item, dict)
        and item in evidence.state["files"]
        and Path(item.get("path", "")).name == item.get("path")
        and type(item.get("bytes")) is int
        and 0 <= item["bytes"] <= limit,
        "E indexed evidence binding unavailable",
    )
    path = evidence.directory / item["path"]
    require(
        not path.is_symlink() and path.is_file() and path.stat().st_size == item["bytes"],
        "E indexed evidence shape changed",
    )
    with path.open("rb") as stream:
        raw = stream.read(limit + 1)
    require(
        len(raw) == item["bytes"] and hashlib.sha256(raw).hexdigest() == item["sha256"],
        "E indexed evidence bytes changed",
    )
    return raw


def e_source(evidence: Evidence) -> str:
    source = evidence.state["source_descriptor"]
    require(
        evidence.state["source_sha"] == source["source_sha"], "E draft source identity mismatch"
    )
    files = [item for item in evidence.state["files"] if item["path"].endswith("-source-manifest")]
    require(len(files) == 1, "E exact source manifest unavailable")
    raw = e_indexed(evidence, files[0], 65536)
    require(
        hashlib.sha256(raw).hexdigest() == evidence.state.get("source_manifest_sha256")
        and decode(raw) == source,
        "E source catalog binding mismatch",
    )
    return hashlib.sha256(e_bytes(source)).hexdigest()


def e_controller(evidence: Evidence) -> dict[str, Any]:
    from .provider_probe_controller import CONTROLLER_SCHEMA

    controller = evidence.state.get("controller_v5", {})
    require(
        controller.get("schema") == CONTROLLER_SCHEMA
        and re.fullmatch("[a-f0-9]{64}", controller.get("sha256", "")),
        "E v5 controller identity unavailable",
    )
    return controller


def e_identity(evidence: Evidence, row: dict[str, Any], run: int) -> tuple[object, ...]:
    require(
        isinstance(row, dict)
        and row.get("name") in CATALOG
        and row.get("catalog_name_shape") == list(CATALOG[row["name"]])
        and row.get("run_id") == run
        and type(row.get("run_attempt")) is int
        and row.get("run_attempt") == 1
        and row.get("head_sha") == evidence.state["source_sha"],
        "E source role mapping unavailable",
    )
    identifier = positive(row.get("id"))
    url = row.get("check_run_url")
    match = re.fullmatch(
        r"https://api\.github\.com/repos/Strukturpiloten/boxferry/check-runs/([1-9][0-9]*)",
        url or "",
    )
    require(match is not None, "E exact check identity unavailable")
    return (
        identifier,
        row["name"],
        row["run_id"],
        row["run_attempt"],
        row["head_sha"],
        url,
        tuple(row["catalog_name_shape"]),
    )


def e_auth(
    evidence: Evidence, observation: dict[str, Any], row: dict[str, Any], run: int
) -> dict[str, Any]:
    require(
        isinstance(observation, dict)
        and observation.get("outcome") == "OBSERVED"
        and observation.get("case") == "E"
        and observation.get("run") == run,
        "E authenticated observation unavailable",
    )
    job, check = object_field(observation, "job_record"), object_field(observation, "check_record")
    observed = {**job, "catalog_name_shape": observation.get("catalog_name_shape")}
    require(
        e_identity(evidence, row, run) == e_identity(evidence, observed, run)
        and observation.get("job") == job["id"]
        and observation.get("check") == check.get("id")
        and job["check_run_url"]
        == f"https://api.github.com/repos/{REPO}/check-runs/{check.get('id')}"
        and check.get("name") == job["name"]
        and check.get("head_sha") == evidence.state["source_sha"]
        and object_field(check, "app").get("id") == 15368
        and object_field(check, "app").get("slug") == "github-actions"
        and check.get("details_url")
        == f"https://github.com/{REPO}/actions/runs/{run}/job/{job['id']}"
        and job.get("status") == check.get("status")
        and job.get("conclusion") == check.get("conclusion")
        and all(row.get(key) == job.get(key) for key in ("status", "conclusion")),
        "E authenticated job/check or inventory contradiction",
    )
    # An observation must also have its actual immutable normalized custody record.
    require(
        any(
            decode(e_indexed(evidence, item)) == observation
            for item in evidence.state["files"]
            if item["path"].endswith("-bound")
        ),
        "E authenticated observation custody unavailable",
    )
    return observation


def e_steps(
    observation: dict[str, Any],
    role: str,
    active: bool = False,
    expected: dict[str, Any] | None = None,
) -> dict[str, Any]:
    steps = observation["job_record"].get("steps")
    require(
        isinstance(steps, list) and all(isinstance(step, dict) for step in steps),
        "E source steps unavailable",
    )
    numbers = [step.get("number") for step in steps]
    require(
        all(type(number) is int and number > 0 for number in numbers)
        and len(numbers) == len(set(numbers)),
        "E duplicate or invalid step numbers",
    )
    names = (
        [E_UPLOADS[role]]
        if role == "prepare"
        else [E_VALIDATION, E_UPLOADS["effect"], HOLDS["effect"]]
    )
    selected = {}
    for name in names:
        rows = [step for step in steps if step.get("name") == name]
        require(len(rows) == 1, "E exact source step missing or duplicated")
        selected[name] = rows[0]
    order = [selected[name]["number"] for name in names]
    require(
        order == sorted(order) and len(order) == len(set(order)),
        "E validation upload hold order invalid",
    )
    for name in names[:1] if role == "prepare" else names[:2]:
        require(
            selected[name].get("status") == "completed"
            and selected[name].get("conclusion") == "success",
            "E required upload or validation did not succeed",
        )
    if role == "effect":
        hold = selected[HOLDS["effect"]]
        require(
            hold.get("status") == ("in_progress" if active else "completed")
            and hold.get("conclusion") == (None if active else "cancelled"),
            "E exact effect hold phase unavailable",
        )
    result = {name: selected[name]["number"] for name in names}
    require(expected is None or result == expected, "E source step identity changed")
    return result


def e_inventory(
    evidence: Evidence, snapshot: dict[str, Any], authenticated_after: bool = True
) -> tuple[dict[int, tuple[object, ...]], dict[tuple[str, str], Any], dict[str, Any]]:
    run = evidence.state["runs"]["E"]["id"]
    require(
        isinstance(snapshot, dict)
        and snapshot.get("case") == "E"
        and snapshot.get("run") == run
        and snapshot.get("outcome") == "OBSERVED"
        and isinstance(snapshot.get("jobs"), list)
        and isinstance(snapshot.get("artifacts"), list),
        "E complete inventory unavailable",
    )
    require(
        any(
            decode(e_indexed(evidence, item)) == snapshot
            for item in evidence.state["files"]
            if item["path"].endswith("-snapshot")
        ),
        "E complete inventory custody unavailable",
    )
    identities, shapes, observations = {}, {}, {}
    for row in snapshot["jobs"]:
        identity = e_identity(evidence, row, run)
        shape = tuple(row["catalog_name_shape"])
        require(
            row["id"] not in identities and shape not in shapes,
            "E duplicate or parallel role identity",
        )
        identities[row["id"]], shapes[shape] = identity, row
        if authenticated_after:
            candidates = [
                observation
                for observation in evidence.state["observations"].values()
                if observation.get("case") == "E"
                and observation.get("run") == run
                and observation.get("job") == row["id"]
                and observation.get("started_elapsed", -1) > snapshot["elapsed"]
            ]
            require(candidates, "E exposed role lacks post-inventory authentication")
            observation = max(candidates, key=lambda value: value["elapsed"])
            observations[shape] = e_auth(evidence, observation, row, run)
    require(
        ("prepare", "expanded-leaf") in shapes and ("effect", "expanded-leaf") in shapes,
        "E unique required expanded leaves unavailable",
    )
    for observation in evidence.state["observations"].values():
        if (
            observation.get("case") == "E"
            and observation.get("run") == run
            and observation.get("started_elapsed", -1) > snapshot["elapsed"]
        ):
            row = {
                **object_field(observation, "job_record"),
                "catalog_name_shape": observation.get("catalog_name_shape"),
            }
            require(
                row.get("id") in identities
                and e_identity(evidence, row, run) == identities[row["id"]],
                "E new replaced or unmapped observed role",
            )
            current = next(item for item in snapshot["jobs"] if item["id"] == row["id"])
            e_auth(evidence, observation, current, run)
    artifacts = snapshot["artifacts"]
    require(
        len(artifacts) == 2 and len({row["id"] for row in artifacts}) == 2,
        "E unique preparation and entry metadata required",
    )
    byrole = {}
    for artifact in artifacts:
        evidence.validate_artifact(artifact, "E", run)
        role = "prepare" if artifact["name"].endswith("-prepare") else "effect"
        require(role not in byrole, "E duplicate artifact role")
        byrole[role] = artifact
    require(
        set(byrole) == {"prepare", "effect"}, "E entry metadata not observed before cancellation"
    )
    return identities, observations, byrole


def e_phase_log(
    evidence: Evidence, case: str, run: int, job: int, check: int, phase: str | None
) -> dict[str, Any]:
    require(case == "E" and phase in {E_PRE, E_POST}, "E explicit uploader evidence phase required")
    candidates = [
        value
        for value in evidence.state["observations"].values()
        if value.get("case") == case
        and value.get("run") == run
        and value.get("job") == job
        and value.get("check") == check
    ]
    require(candidates, "E uploader job/check authentication unavailable")
    observation = max(candidates, key=lambda value: value["elapsed"])
    role = "prepare" if phase == E_PRE else "effect"
    require(
        observation.get("catalog_name_shape") == [role, "expanded-leaf"],
        "E uploader role phase mismatch",
    )
    if phase == E_PRE:
        require(
            not evidence.state.get("e_v5", {}).get("pre_cancel_cut")
            and "cancel:E" not in evidence.state.get("controller_v5", {}).get("effects", {}),
            "E preparation attribution cannot be filled after cut or cancellation",
        )
        successful(observation, "prepare")
    else:
        cut = evidence.state.get("e_v5", {}).get("pre_cancel_cut")
        cancel = evidence.state.get("controller_v5", {}).get("effects", {}).get("cancel:E", {})
        require(
            cut
            and cancel.get("state") == "accepted"
            and (job, check) == (cut["effect"]["job"], cut["effect"]["check"]),
            "E post-cancel uploader lacks accepted same-effect cancellation",
        )
        first = evidence.state["snapshots"].get("E-terminal")
        stable = evidence.state["snapshots"].get("E-terminal-stable")
        require(
            first
            and stable
            and stable["elapsed"] - first["elapsed"] >= 5
            and observation["started_elapsed"] > stable["elapsed"],
            "E terminal stable inventory required before effect log",
        )
        e_terminal_inventories(evidence, cut, first, stable)
        require(
            all(
                observation[key].get("status") == "completed"
                and observation[key].get("conclusion") == "cancelled"
                for key in ("job_record", "check_record")
            ),
            "E active or naturally completed effect log forbidden",
        )
    e_source(evidence)
    row = {**observation["job_record"], "catalog_name_shape": observation["catalog_name_shape"]}
    e_auth(evidence, observation, row, run)
    e_steps(observation, role, active=False) if role == "effect" else e_steps(observation, role)
    return observation


def e_provenance(evidence: Evidence, artifact: dict[str, Any], phase: str) -> dict[str, Any]:
    record = evidence.state.get("e_v5_provenance", {}).get(str(artifact["id"]))
    role = "prepare" if phase == E_PRE else "effect"
    require(
        isinstance(record, dict)
        and record.get("phase") == phase
        and record.get("role") == role
        and record.get("uploader") == evidence.state["source_descriptor"]["uploader"]
        and record.get("artifact_id") == artifact["id"]
        and record.get("artifact_digest") == artifact["digest"][7:]
        and record.get("artifact") == artifact
        and record.get("source_sha") == evidence.state["source_sha"]
        and record.get("series") == evidence.state["series"]
        and record.get("case") == "E"
        and record.get("run") == evidence.state["runs"]["E"]["id"]
        and record.get("attempt") == 1
        and record.get("source_catalog_sha256") == e_source(evidence),
        "E structured uploader provenance unavailable",
    )
    raw = e_indexed(evidence, record["record_file"])
    payload = {key: value for key, value in record.items() if key != "record_file"}
    require(decode(raw) == payload, "E uploader provenance custody changed")
    log = evidence.state.get("e_v5_logs", {}).get(record.get("log_label"))
    require(
        log
        and log == record.get("log")
        and log.get("phase") == phase
        and log.get("job") == record.get("job")
        and log.get("check") == record.get("check")
        and log.get("source_sha") == evidence.state["source_sha"]
        and log.get("case") == "E"
        and log.get("series") == evidence.state["series"]
        and log.get("attempt") == 1
        and log.get("run") == record["run"]
        and log.get("source_catalog_sha256") == record["source_catalog_sha256"],
        "E uploader log provenance binding changed",
    )
    require(
        type(log.get("elapsed")) in {int, float}
        and type(record.get("elapsed")) in {int, float}
        and log["raw_file"]["elapsed"]
        <= log["elapsed"]
        <= record["elapsed"]
        <= record["record_file"]["elapsed"],
        "E uploader provenance time order invalid",
    )
    lograw = e_indexed(evidence, log["raw_file"])
    require(
        hashlib.sha256(lograw).hexdigest() == log["sha256"], "E uploader raw log binding changed"
    )
    start, end = record["line_start"], record["line_end"]
    lines = lograw.decode("utf-8").splitlines(keepends=True)
    require(
        type(start) is int and type(end) is int and 1 <= start <= end <= len(lines),
        "E uploader span invalid",
    )
    span = "".join(lines[start - 1 : end]).encode()
    require(
        hashlib.sha256(span).hexdigest() == record["span_sha256"]
        and span.count(evidence.state["source_descriptor"]["uploader"].encode()) == 1
        and set(re.findall(rb"(?<![A-Fa-f0-9])[a-f0-9]{64}(?![A-Fa-f0-9])", span))
        == {record["artifact_digest"].encode()}
        and {int(value) for value in re.findall(rb"Artifact ID(?: is|:)?\s+([1-9][0-9]*)", span)}
        == {artifact["id"]}
        and record.get("attestation") == "operator-authenticated-exact-source-uploader-step",
        "E exact operator uploader span binding unavailable",
    )
    role = "prepare" if phase == E_PRE else "effect"
    observation = log["observation"]
    row = {**observation["job_record"], "catalog_name_shape": observation["catalog_name_shape"]}
    e_auth(evidence, observation, row, record["run"])
    require(
        observation["catalog_name_shape"] == [role, "expanded-leaf"]
        and record.get("step_name") == E_UPLOADS[role]
        and record.get("step_number") == e_steps(observation, role)[E_UPLOADS[role]],
        "E uploader source step identity mismatch",
    )
    return record


def e_archive(evidence: Evidence, artifact: dict[str, Any], phase: str) -> dict[str, Any]:
    provenance = e_provenance(evidence, artifact, phase)
    record = evidence.state["runs"]["E"].get("verified_artifacts", {}).get(str(artifact["id"]))
    require(
        isinstance(record, dict)
        and record.get("phase") == phase
        and record.get("uploader_provenance_file") == provenance["record_file"]
        and record.get("artifact_id") == artifact["id"]
        and record.get("run") == evidence.state["runs"]["E"]["id"]
        and record.get("case") == "E"
        and record.get("outcome") == "OBSERVED",
        "E phase-specific verified archive unavailable",
    )
    raw = e_indexed(evidence, record["archive_file"], MAX_ARCHIVE)
    witness = verify_archive(
        raw, artifact, evidence.state, "E", record["run"], provenance["artifact_digest"]
    )
    require(
        record.get("archive_sha256") == hashlib.sha256(raw).hexdigest()
        and record.get("archive_bytes") == len(raw)
        and record.get("witness") == witness
        and record.get("witness_sha256") == hashlib.sha256(witness_bytes(raw)).hexdigest(),
        "E verified archive custody mismatch",
    )
    return record


def e_pre_proof(evidence: Evidence, args: argparse.Namespace) -> dict[str, Any]:
    source_catalog = e_source(evidence)
    inventory = evidence.state["snapshots"].get("E-inventory-before")
    identities, observations, artifacts = e_inventory(evidence, inventory)
    preparation = evidence.state["observations"][args.prepare]
    dependent = evidence.state["observations"][args.effect]
    run = evidence.state["runs"]["E"]["id"]
    require(
        inventory.get("run_status") == "in_progress" and inventory.get("run_conclusion") is None,
        "E pre-cancel active complete inventory required",
    )
    for role, observation in [("prepare", preparation), ("effect", dependent)]:
        row = next(
            row
            for row in inventory["jobs"]
            if tuple(row["catalog_name_shape"]) == (role, "expanded-leaf")
        )
        e_auth(evidence, observation, row, run)
    successful(preparation, "prepare")
    holding(dependent, "effect")
    prep_steps, effect_steps = (
        e_steps(preparation, "prepare"),
        e_steps(dependent, "effect", active=True),
    )
    require(
        inventory["elapsed"]
        < dependent["started_elapsed"]
        <= dependent["elapsed"]
        < preparation["started_elapsed"]
        <= preparation["elapsed"],
        "E inventory hold preparation phase order invalid",
    )
    archive = e_archive(evidence, artifacts["prepare"], E_PRE)
    provenance = e_provenance(evidence, artifacts["prepare"], E_PRE)
    require(
        provenance["elapsed"] <= archive["elapsed"] < dependent["started_elapsed"],
        "E attributed preparation archive must precede final effect hold",
    )
    require(
        all(
            record.get("phase") == E_PRE
            for record in evidence.state["runs"]["E"].get("verified_artifacts", {}).values()
        ),
        "E post-cancel archive cannot enter pre-cancel proof",
    )
    wrappers = {}
    for shape, observation in observations.items():
        if shape[1] == "outer-wrapper":
            if shape[0] == "prepare":
                require(
                    all(
                        observation[key].get("status") == "completed"
                        and observation[key].get("conclusion") == "success"
                        for key in ("job_record", "check_record")
                    ),
                    "E observed preparation wrapper contradicts success",
                )
            else:
                require(
                    all(
                        observation[key].get("status") == "in_progress"
                        and observation[key].get("conclusion") is None
                        for key in ("job_record", "check_record")
                    ),
                    "E observed effect wrapper contradicts active leaf",
                )
            wrappers[shape[0]] = observation
    controller = e_controller(evidence)
    return {
        "schema": "boxferry.provider632.E-cut.v5",
        "series": evidence.state["series"],
        "case": "E",
        "run": run,
        "attempt": 1,
        "source_sha": evidence.state["source_sha"],
        "source_manifest_sha256": evidence.state["source_manifest_sha256"],
        "source_catalog_sha256": source_catalog,
        "observer_sha256": evidence.state["observer_sha256"],
        "controller_sha256": controller["sha256"],
        "inventory": inventory,
        "role_identities": identities,
        "preparation": preparation,
        "effect": dependent,
        "preparation_steps": prep_steps,
        "effect_steps": effect_steps,
        "preparation_archive": archive,
        "preparation_provenance": provenance,
        "effect_entry_metadata": artifacts["effect"],
        "wrappers": wrappers,
        "wrapper_scope": "OBSERVED" if "prepare" in wrappers else "CALLER_WRAPPER_UNOBSERVED",
    }


def guard_e(evidence: Evidence, args: argparse.Namespace) -> dict[str, Any]:
    proof = decode(e_bytes(e_pre_proof(evidence, args)))
    state = evidence.state.setdefault("e_v5", {})
    cut = state.get("pre_cancel_cut")
    require(
        "cancel:E" not in evidence.state["controller_v5"]["effects"],
        "E pre-cancel guard cannot run after reservation",
    )
    if cut is None:
        cut = {
            **proof,
            "elapsed": evidence.elapsed(),
            "utc": utc(),
            "files": list(evidence.state["files"]),
        }
        cut["cut_sha256"] = hashlib.sha256(e_bytes(cut)).hexdigest()
        state["pre_cancel_cut"] = cut
        evidence.save("E-v5-pre-cancel-cut", e_bytes(cut))
    else:
        require(
            all(cut.get(key) == value for key, value in proof.items()),
            "E frozen pre-cancel proof changed",
        )
        require(
            hashlib.sha256(
                e_bytes({key: value for key, value in cut.items() if key != "cut_sha256"})
            ).hexdigest()
            == cut.get("cut_sha256"),
            "E pre-cancel cut integrity mismatch",
        )
        require(
            any(
                decode(e_indexed(evidence, item)) == cut
                for item in evidence.state["files"]
                if item["path"].endswith("-E-v5-pre-cancel-cut")
            ),
            "E pre-cancel cut custody missing",
        )
    return {
        "outcome": "OBSERVED",
        "meaning": "phase-typed pre-cancel authorization evidence only",
        "cut_sha256": cut["cut_sha256"],
        "effect_entry_provenance": "DEFERRED_POST_CANCELLATION",
        "caller_wrapper_retention": "UNOBSERVED",
        "scope": cut["wrapper_scope"],
    }


def e_terminal_inventories(
    evidence: Evidence, cut: dict[str, Any], first: dict[str, Any], stable: dict[str, Any]
) -> dict[int, tuple[object, ...]]:
    require(
        first
        and stable
        and first["elapsed"] > cut["elapsed"]
        and stable["elapsed"] - first["elapsed"] >= 5,
        "E first and spaced stable terminal inventories required",
    )
    cancel = evidence.state.get("controller_v5", {}).get("effects", {}).get("cancel:E", {})
    boundary = evidence.state.get("controller_v5", {}).get("E_cancel_boundary", {})
    require(
        boundary.get("cut_sha256") == cut["cut_sha256"]
        and boundary.get("run") == cut["run"]
        and boundary.get("attempt") == 1
        and boundary.get("hold_elapsed") == cut["effect"]["elapsed"]
        and cut["elapsed"]
        <= boundary.get("fresh_run_elapsed", -1)
        <= cancel.get("before_elapsed", -1)
        and 0 <= cancel.get("before_elapsed", -1) - cut["effect"]["elapsed"] <= 5,
        "E cancellation boundary differs from fresh frozen cut",
    )
    require(
        cancel.get("state") == "accepted"
        and cancel.get("before_elapsed", -1) >= cut["elapsed"]
        and cancel.get("case") == "E"
        and cancel.get("kind") == "planned-fixture-cancellation"
        and cancel.get("method") == "POST"
        and cancel.get("payload") is None
        and cancel.get("source_sha") == cut["source_sha"]
        and cancel.get("series") == cut["series"]
        and cancel.get("path") == f"/repos/{REPO}/actions/runs/{cut['run']}/cancel",
        "E final qualification lacks spent accepted planned cancellation",
    )
    require(
        type(boundary.get("accepted_elapsed")) in {int, float}
        and cancel["before_elapsed"] <= boundary["accepted_elapsed"] < first["started_elapsed"],
        "E terminal evidence predates accepted cancellation phase close",
    )
    identities = None
    for snapshot in (first, stable):
        require(
            snapshot["run_status"] == "completed" and snapshot["run_conclusion"] == "cancelled",
            "E actual root cancellation unavailable",
        )
        current, _, artifacts = e_inventory(evidence, snapshot, authenticated_after=False)
        require(
            decode(e_bytes(current)) == cut["role_identities"]
            and (identities is None or current == identities),
            "E terminal new replaced parallel identity",
        )
        identities = current
        require(
            artifacts["effect"] == cut["effect_entry_metadata"],
            "E entry metadata ID or content changed",
        )
        for artifact in snapshot["artifacts"]:
            original = next(
                row for row in cut["inventory"]["artifacts"] if row["id"] == artifact["id"]
            )
            require(artifact == original, "E stable artifact metadata changed")
        byid = {row["id"]: row for row in snapshot["jobs"]}
        prep, effect = byid[cut["preparation"]["job"]], byid[cut["effect"]["job"]]
        require(
            prep.get("status") == "completed" and prep.get("conclusion") == "success",
            "E preparation success changed after cancellation",
        )
        require(
            effect.get("status") == "completed" and effect.get("conclusion") == "cancelled",
            "E accepted cancel followed by natural or ambiguous effect completion",
        )
        for row in snapshot["jobs"]:
            if row["catalog_name_shape"] == ["prepare", "outer-wrapper"]:
                require(
                    row.get("status") == "completed" and row.get("conclusion") == "success",
                    "E terminal preparation wrapper contradicts retention",
                )
            elif row["catalog_name_shape"] == ["effect", "outer-wrapper"]:
                require(
                    row.get("status") == "completed" and row.get("conclusion") == "cancelled",
                    "E terminal effect wrapper contradicts cancelled leaf",
                )
    # Positive same-ID terminal job/check authentication is required in both windows.
    for row in first["jobs"]:
        candidates = [
            observation
            for observation in evidence.state["observations"].values()
            if observation.get("case") == "E"
            and observation.get("run") == cut["run"]
            and observation.get("job") == row["id"]
            and first["elapsed"]
            < observation.get("started_elapsed", -1)
            <= observation.get("elapsed", -1)
            < stable["started_elapsed"]
        ]
        require(candidates, "E first-terminal role authentication missing before stable inventory")
        e_auth(evidence, max(candidates, key=lambda value: value["elapsed"]), row, cut["run"])
    return identities


def compare_e(evidence: Evidence, args: argparse.Namespace) -> dict[str, Any]:
    state = evidence.state.setdefault("e_v5", {})
    facts = {
        "effect_cancellation_observed": False,
        "preparation_leaf_retention_observed": False,
        "effect_entry_provenance_verified": False,
        "caller_wrapper_retention": "UNSUPPORTED",
        "outcome": "INCOMPLETE_E_EVIDENCE",
        "production_ready": False,
    }
    state["qualification_facts"] = facts
    evidence.persist()
    cut = state.get("pre_cancel_cut")
    require(
        cut
        and hashlib.sha256(
            e_bytes({key: value for key, value in cut.items() if key != "cut_sha256"})
        ).hexdigest()
        == cut.get("cut_sha256"),
        "E frozen pre-cancel cut unavailable for qualification",
    )
    require(
        evidence.state.get("controller_v5", {}).get("E_durable_readback", {}).get("cut_sha256")
        == cut["cut_sha256"],
        "E final evidence lacks exact durable pre-cancel cut readback",
    )
    require(
        any(
            decode(e_indexed(evidence, item)) == cut
            for item in evidence.state["files"]
            if item["path"].endswith("-E-v5-pre-cancel-cut")
        ),
        "E frozen cut custody missing at qualification",
    )
    before = evidence.state["observations"][args.prepare_before]
    after = evidence.state["observations"][args.prepare_after]
    require(
        before == cut["preparation"]
        and args.artifact == cut["preparation_archive"]["artifact_id"]
        and evidence.state["snapshots"][args.inventory_before] == cut["inventory"]
        and args.inventory_after == "E-terminal-stable",
        "E qualification arguments differ from frozen cut",
    )
    first, stable = (
        evidence.state["snapshots"].get("E-terminal"),
        evidence.state["snapshots"].get("E-terminal-stable"),
    )
    e_terminal_inventories(evidence, cut, first, stable)
    _, observations, artifacts = e_inventory(evidence, stable)
    preparation, effect = (
        observations[("prepare", "expanded-leaf")],
        observations[("effect", "expanded-leaf")],
    )
    require(
        after == preparation
        and (preparation["job"], preparation["check"])
        == (cut["preparation"]["job"], cut["preparation"]["check"]),
        "E same preparation post-stable authentication missing",
    )
    successful(preparation, "prepare")
    e_steps(preparation, "prepare", expected=cut["preparation_steps"])
    require(
        (effect["job"], effect["check"]) == (cut["effect"]["job"], cut["effect"]["check"])
        and all(
            effect[key].get("status") == "completed"
            and effect[key].get("conclusion") == "cancelled"
            for key in ("job_record", "check_record")
        ),
        "E same effect cancelled job/check unobserved",
    )
    e_steps(effect, "effect", expected=cut["effect_steps"])
    facts["effect_cancellation_observed"] = True
    archive = e_archive(evidence, artifacts["prepare"], E_PRE)
    require(
        archive == cut["preparation_archive"]
        and e_provenance(evidence, artifacts["prepare"], E_PRE) == cut["preparation_provenance"],
        "E preparation archive or provenance changed since cut",
    )
    facts["preparation_leaf_retention_observed"] = True
    facts["caller_wrapper_retention"] = (
        "UNOBSERVED" if "prepare" not in cut["wrappers"] else "UNSUPPORTED"
    )
    if "prepare" in cut["wrappers"]:
        wrapper = observations.get(("prepare", "outer-wrapper"))
        require(
            wrapper is not None
            and (wrapper["job"], wrapper["check"])
            == (cut["wrappers"]["prepare"]["job"], cut["wrappers"]["prepare"]["check"])
            and all(
                wrapper[key].get("status") == "completed"
                and wrapper[key].get("conclusion") == "success"
                for key in ("job_record", "check_record")
            ),
            "E caller wrapper retention contradictory or unsupported",
        )
        facts["caller_wrapper_retention"] = "OBSERVED"
    evidence.persist()  # preserve separate established facts if final corroboration refuses
    final = e_archive(evidence, artifacts["effect"], E_POST)
    provenance = e_provenance(evidence, artifacts["effect"], E_POST)
    require(
        provenance["elapsed"] > stable["elapsed"] and final["elapsed"] >= provenance["elapsed"],
        "E final effect provenance has wrong phase time",
    )
    witness = final["witness"]
    require(
        witness["preparation_artifact_id"] == str(archive["artifact_id"])
        and witness["preparation_artifact_digest"] == archive["archive_sha256"]
        and witness["caller_prerequisite_result"] == "success",
        "E effect inherited preparation linkage mismatch",
    )
    facts["effect_entry_provenance_verified"] = True
    facts["outcome"] = (
        "OBSERVED_E_RETENTION_WITH_WRAPPER"
        if facts["caller_wrapper_retention"] == "OBSERVED"
        else "OBSERVED_LEAF_RETENTION_ONLY"
    )
    facts["qualification"] = (
        "SAME_WRAPPER_RETENTION_OBSERVED"
        if facts["caller_wrapper_retention"] == "OBSERVED"
        else "CALLER_WRAPPER_UNOBSERVED"
    )
    facts["meaning"] = (
        "bounded inert E observation; no full fixture PASS or production recovery capability"
    )
    evidence.persist()
    return facts


def initialize(args: argparse.Namespace) -> dict[str, Any]:
    sha(args.source_sha)
    require(re.fullmatch("[a-f0-9]{32}", args.series), "invalid series")
    positive(args.workflow_id), positive(args.actor_id), positive(args.triggering_actor_id)
    require(
        re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9-]{0,99}(?:\[bot\])?", args.actor)
        and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9-]{0,99}(?:\[bot\])?", args.triggering_actor),
        "invalid actor",
    )
    source_path = Path(args.source_manifest).absolute()
    require(
        all(not part.is_symlink() for part in (source_path, *source_path.parents))
        and source_path.is_file()
        and source_path.stat().st_size <= 65536,
        "source manifest must be a bounded regular file",
    )
    with source_path.open("rb") as stream:
        raw = stream.read(65537)
    require(len(raw) <= 65536, "source manifest exceeds bound")
    source = decode(raw)
    require(isinstance(source, dict), "source manifest must be an object")
    from .provider_probe_source import verify_descriptor

    verify_descriptor(source, raw, args.descriptor_sha256)
    require(
        source.get("source_sha") == args.source_sha and source.get("repository") == REPO,
        "source manifest identity mismatch",
    )
    started_utc = utc()
    started_monotonic = time.monotonic()
    directory = checked_custody(args.custody)
    with open_directory(directory.parent) as parent:
        try:
            os.stat(directory.name, dir_fd=parent, follow_symlinks=False)
        except FileNotFoundError:
            pass
        else:
            raise Inconclusive("custody already exists; series budget cannot be reset")
        # Registration is exclusive and durable before the new custody can be used.
        registration_name = "provider632-series-" + args.series + ".json"
        try:
            with open_directory(REGISTRATION_ROOT) as registrations:
                descriptor = os.open(
                    registration_name,
                    os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                    0o600,
                    dir_fd=registrations,
                )
                with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
                    json.dump(
                        {
                            "series": args.series,
                            "custody": str(directory),
                            "source_sha": args.source_sha,
                            "registered_utc": utc(),
                        },
                        stream,
                        sort_keys=True,
                    )
                    stream.flush()
                    os.fsync(stream.fileno())
                os.fsync(registrations)
        except FileExistsError:
            raise Inconclusive("series already registered; budget reset refused") from None
        os.mkdir(directory.name, mode=0o700, dir_fd=parent)
        info = os.stat(directory.name, dir_fd=parent, follow_symlinks=False)
        directory_identity = (info.st_dev, info.st_ino)
        os.fsync(parent)
    state = {
        "schema": OBSERVER_SCHEMA,
        "series": args.series,
        "custody_directory": str(directory),
        "observer_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "source_sha": args.source_sha,
        "workflow_id": args.workflow_id,
        "actor": {"login": args.actor, "id": args.actor_id},
        "triggering_actor": {"login": args.triggering_actor, "id": args.triggering_actor_id},
        "started_utc": started_utc,
        "started_monotonic": started_monotonic,
        "boot": Path("/proc/sys/kernel/random/boot_id").read_text().strip(),
        "source_manifest_sha256": hashlib.sha256(raw).hexdigest(),
        "requests": 0,
        "transport": [],
        "source_descriptor": source,
        "source_verification": "local exact admitted Git closure; remote preflight still required",
        "files": [],
        "runs": {},
        "reads": {},
        "snapshots": {},
        "observations": {},
        "refusals": [],
        "labels": {},
        "terminal_runs": {},
        "check_to_run": {},
        "discovery_reads": {},
        "discovery_reports": [],
    }
    write_json(directory / "state.json", state, directory_identity)
    evidence = object.__new__(Evidence)
    evidence.read_only = False
    evidence.directory_identity = directory_identity
    evidence.directory, evidence.state_path, evidence.state = (
        directory,
        directory / "state.json",
        state,
    )
    evidence.save("source-manifest", raw)
    evidence.save("observer-source.py", Path(__file__).read_bytes())
    return {"outcome": "LOCAL_INITIALIZATION_ONLY", "custody": str(directory)}


@contextlib.contextmanager
def locked(directory: str | Path) -> Generator[None]:
    path = checked_custody(directory)
    verified = Evidence(path, reconcile=False)  # No lock/marker/write in invalid or old custody.
    with open_directory(path) as custody:
        info = os.fstat(custody)
        require(
            (info.st_dev, info.st_ino) == verified.directory_identity,
            "custody directory identity changed",
        )
        descriptor = os.open(
            "observer.lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600, dir_fd=custody
        )
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            yield
        except BlockingIOError:
            raise Inconclusive("another observer invocation holds custody lock") from None
        finally:
            os.close(descriptor)


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    commands = result.add_subparsers(dest="command", required=True)
    init = commands.add_parser(
        "init", help="local initialization; starts immutable 30-minute clock"
    )
    for field in (
        "custody",
        "series",
        "source-sha",
        "source-manifest",
        "actor",
        "triggering-actor",
    ):
        init.add_argument("--" + field, required=True)
    init.add_argument("--descriptor-sha256", required=True)
    for field in ("workflow-id", "actor-id", "triggering-actor-id"):
        init.add_argument("--" + field, required=True, type=int)
    for name in (
        "snapshot",
        "capture-terminal",
        "poll-run",
        "job",
        "artifact",
        "log",
        "guard-burst",
        "guard-e",
        "compare-e",
        "uploader-provenance",
        "status",
    ):
        command = commands.add_parser(name)
        command.add_argument("--custody", required=True)
        if name in {"snapshot", "capture-terminal", "poll-run", "job", "artifact", "log"}:
            command.add_argument("--case", required=True, choices=list("ABCDE"))
            command.add_argument("--run", required=True, type=int)
            command.add_argument("--label", required=True)
        if name in {"job", "log"}:
            command.add_argument("--job", required=True, type=int)
            command.add_argument("--check", required=True, type=int)
        if name == "log":
            command.add_argument("--e-phase", choices=[E_PRE, E_POST])
        if name == "uploader-provenance":
            command.add_argument("--log-label", required=True)
            command.add_argument("--artifact", required=True, type=int)
            command.add_argument("--uploader-digest", required=True)
            command.add_argument("--step-number", required=True, type=int)
            command.add_argument("--line-start", required=True, type=int)
            command.add_argument("--line-end", required=True, type=int)
        if name == "artifact":
            command.add_argument("--artifact", required=True, type=int)
            command.add_argument("--uploader-digest", required=True)
        if name == "guard-burst":
            for field in ("before", "after", "victim", "pending"):
                command.add_argument("--" + field, required=True)
        if name == "guard-e":
            command.add_argument("--prepare", required=True)
            command.add_argument("--effect", required=True)
        if name == "compare-e":
            for field in ("prepare-before", "prepare-after", "inventory-before", "inventory-after"):
                command.add_argument("--" + field, required=True)
            command.add_argument("--artifact", required=True, type=int)
    return result


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    if args.command == "init":
        print(json.dumps(initialize(args), sort_keys=True))
        return 0
    with locked(args.custody):
        evidence = Evidence(args.custody)
        evidence.observation_marker()
        try:
            if hasattr(args, "label"):
                require(re.fullmatch(r"[A-Za-z0-9_-]{1,40}", args.label), "invalid phase label")
                require(args.label not in evidence.state["labels"], "phase label already used")
                evidence.state["labels"][args.label] = {"command": args.command, "utc": utc()}
                evidence.persist()
            if args.command == "poll-run":
                value, _ = evidence.run(args.case, args.run, args.label)
                require(
                    value.get("status")
                    in {"queued", "requested", "waiting", "pending", "in_progress", "completed"},
                    "run polling status unsupported",
                )
                output = {
                    "outcome": "OBSERVED",
                    "run_status": value["status"],
                    "run_conclusion": value.get("conclusion"),
                    "case": args.case,
                    "run": args.run,
                    "elapsed": evidence.elapsed(),
                }
                evidence.save(
                    args.label + "-poll", (json.dumps(output, sort_keys=True) + "\n").encode()
                )
            elif args.command == "capture-terminal":
                output = evidence.capture_terminal(args.case, args.run, args.label)
            elif args.command == "snapshot":
                output = evidence.snapshot(args.case, args.run, args.label)
            elif args.command == "job":
                output = evidence.job(args.case, args.run, args.job, args.check, args.label)
            elif args.command == "log":
                output = evidence.log(
                    args.case, args.run, args.job, args.check, args.label, args.e_phase
                )
            elif args.command == "uploader-provenance":
                output = evidence.bind_e_uploader(
                    args.log_label,
                    args.artifact,
                    args.uploader_digest,
                    args.step_number,
                    args.line_start,
                    args.line_end,
                )
            elif args.command == "artifact":
                output = evidence.download(
                    args.case, args.run, args.artifact, args.uploader_digest, args.label
                )
            elif args.command == "guard-burst":
                output = guard_burst(evidence, args)
                evidence.save("burst-guard", (json.dumps(output, sort_keys=True) + "\n").encode())
            elif args.command == "guard-e":
                output = guard_e(evidence, args)
                evidence.save(
                    "E-pre-cancel-guard", (json.dumps(output, sort_keys=True) + "\n").encode()
                )
            elif args.command == "compare-e":
                output = compare_e(evidence, args)
                evidence.save(
                    "E-retention-comparison", (json.dumps(output, sort_keys=True) + "\n").encode()
                )
            else:
                output = {
                    "requests": evidence.state["requests"],
                    "elapsed": evidence.elapsed(),
                    "bound_runs": evidence.state["runs"],
                    "refusals": evidence.state["refusals"],
                }
            # Complete raw/normalized records stay in custody. Terminal output is only bounded
            # identity/status.
            print(
                json.dumps(
                    {
                        "outcome": output.get("outcome", "OBSERVED"),
                        "requests": evidence.state["requests"],
                        "files": len(evidence.state["files"]),
                        "custody": str(evidence.directory),
                        "run_status": output.get("run_status"),
                        "terminal_capture": output.get("terminal_capture"),
                    },
                    sort_keys=True,
                )
            )
            evidence.observation_marker(finish=True)
            return 0
        except (Exception, KeyboardInterrupt) as error:
            from .provider_probe_controller import refusal_code

            evidence.state["normal_aborted"] = True
            if "controller_v5" in evidence.state:
                evidence.state["controller_v5"]["aborted"] = True
            evidence.state["refusals"].append(
                {"utc": utc(), "refusal_code": refusal_code(error), "outcome": "INCONCLUSIVE"}
            )
            evidence.persist()
            evidence.observation_marker(finish=True)
            print(
                json.dumps(
                    {
                        "outcome": "INCONCLUSIVE",
                        "refusal_code": refusal_code(error),
                        "requests": evidence.state["requests"],
                    },
                    sort_keys=True,
                )
            )
            return 2
