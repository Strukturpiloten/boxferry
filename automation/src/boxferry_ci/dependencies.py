"""Bounded lockfile age checks. State and event context must come from trusted CI.

The returned first-seen ledger is observation evidence, never publication evidence.
Persist it outside candidate-controlled source and retry at ``retry_at``. No label,
PR title, candidate configuration, or first-party name alone grants an exemption.
"""

from __future__ import annotations

import base64
import hashlib
import json
import re
import signal
import threading
import time
import tomllib
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Callable, Generator, Mapping
from contextlib import contextmanager
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from email.message import Message
from pathlib import Path
from typing import IO, Any

from .git_source import CHECKSUMS, INVENTORY, inventory, match_checksums, source_record

SCHEMA_VERSION = 1
HOLD = timedelta(hours=72)
MAX_FILE_BYTES = 8 * 1024 * 1024
MAX_METADATA_BYTES = 2 * 1024 * 1024
MAX_TOTAL_METADATA_BYTES = 16 * 1024 * 1024
MAX_PACKAGES = 4096
MAX_INTRODUCED = 256
MAX_METADATA_REQUESTS = MAX_INTRODUCED
MAX_STATE_ENTRIES = 4096
CRATES_SOURCE = "registry+https://github.com/rust-lang/crates.io-index"
FIRST_PARTY = frozenset(
    {
        "boxferry",
        "boxferry-model",
        "boxferry-engine",
        "boxferry-compose",
        "boxferry-podman",
        "boxferry-quadlet",
        "compose-lens",
        "podman-lens",
        "quadlet-lens",
        "docker-lens",
        "kubernetes-lens",
    }
)
REPOSITORIES = frozenset(
    f"Strukturpiloten/{name}"
    for name in (
        "boxferry",
        "compose-lens",
        "podman-lens",
        "quadlet-lens",
        "docker-lens",
        "kubernetes-lens",
        "boxferry-website",
    )
)
Fetch = Callable[[str], bytes]
FetchRequest = Callable[[str, Mapping[str, str]], bytes]


class DependencyError(ValueError):
    """Invalid or unsupported dependency evidence."""


@dataclass(frozen=True)
class DependencyContext:
    """Facts obtained by trusted workflow code from the GitHub event/API.

    Never construct this object from the PR's files, title, labels, or environment
    produced by candidate code. Queue candidates without isolated PR context are
    deliberately checked as ordinary updates.
    """

    actor_login: str
    actor_type: str
    repository: str
    base_revision: str
    head_revision: str
    changed_paths: tuple[str, ...]

    @classmethod
    def from_mapping(cls, value: Mapping[str, Any]) -> DependencyContext:
        expected = {
            "actor_login",
            "actor_type",
            "repository",
            "base_revision",
            "head_revision",
            "changed_paths",
        }
        if set(value) != expected:
            raise DependencyError("context fields differ from the trusted context schema")
        paths = value["changed_paths"]
        if not isinstance(paths, list) or not all(isinstance(p, str) for p in paths):
            raise DependencyError("context changed_paths must be strings")
        for key in expected - {"changed_paths"}:
            if not isinstance(value[key], str):
                raise DependencyError(f"context {key} must be a string")
        return cls(
            actor_login=value["actor_login"],
            actor_type=value["actor_type"],
            repository=value["repository"],
            base_revision=value["base_revision"],
            head_revision=value["head_revision"],
            changed_paths=tuple(paths),
        )


@dataclass(frozen=True, order=True)
class Package:
    ecosystem: str
    name: str
    version: str
    source: str
    integrity: tuple[str, ...]

    @property
    def key(self) -> str:
        material = json.dumps(
            [self.ecosystem, self.name, self.version, self.source, self.integrity],
            separators=(",", ":"),
        )
        return hashlib.sha256(material.encode()).hexdigest()


def _read(path: Path) -> bytes:
    if path.is_symlink() or not path.is_file():
        raise DependencyError(f"expected a regular file: {path.name}")
    with path.open("rb") as stream:
        data = stream.read(MAX_FILE_BYTES + 1)
    if len(data) > MAX_FILE_BYTES:
        raise DependencyError(f"file exceeds {MAX_FILE_BYTES} byte bound: {path.name}")
    return data


def _toml(path: Path) -> dict[str, Any]:
    return tomllib.loads(_read(path).decode("utf-8"))


def _name(value: object) -> str:
    if not isinstance(value, str) or not re.fullmatch(r"[@A-Za-z0-9_.+/-]+", value):
        raise DependencyError("invalid registry package name or version")
    return value


def _sha256(value: object) -> str:
    if not isinstance(value, str) or not re.fullmatch(r"[a-f0-9]{64}", value):
        raise DependencyError("missing or invalid SHA-256 integrity")
    return value


def _cargo(path: Path) -> set[Package]:
    result = set()
    for item in _toml(path).get("package", []):
        source = item.get("source")
        if source is None:
            continue
        if source != CRATES_SOURCE:
            raise DependencyError("only the canonical crates.io Cargo registry is supported")
        result.add(
            Package(
                "cargo",
                _name(item["name"]),
                _name(item["version"]),
                source,
                (_sha256(item.get("checksum")),),
            )
        )
    return result


def _sri(value: object) -> str:
    if not isinstance(value, str) or not re.fullmatch(
        r"sha(?:256|384|512)-[A-Za-z0-9+/]+=*", value
    ):
        raise DependencyError("missing or unsupported npm artifact integrity")
    algorithm, digest = value.split("-", 1)
    try:
        raw = base64.b64decode(digest, validate=True)
    except ValueError as error:
        raise DependencyError("malformed npm artifact integrity") from error
    if len(raw) != int(algorithm[3:]) // 8:
        raise DependencyError("incorrect npm integrity digest length")
    return value


def _npm(path: Path) -> set[Package]:
    document = json.loads(_read(path))
    if document.get("lockfileVersion") not in (2, 3) or not isinstance(
        document.get("packages"), dict
    ):
        raise DependencyError("npm lockfile v2/v3 packages are required")
    result = set()
    for location, item in document["packages"].items():
        if not location or item.get("link"):
            continue
        if "node_modules/" not in location:
            raise DependencyError("unsupported npm lockfile package location")
        name = _name(item.get("name", location.rsplit("node_modules/", 1)[1]))
        version = _name(item["version"])
        url = urllib.parse.urlparse(item.get("resolved", ""))
        if (
            url.scheme != "https"
            or url.hostname != "registry.npmjs.org"
            or url.username
            or url.password
        ):
            raise DependencyError("only canonical npm registry artifacts are supported")
        result.add(
            Package(
                "npm",
                name,
                version,
                "https://registry.npmjs.org",
                (_sri(item.get("integrity")),),
            )
        )
    return result


def _uv(path: Path) -> set[Package]:
    result = set()
    for item in _toml(path).get("package", []):
        source = item.get("source", {})
        registry = source.get("registry")
        if registry is None:
            if "virtual" in source or "editable" in source or "directory" in source:
                continue
            raise DependencyError("unsupported uv package source")
        if registry.rstrip("/") != "https://pypi.org/simple":
            raise DependencyError("only the canonical PyPI registry is supported")
        artifacts = [*item.get("wheels", [])]
        if "sdist" in item:
            artifacts.append(item["sdist"])
        hashes = tuple(
            sorted({_sha256(a.get("hash", "").removeprefix("sha256:")) for a in artifacts})
        )
        if not hashes:
            raise DependencyError("uv registry package has no artifact integrity")
        name = re.sub(r"[-_.]+", "-", _name(item["name"])).lower()
        result.add(Package("python", name, _name(item["version"]), "https://pypi.org", hashes))
    return result


def _packages(root: Path) -> set[Package]:
    result: set[Package] = set()
    for name, parser in (
        ("Cargo.lock", _cargo),
        ("package-lock.json", _npm),
        ("automation/devcontainer/package-lock.json", _npm),
        ("uv.lock", _uv),
        ("automation/uv.lock", _uv),
    ):
        path = root / name
        if path.exists():
            result.update(parser(path))
    from .cargo_tools import DOCKERFILE, declarations

    declared = root / DOCKERFILE
    if declared.exists() or declared.is_symlink() or declared.parent.is_symlink():
        result.update(
            Package("cargo-tool", name, version, CRATES_SOURCE, ())
            for name, version in declarations(root).items()
        )
    source_path = root / INVENTORY
    if source_path.exists() or source_path.is_symlink():
        _document, sources = inventory(_read(source_path))
        result.update(
            Package(
                "git-source", "git", source.version, source.url, (source.sha256, source.filename)
            )
            for source in sources
        )
    if len(result) > MAX_PACKAGES:
        raise DependencyError("lock graph and standalone Cargo tools exceed package bound")
    return result


def _instant(value: object, now: datetime) -> datetime:
    if not isinstance(value, str) or not re.fullmatch(
        r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})", value
    ):
        raise DependencyError("publication/observation timestamp must be timezone-aware RFC 3339")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00")).astimezone(UTC)
    except ValueError as error:
        raise DependencyError("malformed publication/observation timestamp") from error
    if parsed > now:
        raise DependencyError("future publication/observation timestamp")
    return parsed


def _format(value: datetime) -> str:
    return value.astimezone(UTC).isoformat().replace("+00:00", "Z")


OCI_BLOB_SECONDS = 30
OCI_BLOB_REDIRECTS = 1
_OCI_BLOB = re.compile(r"https://ghcr\.io/v2/[a-z0-9_./-]+/blobs/(?P<digest>sha256:[a-f0-9]{64})\Z")


class OciBlobRedirect(DependencyError):
    """One unlogged Location, usable only by the immutable OCI blob reader."""

    def __init__(self, location: str) -> None:
        super().__init__("OCI blob redirect requires bounded content acquisition")
        self.location = location


@contextmanager
def _oci_blob_deadline() -> Generator[float]:
    # Match the canonical asset reader's main-thread wall-clock guard. A socket
    # timeout alone does not bound a peer which trickles bytes indefinitely.
    if not hasattr(signal, "SIGALRM") or threading.current_thread() is not threading.main_thread():
        raise DependencyError("OCI blob acquisition requires a main-thread wall-clock guard")
    if signal.getitimer(signal.ITIMER_REAL)[0]:
        raise DependencyError("OCI blob acquisition requires its own wall-clock budget")
    previous = signal.getsignal(signal.SIGALRM)

    def exhausted(_number: int, _frame: object) -> None:
        raise TimeoutError("OCI blob acquisition exceeded its aggregate deadline")

    signal.signal(signal.SIGALRM, exhausted)
    try:
        deadline = time.monotonic() + OCI_BLOB_SECONDS
        signal.setitimer(signal.ITIMER_REAL, OCI_BLOB_SECONDS)
        yield deadline
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)


def fetch_oci_blob(url: str, headers: Mapping[str, str], request: FetchRequest) -> bytes:
    """Follow only GHCR's observed single signed CDN hop for one exact blob.

    Every HTTP attempt uses the caller's request/byte accounting. The ordinary
    metadata transport still refuses redirects; this helper alone handles its
    typed immutable-blob redirect. Signed query strings are never diagnostic data.
    """
    match = _OCI_BLOB.fullmatch(url)
    if match is None or any(
        part in {"", ".", ".."} for part in urllib.parse.urlparse(url).path.split("/")[1:]
    ):
        raise DependencyError("OCI blob needs an exact reviewed credential-free HTTPS identity")
    digest = match["digest"]
    with _oci_blob_deadline() as deadline:
        current, forwarded = url, dict(headers)
        for hop in range(OCI_BLOB_REDIRECTS + 1):
            if time.monotonic() >= deadline:
                raise TimeoutError("OCI blob acquisition exceeded its aggregate deadline")
            try:
                data = request(current, forwarded)
            except OciBlobRedirect as redirect:
                target = urllib.parse.urlparse(redirect.location)
                if (
                    hop >= OCI_BLOB_REDIRECTS
                    or len(redirect.location) > 16384
                    or target.scheme != "https"
                    or target.netloc != "pkg-containers.githubusercontent.com"
                    or target.path != "/ghcr1/blobs/" + digest
                    or target.fragment
                ):
                    raise DependencyError(
                        "OCI blob redirect is outside the exact reviewed CDN identity"
                    ) from None
                current, forwarded = redirect.location, {}
                continue
            if time.monotonic() >= deadline:
                raise TimeoutError("OCI blob acquisition exceeded its aggregate deadline")
            if not isinstance(data, bytes) or len(data) > MAX_METADATA_BYTES:
                raise DependencyError("OCI blob content exceeds its byte bound")
            if "sha256:" + hashlib.sha256(data).hexdigest() != digest:
                raise DependencyError("OCI blob content differs from the requested SHA-256")
            return data
    raise DependencyError("OCI blob acquisition exceeded its redirect bound")


class _NoMetadataRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(
        self,
        req: urllib.request.Request,
        fp: IO[bytes],
        code: int,
        msg: str,
        headers: Message,
        newurl: str,
    ) -> urllib.request.Request | None:
        # Exact registry endpoints need no redirect. Refusing before forwarding
        # also keeps credentials and logical request accounting on one origin.
        fp.close()
        if code == 307 and _OCI_BLOB.fullmatch(req.full_url):
            raise OciBlobRedirect(newurl)
        raise DependencyError("registry metadata redirects are not allowed")


def fetch_metadata(url: str, headers: Mapping[str, str] | None = None) -> bytes:
    """Fetch one authoritative response with time/byte bounds and no redirects."""
    parsed = urllib.parse.urlparse(url)
    if parsed.scheme != "https" or parsed.username or parsed.password:
        raise DependencyError("registry metadata requires a credential-free HTTPS URL")
    request = urllib.request.Request(
        url, headers={"User-Agent": "BoxFerry-dependency-policy/1", **dict(headers or {})}
    )
    opener = urllib.request.build_opener(_NoMetadataRedirect())
    with opener.open(request, timeout=20) as response:
        data = response.read(MAX_METADATA_BYTES + 1)
    if len(data) > MAX_METADATA_BYTES:
        raise DependencyError("version metadata exceeds byte bound")
    return data


def _metadata(
    package: Package, fetch: Fetch, now: datetime
) -> tuple[str, list[datetime], tuple[str, ...]]:
    if package.ecosystem == "git-source":
        if package.name != "git" or len(package.integrity) != 2:
            raise DependencyError("unsupported Git source identity")
        source = source_record(
            {
                "tool": package.name,
                "provider": "kernel.org",
                "currentVersion": package.version,
                "filename": package.integrity[1],
                "url": package.source,
                "sha256": package.integrity[0],
            }
        )
        # No publication time is supplied by this metadata. Integrity must pass
        # before the existing trusted first-seen ledger can observe the identity.
        digest = match_checksums(source, fetch(CHECKSUMS))
        return CHECKSUMS, [], (digest,)
    if package.ecosystem not in {"cargo", "cargo-tool", "npm", "python"}:
        raise DependencyError("unsupported dependency metadata provider")
    name = urllib.parse.quote(package.name, safe="")
    version = urllib.parse.quote(package.version, safe="")
    if package.ecosystem in {"cargo", "cargo-tool"}:
        url = f"https://crates.io/api/v1/crates/{name}/{version}"
    elif package.ecosystem == "npm":
        url = f"https://registry.npmjs.org/{name}/{version}"
    else:
        url = f"https://pypi.org/pypi/{name}/{version}/json"
    raw = fetch(url)
    if not isinstance(raw, bytes) or len(raw) > MAX_METADATA_BYTES:
        raise DependencyError("version metadata exceeds byte bound or is not bytes")
    data = json.loads(raw)
    times: list[Any]
    integrity = package.integrity
    if package.ecosystem in {"cargo", "cargo-tool"}:
        record = data["version"]
        if record.get("crate") != package.name or record.get("num") != package.version:
            raise DependencyError("Cargo metadata identity mismatch")
        digest = _sha256(record.get("checksum"))
        if package.ecosystem == "cargo" and digest != package.integrity[0]:
            raise DependencyError("Cargo checksum differs from authoritative version metadata")
        integrity = (digest,)
        times = [record.get("created_at")]
        if package.ecosystem == "cargo-tool" and times[0] is None:
            raise DependencyError("standalone Cargo tool requires canonical publication timestamp")
    elif package.ecosystem == "npm":
        if data.get("name") != package.name or data.get("version") != package.version:
            raise DependencyError("npm metadata identity mismatch")
        if _sri(data.get("dist", {}).get("integrity")) != package.integrity[0]:
            raise DependencyError("npm integrity differs from authoritative version metadata")
        # The exact-version npm endpoint generally omits publication time. Never
        # fetch an unbounded packument merely to obtain its time map.
        times = [data.get("time", {}).get(package.version)]
    else:
        info = data["info"]
        normalized = re.sub(r"[-_.]+", "-", str(info.get("name", ""))).lower()
        if normalized != package.name or info.get("version") != package.version:
            raise DependencyError("PyPI metadata identity mismatch")
        artifacts = {a.get("digests", {}).get("sha256"): a for a in data["urls"]}
        if not set(package.integrity) <= artifacts.keys():
            raise DependencyError("uv artifact digest differs from authoritative PyPI metadata")
        times = [artifacts[digest].get("upload_time_iso_8601") for digest in package.integrity]
    present = [_instant(value, now) for value in times if value is not None]
    # Any absent artifact timestamp requires independent observation quarantine.
    return url, present if len(present) == len(times) else [], integrity


def _manifests(root: Path) -> dict[str, dict[str, Any]]:
    paths = [root / "Cargo.toml", *(root / "crates").glob("**/Cargo.toml")]
    paths = [p for p in paths if p.exists()]
    if len(paths) > 128:
        raise DependencyError("Cargo manifest count exceeds bound")
    return {p.relative_to(root).as_posix(): _toml(p) for p in paths}


def _manifest_updates(before: object, after: object, path: tuple[str, ...] = ()) -> set[str]:
    if before == after:
        return set()
    if isinstance(before, dict) and isinstance(after, dict) and before.keys() == after.keys():
        result: set[str] = set()
        for key in before:
            if before[key] != after[key]:
                result.update(_manifest_updates(before[key], after[key], (*path, key)))
        return result
    dependency_sections = {"dependencies", "dev-dependencies", "build-dependencies"}
    if len(path) >= 2 and path[-2] in dependency_sections:
        name = path[-1]
    elif len(path) >= 3 and path[-3] in dependency_sections and path[-1] == "version":
        name = path[-2]
    else:
        raise DependencyError("first-party scope includes a non-dependency-version manifest change")
    if name not in FIRST_PARTY or not isinstance(before, str) or not isinstance(after, str):
        raise DependencyError("first-party scope includes an unrelated dependency change")
    return {name}


def _cargo_closure(
    root: Path, names: set[str]
) -> tuple[set[tuple[str, str]], dict[tuple[str, str], dict[str, Any]]]:
    records = _toml(root / "Cargo.lock").get("package", [])
    by_identity = {(item["name"], item["version"]): item for item in records}
    if len(by_identity) != len(records):
        raise DependencyError("duplicate Cargo package identity")
    by_name: dict[str, list[tuple[str, str]]] = {}
    for identity in by_identity:
        by_name.setdefault(identity[0], []).append(identity)
    queue = [identity for identity in by_identity if identity[0] in names]
    visited: set[tuple[str, str]] = set()
    while queue:
        identity = queue.pop()
        if identity in visited:
            continue
        visited.add(identity)
        for dependency in by_identity[identity].get("dependencies", []):
            parts = dependency.split(" ")
            candidates = by_name.get(parts[0], [])
            if len(parts) > 1:
                candidates = [item for item in candidates if item[1] == parts[1]]
            if len(candidates) != 1:
                raise DependencyError("ambiguous or missing Cargo dependency edge")
            queue.append(candidates[0])
    return visited, by_identity


def _first_party_scope(
    base: Path,
    head: Path,
    before: set[Package],
    after: set[Package],
    context: DependencyContext | None,
) -> tuple[bool, str]:
    if {p for p in before if p.ecosystem == "cargo-tool"} != {
        p for p in after if p.ecosystem == "cargo-tool"
    }:
        return False, "standalone Cargo tool changes retain their own third-party age hold"
    if {p for p in before if p.ecosystem == "git-source"} != {
        p for p in after if p.ecosystem == "git-source"
    }:
        return False, "Git source changes retain their own third-party age hold"
    if context is None:
        return False, "no trusted isolated pull-request context"
    if context.actor_login != "renovate[bot]" or context.actor_type != "Bot":
        return False, "actor is not the trusted Renovate application"
    if context.repository not in REPOSITORIES or not all(
        re.fullmatch(r"[a-f0-9]{40}", ref) for ref in (context.base_revision, context.head_revision)
    ):
        return False, "context lacks an allowed repository and immutable revisions"
    old, new = _manifests(base), _manifests(head)
    if old.keys() != new.keys():
        return False, "manifest set changed"
    changed = {name for name in old if old[name] != new[name]}
    if not changed or set(context.changed_paths) != changed | {"Cargo.lock"}:
        return (
            False,
            "update is not isolated to first-party Cargo manifests and lockfile",
        )
    try:
        names: set[str] = set()
        for path in changed:
            names.update(_manifest_updates(old[path], new[path]))
        for name in names:
            old_versions = [p.version for p in before if p.ecosystem == "cargo" and p.name == name]
            new_versions = [p.version for p in after if p.ecosystem == "cargo" and p.name == name]
            if len(old_versions) != 1 or len(new_versions) != 1 or old_versions == new_versions:
                return (
                    False,
                    "first-party crates.io version transition is absent or ambiguous",
                )
        old_closure, old_records = _cargo_closure(base, names)
        new_closure, new_records = _cargo_closure(head, names)
        for identity in old_records.keys() | new_records.keys():
            if old_records.get(identity) == new_records.get(identity):
                continue
            if identity in old_records and identity not in old_closure:
                return (
                    False,
                    "lock maintenance is outside the old first-party closure",
                )
            if identity in new_records and identity not in new_closure:
                return (
                    False,
                    "lock maintenance is outside the new first-party closure",
                )
        return (
            True,
            "isolated first-party crates.io update and its dependency closure",
        )
    except (
        DependencyError,
        ValueError,
        KeyError,
        TypeError,
        AttributeError,
        IndexError,
        OSError,
    ) as error:
        return (
            False,
            f"first-party isolation proof unavailable: {type(error).__name__}: {error}",
        )


def check_dependencies(
    base: Path,
    head: Path,
    *,
    now: datetime | None = None,
    state: dict[str, Any] | None = None,
    context: DependencyContext | None = None,
    fetch: Fetch | None = None,
    fetch_request: FetchRequest | None = None,
) -> dict[str, Any]:
    """Check all introduced lock identities and return JSON-compatible evidence.

    ``waiting`` blocks merging and has an automatic retry deadline. ``failure``
    indicates unavailable/invalid integrity or metadata; transient acquisition
    failures also have retry deadlines. Callers must persist ``state`` from a
    trusted run and arrange rechecks; rerunning without state resets quarantine.
    """
    instant = now or datetime.now(UTC)
    if instant.tzinfo is None:
        raise DependencyError("now must be timezone-aware")
    instant = instant.astimezone(UTC)
    ledger: dict[str, str] = {}
    report: dict[str, Any] = {
        "schema_version": SCHEMA_VERSION,
        "status": "pass",
        "results": [],
        "retry_at": None,
        "retry_eligible": False,
        "first_party_exempt": False,
        "first_party_reason": "not evaluated",
        "state": {"schema_version": 1, "first_seen": ledger},
    }
    try:
        if not base.is_dir() or not head.is_dir():
            raise DependencyError("dependency snapshot root is missing or is not a directory")
        if state is not None:
            if (
                set(state) != {"schema_version", "first_seen"}
                or state["schema_version"] != 1
                or not isinstance(state["first_seen"], dict)
            ):
                raise DependencyError("invalid trusted first-seen state schema")
            if len(state["first_seen"]) > MAX_STATE_ENTRIES:
                raise DependencyError("first-seen state exceeds entry bound")
            for key, value in state["first_seen"].items():
                _sha256(key)
                _instant(value, instant)
                ledger[key] = value
        from .dependency_snapshots import validate_pair

        validate_pair(base, head, context.changed_paths if context is not None else ())
        before, after = _packages(base), _packages(head)
        introduced = sorted(after - before)
        if len(introduced) > MAX_INTRODUCED:
            raise DependencyError("introduced versions exceed request bound")
        exempt, reason = _first_party_scope(base, head, before, after, context)
        report["first_party_exempt"], report["first_party_reason"] = exempt, reason
    except (
        DependencyError,
        ValueError,
        KeyError,
        TypeError,
        AttributeError,
        IndexError,
        OSError,
    ) as error:
        report.update(status="failure", error=str(error))
        return report
    consumed = 0
    requests = 0
    acquire = fetch or fetch_metadata

    def account(value: bytes) -> bytes:
        nonlocal consumed
        if not isinstance(value, bytes):
            raise DependencyError("metadata fetch did not return bytes")
        consumed += len(value)
        if len(value) > MAX_METADATA_BYTES:
            raise DependencyError("metadata exceeds its byte bound")
        if consumed > MAX_TOTAL_METADATA_BYTES:
            raise DependencyError("shared metadata byte budget exhausted")
        return value

    def admit_request() -> None:
        nonlocal requests
        if requests >= MAX_METADATA_REQUESTS:
            raise urllib.error.URLError("shared dependency metadata request budget exhausted")
        requests += 1

    def bounded(url: str) -> bytes:
        admit_request()
        return account(acquire(url))

    def bounded_request(url: str, headers: Mapping[str, str]) -> bytes:
        admit_request()
        request = fetch_request or fetch_metadata
        return account(request(url, headers))

    for package in introduced:
        result: dict[str, Any] = {
            "ecosystem": package.ecosystem,
            "name": package.name,
            "version": package.version,
            "artifact_key": package.key,
            "status": "pass",
            "retry_at": None,
            "retry_eligible": False,
            "age_evidence": None,
        }
        try:
            url, timestamps, integrity = _metadata(package, bounded, instant)
            result["metadata_url"] = url
            result["integrity"] = list(integrity)
            if exempt:
                result.update(age_evidence="first-party-update-exemption", reason=reason)
            else:
                if timestamps:
                    observed = max(timestamps)
                    result["age_evidence"] = "registry-publication"
                else:
                    if package.key not in ledger:
                        if len(ledger) >= MAX_STATE_ENTRIES:
                            raise DependencyError("first-seen state capacity exhausted")
                        ledger[package.key] = _format(instant)
                    observed = _instant(ledger[package.key], instant)
                    result["age_evidence"] = "artifact-first-seen"
                result["evidence_at"] = _format(observed)
                eligible = observed + HOLD
                if instant < eligible:
                    result.update(
                        status="waiting",
                        retry_at=_format(eligible),
                        retry_eligible=True,
                        reason="72-hour publication or artifact observation hold",
                    )
        except urllib.error.HTTPError as error:
            result.update(status="failure", reason=f"registry HTTP status {error.code}")
            if error.code == 429 or error.code >= 500:
                result.update(retry_at=_format(instant + timedelta(hours=1)), retry_eligible=True)
        except (urllib.error.URLError, TimeoutError, ConnectionError) as error:
            result.update(
                status="failure",
                reason=f"metadata acquisition: {type(error).__name__}",
                retry_at=_format(instant + timedelta(hours=1)),
                retry_eligible=True,
            )
        except (
            DependencyError,
            ValueError,
            KeyError,
            TypeError,
            AttributeError,
            IndexError,
            OSError,
        ) as error:
            result.update(status="failure", reason=str(error))
        report["results"].append(result)
    from .operational import check_operational

    operational = check_operational(
        base,
        head,
        now=instant,
        ledger=ledger,
        context=context,
        request=bounded_request,
        remaining=MAX_INTRODUCED - len(introduced),
        state_limit=MAX_STATE_ENTRIES,
    )
    report["results"].extend(operational["results"])
    report["operational_coverage_gaps"] = operational["coverage_gaps"]
    statuses = {item["status"] for item in report["results"]}
    report["status"] = (
        "failure" if "failure" in statuses else "waiting" if "waiting" in statuses else "pass"
    )
    retries = [item["retry_at"] for item in report["results"] if item["retry_eligible"]]
    if retries:
        report.update(retry_at=min(retries), retry_eligible=True)
    return report
