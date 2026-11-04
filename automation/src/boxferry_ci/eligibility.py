"""Metadata-only dependency eligibility, trusted persistence and candidate receipts.

The privileged service runs ONLY from the default-branch caller on
schedule or workflow_run. It never checks out a candidate,
loads candidate Git configuration, invokes candidate commands or consumes candidate
artifacts. Its write token is used only for dependency statuses and one bounded
rerun of an unchanged, read-only CI workflow. Required CI is never synthesized.

First-seen timestamps come only from artifacts whose GitHub run metadata proves the
expected trusted caller, event, default-branch ancestry and immutable shared
implementation. A lost/expired/invalid artifact starts a new conservative hold.
The read-only candidate job validates the same provenance and exact-SHA report.
Queue composition currently supports one entry; wider groups fail explicitly.
"""

from __future__ import annotations

import argparse
import base64
import io
import json
import os
import re
import tempfile
import time
import zipfile
from dataclasses import dataclass, field
from datetime import UTC, datetime
from email.message import Message
from pathlib import Path
from typing import IO, TYPE_CHECKING, Any
from urllib.error import HTTPError, URLError
from urllib.parse import quote, urlparse
from urllib.request import HTTPRedirectHandler, Request, build_opener, urlopen

if TYPE_CHECKING:
    from .operational import SnapshotCache

STATUS_CONTEXT = "Dependency eligibility"
WORKFLOW_PATH = ".github/workflows/dependency-eligibility.yml"
SHARED_WORKFLOW_PATH = (
    "Strukturpiloten/boxferry/.github/workflows/shared-dependency-eligibility.yml"
)
ARTIFACT_NAME = "dependency-eligibility-state-v1"
STATE_FILE = "eligibility-state.json"
ALLOWED_EVENTS = {"schedule", "workflow_run"}
REPOSITORIES = {
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
}
LOCK_PATHS = ("Cargo.lock", "package-lock.json", "uv.lock", "automation/uv.lock")
MAX_JSON_BYTES = 4 * 1024 * 1024
MAX_FILE_BYTES = 2 * 1024 * 1024
MAX_STATE_BYTES = 2 * 1024 * 1024
MAX_SNAPSHOT_BYTES = 16 * 1024 * 1024
MAX_SCHEDULED_PRS = 20
SHA = re.compile(r"[0-9a-f]{40}\Z")


class EligibilityError(ValueError):
    """Unavailable or inconsistent authoritative eligibility evidence."""


class MissingResource(EligibilityError):
    """An optional authoritative file or artifact does not exist."""


class NoRedirect(HTTPRedirectHandler):
    """Keep API credentials away from signed artifact-download redirects."""

    def redirect_request(
        self,
        req: Request,
        fp: IO[bytes],
        code: int,
        msg: str,
        headers: Message,
        newurl: str,
    ) -> None:
        return None


class GitHub:
    """Small fixed-origin, finite REST/GraphQL client; no candidate URLs or remotes."""

    def __init__(self, repository: str, token: str) -> None:
        if repository not in REPOSITORIES:
            raise EligibilityError("repository is outside the shared automation allowlist")
        self.repository = repository
        self.token = token
        self.requests = 0
        self.deadline = time.monotonic() + 480

    def request(self, method: str, path: str, data: dict[str, Any] | None = None) -> object:
        """Request one bounded GitHub JSON response, without automatic retries."""
        self.requests += 1
        if self.requests > 500 or time.monotonic() > self.deadline:
            raise EligibilityError("GitHub metadata request/time budget exhausted")
        if not path.startswith("/") or path.startswith("//"):
            raise EligibilityError("GitHub API path must be relative to its fixed origin")
        request = Request(
            "https://api.github.com" + path,
            data=None if data is None else json.dumps(data).encode(),
            method=method,
            headers={
                "Authorization": f"Bearer {self.token}",
                "Accept": "application/vnd.github+json",
                "X-GitHub-Api-Version": "2022-11-28",
                "User-Agent": "boxferry-eligibility-v1",
                "Content-Type": "application/json",
            },
        )
        try:
            with build_opener(NoRedirect).open(request, timeout=20) as response:
                raw = response.read(MAX_JSON_BYTES + 1)
        except HTTPError as error:
            if error.code == 404:
                raise MissingResource("GitHub metadata resource is absent") from None
            raise EligibilityError(
                f"GitHub metadata request failed with HTTP {error.code}"
            ) from None
        except OSError as error:
            raise EligibilityError(
                f"GitHub metadata transport failed: {type(error).__name__}"
            ) from None
        if len(raw) > MAX_JSON_BYTES:
            raise EligibilityError("GitHub metadata response exceeds its size budget")
        if not raw:
            return None
        try:
            return json.loads(raw)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise EligibilityError("GitHub metadata response is not valid JSON") from error

    def get(self, suffix: str) -> dict[str, Any]:
        value = self.request("GET", f"/repos/{self.repository}" + (f"/{suffix}" if suffix else ""))
        if not isinstance(value, dict):
            raise EligibilityError("GitHub metadata response must be an object")
        return value

    def items(self, suffix: str) -> list[dict[str, Any]]:
        value = self.request("GET", f"/repos/{self.repository}" + (f"/{suffix}" if suffix else ""))
        if not isinstance(value, list) or not all(isinstance(item, dict) for item in value):
            raise EligibilityError("GitHub metadata response must be an object list")
        return value

    def post(self, suffix: str, data: dict[str, Any]) -> None:
        self.request("POST", f"/repos/{self.repository}/{suffix}", data)

    def artifact(self, artifact_id: int) -> bytes:
        """Follow a trusted API's HTTPS signed URL without forwarding any token."""
        request = Request(
            f"https://api.github.com/repos/{self.repository}/actions/artifacts/{artifact_id}/zip",
            headers={
                "Authorization": f"Bearer {self.token}",
                "User-Agent": "boxferry-eligibility-v1",
            },
        )
        try:
            with build_opener(NoRedirect).open(request, timeout=20) as response:
                raw = response.read(MAX_STATE_BYTES + 65537)
        except HTTPError as error:
            if error.code not in {301, 302, 303, 307, 308}:
                raise EligibilityError(
                    f"trusted artifact download failed with HTTP {error.code}"
                ) from None
            location = error.headers.get("Location", "")
            parsed = urlparse(location)
            if (
                parsed.scheme != "https"
                or not parsed.hostname
                or parsed.username
                or parsed.password
            ):
                raise EligibilityError(
                    "trusted artifact redirect is not a safe HTTPS URL"
                ) from None
            try:
                # Deliberately a fresh request: no Authorization header crosses origins.
                with urlopen(Request(location), timeout=20) as response:
                    raw = response.read(MAX_STATE_BYTES + 65537)
            except OSError as nested:
                raise EligibilityError("trusted artifact download transport failed") from nested
        except OSError as error:
            raise EligibilityError("trusted artifact download transport failed") from error
        if len(raw) > MAX_STATE_BYTES + 65536:
            raise EligibilityError("trusted artifact archive exceeds its size budget")
        return raw


def revision(value: object) -> str:
    """Require a complete immutable Git object identifier."""
    if not isinstance(value, str) or not SHA.fullmatch(value):
        raise EligibilityError("authoritative metadata has an invalid commit revision")
    return value


def utc(value: object) -> datetime:
    """Parse timezone-bearing persisted evidence, never a local wall-clock date."""
    if not isinstance(value, str):
        raise EligibilityError("persisted time must be an RFC3339 string")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise EligibilityError("persisted time is malformed") from error
    if parsed.tzinfo is None:
        raise EligibilityError("persisted time has no timezone")
    return parsed.astimezone(UTC)


def workflow_file(repository: str) -> str:
    return (
        ".github/workflows/check.yml"
        if repository.endswith("/docker-lens")
        else ".github/workflows/ci.yml"
    )


def service_revision(run: dict[str, Any]) -> str | None:
    """Derive the exact canonical shared source from GitHub-owned run metadata."""
    references = run.get("referenced_workflows")
    if not isinstance(references, list):
        return None
    prefix = f"{SHARED_WORKFLOW_PATH}@"
    sources: set[str] = set()
    for item in references:
        if not isinstance(item, dict):
            return None
        path = item.get("path")
        if not isinstance(path, str) or not path.startswith(prefix):
            continue
        source = path.removeprefix(prefix)
        if not SHA.fullmatch(source) or item.get("sha") != source:
            return None
        sources.add(source)
    return next(iter(sources)) if len(sources) == 1 else None


def source_ancestor(api: GitHub, source: str, implementation: str) -> bool:
    """Prove historical shared source ancestry in BoxFerry, never the consumer."""
    if not SHA.fullmatch(source) or not SHA.fullmatch(implementation):
        return False
    if source == implementation:
        return True
    comparison = api.request(
        "GET", f"/repos/Strukturpiloten/boxferry/compare/{source}...{implementation}"
    )
    if not isinstance(comparison, dict):
        return False
    merge_base = comparison.get("merge_base_commit")
    return (
        comparison.get("status") in {"ahead", "identical"}
        and isinstance(merge_base, dict)
        and merge_base.get("sha") == source
    )


def trusted_run(
    api: GitHub,
    run: dict[str, Any],
    implementation: str,
    default_branch: str,
    *,
    allow_ancestor_observations: bool = False,
) -> bool:
    """Validate the default-branch consumer and exact canonical shared source."""
    if (
        run.get("repository", {}).get("full_name") != api.repository
        or run.get("event") not in ALLOWED_EVENTS
        or run.get("path") != WORKFLOW_PATH
        or run.get("head_branch") != default_branch
        or run.get("conclusion") != "success"
        or run.get("status") != "completed"
    ):
        return False
    source = service_revision(run)
    if source is None or (
        source != implementation
        and (not allow_ancestor_observations or not source_ancestor(api, source, implementation))
    ):
        return False
    head = run.get("head_sha")
    if not isinstance(head, str) or not SHA.fullmatch(head):
        return False
    comparison = api.get(f"compare/{head}...{quote(default_branch, safe='')}")
    # Historical main commits remain trusted after normal main advancement.
    return comparison.get("status") in {"ahead", "identical"}


def decode_artifact(raw: bytes) -> dict[str, Any]:
    """Read exactly one bounded JSON file without extracting archive paths."""
    try:
        with zipfile.ZipFile(io.BytesIO(raw)) as archive:
            members = archive.infolist()
            if len(members) != 1 or members[0].filename != STATE_FILE:
                raise EligibilityError("trusted artifact has unexpected archive members")
            member = members[0]
            if member.file_size > MAX_STATE_BYTES or member.flag_bits & 1:
                raise EligibilityError("trusted artifact is oversized or encrypted")
            value = json.loads(archive.read(member))
    except (OSError, zipfile.BadZipFile, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise EligibilityError("trusted artifact cannot be decoded") from error
    if not isinstance(value, dict):
        raise EligibilityError("trusted artifact JSON is not an object")
    return value


def validate_state(
    value: dict[str, Any],
    *,
    repository: str,
    run_id: int,
    implementation: str,
    now: datetime,
) -> dict[str, Any]:
    """Validate persisted provenance and all first-seen times before their reuse."""
    if (
        type(value.get("schema")) is not int
        or value.get("schema") != 1
        or value.get("repository") != repository
        or value.get("run_id") != run_id
        or value.get("implementation_revision") != implementation
        or value.get("workflow_path") != WORKFLOW_PATH
    ):
        raise EligibilityError("trusted artifact provenance does not match the verified run")
    observed = utc(value.get("observed_at"))
    if observed > now:
        raise EligibilityError("trusted artifact observation is in the future")
    dependency = value.get("dependency_state")
    if (
        not isinstance(dependency, dict)
        or type(dependency.get("schema_version")) is not int
        or dependency.get("schema_version") != 1
    ):
        raise EligibilityError("trusted dependency state schema is invalid")
    first_seen = dependency.get("first_seen")
    if not isinstance(first_seen, dict) or len(first_seen) > 4096:
        raise EligibilityError("trusted first-seen state exceeds its bounds")
    for key, instant in first_seen.items():
        if not isinstance(key, str) or not re.fullmatch(r"[0-9a-f]{64}", key):
            raise EligibilityError("trusted first-seen key is not an immutable artifact identity")
        if utc(instant) > observed:
            raise EligibilityError("trusted first-seen time is later than its observation")
    if not isinstance(value.get("reports"), dict) or len(value["reports"]) > 100:
        raise EligibilityError("trusted candidate reports are malformed")
    cursor = value.get("scan_cursor", 0)
    if type(cursor) is not int or cursor < 0:
        raise EligibilityError("trusted scheduled scan cursor is malformed")
    reruns = value.get("reruns")
    if not isinstance(reruns, dict) or len(reruns) > 4096:
        raise EligibilityError("trusted rerun ledger is malformed")
    for key, run in reruns.items():
        revision(key)
        if type(run) is not int or run < 1:
            raise EligibilityError("trusted rerun ledger has an invalid run identifier")
    return value


def run_artifact(api: GitHub, run_id: int) -> dict[str, Any]:
    artifacts = api.get(f"actions/runs/{run_id}/artifacts?per_page=100")
    matches = [item for item in artifacts.get("artifacts", []) if item.get("name") == ARTIFACT_NAME]
    if len(matches) != 1 or matches[0].get("expired"):
        raise EligibilityError("trusted service run has no unique live state artifact")
    item = matches[0]
    if (
        type(item.get("id")) is not int
        or item.get("size_in_bytes", MAX_STATE_BYTES + 1) > MAX_STATE_BYTES
    ):
        raise EligibilityError("trusted state artifact metadata exceeds its bounds")
    return decode_artifact(api.artifact(item["id"]))


def previous_state(
    api: GitHub, implementation: str, default_branch: str, now: datetime
) -> dict[str, Any] | None:
    """Restore only this service's successful trusted runs; absence reobserves."""
    try:
        runs = api.get(
            "actions/workflows/dependency-eligibility.yml/runs?per_page=30&status=success"
        )
    except MissingResource:
        return None
    for summary in runs.get("workflow_runs", [])[:30]:
        run_id = summary.get("id")
        if type(run_id) is not int:
            continue
        try:
            run = api.get(f"actions/runs/{run_id}")
            if trusted_run(
                api,
                run,
                implementation,
                default_branch,
                allow_ancestor_observations=True,
            ):
                source = service_revision(run)
                if source is None:
                    continue
                state = validate_state(
                    run_artifact(api, run_id),
                    repository=api.repository,
                    run_id=run_id,
                    implementation=source,
                    now=now,
                )
                if source == implementation:
                    return state
                # Compatibility applies to observations only. An older checker
                # cannot provide reports, retry authority or scan policy to this one.
                return {
                    "run_id": run_id,
                    "implementation_revision": source,
                    "dependency_state": {
                        "schema_version": 1,
                        "first_seen": dict(state["dependency_state"]["first_seen"]),
                    },
                    "reports": {},
                    "reruns": {},
                    "scan_cursor": 0,
                    "observations_migrated": True,
                }
        except EligibilityError:
            continue
    return None


@dataclass(frozen=True)
class Candidate:
    """An exact revision, its source PR and authoritative comparison baseline."""

    base: str
    head: str
    number: int
    source_head: str
    actor_login: str
    actor_type: str
    scope: str


def target_revision(api: GitHub, branch: str) -> str:
    """Read one exact target branch ref; PR.base.sha can lag its live merge base."""
    if not branch or branch.startswith("/") or any(char in branch for char in "\r\n\0"):
        raise EligibilityError("PR target branch identity is malformed")
    value = api.get(f"git/ref/heads/{quote(branch, safe='')}")
    target = value.get("object")
    if (
        value.get("ref") != f"refs/heads/{branch}"
        or not isinstance(target, dict)
        or target.get("type") != "commit"
    ):
        raise EligibilityError("target ref does not identify the expected branch commit")
    return revision(target.get("sha"))


def require_ancestor(api: GitHub, base: str, candidate: str) -> None:
    """Prove exact ancestry through GitHub, without assumptions about parent order."""
    comparison = api.get(f"compare/{base}...{candidate}")
    merge_base = comparison.get("merge_base_commit")
    if (
        comparison.get("status") not in {"ahead", "identical"}
        or not isinstance(merge_base, dict)
        or merge_base.get("sha") != base
    ):
        raise EligibilityError(
            "live target/source is not an ancestor of the synthetic candidate; "
            "retry after recalculation"
        )


def pr_identity(
    pr: dict[str, Any],
    repository: str,
    number: int,
) -> tuple[str, str, str, str, str]:
    """Retain authoritative mutable PR identities while ignoring stale base.sha."""
    base, head = pr.get("base"), pr.get("head")
    if (
        pr.get("number") != number
        or pr.get("state") != "open"
        or not isinstance(base, dict)
        or not isinstance(head, dict)
        or not isinstance(base.get("repo"), dict)
        or base["repo"].get("full_name") != repository
        or pr.get("mergeable") is not True
    ):
        raise EligibilityError(
            "PR is closed, foreign, conflicted or still computing its merge candidate"
        )
    branch = base.get("ref")
    if not isinstance(branch, str) or not branch:
        raise EligibilityError("PR has no authoritative target branch ref")
    actor = pr.get("user") or {}
    if not isinstance(actor, dict):
        raise EligibilityError("PR author metadata is malformed")
    return (
        branch,
        revision(pr.get("merge_commit_sha")),
        revision(head.get("sha")),
        str(actor.get("login", "")),
        str(actor.get("type", "")),
    )


def pr_candidate(api: GitHub, number: int, *, expected_target: str | None = None) -> Candidate:
    """Bind a live synthetic merge to current target/source refs before and after."""
    identity = pr_identity(api.get(f"pulls/{number}"), api.repository, number)
    branch, merge, source, actor_login, actor_type = identity
    if expected_target is not None and branch != expected_target:
        raise EligibilityError("queued PR targets a different branch from the reviewed queue")
    base = target_revision(api, branch)
    require_ancestor(api, base, merge)
    require_ancestor(api, source, merge)
    if pr_identity(api.get(f"pulls/{number}"), api.repository, number) != identity:
        raise EligibilityError(
            "PR source or synthetic candidate changed during verification; next event rechecks"
        )
    if target_revision(api, branch) != base:
        raise EligibilityError("PR target branch moved during verification; next event rechecks")
    return Candidate(base, merge, number, source, actor_login, actor_type, "pull_request")


def require_single_entry_queue(api: GitHub, default_branch: str) -> None:
    """Read effective active rules using the documented Metadata:read endpoint."""
    rules = api.items(f"rules/branches/{quote(default_branch, safe='')}?per_page=100&page=1")
    # A full first page may omit another applicable rule. Do not authorize from
    # a prefix; this bounded reader deliberately rejects that pagination boundary.
    if len(rules) >= 100:
        raise EligibilityError("effective branch rules may be truncated or exceed their bound")
    if not all(isinstance(rule, dict) and isinstance(rule.get("type"), str) for rule in rules):
        raise EligibilityError("effective branch rule metadata is malformed")
    queues = [rule for rule in rules if rule["type"] == "merge_queue"]
    if len(queues) != 1:
        raise EligibilityError("effective branch rules require exactly one active merge queue")
    parameters = queues[0].get("parameters")
    if not isinstance(parameters, dict) or any(
        type(parameters.get(key)) is not int or parameters[key] != 1
        for key in ("max_entries_to_build", "max_entries_to_merge")
    ):
        raise EligibilityError("dependency eligibility requires reviewed one-entry queue limits")


def queue_graphql_errors(response: object) -> str:
    """Project bounded closed error identifiers, never API messages or native text."""
    if not isinstance(response, dict):
        return "graphql=INVALID_RESPONSE"
    errors = response.get("errors")
    if not isinstance(errors, list) or not errors:
        return "graphql=INVALID_ERRORS"
    known_types = {
        "FORBIDDEN",
        "UNAUTHORIZED",
        "UNAUTHENTICATED",
        "NOT_FOUND",
        "RATE_LIMITED",
        "UNPROCESSABLE",
        "GRAPHQL_VALIDATION_FAILED",
        "INTERNAL",
    }
    known_codes = known_types | {
        "undefinedField",
        "argumentNotAccepted",
        "selectionMismatch",
        "missingRequiredArguments",
        "variableMismatch",
    }
    fields = {
        ("repository",): "REPOSITORY",
        ("repository", "mergeQueue"): "QUEUE",
        ("repository", "mergeQueue", "entries"): "ENTRIES",
        ("repository", "mergeQueue", "entries", "totalCount"): "ENTRY_COUNT",
        ("repository", "mergeQueue", "entries", "nodes"): "ENTRY",
        ("repository", "mergeQueue", "entries", "nodes", "baseCommit"): "ENTRY_BASE",
        ("repository", "mergeQueue", "entries", "nodes", "baseCommit", "oid"): "ENTRY_BASE",
        ("repository", "mergeQueue", "entries", "nodes", "headCommit"): "ENTRY_HEAD",
        ("repository", "mergeQueue", "entries", "nodes", "headCommit", "oid"): "ENTRY_HEAD",
        ("repository", "mergeQueue", "entries", "nodes", "pullRequest"): "SOURCE_PR",
        ("repository", "mergeQueue", "entries", "nodes", "pullRequest", "number"): "SOURCE_PR",
        ("repository", "mergeQueue", "entries", "nodes", "pullRequest", "headRefOid"): "SOURCE_PR",
    }
    summaries: list[str] = []
    for error in errors[:8]:
        kind, code, field = "UNKNOWN", "UNKNOWN", "UNKNOWN"
        if isinstance(error, dict):
            value = error.get("type")
            if isinstance(value, str) and value in known_types:
                kind = value
            extensions = error.get("extensions")
            value = extensions.get("code") if isinstance(extensions, dict) else None
            if isinstance(value, str) and value in known_codes:
                code = value
            path = error.get("path")
            if (
                isinstance(path, list)
                and 1 <= len(path) <= 8
                and all(
                    isinstance(part, str) or (type(part) is int and 0 <= part < 100)
                    for part in path
                )
            ):
                field = fields.get(tuple(part for part in path if isinstance(part, str)), "UNKNOWN")
        summaries.append(f"type={kind},code={code},field={field}")
    if len(errors) > 8:
        summaries.append("additional_errors=OMITTED")
    return "; ".join(summaries)


def queue_candidate(api: GitHub, head: str, default_branch: str) -> Candidate:
    """Bind a one-entry group to live queue metadata and the original PR author."""
    owner, name = api.repository.split("/")
    require_single_entry_queue(api, default_branch)
    query = (
        "query($owner:String!,$name:String!,$branch:String!){repository(owner:$owner,name:$name){"
        "mergeQueue(branch:$branch){entries(first:100){totalCount nodes{baseCommit{oid} "
        "headCommit{oid} pullRequest{number headRefOid}}}}}}"
    )
    data = api.request(
        "POST",
        "/graphql",
        {"query": query, "variables": {"owner": owner, "name": name, "branch": default_branch}},
    )
    if not isinstance(data, dict) or "errors" in data:
        raise EligibilityError(
            "authoritative merge queue metadata is unavailable: " + queue_graphql_errors(data)
        )
    payload = data.get("data")
    repository = payload.get("repository") if isinstance(payload, dict) else None
    queue = repository.get("mergeQueue") if isinstance(repository, dict) else None
    if not isinstance(queue, dict):
        raise EligibilityError("authoritative merge queue metadata is absent")
    entries = queue.get("entries", {})
    if not isinstance(entries, dict):
        raise EligibilityError("merge queue entry metadata is malformed")
    nodes = entries.get("nodes", [])
    total = entries.get("totalCount")
    if (
        type(total) is not int
        or not 0 <= total <= 100
        or not isinstance(nodes, list)
        or len(nodes) != total
        or not all(isinstance(node, dict) for node in nodes)
    ):
        raise EligibilityError("merge queue entry metadata is truncated or exceeds its bound")
    base = target_revision(api, default_branch)
    require_ancestor(api, base, head)
    matching: list[Candidate] = []
    candidate_tree: str | None = None
    for node in nodes:
        if (node.get("baseCommit") or {}).get("oid") != base:
            continue
        node_head = (node.get("headCommit") or {}).get("oid")
        if not isinstance(node_head, str) or not SHA.fullmatch(node_head):
            continue  # Waiting entries have no built candidate to authorize.
        number = node.get("pullRequest", {}).get("number")
        if type(number) is not int or number < 1:
            raise EligibilityError("queue entry has no authoritative source PR")
        pr = pr_candidate(api, number, expected_target=default_branch)
        if node.get("pullRequest", {}).get("headRefOid") != pr.source_head:
            continue
        if base != pr.base or node_head not in {head, pr.head, pr.source_head}:
            continue
        if candidate_tree is None:
            candidate_tree = revision(api.get(f"git/commits/{head}").get("tree", {}).get("sha"))
        pr_tree = (
            candidate_tree
            if pr.head == head
            else revision(api.get(f"git/commits/{pr.head}").get("tree", {}).get("sha"))
        )
        if candidate_tree != pr_tree:
            continue
        matching.append(
            Candidate(
                base,
                head,
                number,
                pr.source_head,
                pr.actor_login,
                pr.actor_type,
                "merge_group",
            )
        )
    if len(matching) != 1:
        raise EligibilityError(
            "queue candidate has no unique exact source PR; multi-entry groups are unsupported"
        )
    if target_revision(api, default_branch) != base:
        raise EligibilityError("queue target branch moved during verification; next event rechecks")
    return matching[0]


def changed_paths(api: GitHub, candidate: Candidate) -> list[str]:
    comparison = api.get(f"compare/{candidate.base}...{candidate.head}?per_page=100")
    if comparison.get("merge_base_commit", {}).get("sha") != candidate.base:
        raise EligibilityError("candidate comparison does not bind the exact expected base")
    files = comparison.get("files")
    if not isinstance(files, list) or len(files) >= 300:
        raise EligibilityError("candidate changed-file metadata is missing or may be truncated")
    paths: set[str] = set()
    for item in files:
        for key in ("filename", "previous_filename"):
            path = item.get(key)
            if path is not None:
                if (
                    not isinstance(path, str)
                    or path.startswith("/")
                    or ".." in path.split("/")
                    or "\\" in path
                ):
                    raise EligibilityError("candidate metadata contains an unsafe repository path")
                paths.add(path)
    return sorted(paths)


def file_bytes(api: GitHub, path: str, head: str) -> bytes | None:
    try:
        value = api.get(f"contents/{quote(path, safe='/')}?ref={head}")
    except MissingResource:
        return None
    if (
        not isinstance(value, dict)
        or value.get("type") != "file"
        or value.get("size", MAX_FILE_BYTES + 1) > MAX_FILE_BYTES
    ):
        raise EligibilityError("candidate metadata file is not a bounded regular file")
    if value.get("encoding") != "base64":
        raise EligibilityError("candidate metadata file has no complete bounded inline content")
    try:
        data = base64.b64decode("".join(value.get("content", "").split()), validate=True)
    except (ValueError, TypeError) as error:
        raise EligibilityError("candidate metadata file content is malformed") from error
    if len(data) != value.get("size"):
        raise EligibilityError("candidate metadata file size does not match its content")
    return data


def snapshot(
    api: GitHub,
    head: str,
    destination: Path,
    paths: list[str],
    cache: SnapshotCache | None = None,
) -> None:
    from .operational import SnapshotCache, inventory_blobs, snapshot_paths

    revision(head)
    manifests = {
        path
        for path in paths
        if path == "Cargo.toml" or re.fullmatch(r"crates/[^/]+/Cargo.toml", path)
    }
    operational = snapshot_paths(paths)
    blobs: dict[str, str] = {}
    if operational:
        # Inventory immutable trees, never candidate code/configuration. Loading
        # the complete base inventory avoids re-holding an existing label/digest
        # merely because another workflow or declaration starts using it.
        blobs = inventory_blobs(api.get(f"git/trees/{head}?recursive=1"))
        operational.update(snapshot_paths(list(blobs)))
    content_cache = cache if cache is not None else SnapshotCache()
    total = 0
    for path in sorted(set(LOCK_PATHS) | manifests | {"Cargo.toml"} | operational):
        sha = blobs.get(path)
        if path in operational and sha is None:
            continue
        data = content_cache.get(sha) if sha is not None else None
        if data is None:
            data = file_bytes(api, path, head)
            if data is None:
                if sha is not None:
                    raise EligibilityError(
                        "owned operational blob is missing from its exact snapshot"
                    )
                continue
            if sha is not None:
                content_cache.put(sha, data)
        total += len(data)
        if total > MAX_SNAPSHOT_BYTES:
            raise EligibilityError("candidate dependency snapshot exceeds its size budget")
        target = destination / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)


def registry_fetch(api: GitHub, url: str, headers: dict[str, str] | None = None) -> bytes:
    """Share a run-wide network/deadline budget with bounded registry acquisition."""
    if api.requests >= 450 or time.monotonic() >= api.deadline - 30:
        raise URLError("trusted eligibility metadata budget exhausted; next schedule retries")
    api.requests += 1
    from .dependencies import fetch_metadata

    return fetch_metadata(url, headers)


def evaluate(
    api: GitHub, candidate: Candidate, state: dict[str, Any], now: datetime
) -> dict[str, Any]:
    """Materialize inert dependency evidence only, then run the trusted age policy."""
    from .dependencies import DependencyContext, check_dependencies

    paths = changed_paths(api, candidate)
    context = DependencyContext.from_mapping(
        {
            "actor_login": candidate.actor_login,
            "actor_type": candidate.actor_type,
            "repository": api.repository,
            "base_revision": candidate.base,
            "head_revision": candidate.head,
            "changed_paths": paths,
        }
    )
    with tempfile.TemporaryDirectory(prefix="boxferry-dependency-metadata-") as temporary:
        root = Path(temporary)
        base, head = root / "base", root / "head"
        base.mkdir()
        head.mkdir()
        from .operational import SnapshotCache

        cache = SnapshotCache()
        snapshot(api, candidate.base, base, paths, cache)
        snapshot(api, candidate.head, head, paths, cache)
        result = check_dependencies(
            base,
            head,
            now=now,
            state=state,
            context=context,
            fetch=lambda url: registry_fetch(api, url),
            fetch_request=lambda url, headers: registry_fetch(api, url, dict(headers)),
        )
    result.update(
        candidate_sha=candidate.head,
        base_sha=candidate.base,
        source_head_sha=candidate.source_head,
        pull_request=candidate.number,
        scope=candidate.scope,
    )
    return result


def still_current(api: GitHub, candidate: Candidate, default_branch: str) -> bool:
    try:
        current = (
            queue_candidate(api, candidate.head, default_branch)
            if candidate.scope == "merge_group"
            else pr_candidate(api, candidate.number)
        )
        return current == candidate
    except EligibilityError:
        return False


def candidate_run(api: GitHub, run: dict[str, Any], candidate_sha: str) -> bool:
    """Only normal CI/Check, in this repository, at this exact tested revision."""
    expected_name = "Check" if api.repository.endswith("/docker-lens") else "CI"
    return (
        run.get("repository", {}).get("full_name") == api.repository
        and (run.get("head_repository") or {}).get("full_name") == api.repository
        and run.get("event") in {"pull_request", "merge_group"}
        and run.get("name") == expected_name
        and run.get("path") == workflow_file(api.repository)
        and run.get("head_sha") == candidate_sha
    )


@dataclass
class PinFrame:
    """A narrowly recognized mapping context; arbitrary YAML is never rewritten."""

    indent: int
    kind: str
    uses: str = ""
    repository: bool = False
    owner: PinFrame | None = None
    keys: set[str] = field(default_factory=set)


def quoted_line(value: str) -> bool:
    """Reject multiline quoted YAML instead of mistaking its text for mapping keys."""
    stripped = value.lstrip()
    if not stripped.startswith(("'", '"')):
        return True
    quote_character = stripped[0]
    position = 1
    while position < len(stripped):
        char = stripped[position]
        if quote_character == '"' and char == "\\":
            position += 2
            continue
        if char == quote_character:
            if quote_character == "'" and stripped[position : position + 2] == "''":
                position += 2
                continue
            tail = stripped[position + 1 :].strip()
            return not tail or tail.startswith("#")
        position += 1
    return False


def expected_ci_upgrade(trusted: bytes, before: str, after: str) -> bytes | None:
    """Replace only owned literal pins in recognized CI mappings, preserving bytes."""
    try:
        source = trusted.decode("utf-8")
    except UnicodeDecodeError:
        return None
    if "\0" in source or not SHA.fullmatch(before) or not SHA.fullmatch(after) or before == after:
        return None
    mapping = re.compile(
        r"(?P<indent> *)(?P<sequence>- )?(?P<key>[A-Za-z_][A-Za-z0-9_-]*):"
        r"(?P<value>[^\r\n]*)(?P<ending>\r?\n)?\Z"
    )
    scalar = re.compile(
        r"[ \t]*(?P<quote>['\"]?)(?P<token>[^'\"\s#]+)(?P=quote)"
        r"[ \t]*(?:\#[^\r\n]*)?\Z"
    )
    shared = re.compile(
        r"Strukturpiloten/boxferry/\.github/workflows/shared-[a-z0-9-]+\.yml@" + before + r"\Z"
    )
    local_action = re.compile(r"\./_automation/\.github/actions/shared-source\Z")
    checkout = re.compile(r"actions/checkout@[0-9a-f]{40}\Z")
    annotation = re.compile(
        r"(?P<indent> *)# renovate: datasource=github-digest "
        r"depName=Strukturpiloten/boxferry currentValue=automation-v1[ \t]*(?:\r?\n)?\Z"
    )
    block = re.compile(r"(?:[!&][^\s]+[ \t]+)*[|>][1-9+-]*[ \t]*(?:\#.*)?\Z")
    frames: list[PinFrame] = []
    root_keys: set[str] = set()
    output: list[str] = []
    lines = source.splitlines(keepends=True)
    block_indent: int | None = None
    replacements = 0
    for index, line in enumerate(lines):
        stripped = line.lstrip(" ")
        indent = len(line) - len(stripped)
        if not stripped.strip() or stripped.startswith("#"):
            output.append(line)
            continue
        if block_indent is not None and indent > block_indent:
            output.append(line)
            continue
        block_indent = None
        if stripped.startswith("\t"):
            return None
        while frames and frames[-1].indent >= indent:
            frames.pop()
        match = mapping.fullmatch(line)
        if match is None:
            output.append(line)
            continue
        parent = frames[-1] if frames else None
        key_indent = indent
        if match["sequence"]:
            if parent is None or parent.kind != "steps" or indent != parent.indent + 2:
                output.append(line)
                continue
            parent = PinFrame(indent, "step")
            frames.append(parent)
            key_indent += 2
        key, value = match["key"], match["value"]
        keys = (
            root_keys
            if parent is None and key_indent == 0
            else parent.keys
            if parent is not None and key_indent == parent.indent + 2
            else None
        )
        if keys is not None:
            if key in keys:
                return None  # Duplicate mappings make pin ownership ambiguous.
            keys.add(key)
        if not quoted_line(value):
            return None
        if block.fullmatch(value.strip()):
            block_indent = indent
            output.append(line)
            continue
        literal = scalar.fullmatch(value)
        token = None if literal is None else literal["token"]
        direct_owner = (
            parent is not None
            and parent.kind in {"job", "step"}
            and key_indent == parent.indent + 2
        )
        replace = False
        if key == "uses" and direct_owner and token is not None:
            assert parent is not None
            parent.uses = token
            replace = shared.fullmatch(token) is not None
        elif token == before and parent is not None and key_indent == parent.indent + 2:
            if parent.kind == "env" and key == "AUTOMATION_SHA":
                replace = True
            elif parent.kind == "with" and parent.owner is not None:
                owner = parent.owner
                if key in {"automation_sha", "automation-sha"}:
                    replace = bool(
                        shared.fullmatch(owner.uses) or local_action.fullmatch(owner.uses)
                    )
                elif key == "ref" and parent.repository and checkout.fullmatch(owner.uses):
                    marker = annotation.fullmatch(lines[index - 1]) if index else None
                    replace = marker is not None and len(marker["indent"]) == indent
        if (
            parent is not None
            and parent.kind == "with"
            and parent.owner is not None
            and parent.owner.kind == "step"
            and checkout.fullmatch(parent.owner.uses)
            and key_indent == parent.indent + 2
            and key == "repository"
        ):
            parent.repository = token == "Strukturpiloten/boxferry"
        if replace:
            assert literal is not None and token is not None
            updated = token.removesuffix(before) + after
            start = match.start("value") + literal.start("token")
            end = match.start("value") + literal.end("token")
            line = line[:start] + updated + line[end:]
            replacements += 1
        if not value.strip() or value.lstrip().startswith("#"):
            kind = "other"
            owner = None
            if key == "jobs" and parent is None and key_indent == 0:
                kind = "jobs"
            elif parent is not None and parent.kind == "jobs" and key_indent == parent.indent + 2:
                kind = "job"
            elif key == "steps" and direct_owner and parent is not None and parent.kind == "job":
                kind = "steps"
            elif key == "env" and (direct_owner or (parent is None and key_indent == 0)):
                kind = "env"
            elif key == "with" and direct_owner:
                kind, owner = "with", parent
            frames.append(PinFrame(key_indent, kind, owner=owner))
        output.append(line)
    return "".join(output).encode("utf-8") if replacements else None


def shared_default_tip(api: GitHub) -> tuple[str, str]:
    """Read canonical BoxFerry publication ancestry from fixed-origin API facts."""
    repository = api.request("GET", "/repos/Strukturpiloten/boxferry")
    branch = repository.get("default_branch") if isinstance(repository, dict) else None
    if not isinstance(branch, str) or not branch or any(char in branch for char in "\r\n\0"):
        raise EligibilityError("canonical shared repository has no valid default branch")
    path = f"/repos/Strukturpiloten/boxferry/git/ref/heads/{quote(branch, safe='')}"
    value = api.request("GET", path)
    target = value.get("object") if isinstance(value, dict) else None
    if (
        not isinstance(value, dict)
        or value.get("ref") != f"refs/heads/{branch}"
        or not isinstance(target, dict)
        or target.get("type") != "commit"
    ):
        raise EligibilityError("canonical shared default ref is not a commit")
    return branch, revision(target.get("sha"))


def retry_ci_upgrade(
    api: GitHub,
    candidate: Candidate,
    trusted: bytes,
    proposed: bytes,
    implementation: str,
) -> bool:
    """Permit only a published Renovate self-upgrade, with unchanged policy bytes."""
    from .admission import AdmissionError, parse_manifest

    if candidate.actor_login != "renovate[bot]" or candidate.actor_type != "Bot":
        return False
    try:
        before_manifest = file_bytes(api, ".github/automation.json", candidate.base)
        after_manifest = file_bytes(api, ".github/automation.json", candidate.head)
        if before_manifest is None or after_manifest is None:
            return False
        before, policy = parse_manifest(before_manifest)
        after, after_policy = parse_manifest(after_manifest)
        if before != implementation or before == after or policy != after_policy:
            return False
        baseline = file_bytes(api, workflow_file(api.repository), candidate.base)
        if baseline != trusted:
            return False
        policy_before = file_bytes(api, policy, candidate.base)
        policy_after = file_bytes(api, policy, candidate.head)
        if policy_before is None or policy_after != policy_before:
            return False
        branch, tip = shared_default_tip(api)
        if not source_ancestor(api, before, after) or not source_ancestor(api, after, tip):
            return False
        expected = expected_ci_upgrade(trusted, before, after)
        if expected is None or proposed != expected:
            return False
        # Do not authorize code which was removed from the published branch while
        # the proof was being assembled. Consumer identities are checked at POST.
        return shared_default_tip(api) == (branch, tip)
    except (AdmissionError, EligibilityError):
        return False


def hosted_runner_only_change(trusted: bytes, proposed: bytes, path: str) -> bool:
    """Recognize only corresponding owned runner tokens; every other byte must match."""
    from .operational import WORKFLOW, OperationalError, _runners

    if (
        trusted == proposed
        or max(len(trusted), len(proposed)) > MAX_FILE_BYTES
        or WORKFLOW.fullmatch(path) is None
        or b"\0" in trusted
        or b"\0" in proposed
    ):
        return False
    try:
        before, after = trusted.decode("utf-8"), proposed.decode("utf-8")
        old, new = _runners(before, path), _runners(after, path)
    except (UnicodeDecodeError, OperationalError):
        return False
    old_lines, new_lines = before.splitlines(keepends=True), after.splitlines(keepends=True)
    if not old or len(old) != len(new) or len(old_lines) != len(new_lines):
        return False
    if (
        any(pin.kind != "runner" or pin.source != "github-hosted" for pin in (*old, *new))
        or len({pin.line for pin in old}) != len(old)
        or len({pin.line for pin in new}) != len(new)
    ):
        return False
    # Moving existing plain matrix entries is not a literal version upgrade.
    if [pin.name for pin in old] != [pin.name for pin in new] and sorted(
        pin.name for pin in old
    ) == sorted(pin.name for pin in new):
        return False
    literal = re.compile(
        r" *(?:(?:- +)?runs-on:[ \t]*|- +)"
        r"(?P<runner>(?:ubuntu|macos|windows)-[0-9]+(?:\.[0-9]+)?(?:-[a-z0-9]+)*)"
        r"[ \t]*(?:\#[^\r\n]*)?(?:\r?\n)?\Z"
    )
    expected = list(old_lines)
    changed = False
    for previous, current in zip(old, new, strict=True):
        if (
            previous.line != current.line
            or previous.name.split("-", 1)[0] != current.name.split("-", 1)[0]
        ):
            return False
        left, right = old_lines[previous.line - 1], new_lines[current.line - 1]
        left_match, right_match = literal.fullmatch(left), literal.fullmatch(right)
        if (
            left_match is None
            or right_match is None
            or left_match["runner"] != previous.name
            or right_match["runner"] != current.name
        ):
            return False
        start, end = left_match.span("runner")
        expected[previous.line - 1] = left[:start] + current.name + left[end:]
        changed |= previous.name != current.name
    return changed and "".join(expected).encode("utf-8") == proposed


def retry_hosted_runner_upgrade(
    api: GitHub,
    candidate: Candidate,
    trusted: bytes,
    proposed: bytes,
    implementation: str,
) -> bool:
    """Admit Renovate's runner-only CI change under the unchanged interface-1 policy."""
    from .admission import AdmissionError, parse_manifest

    if candidate.actor_login != "renovate[bot]" or candidate.actor_type != "Bot":
        return False
    try:
        before = file_bytes(api, ".github/automation.json", candidate.base)
        after = file_bytes(api, ".github/automation.json", candidate.head)
        if before is None or after != before:
            return False
        selected, policy = parse_manifest(before)
        if selected != revision(implementation):
            return False
        path = workflow_file(api.repository)
        if file_bytes(api, path, candidate.base) != trusted:
            return False
        policy_before = file_bytes(api, policy, candidate.base)
        if policy_before is None or file_bytes(api, policy, candidate.head) != policy_before:
            return False
        return hosted_runner_only_change(trusted, proposed, path)
    except (AdmissionError, EligibilityError):
        return False


def rerun_ready(
    api: GitHub,
    candidate: Candidate,
    ledger: dict[str, int],
    default_branch: str,
    *,
    implementation: str | None = None,
) -> None:
    """After age eligibility changes, rerun once; never repair or expand a workflow."""
    if candidate.head in ledger:
        return
    file_name = workflow_file(api.repository).split("/")[-1]
    run_head = candidate.head if candidate.scope == "merge_group" else candidate.source_head
    listing = api.get(f"actions/workflows/{file_name}/runs?head_sha={run_head}&per_page=20")
    runs = listing.get("workflow_runs", [])
    for run in runs[:20]:
        if not candidate_run(api, run, run_head):
            continue
        if candidate.scope == "pull_request":
            prs = run.get("pull_requests", [])
            if len(prs) != 1 or prs[0].get("number") != candidate.number:
                continue
            if prs[0].get("head", {}).get("sha") != candidate.source_head:
                continue
            if prs[0].get("base", {}).get("sha") != candidate.base:
                continue
        if run.get("status") != "completed" or run.get("conclusion") != "failure":
            return
        # A candidate cannot use the trusted service to launch a modified workflow.
        trusted = file_bytes(api, workflow_file(api.repository), default_branch)
        proposed = file_bytes(api, workflow_file(api.repository), candidate.head)
        if trusted is None or proposed is None:
            return
        if proposed != trusted and (
            implementation is None
            or not (
                retry_ci_upgrade(api, candidate, trusted, proposed, implementation)
                or retry_hosted_runner_upgrade(api, candidate, trusted, proposed, implementation)
            )
        ):
            return
        run_id = run.get("id")
        if type(run_id) is not int or run_id < 1:
            raise EligibilityError("candidate CI run has an invalid identifier")
        jobs = api.get(f"actions/runs/{run_id}/jobs?per_page=100")
        all_jobs = jobs.get("jobs", [])
        if jobs.get("total_count") != len(all_jobs) or len(all_jobs) > 100:
            return
        age_names = {
            "Lockfile release age",
            "Dependency eligibility",
            "lockfile-release-age",
            "Dependency eligibility / Dependency eligibility",
        }
        failed_age = False
        for job in all_jobs:
            name = str(job.get("name", ""))
            conclusion = job.get("conclusion")
            if job.get("status") != "completed" or conclusion not in {
                "success",
                "skipped",
                "failure",
            }:
                return
            if conclusion == "failure":
                if name in age_names:
                    failed_age = True
                elif name != "Required CI":
                    return
        if not failed_age:
            return
        if not still_current(api, candidate, default_branch):
            return
        api.post(f"actions/runs/{run_id}/rerun-failed-jobs", {})
        ledger[candidate.head] = run_id
        return


def report_status(api: GitHub, report: dict[str, Any], run_id: int) -> None:
    status = report.get("status")
    if not isinstance(status, str):
        raise EligibilityError("dependency policy returned no status")
    state = {"pass": "success", "waiting": "pending", "failure": "failure"}.get(status)
    if state is None:
        raise EligibilityError("dependency policy returned an unknown status")
    detail = {
        "pass": "Dependency policy satisfied; compatibility checks remain required",
        "waiting": (
            "Age eligibility waiting; automatic recheck after "
            f"{report.get('retry_at') or 'next schedule'}"
        ),
        "failure": "Dependency eligibility failed; inspect the trusted service evidence",
    }[status]
    api.post(
        f"statuses/{revision(report.get('candidate_sha'))}",
        {
            "state": state,
            "context": STATUS_CONTEXT,
            "description": detail[:140],
            "target_url": f"https://github.com/{api.repository}/actions/runs/{run_id}",
        },
    )


def service(
    api: GitHub,
    *,
    event_name: str,
    event: dict[str, Any],
    implementation: str,
    run_id: int,
    output: Path,
    now: datetime | None = None,
) -> dict[str, Any]:
    """One serialized trusted service run; policy failure still persists evidence."""
    now = now or datetime.now(UTC)
    revision(implementation)
    if event_name not in ALLOWED_EVENTS or run_id < 1:
        raise EligibilityError("dependency service requires a supported trusted event/run")
    repository = api.get("")
    default_branch = repository.get("default_branch")
    if not isinstance(default_branch, str) or not default_branch:
        raise EligibilityError("repository has no authoritative default branch")
    # GitHub establishes the executing workflow's identity; candidate event data
    # cannot substitute a different current run or an implementation revision.
    current_run = api.get(f"actions/runs/{run_id}")
    if (
        current_run.get("event") != event_name
        or current_run.get("path") != WORKFLOW_PATH
        or current_run.get("head_branch") != default_branch
        or current_run.get("repository", {}).get("full_name") != api.repository
        or service_revision(current_run) != implementation
    ):
        raise EligibilityError("current service run is not the trusted default-branch caller")
    head = revision(current_run.get("head_sha"))
    if api.get(f"compare/{head}...{quote(default_branch, safe='')}").get("status") not in {
        "ahead",
        "identical",
    }:
        raise EligibilityError("current service policy is outside trusted default-branch history")
    previous = previous_state(api, implementation, default_branch, now)
    dependency_state = (
        {"schema_version": 1, "first_seen": {}}
        if previous is None
        else previous["dependency_state"]
    )
    ledger = {} if previous is None else previous["reruns"]
    scan_cursor = 0 if previous is None else previous.get("scan_cursor", 0)
    initial_scan_cursor = scan_cursor
    evaluated_cursor: int | None = None
    candidates: list[Candidate] = []
    errors: list[str] = []
    if event_name == "workflow_run":
        source_id = event.get("workflow_run", {}).get("id")
        if type(source_id) is not int or source_id < 1:
            raise EligibilityError("workflow_run event has no valid source identifier")
        source = api.get(f"actions/runs/{source_id}")
        head = revision(source.get("head_sha"))
        if not candidate_run(api, source, head):
            raise EligibilityError(
                "workflow_run source is outside the read-only candidate CI allowlist"
            )
        if source.get("event") == "merge_group":
            candidates.append(queue_candidate(api, head, default_branch))
        else:
            prs = source.get("pull_requests", [])
            if len(prs) != 1 or type(prs[0].get("number")) is not int:
                raise EligibilityError("candidate CI run has no unique authoritative PR")
            candidate = pr_candidate(api, prs[0]["number"])
            # REST workflow runs may identify the PR source head rather than the
            # synthetic merge SHA. Only the live PR supplies the latter.
            if head not in {candidate.source_head, candidate.head}:
                raise EligibilityError("candidate CI run is stale relative to the live PR")
            candidates.append(candidate)
    else:
        prs = api.items("pulls?state=open&per_page=100&sort=updated&direction=desc")
        if not isinstance(prs, list) or len(prs) >= 100:
            raise EligibilityError("open PR scan exceeds its supported bound")
        numbers = [pr.get("number") for pr in prs]
        if any(type(number) is not int or number < 1 for number in numbers):
            raise EligibilityError("open PR metadata has an invalid PR number")
        ordered = sorted(prs, key=lambda pr: pr["number"])
        selected = (
            [pr for pr in ordered if pr["number"] > scan_cursor]
            + [pr for pr in ordered if pr["number"] <= scan_cursor]
        )[:MAX_SCHEDULED_PRS]
        if selected:
            scan_cursor = selected[-1]["number"]
        for pr in selected:
            try:
                candidates.append(pr_candidate(api, pr["number"]))
            except EligibilityError:
                errors.append(
                    f"PR {pr.get('number')}: merge candidate unavailable; next schedule rechecks"
                )
        # Queue candidates need rechecks even when their PR's merge SHA differs.
        file_name = workflow_file(api.repository).split("/")[-1]
        queue_runs = api.get(f"actions/workflows/{file_name}/runs?event=merge_group&per_page=20")
        for source in queue_runs.get("workflow_runs", [])[:20]:
            head = source.get("head_sha")
            if isinstance(head, str) and SHA.fullmatch(head) and candidate_run(api, source, head):
                try:
                    candidate = queue_candidate(api, head, default_branch)
                    if candidate not in candidates:
                        candidates.append(candidate)
                    break  # Reviewed max-build1 means only one live group.
                except EligibilityError:
                    continue  # Old groups are routinely removed; never waive them.
    reports: dict[str, Any] = {}
    for candidate in candidates:
        try:
            report = evaluate(api, candidate, dependency_state, now)
            dependency_state = report.pop("state")
            if event_name == "schedule" and candidate.scope == "pull_request":
                evaluated_cursor = candidate.number
                scan_cursor = candidate.number
            if not still_current(api, candidate, default_branch):
                errors.append(
                    f"PR {candidate.number}: candidate changed during evaluation; "
                    "next event rechecks"
                )
                continue
            previous_report = None if previous is None else previous["reports"].get(candidate.head)
            bindings = {
                "candidate_sha": candidate.head,
                "base_sha": candidate.base,
                "source_head_sha": candidate.source_head,
                "pull_request": candidate.number,
                "scope": candidate.scope,
            }
            ready_receipt = (
                isinstance(previous_report, dict)
                and previous_report.get("status") == "pass"
                and all(previous_report.get(key) == value for key, value in bindings.items())
                and report["status"] == "pass"
            )
            # A restarted waiter must see a completed artifact BEFORE it starts.
            # Reuse the proven prior run instead of racing this run's upload.
            receipt_run_id = (
                previous["run_id"] if ready_receipt and previous is not None else run_id
            )
            report_status(api, report, receipt_run_id)
            reports[candidate.head] = report
            if ready_receipt:
                rerun_ready(api, candidate, ledger, default_branch, implementation=implementation)
        except EligibilityError as error:
            errors.append(f"PR {candidate.number}: {error}")
            if api.requests >= 450 or time.monotonic() >= api.deadline - 30:
                if event_name == "schedule":
                    scan_cursor = (
                        initial_scan_cursor if evaluated_cursor is None else evaluated_cursor
                    )
                break
    payload = {
        "schema": 1,
        "repository": api.repository,
        "workflow_path": WORKFLOW_PATH,
        "run_id": run_id,
        "implementation_revision": implementation,
        "observed_at": now.isoformat(),
        "dependency_state": dependency_state,
        "reruns": ledger,
        "scan_cursor": scan_cursor,
        "reports": reports,
        "state_source": (
            "new-observation"
            if previous is None
            else (
                f"trusted-age-observations:{previous['run_id']}@{previous['implementation_revision']}"
                if previous.get("observations_migrated")
                else f"trusted-run:{previous['run_id']}"
            )
        ),
        "infrastructure_errors": errors,
    }
    validate_state(
        payload, repository=api.repository, run_id=run_id, implementation=implementation, now=now
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    serialized = json.dumps(payload, indent=2, sort_keys=True) + "\n"
    if len(serialized.encode()) > MAX_STATE_BYTES:
        raise EligibilityError("trusted service output exceeds its persisted-state budget")
    output.write_text(serialized, encoding="utf-8")
    return payload


def candidate_receipt(
    api: GitHub,
    head: str,
    implementation: str,
    default_branch: str,
    now: datetime,
) -> dict[str, Any] | None:
    """Accept only exact-SHA reports from a completed trusted service run."""
    statuses = api.items(f"commits/{head}/statuses?per_page=100")
    for status in statuses[:100]:
        if status.get("context") != STATUS_CONTEXT:
            continue
        target = status.get("target_url", "")
        match = re.fullmatch(
            rf"https://github\.com/{re.escape(api.repository)}/actions/runs/([0-9]+)", target
        )
        if not match or status.get("creator", {}).get("login") != "github-actions[bot]":
            return None
        run_id = int(match[1])
        run = api.get(f"actions/runs/{run_id}")
        if not trusted_run(api, run, implementation, default_branch):
            return None
        payload = validate_state(
            run_artifact(api, run_id),
            repository=api.repository,
            run_id=run_id,
            implementation=implementation,
            now=now,
        )
        report = payload["reports"].get(head)
        if not isinstance(report, dict) or report.get("candidate_sha") != head:
            return None
        expected = {"pass": "success", "waiting": "pending", "failure": "failure"}.get(
            report.get("status")
        )
        if expected != status.get("state"):
            return None
        return report
    return None


def wait_for_candidate(
    api: GitHub, head: str, implementation: str, timeout: int = 90
) -> dict[str, Any]:
    """Bounded read-only wait; age waiting never becomes a synthetic success."""
    revision(head)
    revision(implementation)
    if not 0 <= timeout <= 120:
        raise EligibilityError("candidate status wait must be between zero and 120 seconds")
    default_branch = api.get("").get("default_branch")
    if not isinstance(default_branch, str) or not default_branch:
        raise EligibilityError("repository has no authoritative default branch")
    deadline = time.monotonic() + timeout
    while True:
        try:
            report = candidate_receipt(api, head, implementation, default_branch, datetime.now(UTC))
        except EligibilityError:
            report = None
        if report is not None:
            return report
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return {
                "status": "waiting",
                "candidate_sha": head,
                "retry_eligible": True,
                "reason": (
                    "Trusted exact-candidate eligibility receipt is not ready; "
                    "service events and scheduled rechecks retry automatically"
                ),
            }
        time.sleep(min(10, remaining))


def summary(payload: dict[str, Any]) -> str:
    """Concise operational evidence, with age holds distinct from failures."""
    if "reports" not in payload:
        return (
            f"Dependency eligibility: {payload.get('status')} "
            f"for `{payload.get('candidate_sha')}`. {payload.get('reason', '')}\n"
        )
    lines = [
        "Dependency eligibility (metadata only)",
        "",
        f"First-seen evidence: {payload['state_source']}.",
        "",
    ]
    for head, report in payload["reports"].items():
        retry = (
            f"; next automatic recheck after {report['retry_at']}" if report.get("retry_at") else ""
        )
        lines.append(f"- PR #{report['pull_request']}, `{head}`: {report['status']}{retry}.")
    lines.extend(f"- Infrastructure: {error}." for error in payload["infrastructure_errors"])
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)
    for command in ("service", "wait"):
        child = subcommands.add_parser(command)
        child.add_argument("--repository", required=True, choices=sorted(REPOSITORIES))
        child.add_argument("--implementation-revision", required=True)
        child.add_argument("--summary", type=Path)
        if command == "service":
            child.add_argument("--event-name", required=True, choices=sorted(ALLOWED_EVENTS))
            child.add_argument("--event-path", type=Path, required=True)
            child.add_argument("--run-id", type=int, required=True)
            child.add_argument("--output", type=Path, required=True)
        else:
            child.add_argument("--candidate-sha", required=True)
            child.add_argument("--timeout", type=int, default=90)
    options = parser.parse_args(argv)
    try:
        token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
        if not token:
            raise EligibilityError("GitHub token is required for authoritative metadata")
        api = GitHub(options.repository, token)
        if options.command == "service":
            if options.event_path.stat().st_size > MAX_JSON_BYTES:
                raise EligibilityError("GitHub event payload exceeds its size budget")
            event = json.loads(options.event_path.read_text(encoding="utf-8"))
            payload = service(
                api,
                event_name=options.event_name,
                event=event,
                implementation=options.implementation_revision,
                run_id=options.run_id,
                output=options.output,
            )
            code = 0  # A policy hold/failure must retain trusted state and its report.
        else:
            payload = wait_for_candidate(
                api, options.candidate_sha, options.implementation_revision, options.timeout
            )
            code = 0 if payload.get("status") == "pass" else 1
        rendered = summary(payload)
        print(rendered, end="")
        if options.summary:
            with options.summary.open("a", encoding="utf-8") as output:
                output.write(rendered)
        return code
    except (EligibilityError, OSError, json.JSONDecodeError) as error:
        print(f"Dependency eligibility infrastructure failure: {error}")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
