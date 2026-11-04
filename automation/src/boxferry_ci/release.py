"""Shared, fail-closed release gates and one-crate publication mechanics.

The caller owns its protected release environment, native evidence, tag, GitHub
release, and binary/attestation hooks. This module never creates release state
other than the explicitly requested crates.io publication.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import time
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

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
    pull = read_pull(before.number)
    if not isinstance(pull, dict):
        raise ReleaseError("authoritative queue source PR is unavailable")
    base, head, title = pull.get("base"), pull.get("head"), pull.get("title")
    if (
        pull.get("state") != "open"
        or pull.get("number") != before.number
        or not isinstance(base, dict)
        or base.get("sha") != before.base
        or not isinstance(head, dict)
        or head.get("sha") != before.source_head
        or not isinstance(title, str)
        or not title
        or len(title) > 512
        or "\n" in title
        or "\r" in title
    ):
        raise ReleaseError(
            "queue source PR title is not bound to the live exact source head and base"
        )
    if resolve() != before:
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


def verify_gate(policy: dict[str, Any], needs: object, candidate_sha: str) -> None:
    if not SHA.fullmatch(candidate_sha):
        raise ReleaseError("candidate must be one full commit SHA")
    if not isinstance(needs, dict) or set(needs) != set(policy["required_jobs"]):
        raise ReleaseError("release needs must contain exactly every configured prerequisite")
    for name in policy["required_jobs"]:
        prerequisite = needs[name]
        if not isinstance(prerequisite, dict) or prerequisite.get("result") != "success":
            raise ReleaseError(f"release prerequisite did not succeed: {name}")
        outputs = prerequisite.get("outputs")
        if not isinstance(outputs, dict) or outputs.get("candidate_sha") != candidate_sha:
            raise ReleaseError(f"release prerequisite has no exact candidate binding: {name}")


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
        "command", choices=("gate", "prepare-crate", "publish-crate", "classify-queue")
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
                    candidate.base, candidate.head, candidate.number, candidate.source_head
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
            verify_gate(policy, read_json(args.needs_json), args.candidate_sha)
            write_outputs({"candidate_sha": args.candidate_sha})
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
