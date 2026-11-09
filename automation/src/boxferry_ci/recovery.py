"""Bounded recovery of an unchanged Renovate head across target-base movement.

Artifacts locate historical objects; they never establish the tested GitHub SHA.
Only the original immutable reader can approve that SHA. A failed or ambiguous
external effect consumes the reservation rather than creating an hourly retry.
"""

from __future__ import annotations

import hashlib
import io
import json
import re
import tempfile
import tomllib
import zipfile
from copy import deepcopy
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Any
from urllib.parse import quote

from . import eligibility as core
from .admission import MANIFEST_PATH, parse_manifest

HINT_FILE = "dependency-candidate-hint.json"
HINT_LIMIT = 16 * 1024
REBASE_LABEL = "rebase"
ACTIONS_APP = 15368
READER_PATH = "Strukturpiloten/boxferry/.github/workflows/shared-dependency-wait.yml"
READER_CLOSURE = (
    ".github/workflows/shared-dependency-wait.yml",
    ".github/actions/shared-python/action.yml",
    "automation/src/boxferry_ci/__init__.py",
    "automation/src/boxferry_ci/admission.py",
    "automation/src/boxferry_ci/bootstrap.py",
    "automation/src/boxferry_ci/cargo_tools.py",
    "automation/src/boxferry_ci/contracts.py",
    "automation/src/boxferry_ci/dependencies.py",
    "automation/src/boxferry_ci/dependency_snapshots.py",
    "automation/src/boxferry_ci/eligibility.py",
    "automation/src/boxferry_ci/git_source.py",
    "automation/src/boxferry_ci/operational.py",
    "automation/src/boxferry_ci/queue_tree.py",
    "automation/src/boxferry_ci/recovery.py",
    "automation/src/boxferry_ci/runner.py",
    "automation/pyproject.toml",
    "automation/uv.lock",
    "automation/uv-artifacts.json",
)


# Reviewed complete Docker aggregate; only immutable source/checkout pins vary.
_DOCKER_REQUIRED_GATE = (
    "  check-gate:\n"
    "    name: Required CI\n"
    "    if: always() && !(github.event_name == 'workflow_dispatch' &&"
    " github.ref == 'refs/heads/main')\n"
    "    needs:\n"
    "      [\n"
    "        validation-plan,\n"
    "        rust,\n"
    "        msrv,\n"
    "        dependencies,\n"
    "        api,\n"
    "        documentation,\n"
    "        lockfile-release-age,\n"
    "        native-conformance,\n"
    "      ]\n"
    "    runs-on: ubuntu-24.04\n"
    "    timeout-minutes: 5\n"
    "    outputs:\n"
    "      candidate_sha: ${{ github.sha }}\n"
    "    steps:\n"
    "      - name: Check out immutable shared automation\n"
    "        uses: __CHECKOUT_PIN__\n"
    "        with:\n"
    "          repository: Strukturpiloten/boxferry\n"
    "          # renovate: datasource=github-digest depName=Strukturpi"
    "loten/boxferry currentValue=automation-v1\n"
    "          ref: __ADMISSION_SHA__\n"
    "          path: _automation\n"
    "          persist-credentials: false\n"
    "      - name: Bind exact candidate and trusted policy\n"
    "        id: source\n"
    "        uses: ./_automation/.github/actions/shared-source\n"
    "        with:\n"
    "          candidate-sha: ${{ github.sha }}\n"
    "          automation-sha: __ADMISSION_SHA__\n"
    "          policy-sha: ${{ github.event.pull_request.base.sha || g"
    "ithub.event.merge_group.base_sha || github.sha }}\n"
    "          config-path: scripts/validation-policy.json\n"
    "      - name: Install locked shared Python tooling\n"
    "        uses: ./_automation/.github/actions/shared-python\n"
    "      - name: Require selected jobs and only intentional skips\n"
    "        id: offline\n"
    "        env:\n"
    "          EVENT_NAME: ${{ github.event_name }}\n"
    "          BASE_SHA: ${{ github.event.pull_request.base.sha || git"
    "hub.event.merge_group.base_sha || github.sha }}\n"
    "          HEAD_SHA: ${{ github.event.pull_request.head.sha || git"
    "hub.event.merge_group.head_sha || github.sha }}\n"
    "          POLICY_SHA: ${{ github.event.pull_request.base.sha || g"
    "ithub.event.merge_group.base_sha || github.sha }}\n"
    "          AUTOMATION_SHA: __ADMISSION_SHA__\n"
    "          POLICY_CONFIG: ${{ steps.source.outputs.config }}\n"
    "          PLAN_JSON: ${{ needs.validation-plan.outputs.plan }}\n"
    "          NEEDS_JSON: ${{ toJSON(needs) }}\n"
    "        run: |\n"
    "          NEEDS_JSON=\"$(python3 -c 'import json, os; needs = json"
    '.loads(os.environ["NEEDS_JSON"]); needs.pop("native-conformance")'
    "; print(json.dumps(needs))')\"\n"
    "          uv run --locked --project _automation/automation python"
    " -m boxferry_ci gate \\\n"
    '            --repository-root source --config "${POLICY_CONFIG}" '
    "\\\n"
    '            --event "${EVENT_NAME}" --base-sha "${BASE_SHA}" --he'
    'ad-sha "${HEAD_SHA}" \\\n'
    '            --tested-sha "${GITHUB_SHA}" --policy-sha "${POLICY_S'
    'HA}" \\\n'
    '            --automation-sha "${AUTOMATION_SHA}" \\\n'
    '            --plan-json "${PLAN_JSON}" --needs-json "${NEEDS_JSON'
    '}"\n'
    "      - name: Fail on skipped, cancelled, or failed validation\n"
    "        env:\n"
    "          EVENT_NAME: ${{ github.event_name }}\n"
    "          EVENT_REF: ${{ github.ref }}\n"
    "          SCAFFOLD_RESULT: ${{ steps.offline.outcome }}\n"
    "          NATIVE_RESULT: ${{ needs.native-conformance.result }}\n"
    "        run: |\n"
    '          test "$SCAFFOLD_RESULT" = success\n'
    '          if [[ "$EVENT_NAME" == pull_request || "$EVENT_NAME" =='
    " merge_group ]]; then\n"
    '            test "$NATIVE_RESULT" = skipped\n'
    "            echo 'PR and queue validation is offline-only; native"
    " Engine evidence is not claimed'\n"
    '          elif [[ "$EVENT_NAME" == push && "$EVENT_REF" == refs/h'
    "eads/main ]]; then\n"
    '            test "$NATIVE_RESULT" = success\n'
    "          else\n"
    "            echo 'unexpected validation event; native evidence is"
    " required' >&2\n"
    "            exit 1\n"
    "          fi\n"
)


class RecoveryRefused(ValueError):
    """Missing proof keeps the ordinary candidate policy in force."""


def unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise RecoveryRefused("candidate hint repeats a JSON key")
        result[key] = value
    return result


def hint_artifact(api: core.GitHub, run: dict[str, Any]) -> dict[str, Any] | None:
    """Distinguish complete absence from every malformed or conflicting present hint."""
    run_id, attempt = run.get("id"), run.get("run_attempt")
    if type(run_id) is not int or run_id < 1 or type(attempt) is not int or attempt < 1:
        raise RecoveryRefused("original CI run lacks an exact run/attempt identity")
    listing = api.get(f"actions/runs/{run_id}/artifacts?per_page=100")
    artifacts = listing.get("artifacts")
    if (
        not isinstance(artifacts, list)
        or len(artifacts) >= 100
        or type(listing.get("total_count")) is not int
        or listing["total_count"] != len(artifacts)
        or not all(
            isinstance(item, dict) and isinstance(item.get("name"), str) for item in artifacts
        )
    ):
        raise RecoveryRefused("candidate hint artifact listing is incomplete")
    name = f"dependency-candidate-hint-{run_id}-{attempt}"
    hints = [item for item in artifacts if item["name"].startswith("dependency-candidate-hint")]
    if not hints:
        return None
    if len(hints) != 1 or hints[0]["name"] != name:
        raise RecoveryRefused("candidate hint artifacts are ambiguous or conflict with run/attempt")
    return hints[0]


def candidate_hint(
    api: core.GitHub,
    run: dict[str, Any],
    candidate: core.Candidate,
    *,
    artifact: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Read one bounded, explicitly untrusted run/attempt-scoped hint."""
    run_id, attempt = run.get("id"), run.get("run_attempt")
    artifact = hint_artifact(api, run) if artifact is None else artifact
    if artifact is None:
        raise RecoveryRefused("original CI has no unique run/attempt candidate hint")
    if (
        artifact.get("expired") is not False
        or type(artifact.get("id")) is not int
        or artifact["id"] < 1
        or type(artifact.get("size_in_bytes")) is not int
        or not 0 < artifact["size_in_bytes"] <= HINT_LIMIT
    ):
        raise RecoveryRefused("candidate hint artifact is expired or exceeds its bounds")
    raw = api.artifact(artifact["id"])
    if len(raw) > HINT_LIMIT:
        raise RecoveryRefused("candidate hint archive exceeds its byte bound")
    try:
        with zipfile.ZipFile(io.BytesIO(raw)) as archive:
            entries = archive.infolist()
            if (
                len(entries) != 1
                or entries[0].filename != HINT_FILE
                or entries[0].file_size > 4096
                or entries[0].flag_bits & 1
                or entries[0].is_dir()
                or (entries[0].external_attr >> 16) & 0o170000 == 0o120000
            ):
                raise RecoveryRefused("candidate hint archive has an unsupported member")
            payload = json.loads(archive.read(entries[0]), object_pairs_hook=unique_object)
    except (
        zipfile.BadZipFile,
        UnicodeDecodeError,
        json.JSONDecodeError,
        NotImplementedError,
    ) as error:
        raise RecoveryRefused("candidate hint archive is malformed") from error
    expected = {
        "schema": 1,
        "repository": api.repository,
        "run_id": run_id,
        "run_attempt": attempt,
        "pull_request": candidate.number,
        "source_head_sha": candidate.source_head,
    }
    if (
        not isinstance(payload, dict)
        or set(payload) != set(expected) | {"candidate_sha", "base_sha"}
        or any(
            type(payload.get(key)) is not type(value) or payload[key] != value
            for key, value in expected.items()
        )
    ):
        raise RecoveryRefused("candidate hint does not match this original run/attempt/source")
    core.revision(payload["candidate_sha"])
    core.revision(payload["base_sha"])
    return payload


def ordered_merge(api: core.GitHub, merge: str, base: str, source: str) -> None:
    """Ancestry cannot substitute for the exact ordered synthetic merge parents."""
    commit = api.get(f"git/commits/{core.revision(merge)}")
    parents = commit.get("parents")
    if (
        commit.get("sha") != merge
        or not isinstance(parents, list)
        or [parent.get("sha") if isinstance(parent, dict) else None for parent in parents]
        != [base, source]
    ):
        raise RecoveryRefused("historical/current merge must have direct parents exactly [base, H]")


def inventory(
    api: core.GitHub, revision: str, *, modes: dict[str, str] | None = None
) -> dict[str, str]:
    tree = api.get(f"git/trees/{core.revision(revision)}?recursive=1")
    entries = tree.get("tree")
    if tree.get("truncated") is not False or not isinstance(entries, list) or len(entries) > 20000:
        raise RecoveryRefused("recovery Git tree inventory is incomplete or exceeds its bound")
    result: dict[str, str] = {}
    seen: set[str] = set()
    for entry in entries:
        if not isinstance(entry, dict) or not isinstance(entry.get("path"), str):
            raise RecoveryRefused("recovery Git tree inventory is malformed")
        path = entry["path"]
        if path in seen:
            raise RecoveryRefused("recovery Git tree repeats a path")
        seen.add(path)
        if entry.get("type") == "blob" and entry.get("mode") in {"100644", "100755"}:
            result[path] = core.revision(entry.get("sha"))
            if modes is not None:
                modes[path] = entry["mode"]
        elif path == MANIFEST_PATH or (
            path.startswith(".github/workflows/") and path.endswith((".yml", ".yaml"))
        ):
            raise RecoveryRefused("workflow catalog contains a nonregular input")
    return result


def required_file(api: core.GitHub, path: str, sha: str, tree: dict[str, str]) -> bytes:
    if path not in tree:
        raise RecoveryRefused(f"recovery requires the regular tracked file {path}")
    content = core.file_bytes(api, path, sha)
    if content is None:
        raise RecoveryRefused(f"recovery cannot read the exact tracked file {path}")
    actual = hashlib.sha1(
        b"blob " + str(len(content)).encode() + b"\0" + content, usedforsecurity=False
    ).hexdigest()
    if actual != tree[path]:
        raise RecoveryRefused("recovery file bytes disagree with the tracked Git blob")
    return content


def authority(api: core.GitHub, base: str, tree: dict[str, str]) -> tuple[str, str, bytes]:
    selected, policy_path = parse_manifest(required_file(api, MANIFEST_PATH, base, tree))
    policy = required_file(api, policy_path, base, tree)
    if len(policy) > 65536:
        raise RecoveryRefused("tracked admission policy exceeds its byte bound")
    return selected, policy_path, policy


def job_blocks(content: bytes) -> dict[str, str]:
    """Recognize the reviewed flat caller shape, never evaluate arbitrary YAML."""
    text = content.decode("utf-8")
    if text.count("\njobs:\n") != 1 or "\t" in text:
        raise RecoveryRefused("caller is outside the reviewed flat workflow shape")
    jobs = text.split("\njobs:\n", 1)[1]
    jobs = re.split(r"(?m)^(?=[A-Za-z][A-Za-z0-9_-]*:)", jobs, maxsplit=1)[0]
    starts = list(re.finditer(r"(?m)^  ([A-Za-z0-9_-]+):\s*$", jobs))
    result = {}
    for index, start in enumerate(starts):
        name = start[1]
        if name in result:
            raise RecoveryRefused("caller repeats a job identity")
        end = starts[index + 1].start() if index + 1 < len(starts) else len(jobs)
        result[name] = jobs[start.start() : end]
    return result


def reader_shape(content: bytes, implementation: str) -> None:
    jobs = job_blocks(content)
    reader = jobs.get("lockfile-release-age", "")
    uses = re.findall(r"(?m)^    uses: ([^\s#]+)", reader)
    required = (
        "    name: Dependency eligibility\n",
        "    needs: validation-plan\n",
        "    if: needs.validation-plan.outputs.select_lockfile_release_age == 'true'\n",
        "      candidate_sha: ${{ github.sha }}\n",
        f"      automation_sha: {implementation}\n",
        "      policy_sha: ${{ github.event.pull_request.base.sha || "
        "github.event.merge_group.base_sha || github.sha }}\n",
    )
    if (
        uses != [f"{READER_PATH}@{implementation}"]
        or any(value not in reader for value in required)
        or re.search(r"(?m)^    (?:steps|secrets|env|strategy):", reader)
        or re.search(r"(?m)^      [\w-]+: (?:write|inherit)\s*$", reader)
    ):
        raise RecoveryRefused(
            "actual caller does not have the supported immutable read-only reader"
        )
    aggregates = [block for block in jobs.values() if "    name: Required CI\n" in block]
    if len(aggregates) != 1:
        raise RecoveryRefused("caller lacks a unique fixed Required CI aggregate")
    gate = aggregates[0]
    tokens = (
        "lockfile-release-age",
        "validation-plan",
        "boxferry_ci gate",
        '--tested-sha "${GITHUB_SHA}"',
        '--plan-json "${PLAN_JSON}"',
        '--needs-json "${NEEDS_JSON}"',
        "github.event.merge_group.base_sha",
        f"AUTOMATION_SHA: {implementation}",
    )
    if (
        any(value not in gate for value in tokens)
        or not re.search(r"(?m)^    if: always\(\)(?:\s|$)", gate)
        or not re.search(rb"(?m)^  merge_group:(?: null)?[ \t]*$", content)
        or not any(
            value in gate
            for value in (
                '--policy-sha "${POLICY_SHA}"',
                '--policy-sha "${BASE_SHA}"',
            )
        )
        or not any(
            value in gate
            for value in (
                "HEAD_SHA: ${{ github.event.pull_request.head.sha || "
                "github.event.merge_group.head_sha }}",
                "HEAD_SHA: ${{ github.event.pull_request.head.sha || github.sha }}",
                "HEAD_SHA: ${{ github.event.pull_request.head.sha || "
                "github.event.merge_group.head_sha || github.sha }}",
            )
        )
    ):
        raise RecoveryRefused("fixed aggregate lacks the full combined-candidate queue contract")


def reader_closure(api: core.GitHub, implementation: str) -> None:
    """Read only the canonical immutable closure, matching the executing source."""
    canonical = core.GitHub("Strukturpiloten/boxferry", api.token)
    canonical.deadline = api.deadline
    root = Path(__file__).resolve().parents[3]
    try:
        tree = inventory(canonical, implementation)
        for path in READER_CLOSURE:
            remote = required_file(canonical, path, implementation, tree)
            local = root / path
            if local.is_symlink() or not local.is_file() or local.read_bytes() != remote:
                raise RecoveryRefused("immutable reader closure differs from trusted service C")
            if path.endswith("eligibility.py") and core.reader_contract(remote) != 2:
                raise RecoveryRefused("selected immutable reader lacks expected-base contract 2")
    finally:
        api.requests += canonical.requests


def ruleset_bypasses(
    api: core.GitHub, identifier: int, rules: list[dict[str, Any]]
) -> list[dict[str, Any]]:
    """Require visible, bounded bypass facts; a filtered field is not an empty list."""
    owner = api.repository.split("/", 1)[0]
    sources = {
        f"{kind} {source}"
        for rule in rules
        if rule.get("ruleset_id") == identifier
        for kind, source in (("Organization", owner), ("Repository", api.repository))
        if rule.get("ruleset_source_type") == kind and rule.get("ruleset_source") == source
    }
    source = next(iter(sources)) if len(sources) == 1 else f"via {api.repository}"
    label = f"ruleset {identifier} ({source})"
    try:
        detail = api.get(f"rulesets/{identifier}?includes_parents=true")
    except core.EligibilityError as error:
        # The client exposes errors as text. Only recognize fixed client messages;
        # never copy provider bodies, exception text or candidate metadata to the report.
        reason = "inaccessible; bypass proof unavailable"
        if str(error) in {
            "GitHub metadata response is not valid JSON",
            "GitHub metadata response must be an object",
        }:
            reason = "malformed detail response"
        elif str(error) == "GitHub metadata response exceeds its size budget":
            reason = "oversized detail response"
        elif str(error).startswith("GitHub metadata transport failed:"):
            reason = "transport failed; bypass proof unavailable"
        raise RecoveryRefused(f"{label}: {reason}") from None
    if not isinstance(detail, dict):
        raise RecoveryRefused(f"{label}: malformed detail response")
    if type(detail.get("id")) is not int or detail["id"] != identifier:
        raise RecoveryRefused(f"{label}: wrong-identity detail response")
    if "bypass_actors" not in detail:
        raise RecoveryRefused(
            f"{label}: bypass_actors not returned; bypass proof unavailable "
            "(GitHub requires ruleset write access to expose this field)"
        )
    actors = detail["bypass_actors"]
    if not isinstance(actors, list):
        raise RecoveryRefused(f"{label}: malformed bypass_actors; expected a list")
    if len(actors) > 20:
        raise RecoveryRefused(f"{label}: oversized bypass_actors; limit is 20")
    for actor in actors:
        if not isinstance(actor, dict) or "actor_id" not in actor:
            raise RecoveryRefused(f"{label}: malformed bypass actor")
        actor_id = actor["actor_id"]
        # GitHub ignores OrganizationAdmin IDs and returns explicit null. Other
        # actor types still require their exact reviewed integer identity.
        admin = actor.get("actor_type") == "OrganizationAdmin" and (
            actor_id is None or type(actor_id) is int
        )
        role = (
            actor.get("actor_type") == "RepositoryRole" and type(actor_id) is int and actor_id == 5
        )
        if actor.get("bypass_mode") != "always" or not (admin or role):
            raise RecoveryRefused(f"{label}: unsupported or integration bypass actor")
    return actors


def protection(api: core.GitHub, branch: str) -> dict[str, Any]:
    core.require_single_entry_queue(api, branch)
    rules = api.items(f"rules/branches/{quote(branch, safe='')}?per_page=100&page=1")
    if len(rules) >= 100:
        raise RecoveryRefused("recovery effective rules are incomplete")
    if any(
        not isinstance(rule.get("parameters"), dict)
        for rule in rules
        if rule.get("type") in {"merge_queue", "required_status_checks"}
    ):
        raise RecoveryRefused("recovery effective rule parameters are malformed")
    for rule in rules:
        if rule.get("type") == "required_status_checks":
            checks = rule["parameters"].get("required_status_checks")
            if (
                not isinstance(checks, list)
                or len(checks) > 100
                or not all(
                    isinstance(check, dict) and isinstance(check.get("context"), str)
                    for check in checks
                )
            ):
                raise RecoveryRefused("required status check definitions are malformed")
    queue = [rule for rule in rules if rule.get("type") == "merge_queue"]
    if len(queue) != 1 or queue[0].get("parameters", {}).get("grouping_strategy") != "ALLGREEN":
        raise RecoveryRefused("recovery requires the reviewed ALLGREEN one-entry queue")
    checks = [
        check
        for rule in rules
        if rule.get("type") == "required_status_checks"
        for check in rule.get("parameters", {}).get("required_status_checks", [])
        if isinstance(check, dict) and check.get("context") == "Required CI"
    ]
    if not checks or any(
        (
            check.get("integration_id") is not None
            and (type(check["integration_id"]) is not int or check["integration_id"] != ACTIONS_APP)
        )
        for check in checks
    ):
        raise RecoveryRefused("effective Required CI rule is absent or binds another provider")
    rulesets = {rule.get("ruleset_id") for rule in rules}
    if not rulesets or any(
        type(identifier) is not int or identifier < 1 for identifier in rulesets
    ):
        raise RecoveryRefused("effective protection lacks dereferenceable ruleset identities")
    bypasses: list[dict[str, Any]] = []
    for identifier in sorted(identifier for identifier in rulesets if isinstance(identifier, int)):
        bypasses.extend(ruleset_bypasses(api, identifier, rules))
    return {
        "required_ci_binding": "any-app"
        if any(check.get("integration_id") is None for check in checks)
        else "github-actions",
        "bypass_actors": bypasses,
    }


def original_run(
    api: core.GitHub,
    run_id: int,
    candidate: core.Candidate,
    implementation: str | None,
    *,
    excluded_jobs: frozenset[str] = frozenset(),
) -> dict[str, Any]:
    run = api.get(f"actions/runs/{run_id}")
    if not all(isinstance(run.get(field), dict) for field in ("repository", "head_repository")):
        raise RecoveryRefused("original CI repository metadata is malformed")
    if (
        run.get("id") != run_id
        or not core.candidate_run(api, run, candidate.source_head)
        or run.get("event") != "pull_request"
        or run.get("status") != "completed"
        or run.get("conclusion") != "failure"
        or type(run.get("run_attempt")) is not int
        or run["run_attempt"] != 1
    ):
        raise RecoveryRefused("original CI is changed, active, successful or already attempted")
    prs = run.get("pull_requests", [])
    if (
        not isinstance(prs, list)
        or len(prs) != 1
        or not isinstance(prs[0], dict)
        or not isinstance(prs[0].get("head"), dict)
        or prs[0].get("number") != candidate.number
        or prs[0].get("head", {}).get("sha") != candidate.source_head
    ):
        raise RecoveryRefused("original CI does not identify the unchanged owned source H")
    if implementation is not None:
        reader_reference(run, implementation)
    attempt = run["run_attempt"]
    listing = api.get(f"actions/runs/{run_id}/attempts/{attempt}/jobs?per_page=100")
    jobs = listing.get("jobs")
    if not isinstance(jobs, list) or len(jobs) > 100 or listing.get("total_count") != len(jobs):
        raise RecoveryRefused("original CI attempt job listing is incomplete")
    aggregates = [job for job in jobs if isinstance(job, dict) and job.get("name") == "Required CI"]
    if len(aggregates) != 1:
        raise RecoveryRefused("original attempt lacks one exact Required CI job")
    aggregate = aggregates[0]
    if any(
        aggregate.get(key) != value
        for key, value in {
            "run_id": run_id,
            "run_attempt": attempt,
            "status": "completed",
            "conclusion": "failure",
        }.items()
    ):
        raise RecoveryRefused("Required CI job belongs to another run/attempt or state")
    url = aggregate.get("check_run_url", "")
    match = re.fullmatch(
        rf"https://api\.github\.com/repos/{re.escape(api.repository)}/check-runs/([0-9]+)", url
    )
    if not match:
        raise RecoveryRefused("Required CI check URL is outside the fixed repository API")
    check = api.get(f"check-runs/{match[1]}")
    if (
        check.get("id") != int(match[1])
        or check.get("name") != "Required CI"
        or check.get("head_sha") != run.get("head_sha")
        or check.get("app", {}).get("id") != ACTIONS_APP
        or check.get("app", {}).get("slug") != "github-actions"
        or check.get("status") != "completed"
        or check.get("conclusion") != "failure"
    ):
        raise RecoveryRefused("actual Required CI check lacks exact GitHub Actions provenance")
    if aggregate.get("check_suite_id") is not None and aggregate["check_suite_id"] != check.get(
        "check_suite", {}
    ).get("id"):
        raise RecoveryRefused("Required CI check suite does not match the original job")
    for name in excluded_jobs:
        matches = [job for job in jobs if isinstance(job, dict) and job.get("name") == name]
        if len(matches) != 1 or matches[0].get("conclusion") != "skipped":
            raise RecoveryRefused(
                "static excluded original job is missing, duplicated or not skipped"
            )
    failed_reader = False
    for job in jobs:
        if not isinstance(job, dict) or job.get("status") != "completed":
            raise RecoveryRefused("original CI attempt is not fully completed")
        conclusion = job.get("conclusion")
        if conclusion == "skipped" and job.get("name") in excluded_jobs:
            continue
        # No artifact/failed aggregate authenticates a selected skip plan here.
        # Until a bounded trusted plan is available, even planned skips refuse recovery.
        if conclusion not in {"success", "failure"}:
            raise RecoveryRefused("original CI contains unsupported job outcomes")
        if conclusion == "failure" and job.get("name") != "Required CI":
            if job.get("name") not in {
                "Dependency eligibility / Dependency eligibility",
                "Dependency eligibility",
                "Lockfile release age",
            }:
                raise RecoveryRefused("original CI has a failure outside dependency admission")
            failed_reader = True
    if not failed_reader:
        raise RecoveryRefused("original CI has no failed dependency reader")
    return run


def report_matches(
    report: object, candidate: core.Candidate, statuses: frozenset[str] = frozenset({"pass"})
) -> bool:
    return (
        isinstance(report, dict)
        and report.get("status") in statuses
        and all(
            report.get(key) == value
            for key, value in {
                "candidate_sha": candidate.head,
                "base_sha": candidate.base,
                "source_head_sha": candidate.source_head,
                "pull_request": candidate.number,
                "scope": candidate.scope,
            }.items()
        )
    )


@dataclass
class ServiceContext:
    """One immutable protocol checkpoint and the current execution's effect fence."""

    previous: dict[str, Any] | None
    frontier: dict[str, Any]
    created_at: datetime
    run_id: int
    attempt: int
    can_prepare: bool
    can_effect: bool
    checkpoint_run: dict[str, Any] | None = None


def effect_floor(api: core.GitHub, implementation: str) -> datetime:
    """Use C's immutable date as an enforced policy floor, never creation/age evidence."""
    value = api.request("GET", f"/repos/Strukturpiloten/boxferry/git/commits/{implementation}")
    if (
        not isinstance(value, dict)
        or value.get("sha") != implementation
        or not isinstance(value.get("committer"), dict)
    ):
        raise RecoveryRefused("canonical C has no independently readable immutable effect floor")
    return core.utc(value["committer"].get("date"))


def unstarted_service(api: core.GitHub, run: dict[str, Any]) -> bool:
    """Only a genuinely queued first attempt can use the no-start branch."""
    queued = ("queued", "waiting", "pending", "requested")
    if (
        type(run.get("id")) is not int
        or run["id"] < 1
        or type(run.get("run_attempt")) is not int
        or run["run_attempt"] != 1
        or run.get("status") not in queued
        or "conclusion" not in run
        or run["conclusion"] is not None
    ):
        return False
    listing = api.get(f"actions/runs/{run['id']}/attempts/1/jobs?per_page=100")
    jobs = listing.get("jobs")
    return (
        isinstance(jobs, list)
        and len(jobs) <= 100
        and type(listing.get("total_count")) is int
        and listing.get("total_count") == len(jobs)
        and all(
            isinstance(job, dict)
            and type(job.get("id")) is int
            and job["id"] > 0
            and type(job.get("run_id")) is int
            and job["run_id"] == run["id"]
            and type(job.get("run_attempt")) is int
            and job["run_attempt"] == 1
            and "started_at" in job
            and job.get("started_at") is None
            and "conclusion" in job
            and job.get("conclusion") is None
            and job.get("status") in queued
            for job in jobs
        )
        and len({job["id"] for job in jobs}) == len(jobs)
    )


def skipped_service(
    api: core.GitHub, run: dict[str, Any], implementation: str, branch: str
) -> bool:
    """Prove the exact guarded outer call never dispatched; this is no receipt."""
    if (
        api.repository not in core.REPOSITORIES
        or type(run.get("id")) is not int
        or run["id"] < 1
        or type(run.get("run_attempt")) is not int
        or run["run_attempt"] != 1
        or run.get("status") != "completed"
        or run.get("conclusion") != "skipped"
        or not isinstance(run.get("repository"), dict)
        or run["repository"].get("full_name") != api.repository
        or not isinstance(run.get("event"), str)
        or run["event"] not in core.ALLOWED_EVENTS
        or run.get("path") != core.WORKFLOW_PATH
        or run.get("head_branch") != branch
        or run.get("name") != "Dependency eligibility service"
        or run.get("referenced_workflows")
        != [{"path": f"{core.SHARED_WORKFLOW_PATH}@{implementation}", "sha": implementation}]
    ):
        return False
    head = core.revision(run.get("head_sha"))
    core.revision(implementation)
    workflow = api.get("actions/workflows/dependency-eligibility.yml")
    if (
        type(run.get("workflow_id")) is not int
        or run["workflow_id"] < 1
        or type(workflow.get("id")) is not int
        or workflow["id"] != run["workflow_id"]
        or workflow.get("path") != core.WORKFLOW_PATH
        or workflow.get("name") != run["name"]
    ):
        return False
    if api.get(f"compare/{head}...{quote(branch, safe='')}").get("status") not in (
        "ahead",
        "identical",
    ):
        return False
    caller = core.file_bytes(api, core.WORKFLOW_PATH, head)
    if caller is None:
        return False
    expected = (
        "  eligibility:\n    name: Trusted dependency metadata\n    if: >-\n"
        "      github.event_name == 'schedule' || (github.event_name == 'workflow_run' && "
        "(github.event.workflow_run.event == 'pull_request' || "
        "github.event.workflow_run.event == 'merge_group'))\n"
        "    permissions:\n      actions: write\n      checks: read\n      contents: read\n"
        "      pull-requests: write\n      statuses: write\n"
        f"    uses: {core.SHARED_WORKFLOW_PATH}@{implementation} # automation-v1 interface v1\n"
        f"    with:\n      automation-sha: {implementation}\n"
    )
    if job_blocks(caller) != {"eligibility": expected}:
        return False
    listing = api.get(f"actions/runs/{run['id']}/attempts/1/jobs?per_page=100")
    jobs = listing.get("jobs")
    if type(listing.get("total_count")) is not int or listing["total_count"] != 1:
        return False
    if not isinstance(jobs, list) or len(jobs) != 1 or not isinstance(jobs[0], dict):
        return False
    job = jobs[0]
    if (
        type(job.get("id")) is not int
        or job["id"] < 1
        or type(job.get("run_id")) is not int
        or job["run_id"] != run["id"]
        or type(job.get("run_attempt")) is not int
        or job["run_attempt"] != 1
        or job.get("name") != "Trusted dependency metadata"
        or job.get("workflow_name") != run["name"]
        or job.get("head_sha") != head
        or job.get("head_branch") != branch
        or job.get("status") != "completed"
        or job.get("conclusion") != "skipped"
        or job.get("steps") != []
        or job.get("labels") != []
        or not {"runner_id", "runner_name", "runner_group_id", "runner_group_name"} <= job.keys()
        or any(value is not None for key, value in job.items() if key.startswith("runner_"))
    ):
        return False
    for mapping, fields in (
        (run, ("created_at", "updated_at", "run_started_at")),
        (job, ("started_at", "completed_at")),
    ):
        for field in fields:
            if field not in mapping or (
                mapping[field] is not None and not isinstance(mapping[field], str)
            ):
                return False
            if mapping[field] is not None:
                core.utc(mapping[field])
    artifacts = api.get(f"actions/runs/{run['id']}/artifacts?per_page=100")
    return (
        type(artifacts.get("total_count")) is int
        and artifacts["total_count"] == 0
        and artifacts.get("artifacts") == []
    )


def immutable_service_source(run: dict[str, Any]) -> str:
    references = run.get("referenced_workflows")
    if (
        not isinstance(references, list)
        or len(references) > 30
        or not all(
            isinstance(item, dict)
            and isinstance(item.get("path"), str)
            and isinstance(item.get("sha"), str)
            for item in references
        )
    ):
        raise RecoveryRefused("service history immutable reference metadata is malformed")
    source = core.service_revision(run)
    if source is None:
        raise RecoveryRefused("service history has unknown immutable implementation")
    return source


def checkpoint_payload(
    api: core.GitHub,
    run: dict[str, Any],
    implementation: str,
    branch: str,
    floor: datetime,
    now: datetime,
) -> dict[str, Any]:
    if immutable_service_source(run) != implementation or not core.trusted_run(
        api, run, implementation, branch
    ):
        raise RecoveryRefused(
            "service history has no completed-success same-C checkpoint authority"
        )
    identifier, attempt = run.get("id"), run.get("run_attempt")
    if type(identifier) is not int or identifier < 1 or type(attempt) is not int or attempt < 1:
        raise RecoveryRefused("checkpoint run/attempt identity is malformed")
    created = core.utc(run.get("created_at"))
    payload = core.validate_state(
        core.run_artifact(api, identifier),
        repository=api.repository,
        run_id=identifier,
        implementation=implementation,
        now=now,
    )
    frontier = payload.get("recovery_frontier")
    if (
        payload.get("run_attempt") != attempt
        or not isinstance(frontier, dict)
        or frontier.get("protocol") != 1
        or frontier.get("floor") != floor.isoformat()
        or frontier.get("service_created_at") != created.isoformat()
        or type(frontier.get("authority")) is not bool
        or created < floor
    ):
        raise RecoveryRefused("checkpoint artifact does not bind its exact server attempt/floor")
    return payload


def merge_budgets(selected: dict[str, Any], evidence: list[dict[str, Any]]) -> dict[str, Any]:
    """Union only conservative observations/budgets; reports remain one exact receipt."""
    result = deepcopy(selected)
    reruns = result.setdefault("reruns", {})
    recoveries = result.setdefault("recoveries", {})
    first_seen = result["dependency_state"]["first_seen"]
    for payload in evidence:
        for head, identifier in payload.get("reruns", {}).items():
            if head in reruns and reruns[head] != identifier:
                raise RecoveryRefused("same-C history has conflicting normal retry budgets")
            reruns[head] = identifier
        for key, entry in payload.get("recoveries", {}).items():
            existing = recoveries.get(key)
            if existing is not None:
                bindings = (
                    "original_run_id",
                    "original_attempt",
                    "source_head_sha",
                    "authority",
                    "hinted_candidate_sha",
                    "hinted_base_sha",
                    "kind",
                    "merge_base_sha",
                    "transition_fingerprint",
                    "original_identity_sha256",
                )
                if any(existing.get(name) != entry.get(name) for name in bindings):
                    raise RecoveryRefused(
                        "same-C history has conflicting original recovery bindings"
                    )
                if existing["phase"] != "prepared":
                    continue
            recoveries[key] = deepcopy(entry)
        for key, instant in payload["dependency_state"]["first_seen"].items():
            if key not in first_seen or core.utc(instant) < core.utc(first_seen[key]):
                first_seen[key] = instant
    return result


def history_runs(api: core.GitHub, lower: datetime) -> list[dict[str, Any]]:
    listing = api.get(
        "actions/workflows/dependency-eligibility.yml/runs?per_page=100&created="
        + quote(">=" + lower.isoformat(), safe="")
    )
    runs = listing.get("workflow_runs")
    if (
        not isinstance(runs, list)
        or len(runs) > 100
        or type(listing.get("total_count")) is not int
        or listing.get("total_count") != len(runs)
        or not all(isinstance(run, dict) for run in runs)
    ):
        raise RecoveryRefused(
            "service checkpoint history is incomplete; effect budget is suspended"
        )
    identifiers = [run.get("id") for run in runs]
    if any(type(identifier) is not int or identifier < 1 for identifier in identifiers) or len(
        set(identifiers)
    ) != len(identifiers):
        raise RecoveryRefused("service checkpoint history has ambiguous run identities")
    result = [api.get(f"actions/runs/{identifier}") for identifier in identifiers]
    if any(
        type(run.get("id")) is not int or run["id"] != identifier
        for run, identifier in zip(result, identifiers, strict=True)
    ):
        raise RecoveryRefused("service history response changed an exact run identity")
    return result


def service_context(
    api: core.GitHub,
    current: dict[str, Any],
    previous: dict[str, Any] | None,
    implementation: str,
    branch: str,
    now: datetime,
) -> ServiceContext:
    """Advance a bounded same-C checkpoint, preserving every uncertainty and budget."""
    identifier, attempt = current["id"], current["run_attempt"]
    created = core.utc(current.get("created_at"))
    floor: datetime | None = None
    fence: str | None = None
    checkpoint: dict[str, Any] | None = None
    selected = previous
    evidence: list[dict[str, Any]] = []
    candidates: list[tuple[dict[str, Any], dict[str, Any]]] = []
    pending: list[dict[str, int]] = []
    try:
        if type(identifier) is not int or identifier < 1 or type(attempt) is not int or attempt < 1:
            raise RecoveryRefused("current service run/attempt identity is malformed")
        floor = effect_floor(api, implementation)
        lower = floor
        if previous is not None and previous.get("implementation_revision") == implementation:
            anchor_id = previous["run_id"]
            prior_frontier = previous.get("recovery_frontier", {})
            if prior_frontier.get("authority") is False:
                anchor_id = prior_frontier.get("predecessor_run_id")
            if anchor_id is not None:
                anchor = api.get(f"actions/runs/{anchor_id}")
                if anchor.get("id") != anchor_id:
                    raise RecoveryRefused(
                        "checkpoint predecessor response changed its exact run ID"
                    )
                anchor_state = checkpoint_payload(api, anchor, implementation, branch, floor, now)
                if not anchor_state["recovery_frontier"]["authority"]:
                    raise RecoveryRefused("selected predecessor is not checkpoint authority")
                lower = core.utc(anchor["created_at"])
                candidates.append((anchor, anchor_state))
                evidence.append(anchor_state)
        window = history_runs(api, lower)
        if created >= lower and not any(run.get("id") == identifier for run in window):
            raise RecoveryRefused("service history omits the independently known current execution")
        for run in window:
            if run.get("id") == identifier and run.get("run_attempt") == attempt:
                continue
            if (
                not isinstance(run.get("repository"), dict)
                or run["repository"].get("full_name") != api.repository
                or run.get("path") != core.WORKFLOW_PATH
                or run.get("head_branch") != branch
            ):
                raise RecoveryRefused("service history has unsupported caller provenance")
            source = immutable_service_source(run)
            if source != implementation:
                continue
            if core.utc(run.get("created_at")) < lower:
                raise RecoveryRefused("service history escaped its exact checkpoint time window")
            if run.get("run_attempt") != 1:
                fence = "prior same-C service attempts are ambiguous; effect budget is suspended"
                continue
            if run.get("status") != "completed":
                if not unstarted_service(api, run):
                    pending.append({"run_id": run["id"], "run_attempt": run["run_attempt"]})
                continue
            if run.get("conclusion") != "success":
                if not unstarted_service(api, run) and not skipped_service(
                    api, run, implementation, branch
                ):
                    fence = (
                        "prior same-C service may have effected without terminal budget evidence"
                    )
                continue
            try:
                state = checkpoint_payload(api, run, implementation, branch, floor, now)
            except ValueError as error:
                fence = str(error)
                continue
            evidence.append(state)
            if state["recovery_frontier"]["authority"]:
                candidates.append((run, state))
        if candidates:
            checkpoint, selected = max(
                candidates,
                key=lambda item: (
                    core.utc(item[0]["created_at"]),
                    item[0]["id"],
                ),
            )
        if selected is None and evidence:
            selected = evidence[-1]
        if selected is not None:
            selected = merge_budgets(selected, evidence)
        for state in evidence:
            inherited = state.get("recovery_frontier", {}).get("fence")
            if inherited:
                fence = inherited
        # A lost all-artifact history cannot become a clean empty checkpoint next hour.
        if previous is not None and previous.get("recovery_frontier", {}).get("fence"):
            fence = previous["recovery_frontier"]["fence"]
    except ValueError as error:
        fence = str(error)
    if attempt != 1:
        fence = "current service rerun has unresolved prior attempts; effect budget is suspended"
    ordered = all(
        core.utc(run["created_at"]) <= created and run["id"] < identifier for run, _ in candidates
    )
    active = floor is not None and created >= floor
    authority = active and ordered and not pending
    frontier = {
        "protocol": 1,
        "floor": None if floor is None else floor.isoformat(),
        "service_created_at": created.isoformat(),
        "authority": authority,
        "fence": fence,
        "pending": pending,
        "predecessor_run_id": None if checkpoint is None else checkpoint["id"],
    }
    if not ordered:
        frontier["refusal"] = (
            "an independently verified newer checkpoint forbids this older execution"
        )
    elif not active:
        frontier["refusal"] = "current service predates the immutable C effect floor"
    return ServiceContext(
        selected,
        frontier,
        created,
        identifier,
        attempt,
        authority and fence is None and attempt == 1,
        authority and fence is None and attempt == 1 and checkpoint is not None,
        checkpoint,
    )


def require_effect(
    api: core.GitHub, context: ServiceContext, implementation: str, branch: str
) -> None:
    """Fresh exact-attempt and monotonic checks immediately before either action path."""
    if not context.can_effect or context.checkpoint_run is None:
        raise RecoveryRefused("no completed clean same-C checkpoint authorizes this service effect")
    current = api.get(f"actions/runs/{context.run_id}")
    checkpoint = api.get(f"actions/runs/{context.checkpoint_run['id']}")
    if (
        current.get("id") != context.run_id
        or current.get("status") != "in_progress"
        or "conclusion" not in current
        or current["conclusion"] is not None
        or current.get("run_attempt") != context.attempt
        or context.attempt != 1
        or core.utc(current.get("created_at")) != context.created_at
        or checkpoint.get("id") != context.checkpoint_run.get("id")
        or immutable_service_source(current) != implementation
        or current.get("path") != core.WORKFLOW_PATH
        or current.get("head_branch") != branch
        or checkpoint.get("run_attempt") != context.checkpoint_run.get("run_attempt")
        or not core.trusted_run(api, checkpoint, implementation, branch)
        or core.utc(checkpoint.get("created_at")) > context.created_at
        or checkpoint.get("id", context.run_id) >= context.run_id
    ):
        context.frontier["fence"] = "effect-time actor/checkpoint changed; budget suspended"
        context.can_effect = False
        context.can_prepare = False
        raise RecoveryRefused(str(context.frontier["fence"]))


def reader_reference(run: dict[str, Any], revision: str) -> None:
    values = run.get("referenced_workflows")
    if (
        not isinstance(values, list)
        or len(values) > 30
        or not all(isinstance(item, dict) and isinstance(item.get("path"), str) for item in values)
    ):
        raise RecoveryRefused("original immutable workflow references are malformed")
    references = [
        item
        for item in run.get("referenced_workflows", [])
        if isinstance(item, dict) and item.get("path", "").split("@", 1)[0] == READER_PATH
    ]
    if len(references) != 1 or any(
        references[0].get(key) != value
        for key, value in {
            "path": f"{READER_PATH}@{revision}",
            "sha": revision,
        }.items()
    ):
        raise RecoveryRefused("original run lacks the exact verified immutable reader reference")


def graph_proof(
    api: core.GitHub,
    run: dict[str, Any],
    candidate: core.Candidate,
    implementation: str,
    *,
    hint: dict[str, Any] | None = None,
) -> tuple[core.Candidate, str, str]:
    hint = candidate_hint(api, run, candidate) if hint is None else hint
    historical = core.Candidate(
        hint["base_sha"],
        hint["candidate_sha"],
        candidate.number,
        candidate.source_head,
        candidate.actor_login,
        candidate.actor_type,
        "pull_request",
    )
    if historical.base == candidate.base:
        raise RecoveryRefused("historical hint does not identify an older base")
    ordered_merge(api, historical.head, historical.base, candidate.source_head)
    ordered_merge(api, candidate.head, candidate.base, candidate.source_head)
    core.require_ancestor(api, historical.base, candidate.base)
    trees = {
        sha: inventory(api, sha)
        for sha in {
            historical.base,
            historical.head,
            candidate.source_head,
            candidate.base,
            candidate.head,
        }
    }
    old_authority = authority(api, historical.base, trees[historical.base])
    new_authority = authority(api, candidate.base, trees[candidate.base])
    if new_authority[0] != implementation:
        raise RecoveryRefused("current tracked authority does not equal executing trusted C")
    catalogs = {
        sha: {
            path: blob
            for path, blob in tree.items()
            if re.fullmatch(r"\.github/workflows/[^/]+\.ya?ml", path)
        }
        for sha, tree in trees.items()
    }
    if any(not catalog for catalog in catalogs.values()):
        raise RecoveryRefused("recovery workflow catalog is empty")
    path = core.workflow_file(api.repository)
    old_caller = required_file(api, path, historical.base, trees[historical.base])
    tested_caller = required_file(api, path, historical.head, trees[historical.head])
    current_caller = required_file(api, path, candidate.base, trees[candidate.base])
    # A candidate-controlled workflow change needs the existing exact pin-upgrade proof.
    reader_revision = old_authority[0]
    if tested_caller != old_caller:
        if not core.retry_ci_upgrade(api, historical, old_caller, tested_caller, implementation):
            raise RecoveryRefused(
                "historical caller changed outside the reviewed exact pin upgrade"
            )
        reader_revision = implementation
    reader_shape(tested_caller, reader_revision)
    reader_reference(run, reader_revision)
    reader_shape(current_caller, implementation)
    kind = "retained-graph"
    if old_authority != new_authority:
        kind = "handoff"
    if any(catalog != catalogs[historical.base] for catalog in catalogs.values()):
        # Changed catalogs must be explained by a genuine trusted authority transition or
        # the already proven exact original caller upgrade. Otherwise refuse, never label.
        if old_authority[0] == implementation and tested_caller == old_caller:
            raise RecoveryRefused("unexplained workflow catalog change refuses recovery")
        kind = "handoff"
    if kind == "retained-graph":
        reader_closure(api, implementation)
    return historical, kind, reader_revision


def label_unused(
    api: core.GitHub, number: int, now: datetime, *, source_head: str, since: str
) -> None:
    try:
        definition = api.get(f"labels/{REBASE_LABEL}")
    except core.MissingResource as error:
        raise RecoveryRefused(
            "provision the fixed repository rebase label before handoff"
        ) from error
    if definition.get("name") != REBASE_LABEL:
        raise RecoveryRefused("fixed Renovate rebase label metadata is untrusted")
    labels = api.items(f"issues/{number}/labels?per_page=100")
    if len(labels) >= 100:
        raise RecoveryRefused("current label listing is truncated; handoff budget is suspended")
    if any(label.get("name") == REBASE_LABEL for label in labels):
        raise RecoveryRefused("fixed rebase label is already present; awaiting Renovate")
    lower = core.utc(since)
    if lower > now:
        raise RecoveryRefused("original CI label-history boundary is in the future")
    owner, repository = api.repository.split("/", 1)
    query = """query RecoveryLabelHistory(
      $owner: String!, $name: String!, $number: Int!, $since: DateTime!
    ) {
      repository(owner:$owner,name:$name) {
        nameWithOwner
        pullRequest(number:$number) {
          number state headRefOid
          timelineItems(last:100,itemTypes:[LABELED_EVENT,UNLABELED_EVENT],since:$since) {
            pageInfo {hasPreviousPage hasNextPage startCursor endCursor}
            nodes {
              __typename
              ... on LabeledEvent {createdAt label {name}}
              ... on UnlabeledEvent {createdAt label {name}}
            }
          }
        }
      }
    }"""
    response = api.request(
        "POST",
        "/graphql",
        {
            "query": query,
            "variables": {
                "owner": owner,
                "name": repository,
                "number": number,
                "since": lower.isoformat(),
            },
        },
    )
    if not isinstance(response, dict) or response.get("errors"):
        raise RecoveryRefused("filtered label history query failed; handoff budget is suspended")
    data = response.get("data")
    repository_data = data.get("repository") if isinstance(data, dict) else None
    pr = repository_data.get("pullRequest") if isinstance(repository_data, dict) else None
    if (
        not isinstance(repository_data, dict)
        or repository_data.get("nameWithOwner") != api.repository
        or not isinstance(pr, dict)
        or pr.get("number") != number
        or pr.get("state") != "OPEN"
        or pr.get("headRefOid") != source_head
    ):
        raise RecoveryRefused("filtered label history does not identify the unchanged owned PR")
    timeline = pr.get("timelineItems")
    page = timeline.get("pageInfo") if isinstance(timeline, dict) else None
    nodes = timeline.get("nodes") if isinstance(timeline, dict) else None
    if (
        not isinstance(page, dict)
        or page.get("hasPreviousPage") is not False
        or page.get("hasNextPage") is not False
        or not isinstance(nodes, list)
        or len(nodes) > 100
    ):
        raise RecoveryRefused("filtered label history is incomplete; handoff budget is suspended")
    # GitHub's totalCount can describe the unfiltered timeline even with since/itemTypes.
    # Completeness comes from both pageInfo flags, never equality with totalCount.
    for event in nodes:
        if (
            not isinstance(event, dict)
            or event.get("__typename") not in {"LabeledEvent", "UnlabeledEvent"}
            or not isinstance(event.get("label"), dict)
            or not isinstance(event["label"].get("name"), str)
        ):
            raise RecoveryRefused("filtered label history contains malformed/unsupported events")
        instant = core.utc(event.get("createdAt"))
        if not lower <= instant <= now:
            raise RecoveryRefused("filtered label history has an invalid timestamp")
        if event["label"]["name"] == REBASE_LABEL:
            raise RecoveryRefused(
                "fixed rebase handoff was already consumed; awaiting a fresh head"
            )


def historical_report(
    api: core.GitHub,
    historical: core.Candidate,
    state: dict[str, Any],
    previous: dict[str, Any] | None,
    reports: dict[str, Any],
    service_run: int,
    now: datetime,
) -> tuple[dict[str, Any], dict[str, Any]]:
    report = core.evaluate(api, historical, state, now)
    state = report.pop("state")
    report["evidence_kind"] = "independently-evaluated-historical-hint"
    previous_report = None if previous is None else previous["reports"].get(historical.head)
    receipt = (
        previous["run_id"]
        if previous is not None and report_matches(previous_report, historical)
        else service_run
    )
    core.report_status(api, report, receipt)
    reports[historical.head] = report
    return report, state


def consume(
    api: core.GitHub,
    run: dict[str, Any],
    candidate: core.Candidate,
    reservation: dict[str, Any],
    reader_revision: str | None,
    service_run: int,
    branch: str,
    now: datetime,
    context: ServiceContext,
    *,
    retained: RetainedBinding | None = None,
) -> None:
    identifier = run["id"]
    label_handoff = reservation["kind"] in {"handoff", "native-legacy-handoff"}
    if retained is None:
        require_effect(api, context, reservation["authority"], branch)
        original_run(api, identifier, candidate, reader_revision)
        protection(api, branch)
        if not core.still_current(api, candidate, branch):
            raise RecoveryRefused("PR changed before consuming the prepared recovery")
    else:
        fresh = retained_proof(api, candidate.number, reservation["authority"], branch)
        if fresh != retained or reservation.get("original_identity_sha256") != fresh.identity:
            raise RecoveryRefused("retained original changed before consuming prepared recovery")
        protection(api, branch)
        if retained_pr(api, candidate.number, branch) != RetainedBinding(
            fresh.historical, fresh.target, fresh.identity
        ):
            raise RecoveryRefused("retained source or target changed before the effect")
        original = original_run(
            api, identifier, candidate, reader_revision, excluded_jobs=fresh.excluded_jobs
        )
        hint = candidate_hint(api, original, candidate)
        if (hint["candidate_sha"], hint["base_sha"]) != (candidate.head, candidate.base):
            raise RecoveryRefused("original hint changed before consuming prepared recovery")
        # The retained closed proof is more expensive than ordinary resolution.
        # Recheck the actor/checkpoint immediately after it, not before that work.
        require_effect(api, context, reservation["authority"], branch)
    if label_handoff:
        label_unused(
            api, candidate.number, now, source_head=candidate.source_head, since=run["created_at"]
        )
    reservation["phase"] = "attempted"
    reservation["effect_service_run"] = service_run
    try:
        if label_handoff:
            api.post(f"issues/{candidate.number}/labels", {"labels": [REBASE_LABEL]})
        else:
            api.post(f"actions/runs/{identifier}/rerun-failed-jobs", {})
    except ValueError:
        reservation["phase"] = "uncertain"
        # Reconciliation only: a timeout can never trigger another POST.
        if label_handoff:
            reservation["observed_labels"] = [
                label.get("name")
                for label in api.items(f"issues/{candidate.number}/labels?per_page=100")
            ][:100]
        else:
            observed = api.get(f"actions/runs/{identifier}")
            reservation["observed_attempt"] = observed.get("run_attempt")


def native_caller(content: bytes) -> None:
    """Recognize only the reviewed historical native 72-hour admission shape."""
    jobs = job_blocks(content)
    age = jobs.get("lockfile-release-age", "")
    tokens = (
        "    name: Lockfile release age\n",
        "    needs: validation-plan\n",
        "    steps:\n",
        "    if: needs.validation-plan.outputs.select_lockfile_release_age == 'true'\n",
        "      - name: Reject newly introduced dependencies younger than three days\n",
        "python3 .github/strukturpiloten-shared/scripts/lockfile_release_age.py",
        '--base "${BASE_SHA}" --head "${HEAD_SHA}" --minimum-age-hours 72',
        "BASE_SHA: ${{ github.event.pull_request.base.sha || github.event.merge_group.base_sha }}",
        "HEAD_SHA: ${{ github.event.pull_request.head.sha || github.event.merge_group.head_sha }}",
    )
    aggregates = [job for job in jobs.values() if "    name: Required CI\n" in job]
    if (
        any(token not in age for token in tokens)
        or READER_PATH.encode() in content
        or re.search(r"(?m)^    uses:", age)
        or len(aggregates) != 1
        or any(
            token not in aggregates[0]
            for token in (
                "    if: always()",
                "lockfile-release-age",
                "validation-plan",
                '--tested-sha "${GITHUB_SHA}"',
                '--plan-json "${PLAN_JSON}"',
                '--needs-json "${NEEDS_JSON}"',
            )
        )
    ):
        raise RecoveryRefused("no-hint source is outside the reviewed native admission profile")


def native_age_failure(api: core.GitHub, run: dict[str, Any]) -> None:
    listing = api.get(f"actions/runs/{run['id']}/attempts/{run['run_attempt']}/jobs?per_page=100")
    jobs = listing.get("jobs")
    if not isinstance(jobs, list) or len(jobs) > 100 or listing.get("total_count") != len(jobs):
        raise RecoveryRefused("native admission attempt job listing is incomplete")
    age = [
        job for job in jobs if isinstance(job, dict) and job.get("name") == "Lockfile release age"
    ]
    if len(age) != 1 or any(
        age[0].get(key) != value
        for key, value in {
            "run_id": run["id"],
            "run_attempt": run["run_attempt"],
            "status": "completed",
            "conclusion": "failure",
        }.items()
    ):
        raise RecoveryRefused("native admission failure lacks the exact original run/attempt")
    url = age[0].get("check_run_url", "")
    match = re.fullmatch(
        rf"https://api\.github\.com/repos/{re.escape(api.repository)}/check-runs/([0-9]+)", url
    )
    if not match:
        raise RecoveryRefused("native admission check URL is outside the fixed repository API")
    check = api.get(f"check-runs/{match[1]}")
    app = check.get("app")
    if (
        not isinstance(app, dict)
        or check.get("id") != int(match[1])
        or check.get("name") != "Lockfile release age"
        or check.get("head_sha") != run["head_sha"]
        or app.get("id") != ACTIONS_APP
        or app.get("slug") != "github-actions"
        or check.get("status") != "completed"
        or check.get("conclusion") != "failure"
    ):
        raise RecoveryRefused("native admission failure lacks exact GitHub Actions provenance")
    steps = age[0].get("steps")
    if (
        not isinstance(steps, list)
        or not 1 <= len(steps) <= 100
        or not all(
            isinstance(step, dict)
            and step.get("status") == "completed"
            and step.get("conclusion") in {"success", "skipped", "failure"}
            for step in steps
        )
        or [step.get("name") for step in steps if step["conclusion"] == "failure"]
        != ["Reject newly introduced dependencies younger than three days"]
    ):
        raise RecoveryRefused("native failure is outside the known admission policy step")


def renovate_commit(commit: dict[str, Any]) -> bool:
    author, committer = commit.get("author"), commit.get("committer")
    if (
        not isinstance(author, dict)
        or author.get("login") != "renovate[bot]"
        or author.get("type") != "Bot"
        or not isinstance(committer, dict)
    ):
        return False
    if committer.get("login") == "renovate[bot]" and committer.get("type") == "Bot":
        return True
    record = commit.get("commit")
    verification = record.get("verification") if isinstance(record, dict) else None
    return (
        committer.get("login") == "web-flow"
        and committer.get("type") == "User"
        and isinstance(verification, dict)
        and verification.get("verified") is True
        and verification.get("reason") == "valid"
    )


def version_only(before: object, after: object, path: tuple[str, ...] = ()) -> bool:
    if before == after:
        return True
    if isinstance(before, dict) and isinstance(after, dict) and before.keys() == after.keys():
        return all(version_only(before[key], after[key], (*path, key)) for key in before)
    sections = {
        "dependencies",
        "dev-dependencies",
        "build-dependencies",
        "devDependencies",
        "optionalDependencies",
        "peerDependencies",
    }
    return (
        isinstance(before, str)
        and isinstance(after, str)
        and (
            (len(path) >= 2 and path[-2] in sections)
            or (len(path) >= 3 and path[-3] in sections and path[-1] == "version")
        )
    )


def python_pin_changes(before: object, after: object) -> set[tuple[str, str, str]]:
    """Compare only existing exact dotted package pins at four finite TOML locations."""
    normal: set[tuple[str, str, str]] = set()
    builders: set[tuple[str, str, str]] = set()
    pin = re.compile(r"([A-Za-z0-9][A-Za-z0-9._-]*)==([0-9]+(?:\.[0-9]+)+)")

    def compare(old: object, new: object, path: tuple[str, ...]) -> None:
        if old == new:
            return
        if isinstance(old, dict) and isinstance(new, dict) and old.keys() == new.keys():
            for key in old:
                compare(old[key], new[key], (*path, key))
            return
        allowed = (
            path in {("project", "dependencies"), ("build-system", "requires")}
            or (len(path) == 3 and path[:2] == ("project", "optional-dependencies"))
            or (len(path) == 2 and path[0] == "dependency-groups")
        )
        if (
            not allowed
            or not isinstance(old, list)
            or not isinstance(new, list)
            or len(old) != len(new)
        ):
            raise RecoveryRefused("Python source changed outside exact dependency-version pins")
        for previous, current in zip(old, new, strict=True):
            if previous == current:
                continue
            left = pin.fullmatch(previous) if isinstance(previous, str) else None
            right = pin.fullmatch(current) if isinstance(current, str) else None
            if left is None or right is None or left[1] != right[1]:
                raise RecoveryRefused("Python dependency pin syntax, name or order is unsupported")
            changed = (left[1], left[2], right[2])
            (builders if path == ("build-system", "requires") else normal).add(changed)

    compare(before, after, ())
    if not normal or not builders <= normal:
        raise RecoveryRefused("Python builder must mirror a changed locked normal/dev package pin")
    return normal


def python_lock_coverage(
    api: core.GitHub,
    path: str,
    changes: set[tuple[str, str, str]],
    base: str,
    source: str,
    files: list[Any],
    trees: dict[str, dict[str, str]],
) -> None:
    """Use the normal PyPI identity/integrity decoder, never any changed lock bytes."""
    from .dependencies import _uv

    lock = "automation/uv.lock" if path == "automation/pyproject.toml" else "uv.lock"
    if not any(
        isinstance(item, dict)
        and item.get("filename") == lock
        and item.get("status") == "modified"
        and item.get("previous_filename") is None
        for item in files
    ):
        raise RecoveryRefused("Python pins require their modified same-project uv.lock")
    with tempfile.TemporaryDirectory(prefix="boxferry-retained-python-metadata-") as temporary:
        root = Path(temporary)
        before, after = root / "before.lock", root / "after.lock"
        before.write_bytes(required_file(api, lock, base, trees[base]))
        after.write_bytes(required_file(api, lock, source, trees[source]))
        old, new = _uv(before), _uv(after)
    for name, old_version, new_version in changes:
        normalized = re.sub(r"[-_.]+", "-", name).lower()
        previous = {package for package in old if package.name == normalized}
        current = {package for package in new if package.name == normalized}
        if len(previous) != 1 or len(current) != 1:
            raise RecoveryRefused(
                "Python pin coverage requires one canonical locked package per name"
            )
        left, right = next(iter(previous)), next(iter(current))
        if (
            left.version != old_version
            or right.version != new_version
            or old_version == new_version
            or left.ecosystem != "python"
            or right.ecosystem != "python"
            or left.source != "https://pypi.org"
            or right.source != "https://pypi.org"
            or not left.integrity
            or not right.integrity
        ):
            raise RecoveryRefused(
                "Python pin versions do not match exact old/new locked identities"
            )


def ordinary_native_changes(
    api: core.GitHub,
    comparison: dict[str, Any],
    candidate: core.Candidate,
    merge_base: str,
    trees: dict[str, dict[str, str]],
    modes: dict[str, dict[str, str]],
    *,
    python_manifests: bool = False,
) -> None:
    commits, files = comparison.get("commits"), comparison.get("files")
    if (
        not isinstance(commits, list)
        or not 1 <= len(commits) <= 20
        or type(comparison.get("total_commits")) is not int
        or type(comparison.get("ahead_by")) is not int
        or comparison.get("total_commits") != len(commits)
        or comparison.get("ahead_by") != len(commits)
        or not all(
            isinstance(commit, dict)
            and isinstance(commit.get("sha"), str)
            and core.SHA.fullmatch(commit["sha"]) is not None
            and renovate_commit(commit)
            for commit in commits
        )
        or not any(commit.get("sha") == candidate.source_head for commit in commits)
    ):
        raise RecoveryRefused(
            "native branch has unknown/human changes or incomplete commit ownership"
        )
    tip = api.get(f"commits/{candidate.source_head}")
    if tip.get("sha") != candidate.source_head or not renovate_commit(tip):
        raise RecoveryRefused("native branch tip is not the exact owned Renovate commit")
    if not isinstance(files, list) or not 1 <= len(files) < 300:
        raise RecoveryRefused("native ordinary dependency file scope is incomplete")
    seen: set[str] = set()
    for item in files:
        path = item.get("filename") if isinstance(item, dict) else None
        if (
            not isinstance(path, str)
            or path in seen
            or item.get("status") != "modified"
            or item.get("previous_filename") is not None
        ):
            raise RecoveryRefused(
                "native handoff requires unchanged closed dependency file ownership"
            )
        seen.add(path)
        cargo = path == "Cargo.toml" or re.fullmatch(r"crates/[^/]+/Cargo.toml", path) is not None
        npm = path in {"package.json", "automation/devcontainer/package.json"}
        python = python_manifests and path in {"pyproject.toml", "automation/pyproject.toml"}
        if path not in core.LOCK_PATHS and not cargo and not npm and not python:
            raise RecoveryRefused(
                "retained source supports only the reviewed package manifests and known locks"
                if python_manifests
                else "native handoff excludes publication or unrelated product changes"
            )
        if (
            path not in trees[merge_base]
            or path not in trees[candidate.source_head]
            or (modes[merge_base][path] != modes[candidate.source_head][path])
        ):
            raise RecoveryRefused("native dependency file is new, missing or changed mode")
        if cargo or npm or python:
            before = required_file(api, path, merge_base, trees[merge_base])
            after = required_file(api, path, candidate.source_head, trees[candidate.source_head])
            if cargo or python:
                old, new = tomllib.loads(before.decode()), tomllib.loads(after.decode())
            else:
                old = json.loads(before, object_pairs_hook=unique_object)
                new = json.loads(after, object_pairs_hook=unique_object)
            if python:
                python_lock_coverage(
                    api,
                    path,
                    python_pin_changes(old, new),
                    merge_base,
                    candidate.source_head,
                    files,
                    trees,
                )
            elif not version_only(old, new):
                raise RecoveryRefused("native manifest includes a non-dependency-version change")


def native_proof(
    api: core.GitHub,
    run: dict[str, Any],
    candidate: core.Candidate,
    implementation: str,
    branch: str,
) -> dict[str, str]:
    """Prove a current-only transition; M is never described as old tested P/X/A."""
    if (
        candidate.scope != "pull_request"
        or candidate.actor_login != "renovate[bot]"
        or candidate.actor_type != "Bot"
        or core.pr_candidate(api, candidate.number, expected_target=branch) != candidate
    ):
        raise RecoveryRefused("native handoff requires the exact current owned ordinary PR")
    pull = api.get(f"pulls/{candidate.number}")
    identity = core.pr_identity(pull, api.repository, candidate.number, expected_target=branch)
    head = pull.get("head")
    if (
        identity != (branch, candidate.head, candidate.source_head, "renovate[bot]", "Bot")
        or pull.get("draft") is not False
        or not isinstance(head, dict)
        or not isinstance(head.get("repo"), dict)
        or head["repo"].get("full_name") != api.repository
        or not isinstance(head.get("ref"), str)
        or re.fullmatch(r"renovate/[A-Za-z0-9._/-]+", head["ref"]) is None
    ):
        raise RecoveryRefused("native handoff refuses foreign, modified or nonordinary ownership")
    if hint_artifact(api, run) is not None:
        raise RecoveryRefused("native handoff requires complete true hint absence")
    comparison = api.get(f"compare/{candidate.base}...{candidate.source_head}")
    base = comparison.get("merge_base_commit")
    merge_base = core.revision(base.get("sha") if isinstance(base, dict) else None)
    base_commit = comparison.get("base_commit")
    if (
        not isinstance(base_commit, dict)
        or base_commit.get("sha") != candidate.base
        or comparison.get("status") not in {"ahead", "diverged"}
        or merge_base == candidate.base
    ):
        raise RecoveryRefused("native handoff has no genuine admitted reader transition")
    core.require_ancestor(api, merge_base, candidate.base)
    core.require_ancestor(api, merge_base, candidate.source_head)
    ordered_merge(api, candidate.head, candidate.base, candidate.source_head)
    modes: dict[str, dict[str, str]] = {
        sha: {}
        for sha in {
            merge_base,
            candidate.source_head,
            candidate.base,
            candidate.head,
        }
    }
    trees = {sha: inventory(api, sha, modes=modes[sha]) for sha in modes}
    catalogs = {
        sha: {
            path: (blob, modes[sha][path])
            for path, blob in tree.items()
            if re.fullmatch(r"\.github/workflows/[^/]+\.ya?ml", path)
        }
        for sha, tree in trees.items()
    }
    if (
        not catalogs[merge_base]
        or catalogs[candidate.source_head] != catalogs[merge_base]
        or catalogs[candidate.base] == catalogs[merge_base]
        or catalogs[candidate.head] != catalogs[candidate.base]
    ):
        raise RecoveryRefused("native source/current workflow catalogs do not prove a transition")
    manifests: dict[str, tuple[str, str] | None] = {}
    for sha, tree in trees.items():
        if MANIFEST_PATH not in tree:
            manifests[sha] = None
        else:
            content = required_file(api, MANIFEST_PATH, sha, tree)
            if len(content) > 8192:
                raise RecoveryRefused("native manifest record exceeds its bound")
            manifests[sha] = (modes[sha][MANIFEST_PATH], hashlib.sha256(content).hexdigest())
    if (
        manifests[candidate.source_head] != manifests[merge_base]
        or manifests[candidate.head] != manifests[candidate.base]
    ):
        raise RecoveryRefused(
            "native manifest presence/mode/bytes changed outside the trusted base"
        )
    path = core.workflow_file(api.repository)
    native_caller(required_file(api, path, merge_base, trees[merge_base]))
    native_age_failure(api, run)
    selected = authority(api, candidate.base, trees[candidate.base])
    if selected[0] != implementation:
        raise RecoveryRefused("current native handoff base does not select admitted C")
    reader_shape(required_file(api, path, candidate.base, trees[candidate.base]), implementation)
    reader_closure(api, implementation)
    ordinary_native_changes(api, comparison, candidate, merge_base, trees, modes)
    fingerprint = hashlib.sha256(
        json.dumps(
            {
                "merge_base": merge_base,
                "legacy_catalog": catalogs[merge_base],
                "current_catalog": catalogs[candidate.base],
                "legacy_manifest": manifests[merge_base],
                "current_manifest": manifests[candidate.base],
                "authority": implementation,
                "policy": [selected[1], hashlib.sha256(selected[2]).hexdigest()],
            },
            sort_keys=True,
        ).encode()
    ).hexdigest()
    return {"merge_base_sha": merge_base, "transition_fingerprint": fingerprint}


def native_handoff(
    api: core.GitHub,
    run: dict[str, Any],
    candidate: core.Candidate,
    report: dict[str, Any],
    previous: dict[str, Any] | None,
    reservations: dict[str, Any],
    key: str,
    *,
    implementation: str,
    service_run: int,
    branch: str,
    now: datetime,
    context: ServiceContext,
) -> None:
    if not report_matches(report, candidate):
        raise RecoveryRefused("native no-hint handoff requires a full current N/B eligibility pass")
    if core.utc(run.get("created_at")) > context.created_at:
        raise RecoveryRefused("earlier-created service cannot target a future original CI run")
    proof = native_proof(api, run, candidate, implementation, branch)
    facts = protection(api, branch)
    bindings = {
        "kind": "native-legacy-handoff",
        "original_run_id": run["id"],
        "original_attempt": run["run_attempt"],
        "source_head_sha": candidate.source_head,
        "authority": implementation,
        **proof,
    }
    reservation = reservations.get(key)
    if reservation is None:
        reservations[key] = {
            **bindings,
            "phase": "prepared",
            "prepared_at": now.isoformat(),
            "prepared_service_run": service_run,
            "protection": facts,
        }
        return
    if any(reservation.get(name) != value for name, value in bindings.items()):
        raise RecoveryRefused("native handoff transition changed; original budget is suspended")
    prior = None if previous is None else previous.get("recoveries", {}).get(key)
    if (
        not context.can_effect
        or previous is None
        or previous.get("implementation_revision") != implementation
        or not isinstance(prior, dict)
        or prior.get("phase") != "prepared"
        or any(prior.get(name) != value for name, value in bindings.items())
        or not report_matches(previous["reports"].get(candidate.head), candidate)
    ):
        return
    if native_proof(api, run, candidate, implementation, branch) != proof:
        raise RecoveryRefused("native transition drifted immediately before handoff")
    consume(api, run, candidate, reservation, None, service_run, branch, now, context)


def recover_run(
    api: core.GitHub,
    identifier: int,
    candidate: core.Candidate,
    report: dict[str, Any],
    state: dict[str, Any],
    previous: dict[str, Any] | None,
    reservations: dict[str, Any],
    reports: dict[str, Any],
    *,
    implementation: str,
    service_run: int,
    branch: str,
    now: datetime,
    context: ServiceContext,
) -> dict[str, Any]:
    if not context.can_prepare:
        return state
    if previous is not None and identifier in previous.get("reruns", {}).values():
        # The normal equal-base path already consumed this original run's budget.
        # Do not depend on eventual run_attempt propagation or a changing merge SHA.
        return state
    key = hashlib.sha256(
        f"{api.repository}:{identifier}:{candidate.source_head}:{implementation}".encode()
    ).hexdigest()
    reservation = reservations.get(key)
    if reservation is not None and reservation.get("phase") != "prepared":
        return state
    try:
        run = original_run(api, identifier, candidate, None)
        artifact = hint_artifact(api, run)
        if artifact is None:
            native_handoff(
                api,
                run,
                candidate,
                report,
                previous,
                reservations,
                key,
                implementation=implementation,
                service_run=service_run,
                branch=branch,
                now=now,
                context=context,
            )
            return state
        hint = candidate_hint(api, run, candidate, artifact=artifact)
        if hint["base_sha"] == candidate.base:
            return state  # The unchanged ordinary equal-base path owns this run.
        historical, kind, reader_revision = graph_proof(
            api, run, candidate, implementation, hint=hint
        )
        facts = protection(api, branch)
        old_report, state = historical_report(
            api, historical, state, previous, reports, service_run, now
        )
        if not core.still_current(api, candidate, branch):
            raise RecoveryRefused("source/current candidate changed during historical evaluation")
        # Queued workflows can be created before a preparation's wall-clock timestamp.
        # Scan from the original CI creation, including failed and cancelled executions.
        if core.utc(run.get("created_at")) > context.created_at:
            raise RecoveryRefused("earlier-created service cannot target a future original CI run")
        bindings = {
            "original_run_id": identifier,
            "original_attempt": run["run_attempt"],
            "source_head_sha": candidate.source_head,
            "authority": implementation,
            "hinted_candidate_sha": historical.head,
            "hinted_base_sha": historical.base,
            "kind": kind,
        }
        if reservation is None:
            reservations[key] = {
                **bindings,
                "phase": "prepared",
                "prepared_at": now.isoformat(),
                "prepared_service_run": service_run,
                "protection": facts,
            }
            return state
        if any(reservation.get(name) != value for name, value in bindings.items()):
            raise RecoveryRefused("prepared recovery binding changed; retry budget is suspended")
        previous_reports = {} if previous is None else previous["reports"]
        current = (old_report, report)
        prior = (previous_reports.get(historical.head), previous_reports.get(candidate.head))
        if kind == "retained-graph":
            ready = all(
                report_matches(value, expected)
                for value, expected in zip(
                    (*current, *prior), (historical, candidate, historical, candidate), strict=True
                )
            )
        else:
            # A label requests a fresh graph; it does not approve either graph or waive its hold.
            ready = all(
                report_matches(value, expected, frozenset({"pass", "waiting"}))
                for value, expected in zip(
                    (*current, *prior), (historical, candidate, historical, candidate), strict=True
                )
            )
        if ready and context.can_effect:
            consume(
                api, run, candidate, reservation, reader_revision, service_run, branch, now, context
            )
        return state
    except ValueError as error:
        if reservation is not None:
            reservation["phase"] = "suspended"
            reservation["reason"] = str(error)
        raise RecoveryRefused(str(error)) from error


class RetainedBindingChanged(RecoveryRefused):
    """An observed identity/target movement cannot restart resolution within this actor."""


@dataclass(frozen=True)
class RetainedBinding:
    """A historical PR graph and observed live target, never a current merge candidate."""

    historical: core.Candidate
    target: str
    identity: str
    excluded_jobs: frozenset[str] = frozenset()


def static_pr_exclusions(repository: str, caller: bytes, policy: bytes) -> frozenset[str]:
    """Recognize only three reviewed false-on-PR job shapes, not selector expressions."""
    blocks = job_blocks(caller)
    try:
        jobs = json.loads(policy, object_pairs_hook=unique_object)["jobs"]
    except (ValueError, KeyError, TypeError) as error:
        raise RecoveryRefused(
            "static event exclusions lack a trusted policy job inventory"
        ) from error
    if (
        not isinstance(jobs, list)
        or not 1 <= len(jobs) <= 100
        or not all(isinstance(job, str) and re.fullmatch(r"[A-Za-z0-9_-]+", job) for job in jobs)
        or len(set(jobs)) != len(jobs)
    ):
        raise RecoveryRefused("static event exclusions have malformed policy jobs")
    shapes: tuple[tuple[str, str, str], ...] = ()
    if repository == "Strukturpiloten/docker-lens":
        shapes = (
            (
                "hosted-prerequisite-diagnostic",
                "Hosted prerequisite diagnostic (not native validation)",
                "github.event_name == 'workflow_dispatch' && github.ref == 'refs/heads/main'",
            ),
            (
                "native-conformance",
                "Native Engine ${{ matrix.lane }}",
                "github.event_name == 'push' && github.ref == 'refs/heads/main'",
            ),
        )
    elif repository == "Strukturpiloten/boxferry-website":
        shapes = (
            (
                "publish-site",
                "Publish validated website",
                ">-\n      github.event_name == 'push' && github.ref == 'refs/heads/main' &&\n"
                "      needs.pr-gate.result == 'success'",
            ),
        )
    excluded: set[str] = set()
    for identifier, name, condition in shapes:
        if identifier not in blocks:
            continue
        block = blocks[identifier]
        names = re.findall(r"(?m)^    name: ([^\n]+)$", block)
        conditions = re.findall(r"(?m)^    if: ([^\n]*(?:\n      [^\n]+)*)", block)
        if names != [name] or conditions != [condition] or identifier in jobs:
            raise RecoveryRefused("non-PR job differs from its exact trusted event shape")
        if sum(f"    name: {name}\n" in value for value in blocks.values()) != 1:
            raise RecoveryRefused("static excluded job name is ambiguous")
        if identifier == "native-conformance":
            strategy = re.findall(r"(?m)^    strategy:\n((?:      [^\n]*\n)+)", block)
            expected = (
                "      fail-fast: false\n      matrix:\n        lane:\n          [\n"
                "            debian11-rootful,\n            debian11-rootless,\n"
                "            upstream-rootful,\n            upstream-rootless,\n          ]\n"
            )
            if strategy != [expected]:
                raise RecoveryRefused("native event exclusion has an unsupported matrix shape")
            gate = blocks.get("check-gate", "")
            revisions = re.findall(
                rf"(?m)^    uses: {re.escape(READER_PATH)}@([a-f0-9]{{40}})"
                r"(?:[ \t]+#[^\n]*)?$",
                blocks.get("lockfile-release-age", ""),
            )
            checkouts = re.findall(
                r"(?m)^        uses: (actions/checkout@[a-f0-9]{40} # v[0-9]+\.[0-9]+\.[0-9]+)$",
                gate,
            )
            if len(revisions) != 1 or len(checkouts) != 1:
                raise RecoveryRefused("native aggregate lacks its exact immutable pin bindings")
            expected_gate = _DOCKER_REQUIRED_GATE.replace(
                "__ADMISSION_SHA__", revisions[0]
            ).replace("__CHECKOUT_PIN__", checkouts[0])
            if gate.rstrip("\n") != expected_gate.rstrip("\n"):
                raise RecoveryRefused(
                    "native event exclusion requires the complete reviewed aggregate"
                )
        if identifier == "publish-site" and (
            "    needs: [pr-gate]\n" not in block
            or "    needs: [validation-plan, quality, lockfile-release-age]\n"
            not in blocks.get("pr-gate", "")
        ):
            raise RecoveryRefused("publisher exclusion is not downstream of the fixed PR gate")
        excluded.add(name)
    return frozenset(excluded)


def retained_pr(api: core.GitHub, number: int, branch: str) -> RetainedBinding:
    """Positively prove the stale [P,H] graph; the live candidate resolver stays strict."""

    def identity(pull: dict[str, Any]) -> tuple[tuple[str, str, str, str, str], str]:
        value = core.pr_identity(pull, api.repository, number, expected_target=branch)
        head, title = pull.get("head"), pull.get("title")
        if (
            value[3:] != ("renovate[bot]", "Bot")
            or pull.get("draft") is not False
            or not isinstance(title, str)
            or not 1 <= len(title) <= 512
            or not isinstance(head, dict)
            or not isinstance(head.get("repo"), dict)
            or head["repo"].get("full_name") != api.repository
            or not isinstance(head.get("ref"), str)
            or re.fullmatch(r"renovate/[A-Za-z0-9._/-]+", head["ref"]) is None
        ):
            raise RecoveryRefused("retained original requires an unchanged owned Renovate PR")
        fingerprint = hashlib.sha256(
            json.dumps([value, title, head["ref"]], separators=(",", ":")).encode()
        ).hexdigest()
        return value, fingerprint

    observed, fingerprint = identity(api.get(f"pulls/{number}"))
    _, merge, source, actor, actor_type = observed
    target = core.target_revision(api, branch)
    commit = api.get(f"git/commits/{merge}")
    parents = commit.get("parents")
    if not isinstance(parents, list) or len(parents) != 2:
        raise RecoveryRefused("retained original lacks two ordered merge parents")
    parent = parents[0]
    base = core.revision(parent.get("sha") if isinstance(parent, dict) else None)
    ordered_merge(api, merge, base, source)
    if base == target:
        raise RecoveryRefused("retained original is not an older-base graph")
    core.require_ancestor(api, base, target)
    comparison = api.get(f"compare/{target}...{merge}")
    merge_base = comparison.get("merge_base_commit")
    if (
        comparison.get("status") != "diverged"
        or not isinstance(merge_base, dict)
        or merge_base.get("sha") != base
    ):
        raise RecoveryRefused("PR does not expose the supported stale original merge")
    try:
        unchanged = identity(api.get(f"pulls/{number}")) == (observed, fingerprint)
        unchanged = unchanged and core.target_revision(api, branch) == target
    except ValueError as error:
        raise RetainedBindingChanged("retained PR changed during proof") from error
    if not unchanged:
        raise RetainedBindingChanged("retained PR source, title or target moved during proof")
    return RetainedBinding(
        core.Candidate(base, merge, number, source, actor, actor_type, "pull_request"),
        target,
        fingerprint,
    )


def probe_retained(
    api: core.GitHub,
    number: int,
    branch: str,
    *,
    run: dict[str, Any] | None = None,
    source_head: str | None = None,
) -> RetainedBinding | None:
    """A bounded hint locates work; only affirmative stale proof selects recovery."""
    try:
        if run is None:
            if not isinstance(source_head, str) or core.SHA.fullmatch(source_head) is None:
                return None
            workflow = core.workflow_file(api.repository).rsplit("/", 1)[1]
            listing = api.get(
                f"actions/workflows/{workflow}/runs?head_sha={source_head}&per_page=20"
            )
            runs = listing.get("workflow_runs")
            if not isinstance(runs, list) or len(runs) > 20:
                return None
            run = next(
                (
                    item
                    for item in runs
                    if isinstance(item, dict) and core.candidate_run(api, item, source_head)
                ),
                None,
            )
            if run is None:
                return None
        artifact = hint_artifact(api, run)
        if artifact is None:
            return None  # No extra live PR/target reads for ordinary no-hint callers.
        binding = retained_pr(api, number, branch)
        hint = candidate_hint(api, run, binding.historical, artifact=artifact)
        if (hint["candidate_sha"], hint["base_sha"]) != (
            binding.historical.head,
            binding.historical.base,
        ):
            return None
        return binding
    except RetainedBindingChanged as error:
        raise core.EligibilityError("retained binding moved; next event must re-resolve") from error
    except ValueError:
        # A failed locator/proof grants no authority; the strict live resolver decides.
        return None


def retained_proof(
    api: core.GitHub, number: int, implementation: str, branch: str
) -> RetainedBinding:
    """Reuse closed source/reader and ordinary dependency ownership, without inventing N/B."""
    binding = retained_pr(api, number, branch)
    original = binding.historical
    comparison = api.get(f"compare/{original.base}...{original.source_head}")
    base_commit, merge_base = comparison.get("base_commit"), comparison.get("merge_base_commit")
    if not isinstance(base_commit, dict) or not isinstance(merge_base, dict):
        raise RecoveryRefused("retained source comparison lacks exact base/merge-base metadata")
    source_base = core.revision(merge_base.get("sha"))
    if (
        base_commit.get("sha") != original.base
        or comparison.get("status") not in {"ahead", "diverged"}
        or (comparison["status"] == "ahead") != (source_base == original.base)
    ):
        raise RecoveryRefused("retained source comparison is not anchored to the original base")
    if source_base != original.base:
        core.require_ancestor(api, source_base, original.base)
        core.require_ancestor(api, source_base, original.source_head)
    modes: dict[str, dict[str, str]] = {
        sha: {}
        for sha in {original.base, original.head, original.source_head, binding.target, source_base}
    }
    trees = {sha: inventory(api, sha, modes=modes[sha]) for sha in modes}
    catalogs = [
        {
            path: (blob, modes[sha][path])
            for path, blob in tree.items()
            if re.fullmatch(r"\.github/workflows/[^/]+\.ya?ml", path)
        }
        for sha, tree in trees.items()
    ]
    if not all(catalogs) or any(catalog != catalogs[0] for catalog in catalogs):
        raise RecoveryRefused("retained original workflow catalogs changed; transition required")
    selected = authority(api, original.base, trees[original.base])
    if selected[0] != implementation or any(
        authority(api, sha, tree) != selected for sha, tree in trees.items()
    ):
        raise RecoveryRefused("retained original requires the same admitted authority and policy")
    caller = required_file(
        api, core.workflow_file(api.repository), original.base, trees[original.base]
    )
    reader_shape(caller, implementation)
    reader_closure(api, implementation)
    ordinary_native_changes(
        api, comparison, original, source_base, trees, modes, python_manifests=True
    )
    excluded = static_pr_exclusions(api.repository, caller, selected[2])
    if retained_pr(api, number, branch) != binding:
        raise RecoveryRefused("retained binding changed during closed-source proof")
    return RetainedBinding(original, binding.target, binding.identity, excluded)


def recover_retained(
    api: core.GitHub,
    number: int,
    state: dict[str, Any],
    previous: dict[str, Any] | None,
    reservations: dict[str, Any],
    reports: dict[str, Any],
    *,
    implementation: str,
    service_run: int,
    branch: str,
    now: datetime,
    context: ServiceContext,
    original_run_id: int | None = None,
) -> dict[str, Any]:
    """Recheck only O/P and retry its existing waiter; fresh protected Q remains mandatory."""
    if not context.can_prepare:
        return state
    observed = retained_pr(api, number, branch)
    historical = observed.historical
    if original_run_id is None:
        file_name = core.workflow_file(api.repository).rsplit("/", 1)[1]
        listing = api.get(
            f"actions/workflows/{file_name}/runs?head_sha={historical.source_head}&per_page=20"
        )
        runs = listing.get("workflow_runs")
        if not isinstance(runs, list) or len(runs) > 20:
            raise RecoveryRefused("retained original CI discovery exceeds its bound")
        run = next(
            (
                item
                for item in runs
                if isinstance(item, dict) and core.candidate_run(api, item, historical.source_head)
            ),
            None,
        )
        if run is None:
            return state
        original_run_id = run.get("id")
    if type(original_run_id) is not int or original_run_id < 1:
        raise RecoveryRefused("retained original run identifier is malformed")
    key = hashlib.sha256(
        f"{api.repository}:{original_run_id}:{historical.source_head}:{implementation}".encode()
    ).hexdigest()
    reservation = reservations.get(key)
    if (reservation is not None and reservation.get("phase") != "prepared") or (
        previous is not None and original_run_id in previous.get("reruns", {}).values()
    ):
        return state
    try:
        raw = api.get(f"actions/runs/{original_run_id}")
        if raw.get("status") != "completed" or raw.get("conclusion") == "success":
            return state
        proof = retained_proof(api, number, implementation, branch)
        if (proof.historical, proof.target, proof.identity) != (
            observed.historical,
            observed.target,
            observed.identity,
        ):
            raise RecoveryRefused("retained original changed during discovery")
        run = original_run(
            api, original_run_id, historical, implementation, excluded_jobs=proof.excluded_jobs
        )
        hint = candidate_hint(api, run, historical)
        if (hint["candidate_sha"], hint["base_sha"]) != (historical.head, historical.base):
            raise RecoveryRefused("hint does not locate the observed stale original graph")
        if core.utc(run.get("created_at")) > context.created_at:
            raise RecoveryRefused("earlier-created service cannot target future original CI run")
        facts = protection(api, branch)
        bindings = {
            "original_run_id": original_run_id,
            "original_attempt": run["run_attempt"],
            "source_head_sha": historical.source_head,
            "authority": implementation,
            "hinted_candidate_sha": historical.head,
            "hinted_base_sha": historical.base,
            "original_identity_sha256": proof.identity,
            "kind": "retained-original",
        }
        if reservation is not None and any(
            reservation.get(name) != value for name, value in bindings.items()
        ):
            raise RecoveryRefused("prepared retained-original bindings changed; budget suspended")
        report = core.evaluate(api, historical, state, now)
        state = report.pop("state")
        if retained_pr(api, number, branch) != observed:
            raise RecoveryRefused("retained original changed during metadata evaluation")
        report["evidence_kind"] = "independently-evaluated-historical-hint"
        report["protected_queue_required"] = True
        prior = None if previous is None else previous["reports"].get(historical.head)
        receipt = (
            previous["run_id"]
            if previous is not None and report_matches(prior, historical)
            else service_run
        )
        core.report_status(api, report, receipt)
        reports[historical.head] = report
        if reservation is None:
            reservations[key] = {
                **bindings,
                "phase": "prepared",
                "prepared_at": now.isoformat(),
                "prepared_service_run": service_run,
                "protection": facts,
            }
        elif report_matches(report, historical) and report_matches(prior, historical):
            if context.can_effect:
                consume(
                    api,
                    run,
                    historical,
                    reservation,
                    implementation,
                    service_run,
                    branch,
                    now,
                    context,
                    retained=proof,
                )
        return state
    except ValueError as error:
        if reservation is not None:
            reservation["phase"] = "suspended"
            reservation["reason"] = str(error)
        raise RecoveryRefused(str(error)) from error


def recover(
    api: core.GitHub,
    candidate: core.Candidate,
    report: dict[str, Any],
    state: dict[str, Any],
    previous: dict[str, Any] | None,
    reservations: dict[str, Any],
    reports: dict[str, Any],
    *,
    implementation: str,
    service_run: int,
    branch: str,
    now: datetime,
    context: ServiceContext,
) -> dict[str, Any]:
    """Prepare then consume one older-base recovery; equal-base retries stay separate."""
    if candidate.scope != "pull_request":
        return state
    file_name = core.workflow_file(api.repository).rsplit("/", 1)[1]
    listing = api.get(
        f"actions/workflows/{file_name}/runs?head_sha={candidate.source_head}&per_page=20"
    )
    runs = listing.get("workflow_runs", [])
    if not isinstance(runs, list) or len(runs) > 20:
        raise RecoveryRefused("candidate CI listing exceeds its supported recovery bound")
    for summary in runs:
        if not core.candidate_run(api, summary, candidate.source_head):
            continue
        prs = summary.get("pull_requests", [])
        if len(prs) != 1 or prs[0].get("number") != candidate.number:
            continue
        identifier = summary.get("id")
        if type(identifier) is not int or identifier < 1:
            raise RecoveryRefused("original CI identifier is malformed")
        return recover_run(
            api,
            identifier,
            candidate,
            report,
            state,
            previous,
            reservations,
            reports,
            implementation=implementation,
            service_run=service_run,
            branch=branch,
            now=now,
            context=context,
        )
    return state
