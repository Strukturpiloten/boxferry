"""Advance the existing automation-v1 branch after exact, complete main CI.

Only trusted default-branch code executes. Candidate revisions are API metadata,
never checkouts, artifacts, scripts or configuration. Consumers retain immutable
SHA pins; Renovate observes this branch as the source of reviewed updates.
"""

from __future__ import annotations

import argparse
import json
import os
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from boxferry_ci.eligibility import EligibilityError, GitHub, MissingResource, revision

REPOSITORY = "Strukturpiloten/boxferry"
WORKFLOW = ".github/workflows/distribute-automation.yml"
CI_WORKFLOW = ".github/workflows/ci.yml"
BRANCH = "automation-v1"
REF = f"git/refs/heads/{BRANCH}"
MAX_EVENT_BYTES = 1024 * 1024
MAX_REQUESTS = 40


class DistributionError(EligibilityError):
    """Distribution evidence is absent, inconsistent or outside its bounds."""


class API(GitHub):
    """Fixed repository, small metadata budget and exactly one writable ref."""

    def __init__(self, token: str) -> None:
        super().__init__(REPOSITORY, token)
        self.deadline = time.monotonic() + 180

    def request(self, method: str, path: str, data: dict[str, Any] | None = None) -> object:
        prefix = f"/repos/{REPOSITORY}/"
        if self.requests >= MAX_REQUESTS:
            raise DistributionError("distribution API request budget exhausted")
        if not (path == prefix[:-1] or path.startswith(prefix)):
            raise DistributionError("distribution only reads the canonical repository")
        if method != "GET" and not (
            method == "PATCH"
            and path == prefix + REF
            and data is not None
            and set(data) == {"sha", "force"}
            and data["force"] is False
        ):
            raise DistributionError("only a non-forced automation-v1 update is permitted")
        if method == "PATCH":
            revision(data["sha"] if data else None)
        return super().request(method, path, data)


@dataclass(frozen=True)
class Result:
    status: str
    candidate: str
    previous: str
    reason: str


def _number(value: object, label: str) -> int:
    if type(value) is not int or value < 1:
        raise DistributionError(f"{label} must be a positive integer")
    return value


def _repository(value: object) -> bool:
    return isinstance(value, dict) and value.get("full_name") == REPOSITORY


def _compare(api: GitHub, base: str, head: str) -> dict[str, Any]:
    value = api.get(f"compare/{revision(base)}...{revision(head)}?per_page=1")
    if value.get("status") not in {"ahead", "behind", "identical", "diverged"}:
        raise DistributionError("GitHub returned an unknown ancestry relationship")
    return value


def _ancestor(api: GitHub, base: str, head: str) -> bool:
    return base == head or _compare(api, base, head)["status"] in {"ahead", "identical"}


def _ref(api: GitHub, branch: str) -> str:
    try:
        value = api.get(f"git/ref/heads/{branch}")
    except MissingResource as error:
        raise DistributionError(
            f"branch {branch} is absent; the maintainer must bootstrap its verified initial "
            "commit after the foundation merge; this service never creates branches"
        ) from error
    if (
        value.get("ref") != f"refs/heads/{branch}"
        or value.get("object", {}).get("type") != "commit"
    ):
        raise DistributionError(f"branch {branch} does not identify a commit ref")
    return revision(value["object"].get("sha"))


def shared_path(path: str) -> bool:
    """Producer-owned paths only; consumer SHA adoption must not feed a loop."""
    return (
        path.startswith(("automation/", "renovate-presets/", "templates/"))
        or path.startswith(".github/workflows/shared-")
        or path.startswith(".github/actions/shared-")
        or path == ".devcontainer/Dockerfile"
        or path == WORKFLOW
    )


def _shared_changes(comparison: dict[str, Any]) -> bool:
    # REST compare returns at most 300 files, independently of commit pagination.
    # At the cap, omitted paths could otherwise hide a producer change.
    files = comparison.get("files")
    if not isinstance(files, list) or len(files) >= 300:
        raise DistributionError("changed-file evidence is absent or reaches the 300-file cap")
    relevant = False
    for file in files:
        if not isinstance(file, dict) or not isinstance(file.get("filename"), str):
            raise DistributionError("changed-file metadata is malformed")
        paths = [file["filename"]]
        if "previous_filename" in file:
            if not isinstance(file["previous_filename"], str):
                raise DistributionError("renamed-file metadata is malformed")
            paths.append(file["previous_filename"])
        relevant |= any(shared_path(path) for path in paths)
    return relevant


def _source(api: GitHub, expected: dict[str, Any], workflow_id: int) -> dict[str, Any]:
    source_id = _number(expected.get("id"), "source run id")
    run = api.get(f"actions/runs/{source_id}")
    if (
        run.get("id") != source_id
        or run.get("workflow_id") != workflow_id
        or run.get("name") != "CI"
        or run.get("path") != CI_WORKFLOW
        or not _repository(run.get("repository"))
        or not _repository(run.get("head_repository"))
        or run.get("head_branch") != "main"
        or run.get("event") != "push"
        or run.get("status") != "completed"
        or run.get("conclusion") != "success"
        or run.get("pull_requests") != []
    ):
        raise DistributionError("source must be successful canonical push-main CI")
    revision(run.get("head_sha"))
    _number(run.get("run_attempt"), "source run attempt")
    if any(run.get(key) != expected.get(key) for key in ("id", "head_sha", "run_attempt")):
        raise DistributionError("source event was superseded by another CI head or attempt")
    return run


def _jobs(api: GitHub, source: dict[str, Any]) -> None:
    source_id, attempt = source["id"], source["run_attempt"]
    listing = api.get(f"actions/runs/{source_id}/attempts/{attempt}/jobs?per_page=100")
    jobs = listing.get("jobs")
    if (
        not isinstance(jobs, list)
        or not jobs
        or len(jobs) > 100
        or type(listing.get("total_count")) is not int
        or listing["total_count"] != len(jobs)
    ):
        raise DistributionError("complete current-attempt CI jobs exceed or lack bounded evidence")
    required = 0
    for job in jobs:
        if (
            not isinstance(job, dict)
            or job.get("run_id") != source_id
            or job.get("run_attempt") != attempt
            or job.get("head_sha") != source["head_sha"]
            or job.get("status") != "completed"
            or job.get("conclusion") != "success"
        ):
            raise DistributionError("every current-attempt main CI job must succeed")
        required += job.get("name") == "Required CI"
    if required != 1:
        raise DistributionError("exactly one successful Required CI job must admit this main run")


def distribute(
    api: GitHub,
    *,
    event_name: str,
    event: dict[str, Any],
    implementation: str,
    run_id: int,
    run_attempt: int,
) -> Result:
    """Verify independent service/source identities before a monotonic ref update."""
    revision(implementation)
    _number(run_id, "service run id")
    _number(run_attempt, "service run attempt")
    if (
        api.repository != REPOSITORY
        or event_name != "workflow_run"
        or event.get("action") != "completed"
        or not _repository(event.get("repository"))
    ):
        raise DistributionError("distribution requires canonical workflow_run.completed")
    if api.get("").get("default_branch") != "main":
        raise DistributionError("default branch changed; review the explicit main-only contract")
    service = api.get(f"actions/runs/{run_id}")
    if (
        service.get("id") != run_id
        or service.get("run_attempt") != run_attempt
        or service.get("event") != "workflow_run"
        or service.get("path") != WORKFLOW
        or service.get("head_branch") != "main"
        or service.get("head_sha") != implementation
        or not _repository(service.get("repository"))
        or not _repository(service.get("head_repository"))
    ):
        raise DistributionError("executing service is not the trusted exact default-branch code")
    main = _ref(api, "main")
    if not _ancestor(api, implementation, main):
        raise DistributionError("service implementation is outside canonical main history")
    workflow = api.get("actions/workflows/ci.yml")
    workflow_id = _number(workflow.get("id"), "canonical CI workflow id")
    if workflow.get("path") != CI_WORKFLOW or workflow.get("state") != "active":
        raise DistributionError("canonical CI workflow is absent or inactive")
    expected = event.get("workflow_run")
    if not isinstance(expected, dict):
        raise DistributionError("completed event lacks its source workflow run")
    source = _source(api, expected, workflow_id)
    candidate = source["head_sha"]
    if not _ancestor(api, candidate, main):
        raise DistributionError("CI candidate is outside current canonical main history")
    _jobs(api, source)
    previous = _ref(api, BRANCH)

    def held(tip: str, current_main: str) -> bool:
        if not _ancestor(api, tip, current_main):
            raise DistributionError("distribution branch is outside canonical main history")
        return _ancestor(api, candidate, tip)

    if held(previous, main):
        return Result(
            "unchanged", candidate, previous, "candidate already distributed or superseded"
        )
    comparison = _compare(api, previous, candidate)
    if comparison["status"] != "ahead":
        raise DistributionError("distribution branch must be an ancestor of the tested candidate")
    if not _shared_changes(comparison):
        return Result("unchanged", candidate, previous, "no producer-owned automation changes")
    # Re-read all mutable authority immediately before the sole write. A newer
    # main commit is fine: only the exact independently tested ancestor advances.
    _source(api, expected, workflow_id)
    main = _ref(api, "main")
    if api.get("").get("default_branch") != "main":
        raise DistributionError(
            "default branch changed before update; review the main-only contract"
        )
    if not _ancestor(api, implementation, main) or not _ancestor(api, candidate, main):
        raise DistributionError("service or candidate left canonical main history before update")
    current = _ref(api, BRANCH)
    if current != previous:
        if held(current, main):
            return Result(
                "unchanged", candidate, current, "concurrent verified newer branch retained"
            )
        raise DistributionError("distribution branch changed; retry this exact source event safely")
    try:
        api.request("PATCH", f"/repos/{REPOSITORY}/{REF}", {"sha": candidate, "force": False})
    except EligibilityError:
        # Non-fast-forward races never justify force or an unverified retry.
        current, main = _ref(api, BRANCH), _ref(api, "main")
        if current != previous and held(current, main):
            return Result(
                "unchanged", candidate, current, "concurrent verified newer branch retained"
            )
        raise
    current, main = _ref(api, BRANCH), _ref(api, "main")
    if not held(current, main):
        raise DistributionError("updated branch readback does not contain the tested candidate")
    return Result("advanced", candidate, previous, f"verified non-forced {BRANCH} update")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--event-name", required=True)
    parser.add_argument("--event-path", required=True, type=Path)
    parser.add_argument("--implementation-revision", required=True)
    parser.add_argument("--run-id", required=True, type=int)
    parser.add_argument("--run-attempt", required=True, type=int)
    parser.add_argument("--summary", type=Path)
    options = parser.parse_args(argv)
    try:
        token = os.environ.get("GH_TOKEN")
        if not token:
            raise DistributionError("GH_TOKEN is required for authoritative GitHub metadata")
        if options.event_path.stat().st_size > MAX_EVENT_BYTES:
            raise DistributionError("workflow event exceeds its one-MiB budget")
        event = json.loads(options.event_path.read_text(encoding="utf-8"))
        if not isinstance(event, dict):
            raise DistributionError("workflow event must be an object")
        result = distribute(
            API(token),
            event_name=options.event_name,
            event=event,
            implementation=options.implementation_revision,
            run_id=options.run_id,
            run_attempt=options.run_attempt,
        )
        rendered = (
            f"Shared automation: **{result.status}**. {result.reason}.\n\n"
            f"Exact successful candidate: `{result.candidate}`; "
            f"previous tip: `{result.previous}`.\n"
        )
        print(rendered, end="")
        if options.summary:
            with options.summary.open("a", encoding="utf-8") as output:
                output.write(rendered)
        return 0
    except (EligibilityError, OSError, json.JSONDecodeError) as error:
        # These messages contain only bounded service-owned descriptions.
        print(f"::error::Shared automation distribution failed: {error}")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
