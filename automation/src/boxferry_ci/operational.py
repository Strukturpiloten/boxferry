"""Trusted, finite readers for missing-date operational pins.

No candidate configuration or executable is evaluated. OCI observation proves
only that the exact manifest existed at observation time, not publication time.
Runner observation concerns a selector; hosted runner images remain mutable.
"""

from __future__ import annotations

import gzip
import hashlib
import io
import json
import re
import tarfile
import urllib.error
import urllib.parse
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass, field, replace
from datetime import UTC, datetime, timedelta
from pathlib import Path, PurePosixPath
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from .dependencies import DependencyContext

Request = Callable[[str, Mapping[str, str]], bytes]
MAX_FILE_BYTES = 2 * 1024 * 1024
MAX_SNAPSHOT_BYTES = 16 * 1024 * 1024
MAX_FILES = 256
MAX_PINS = 4096
MAX_TREE_ENTRIES = 20000
MAX_CACHE_BYTES = MAX_SNAPSHOT_BYTES
HOLD = timedelta(hours=72)
FEATURE = ".devcontainer/devcontainer.json"
FEATURE_LOCK = ".devcontainer/devcontainer-lock.json"
DOCKERFILE = ".devcontainer/Dockerfile"
PROBE = "scripts/podman-live-conformance.sh"
IMAGE_JSON = frozenset({"automation/renovate-runtime.json", "scripts/renovate-tool.json"})
CATALOGUES = frozenset(
    {
        f"fixtures/conformance/{name}-application/images.tsv"
        for name in ("forgejo", "immich", "nextcloud", "observability", "paperless-ngx", "supabase")
    }
    | {"fixtures/conformance/podman-live/matrix.tsv"}
)
EXCLUDED_PATHS = frozenset({"scripts/native-runtime-pins.sh", "scripts/kind-vm-image.json"})
FIXED_PATHS = IMAGE_JSON | CATALOGUES | {FEATURE, FEATURE_LOCK, DOCKERFILE, PROBE}
WORKFLOW = re.compile(r"\.github/workflows/[A-Za-z0-9_.-]+\.ya?ml\Z")
SELECTOR = re.compile(r"(?:ubuntu|macos|windows)-[0-9]+(?:\.[0-9]+)?(?:-[a-z0-9]+)*\Z")
REF = re.compile(
    r"(?P<registry>[a-z0-9.-]+)/(?P<name>[a-z0-9_./-]+)"
    r"(?::(?P<version>[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}))?"
    r"@(?P<digest>sha256:[a-f0-9]{64})\Z"
)
FEATURE_REF = re.compile(
    r"(?P<registry>[a-z0-9.-]+)/(?P<name>[a-z0-9_./-]+):"
    r"(?P<version>[A-Za-z0-9_][A-Za-z0-9_.-]{0,127})\Z"
)
MEDIA_TYPES = frozenset(
    {
        "application/vnd.oci.image.manifest.v1+json",
        "application/vnd.oci.image.index.v1+json",
        "application/vnd.docker.distribution.manifest.v2+json",
        "application/vnd.docker.distribution.manifest.list.v2+json",
    }
)


class OperationalError(ValueError):
    """Malformed or unsupported evidence in an owned operational definition."""


@dataclass(frozen=True)
class Pin:
    kind: str
    source: str
    name: str
    digest: str
    version: str
    path: str
    line: int

    @property
    def key(self) -> str:
        # Tags, paths and commits are deliberately absent from OCI identity.
        identity = ["boxferry-operational-v1", self.kind, self.source, self.name, self.digest]
        return hashlib.sha256(json.dumps(identity, separators=(",", ":")).encode()).hexdigest()

    @property
    def covered(self) -> bool:
        return (
            self.kind == "runner"
            or self.source == "ghcr.io"
            or (
                (self.source, self.name)
                in {
                    ("quay.io", "libpod/alpine"),
                    ("mcr.microsoft.com", "devcontainers/rust"),
                    ("codeberg.org", "forgejo/forgejo"),
                }
            )
        )

    @property
    def native_dated(self) -> bool:
        return self.source in {"docker.io", "index.docker.io", "registry-1.docker.io"}


def snapshot_paths(paths: Sequence[str]) -> set[str]:
    """Select inert inputs from trusted fixed ownership, including feature counterparts.

    The service already includes both filename and previous_filename for renames.
    Unknown paths cannot gain timestamp-optional treatment through candidate config.
    """
    result = {path for path in paths if path in FIXED_PATHS or WORKFLOW.fullmatch(path)}
    if result & {FEATURE, FEATURE_LOCK}:
        result.update({FEATURE, FEATURE_LOCK})
    if len(result) > MAX_FILES:
        raise OperationalError("operational snapshot file count exceeds its bound")
    return result


def inventory_blobs(document: Mapping[str, Any]) -> dict[str, str]:
    """Select regular owned blobs from one complete immutable Git tree response."""
    entries = document.get("tree")
    if document.get("truncated") is not False or not isinstance(entries, list):
        raise OperationalError("operational Git tree is incomplete or truncated")
    if len(entries) > MAX_TREE_ENTRIES:
        raise OperationalError("operational Git tree exceeds its entry bound")
    result = {}
    for entry in entries:
        if not isinstance(entry, dict) or not isinstance(entry.get("path"), str):
            raise OperationalError("operational Git tree entry is malformed")
        path = entry["path"]
        if path not in FIXED_PATHS and WORKFLOW.fullmatch(path) is None:
            continue
        if (
            entry.get("type") != "blob"
            or entry.get("mode") not in {"100644", "100755"}
            or not isinstance(entry.get("sha"), str)
            or re.fullmatch(r"[a-f0-9]{40}", entry["sha"]) is None
            or path in result
        ):
            raise OperationalError("owned operational input is not one regular immutable blob")
        result[path] = entry["sha"]
    if len(result) > MAX_FILES:
        raise OperationalError("operational inventory exceeds its file bound")
    return result


@dataclass
class SnapshotCache:
    """Run-local content cache only; never stores candidate/ref/actor API responses."""

    contents: dict[str, bytes] = field(default_factory=dict)
    size: int = 0

    def get(self, sha: str) -> bytes | None:
        return self.contents.get(sha)

    def put(self, sha: str, content: bytes) -> None:
        material = b"blob " + str(len(content)).encode() + b"\0" + content
        # Git's authoritative blob identity uses SHA-1; OCI age keys remain SHA-256.
        actual = hashlib.sha1(material, usedforsecurity=False).hexdigest()
        if actual != sha:
            raise OperationalError("snapshot bytes differ from their immutable Git blob identity")
        if sha in self.contents:
            if self.contents[sha] != content:
                raise OperationalError("one Git blob identity returned inconsistent content")
            return
        if self.size + len(content) > MAX_CACHE_BYTES:
            raise OperationalError("operational snapshot cache exceeds its memory bound")
        self.contents[sha] = content
        self.size += len(content)


def _objects(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise OperationalError("duplicate JSON object key in operational evidence")
        result[key] = value
    return result


def _json(data: str | bytes) -> dict[str, Any]:
    value = json.loads(data, object_pairs_hook=_objects)
    if not isinstance(value, dict):
        raise OperationalError("operational JSON document must be an object")
    return value


def _read(root: Path, path: str) -> str | None:
    target = root / path
    if target.is_symlink():
        raise OperationalError("operational evidence must be a regular file")
    if not target.exists():
        return None
    if not target.is_file():
        raise OperationalError("operational evidence must be a regular file")
    with target.open("rb") as stream:
        value = stream.read(MAX_FILE_BYTES + 1)
    if len(value) > MAX_FILE_BYTES:
        raise OperationalError("operational file exceeds its byte bound")
    return value.decode("utf-8")


def _ref(value: str, path: str, line: int) -> Pin:
    match = REF.fullmatch(value)
    if match is None or any(part in {"", ".", ".."} for part in match["name"].split("/")):
        raise OperationalError("image needs a canonical repository and SHA-256 manifest")
    return Pin(
        "oci", match["registry"], match["name"], match["digest"], match["version"] or "", path, line
    )


def _dockerfile(text: str, path: str) -> list[Pin]:
    lines = text.splitlines()
    result = []
    for index, line in enumerate(lines):
        if not re.match(r"\s*(?i:FROM)\s", line):
            continue
        marker = (
            re.fullmatch(r"# renovate: datasource=docker depName=([^\s]+)", lines[index - 2])
            if index >= 2
            else None
        )
        release = re.fullmatch(r"# release: ([^\s]+)", lines[index - 1]) if index else None
        instruction = re.fullmatch(r"FROM ([^\s]+)(?: AS [A-Za-z0-9_.-]+)?", line)
        if marker is None or release is None or instruction is None:
            raise OperationalError("owned FROM needs its canonical source/release annotation")
        pin = _ref(instruction[1], path, index + 1)
        if marker[1] != f"{pin.source}/{pin.name}" or (pin.version and pin.version != release[1]):
            raise OperationalError("Dockerfile source, release annotation and FROM disagree")
        result.append(replace(pin, version=release[1]))
    return result


def _catalogue(text: str, path: str) -> list[Pin]:
    result = []
    names = set()
    for number, line in enumerate(text.splitlines(), 1):
        if not line.strip() or line.startswith("#"):
            continue
        columns = line.split("\t")
        if len(columns) < 2 or not columns[0] or columns[0] in names:
            raise OperationalError("owned image catalogue row is malformed or duplicated")
        names.add(columns[0])
        result.append(_ref(columns[1], path, number))
    return result


def _probe(text: str, path: str) -> list[Pin]:
    lines = text.splitlines()
    found = []
    for index, line in enumerate(lines):
        if not re.match(r"workload_image\s*=", line):
            continue
        marker = "# renovate: datasource=docker depName=quay.io/libpod/alpine"
        value = re.fullmatch(r'workload_image="([^"$`]+)"', line)
        if index == 0 or lines[index - 1] != marker or value is None:
            raise OperationalError("owned workload probe must be an annotated inert image literal")
        pin = _ref(value[1], path, index + 1)
        if pin.source != "quay.io" or pin.name != "libpod/alpine":
            raise OperationalError("workload probe source differs from its owned identity")
        found.append(pin)
    if len(found) != 1:
        raise OperationalError("owned workload probe must have exactly one image declaration")
    return found


def _image_json(text: str, path: str) -> list[Pin]:
    document = _json(text)
    if set(document) != {"image"} or not isinstance(document["image"], str):
        raise OperationalError("owned runtime JSON must contain only its immutable image reference")
    return [_ref(document["image"], path, 1)]


def _feature_source(source: str) -> tuple[str, str]:
    """Return a reviewed OCI identity and a numeric selector or exact digest."""
    match = FEATURE_REF.fullmatch(source)
    if match is not None:
        identity = f"{match['registry']}/{match['name']}"
        selector = match["version"]
        if re.fullmatch(r"(?:0|[1-9][0-9]*)(?:\.(?:0|[1-9][0-9]*)){0,2}", selector) is None:
            raise OperationalError("feature selector needs a numeric major, minor or exact version")
    else:
        pin = _ref(source, FEATURE_LOCK, 1)
        if pin.version:
            raise OperationalError("feature source cannot combine a tag and digest")
        identity, selector = f"{pin.source}/{pin.name}", pin.digest
    if not identity.startswith("ghcr.io/") or any(
        part in {"", ".", ".."} for part in identity.split("/")
    ):
        raise OperationalError("feature source is outside the reviewed public OCI registry")
    return identity, selector


def _features(root: Path) -> tuple[dict[str, object], dict[str, Pin], dict[str, str]]:
    declaration = _read(root, FEATURE)
    lock = _read(root, FEATURE_LOCK)
    declared = _json(declaration).get("features", {}) if declaration is not None else {}
    lock_document = _json(lock) if lock is not None else {"features": {}}
    if set(lock_document) != {"features"}:
        raise OperationalError("feature lock document has unsupported fields")
    locked = lock_document["features"]
    if not isinstance(declared, dict) or not isinstance(locked, dict):
        raise OperationalError("Dev Container features and feature lock must be objects")
    if len(declared.keys() | locked.keys()) > MAX_FILES:
        raise OperationalError("feature graph exceeds its node bound")
    signatures: dict[str, object] = {}
    valid: dict[str, Pin] = {}
    errors: dict[str, str] = {}
    edges: dict[str, list[str]] = {}
    for source in sorted(declared.keys() | locked.keys()):
        entry = locked.get(source)
        signatures[source] = (source in declared, declared.get(source), entry)
        try:
            identity, selector = _feature_source(source)
            if source in declared and re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", selector) is None:
                raise OperationalError("feature declaration needs exact canonical version")
            if not isinstance(entry, dict):
                raise OperationalError(
                    "changed feature requires its committed immutable feature lock"
                )
            if set(entry) not in (
                {"version", "resolved", "integrity"},
                {"version", "resolved", "integrity", "dependsOn"},
            ):
                raise OperationalError("feature lock fields differ from official graph grammar")
            version = entry["version"]
            if (
                not isinstance(version, str)
                or re.fullmatch(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)", version)
                is None
            ):
                raise OperationalError("feature lock needs an exact semantic version")
            pin = _ref(entry["resolved"], FEATURE_LOCK, 1)
            if (
                pin.version
                or identity != f"{pin.source}/{pin.name}"
                or entry["integrity"] != pin.digest
                or (selector.startswith("sha256:") and selector != pin.digest)
                or (
                    not selector.startswith("sha256:")
                    and version.split(".")[: len(selector.split("."))] != selector.split(".")
                )
            ):
                raise OperationalError("feature version, source and lock integrity disagree")
            dependencies = entry.get("dependsOn", [])
            if (
                not isinstance(dependencies, list)
                or any(not isinstance(item, str) for item in dependencies)
                or len(dependencies) != len(set(dependencies))
                or len(dependencies) > MAX_FILES
            ):
                raise OperationalError(
                    "feature lock dependencies must be bounded unique source IDs"
                )
            edges[source] = dependencies
            valid[source] = replace(pin, version=version)
        except (OperationalError, TypeError, KeyError) as error:
            errors[source] = str(error)
    visiting: set[str] = set()
    reached: set[str] = set()

    def visit(source: str) -> None:
        if source in visiting:
            raise OperationalError("feature dependency graph contains a cycle")
        if source in reached:
            return
        if source not in valid:
            raise OperationalError(
                "feature dependency graph node is missing or unsupported: official selector "
                "alias collapse needs reviewed identity/options resolution; use explicit "
                "distinct dependency IDs until this shape is supported"
            )
        visiting.add(source)
        for dependency in edges[source]:
            visit(dependency)
        visiting.remove(source)
        reached.add(source)

    try:
        for source in declared:
            visit(source)
        if reached != locked.keys():
            raise OperationalError("feature lock contains an unreachable orphan node")
    except OperationalError as error:
        for source in signatures:
            errors.setdefault(source, str(error))
    return signatures, valid, errors


def _registry_request(pin: Pin, request: Request) -> Request:
    """Return a bounded caller-owned request with only anonymous, exact-origin auth."""
    from .dependencies import fetch_oci_blob

    token: str | None = None

    def acquire(url: str, headers: Mapping[str, str]) -> bytes:
        if "/blobs/sha256:" in url:
            return fetch_oci_blob(url, headers, request)
        return request(url, headers)

    def get(url: str, headers: Mapping[str, str]) -> bytes:
        nonlocal token
        origin = f"https://{pin.source}/v2/{pin.name}/"
        if not url.startswith(origin):
            raise OperationalError("feature content request escaped its immutable origin")
        try:
            return acquire(
                url, {**headers, **({"Authorization": "Bearer " + token} if token else {})}
            )
        except urllib.error.HTTPError as error:
            if error.code != 401 or token is not None:
                raise
            error.close()
            query = urllib.parse.urlencode(
                {"service": "ghcr.io", "scope": f"repository:{pin.name}:pull"}
            )
            response = _json(request(f"https://ghcr.io/token?{query}", {}))
            candidate = response.get("token", response.get("access_token"))
            if (
                not isinstance(candidate, str)
                or not candidate
                or len(candidate) > 16384
                or re.search(r"[\r\n]", candidate)
            ):
                raise OperationalError(
                    "anonymous feature authentication returned invalid evidence"
                ) from None
            token = candidate
            return acquire(url, {**headers, "Authorization": "Bearer " + token})

    return get


def _feature_metadata(pin: Pin, request: Request, now: datetime) -> dict[str, Any]:
    """Verify the manifest, every content descriptor, and inert packaged Feature metadata.

    No archive member is extracted and no Feature installer is executed. The caller's
    shared byte/request budget applies to config and archive requests too.
    """
    get = _registry_request(pin, request)
    base = f"https://{pin.source}/v2/{pin.name}"
    body = get(
        f"{base}/manifests/{pin.digest}", {"Accept": "application/vnd.oci.image.manifest.v1+json"}
    )
    if "sha256:" + hashlib.sha256(body).hexdigest() != pin.digest:
        raise OperationalError("feature manifest bytes differ from pinned SHA-256")
    document = _json(body)
    if (
        type(document.get("schemaVersion")) is not int
        or document["schemaVersion"] != 2
        or document.get("mediaType") != "application/vnd.oci.image.manifest.v1+json"
    ):
        raise OperationalError("feature manifest needs the supported OCI schema")
    config, layers = document.get("config"), document.get("layers")
    if not isinstance(config, dict) or not isinstance(layers, list) or len(layers) != 1:
        raise OperationalError("feature manifest needs one config and one archive")
    if (
        config.get("mediaType") != "application/vnd.devcontainers"
        or not isinstance(layers[0], dict)
        or layers[0].get("mediaType") != "application/vnd.devcontainers.layer.v1+tar"
    ):
        raise OperationalError("feature content has an unsupported media type")
    contents: list[bytes] = []
    for descriptor in (config, *layers):
        digest, size = descriptor.get("digest"), descriptor.get("size")
        if (
            not isinstance(digest, str)
            or re.fullmatch(r"sha256:[a-f0-9]{64}", digest) is None
            or type(size) is not int
            or not 0 <= size <= MAX_FILE_BYTES
        ):
            raise OperationalError("feature content descriptor exceeds its identity/size bound")
        content = get(f"{base}/blobs/{digest}", {})
        if len(content) != size or "sha256:" + hashlib.sha256(content).hexdigest() != digest:
            raise OperationalError("feature content bytes differ from descriptor size or SHA-256")
        contents.append(content)
    if _json(contents[0]) != {}:
        raise OperationalError("feature OCI configuration has unsupported fields")
    annotations = document.get("annotations")
    if annotations is None:
        annotations = {}
    if not isinstance(annotations, dict):
        raise OperationalError("feature manifest annotations are malformed")
    created = annotations.get("org.opencontainers.image.created")
    if created is not None:
        if not isinstance(created, str):
            raise OperationalError("feature manifest publication time is malformed")
        _instant(created, now)
    annotated = annotations.get("dev.containers.metadata")
    if annotated is not None and not isinstance(annotated, str):
        raise OperationalError("feature metadata annotation is malformed")
    metadata = _json(annotated) if annotated is not None else None
    packaged: dict[str, Any] | None = None
    total, names = 0, set()
    try:
        archive_bytes = contents[1]
        if archive_bytes.startswith(b"\x1f\x8b"):
            with gzip.GzipFile(fileobj=io.BytesIO(archive_bytes)) as compressed:
                archive_bytes = compressed.read(MAX_FILE_BYTES + 1)
        if len(archive_bytes) > MAX_FILE_BYTES:
            raise OperationalError("feature archive exceeds its decompression byte bound")
        with tarfile.open(fileobj=io.BytesIO(archive_bytes), mode="r:") as archive:
            for index, member in enumerate(archive):
                path = PurePosixPath(member.name)
                if (
                    index >= MAX_FILES
                    or path.is_absolute()
                    or ".." in path.parts
                    or member.name in names
                    or not (member.isdir() or member.isfile())
                ):
                    raise OperationalError(
                        "feature archive contains unsupported or duplicate members"
                    )
                names.add(member.name)
                total += member.size
                if total > MAX_FILE_BYTES:
                    raise OperationalError("feature archive exceeds its unpacked byte bound")
                if path.as_posix().removeprefix("./") == "devcontainer-feature.json":
                    stream = archive.extractfile(member)
                    if stream is None:
                        raise OperationalError("feature archive metadata is not regular content")
                    packaged = _json(stream.read(MAX_FILE_BYTES + 1))
    except tarfile.TarError as error:
        raise OperationalError("feature archive is malformed") from error
    if metadata is None:
        metadata = packaged
    if (
        metadata is None
        or packaged != metadata
        or metadata.get("version") != pin.version
        or metadata.get("id") != pin.name.rsplit("/", 1)[1]
    ):
        raise OperationalError("feature archive, manifest metadata and locked version disagree")
    return metadata


def validate_feature_graph(
    root: Path, request: Request, *, now: datetime | None = None
) -> dict[str, Pin]:
    """Validate a complete official lock graph against digest-verified OCI content.

    This proves content and dependency edges, not eligibility by age. Required
    ``check_operational`` independently holds every introduced digest for 72 hours
    in its trusted exact-digest ledger, including transitive-only changes.
    """
    _signatures, valid, errors = _features(root)
    if errors:
        raise OperationalError(next(iter(errors.values())))
    document = _json(_read(root, FEATURE_LOCK) or '{"features":{}}')["features"]
    requests, consumed = 0, 0

    def bounded(url: str, headers: Mapping[str, str]) -> bytes:
        nonlocal requests, consumed
        requests += 1
        if requests > MAX_FILES:
            raise OperationalError("feature graph exceeded its aggregate metadata request bound")
        body = request(url, headers)
        if not isinstance(body, bytes) or len(body) > MAX_FILE_BYTES:
            raise OperationalError("feature graph metadata exceeded its per-response byte bound")
        consumed += len(body)
        if consumed > MAX_SNAPSHOT_BYTES:
            raise OperationalError("feature graph metadata exceeded its aggregate byte bound")
        return body

    for source, pin in valid.items():
        metadata = _feature_metadata(pin, bounded, now or datetime.now(UTC))
        dependencies = metadata.get("dependsOn", {})
        if not isinstance(dependencies, dict) or any(
            not isinstance(value, dict) for value in dependencies.values()
        ):
            raise OperationalError("feature metadata dependencies have unsupported options")
        if set(dependencies) != set(document[source].get("dependsOn", [])):
            raise OperationalError(
                "feature lock dependencies disagree with verified Feature metadata"
            )
    return valid


def _yaml_content(line: str, quote: str | None) -> tuple[str, str | None]:
    """Strip comments while recognizing quoted continuation lines, not YAML values."""
    index = 0
    while index < len(line):
        char = line[index]
        if quote == '"' and char == "\\":
            index += 2
            continue
        if quote == "'" and char == "'" and index + 1 < len(line) and line[index + 1] == "'":
            index += 2
            continue
        if quote is None and char == "#":
            return line[:index], None
        if char in {"'", '"'}:
            if quote is None and (index == 0 or line[index - 1] in " \t[{:, "):
                quote = char
            elif quote == char:
                quote = None
        index += 1
    return line, quote


def _runners(text: str, path: str) -> list[Pin]:
    result = []
    stack: list[tuple[int, str]] = []
    block: int | None = None
    quote: str | None = None
    list_indent: int | None = None
    previous = ""
    for number, raw in enumerate(text.splitlines(), 1):
        indent = len(raw) - len(raw.lstrip(" "))
        if block is not None and (not raw.strip() or indent > block):
            continue
        block = None
        continued = quote is not None
        content, quote = _yaml_content(raw, quote)
        if continued:
            continue
        if not content.strip():
            previous = raw.strip()
            continue
        if "\t" in raw[: len(raw) - len(raw.lstrip())]:
            raise OperationalError("owned runner grammar requires space indentation")
        if list_indent is not None:
            if indent > list_indent:
                item = re.fullmatch(r"\s*-\s+([^\s]+)\s*", content)
                if item is None or SELECTOR.fullmatch(item[1]) is None:
                    raise OperationalError("marked runner matrix requires literal hosted selectors")
                result.append(Pin("runner", "github-hosted", item[1], "", item[1], path, number))
                continue
            list_indent = None
        match = re.fullmatch(r" *(?P<sequence>- +)?(?P<key>[A-Za-z0-9_.-]+):(?P<value>.*)", content)
        if match is None:
            previous = raw.strip()
            continue
        level = indent + (2 if match["sequence"] else 0)
        while stack and stack[-1][0] >= level:
            stack.pop()
        ancestors = tuple(key for _, key in stack)
        key, value = match["key"], match["value"].strip()
        if key == "jobs" and value.startswith("{"):
            raise OperationalError("owned runner grammar requires block job mappings")
        if (
            key == "include"
            and ancestors[-2:] == ("strategy", "matrix")
            and value.startswith(("[", "{"))
        ):
            raise OperationalError("owned runner grammar requires block matrix entries")
        if value.startswith(("|", ">")):
            block = level
        direct = len(ancestors) == 2 and ancestors[0] == "jobs"
        included = (
            len(ancestors) >= 5
            and ancestors[0] == "jobs"
            and ancestors[-3:] == ("strategy", "matrix", "include")
        )
        if key == "runs-on" and (direct or included):
            if SELECTOR.fullmatch(value):
                result.append(Pin("runner", "github-hosted", value, "", value, path, number))
            elif (
                value in {"${{ matrix.runner }}", "${{ matrix.runs-on }}"} or "self-hosted" in value
            ):
                pass
            elif value.startswith(("${{", "[", "*", "&", "!", "'", '"')):
                fingerprint = hashlib.sha256(value.encode()).hexdigest()
                result.append(
                    Pin(
                        "unsupported-runner",
                        "github-hosted",
                        "nonliteral selector",
                        fingerprint,
                        "",
                        path,
                        number,
                    )
                )
        if key == "runner" and ancestors[-2:] == ("strategy", "matrix"):
            if previous != "# renovate: datasource=github-runners" or value:
                raise OperationalError("runner matrix needs its marker and literal block list")
            list_indent = level
        stack.append((level, key))
        previous = raw.strip()
    return result


def _inventory(root: Path) -> dict[str, Pin]:
    paths = set(FIXED_PATHS)
    workflows = root / ".github/workflows"
    if workflows.exists():
        paths.update(
            p.relative_to(root).as_posix()
            for p in workflows.iterdir()
            if WORKFLOW.fullmatch(p.relative_to(root).as_posix())
        )
    if len(paths) > MAX_FILES:
        raise OperationalError("operational inventory exceeds its file bound")
    result = {}
    total = 0
    for path in sorted(paths):
        text = _read(root, path)
        if text is None:
            continue
        total += len(text.encode())
        if total > MAX_SNAPSHOT_BYTES:
            raise OperationalError("operational snapshot exceeds its byte bound")
        if path in {FEATURE, FEATURE_LOCK}:
            continue
        if path == DOCKERFILE:
            pins = _dockerfile(text, path)
        elif path == PROBE:
            pins = _probe(text, path)
        elif path in CATALOGUES:
            pins = _catalogue(text, path)
        elif path in IMAGE_JSON:
            pins = _image_json(text, path)
        else:
            pins = _runners(text, path)
        for pin in pins:
            result[pin.key] = pin
            if len(result) > MAX_PINS:
                raise OperationalError("operational inventory exceeds its identity bound")
    return result


def _instant(value: str, now: datetime) -> datetime:
    try:
        observed = datetime.fromisoformat(value.replace("Z", "+00:00"))
        if observed.tzinfo is None or observed > now:
            raise ValueError("invalid observation")
    except ValueError as error:
        raise OperationalError("first-seen observation is malformed or in the future") from error
    return observed.astimezone(UTC)


def _format(value: datetime) -> str:
    return value.astimezone(UTC).isoformat().replace("+00:00", "Z")


def verify_manifest(pin: Pin, request: Request) -> None:
    """Read only an exact reviewed-origin manifest, with anonymous pull authentication."""
    if pin.kind != "oci" or not pin.covered:
        raise OperationalError("manifest identity is outside reviewed origin/repository policy")
    _ref(f"{pin.source}/{pin.name}@{pin.digest}", pin.path, pin.line)
    url = f"https://{pin.source}/v2/{pin.name}/manifests/{pin.digest}"
    headers = {"Accept": ", ".join(sorted(MEDIA_TYPES))}
    try:
        data = request(url, headers)
    except urllib.error.HTTPError as error:
        if error.code != 401:
            raise
        error.close()
        profiles = {
            "ghcr.io": ("https://ghcr.io/token", "ghcr.io"),
            "quay.io": ("https://quay.io/v2/auth", "quay.io"),
            "codeberg.org": ("https://codeberg.org/v2/token", "container_registry"),
        }
        if pin.source not in profiles:
            raise OperationalError(
                "reviewed public registry unexpectedly requires authentication"
            ) from None
        endpoint, service = profiles[pin.source]
        scope = urllib.parse.urlencode({"service": service, "scope": f"repository:{pin.name}:pull"})
        token_data = _json(request(f"{endpoint}?{scope}", {}))
        token = token_data.get("token", token_data.get("access_token"))
        if (
            not isinstance(token, str)
            or not token
            or len(token) > 16384
            or re.search(r"[\r\n]", token)
        ):
            raise OperationalError(
                "anonymous registry authentication returned invalid evidence"
            ) from None
        data = request(url, {**headers, "Authorization": "Bearer " + token})
    if "sha256:" + hashlib.sha256(data).hexdigest() != pin.digest:
        raise OperationalError("registry manifest bytes differ from the pinned SHA-256")
    manifest = _json(data)
    if (
        type(manifest.get("schemaVersion")) is not int
        or manifest["schemaVersion"] != 2
        or manifest.get("mediaType") not in MEDIA_TYPES
    ):
        raise OperationalError("registry manifest has an unsupported schema or media type")
    index = manifest["mediaType"] in {
        "application/vnd.oci.image.index.v1+json",
        "application/vnd.docker.distribution.manifest.list.v2+json",
    }
    descriptors = manifest.get("manifests") if index else manifest.get("layers")
    if not isinstance(descriptors, list) or (index and not descriptors):
        raise OperationalError("registry manifest descriptors are malformed")
    if not index:
        descriptors = [manifest.get("config"), *descriptors]
    for descriptor in descriptors:
        if (
            not isinstance(descriptor, dict)
            or not isinstance(descriptor.get("digest"), str)
            or re.fullmatch(r"sha256:[a-f0-9]{64}", descriptor["digest"]) is None
            or type(descriptor.get("size")) is not int
            or descriptor["size"] < 0
            or not isinstance(descriptor.get("mediaType"), str)
            or (index and descriptor["mediaType"] not in MEDIA_TYPES)
        ):
            raise OperationalError("registry manifest descriptor identity is malformed")


def check_operational(
    base: Path,
    head: Path,
    *,
    now: datetime,
    ledger: dict[str, str],
    context: DependencyContext | None,
    request: Request,
    remaining: int,
    state_limit: int,
) -> dict[str, Any]:
    """Return outcomes using the caller's one ledger and metadata/request budgets."""
    report: dict[str, Any] = {"results": [], "coverage_gaps": []}
    try:
        from .dependencies import REPOSITORIES

        if context is not None and context.repository not in REPOSITORIES:
            raise OperationalError("operational evidence repository is outside trusted ownership")
        if context is not None:
            for path in snapshot_paths(context.changed_paths) & set(context.changed_paths):
                if not (base / path).exists() and not (head / path).exists():
                    raise OperationalError(
                        "changed operational input is missing from exact snapshots"
                    )
        before, after = _inventory(base), _inventory(head)
        old_features, old_valid, old_feature_errors = _features(base)
        new_features, new_valid, feature_errors = _features(head)
        for source in old_features.keys() | new_features.keys():
            if old_features.get(source) != new_features.get(source) and source in feature_errors:
                raise OperationalError(feature_errors[source])
        if old_features != new_features:
            validate_feature_graph(head, request, now=now)
            # An invalid old graph cannot launder previously orphaned identities
            # into accepted preexisting dependencies. The shared ledger still
            # preserves any independently established exact-digest observation.
            if old_feature_errors:
                old_valid = {}
        for pin in old_valid.values():
            before[pin.key] = pin
        for pin in new_valid.values():
            after[pin.key] = pin
        introduced = [after[key] for key in sorted(after.keys() - before.keys())]
        if sum(pin.covered or pin.kind == "unsupported-runner" for pin in introduced) > remaining:
            raise OperationalError("combined introduced identities exceed their request bound")
    except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError, ConnectionError) as error:
        retryable = (
            not isinstance(error, urllib.error.HTTPError) or error.code == 429 or error.code >= 500
        )
        report["results"].append(
            {
                "ecosystem": "operational",
                "status": "failure",
                "reason": "feature graph metadata acquisition: " + type(error).__name__,
                "retry_at": _format(now + timedelta(hours=1)) if retryable else None,
                "retry_eligible": retryable,
            }
        )
        return report
    except (OperationalError, ValueError, TypeError, KeyError, OSError) as error:
        report["results"].append(
            {
                "ecosystem": "operational",
                "status": "failure",
                "reason": str(error),
                "retry_at": None,
                "retry_eligible": False,
            }
        )
        return report
    # Author identity controls only the registry first-party exemption. Every
    # introduced covered operational identity receives the same observation hold.
    for pin in introduced:
        if pin.native_dated:
            continue
        if pin.kind == "oci" and not pin.covered:
            report["coverage_gaps"].append(
                {
                    "source": pin.source,
                    "path": pin.path,
                    "policy": "native-timestamp-required; no operational fallback",
                }
            )
            continue
        result: dict[str, Any] = {
            "ecosystem": "github-runners" if pin.source == "github-hosted" else "oci",
            "name": f"{pin.source}/{pin.name}",
            "version": pin.version,
            "artifact_key": pin.key,
            "artifact_immutable": pin.kind == "oci",
            "path": pin.path,
            "line": pin.line,
            "status": "pass",
            "age_evidence": "runner-label-first-seen"
            if pin.kind == "runner"
            else "manifest-first-seen",
            "retry_at": None,
            "retry_eligible": False,
        }
        try:
            if not pin.covered:
                report["coverage_gaps"].append({"source": pin.source, "path": pin.path})
                raise OperationalError(
                    "operational source or runner grammar lacks reviewed fallback"
                )
            if pin.key not in ledger:
                if len(ledger) >= state_limit:
                    raise OperationalError("shared first-seen state capacity exhausted")
                if pin.kind == "oci":
                    verify_manifest(pin, request)
                ledger[pin.key] = _format(now)
            observed = _instant(ledger[pin.key], now)
            result["evidence_at"] = _format(observed)
            if now < observed + HOLD:
                result.update(
                    status="waiting",
                    retry_at=_format(observed + HOLD),
                    retry_eligible=True,
                    reason="72-hour operational observation hold",
                )
        except urllib.error.HTTPError as error:
            result.update(status="failure", reason=f"registry manifest HTTP status {error.code}")
            if error.code == 429 or error.code >= 500:
                result.update(retry_at=_format(now + timedelta(hours=1)), retry_eligible=True)
        except (urllib.error.URLError, TimeoutError, ConnectionError) as error:
            result.update(
                status="failure",
                reason=f"manifest acquisition: {type(error).__name__}",
                retry_at=_format(now + timedelta(hours=1)),
                retry_eligible=True,
            )
        except (OperationalError, ValueError, TypeError, OSError) as error:
            result.update(status="failure", reason=str(error))
        report["results"].append(result)
    return report
