"""Resolve dependency admission from an exact trusted base, independently of an upgrade.

A PR may propose shared automation B while the trusted service still runs A. This
bootstrap reads A from the base Git object and never describes A's receipt as B's
policy approval. It runs before uv installation and therefore uses only stdlib.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from collections.abc import Callable
from dataclasses import asdict, dataclass
from pathlib import Path, PurePosixPath
from typing import Literal, cast

SHARED_REPOSITORY = "Strukturpiloten/boxferry"
CALLER_REPOSITORIES = frozenset(
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
ADMISSION_EVENTS = frozenset({"pull_request", "merge_group"})
FULL_EVENTS = frozenset({"push", "workflow_dispatch", "workflow_call"})
MANIFEST_PATH = ".github/automation.json"
MANIFEST_KEYS = frozenset({"schema", "repository", "revision", "policy", "interface"})
MAX_MANIFEST_BYTES = 8192
MAX_POLICY_BYTES = 65536
SHA = re.compile(r"[0-9a-f]{40}")
GitReader = Callable[[Path, tuple[str, ...], int], bytes]


class AdmissionError(ValueError):
    """A trusted policy or its immutable source could not be established."""


@dataclass(frozen=True)
class AdmissionContext:
    """Explicit caller facts; event-specific bases come from GitHub's context."""

    event: str
    repository: str
    candidate_sha: str
    automation_sha: str
    policy_sha: str
    tested_sha: str
    pr_base_sha: str = ""
    queue_base_sha: str = ""


@dataclass(frozen=True)
class AdmissionSelection:
    """The policy source a reader must use, or an explicit non-admission event."""

    status: Literal["required", "not-applicable"]
    candidate_sha: str
    policy_sha: str
    proposed_automation_sha: str
    policy_automation_sha: str | None = None
    policy_path: str | None = None

    def outputs(self) -> dict[str, str]:
        return {
            "required": "true" if self.status == "required" else "false",
            "candidate_sha": self.candidate_sha,
            "policy_sha": self.policy_sha,
            "proposed_automation_sha": self.proposed_automation_sha,
            "policy_automation_sha": self.policy_automation_sha or "",
            "policy_path": self.policy_path or "",
        }


def _revision(value: str, label: str) -> str:
    if not SHA.fullmatch(value):
        raise AdmissionError(f"{label} must be an exact lowercase 40-character commit SHA")
    return value


def validate_context(context: AdmissionContext) -> bool:
    """Return whether admission is required after validating all identity fields."""
    if context.repository not in CALLER_REPOSITORIES:
        raise AdmissionError("caller repository is outside the reviewed automation allowlist")
    for label, value in (
        ("candidate SHA", context.candidate_sha),
        ("proposed automation SHA", context.automation_sha),
        ("policy SHA", context.policy_sha),
        ("GitHub tested SHA", context.tested_sha),
    ):
        _revision(value, label)
    if context.candidate_sha != context.tested_sha:
        raise AdmissionError("candidate SHA does not match GitHub's exact tested SHA")
    if context.event == "pull_request":
        expected = _revision(context.pr_base_sha, "GitHub pull-request base SHA")
    elif context.event == "merge_group":
        expected = _revision(context.queue_base_sha, "GitHub merge-group base SHA")
    elif context.event in FULL_EVENTS:
        expected = context.candidate_sha
    else:
        raise AdmissionError(f"event {context.event!r} cannot select dependency admission policy")
    if context.policy_sha != expected:
        raise AdmissionError("policy SHA does not match the independently trusted event revision")
    return context.event in ADMISSION_EVENTS


def read_git(root: Path, arguments: tuple[str, ...], limit: int) -> bytes:
    """Read local immutable Git data without inherited Git overrides or remote access."""
    environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    environment.update(
        GIT_CONFIG_NOSYSTEM="1",
        GIT_CONFIG_GLOBAL=os.devnull,
        GIT_CONFIG_SYSTEM=os.devnull,
        GIT_TERMINAL_PROMPT="0",
        GIT_NO_REPLACE_OBJECTS="1",
    )
    command = [
        "git",
        "--no-replace-objects",
        "--literal-pathspecs",
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "core.fsmonitor=false",
        "-C",
        str(root),
        *arguments,
    ]
    try:
        result = subprocess.run(
            command, env=environment, capture_output=True, check=False, timeout=15
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise AdmissionError(
            f"cannot read trusted Git data during {arguments[0]}: {error}"
        ) from error
    if result.returncode:
        raise AdmissionError(
            f"trusted Git {arguments[0]} read failed; check the exact checkout and tracked policy"
        )
    if len(result.stdout) > limit:
        raise AdmissionError(f"trusted Git {arguments[0]} output exceeds its size budget")
    return result.stdout


def _head(root: Path, reader: GitReader) -> str:
    try:
        value = reader(root, ("rev-parse", "HEAD"), 128).decode("ascii").strip()
    except UnicodeDecodeError as error:
        raise AdmissionError("checkout HEAD is not an ASCII revision") from error
    return _revision(value, "checkout HEAD")


def _tracked_blob(root: Path, revision: str, path: str, reader: GitReader, limit: int) -> bytes:
    tree = reader(root, ("ls-tree", "-z", revision, "--", path), 1024)
    if not tree:
        raise AdmissionError(
            f"trusted base lacks {path}; merge the service/manifest prerequisite before the waiter"
        )
    if not tree.endswith(b"\0") or tree.count(b"\0") != 1:
        raise AdmissionError("trusted policy path must identify exactly one tracked file")
    try:
        header, name = tree[:-1].split(b"\t", 1)
        mode, kind, object_id = header.split(b" ")
        object_sha = object_id.decode("ascii")
    except (ValueError, UnicodeDecodeError) as error:
        raise AdmissionError("trusted policy Git tree entry is malformed") from error
    if mode not in {b"100644", b"100755"} or kind != b"blob" or name != path.encode("utf-8"):
        raise AdmissionError(
            "trusted policy path must be a regular tracked file, not a link or tree"
        )
    _revision(object_sha, "tracked policy blob")
    try:
        size = int(reader(root, ("cat-file", "-s", object_sha), 32))
    except ValueError as error:
        raise AdmissionError("trusted policy blob size is malformed") from error
    if not 0 <= size <= limit:
        raise AdmissionError(f"trusted {path} exceeds its {limit}-byte size budget")
    content = reader(root, ("cat-file", "blob", object_sha), limit)
    if len(content) != size:
        raise AdmissionError("trusted policy blob changed size while being read")
    return content


def _unique_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise AdmissionError(f"automation manifest repeats key {key!r}")
        result[key] = value
    return result


def parse_manifest(content: bytes) -> tuple[str, str]:
    """Validate the interface-1 data contract; no candidate manifest is consulted."""
    if len(content) > MAX_MANIFEST_BYTES:
        raise AdmissionError("automation manifest exceeds its size budget")
    try:
        value: object = json.loads(content.decode("utf-8"), object_pairs_hook=_unique_object)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise AdmissionError("trusted automation manifest is not valid UTF-8 JSON") from error
    if not isinstance(value, dict) or set(value) != MANIFEST_KEYS:
        raise AdmissionError("automation manifest must contain exactly the interface-1 keys")
    data = cast("dict[str, object]", value)
    if type(data["schema"]) is not int or data["schema"] != 1:
        raise AdmissionError("automation manifest schema must be integer 1")
    if type(data["interface"]) is not int or data["interface"] != 1:
        raise AdmissionError("automation manifest interface must be integer 1")
    if data["repository"] != SHARED_REPOSITORY:
        raise AdmissionError("automation manifest must select Strukturpiloten/boxferry")
    revision = data["revision"]
    if not isinstance(revision, str):
        raise AdmissionError("automation manifest revision must be a commit SHA string")
    _revision(revision, "trusted automation revision")
    policy = data["policy"]
    if (
        not isinstance(policy, str)
        or not policy
        or len(policy.encode("utf-8")) > 512
        or "\\" in policy
        or any(ord(character) < 32 or ord(character) == 127 for character in policy)
        or PurePosixPath(policy).is_absolute()
        or any(part in {"", ".", "..", ".git"} for part in policy.split("/"))
        or not policy.endswith(".json")
    ):
        raise AdmissionError("manifest policy must be a safe repository-relative JSON path")
    return revision, policy


def resolve_admission(
    context: AdmissionContext,
    implementation_root: Path,
    policy_root: Path,
    *,
    reader: GitReader = read_git,
) -> AdmissionSelection:
    """Bind B's setup code to A's admission authority from the exact trusted base."""
    required = validate_context(context)
    if _head(implementation_root, reader) != context.automation_sha:
        raise AdmissionError("proposed automation checkout does not match its immutable SHA")
    if not required:
        return AdmissionSelection(
            "not-applicable", context.candidate_sha, context.policy_sha, context.automation_sha
        )
    if _head(policy_root, reader) != context.policy_sha:
        raise AdmissionError("trusted policy checkout does not match its exact event base SHA")
    manifest = _tracked_blob(
        policy_root, context.policy_sha, MANIFEST_PATH, reader, MAX_MANIFEST_BYTES
    )
    revision, policy = parse_manifest(manifest)
    _tracked_blob(policy_root, context.policy_sha, policy, reader, MAX_POLICY_BYTES)
    return AdmissionSelection(
        "required",
        context.candidate_sha,
        context.policy_sha,
        context.automation_sha,
        revision,
        policy,
    )


def _summary(selection: AdmissionSelection) -> str:
    lines = ["### Dependency admission policy", ""]
    if selection.status == "required":
        lines.extend(
            [
                f"Reader policy: `{SHARED_REPOSITORY}@{selection.policy_automation_sha}`.",
                f"Trusted consumer base: `{selection.policy_sha}`.",
                f"Exact tested candidate: `{selection.candidate_sha}`.",
                f"Proposed shared code `{selection.proposed_automation_sha}` "
                "is validated separately; "
                "it does not replace admission policy before merge.",
            ]
        )
    else:
        lines.append(
            "Age admission is not applicable to this complete validation event. "
            "Integrity, audit and compatibility checks remain required."
        )
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--event-name", required=True)
    parser.add_argument("--candidate-sha", required=True)
    parser.add_argument("--automation-sha", required=True)
    parser.add_argument("--policy-sha", required=True)
    parser.add_argument("--tested-sha", required=True)
    parser.add_argument("--pr-base-sha", default="")
    parser.add_argument("--queue-base-sha", default="")
    parser.add_argument("--implementation-root", required=True, type=Path)
    parser.add_argument("--policy-root", required=True, type=Path)
    parser.add_argument("--github-output", type=Path)
    parser.add_argument("--summary", type=Path)
    options = parser.parse_args(argv)
    try:
        selection = resolve_admission(
            AdmissionContext(
                options.event_name,
                options.repository,
                options.candidate_sha,
                options.automation_sha,
                options.policy_sha,
                options.tested_sha,
                options.pr_base_sha,
                options.queue_base_sha,
            ),
            options.implementation_root,
            options.policy_root,
        )
        if options.github_output:
            with options.github_output.open("a", encoding="utf-8") as output:
                for key, value in selection.outputs().items():
                    output.write(f"{key}={value}\n")
        if options.summary:
            with options.summary.open("a", encoding="utf-8") as output:
                output.write(_summary(selection))
        print(json.dumps(asdict(selection), sort_keys=True))
        return 0
    except (AdmissionError, OSError) as error:
        print(f"Dependency admission setup failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
