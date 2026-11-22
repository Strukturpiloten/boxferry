"""Shared, fail-closed release gates and one-crate publication mechanics.

The caller owns its protected release environment, native evidence, tag, GitHub
release, and binary/attestation hooks. The preparation helper changes only validated
local Git metadata. This module never creates remote release state other than explicitly
requested crates.io publication.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import select
import signal
import subprocess
import time
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

from boxferry_ci.bootstrap import BootstrapError, git_environment, run_git

SHA = re.compile(r"[0-9a-f]{40}\Z")
PACKAGE = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*\Z")
VERSION = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+\Z")
JOB = re.compile(r"[A-Za-z_][A-Za-z0-9_-]*\Z")
BREAKING_SUBJECT = re.compile(r"(?:feat|fix|perf|refactor|revert)(?:\([^)]+\))?!: .+\Z")
REPOSITORIES = frozenset(
    f"Strukturpiloten/{name}"
    for name in (
        "boxferry",
        "compose-lens",
        "podman-lens",
        "quadlet-lens",
        "docker-lens",
        "kubernetes-lens",
    )
)


class ReleaseError(ValueError):
    """Release evidence or publication policy is incomplete or inconsistent."""


@dataclass(frozen=True)
class QueueBinding:
    """The authoritative eligibility service's exact one-entry queue identity."""

    base: str
    head: str
    number: int
    source_head: str
    title: str = ""
    target_branch: str = ""
    repository: str = ""
    actor_login: str = ""
    actor_type: str = ""


def _queue_title(binding: QueueBinding, pull: object) -> str:
    """Validate just source identity/title; unrelated synthetic metadata may lag."""
    if not isinstance(pull, dict):
        raise ReleaseError("authoritative queue source PR is unavailable")
    base, head, title = pull.get("base"), pull.get("head"), pull.get("title")
    if (
        pull.get("state") != "open"
        or pull.get("number") != binding.number
        or not isinstance(base, dict)
        or not isinstance(head, dict)
        or head.get("sha") != binding.source_head
        or not isinstance(title, str)
        or not 1 <= len(title) <= 512
        or any(char in title for char in "\r\n\0")
        or (bool(binding.title) and title != binding.title)
    ):
        raise ReleaseError("queue source PR title is not bound to its live exact identity")
    if binding.target_branch:
        from .eligibility import REPOSITORIES as QUEUE_REPOSITORIES

        actor = pull.get("user")
        if (
            binding.repository not in QUEUE_REPOSITORIES
            or not binding.title
            or not binding.actor_login
            or binding.actor_type not in {"User", "Bot"}
            or base.get("ref") != binding.target_branch
            or not isinstance(base.get("repo"), dict)
            or base["repo"].get("full_name") != binding.repository
            or not isinstance(head.get("repo"), dict)
            or head["repo"].get("full_name") != binding.repository
            or not isinstance(actor, dict)
            or actor.get("login") != binding.actor_login
            or actor.get("type") != binding.actor_type
        ):
            raise ReleaseError("current proved queue source/target/author identity changed")
    elif (
        base.get("sha") != binding.base
        or binding.repository
        or binding.actor_login
        or binding.actor_type
    ):
        # Only the independently proved current queue contract can tolerate the
        # stale REST base SHA. Historical synthetic hints retain exact-base rules.
        raise ReleaseError("legacy queue source PR does not bind the exact base")
    return title


def queue_release_type(
    base_sha: str,
    candidate_sha: str,
    resolve: Callable[[], QueueBinding],
    read_pull: Callable[[int], object],
) -> str:
    """Preserve PR-title SemVer intent only for a fresh, exactly bound queue entry."""
    if not SHA.fullmatch(base_sha) or not SHA.fullmatch(candidate_sha):
        raise ReleaseError("queue classification requires full base and tested commit SHAs")
    before = resolve()
    if (
        before.base != base_sha
        or before.head != candidate_sha
        or not SHA.fullmatch(before.source_head)
        or type(before.number) is not int
        or before.number < 1
    ):
        raise ReleaseError("authoritative queue entry does not match the exact tested comparison")
    title = _queue_title(before, read_pull(before.number))
    if resolve() != before or _queue_title(before, read_pull(before.number)) != title:
        raise ReleaseError(
            "authoritative merge queue identity changed during SemVer classification"
        )
    return "major" if BREAKING_SUBJECT.fullmatch(title) else ""


def unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ReleaseError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def read_json(text: str) -> object:
    return json.loads(text, object_pairs_hook=unique_object)


def string_list(value: object, pattern: re.Pattern[str], label: str) -> list[str]:
    if (
        not isinstance(value, list)
        or not 1 <= len(value) <= 64
        or any(not isinstance(item, str) or not pattern.fullmatch(item) for item in value)
        or len(set(value)) != len(value)
    ):
        raise ReleaseError(f"{label} must be a nonempty unique list of valid names")
    return value


def load_policy(path: Path, repository: str) -> dict[str, Any]:
    policy = read_json(path.read_text(encoding="utf-8"))
    required = {"schema", "repository", "packages", "required_jobs"}
    optional = {"publication_enabled", "bootstrap_version"}
    if not isinstance(policy, dict) or not required <= policy.keys() <= required | optional:
        raise ReleaseError("release policy has missing or unknown fields")
    if type(policy["schema"]) is not int or policy["schema"] != 1:
        raise ReleaseError("unsupported release policy schema")
    if repository not in REPOSITORIES or policy["repository"] != repository:
        raise ReleaseError("release policy repository does not match the allowlisted caller")
    string_list(policy["packages"], PACKAGE, "packages")
    string_list(policy["required_jobs"], JOB, "required_jobs")
    if type(policy.get("publication_enabled", False)) is not bool:
        raise ReleaseError("publication_enabled must be a boolean")
    bootstrap = policy.get("bootstrap_version")
    if bootstrap is not None and (
        not isinstance(bootstrap, str) or not VERSION.fullmatch(bootstrap)
    ):
        raise ReleaseError("bootstrap_version must be an explicit stable version")
    return policy


def verify_gate(policy: dict[str, Any], needs: object, candidate_sha: str) -> str:
    if not SHA.fullmatch(candidate_sha):
        raise ReleaseError("candidate must be one full commit SHA")
    if not isinstance(needs, dict) or set(needs) != set(policy["required_jobs"]):
        raise ReleaseError("release needs must contain exactly every configured prerequisite")
    published_candidates: list[str] = []
    for name in policy["required_jobs"]:
        prerequisite = needs[name]
        if not isinstance(prerequisite, dict) or prerequisite.get("result") != "success":
            raise ReleaseError(f"release prerequisite did not succeed: {name}")
        outputs = prerequisite.get("outputs")
        if not isinstance(outputs, dict) or outputs.get("candidate_sha") != candidate_sha:
            raise ReleaseError(f"release prerequisite has no exact candidate binding: {name}")
        if "published_api_candidate_sha" in outputs:
            published_candidate = outputs["published_api_candidate_sha"]
            if (
                not isinstance(published_candidate, str)
                or not SHA.fullmatch(published_candidate)
                or published_candidate != candidate_sha
            ):
                raise ReleaseError(
                    f"release prerequisite has invalid published API binding: {name}"
                )
            published_candidates.append(published_candidate)
    if len(published_candidates) != 1:
        raise ReleaseError("release needs exactly one explicit published API candidate binding")
    return published_candidates[0]


def publication_inventory(
    metadata: object, policy: dict[str, Any], repository_root: Path
) -> tuple[dict[str, Any], dict[str, set[str]]]:
    if not isinstance(metadata, dict) or not isinstance(metadata.get("packages"), list):
        raise ReleaseError("invalid Cargo metadata")
    members = metadata.get("workspace_members")
    if not isinstance(members, list) or any(not isinstance(item, str) for item in members):
        raise ReleaseError("invalid Cargo workspace members")
    packages: dict[str, Any] = {}
    workspace_names: set[str] = set()
    root = repository_root.resolve(strict=True)
    for package in metadata["packages"]:
        if not isinstance(package, dict) or package.get("id") not in members:
            continue
        name = package.get("name")
        if not isinstance(name, str) or not PACKAGE.fullmatch(name) or name in workspace_names:
            raise ReleaseError("invalid or duplicate workspace package")
        workspace_names.add(name)
        if package.get("publish") == []:
            continue
        publish = package.get("publish")
        if publish is not None and publish != ["crates-io"]:
            raise ReleaseError(f"unsupported publication registry: {name}")
        version = package.get("version")
        if not isinstance(version, str) or not VERSION.fullmatch(version):
            raise ReleaseError(f"publication requires a stable package version: {name}")
        manifest_value = package.get("manifest_path")
        if not isinstance(manifest_value, str):
            raise ReleaseError(f"missing package manifest: {name}")
        manifest = Path(manifest_value).resolve(strict=True)
        if not manifest.is_relative_to(root) or not manifest.is_file():
            raise ReleaseError(f"package manifest escapes the candidate: {name}")
        packages[name] = package
    if set(packages) != set(policy["packages"]):
        raise ReleaseError("configured packages must exactly match publishable workspace packages")
    if len({package["version"] for package in packages.values()}) != 1:
        raise ReleaseError("shared release publication requires one lockstep workspace version")
    dependencies: dict[str, set[str]] = {}
    available: set[str] = set()
    for name in policy["packages"]:
        package = packages[name]
        declared = package.get("dependencies")
        if not isinstance(declared, list):
            raise ReleaseError(f"invalid Cargo dependencies: {name}")
        internal: set[str] = set()
        for dependency in declared:
            if not isinstance(dependency, dict) or dependency.get("kind") not in (
                None,
                "normal",
                "build",
                "dev",
            ):
                raise ReleaseError(f"invalid dependency kind: {name}")
            dependency_name = dependency.get("name")
            if dependency.get("path") is not None and dependency_name in workspace_names:
                if dependency_name not in packages:
                    raise ReleaseError(
                        f"publication depends on an unpublished workspace crate: {name}"
                    )
                internal.add(dependency_name)
        if not internal <= available:
            raise ReleaseError(
                f"configured publication order violates normal/build/dev dependencies: {name}"
            )
        dependencies[name] = internal
        available.add(name)
    return packages, dependencies


def registry_status(package: str, version: str) -> int:
    if not PACKAGE.fullmatch(package) or not VERSION.fullmatch(version):
        raise ReleaseError("invalid canonical registry package/version")
    url = f"https://crates.io/api/v1/crates/{package}/{version}"
    request = Request(url, headers={"User-Agent": "Strukturpiloten shared Rust release"})
    for attempt in range(4):
        try:
            with urlopen(request, timeout=60) as response:
                if response.geturl() != url:
                    raise ReleaseError("canonical registry request was redirected")
                status = response.status
        except HTTPError as error:
            if error.url != url:
                raise ReleaseError("canonical registry error response was redirected") from error
            status = error.code
        except (URLError, TimeoutError) as error:
            if attempt == 3:
                raise ReleaseError("canonical registry request failed") from error
            time.sleep(5)
            continue
        if status in (200, 404):
            return status
        if attempt == 3 or (status != 429 and status < 500):
            raise ReleaseError(f"unexpected canonical crates.io response: HTTP {status}")
        time.sleep(5)
    raise ReleaseError("canonical registry request exhausted its retry budget")


def publish_crate(repository_root: Path, package: str, version: str) -> None:
    if registry_status(package, version) == 200:
        print(f"{package} {version} already published; retaining existing publication")
        return
    subprocess.run(
        ["cargo", "publish", "--locked", "--package", package],
        cwd=repository_root,
        check=True,
        timeout=1800,
    )
    for _ in range(60):
        if registry_status(package, version) == 200:
            print(f"{package} {version} is visible on crates.io")
            return
        time.sleep(5)
    raise ReleaseError(f"timed out waiting for {package} {version} on crates.io")


def verify_bindings(repository_root: Path, candidate_sha: str, automation_sha: str) -> None:
    if not SHA.fullmatch(candidate_sha) or not SHA.fullmatch(automation_sha):
        raise ReleaseError("candidate and automation identities must be full commit SHAs")
    for root, expected in (
        (repository_root, candidate_sha),
        (Path(__file__).resolve().parents[3], automation_sha),
    ):
        actual = subprocess.check_output(
            ["git", "-C", str(root), "rev-parse", "HEAD"], text=True
        ).strip()
        if actual != expected:
            raise ReleaseError("held checkout does not match its exact source identity")
    if os.environ.get("GITHUB_SHA") != candidate_sha:
        raise ReleaseError("candidate does not match this caller run")


def _preparation_git(root: Path, *arguments: str) -> str:
    """Use the existing bounded runner without ambient Git overrides or hooks."""
    try:
        result = run_git(root, *arguments)
    except BootstrapError as error:
        raise ReleaseError("release preparation Git operation failed") from error
    if not isinstance(result, str):
        raise ReleaseError("release preparation Git operation did not return text")
    return result


def _preparation_checkout(
    root: Path, candidate_sha: str, branch_ref: str, tracking_ref: str
) -> tuple[str, dict[str, str]]:
    """Validate the held checkout before and after local metadata attachment."""
    if Path(_preparation_git(root, "rev-parse", "--show-toplevel")).resolve() != root:
        raise ReleaseError("release preparation needs the exact checkout root")
    if _preparation_git(root, "rev-parse", "--verify", "HEAD^{commit}") != candidate_sha:
        raise ReleaseError("release preparation checkout moved from the validated candidate")
    if _preparation_git(root, "status", "--porcelain=v1", "--untracked-files=all"):
        raise ReleaseError("release preparation requires clean source and index")
    current_ref = _preparation_git(root, "rev-parse", "--symbolic-full-name", "HEAD")
    if current_ref not in {"HEAD", branch_ref}:
        raise ReleaseError("release preparation is attached to another branch")
    refs = {}
    for line in _preparation_git(
        root, "for-each-ref", "--format=%(refname) %(objectname)", branch_ref, tracking_ref
    ).splitlines():
        name, value = line.split(" ", 1)
        if name in {branch_ref, tracking_ref}:
            if value != candidate_sha:
                raise ReleaseError("release preparation default branch moved or conflicts")
            refs[name] = value
    return current_ref, refs


def _preparation_command(root: Path, *arguments: str) -> tuple[str, ...]:
    """Apply the existing Git runner's isolation to the one stdin transaction."""
    return (
        "git",
        "--no-replace-objects",
        *(
            part
            for setting in (
                "core.hooksPath=/dev/null",
                "maintenance.auto=false",
                "gc.auto=0",
                "protocol.file.allow=never",
                "protocol.ext.allow=never",
            )
            for part in ("-c", setting)
        ),
        "-C",
        str(root),
        *arguments,
    )


def _preparation_exchange(
    process: subprocess.Popen[bytes], payload: bytes, reply: bytes, deadline: float
) -> None:
    """Exchange one fixed Git acknowledgement within a shared short deadline and cap."""
    if process.stdin is None or process.stdout is None or len(payload) > 4096:
        raise ReleaseError("release preparation Git transaction protocol failed")
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise ReleaseError("release preparation Git transaction timed out")
    if payload:
        if not select.select([], [process.stdin], [], remaining)[1]:
            raise ReleaseError("release preparation Git transaction timed out")
        if os.write(process.stdin.fileno(), payload) != len(payload):
            raise ReleaseError("release preparation Git transaction input was incomplete")
    response = bytearray()
    while not response.endswith(b"\n"):
        remaining = deadline - time.monotonic()
        if remaining <= 0 or not select.select([process.stdout], [], [], remaining)[0]:
            raise ReleaseError("release preparation Git transaction timed out")
        chunk = os.read(process.stdout.fileno(), 1)
        if not chunk or len(response) >= 4096:
            raise ReleaseError(
                "release preparation Git symref transaction failed or is unsupported"
            )
        response.extend(chunk)
    if response != reply:
        raise ReleaseError("release preparation Git symref transaction failed or is unsupported")


def _prepared_read(
    root: Path, deadline: float, *arguments: str
) -> subprocess.CompletedProcess[bytes]:
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise ReleaseError("release preparation Git transaction timed out")
    result = subprocess.run(
        _preparation_command(root, *arguments),
        env=git_environment(),
        capture_output=True,
        timeout=remaining,
        check=False,
    )
    if result.returncode not in {0, 1} or len(result.stdout) + len(result.stderr) > 4096:
        raise ReleaseError("release preparation locked reference verification failed")
    return result


def _attach_preparation_head(
    root: Path, candidate_sha: str, branch_ref: str, tracking_ref: str, refs: dict[str, str]
) -> None:
    """Let Git lock/verify refs, inspect raw detached HEAD under those locks, then commit."""
    commands = ["start"]
    commands.append(
        f"verify {branch_ref} {candidate_sha}"
        if branch_ref in refs
        else f"create {branch_ref} {candidate_sha}"
    )
    if tracking_ref in refs:
        commands.append(f"verify {tracking_ref} {candidate_sha}")
    commands.extend([f"symref-update HEAD {branch_ref} oid {candidate_sha}", "prepare"])
    payload = ("\n".join(commands) + "\n").encode()
    deadline = time.monotonic() + 10
    process = None
    committed = False

    def cancelled(_number: int, _frame: object) -> None:
        raise ReleaseError("release preparation Git transaction cancelled")

    previous = signal.signal(signal.SIGTERM, cancelled)
    try:
        process = subprocess.Popen(
            _preparation_command(root, "update-ref", "--stdin", "--no-deref"),
            env=git_environment(),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            bufsize=0,
        )
        if process.stdin is None:
            raise ReleaseError("release preparation Git transaction has no input")
        os.set_blocking(process.stdin.fileno(), False)
        _preparation_exchange(process, payload, b"start: ok\n", deadline)
        _preparation_exchange(process, b"", b"prepare: ok\n", deadline)
        # Expected-OID symref-update also accepts a symbolic HEAD at that OID.
        # The prepared transaction owns HEAD's lock: check its raw type before commit.
        raw = _prepared_read(root, deadline, "symbolic-ref", "--quiet", "--no-recurse", "HEAD")
        oid = _prepared_read(root, deadline, "rev-parse", "--verify", "HEAD^{commit}")
        branch = _prepared_read(
            root,
            deadline,
            "for-each-ref",
            "--format=%(refname) %(objectname) %(symref)",
            branch_ref,
        )
        expected = f"{branch_ref} {candidate_sha} " if branch_ref in refs else ""
        if (
            raw.returncode != 1
            or raw.stdout
            or oid.returncode != 0
            or oid.stdout.strip() != candidate_sha.encode()
            or branch.returncode != 0
            or branch.stdout.decode().rstrip("\n") != expected
        ):
            raise ReleaseError(
                "release preparation HEAD or default reference changed before attachment"
            )
        _preparation_exchange(process, b"commit\n", b"commit: ok\n", deadline)
        process.stdin.close()
        if process.wait(timeout=max(0.001, deadline - time.monotonic())) != 0:
            raise ReleaseError("release preparation Git transaction failed")
        committed = True
    except (OSError, subprocess.SubprocessError) as error:
        raise ReleaseError("release preparation Git transaction failed") from error
    finally:
        # EOF after start without commit also aborts Git's native transaction.
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        try:
            if process is not None:
                if process.stdin is not None and not process.stdin.closed:
                    if not committed and process.poll() is None:
                        try:
                            os.write(process.stdin.fileno(), b"abort\n")
                        except OSError:
                            pass
                    try:
                        process.stdin.close()
                    except OSError:
                        pass
                try:
                    process.wait(timeout=1)
                except subprocess.TimeoutExpired:
                    try:
                        process.terminate()
                    except ProcessLookupError:
                        pass
                    try:
                        process.wait(timeout=1)
                    except subprocess.TimeoutExpired:
                        try:
                            process.kill()
                        except ProcessLookupError:
                            pass
                        process.wait(timeout=1)
                if process.stdout is not None:
                    process.stdout.close()
        finally:
            signal.signal(signal.SIGTERM, previous)


def prepare_branch(repository_root: Path, candidate_sha: str, default_branch: str) -> None:
    """Attach a clean exact candidate locally, without checkout, reset, fetch or remote writes.

    Existing branch/tracking tips must agree with the admitted candidate. A missing
    local branch and HEAD attachment commit in one expected-value Git transaction.
    While Git holds its prepared locks, recheck raw HEAD identity before committing.
    Recheck all source/ref conditions before allowing release-plz to run.
    """
    if not SHA.fullmatch(candidate_sha) or not default_branch or default_branch.startswith("-"):
        raise ReleaseError("release preparation needs an exact candidate and default branch")
    branch_ref = f"refs/heads/{default_branch}"
    if (
        os.environ.get("GITHUB_REPOSITORY") not in REPOSITORIES
        or os.environ.get("GITHUB_EVENT_NAME") not in {"push", "workflow_dispatch"}
        or os.environ.get("GITHUB_REF") != branch_ref
        or os.environ.get("GITHUB_SHA") != candidate_sha
    ):
        raise ReleaseError("release preparation requires the exact default-branch event")
    root = repository_root.resolve(strict=True)
    _preparation_git(root, "check-ref-format", branch_ref)
    _preparation_git(root, "check-ref-format", "--branch", default_branch)
    tracking_ref = f"refs/remotes/origin/{default_branch}"
    current_ref, refs = _preparation_checkout(root, candidate_sha, branch_ref, tracking_ref)
    if current_ref == "HEAD":
        _attach_preparation_head(root, candidate_sha, branch_ref, tracking_ref, refs)
    current_ref, refs = _preparation_checkout(root, candidate_sha, branch_ref, tracking_ref)
    if current_ref != branch_ref or refs.get(branch_ref) != candidate_sha:
        raise ReleaseError("release preparation did not retain the exact attached default branch")


def verify_publication_context(policy: dict[str, Any], candidate_sha: str) -> None:
    branch = os.environ.get("GITHUB_REF")
    default_branch = os.environ.get("BOXFERRY_DEFAULT_BRANCH")
    if (
        os.environ.get("GITHUB_EVENT_NAME") != "workflow_dispatch"
        or not default_branch
        or branch != f"refs/heads/{default_branch}"
        or os.environ.get("GITHUB_SHA") != candidate_sha
    ):
        raise ReleaseError("publication requires an explicit exact-default-branch release dispatch")
    if not policy.get("publication_enabled", False):
        raise ReleaseError("this product is validation-only; publication is not enabled")


def write_outputs(values: dict[str, str]) -> None:
    if output := os.environ.get("GITHUB_OUTPUT"):
        with Path(output).open("a", encoding="utf-8") as stream:
            for key, value in values.items():
                stream.write(f"{key}={value}\n")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "command",
        choices=("gate", "prepare-crate", "publish-crate", "classify-queue", "prepare-branch"),
    )
    parser.add_argument("--repository-root", type=Path, required=True)
    parser.add_argument("--config", type=Path)
    parser.add_argument("--candidate-sha", required=True)
    parser.add_argument("--automation-sha", required=True)
    parser.add_argument("--needs-json")
    parser.add_argument("--package")
    parser.add_argument("--validation-sha")
    parser.add_argument("--validation-result")
    parser.add_argument("--validation-only", choices=("true", "false"), default="true")
    parser.add_argument("--bootstrap", choices=("true", "false"), default="false")
    parser.add_argument("--queue-base-sha")
    parser.add_argument("--default-branch")
    args = parser.parse_args(argv)
    try:
        verify_bindings(args.repository_root, args.candidate_sha, args.automation_sha)
        if args.command == "prepare-branch":
            if not args.default_branch:
                raise ReleaseError("release preparation requires the default branch")
            prepare_branch(args.repository_root, args.candidate_sha, args.default_branch)
            verify_bindings(args.repository_root, args.candidate_sha, args.automation_sha)
            return 0
        if args.command == "classify-queue":
            if (
                os.environ.get("GITHUB_EVENT_NAME") != "merge_group"
                or not args.queue_base_sha
                or not args.default_branch
                or not os.environ.get("GH_TOKEN")
            ):
                raise ReleaseError(
                    "queue classification needs the exact event and read-only API access"
                )
            # One shared authoritative proof: do not infer source PRs from branch
            # names, candidate subjects, or heuristic commit-associated PR lists.
            from boxferry_ci.eligibility import EligibilityError, GitHub, queue_candidate

            api = GitHub(os.environ.get("GITHUB_REPOSITORY", ""), os.environ["GH_TOKEN"])

            def resolve() -> QueueBinding:
                candidate = queue_candidate(api, args.candidate_sha, args.default_branch)
                return QueueBinding(
                    candidate.base,
                    candidate.head,
                    candidate.number,
                    candidate.source_head,
                    candidate.title,
                    candidate.queue_target,
                    api.repository if candidate.queue_target else "",
                    candidate.actor_login if candidate.queue_target else "",
                    candidate.actor_type if candidate.queue_target else "",
                )

            try:
                release_type = queue_release_type(
                    args.queue_base_sha,
                    args.candidate_sha,
                    resolve,
                    lambda number: api.get(f"pulls/{number}"),
                )
            except EligibilityError as error:
                raise ReleaseError(
                    f"authoritative queue SemVer metadata unavailable: {error}"
                ) from error
            write_outputs({"release_type": release_type})
            return 0
        if args.config is None:
            raise ReleaseError("release operations require an explicit typed policy")
        policy = load_policy(args.config, os.environ.get("GITHUB_REPOSITORY", ""))
        if args.command == "gate":
            if args.needs_json is None:
                raise ReleaseError("release gate needs exact prerequisite results")
            published_candidate = verify_gate(
                policy, read_json(args.needs_json), args.candidate_sha
            )
            write_outputs(
                {
                    "candidate_sha": args.candidate_sha,
                    "published_api_candidate_sha": published_candidate,
                }
            )
            return 0
        if args.validation_sha != args.candidate_sha or args.validation_result != "success":
            raise ReleaseError(
                "publication preparation needs the complete exact-candidate release gate"
            )
        metadata = read_json(
            subprocess.check_output(
                ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
                cwd=args.repository_root,
                text=True,
                timeout=120,
            )
        )
        packages, dependencies = publication_inventory(metadata, policy, args.repository_root)
        if args.package not in packages:
            raise ReleaseError("package is outside the configured publication inventory")
        version = packages[args.package]["version"]
        if args.bootstrap == "true" and policy.get("bootstrap_version") != version:
            raise ReleaseError(
                "temporary bootstrap token is restricted to its configured initial version"
            )
        if args.validation_only == "true":
            if args.command == "publish-crate":
                raise ReleaseError("validation-only mode cannot invoke publication")
            write_outputs({"version": version, "published": "false", "bootstrap": args.bootstrap})
            return 0
        verify_publication_context(policy, args.candidate_sha)
        for dependency in sorted(dependencies[args.package]):
            if registry_status(dependency, packages[dependency]["version"]) != 200:
                raise ReleaseError(f"workspace dependency is not yet published: {dependency}")
        if args.command == "prepare-crate":
            published = str(registry_status(args.package, version) == 200).lower()
            write_outputs({"version": version, "published": published, "bootstrap": args.bootstrap})
            return 0
        if not os.environ.get("CARGO_REGISTRY_TOKEN"):
            raise ReleaseError("publication requires a fresh narrowly scoped registry credential")
        publish_crate(args.repository_root, args.package, version)
        return 0
    except (ReleaseError, OSError, json.JSONDecodeError, subprocess.SubprocessError) as error:
        parser.exit(1, f"release contract failed: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
