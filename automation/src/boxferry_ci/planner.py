"""One fail-closed planner for local, pull-request and complete queue candidates."""

from __future__ import annotations

import fnmatch
import hashlib
import json
import re
import tomllib
from dataclasses import dataclass
from pathlib import Path

from boxferry_ci.contracts import ContractError, exact_sha, relative_path
from boxferry_ci.markdown import unchanged_code
from boxferry_ci.policy import Policy
from boxferry_ci.runner import MAX_OUTPUT_BYTES, run

PLAN_SCHEMA = 3
MAX_PATHS = 5_000
_CODE = re.compile(
    rb"(?m)^[ \t]*(?:```|~~~)|^(?: {4,}| {0,3}\t)[ \t]*\S|`[^`\r\n]+`|<(?:code|pre)\b",
    re.IGNORECASE,
)


@dataclass(frozen=True)
class Comparison:
    event: str
    base_sha: str
    head_sha: str
    tested_sha: str


def git(root: Path, *args: str, optional: bool = False) -> bytes | None:
    result = run(
        ("git", "-c", "diff.external=", "-c", "core.fsmonitor=false", *args),
        cwd=root,
        stage="Git comparison",
        timeout_seconds=30,
        affected_input=str(root),
    )
    if result.status != "success":
        if optional and result.status == "failed":
            return None
        raise ContractError(f"git {args[0]}: {result.diagnostic.cause}; {result.stderr[:500]}")
    return result.stdout_bytes


def git_text(root: Path, *args: str) -> str:
    value = git(root, *args)
    if value is None:
        raise ContractError("Git unexpectedly returned no output")
    return value.decode("utf-8").strip()


def parse_diff(raw: bytes) -> list[tuple[str, tuple[str, ...]]]:
    if len(raw) > MAX_OUTPUT_BYTES or (raw and not raw.endswith(b"\0")):
        raise ContractError("Git comparison was truncated or exceeded its output bound")
    fields = raw.split(b"\0")[:-1]
    records: list[tuple[str, tuple[str, ...]]] = []
    index = 0
    while index < len(fields):
        try:
            status = fields[index].decode("ascii")
            index += 1
            count = 2 if re.fullmatch(r"[RC](?:100|[0-9]{1,2})", status) else 1
            if status not in {"A", "M", "D", "T"} and count != 2:
                raise ContractError(f"unsupported Git change status {status!r}")
            names = tuple(
                relative_path(field.decode("utf-8"), "changed path")
                for field in fields[index : index + count]
            )
            if len(names) != count:
                raise ContractError("Git change record is incomplete")
            records.append((status[0], names))
            index += count
        except (UnicodeError, IndexError) as error:
            raise ContractError("Git change paths or statuses are malformed") from error
        if sum(len(names) for _, names in records) > MAX_PATHS:
            raise ContractError("Git comparison exceeded its changed-path bound")
    return records


def changes_between(root: Path, base: str, head: str) -> list[tuple[str, tuple[str, ...]]]:
    raw = git(
        root,
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--name-status",
        "-z",
        "--find-renames",
        base,
        head,
        "--",
    )
    return parse_diff(raw or b"")


def regular_contents(root: Path, revision: str, path: str) -> bytes | None:
    listing = git(root, "ls-tree", "-z", revision, "--", path)
    if not listing:
        return None
    if not listing.startswith((b"100644 blob ", b"100755 blob ")):
        raise ContractError(f"non-regular Git path {path!r}")
    value = git(root, "show", f"{revision}:{path}")
    if value is None:
        raise ContractError(f"unreadable Git path {path!r}")
    return value


def _example_pages(policy: Policy) -> set[str]:
    if policy.documentation_examples_manifest is None:
        return set()
    # Locate the policy's repository, rather than assuming scripts/ depth.
    policy_root = Path(git_text(policy.path.parent, "rev-parse", "--show-toplevel"))
    with (policy_root / policy.documentation_examples_manifest).open("rb") as stream:
        manifest = tomllib.load(stream)
    examples = manifest.get("examples")
    if type(manifest.get("schema")) is not int or manifest["schema"] != 1:
        raise ContractError("documentation examples manifest has an unsupported schema")
    if not isinstance(examples, list):
        raise ContractError("documentation examples manifest has no example list")
    pages: set[str] = set()
    for example in examples:
        if not isinstance(example, dict) or not isinstance(example.get("pages"), list):
            raise ContractError("documentation example has no page list")
        for page in example["pages"]:
            pages.add(relative_path(page, "documentation example page"))
    return pages


def path_profiles(
    path: str,
    contents: tuple[bytes, ...],
    policy: Policy,
    pages: set[str],
    *,
    complete_pair: bool = False,
) -> set[str]:
    # A candidate policy and canonical lock cannot opt out of their own validation.
    if path.endswith("validation-policy.json") or path == "uv.lock" or path.endswith("/uv.lock"):
        return {"full"}
    if any(path.startswith(root) for root in policy.full_documentation_roots):
        return {"full"}
    configured = {
        profile
        for profile, patterns in policy.path_profiles.items()
        if any(fnmatch.fnmatchcase(path, pattern) for pattern in patterns)
    }
    extra = configured - {"prose"}
    if path in policy.executable_documentation_paths or path in pages:
        return {"executable-docs"} | extra
    if path.endswith(".md") and any(
        path.startswith(root) for root in policy.executable_documentation_roots
    ):
        return {"executable-docs"} | extra
    if path.startswith(policy.public_documentation_root) and path.endswith(".md"):
        selected = "executable-docs" if any(_CODE.search(value) for value in contents) else "prose"
        return {selected} | extra
    if "prose" in configured:
        # Only exact maintainer paths are eligible; a broad Markdown glob cannot
        # silently expand this exception to otherwise unknown files.
        if (
            not path.endswith(".md")
            or path not in policy.path_profiles.get("prose", ())
            or not complete_pair
            or not unchanged_code(contents)
        ):
            return {"full"}
    return configured or {"full"}


def classify(
    root: Path,
    comparison: Comparison,
    policy: Policy,
    *,
    local: bool = False,
) -> tuple[tuple[str, ...], list[str], str | None]:
    records = changes_between(root, comparison.base_sha, comparison.tested_sha)
    fingerprint: str | None = None
    worktree: set[str] = set()
    digest = hashlib.sha256()
    if local:
        for args in (
            ("diff", "--no-ext-diff", "--no-textconv", "--name-status", "-z", "HEAD", "--"),
            ("diff", "--no-ext-diff", "--no-textconv", "--cached", "--name-status", "-z", "--"),
        ):
            extra = parse_diff(git(root, *args) or b"")
            records.extend(extra)
            worktree.update(path for _, names in extra for path in names)
        untracked = git(root, "ls-files", "--others", "--exclude-standard", "-z") or b""
        for raw in untracked.split(b"\0"):
            if raw:
                path = relative_path(raw.decode("utf-8"), "untracked path")
                worktree.add(path)
                records.append(("A", (path,)))
        digest.update(json.dumps(records, ensure_ascii=True).encode())
    if not records or sum(len(names) for _, names in records) > MAX_PATHS:
        return (
            ("full",),
            ["empty or oversized comparison requires complete validation"],
            fingerprint,
        )
    pages = _example_pages(policy)
    profiles: set[str] = set()
    for kind, names in records:
        if kind == "T":
            return ("full",), ["Git object type changed"], fingerprint
        for position, path in enumerate(names):
            values: list[bytes] = []
            before_seen = False
            after_seen = False
            old_expected = kind != "A" and position == 0
            new_expected = kind != "D" and position == len(names) - 1
            if old_expected:
                old = regular_contents(root, comparison.base_sha, path)
                if old is not None:
                    values.append(old)
                    before_seen = True
                elif not local:
                    raise ContractError(f"missing baseline path {path!r}")
            if new_expected:
                new = regular_contents(root, comparison.tested_sha, path)
                if new is not None:
                    values.append(new)
                    after_seen = True
                elif not local:
                    raise ContractError(f"missing candidate path {path!r}")
            if local and path in worktree:
                indexed = git(root, "show", f":{path}", optional=True)
                if indexed is not None:
                    values.append(indexed)
                    digest.update(indexed)
                current = root / path
                if current.is_symlink() or (current.exists() and not current.is_file()):
                    raise ContractError(f"non-regular working-tree path {path!r}")
                if current.exists():
                    if current.stat().st_size > MAX_OUTPUT_BYTES:
                        raise ContractError(f"working-tree path too large {path!r}")
                    content = current.read_bytes()
                    values.append(content)
                    digest.update(path.encode())
                    digest.update(content)
            profiles.update(
                path_profiles(
                    path,
                    tuple(values),
                    policy,
                    pages,
                    complete_pair=kind == "M" and len(names) == 1 and before_seen and after_seen,
                )
            )
    if local:
        fingerprint = digest.hexdigest()
    if "full" in profiles or not profiles <= policy.profile_jobs.keys():
        return (
            ("full",),
            ["source, dependency, policy, or unknown scope requires complete validation"],
            fingerprint,
        )
    # The executable profile subsumes prose by policy invariant.
    if "executable-docs" in profiles:
        profiles.discard("prose")
    return (
        tuple(sorted(profiles)),
        ["known changed paths select the union of their profiles"],
        fingerprint,
    )


def verify_policy_revision(policy: Policy, policy_sha: str, comparison: Comparison) -> None:
    expected = exact_sha(policy_sha, "trusted policy SHA")
    if expected != comparison.base_sha:
        raise ContractError("PR/queue policy must come from the exact comparison base")
    root = Path(git_text(policy.path.parent, "rev-parse", "--show-toplevel"))
    relative = policy.path.relative_to(root).as_posix()
    committed = regular_contents(root, expected, relative)
    if committed != policy.path.read_bytes():
        raise ContractError("policy contents differ from the trusted base revision")
    if policy.documentation_examples_manifest is not None:
        manifest = policy.documentation_examples_manifest
        if regular_contents(root, expected, manifest) != (root / manifest).read_bytes():
            raise ContractError("example manifest differs from the trusted base revision")


def make_plan(
    root: Path,
    policy: Policy,
    *,
    event: str,
    base_sha: str = "",
    head_sha: str = "",
    tested_sha: str = "",
    policy_sha: str = "",
    base_ref: str = "origin/main",
    force_full: bool = False,
) -> dict[str, object]:
    root = root.resolve()
    tested = exact_sha(tested_sha or git_text(root, "rev-parse", "HEAD"), "tested SHA")
    fingerprint: str | None = None
    if event == "local":
        head = exact_sha(git_text(root, "rev-parse", "HEAD"), "HEAD")
        if tested != head:
            raise ContractError("local tested SHA must be HEAD")
        found_base = git(root, "merge-base", "HEAD", base_ref, optional=True)
        base = exact_sha(found_base.decode().strip(), "local base") if found_base else head
        comparison = Comparison(event, base, head, tested)
        if not found_base:
            force_full = True
    elif event in {"pull_request", "merge_group"}:
        comparison = Comparison(
            event,
            exact_sha(base_sha, "base SHA"),
            exact_sha(head_sha, "head SHA"),
            tested,
        )
        verify_policy_revision(policy, policy_sha, comparison)
    elif event in {"push", "workflow_dispatch", "workflow_call"}:
        comparison = Comparison(event, tested, tested, tested)
        force_full = True
    else:
        raise ContractError(f"unsupported validation event {event!r}")
    if force_full:
        profiles, reasons = (
            ("full",),
            ["this event or explicit complete mode requires full validation"],
        )
    else:
        try:
            if event in {"pull_request", "merge_group"}:
                if (
                    git(
                        root,
                        "merge-base",
                        "--is-ancestor",
                        comparison.head_sha,
                        tested,
                        optional=True,
                    )
                    is None
                ):
                    raise ContractError("tested revision does not contain the exact candidate head")
            profiles, reasons, fingerprint = classify(
                root, comparison, policy, local=event == "local"
            )
        except (OSError, UnicodeError, ValueError) as error:
            profiles, reasons = ("full",), [f"classification unavailable: {error}"]
    selected = policy.selected_jobs(profiles)
    return {
        "schema": PLAN_SCHEMA,
        "repository": policy.repository,
        "event": event,
        "base_sha": comparison.base_sha,
        "head_sha": comparison.head_sha,
        "tested_sha": comparison.tested_sha,
        "profile": profiles[0] if len(profiles) == 1 else "mixed",
        "profiles": list(profiles),
        "jobs": {job: job in selected for job in policy.jobs},
        "reasons": reasons,
        "local_fingerprint": fingerprint,
    }
