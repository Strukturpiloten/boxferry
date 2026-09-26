#!/usr/bin/env python3
"""Fail-closed, repository-configured local and CI validation selection.

This standard-library implementation may be vendored into Lens repositories;
product-specific paths and job names live in validation-policy.json. On a PR,
run the copy and policy from the trusted base revision, not from the PR head.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import resource
import subprocess
import sys
import time
import tomllib
from typing import Any


SHA = re.compile(r"[0-9a-f]{40}\Z")
FENCE = re.compile(rb"(?m)^[ \t]*(?:```|~~~)")
# CommonMark tabs advance to the next four-column stop, including after 1-3 spaces.
INDENTED_CODE = re.compile(rb"(?m)^(?: {4,}| {0,3}\t)[ \t]*\S")
INLINE_CODE = re.compile(rb"`[^`\r\n]+`|<(?:code|pre)\b", re.IGNORECASE)
MAX_DIFF_BYTES = 2_000_000
MAX_PATHS = 5_000
SCHEMA = 2


class PlanError(Exception):
    """Invalid contract or unsafe-to-classify input."""


def git(root: Path, *args: str, allow_failure: bool = False) -> bytes | None:
    command = ["git", "-c", "diff.external=", *args]
    result = subprocess.run(command, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    if result.returncode != 0:
        if allow_failure:
            return None
        raise PlanError(f"git {args[0]} failed: {result.stderr.decode('utf-8', 'replace')[:300]}")
    if len(result.stdout) > MAX_DIFF_BYTES:
        raise PlanError("Git comparison exceeded the bounded output limit")
    return result.stdout


def sha(value: str, label: str) -> str:
    if not SHA.fullmatch(value):
        raise PlanError(f"{label} must be an exact 40-character Git SHA")
    return value


def policy_at(root: Path) -> dict[str, Any]:
    try:
        policy = json.loads((root / "scripts/validation-policy.json").read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise PlanError(f"cannot load validation policy: {error}") from error
    required = {
        "schema", "repository", "jobs", "profile_jobs", "public_documentation_root",
        "full_documentation_roots", "executable_documentation_roots", "executable_documentation_paths",
        "documentation_examples_manifest",
        "local_commands",
    }
    if set(policy) != required or policy["schema"] != SCHEMA:
        raise PlanError("validation policy has an unsupported schema or key set")
    jobs = policy["jobs"]
    profile_jobs = policy["profile_jobs"]
    if (
        not isinstance(jobs, list) or not jobs or not all(isinstance(job, str) and job for job in jobs)
        or len(set(jobs)) != len(jobs) or not isinstance(profile_jobs, dict)
        or set(profile_jobs) != {"prose", "executable-docs"}
    ):
        raise PlanError("validation policy job sets are invalid")
    for profile in ("prose", "executable-docs"):
        selected = profile_jobs[profile]
        if (
            not isinstance(selected, list) or not selected
            or not all(isinstance(job, str) and job in jobs for job in selected)
            or len(set(selected)) != len(selected)
        ):
            raise PlanError(f"validation policy {profile} jobs are invalid")
    if not set(profile_jobs["prose"]).issubset(profile_jobs["executable-docs"]):
        raise PlanError("executable documentation may not select fewer jobs than prose")
    for key in ("public_documentation_root", "repository"):
        if not isinstance(policy[key], str) or not policy[key]:
            raise PlanError(f"validation policy {key} is invalid")
    manifest = policy["documentation_examples_manifest"]
    if manifest is not None and (not isinstance(manifest, str) or not manifest):
        raise PlanError("validation policy documentation_examples_manifest is invalid")
    for key in ("full_documentation_roots", "executable_documentation_roots", "executable_documentation_paths"):
        if not isinstance(policy[key], list) or not all(isinstance(path, str) for path in policy[key]):
            raise PlanError(f"validation policy {key} is invalid")
    commands = policy["local_commands"]
    if not isinstance(commands, dict) or set(commands) != {"prose", "executable-docs", "full"}:
        raise PlanError("validation policy local command profiles are invalid")
    for profile, calls in commands.items():
        if not isinstance(calls, list) or not calls:
            raise PlanError(f"validation policy {profile} has no local commands")
        if any(not isinstance(call, list) or not call or any(not isinstance(arg, str) or not arg for arg in call) for call in calls):
            raise PlanError(f"validation policy {profile} has an invalid local command")
    if commands["executable-docs"] != commands["full"] and any(
        call not in commands["executable-docs"] for call in commands["prose"]
    ):
        raise PlanError("executable documentation must include prose commands or run the full gate")
    return policy


def parse_diff(raw: bytes) -> list[tuple[str, tuple[str, ...]]]:
    fields = raw.split(b"\0")
    if not fields or fields[-1] != b"":
        raise PlanError("Git comparison has an incomplete NUL-delimited record")
    fields.pop()
    records: list[tuple[str, tuple[str, ...]]] = []
    position = 0
    while position < len(fields):
        try:
            status = fields[position].decode("ascii")
        except UnicodeError as error:
            raise PlanError("Git comparison has a non-ASCII status") from error
        position += 1
        kind = status[:1]
        if kind not in {"A", "D", "M", "R", "C", "T"} or (kind in {"R", "C"} and not status[1:].isdigit()):
            raise PlanError(f"unsupported Git change status {status!r}")
        count = 2 if kind in {"R", "C"} else 1
        if position + count > len(fields):
            raise PlanError("Git comparison has a truncated path record")
        try:
            paths = tuple(field.decode("utf-8") for field in fields[position : position + count])
        except UnicodeError as error:
            raise PlanError("Git comparison has a non-UTF-8 path") from error
        if any(not path or path.startswith("/") or ".." in Path(path).parts for path in paths):
            raise PlanError("Git comparison has an unsafe path")
        records.append((kind, paths))
        position += count
        if len(records) > MAX_PATHS:
            raise PlanError("Git comparison exceeded the path limit")
    return records


def changes_between(root: Path, base: str, head: str) -> list[tuple[str, tuple[str, ...]]]:
    raw = git(root, "diff", "--no-ext-diff", "--no-textconv", "--name-status", "-z", "--find-renames", base, head, "--")
    assert raw is not None
    return parse_diff(raw)


def blob(root: Path, revision: str, path: str) -> bytes | None:
    return git(root, "show", f"{revision}:{path}", allow_failure=True)


def regular_blob(root: Path, revision: str, path: str) -> bool:
    raw = git(root, "ls-tree", "-z", revision, "--", path, allow_failure=True)
    if not raw:
        return False
    entries = raw.split(b"\0")
    if len(entries) != 2 or entries[-1] != b"":
        return False
    return entries[0].startswith((b"100644 blob ", b"100755 blob "))


def index_blob(root: Path, path: str) -> bytes | None:
    raw = git(root, "ls-files", "--stage", "-z", "--", path)
    assert raw is not None
    if not raw:
        return None
    entries = raw.split(b"\0")
    if len(entries) != 2 or entries[-1] != b"":
        raise PlanError(f"ambiguous index entries for {path!r}")
    metadata, separator, listed_path = entries[0].partition(b"\t")
    if not separator or listed_path != path.encode() or not metadata.startswith((b"100644 ", b"100755 ")):
        raise PlanError(f"non-regular or unresolved index entry for {path!r}")
    if not metadata.endswith(b" 0"):
        raise PlanError(f"unmerged index entry for {path!r}")
    indexed = git(root, "show", f":{path}", allow_failure=True)
    if indexed is None:
        raise PlanError(f"unreadable index path {path!r}")
    return indexed


def example_pages(policy_root: Path, policy: dict[str, Any]) -> set[str]:
    if policy["documentation_examples_manifest"] is None:
        return set()
    with (policy_root / policy["documentation_examples_manifest"]).open("rb") as stream:
        manifest = tomllib.load(stream)
    if manifest.get("schema") != 1 or not isinstance(manifest.get("examples"), list):
        raise PlanError("documentation examples manifest has an unsupported schema")
    pages: set[str] = set()
    for item in manifest["examples"]:
        if not isinstance(item, dict) or not isinstance(item.get("pages"), list):
            raise PlanError("documentation example has no page list")
        for path in item["pages"]:
            if not isinstance(path, str):
                raise PlanError("documentation example page is not a path")
            pages.add(path)
    return pages


def path_profile(path: str, contents: list[bytes], policy: dict[str, Any], pages: set[str]) -> str:
    if path in policy["executable_documentation_paths"]:
        return "executable-docs"
    if not path.startswith(policy["public_documentation_root"]) or not path.endswith(".md"):
        return "full"
    if any(path.startswith(root) for root in policy["full_documentation_roots"]):
        return "full"
    if any(path.startswith(root) for root in policy["executable_documentation_roots"]):
        return "executable-docs"
    if path in pages or any(
        FENCE.search(content) or INDENTED_CODE.search(content) or INLINE_CODE.search(content)
        for content in contents
    ):
        return "executable-docs"
    return "prose"


def profile_for_changes(
    root: Path, base: str, head: str, records: list[tuple[str, tuple[str, ...]]],
    policy: dict[str, Any], policy_root: Path, *, local: bool,
) -> tuple[str, list[str]]:
    if not records:
        return "full", ["empty comparison: complete validation"]
    pages = example_pages(policy_root, policy)
    paths: dict[str, list[bytes]] = {}
    for kind, names in records:
        if kind == "T":
            return "full", ["Git object type changed: complete validation"]
        for index, path in enumerate(names):
            if path_profile(path, [], policy, pages) == "full":
                return "full", [f"code, policy or unknown path: {path!r}"]
            if path not in paths:
                paths[path] = []
            old_path = index == 0 and kind not in {"A"}
            new_path = index == len(names) - 1 and kind not in {"D"}
            if old_path and regular_blob(root, base, path):
                old = blob(root, base, path)
                if old is None:
                    return "full", [f"unreadable baseline path: {path!r}"]
                paths[path].append(old)
            elif old_path and not (kind == "M" and blob(root, base, path) is None):
                return "full", [f"non-regular or missing baseline path: {path!r}"]
            if new_path:
                if local:
                    indexed = index_blob(root, path)
                    if indexed is not None:
                        paths[path].append(indexed)
                    current = root / path
                    if current.is_symlink() or (current.exists() and not current.is_file()):
                        return "full", [f"non-regular working-tree path: {path!r}"]
                    if current.exists():
                        if current.stat().st_size > MAX_DIFF_BYTES:
                            return "full", [f"working-tree path exceeds classifier limit: {path!r}"]
                        paths[path].append(current.read_bytes())
                elif regular_blob(root, head, path):
                    new = blob(root, head, path)
                    if new is None:
                        return "full", [f"unreadable candidate path: {path!r}"]
                    paths[path].append(new)
                else:
                    return "full", [f"non-regular or missing candidate path: {path!r}"]
    kinds = {path_profile(path, contents, policy, pages) for path, contents in paths.items()}
    if "full" in kinds:
        return "full", ["code, policy, unknown or mixed-impact path: complete validation"]
    if "executable-docs" in kinds:
        return "executable-docs", ["public executable documentation: file checks and focused examples"]
    return "prose", ["public prose without executable blocks: file and offline-link checks"]


def local_changes(root: Path, base: str, head: str) -> tuple[list[tuple[str, tuple[str, ...]]], str]:
    records = changes_between(root, base, head)
    for arguments in (
        ("diff", "--cached", "--no-ext-diff", "--no-textconv", "--name-status", "-z", "--find-renames", "HEAD", "--"),
        ("diff", "--no-ext-diff", "--no-textconv", "--name-status", "-z", "--find-renames", "--"),
    ):
        raw = git(root, *arguments)
        assert raw is not None
        records += parse_diff(raw)
    unmerged = git(root, "ls-files", "--unmerged", "-z")
    if unmerged:
        raise PlanError("local index contains unresolved conflicts")
    untracked = git(root, "ls-files", "--others", "--exclude-standard", "-z")
    assert untracked is not None
    for raw_path in untracked.split(b"\0")[:-1]:
        try:
            records.append(("A", (raw_path.decode("utf-8"),)))
        except UnicodeError as error:
            raise PlanError("untracked path is not UTF-8") from error
    if len(records) > MAX_PATHS:
        raise PlanError("local comparison exceeded the path limit")
    digest = hashlib.sha256()
    digest.update(base.encode())
    digest.update(head.encode())
    for kind, names in records:
        digest.update(kind.encode() + b"\0")
        for path in names:
            digest.update(path.encode() + b"\0")
            index_entry = git(root, "ls-files", "--stage", "-z", "--", path)
            assert index_entry is not None
            digest.update(index_entry)
            current = root / path
            if current.is_symlink():
                digest.update(b"SYMLINK\0")
            elif current.is_file():
                with current.open("rb") as stream:
                    for chunk in iter(lambda: stream.read(65_536), b""):
                        digest.update(chunk)
            else:
                digest.update(b"ABSENT\0")
    return records, digest.hexdigest()


def make_plan(args: argparse.Namespace, policy: dict[str, Any]) -> dict[str, Any]:
    root = args.repository_root.resolve()
    policy_root = args.policy_root.resolve()
    event = args.event
    tested = sha(args.tested_sha or (git(root, "rev-parse", "HEAD") or b"").decode().strip(), "tested SHA")
    reason: list[str]
    fingerprint = None
    if event == "local":
        head = sha((git(root, "rev-parse", "HEAD") or b"").decode().strip(), "HEAD")
        merge_base = git(root, "merge-base", "HEAD", args.base_ref, allow_failure=True)
        if merge_base is None:
            base = head
            try:
                _, fingerprint = local_changes(root, base, head)
            except (OSError, ValueError, PlanError):
                fingerprint = None
            profile, reason = "full", ["local base is unavailable: complete validation"]
        else:
            base = sha(merge_base.decode().strip(), "local base")
            try:
                records, fingerprint = local_changes(root, base, head)
                profile, reason = profile_for_changes(root, base, head, records, policy, policy_root, local=True)
            except (OSError, ValueError, PlanError) as error:
                profile, reason = "full", [f"local classification unavailable: {error}"]
    elif event == "pull_request":
        base = sha(args.base_sha, "PR base")
        head = sha(args.head_sha, "PR head")
        if args.force_full:
            profile, reason = "full", ["trusted base has no classifier or complete mode was requested"]
        else:
            try:
                records = changes_between(root, base, head)
                profile, reason = profile_for_changes(root, base, head, records, policy, policy_root, local=False)
            except (OSError, ValueError, PlanError) as error:
                profile, reason = "full", [f"PR classification unavailable: {error}"]
    elif event in {"push", "workflow_dispatch", "workflow_call"}:
        base = head = tested
        profile, reason = "full", [f"{event} always requires complete validation"]
    else:
        raise PlanError(f"unsupported validation event {event!r}")
    if args.force_full and event == "local":
        profile, reason = "full", ["explicit complete mode was requested"]
    selected = set(policy["jobs"] if profile == "full" else policy["profile_jobs"][profile])
    return {
        "schema": SCHEMA, "repository": policy["repository"], "event": event,
        "base_sha": base, "head_sha": head, "tested_sha": tested,
        "profile": profile, "jobs": {job: job in selected for job in policy["jobs"]},
        "reasons": reason, "local_fingerprint": fingerprint,
    }


def verify_gate(plan: dict[str, Any], needs: dict[str, Any], policy: dict[str, Any], args: argparse.Namespace) -> list[tuple[str, str]]:
    expected_keys = {"schema", "repository", "event", "base_sha", "head_sha", "tested_sha", "profile", "jobs", "reasons", "local_fingerprint"}
    if not isinstance(plan, dict) or set(plan) != expected_keys or plan["schema"] != SCHEMA:
        raise PlanError("missing or unsupported validation plan")
    if plan["repository"] != policy["repository"] or plan["event"] != args.event:
        raise PlanError("validation plan repository/event mismatch")
    if plan["tested_sha"] != sha(args.tested_sha, "tested SHA"):
        raise PlanError("validation plan is not bound to the tested revision")
    if args.event == "pull_request":
        if plan["base_sha"] != sha(args.base_sha, "PR base") or plan["head_sha"] != sha(args.head_sha, "PR head"):
            raise PlanError("validation plan is not bound to the exact PR comparison")
    elif plan["profile"] != "full" or plan["base_sha"] != plan["tested_sha"] or plan["head_sha"] != plan["tested_sha"]:
        raise PlanError("non-PR validation must be complete at the tested revision")
    jobs = plan["jobs"]
    if not isinstance(jobs, dict) or set(jobs) != set(policy["jobs"]) or any(type(value) is not bool for value in jobs.values()):
        raise PlanError("validation plan has an unknown, missing or non-boolean job selection")
    profile = plan["profile"]
    if profile not in {"full", "prose", "executable-docs"}:
        raise PlanError("validation plan profile is unknown")
    expected_selected = set(policy["jobs"] if profile == "full" else policy["profile_jobs"][profile])
    if {job for job, selected in jobs.items() if selected} != expected_selected:
        raise PlanError("validation plan does not match its declared profile")
    if not isinstance(needs, dict) or set(needs) != {"validation-plan", *policy["jobs"]}:
        raise PlanError("aggregate job dependencies do not match policy")
    if needs["validation-plan"].get("result") != "success":
        raise PlanError("validation planning did not succeed")
    results: list[tuple[str, str]] = []
    failures: list[str] = []
    for job in policy["jobs"]:
        result = needs[job].get("result")
        results.append((job, str(result)))
        if jobs[job] and result != "success":
            failures.append(f"selected {job} concluded {result}")
        elif not jobs[job] and result not in {"skipped", "success"}:
            failures.append(f"unselected {job} concluded {result}")
    if failures:
        raise PlanError("; ".join(failures))
    return results


def require_worktree_local_target(root: Path) -> None:
    configured = os.environ.get("CARGO_TARGET_DIR")
    if not configured:
        return
    resolved_root = root.resolve()
    target = (resolved_root / configured).resolve()
    if target == resolved_root or not target.is_relative_to(resolved_root):
        raise PlanError(
            "CARGO_TARGET_DIR must be inside this worktree; unset it or choose a worktree-local target directory"
        )


def run_local(args: argparse.Namespace, policy: dict[str, Any]) -> int:
    require_worktree_local_target(args.repository_root)
    args.event = "local"
    args.base_sha = args.head_sha = args.tested_sha = ""
    args.force_full = args.full
    initial = make_plan(args, policy)
    if initial["local_fingerprint"] is None:
        raise PlanError("cannot bind local validation to a working-tree snapshot")
    profile = initial["profile"]
    if args.docs_only and profile != "prose":
        raise PlanError(f"docs-only task requires a prose-only plan; classified {profile}")
    print(f"Local validation profile: {profile}", flush=True)
    for reason in initial["reasons"]:
        print(f"  {reason}", flush=True)
    start = time.monotonic()
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    for command in policy["local_commands"][profile]:
        print("+ " + " ".join(command), flush=True)
        outcome = subprocess.run(command, cwd=args.repository_root, check=False)
        if outcome.returncode != 0:
            raise PlanError(f"local validation command failed ({outcome.returncode}): {command[0]}")
    final = make_plan(args, policy)
    if final != initial:
        raise PlanError("local source changed during validation; rerun against the new snapshot")
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    print(
        f"Local {profile} validation passed: elapsed_s={time.monotonic() - start:.2f} "
        f"child_user_cpu_s={after.ru_utime - before.ru_utime:.2f} "
        f"child_system_cpu_s={after.ru_stime - before.ru_stime:.2f} "
        f"max_child_rss_kib={after.ru_maxrss}",
        flush=True,
    )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    plan_parser = sub.add_parser("plan")
    gate_parser = sub.add_parser("gate")
    for command in (plan_parser, gate_parser):
        command.add_argument("--repository-root", type=Path, default=Path(__file__).resolve().parent.parent)
        command.add_argument("--policy-root", type=Path, default=Path(__file__).resolve().parent.parent)
        command.add_argument("--event", required=True)
        command.add_argument("--base-sha", default="")
        command.add_argument("--head-sha", default="")
        command.add_argument("--tested-sha", default="")
    plan_parser.add_argument("--base-ref", default="origin/main")
    plan_parser.add_argument("--force-full", action="store_true")
    plan_parser.add_argument("--github-output", type=Path)
    local_parser = sub.add_parser("run-local")
    local_parser.add_argument("--repository-root", type=Path, default=Path(__file__).resolve().parent.parent)
    local_parser.add_argument("--policy-root", type=Path, default=Path(__file__).resolve().parent.parent)
    local_parser.add_argument("--base-ref", default="origin/main")
    local_parser.add_argument("--full", action="store_true")
    local_parser.add_argument("--docs-only", action="store_true")
    gate_parser.add_argument("--plan-json", required=True)
    gate_parser.add_argument("--needs-json", required=True)
    args = parser.parse_args()
    try:
        policy = policy_at(args.policy_root)
        if args.command == "plan":
            plan = make_plan(args, policy)
            encoded = json.dumps(plan, sort_keys=True, separators=(",", ":"))
            if args.github_output:
                with args.github_output.open("a", encoding="utf-8") as stream:
                    stream.write(f"plan={encoded}\nprofile={plan['profile']}\n")
                    for job, selected in plan["jobs"].items():
                        stream.write(f"select_{job.replace('-', '_')}={str(selected).lower()}\n")
            print(encoded)
        elif args.command == "gate":
            plan = json.loads(args.plan_json)
            needs = json.loads(args.needs_json)
            results = verify_gate(plan, needs, policy, args)
            print(f"Validation plan {plan['profile']} for {plan['tested_sha']}")
            for job, result in results:
                print(f"{job}: {'required' if plan['jobs'][job] else 'optional'} / {result}")
        else:
            return run_local(args, policy)
    except (OSError, ValueError, PlanError) as error:
        print(f"Validation contract failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
