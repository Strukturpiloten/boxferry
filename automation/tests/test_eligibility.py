"""Independent adversarial examples for metadata-only eligibility and persistence."""

from __future__ import annotations

import base64
import io
import json
import tempfile
import unittest
import zipfile
from collections.abc import Callable
from copy import deepcopy
from datetime import UTC, datetime
from pathlib import Path
from typing import Any
from unittest.mock import patch
from urllib.error import URLError

from boxferry_ci import eligibility as subject

NOW = datetime(2026, 11, 3, 12, tzinfo=UTC)
BASE, HEAD, SOURCE, IMPLEMENTATION, TREE = (letter * 40 for letter in "abcef")
REPOSITORY = "Strukturpiloten/boxferry"


class API(subject.GitHub):
    def __init__(self) -> None:
        super().__init__(REPOSITORY, "not-a-real-token")
        self.responses: dict[str, Any] = {}
        self.posted: list[tuple[str, dict[str, Any]]] = []
        self.archives: dict[int, bytes] = {}
        self.graph: dict[str, Any] = {}
        self.requested: list[tuple[str, str]] = []
        self.read_paths: list[str] = []

    def get(self, suffix: str) -> dict[str, Any]:
        self.read_paths.append(suffix)
        value = self.responses.get(suffix)
        if not isinstance(value, dict):
            raise subject.MissingResource(suffix)
        return deepcopy(value)

    def items(self, suffix: str) -> list[dict[str, Any]]:
        value = self.responses.get(suffix)
        if not isinstance(value, list):
            raise subject.MissingResource(suffix)
        return deepcopy(value)

    def post(self, suffix: str, data: dict[str, Any]) -> None:
        self.posted.append((suffix, deepcopy(data)))

    def request(self, method: str, path: str, data: dict[str, Any] | None = None) -> object:
        self.requested.append((method, path))
        if method == "GET" and path.startswith("/repos/Strukturpiloten/boxferry"):
            value = self.responses.get(path)
            if not isinstance(value, dict):
                raise subject.MissingResource(path)
            return deepcopy(value)
        if method != "POST" or path != "/graphql":
            raise AssertionError("unexpected request")
        return deepcopy(self.graph)

    def artifact(self, artifact_id: int) -> bytes:
        return self.archives[artifact_id]


def zip_state(value: dict[str, Any], name: str = subject.STATE_FILE) -> bytes:
    data = io.BytesIO()
    with zipfile.ZipFile(data, "w") as output:
        output.writestr(name, json.dumps(value))
    return data.getvalue()


def run_metadata(run_id: int = 10, implementation: str = IMPLEMENTATION) -> dict[str, Any]:
    return {
        "id": run_id,
        "event": "schedule",
        "path": subject.WORKFLOW_PATH,
        "repository": {"full_name": REPOSITORY},
        "head_sha": BASE,
        "head_branch": "main",
        "status": "completed",
        "conclusion": "success",
        "referenced_workflows": [
            {
                "path": f"{subject.SHARED_WORKFLOW_PATH}@{implementation}",
                "sha": implementation,
            }
        ],
    }


def state_payload(run_id: int = 10) -> dict[str, Any]:
    return {
        "schema": 1,
        "repository": REPOSITORY,
        "workflow_path": subject.WORKFLOW_PATH,
        "implementation_revision": IMPLEMENTATION,
        "run_id": run_id,
        "observed_at": NOW.isoformat(),
        "dependency_state": {
            "schema_version": 1,
            "first_seen": {"f" * 64: "2026-10-30T12:00:00Z"},
        },
        "reruns": {},
        "reports": {HEAD: {"status": "pass", "candidate_sha": HEAD}},
    }


def pr_metadata(number: int = 17, source_head: str = SOURCE, merge: str = HEAD) -> dict[str, Any]:
    return {
        "number": number,
        "state": "open",
        "mergeable": True,
        "merge_commit_sha": merge,
        "base": {"sha": BASE, "ref": "main", "repo": {"full_name": REPOSITORY}},
        "head": {"sha": source_head},
        "user": {"login": "renovate[bot]", "type": "Bot"},
    }


def upgrade_workflow(pin: str, script_pin: str = IMPLEMENTATION) -> bytes:
    """Expected fixture distinguishes owned pins from shell data using the same SHA."""
    shared = "Strukturpiloten/boxferry/.github/workflows/shared-rust-ci.yml"
    annotation = (
        "# renovate: datasource=github-digest depName=Strukturpiloten/boxferry "
        "currentValue=automation-v1"
    )
    return f"""---
name: CI
on:
  pull_request:
env:
  AUTOMATION_SHA: {pin}
jobs:
  rust:
    uses: {shared}@{pin} # automation-v1 interface v1
    with:
      automation_sha: {pin}
  plan:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@{"1" * 40} # v7.0.1
        with:
          repository: Strukturpiloten/boxferry
          {annotation}
          ref: {pin}
          path: _automation
      - uses: ./_automation/.github/actions/shared-source
        with:
          automation-sha: '{pin}'
      - name: Keep shell text unchanged
        env:
          AUTOMATION_SHA: "{pin}"
        run: |
          cat <<'YAML'
          env:
            AUTOMATION_SHA: {script_pin}
          uses: {shared}@{script_pin}
          YAML
""".encode()


def ci_run(run_id: int = 50) -> dict[str, Any]:
    return {
        "id": run_id,
        "event": "pull_request",
        "name": "CI",
        "path": ".github/workflows/ci.yml",
        "repository": {"full_name": REPOSITORY},
        "head_repository": {"full_name": REPOSITORY},
        "head_sha": SOURCE,
        "status": "completed",
        "conclusion": "failure",
        "pull_requests": [
            {
                "number": 17,
                "head": {"sha": SOURCE},
                "base": {"sha": BASE},
            }
        ],
    }


class EligibilityTests(unittest.TestCase):
    def setUp(self) -> None:
        self.api = API()
        self.api.responses[""] = {"default_branch": "main"}
        self.api.responses[f"compare/{BASE}...main"] = {"status": "ahead"}
        self.api.responses["pulls/17"] = pr_metadata()
        self.api.responses["git/ref/heads/main"] = {
            "ref": "refs/heads/main",
            "object": {"type": "commit", "sha": BASE},
        }
        self.bind_merge(HEAD)
        self.candidate = subject.Candidate(
            BASE, HEAD, 17, SOURCE, "renovate[bot]", "Bot", "pull_request"
        )
        self.api.responses["actions/runs/50/jobs?per_page=100"] = {
            "total_count": 3,
            "jobs": [
                {"name": "Lockfile release age", "status": "completed", "conclusion": "failure"},
                {"name": "Required CI", "status": "completed", "conclusion": "failure"},
                {"name": "Rust quality", "status": "completed", "conclusion": "success"},
            ],
        }

    def bind_merge(
        self, head: str, *, base: str = BASE, source: str = SOURCE, tree: str = TREE
    ) -> None:
        self.api.responses[f"compare/{base}...{head}"] = {
            "status": "ahead",
            "merge_base_commit": {"sha": base},
        }
        self.api.responses[f"compare/{source}...{head}"] = {
            "status": "ahead",
            "merge_base_commit": {"sha": source},
        }
        self.api.responses[f"git/commits/{head}"] = {"tree": {"sha": tree}}

    def install_artifact(self, value: dict[str, Any] | None = None) -> None:
        self.api.responses["actions/runs/10"] = run_metadata()
        self.api.responses["actions/runs/10/artifacts?per_page=100"] = {
            "artifacts": [
                {"id": 100, "name": subject.ARTIFACT_NAME, "expired": False, "size_in_bytes": 500}
            ],
        }
        self.api.archives[100] = zip_state(value or state_payload())

    def test_historical_default_branch_state_survives_main_advancement(self) -> None:
        self.assertTrue(subject.trusted_run(self.api, run_metadata(), IMPLEMENTATION, "main"))
        self.api.responses[f"compare/{BASE}...main"] = {"status": "diverged"}
        self.assertFalse(subject.trusted_run(self.api, run_metadata(), IMPLEMENTATION, "main"))

    def test_candidate_or_foreign_or_wrong_pin_runs_cannot_seed_first_seen(self) -> None:
        for key, value in (
            ("event", "pull_request"),
            ("head_branch", "candidate"),
            ("path", ".github/workflows/ci.yml"),
            ("repository", {"full_name": "attacker/boxferry"}),
            ("conclusion", "failure"),
            ("referenced_workflows", []),
            (
                "referenced_workflows",
                [
                    {
                        "path": f"{subject.SHARED_WORKFLOW_PATH}@main",
                        "sha": IMPLEMENTATION,
                    }
                ],
            ),
        ):
            run = run_metadata()
            run[key] = value
            with self.subTest(key=key, value=value):
                self.assertFalse(subject.trusted_run(self.api, run, IMPLEMENTATION, "main"))

    def test_prior_state_restoration_uses_only_verified_service_artifact(self) -> None:
        self.install_artifact()
        self.api.responses[
            "actions/workflows/dependency-eligibility.yml/runs?per_page=30&status=success"
        ] = {
            "workflow_runs": [{"id": 10}],
        }
        state = subject.previous_state(self.api, IMPLEMENTATION, "main", NOW)
        self.assertIsNotNone(state)
        assert state is not None
        self.assertEqual(state["dependency_state"]["first_seen"]["f" * 64], "2026-10-30T12:00:00Z")
        self.api.responses["actions/runs/10"]["event"] = "pull_request"
        self.assertIsNone(subject.previous_state(self.api, IMPLEMENTATION, "main", NOW))

    def historical_state(self, source: str = "d" * 40) -> dict[str, Any]:
        value = state_payload()
        value["implementation_revision"] = source
        value["reruns"] = {HEAD: 50}
        value["scan_cursor"] = 17
        value["dependency_state"]["unrecognized_old_policy"] = "must not migrate"
        self.install_artifact(value)
        self.api.responses["actions/runs/10"] = run_metadata(implementation=source)
        self.api.responses[
            "actions/workflows/dependency-eligibility.yml/runs?per_page=30&status=success"
        ] = {"workflow_runs": [{"id": 10}]}
        self.api.responses[
            f"/repos/Strukturpiloten/boxferry/compare/{source}...{IMPLEMENTATION}"
        ] = {
            "status": "ahead",
            "merge_base_commit": {"sha": source},
        }
        return value

    def test_shared_pin_upgrade_preserves_only_mature_immutable_age_facts(self) -> None:
        original = self.historical_state()
        restored = subject.previous_state(self.api, IMPLEMENTATION, "main", NOW)
        self.assertIsNotNone(restored)
        assert restored is not None
        self.assertEqual(
            restored["dependency_state"],
            {
                "schema_version": 1,
                "first_seen": {"f" * 64: "2026-10-30T12:00:00Z"},
            },
        )
        self.assertEqual(restored["reports"], {})
        self.assertEqual(restored["reruns"], {})
        self.assertEqual(restored["scan_cursor"], 0)
        self.assertTrue(restored["observations_migrated"])
        self.assertEqual(restored["implementation_revision"], "d" * 40)
        self.assertEqual(original["reruns"], {HEAD: 50})
        # The same completed source observations survive another compatible pin
        # upgrade; their first-seen instant is not replaced with this run's time.
        next_implementation = "9" * 40
        self.api.responses[
            f"/repos/Strukturpiloten/boxferry/compare/{'d' * 40}...{next_implementation}"
        ] = {
            "status": "ahead",
            "merge_base_commit": {"sha": "d" * 40},
        }
        migrated_again = subject.previous_state(self.api, next_implementation, "main", NOW)
        assert migrated_again is not None
        self.assertEqual(migrated_again["dependency_state"], restored["dependency_state"])

    def test_shared_source_ancestry_is_proved_in_boxferry_not_the_consumer(self) -> None:
        original = self.historical_state()
        consumer = "Strukturpiloten/quadlet-lens"
        self.api.repository = consumer
        self.api.responses["actions/runs/10"]["repository"]["full_name"] = consumer
        original["repository"] = consumer
        self.api.archives[100] = zip_state(original)
        self.assertIsNotNone(subject.previous_state(self.api, IMPLEMENTATION, "main", NOW))
        self.assertIn(
            ("GET", f"/repos/Strukturpiloten/boxferry/compare/{'d' * 40}...{IMPLEMENTATION}"),
            self.api.requested,
        )
        self.assertFalse(any("quadlet-lens/compare/" in path for _, path in self.api.requested))

    def test_unrelated_future_and_malformed_shared_source_proofs_reject_migration(self) -> None:
        self.historical_state()
        endpoint = f"/repos/Strukturpiloten/boxferry/compare/{'d' * 40}...{IMPLEMENTATION}"
        for comparison in (
            {"status": "diverged", "merge_base_commit": {"sha": "a" * 40}},
            {"status": "behind", "merge_base_commit": {"sha": IMPLEMENTATION}},
            {"status": "ahead", "merge_base_commit": {"sha": "a" * 40}},
            {"status": "ahead", "merge_base_commit": None},
        ):
            self.api.responses[endpoint] = comparison
            with self.subTest(comparison=comparison):
                self.assertIsNone(subject.previous_state(self.api, IMPLEMENTATION, "main", NOW))
        del self.api.responses[endpoint]
        self.assertIsNone(subject.previous_state(self.api, IMPLEMENTATION, "main", NOW))

    def test_only_git_owned_canonical_full_sha_reference_can_migrate_age(self) -> None:
        self.historical_state()
        for references in (
            [{"path": f"{subject.SHARED_WORKFLOW_PATH}@main", "sha": "d" * 40}],
            [
                {
                    "path": (
                        "attacker/fork/.github/workflows/"
                        f"shared-dependency-eligibility.yml@{'d' * 40}"
                    ),
                    "sha": "d" * 40,
                }
            ],
            [{"path": f"{subject.SHARED_WORKFLOW_PATH}@{'d' * 40}", "sha": IMPLEMENTATION}],
            [{"path": f"{subject.SHARED_WORKFLOW_PATH}@{'d' * 39}", "sha": "d" * 39}],
            [
                {"path": f"{subject.SHARED_WORKFLOW_PATH}@{'d' * 40}", "sha": "d" * 40},
                {"path": f"{subject.SHARED_WORKFLOW_PATH}@{IMPLEMENTATION}", "sha": IMPLEMENTATION},
            ],
        ):
            self.api.responses["actions/runs/10"]["referenced_workflows"] = references
            with self.subTest(references=references):
                self.assertIsNone(subject.previous_state(self.api, IMPLEMENTATION, "main", NOW))

    def test_cross_pin_age_artifact_must_match_derived_source_and_compatible_schema(self) -> None:
        original = self.historical_state()
        for mutation in (
            lambda value: value.update(implementation_revision=IMPLEMENTATION),
            lambda value: value.update(schema=2),
            lambda value: value.update(schema=True),
            lambda value: value["dependency_state"].update(schema_version=2),
            lambda value: value["dependency_state"].update(schema_version=True),
            lambda value: value.update(observed_at="2099-01-01T00:00:00Z"),
            lambda value: value["dependency_state"]["first_seen"].update(
                {"f" * 64: "2099-01-01T00:00:00Z"}
            ),
        ):
            value = deepcopy(original)
            mutation(value)
            self.api.archives[100] = zip_state(value)
            self.assertIsNone(subject.previous_state(self.api, IMPLEMENTATION, "main", NOW))

    def test_ancestor_reports_never_become_current_checker_or_retry_authority(self) -> None:
        self.historical_state()
        self.api.responses["actions/runs/11"] = run_metadata(11)
        self.api.responses["pulls?state=open&per_page=100&sort=updated&direction=desc"] = [
            {"number": 17}
        ]
        self.api.responses["actions/workflows/ci.yml/runs?event=merge_group&per_page=20"] = {
            "workflow_runs": []
        }
        observed: list[dict[str, Any]] = []

        def current_checker(
            api: subject.GitHub, candidate: subject.Candidate, state: dict[str, Any], now: datetime
        ) -> dict[str, Any]:
            observed.append(deepcopy(state))
            return {
                "status": "failure",
                "candidate_sha": HEAD,
                "base_sha": BASE,
                "source_head_sha": SOURCE,
                "pull_request": 17,
                "scope": "pull_request",
                "state": state,
            }

        with (
            tempfile.TemporaryDirectory() as temporary,
            patch.object(subject, "evaluate", side_effect=current_checker),
            patch.object(subject, "rerun_ready") as retry,
        ):
            payload = subject.service(
                self.api,
                event_name="schedule",
                event={},
                implementation=IMPLEMENTATION,
                run_id=11,
                output=Path(temporary) / subject.STATE_FILE,
                now=NOW,
            )
        self.assertEqual(
            observed, [{"schema_version": 1, "first_seen": {"f" * 64: "2026-10-30T12:00:00Z"}}]
        )
        self.assertEqual(payload["reports"][HEAD]["status"], "failure")
        self.assertEqual(payload["reruns"], {})
        self.assertEqual(payload["implementation_revision"], IMPLEMENTATION)
        self.assertTrue(payload["state_source"].startswith("trusted-age-observations:10@"))
        self.assertEqual(self.api.posted[-1][1]["state"], "failure")
        retry.assert_not_called()

    def test_ancestor_receipt_cannot_authorize_current_waiter_without_reevaluation(self) -> None:
        self.historical_state()
        self.api.responses[f"commits/{HEAD}/statuses?per_page=100"] = [
            {
                "context": subject.STATUS_CONTEXT,
                "state": "success",
                "creator": {"login": "github-actions[bot]"},
                "target_url": f"https://github.com/{REPOSITORY}/actions/runs/10",
            }
        ]
        self.assertIsNone(subject.candidate_receipt(self.api, HEAD, IMPLEMENTATION, "main", NOW))

    def test_pull_request_target_is_neither_execution_nor_historical_state_source(self) -> None:
        self.historical_state()
        self.api.responses["actions/runs/10"]["event"] = "pull_request_target"
        self.assertIsNone(subject.previous_state(self.api, IMPLEMENTATION, "main", NOW))
        self.assertNotIn("pull_request_target", subject.ALLOWED_EVENTS)
        with self.assertRaises(subject.EligibilityError):
            subject.service(
                self.api,
                event_name="pull_request_target",
                event={"number": 17},
                implementation=IMPLEMENTATION,
                run_id=10,
                output=Path("unused"),
                now=NOW,
            )

    def test_invalid_or_lost_state_conservatively_reobserves(self) -> None:
        self.install_artifact()
        self.api.responses[
            "actions/workflows/dependency-eligibility.yml/runs?per_page=30&status=success"
        ] = {
            "workflow_runs": [{"id": 10}],
        }
        self.api.archives[100] = zip_state(state_payload(), "../../first-seen.json")
        self.assertIsNone(subject.previous_state(self.api, IMPLEMENTATION, "main", NOW))

    def test_state_provenance_future_times_and_unbounded_maps_fail(self) -> None:
        for mutate in (
            lambda value: value.update(run_id=99),
            lambda value: value.update(implementation_revision="0" * 40),
            lambda value: value.update(observed_at="2099-01-01T00:00:00Z"),
            lambda value: value["dependency_state"]["first_seen"].update(
                {"f" * 64: "2026-11-04T00:00:00Z"}
            ),
            lambda value: value["dependency_state"]["first_seen"].update(
                {"mutable-name": NOW.isoformat()}
            ),
            lambda value: value.update(reruns={HEAD: "not-an-integer"}),
        ):
            value = state_payload()
            mutate(value)
            with self.assertRaises(subject.EligibilityError):
                subject.validate_state(
                    value, repository=REPOSITORY, run_id=10, implementation=IMPLEMENTATION, now=NOW
                )

    def test_artifact_archive_must_contain_exactly_the_state_file(self) -> None:
        for raw in (
            b"not a zip",
            zip_state({}, "/absolute.json"),
            zip_state({}, "nested/eligibility-state.json"),
        ):
            with self.assertRaises(subject.EligibilityError):
                subject.decode_artifact(raw)
        raw = io.BytesIO()
        with zipfile.ZipFile(raw, "w") as archive:
            archive.writestr(subject.STATE_FILE, "{}")
            archive.writestr("extra", "{}")
        with self.assertRaises(subject.EligibilityError):
            subject.decode_artifact(raw.getvalue())

    def test_pr_candidate_uses_authoritative_synthetic_merge_not_author_claim(self) -> None:
        candidate = subject.pr_candidate(self.api, 17)
        self.assertEqual(candidate.head, HEAD)
        self.assertEqual(candidate.source_head, SOURCE)
        self.assertEqual(candidate.actor_login, "renovate[bot]")
        self.api.responses["pulls/17"] = {**pr_metadata(), "mergeable": None}
        with self.assertRaises(subject.CandidateComputationPending):
            subject.pr_candidate(self.api, 17)
        for key, value in (("state", "closed"), ("merge_commit_sha", "main")):
            self.api.responses["pulls/17"] = {**pr_metadata(), key: value}
            with self.assertRaises(subject.EligibilityError):
                subject.pr_candidate(self.api, 17)

    def test_live_target_ref_replaces_old_rest_base_and_excludes_intervening_prose(self) -> None:
        old_rest_base = "0" * 40
        self.api.responses["pulls/17"]["base"]["sha"] = old_rest_base
        current_delta = f"compare/{BASE}...{HEAD}?per_page=100"
        old_delta = f"compare/{old_rest_base}...{HEAD}?per_page=100"
        self.api.responses[current_delta] = {
            "merge_base_commit": {"sha": BASE},
            "files": [{"filename": "Cargo.toml"}, {"filename": "Cargo.lock"}],
        }
        self.api.responses[old_delta] = {
            "merge_base_commit": {"sha": old_rest_base},
            "files": [
                {"filename": "docs/public/main-note.md"},
                {"filename": "Cargo.toml"},
                {"filename": "Cargo.lock"},
            ],
        }
        candidate = subject.pr_candidate(self.api, 17)
        self.assertEqual(candidate.base, BASE)
        self.assertEqual(candidate.head, HEAD)
        self.assertEqual(subject.changed_paths(self.api, candidate), ["Cargo.lock", "Cargo.toml"])
        self.assertNotIn(old_delta, self.api.read_paths)
        self.assertIn(f"compare/{BASE}...{HEAD}", self.api.read_paths)
        self.assertIn(f"compare/{SOURCE}...{HEAD}", self.api.read_paths)

    def test_target_branch_moving_during_pr_verification_requires_retry(self) -> None:
        get = self.api.get
        calls = 0

        def moving(suffix: str) -> dict[str, Any]:
            nonlocal calls
            if suffix == "git/ref/heads/main":
                calls += 1
                return {
                    "ref": "refs/heads/main",
                    "object": {"type": "commit", "sha": BASE if calls == 1 else "9" * 40},
                }
            return get(suffix)

        with patch.object(self.api, "get", side_effect=moving):
            with self.assertRaisesRegex(subject.EligibilityError, "target branch moved"):
                subject.pr_candidate(self.api, 17)

    def test_source_or_synthetic_merge_recalculation_during_readback_requires_retry(self) -> None:
        get = self.api.get
        for change in ("source", "merge"):
            calls = 0

            def moving(suffix: str, change: str = change) -> dict[str, Any]:
                nonlocal calls
                value = get(suffix)
                if suffix == "pulls/17":
                    calls += 1
                    if calls == 2:
                        if change == "source":
                            value["head"]["sha"] = "9" * 40
                        else:
                            value["merge_commit_sha"] = "9" * 40
                return value

            with patch.object(self.api, "get", side_effect=moving):
                with (
                    self.subTest(change=change),
                    self.assertRaisesRegex(subject.EligibilityError, "changed during verification"),
                ):
                    subject.pr_candidate(self.api, 17)

    def test_stale_synthetic_merge_must_include_both_live_target_and_source(self) -> None:
        for ancestor in (BASE, SOURCE):
            self.bind_merge(HEAD)
            self.api.responses[f"compare/{ancestor}...{HEAD}"] = {
                "status": "diverged",
                "merge_base_commit": {"sha": "0" * 40},
            }
            with (
                self.subTest(ancestor=ancestor),
                self.assertRaisesRegex(subject.EligibilityError, "not an ancestor"),
            ):
                subject.pr_candidate(self.api, 17)

    def test_fixed_target_ref_endpoint_verifies_ref_identity_and_encodes_branch(self) -> None:
        self.api.responses["pulls/17"]["base"]["ref"] = "release/next"
        self.api.responses["git/ref/heads/release%2Fnext"] = {
            "ref": "refs/heads/release/next",
            "object": {"type": "commit", "sha": BASE},
        }
        self.assertEqual(subject.pr_candidate(self.api, 17).base, BASE)
        self.assertIn("git/ref/heads/release%2Fnext", self.api.read_paths)
        self.api.responses["git/ref/heads/release%2Fnext"]["object"]["type"] = "tag"
        with self.assertRaisesRegex(subject.EligibilityError, "expected branch commit"):
            subject.pr_candidate(self.api, 17)

    def test_still_current_repeats_live_target_and_source_proofs(self) -> None:
        candidate = subject.pr_candidate(self.api, 17)
        self.api.responses["git/ref/heads/main"]["object"]["sha"] = "9" * 40
        self.assertFalse(subject.still_current(self.api, candidate, "main"))

    def queue(self, nodes: list[dict[str, Any]]) -> None:
        self.api.graph = {
            "data": {
                "repository": {
                    "mergeQueue": {
                        "entries": {
                            "totalCount": len(nodes),
                            "nodes": nodes,
                        }
                    }
                }
            }
        }
        self.api.responses["rules/branches/main?per_page=100&page=1"] = [self.queue_rule()]
        self.api.responses[f"compare/main...{HEAD}"] = {"merge_base_commit": {"sha": BASE}}

    def queue_rule(self) -> dict[str, Any]:
        return {
            "type": "merge_queue",
            "parameters": {"max_entries_to_build": 1, "max_entries_to_merge": 1},
        }

    def node(self, number: int = 17, head: str = HEAD, source: str = SOURCE) -> dict[str, Any]:
        return {
            "baseCommit": {"oid": BASE},
            "headCommit": {"oid": head},
            "pullRequest": {"number": number, "headRefOid": source},
        }

    def test_queue_preserves_exact_original_bot_author_and_firstparty_context(self) -> None:
        self.queue([self.node()])
        candidate = subject.queue_candidate(self.api, HEAD, "main")
        self.assertEqual(candidate.actor_login, "renovate[bot]")
        self.assertEqual(candidate.scope, "merge_group")
        self.assertEqual(candidate.source_head, SOURCE)

    def test_queue_node_base_uses_current_target_despite_old_pr_rest_base(self) -> None:
        self.api.responses["pulls/17"]["base"]["sha"] = "0" * 40
        self.queue([self.node()])
        candidate = subject.queue_candidate(self.api, HEAD, "main")
        self.assertEqual(candidate.base, BASE)
        self.assertEqual(candidate.source_head, SOURCE)
        self.assertEqual(candidate.actor_login, "renovate[bot]")
        self.assertNotIn(f"compare/main...{HEAD}", self.api.read_paths)

    def test_queue_target_ref_moves_after_pr_verification_requires_retry(self) -> None:
        self.queue([self.node()])
        get = self.api.get
        calls = 0

        def moving(suffix: str) -> dict[str, Any]:
            nonlocal calls
            if suffix == "git/ref/heads/main":
                calls += 1
                return {
                    "ref": "refs/heads/main",
                    "object": {"type": "commit", "sha": BASE if calls < 4 else "9" * 40},
                }
            return get(suffix)

        with patch.object(self.api, "get", side_effect=moving):
            with self.assertRaisesRegex(subject.EligibilityError, "queue target branch moved"):
                subject.queue_candidate(self.api, HEAD, "main")

    def test_exact_queue_head_still_requires_current_pr_synthetic_tree(self) -> None:
        self.queue([self.node()])
        merge = "7" * 40
        self.api.responses["pulls/17"]["merge_commit_sha"] = merge
        self.bind_merge(merge, tree="8" * 40)
        with self.assertRaisesRegex(subject.EligibilityError, "no unique exact source PR"):
            subject.queue_candidate(self.api, HEAD, "main")

    def test_queue_backlog_does_not_mean_multi_entry_candidate(self) -> None:
        waiting = [
            {"baseCommit": {"oid": "d" * 40}, "headCommit": None, "pullRequest": {"number": number}}
            for number in (18, 19)
        ]
        self.queue([*waiting, self.node()])
        candidate = subject.queue_candidate(self.api, HEAD, "main")
        self.assertEqual(candidate.number, 17)

    def test_waiting_entry_at_same_base_cannot_supply_queue_authorship(self) -> None:
        waiting = {
            "baseCommit": {"oid": BASE},
            "headCommit": None,
            "pullRequest": {"number": 18, "headRefOid": SOURCE},
        }
        self.queue([waiting, self.node()])
        self.assertEqual(subject.queue_candidate(self.api, HEAD, "main").number, 17)
        self.assertNotIn("pulls/18", self.api.read_paths)
        self.queue([self.node(head="9" * 40)])
        with self.assertRaisesRegex(subject.EligibilityError, "no unique exact source PR"):
            subject.queue_candidate(self.api, HEAD, "main")

    def test_queue_fallback_requires_exact_base_current_source_and_full_tree(self) -> None:
        self.queue([self.node(head=SOURCE)])
        self.api.responses[f"git/commits/{HEAD}"] = {"tree": {"sha": TREE}}
        self.assertEqual(subject.queue_candidate(self.api, HEAD, "main").number, 17)
        self.api.responses[f"git/commits/{HEAD}"] = {"tree": {"sha": "d" * 40}}
        # Synthetic PR and queue identities can differ while trees match.
        merge = "f" * 40
        self.api.responses["pulls/17"] = pr_metadata(merge=merge)
        self.bind_merge(merge)
        self.api.responses[f"git/commits/{merge}"] = {"tree": {"sha": TREE}}
        with self.assertRaises(subject.EligibilityError):
            subject.queue_candidate(self.api, HEAD, "main")
        self.api.responses[f"git/commits/{HEAD}"] = {"tree": {"sha": TREE}}
        self.assertEqual(subject.queue_candidate(self.api, HEAD, "main").number, 17)
        self.api.responses["pulls/17"]["head"]["sha"] = "9" * 40
        with self.assertRaises(subject.EligibilityError):
            subject.queue_candidate(self.api, HEAD, "main")

    def test_changed_queue_limits_fail_explicitly(self) -> None:
        self.queue([self.node()])
        self.api.responses["rules/branches/main?per_page=100&page=1"][0]["parameters"][
            "max_entries_to_build"
        ] = 2
        with self.assertRaisesRegex(subject.EligibilityError, "one-entry queue"):
            subject.queue_candidate(self.api, HEAD, "main")

    def test_queue_reads_effective_rules_and_only_exact_entries_graphql(self) -> None:
        self.queue([self.node()])
        with (
            patch.object(self.api, "items", wraps=self.api.items) as items,
            patch.object(self.api, "request", wraps=self.api.request) as request,
        ):
            self.assertEqual(subject.queue_candidate(self.api, HEAD, "main").number, 17)
        items.assert_called_once_with("rules/branches/main?per_page=100&page=1")
        request.assert_called_once_with(
            "POST",
            "/graphql",
            {
                "query": (
                    "query($owner:String!,$name:String!,$branch:String!){"
                    "repository(owner:$owner,name:$name){mergeQueue(branch:$branch){"
                    "entries(first:100){totalCount nodes{baseCommit{oid} "
                    "headCommit{oid} pullRequest{number headRefOid}}}}}}"
                ),
                "variables": {"owner": "Strukturpiloten", "name": "boxferry", "branch": "main"},
            },
        )

    def test_queue_limits_use_metadata_endpoint_with_encoded_exact_branch(self) -> None:
        api = subject.GitHub(REPOSITORY, "never-print-this-token")
        with patch.object(api, "request", return_value=[self.queue_rule()]) as request:
            subject.require_single_entry_queue(api, "release/next")
        request.assert_called_once_with(
            "GET", f"/repos/{REPOSITORY}/rules/branches/release%2Fnext?per_page=100&page=1"
        )

    def test_queue_rules_reject_nonarray_and_malformed_entries(self) -> None:
        api = subject.GitHub(REPOSITORY, "never-print-this-token")
        for rules in (None, {}, "private-text", [None], [1], ["private-text"], [{}], [{"type": 1}]):
            with (
                self.subTest(rules=rules),
                patch.object(api, "request", return_value=rules),
                self.assertRaises(subject.EligibilityError),
            ):
                subject.require_single_entry_queue(api, "main")

    def test_queue_rules_require_one_unambiguous_active_queue(self) -> None:
        for rules in ([], [{"type": "required_status_checks"}], [self.queue_rule()] * 2):
            self.queue([self.node()])
            self.api.responses["rules/branches/main?per_page=100&page=1"] = rules
            with (
                self.subTest(rules=rules),
                self.assertRaisesRegex(subject.EligibilityError, "exactly one active merge queue"),
            ):
                subject.queue_candidate(self.api, HEAD, "main")
            self.assertEqual(self.api.requested, [])

    def test_queue_limits_reject_missing_malformed_boolean_and_nonone_values(self) -> None:
        for key in ("max_entries_to_build", "max_entries_to_merge"):
            for value in (None, False, True, 1.0, "1", 0, 2, -1, [], {}):
                self.queue([self.node()])
                rule = self.api.responses["rules/branches/main?per_page=100&page=1"][0]
                if value is None:
                    del rule["parameters"][key]
                else:
                    rule["parameters"][key] = value
                with (
                    self.subTest(key=key, value=value),
                    self.assertRaisesRegex(subject.EligibilityError, "one-entry queue"),
                ):
                    subject.queue_candidate(self.api, HEAD, "main")
        for parameters in (None, [], "private-text", 1):
            self.queue([self.node()])
            self.api.responses["rules/branches/main?per_page=100&page=1"][0]["parameters"] = (
                parameters
            )
            with self.subTest(parameters=parameters), self.assertRaises(subject.EligibilityError):
                subject.queue_candidate(self.api, HEAD, "main")

    def test_queue_rule_pagination_boundary_fails_without_prefix_authorization(self) -> None:
        for size in (99, 100, 101):
            self.queue([self.node()])
            self.api.responses["rules/branches/main?per_page=100&page=1"] = [
                *[{"type": "required_status_checks"}] * (size - 1),
                self.queue_rule(),
            ]
            with self.subTest(size=size):
                if size == 99:
                    self.assertEqual(subject.queue_candidate(self.api, HEAD, "main").number, 17)
                else:
                    with self.assertRaisesRegex(subject.EligibilityError, "truncated"):
                        subject.queue_candidate(self.api, HEAD, "main")

    def test_missing_or_failed_effective_rules_never_falls_back_to_queue_configuration(
        self,
    ) -> None:
        self.queue([self.node()])
        self.api.graph["data"]["repository"]["mergeQueue"]["configuration"] = {
            "maximumEntriesToBuild": 1,
            "maximumEntriesToMerge": 1,
        }
        del self.api.responses["rules/branches/main?per_page=100&page=1"]
        with self.assertRaises(subject.MissingResource):
            subject.queue_candidate(self.api, HEAD, "main")
        with patch.object(self.api, "items", side_effect=subject.EligibilityError("HTTP 403")):
            with self.assertRaisesRegex(subject.EligibilityError, "HTTP 403"):
                subject.queue_candidate(self.api, HEAD, "main")
        self.assertEqual(self.api.requested, [])

    def test_graphql_error_rejects_partial_valid_data_and_projects_closed_identifiers(self) -> None:
        self.queue([self.node()])
        self.api.graph["errors"] = [
            {
                "type": "FORBIDDEN",
                "message": "secret-token\n::error::private-text",
                "extensions": {"code": "FORBIDDEN", "private": "secret-token"},
                "path": ["repository", "mergeQueue", "entries", "nodes", 0, "headCommit", "oid"],
            }
        ]
        with self.assertRaises(subject.EligibilityError) as raised:
            subject.queue_candidate(self.api, HEAD, "main")
        self.assertEqual(
            str(raised.exception),
            "authoritative merge queue metadata is unavailable: "
            "type=FORBIDDEN,code=FORBIDDEN,field=ENTRY_HEAD",
        )

    def test_present_empty_or_malformed_graphql_errors_cannot_authorize_valid_data(self) -> None:
        for errors in (False, {}, None, [], "", 0):
            self.queue([self.node()])
            self.api.graph["errors"] = errors
            with (
                self.subTest(errors=errors),
                self.assertRaisesRegex(subject.EligibilityError, "graphql=INVALID_ERRORS"),
            ):
                subject.queue_candidate(self.api, HEAD, "main")
        self.queue([self.node()])
        self.assertNotIn("errors", self.api.graph)
        self.assertEqual(subject.queue_candidate(self.api, HEAD, "main").number, 17)

    def test_graphql_error_diagnostics_are_bounded_and_never_echo_untrusted_fields(self) -> None:
        for error in (
            None,
            "private-text",
            [],
            {},
            {"type": "private-text", "extensions": {"code": "secret-token"}, "path": ["private"]},
            {"type": [], "extensions": [], "path": [{"secret": "token"}]},
            {"type": {}, "extensions": {"code": []}, "path": ["repository", -1]},
            {"path": ["repository", "mergeQueue", "entries", "nodes", True]},
            {"path": ["repository"] * 100},
        ):
            with self.subTest(error=error):
                self.assertEqual(
                    subject.queue_graphql_errors({"errors": [error]}),
                    "type=UNKNOWN,code=UNKNOWN,field=UNKNOWN",
                )
        for response, expected in (
            (None, "graphql=INVALID_RESPONSE"),
            ({}, "graphql=INVALID_ERRORS"),
            ({"errors": "secret-token"}, "graphql=INVALID_ERRORS"),
            ({"errors": []}, "graphql=INVALID_ERRORS"),
        ):
            self.assertEqual(subject.queue_graphql_errors(response), expected)
        summary = subject.queue_graphql_errors({"errors": [{"message": "secret-token"}] * 10000})
        self.assertLess(len(summary), 500)
        self.assertEqual(summary.count("type=UNKNOWN"), 8)
        self.assertTrue(summary.endswith("additional_errors=OMITTED"))

    def test_graphql_error_known_schema_code_and_source_pr_field_are_actionable(self) -> None:
        self.assertEqual(
            subject.queue_graphql_errors(
                {
                    "errors": [
                        {
                            "extensions": {"code": "undefinedField"},
                            "path": [
                                "repository",
                                "mergeQueue",
                                "entries",
                                "nodes",
                                4,
                                "pullRequest",
                                "headRefOid",
                            ],
                        }
                    ]
                }
            ),
            "type=UNKNOWN,code=undefinedField,field=SOURCE_PR",
        )

    def test_queue_graphql_malformed_payload_fails_closed(self) -> None:
        for payload in (None, {}, {"repository": None}, {"repository": {"mergeQueue": None}}):
            self.queue([self.node()])
            self.api.graph["data"] = payload
            with self.subTest(payload=payload), self.assertRaises(subject.EligibilityError):
                subject.queue_candidate(self.api, HEAD, "main")

    def test_queue_graphql_malformed_and_truncated_entries_fail_closed(self) -> None:
        for entries in (
            None,
            [],
            {},
            {"totalCount": True, "nodes": [self.node()]},
            {"totalCount": -1, "nodes": []},
            {"totalCount": 101, "nodes": []},
            {"totalCount": 2, "nodes": [self.node()]},
            {"totalCount": 1, "nodes": "private-text"},
            {"totalCount": 1, "nodes": [None]},
        ):
            self.queue([self.node()])
            self.api.graph["data"]["repository"]["mergeQueue"]["entries"] = entries
            with self.subTest(entries=entries), self.assertRaises(subject.EligibilityError):
                subject.queue_candidate(self.api, HEAD, "main")

    def test_multiple_matching_queue_entries_fail_instead_of_blanket_exemption(self) -> None:
        self.queue([self.node(), self.node(18)])
        self.api.responses["pulls/18"] = pr_metadata(18)
        with self.assertRaisesRegex(subject.EligibilityError, "multi-entry"):
            subject.queue_candidate(self.api, HEAD, "main")

    def test_changed_path_metadata_rejects_truncation_and_keeps_rename_evidence(self) -> None:
        path = f"compare/{BASE}...{HEAD}?per_page=100"
        self.api.responses[path] = {
            "merge_base_commit": {"sha": BASE},
            "files": [{"filename": "Cargo.lock", "previous_filename": "old.lock"}],
        }
        self.assertEqual(
            subject.changed_paths(self.api, self.candidate), ["Cargo.lock", "old.lock"]
        )
        self.api.responses[path]["files"] = [{"filename": "Cargo.lock"}] * 300
        with self.assertRaisesRegex(subject.EligibilityError, "truncated"):
            subject.changed_paths(self.api, self.candidate)

    def contents(self, path: str, head: str, raw: bytes) -> None:
        self.api.responses[f"contents/{path}?ref={head}"] = {
            "type": "file",
            "size": len(raw),
            "encoding": "base64",
            "content": base64.b64encode(raw).decode(),
        }

    def test_snapshot_only_downloads_inert_dependency_paths_never_candidate_scripts(self) -> None:
        self.contents("Cargo.lock", HEAD, b"version = 4\n")
        cli_lock = (
            b'{"lockfileVersion":3,"packages":{'
            b'"node_modules/@devcontainers/cli":{"version":"0.89.0"}}}'
        )
        self.contents("automation/devcontainer/package-lock.json", HEAD, cli_lock)
        self.contents("crates/owned/Cargo.toml", HEAD, b"[package]\n")
        self.contents("scripts/exploit.py", HEAD, b"raise RuntimeError('candidate code')")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            subject.snapshot(
                self.api, HEAD, root, ["scripts/exploit.py", "crates/owned/Cargo.toml"]
            )
            self.assertTrue((root / "Cargo.lock").is_file())
            self.assertEqual(
                (root / "automation/devcontainer/package-lock.json").read_bytes(), cli_lock
            )
            self.assertTrue((root / "crates/owned/Cargo.toml").is_file())
            self.assertFalse((root / "scripts").exists())

    def test_rerun_is_once_bound_to_source_head_and_unchanged_workflow(self) -> None:
        query = f"actions/workflows/ci.yml/runs?head_sha={SOURCE}&per_page=20"
        self.api.responses[query] = {"workflow_runs": [ci_run()]}
        self.contents(".github/workflows/ci.yml", "main", b"permissions: read-all\n")
        self.contents(".github/workflows/ci.yml", HEAD, b"permissions: read-all\n")
        ledger: dict[str, int] = {}
        subject.rerun_ready(self.api, self.candidate, ledger, "main")
        subject.rerun_ready(self.api, self.candidate, ledger, "main")
        self.assertEqual(self.api.posted, [("actions/runs/50/rerun-failed-jobs", {})])
        self.assertEqual(ledger, {HEAD: 50})
        self.api.posted.clear()
        self.contents(".github/workflows/ci.yml", HEAD, b"permissions: write-all\n")
        subject.rerun_ready(self.api, self.candidate, {}, "main")
        self.assertEqual(self.api.posted, [])

    def test_exact_shared_reader_label_can_be_retried_after_completed_receipt(self) -> None:
        query = f"actions/workflows/ci.yml/runs?head_sha={SOURCE}&per_page=20"
        self.api.responses[query] = {"workflow_runs": [ci_run()]}
        self.contents(".github/workflows/ci.yml", "main", b"permissions: read-all\n")
        self.contents(".github/workflows/ci.yml", HEAD, b"permissions: read-all\n")
        jobs = self.api.responses["actions/runs/50/jobs?per_page=100"]["jobs"]
        jobs[0]["name"] = "Dependency eligibility / Dependency eligibility"
        subject.rerun_ready(self.api, self.candidate, {}, "main")
        self.assertEqual(self.api.posted, [("actions/runs/50/rerun-failed-jobs", {})])

    def test_nested_reader_suffix_cannot_authorize_rust_failure_or_arbitrary_prefix(self) -> None:
        query = f"actions/workflows/ci.yml/runs?head_sha={SOURCE}&per_page=20"
        self.api.responses[query] = {"workflow_runs": [ci_run()]}
        self.contents(".github/workflows/ci.yml", "main", b"permissions: read-all\n")
        self.contents(".github/workflows/ci.yml", HEAD, b"permissions: read-all\n")
        jobs = self.api.responses["actions/runs/50/jobs?per_page=100"]["jobs"]
        jobs[0]["name"] = "Dependency eligibility / Dependency eligibility"
        jobs[-1].update(name="Rust quality / Dependency eligibility", conclusion="failure")
        subject.rerun_ready(self.api, self.candidate, {}, "main")
        self.assertEqual(self.api.posted, [])
        jobs[-1]["conclusion"] = "success"
        jobs[0]["name"] = "Unreviewed prefix / Dependency eligibility"
        subject.rerun_ready(self.api, self.candidate, {}, "main")
        self.assertEqual(self.api.posted, [])

    def upgrade(self, after: str = "7" * 40) -> tuple[bytes, bytes]:
        before_ci, after_ci = upgrade_workflow(IMPLEMENTATION), upgrade_workflow(after)
        manifest = {
            "schema": 1,
            "repository": REPOSITORY,
            "revision": IMPLEMENTATION,
            "policy": "scripts/validation-policy.json",
            "interface": 1,
        }
        self.contents(".github/automation.json", BASE, json.dumps(manifest).encode())
        self.contents(
            ".github/automation.json", HEAD, json.dumps({**manifest, "revision": after}).encode()
        )
        self.contents("scripts/validation-policy.json", BASE, b'{"schema":2}\n')
        self.contents("scripts/validation-policy.json", HEAD, b'{"schema":2}\n')
        self.contents(".github/workflows/ci.yml", "main", before_ci)
        self.contents(".github/workflows/ci.yml", BASE, before_ci)
        self.contents(".github/workflows/ci.yml", HEAD, after_ci)
        self.api.responses["/repos/Strukturpiloten/boxferry"] = {"default_branch": "main"}
        self.api.responses["/repos/Strukturpiloten/boxferry/git/ref/heads/main"] = {
            "ref": "refs/heads/main",
            "object": {"type": "commit", "sha": BASE},
        }
        self.api.responses[
            f"/repos/Strukturpiloten/boxferry/compare/{IMPLEMENTATION}...{after}"
        ] = {
            "status": "ahead",
            "merge_base_commit": {"sha": IMPLEMENTATION},
        }
        self.api.responses[f"/repos/Strukturpiloten/boxferry/compare/{after}...{BASE}"] = {
            "status": "ahead",
            "merge_base_commit": {"sha": after},
        }
        self.api.responses[f"actions/workflows/ci.yml/runs?head_sha={SOURCE}&per_page=20"] = {
            "workflow_runs": [ci_run()],
        }
        return before_ci, after_ci

    def runner_upgrade(self) -> tuple[bytes, bytes]:
        before = b"""---
name: CI
on:
  pull_request:
permissions:
  contents: read
jobs:
  direct:
    runs-on: ubuntu-24.04
    steps:
      - run: echo unchanged
  include:
    runs-on: ${{ matrix.runs-on }}
    strategy:
      matrix:
        include:
          - target: apple
            runs-on: macos-15-intel
  marked:
    runs-on: ${{ matrix.runner }}
    strategy:
      matrix:
        # renovate: datasource=github-runners
        runner:
          - windows-2022
          - ubuntu-24.04-arm
    steps:
      - env:
          runs-on: ubuntu-99.04
        run: |
          jobs:
            fake:
              runs-on: ubuntu-99.04
"""
        after = (
            before.replace(b"ubuntu-24.04", b"ubuntu-26.04")
            .replace(b"macos-15-intel", b"macos-26-intel")
            .replace(b"windows-2022", b"windows-2025")
        )
        manifest = json.dumps(
            {
                "schema": 1,
                "repository": REPOSITORY,
                "revision": IMPLEMENTATION,
                "policy": "scripts/validation-policy.json",
                "interface": 1,
            }
        ).encode()
        for head in (BASE, HEAD):
            self.contents(".github/automation.json", head, manifest)
            self.contents("scripts/validation-policy.json", head, b'{"schema":2}\n')
        self.contents(".github/workflows/ci.yml", "main", before)
        self.contents(".github/workflows/ci.yml", BASE, before)
        self.contents(".github/workflows/ci.yml", HEAD, after)
        self.api.responses[f"actions/workflows/ci.yml/runs?head_sha={SOURCE}&per_page=20"] = {
            "workflow_runs": [ci_run()],
        }
        return before, after

    def test_runner_retry_uses_existing_owned_direct_include_and_marked_matrix_grammar(
        self,
    ) -> None:
        before, after = self.runner_upgrade()
        for old, new in (
            (b"ubuntu-24.04", b"ubuntu-26.04"),
            (b"macos-15-intel", b"macos-26-intel"),
            (b"windows-2022", b"windows-2025"),
        ):
            with self.subTest(old=old):
                self.assertTrue(
                    subject.hosted_runner_only_change(
                        before, before.replace(old, new), ".github/workflows/ci.yml"
                    )
                )
        self.assertTrue(
            subject.hosted_runner_only_change(before, after, ".github/workflows/ci.yml")
        )
        self.assertTrue(
            subject.hosted_runner_only_change(
                before.replace(b"\n", b"\r\n"),
                after.replace(b"\n", b"\r\n"),
                ".github/workflows/ci.yml",
            )
        )
        ledger: dict[str, int] = {}
        subject.rerun_ready(self.api, self.candidate, ledger, "main", implementation=IMPLEMENTATION)
        subject.rerun_ready(self.api, self.candidate, ledger, "main", implementation=IMPLEMENTATION)
        self.assertEqual(self.api.posted, [("actions/runs/50/rerun-failed-jobs", {})])
        self.assertEqual(ledger, {HEAD: 50})

    def test_runner_retry_rejects_all_nonselector_bytes_and_nonowned_contexts(self) -> None:
        before, after = self.runner_upgrade()
        changes = (
            (b"contents: read", b"contents: write"),
            (b"- run: echo unchanged", b"- uses: attacker/action@v1"),
            (b"runs-on: ubuntu-26.04", b"runs-on:  ubuntu-26.04"),
            (b"runs-on: ubuntu-26.04", b"runs-on: ubuntu-26.04 # changed"),
            (b"          - windows-2025\n", b""),
            (b"echo unchanged", b"curl https://attacker.invalid"),
            (b"ubuntu-99.04", b"ubuntu-26.04"),
            (b"target: apple", b"target: different"),
            (b"name: CI", b"name: Different"),
            (b"pull_request:", b"pull_request_target:"),
            (b"# renovate: datasource=github-runners", b"# unowned"),
            (b"    steps:", b"    if: always()\n    steps:"),
            (b"runs-on: ubuntu-26.04", b"runs-on: [self-hosted, ubuntu-26.04]"),
            (b"runs-on: ubuntu-26.04", b'runs-on: "ubuntu-26.04"'),
            (b"runs-on: ubuntu-26.04", b"runs-on: windows-2025"),
            (b"- windows-2025", b"- windows-2025\n          - windows-2022"),
            (b"- windows-2025", b"- ${{ inputs.runner }}"),
        )
        for old, new in changes:
            proposed = after.replace(old, new)
            with self.subTest(old=old):
                self.assertFalse(
                    subject.hosted_runner_only_change(before, proposed, ".github/workflows/ci.yml")
                )
        self.assertFalse(
            subject.hosted_runner_only_change(before, before, ".github/workflows/ci.yml")
        )
        self.assertFalse(subject.hosted_runner_only_change(before, after, "scripts/ci.yml"))
        self.assertFalse(
            subject.hosted_runner_only_change(
                before, after + b"# changed\n", ".github/workflows/ci.yml"
            )
        )
        reordered = before.replace(
            b"- windows-2022\n          - ubuntu-24.04-arm",
            b"- ubuntu-24.04-arm\n          - windows-2022",
        )
        self.assertFalse(
            subject.hosted_runner_only_change(before, reordered, ".github/workflows/ci.yml")
        )

    def test_runner_retry_preserves_prefix_comments_newlines_and_plain_matrix_order(self) -> None:
        before = b"""jobs:
  test:
    runs-on: ubuntu-24.04 # keep this comment
    strategy:
      matrix:
        # renovate: datasource=github-runners
        runner:
          - ubuntu-22.04
          - ubuntu-24.04
"""
        updated = before.replace(b"runs-on: ubuntu-24.04", b"runs-on: ubuntu-26.04")
        self.assertTrue(
            subject.hosted_runner_only_change(before, updated, ".github/workflows/ci.yml")
        )
        reordered = before.replace(
            b"- ubuntu-22.04\n          - ubuntu-24.04",
            b"- ubuntu-24.04\n          - ubuntu-22.04",
        )
        self.assertFalse(
            subject.hosted_runner_only_change(before, reordered, ".github/workflows/ci.yml")
        )
        self.assertFalse(
            subject.hosted_runner_only_change(
                before, updated.replace(b"\n", b"\r\n"), ".github/workflows/ci.yml"
            )
        )
        self.assertFalse(
            subject.hosted_runner_only_change(
                before,
                updated.replace(b"# keep this comment", b"# different"),
                ".github/workflows/ci.yml",
            )
        )

    def test_runner_retry_requires_same_service_manifest_policy_and_current_bot_actor(self) -> None:
        before, after = self.runner_upgrade()
        for actor, actor_type in (
            ("human", "User"),
            ("renovate[bot]", "User"),
            ("renovate-other[bot]", "Bot"),
        ):
            candidate = subject.Candidate(BASE, HEAD, 17, SOURCE, actor, actor_type, "pull_request")
            self.assertFalse(
                subject.retry_hosted_runner_upgrade(
                    self.api, candidate, before, after, IMPLEMENTATION
                )
            )
        self.assertFalse(
            subject.retry_hosted_runner_upgrade(self.api, self.candidate, before, after, "9" * 40)
        )
        self.api.responses["pulls/17"]["user"] = {"login": "human", "type": "User"}
        subject.rerun_ready(self.api, self.candidate, {}, "main", implementation=IMPLEMENTATION)
        self.assertEqual(self.api.posted, [])
        self.api.responses["pulls/17"] = pr_metadata()
        for path, data in (
            (".github/automation.json", b"{}"),
            ("scripts/validation-policy.json", b'{"schema":3}\n'),
        ):
            self.runner_upgrade()
            self.contents(path, HEAD, data)
            self.assertFalse(
                subject.retry_hosted_runner_upgrade(
                    self.api, self.candidate, before, after, IMPLEMENTATION
                )
            )
        self.runner_upgrade()
        self.contents(".github/workflows/ci.yml", BASE, before + b"# drift\n")
        self.assertFalse(
            subject.retry_hosted_runner_upgrade(
                self.api, self.candidate, before, after, IMPLEMENTATION
            )
        )

    def test_runner_retry_keeps_failed_age_only_exact_candidate_and_once_guards(self) -> None:
        self.runner_upgrade()
        jobs = self.api.responses["actions/runs/50/jobs?per_page=100"]["jobs"]
        for result in ("failure", "cancelled", "timed_out"):
            jobs[-1]["conclusion"] = result
            subject.rerun_ready(self.api, self.candidate, {}, "main", implementation=IMPLEMENTATION)
            self.assertEqual(self.api.posted, [])
        jobs[-1]["conclusion"] = "success"
        query = f"actions/workflows/ci.yml/runs?head_sha={SOURCE}&per_page=20"
        self.api.responses[query]["workflow_runs"][0]["head_sha"] = "9" * 40
        subject.rerun_ready(self.api, self.candidate, {}, "main", implementation=IMPLEMENTATION)
        self.assertEqual(self.api.posted, [])

    def test_runner_retry_still_waits_for_a_completed_exact_prior_pass_receipt(self) -> None:
        self.runner_upgrade()
        self.api.responses["actions/runs/11"] = run_metadata(11)
        self.api.responses["pulls?state=open&per_page=100&sort=updated&direction=desc"] = [
            {"number": 17}
        ]
        self.api.responses["actions/workflows/ci.yml/runs?event=merge_group&per_page=20"] = {
            "workflow_runs": []
        }
        report = {
            "status": "pass",
            "candidate_sha": HEAD,
            "base_sha": BASE,
            "source_head_sha": SOURCE,
            "pull_request": 17,
            "scope": "pull_request",
            "state": {"schema_version": 1, "first_seen": {}},
        }
        previous = state_payload()
        previous["reports"][HEAD] = {key: value for key, value in report.items() if key != "state"}
        for prior in (
            None,
            {**previous, "reports": {HEAD: {**previous["reports"][HEAD], "base_sha": "9" * 40}}},
            previous,
        ):
            self.api.posted.clear()
            with (
                tempfile.TemporaryDirectory() as temporary,
                patch.object(subject, "previous_state", return_value=prior),
                patch.object(subject, "evaluate", return_value=deepcopy(report)),
            ):
                result = subject.service(
                    self.api,
                    event_name="schedule",
                    event={},
                    implementation=IMPLEMENTATION,
                    run_id=11,
                    output=Path(temporary) / subject.STATE_FILE,
                    now=NOW,
                )
            reruns = [path for path, _ in self.api.posted if path.endswith("rerun-failed-jobs")]
            self.assertEqual(
                reruns, ["actions/runs/50/rerun-failed-jobs"] if prior is previous else []
            )
            self.assertEqual(result["reruns"], {HEAD: 50} if prior is previous else {})

    def test_ci_transform_changes_owned_pins_but_preserves_shell_bytes(self) -> None:
        before, after = self.upgrade()
        self.assertEqual(subject.expected_ci_upgrade(before, IMPLEMENTATION, "7" * 40), after)
        self.assertIn(f"AUTOMATION_SHA: {IMPLEMENTATION}".encode(), after)
        self.assertEqual(after.count(("7" * 40).encode()), 6)
        self.assertEqual(after.count(IMPLEMENTATION.encode()), 2)

    def test_late_receipt_shared_pin_upgrade_retries_once_after_completed_receipt(self) -> None:
        self.upgrade()
        self.api.responses["actions/runs/11"] = run_metadata(11)
        self.api.responses["pulls?state=open&per_page=100&sort=updated&direction=desc"] = [
            {"number": 17}
        ]
        self.api.responses["actions/workflows/ci.yml/runs?event=merge_group&per_page=20"] = {
            "workflow_runs": []
        }
        report = {
            "status": "pass",
            "candidate_sha": HEAD,
            "base_sha": BASE,
            "source_head_sha": SOURCE,
            "pull_request": 17,
            "scope": "pull_request",
            "state": {"schema_version": 1, "first_seen": {}},
        }
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / subject.STATE_FILE
            with (
                patch.object(subject, "previous_state", return_value=None),
                patch.object(subject, "evaluate", return_value=deepcopy(report)),
            ):
                first = subject.service(
                    self.api,
                    event_name="schedule",
                    event={},
                    implementation=IMPLEMENTATION,
                    run_id=11,
                    output=output,
                    now=NOW,
                )
            self.assertFalse(any(path.endswith("rerun-failed-jobs") for path, _ in self.api.posted))
            self.api.responses["actions/runs/11/artifacts?per_page=100"] = {
                "artifacts": [
                    {
                        "id": 111,
                        "name": subject.ARTIFACT_NAME,
                        "expired": False,
                        "size_in_bytes": 500,
                    }
                ],
            }
            self.api.archives[111] = zip_state(first)
            self.api.responses[
                "actions/workflows/dependency-eligibility.yml/runs?per_page=30&status=success"
            ] = {
                "workflow_runs": [{"id": 11}],
            }
            self.api.responses["actions/runs/12"] = run_metadata(12)
            with patch.object(subject, "evaluate", return_value=deepcopy(report)):
                second = subject.service(
                    self.api,
                    event_name="schedule",
                    event={},
                    implementation=IMPLEMENTATION,
                    run_id=12,
                    output=output,
                    now=NOW,
                )
        self.assertEqual(second["reruns"], {HEAD: 50})
        self.assertEqual(
            [path for path, _ in self.api.posted if path.endswith("rerun-failed-jobs")],
            ["actions/runs/50/rerun-failed-jobs"],
        )
        status = [body for path, body in self.api.posted if path.startswith("statuses/")][-1]
        self.assertTrue(status["target_url"].endswith("/11"))
        subject.rerun_ready(
            self.api, self.candidate, second["reruns"], "main", implementation=IMPLEMENTATION
        )
        self.assertEqual(sum(path.endswith("rerun-failed-jobs") for path, _ in self.api.posted), 1)

    def test_upgrade_without_running_service_pin_cannot_bypass_byte_equality(self) -> None:
        self.upgrade()
        subject.rerun_ready(self.api, self.candidate, {}, "main")
        self.assertEqual(self.api.posted, [])
        subject.rerun_ready(self.api, self.candidate, {}, "main", implementation="9" * 40)
        self.assertEqual(self.api.posted, [])

    def test_upgrade_requires_exact_bot_and_fresh_authoritative_actor(self) -> None:
        before, after = self.upgrade()
        for actor, actor_type in (
            ("human", "User"),
            ("renovate[bot]", "User"),
            ("renovate-other[bot]", "Bot"),
        ):
            candidate = subject.Candidate(BASE, HEAD, 17, SOURCE, actor, actor_type, "pull_request")
            self.assertFalse(
                subject.retry_ci_upgrade(self.api, candidate, before, after, IMPLEMENTATION)
            )
        self.api.responses["pulls/17"]["user"] = {"login": "human", "type": "User"}
        subject.rerun_ready(self.api, self.candidate, {}, "main", implementation=IMPLEMENTATION)
        self.assertEqual(self.api.posted, [])

    def test_upgrade_requires_compatible_manifests_and_identical_policy_scope(self) -> None:
        before, after = self.upgrade()
        valid = json.loads(subject.file_bytes(self.api, ".github/automation.json", HEAD) or b"{}")
        for field, value in (
            ("schema", True),
            ("interface", 2),
            ("repository", "attacker/boxferry"),
            ("policy", "other-policy.json"),
            ("revision", "main"),
        ):
            self.contents(
                ".github/automation.json", HEAD, json.dumps({**valid, field: value}).encode()
            )
            with self.subTest(field=field):
                self.assertFalse(
                    subject.retry_ci_upgrade(
                        self.api, self.candidate, before, after, IMPLEMENTATION
                    )
                )
        self.contents(".github/automation.json", HEAD, json.dumps(valid).encode())
        self.contents("scripts/validation-policy.json", HEAD, b'{"schema":2,"weakened":true}\n')
        self.assertFalse(
            subject.retry_ci_upgrade(self.api, self.candidate, before, after, IMPLEMENTATION)
        )

    def test_upgrade_target_must_already_be_on_canonical_boxferry_default_history(self) -> None:
        before, after = self.upgrade()
        for endpoint, invalid in (
            (
                f"/repos/Strukturpiloten/boxferry/compare/{IMPLEMENTATION}...{'7' * 40}",
                {"status": "diverged", "merge_base_commit": {"sha": "0" * 40}},
            ),
            (
                f"/repos/Strukturpiloten/boxferry/compare/{'7' * 40}...{BASE}",
                {"status": "behind", "merge_base_commit": {"sha": BASE}},
            ),
        ):
            self.upgrade()
            self.api.responses[endpoint] = invalid
            self.assertFalse(
                subject.retry_ci_upgrade(self.api, self.candidate, before, after, IMPLEMENTATION)
            )
        self.upgrade()
        self.api.responses["/repos/Strukturpiloten/boxferry/git/ref/heads/main"]["ref"] = (
            "refs/heads/attacker"
        )
        self.assertFalse(
            subject.retry_ci_upgrade(self.api, self.candidate, before, after, IMPLEMENTATION)
        )

    def test_upgrade_rejects_extra_ci_structure_run_edits_or_unknown_pin_changes(self) -> None:
        before, after = self.upgrade()
        variants = (
            after.replace(b"runs-on: ubuntu-24.04", b"runs-on: self-hosted"),
            after.replace(b"name: CI", b"name: CI\npermissions: write-all"),
            upgrade_workflow("7" * 40, script_pin="7" * 40),
            after.replace(b"shared-rust-ci.yml", b"unreviewed-rust-ci.yml"),
            after + b"unknown:\n  AUTOMATION_SHA: " + ("7" * 40).encode() + b"\n",
        )
        for proposed in variants:
            with self.subTest(proposed=proposed):
                self.assertFalse(
                    subject.retry_ci_upgrade(
                        self.api, self.candidate, before, proposed, IMPLEMENTATION
                    )
                )

    def test_annotated_checkout_upgrade_requires_exact_owned_marker_and_repository(self) -> None:
        before, _ = self.upgrade()
        for old, replacement in (
            (b"currentValue=automation-v1", b"currentValue=main"),
            (b"depName=Strukturpiloten/boxferry", b"depName=attacker/fork"),
            (b"repository: Strukturpiloten/boxferry", b"repository: attacker/fork"),
            (b"# renovate: datasource=github-digest", b"# unowned: datasource=github-digest"),
        ):
            baseline = before.replace(old, replacement)
            candidate = upgrade_workflow("7" * 40).replace(old, replacement)
            expected = subject.expected_ci_upgrade(baseline, IMPLEMENTATION, "7" * 40)
            self.assertNotEqual(expected, candidate)
            assert expected is not None
            self.assertIn(f"ref: {IMPLEMENTATION}".encode(), expected)

    def test_yaml_block_styles_never_authorize_pins_in_their_text(self) -> None:
        for style in ("|", ">", "|-", ">+2", "!!str |", "&script |-"):
            baseline = upgrade_workflow(IMPLEMENTATION).replace(b"run: |", f"run: {style}".encode())
            candidate = upgrade_workflow("7" * 40).replace(b"run: |", f"run: {style}".encode())
            self.assertEqual(
                subject.expected_ci_upgrade(baseline, IMPLEMENTATION, "7" * 40), candidate
            )
            tampered = candidate.replace(
                f"            AUTOMATION_SHA: {IMPLEMENTATION}".encode(),
                f"            AUTOMATION_SHA: {'7' * 40}".encode(),
            )
            self.assertNotEqual(
                subject.expected_ci_upgrade(baseline, IMPLEMENTATION, "7" * 40), tampered
            )
        multiline = (
            upgrade_workflow(IMPLEMENTATION)
            + b'unknown: "multiline\n  AUTOMATION_SHA: '
            + IMPLEMENTATION.encode()
            + b'\n"\n'
        )
        self.assertIsNone(subject.expected_ci_upgrade(multiline, IMPLEMENTATION, "7" * 40))
        self.assertIsNone(
            subject.expected_ci_upgrade(
                b"jobs:\n  script:\n    steps:\n      - run: |\n          AUTOMATION_SHA: "
                + IMPLEMENTATION.encode()
                + b"\n",
                IMPLEMENTATION,
                "7" * 40,
            )
        )

    def test_unknown_yaml_and_unowned_automation_scalar_contexts_are_not_rewritten(self) -> None:
        extra = f"""unknown:
  AUTOMATION_SHA: {IMPLEMENTATION}
jobs_extra:
  uses: Strukturpiloten/boxferry/.github/workflows/shared-rust-ci.yml@{IMPLEMENTATION}
""".encode()
        baseline = upgrade_workflow(IMPLEMENTATION) + extra
        candidate = upgrade_workflow("7" * 40) + extra
        self.assertEqual(subject.expected_ci_upgrade(baseline, IMPLEMENTATION, "7" * 40), candidate)
        self.assertNotEqual(
            subject.expected_ci_upgrade(baseline, IMPLEMENTATION, "7" * 40),
            candidate.replace(extra, extra.replace(IMPLEMENTATION.encode(), ("7" * 40).encode())),
        )

    def test_unowned_with_inputs_flow_mappings_and_unpinned_checkouts_are_not_rewritten(
        self,
    ) -> None:
        before = upgrade_workflow(IMPLEMENTATION)
        for old, changed in (
            (
                b"uses: ./_automation/.github/actions/shared-source",
                b"uses: ./_automation/.github/actions/unreviewed",
            ),
            (
                b"uses: ./_automation/.github/actions/shared-source",
                b"uses: attacker/action@" + b"1" * 40,
            ),
            (b"uses: actions/checkout@" + b"1" * 40, b"uses: actions/checkout@v7"),
        ):
            baseline = before.replace(old, changed)
            candidate = upgrade_workflow("7" * 40).replace(old, changed)
            self.assertNotEqual(
                subject.expected_ci_upgrade(baseline, IMPLEMENTATION, "7" * 40), candidate
            )
        block = f"      automation_sha: {IMPLEMENTATION}".encode()
        baseline = before.replace(
            block, f"      inline: {{automation_sha: {IMPLEMENTATION}}}".encode()
        )
        candidate = upgrade_workflow("7" * 40).replace(
            f"      automation_sha: {'7' * 40}".encode(),
            f"      inline: {{automation_sha: {'7' * 40}}}".encode(),
        )
        self.assertNotEqual(
            subject.expected_ci_upgrade(baseline, IMPLEMENTATION, "7" * 40), candidate
        )

    def test_duplicate_yaml_mapping_keys_cannot_hide_checkout_or_action_overrides(self) -> None:
        before = upgrade_workflow(IMPLEMENTATION)
        for baseline in (
            before.replace(
                b"          path: _automation",
                b"          path: _automation\n          repository: attacker/fork",
            ),
            before.replace(
                b"    with:\n      automation_sha:",
                b"    uses: attacker/workflow@v1\n    with:\n      automation_sha:",
            ),
            before + b"jobs: {}\n",
        ):
            self.assertIsNone(subject.expected_ci_upgrade(baseline, IMPLEMENTATION, "7" * 40))

    def test_canonical_source_tip_moving_during_upgrade_proof_requires_retry(self) -> None:
        before, after = self.upgrade()
        request = self.api.request
        calls = 0

        def moving(method: str, path: str, data: dict[str, Any] | None = None) -> object:
            nonlocal calls
            value = request(method, path, data)
            if path == "/repos/Strukturpiloten/boxferry/git/ref/heads/main":
                calls += 1
                if calls == 2:
                    assert isinstance(value, dict)
                    value["object"]["sha"] = "9" * 40
            return value

        with patch.object(self.api, "request", side_effect=moving):
            self.assertFalse(
                subject.retry_ci_upgrade(self.api, self.candidate, before, after, IMPLEMENTATION)
            )

    def test_upgrade_freshness_readback_and_existing_failure_guards_still_apply(self) -> None:
        self.upgrade()
        self.api.responses["actions/runs/50/jobs?per_page=100"]["jobs"][-1]["conclusion"] = (
            "failure"
        )
        subject.rerun_ready(self.api, self.candidate, {}, "main", implementation=IMPLEMENTATION)
        self.assertEqual(self.api.posted, [])
        self.upgrade()
        self.api.responses["actions/runs/50/jobs?per_page=100"]["jobs"][-1]["conclusion"] = (
            "success"
        )
        with patch.object(subject, "still_current", return_value=False):
            subject.rerun_ready(self.api, self.candidate, {}, "main", implementation=IMPLEMENTATION)
        self.assertEqual(self.api.posted, [])

    def test_stale_or_foreign_ci_cannot_be_rerun_by_service(self) -> None:
        self.api.responses[f"actions/workflows/ci.yml/runs?head_sha={SOURCE}&per_page=20"] = {
            "workflow_runs": [{**ci_run(), "head_sha": "9" * 40}],
        }
        subject.rerun_ready(self.api, self.candidate, {}, "main")
        self.assertEqual(self.api.posted, [])
        self.assertFalse(
            subject.candidate_run(self.api, {**ci_run(), "event": "workflow_dispatch"}, SOURCE)
        )

    def test_receipt_needs_exact_sha_and_trusted_run_artifact_not_only_commit_status(self) -> None:
        self.install_artifact()
        status = {
            "context": subject.STATUS_CONTEXT,
            "state": "success",
            "creator": {"login": "github-actions[bot]"},
            "target_url": f"https://github.com/{REPOSITORY}/actions/runs/10",
        }
        self.api.responses[f"commits/{HEAD}/statuses?per_page=100"] = [status]
        report = subject.candidate_receipt(self.api, HEAD, IMPLEMENTATION, "main", NOW)
        self.assertEqual(report, {"candidate_sha": HEAD, "status": "pass"})
        self.api.responses["actions/runs/10"]["event"] = "pull_request"
        self.assertIsNone(subject.candidate_receipt(self.api, HEAD, IMPLEMENTATION, "main", NOW))
        self.api.responses["actions/runs/10"] = run_metadata()
        state = state_payload()
        state["reports"][HEAD]["candidate_sha"] = SOURCE
        self.api.archives[100] = zip_state(state)
        self.assertIsNone(subject.candidate_receipt(self.api, HEAD, IMPLEMENTATION, "main", NOW))

    def test_waiting_and_policy_failure_are_distinct_from_pass_and_required_ci(self) -> None:
        for result, expected in (
            ("waiting", "pending"),
            ("failure", "failure"),
            ("pass", "success"),
        ):
            subject.report_status(
                self.api,
                {"status": result, "candidate_sha": HEAD, "retry_at": "2026-11-06T12:00:00Z"},
                10,
            )
            self.assertEqual(self.api.posted[-1][1]["state"], expected)
            self.assertEqual(self.api.posted[-1][1]["context"], "Dependency eligibility")
        self.api.responses[f"commits/{HEAD}/statuses?per_page=100"] = []
        result = subject.wait_for_candidate(self.api, HEAD, IMPLEMENTATION, timeout=0)
        self.assertEqual(result["status"], "waiting")
        self.assertTrue(result["retry_eligible"])

    def test_rerun_never_repairs_rust_failure_or_user_cancellation(self) -> None:
        query = f"actions/workflows/ci.yml/runs?head_sha={SOURCE}&per_page=20"
        self.api.responses[query] = {"workflow_runs": [ci_run()]}
        self.contents(".github/workflows/ci.yml", "main", b"permissions: read-all\n")
        self.contents(".github/workflows/ci.yml", HEAD, b"permissions: read-all\n")
        self.api.responses["actions/runs/50/jobs?per_page=100"]["jobs"][-1]["conclusion"] = (
            "failure"
        )
        subject.rerun_ready(self.api, self.candidate, {}, "main")
        self.assertEqual(self.api.posted, [])
        self.api.responses[query]["workflow_runs"][0]["conclusion"] = "cancelled"
        subject.rerun_ready(self.api, self.candidate, {}, "main")
        self.assertEqual(self.api.posted, [])

    def test_first_pass_defers_rerun_until_completed_prior_receipt(self) -> None:
        self.api.responses["actions/runs/11"] = {
            **run_metadata(11),
            "status": "in_progress",
            "conclusion": None,
        }
        self.api.responses["pulls?state=open&per_page=100&sort=updated&direction=desc"] = [
            {"number": 17}
        ]
        self.api.responses["actions/workflows/ci.yml/runs?event=merge_group&per_page=20"] = {
            "workflow_runs": []
        }
        report = {
            "status": "pass",
            "candidate_sha": HEAD,
            "base_sha": BASE,
            "source_head_sha": SOURCE,
            "scope": "pull_request",
            "pull_request": 17,
            "state": {"schema_version": 1, "first_seen": {}},
        }
        with (
            tempfile.TemporaryDirectory() as temporary,
            patch.object(subject, "previous_state", return_value=None),
            patch.object(subject, "evaluate", return_value=deepcopy(report)),
            patch.object(subject, "rerun_ready") as rerun,
        ):
            subject.service(
                self.api,
                event_name="schedule",
                event={},
                implementation=IMPLEMENTATION,
                run_id=11,
                output=Path(temporary) / subject.STATE_FILE,
                now=NOW,
            )
            rerun.assert_not_called()
        self.assertTrue(self.api.posted[-1][1]["target_url"].endswith("/11"))
        previous = state_payload()
        previous["reports"][HEAD] = {key: value for key, value in report.items() if key != "state"}
        # The new service is still running; its evaluation may take arbitrarily
        # longer than the candidate wait. The target must stay completed run10.
        with (
            tempfile.TemporaryDirectory() as temporary,
            patch.object(subject, "previous_state", return_value=previous),
            patch.object(subject, "evaluate", return_value=deepcopy(report)),
            patch.object(subject, "rerun_ready") as rerun,
        ):
            subject.service(
                self.api,
                event_name="schedule",
                event={},
                implementation=IMPLEMENTATION,
                run_id=11,
                output=Path(temporary) / subject.STATE_FILE,
                now=NOW,
            )
            rerun.assert_called_once()
        self.assertTrue(self.api.posted[-1][1]["target_url"].endswith("/10"))

    def test_prior_failure_or_stale_binding_never_authorizes_rerun(self) -> None:
        self.api.responses["actions/runs/11"] = {
            **run_metadata(11),
            "status": "in_progress",
            "conclusion": None,
        }
        self.api.responses["pulls?state=open&per_page=100&sort=updated&direction=desc"] = [
            {"number": 17}
        ]
        self.api.responses["actions/workflows/ci.yml/runs?event=merge_group&per_page=20"] = {
            "workflow_runs": []
        }
        report = {
            "status": "pass",
            "candidate_sha": HEAD,
            "base_sha": BASE,
            "source_head_sha": SOURCE,
            "scope": "pull_request",
            "pull_request": 17,
            "state": {"schema_version": 1, "first_seen": {}},
        }
        for key, value in (
            ("status", "failure"),
            ("base_sha", "0" * 40),
            ("source_head_sha", "0" * 40),
            ("scope", "merge_group"),
        ):
            previous = state_payload()
            previous["reports"][HEAD] = {**report, key: value}
            with (
                tempfile.TemporaryDirectory() as temporary,
                patch.object(subject, "previous_state", return_value=previous),
                patch.object(subject, "evaluate", return_value=deepcopy(report)),
                patch.object(subject, "rerun_ready") as rerun,
            ):
                subject.service(
                    self.api,
                    event_name="schedule",
                    event={},
                    implementation=IMPLEMENTATION,
                    run_id=11,
                    output=Path(temporary) / subject.STATE_FILE,
                    now=NOW,
                )
                rerun.assert_not_called()

    def test_service_refuses_candidate_context_even_if_event_payload_claims_trust(self) -> None:
        with self.assertRaises(subject.EligibilityError):
            subject.service(
                self.api,
                event_name="merge_group",
                event={},
                implementation=IMPLEMENTATION,
                run_id=10,
                output=Path("unused"),
                now=NOW,
            )
        self.api.responses["actions/runs/10"] = {
            **run_metadata(),
            "path": ".github/workflows/ci.yml",
        }
        with self.assertRaises(subject.EligibilityError):
            subject.service(
                self.api,
                event_name="schedule",
                event={},
                implementation=IMPLEMENTATION,
                run_id=10,
                output=Path("unused"),
                now=NOW,
            )

    def pending_callback(self, kind: str) -> None:
        current = run_metadata(11)
        current["event"] = "workflow_run"
        self.api.responses["actions/runs/11"] = current
        self.api.responses[
            "actions/workflows/dependency-eligibility.yml/runs?per_page=30&status=success"
        ] = {"workflow_runs": []}
        source = ci_run()
        source.update(
            event=kind,
            head_sha=HEAD if kind == "merge_group" else SOURCE,
            status="in_progress",
            conclusion=None,
        )
        self.api.responses["actions/runs/50"] = source
        if kind == "merge_group":
            self.queue([self.node()])
        self.api.responses["pulls/17"].update(mergeable=None, merge_commit_sha=None)

    def test_workflow_callbacks_defer_null_then_bind_computed_exact_candidate(self) -> None:
        for kind in ("pull_request", "merge_group"):
            with self.subTest(source_event=kind):
                self.setUp()
                self.pending_callback(kind)
                report = {
                    "status": "pass",
                    "candidate_sha": HEAD,
                    "base_sha": BASE,
                    "source_head_sha": SOURCE,
                    "pull_request": 17,
                    "scope": kind,
                    "state": {"schema_version": 1, "first_seen": {}},
                }
                with (
                    tempfile.TemporaryDirectory() as temporary,
                    patch.object(subject, "evaluate", return_value=deepcopy(report)) as evaluate,
                    patch.object(subject, "rerun_ready") as retry,
                ):
                    output = Path(temporary) / subject.STATE_FILE
                    pending = subject.service(
                        self.api,
                        event_name="workflow_run",
                        event={"action": "requested", "workflow_run": {"id": 50}},
                        implementation=IMPLEMENTATION,
                        run_id=11,
                        output=output,
                        now=NOW,
                    )
                    self.assertEqual(json.loads(output.read_text()), pending)
                    self.assertEqual(pending["reports"], {})
                    self.assertEqual(pending["infrastructure_errors"], [])
                    self.assertEqual(len(pending["deferred_candidates"]), 1)
                    deferred = pending["deferred_candidates"][0]
                    self.assertEqual(deferred["pull_request"], 17)
                    self.assertEqual(deferred["source_run_id"], 50)
                    self.assertEqual(
                        deferred["source_head_sha"], HEAD if kind == "merge_group" else SOURCE
                    )
                    self.assertIn("computing", deferred["reason"])
                    self.assertIn("completed CI callback", deferred["next_action"])
                    self.assertIn("hourly schedule", subject.summary(pending))
                    self.assertEqual(self.api.posted, [])
                    evaluate.assert_not_called()
                    retry.assert_not_called()

                    self.api.responses["pulls/17"] = pr_metadata()
                    self.api.responses["actions/runs/50"].update(
                        status="completed", conclusion="success"
                    )
                    next_run = run_metadata(12)
                    next_run["event"] = "workflow_run"
                    self.api.responses["actions/runs/12"] = next_run
                    completed = subject.service(
                        self.api,
                        event_name="workflow_run",
                        event={"action": "completed", "workflow_run": {"id": 50}},
                        implementation=IMPLEMENTATION,
                        run_id=12,
                        output=output,
                        now=NOW,
                    )
                    self.assertEqual(completed["deferred_candidates"], [])
                    self.assertEqual(set(completed["reports"]), {HEAD})
                    candidate = evaluate.call_args.args[1]
                    self.assertEqual(
                        (candidate.head, candidate.source_head, candidate.scope),
                        (HEAD, SOURCE, kind),
                    )
                    self.assertEqual([path for path, _ in self.api.posted], [f"statuses/{HEAD}"])
                    self.assertEqual(self.api.posted[0][1]["state"], "success")
                    self.assertEqual(self.api.read_paths.count("actions/runs/50"), 2)
                    retry.assert_not_called()

    def test_completed_callbacks_for_owned_closed_pr_are_no_action_not_authorization(self) -> None:
        for kind in ("pull_request", "merge_group"):
            for merged in (False, True):
                with self.subTest(source_event=kind, merged=merged):
                    self.setUp()
                    self.pending_callback(kind)
                    self.api.responses["actions/runs/50"].update(
                        status="completed", conclusion="success"
                    )
                    self.api.responses["pulls/17"] = {
                        **pr_metadata(),
                        "state": "closed",
                        "mergeable": None,
                        "merged": merged,
                    }
                    with self.assertRaises(subject.ClosedPullRequest):
                        subject.pr_candidate(self.api, 17)
                    previous = state_payload()
                    previous["reruns"] = {HEAD: 50}
                    with (
                        tempfile.TemporaryDirectory() as temporary,
                        patch.object(subject, "previous_state", return_value=previous),
                        patch.object(subject, "evaluate") as evaluate,
                        patch.object(subject, "rerun_ready") as retry,
                    ):
                        output = Path(temporary) / subject.STATE_FILE
                        payload = subject.service(
                            self.api,
                            event_name="workflow_run",
                            event={"action": "completed", "workflow_run": {"id": 50}},
                            implementation=IMPLEMENTATION,
                            run_id=11,
                            output=output,
                            now=NOW,
                        )
                        self.assertEqual(json.loads(output.read_text()), payload)
                    self.assertEqual(payload["reports"], {})
                    self.assertEqual(payload["deferred_candidates"], [])
                    self.assertEqual(payload["infrastructure_errors"], [])
                    self.assertEqual(payload["dependency_state"], previous["dependency_state"])
                    self.assertEqual(payload["reruns"], previous["reruns"])
                    self.assertEqual(len(payload["no_action_candidates"]), 1)
                    item = payload["no_action_candidates"][0]
                    self.assertEqual((item["pull_request"], item["source_run_id"]), (17, 50))
                    self.assertEqual(
                        item["source_head_sha"], HEAD if kind == "merge_group" else SOURCE
                    )
                    self.assertNotIn("candidate_sha", item)
                    self.assertIn("No action", subject.summary(payload))
                    self.assertEqual(self.api.posted, [])
                    evaluate.assert_not_called()
                    retry.assert_not_called()

    def test_closed_callback_requires_completed_source_and_exact_owned_metadata(self) -> None:
        for defect in (
            "foreign",
            "branch",
            "actor",
            "source",
            "merge",
            "mergeable",
            "stale-source",
            "requested",
            "running",
            "open-conflict",
        ):
            with self.subTest(defect=defect):
                self.setUp()
                self.pending_callback("pull_request")
                self.api.responses["actions/runs/50"].update(status="completed")
                record: dict[str, Any] = {**pr_metadata(), "state": "closed", "mergeable": None}
                action = "completed"
                if defect == "foreign":
                    record["base"]["repo"]["full_name"] = "Other/project"
                elif defect == "branch":
                    record["base"]["ref"] = "bad\nref"
                elif defect == "actor":
                    record["user"] = {"login": "", "type": "Bot"}
                elif defect == "source":
                    record["head"]["sha"] = "invalid"
                elif defect == "merge":
                    record["merge_commit_sha"] = "invalid"
                elif defect == "mergeable":
                    record.pop("mergeable")
                elif defect == "stale-source":
                    record["head"]["sha"] = "9" * 40
                    record["merge_commit_sha"] = None
                elif defect == "requested":
                    action = "requested"
                elif defect == "running":
                    self.api.responses["actions/runs/50"]["status"] = "in_progress"
                else:
                    record.update(state="open", mergeable=False)
                self.api.responses["pulls/17"] = record
                with (
                    tempfile.TemporaryDirectory() as temporary,
                    patch.object(subject, "evaluate") as evaluate,
                    patch.object(subject, "rerun_ready") as retry,
                ):
                    output = Path(temporary) / subject.STATE_FILE
                    with self.assertRaises(subject.EligibilityError):
                        subject.service(
                            self.api,
                            event_name="workflow_run",
                            event={"action": action, "workflow_run": {"id": 50}},
                            implementation=IMPLEMENTATION,
                            run_id=11,
                            output=output,
                            now=NOW,
                        )
                    self.assertFalse(output.exists())
                    self.assertEqual(self.api.posted, [])
                    evaluate.assert_not_called()
                    retry.assert_not_called()

    def test_closed_queue_no_action_requires_existing_exact_queue_binding(self) -> None:
        for defect in ("empty", "foreign-source", "wrong-head", "wrong-target"):
            with self.subTest(defect=defect):
                self.setUp()
                self.pending_callback("merge_group")
                self.api.responses["actions/runs/50"].update(status="completed")
                record: dict[str, Any] = {**pr_metadata(), "state": "closed", "mergeable": None}
                self.api.responses["pulls/17"] = record
                if defect == "empty":
                    self.queue([])
                elif defect == "foreign-source":
                    self.queue([self.node(source="9" * 40)])
                elif defect == "wrong-head":
                    self.queue([self.node(head="9" * 40)])
                else:
                    record["base"]["ref"] = "release/next"
                with (
                    tempfile.TemporaryDirectory() as temporary,
                    patch.object(subject, "evaluate") as evaluate,
                ):
                    output = Path(temporary) / subject.STATE_FILE
                    with self.assertRaises(subject.EligibilityError):
                        subject.service(
                            self.api,
                            event_name="workflow_run",
                            event={"action": "completed", "workflow_run": {"id": 50}},
                            implementation=IMPLEMENTATION,
                            run_id=11,
                            output=output,
                            now=NOW,
                        )
                    self.assertFalse(output.exists())
                    self.assertEqual(self.api.posted, [])
                    evaluate.assert_not_called()

    def test_deferred_callback_retains_trusted_observations_without_a_receipt(self) -> None:
        self.pending_callback("pull_request")
        previous = state_payload()
        previous["reruns"] = {HEAD: 50}
        with (
            tempfile.TemporaryDirectory() as temporary,
            patch.object(subject, "previous_state", return_value=previous),
            patch.object(subject, "evaluate") as evaluate,
        ):
            payload = subject.service(
                self.api,
                event_name="workflow_run",
                event={"workflow_run": {"id": 50}},
                implementation=IMPLEMENTATION,
                run_id=11,
                output=Path(temporary) / subject.STATE_FILE,
                now=NOW,
            )
        self.assertEqual(payload["dependency_state"], previous["dependency_state"])
        self.assertEqual(payload["reruns"], previous["reruns"])
        self.assertEqual(payload["reports"], {})
        self.assertNotIn("candidate_sha", payload["deferred_candidates"][0])
        self.assertEqual(self.api.posted, [])
        evaluate.assert_not_called()

    def test_workflow_null_mergeability_never_defers_terminal_or_malformed_pr(self) -> None:
        terminal: list[dict[str, Any]] = []
        for key, value in (
            ("state", "closed"),
            ("mergeable", False),
            ("mergeable", 1),
            ("mergeable", "pending"),
            ("merge_commit_sha", "main"),
            ("user", None),
            ("user", {"login": "bot", "type": []}),
        ):
            record = {**pr_metadata(), "mergeable": None, key: value}
            terminal.append(record)
        missing = pr_metadata()
        missing.pop("mergeable")
        terminal.append(missing)
        for section, field, value in (
            ("head", "sha", "broken"),
            ("base", "ref", ""),
            ("base", "ref", "bad\nref"),
        ):
            record = pr_metadata()
            record["mergeable"] = None
            record[section][field] = value
            terminal.append(record)
        foreign = pr_metadata()
        foreign["mergeable"] = None
        foreign["base"]["repo"]["full_name"] = "Other/project"
        terminal.append(foreign)
        stale = pr_metadata(source_head="9" * 40)
        stale["mergeable"] = None
        terminal.append(stale)
        for record in terminal:
            with self.subTest(metadata=record):
                self.setUp()
                self.pending_callback("pull_request")
                self.api.responses["pulls/17"] = record
                with (
                    tempfile.TemporaryDirectory() as temporary,
                    patch.object(subject, "evaluate") as evaluate,
                    patch.object(subject, "rerun_ready") as retry,
                ):
                    output = Path(temporary) / subject.STATE_FILE
                    with self.assertRaises(subject.EligibilityError) as raised:
                        subject.service(
                            self.api,
                            event_name="workflow_run",
                            event={"workflow_run": {"id": 50}},
                            implementation=IMPLEMENTATION,
                            run_id=11,
                            output=output,
                            now=NOW,
                        )
                    self.assertNotIsInstance(raised.exception, subject.CandidateComputationPending)
                    self.assertFalse(output.exists())
                    self.assertEqual(self.api.posted, [])
                    evaluate.assert_not_called()
                    retry.assert_not_called()

    def test_pending_queue_requires_exact_source_and_default_target(self) -> None:
        for mutation in ("target", "source"):
            with self.subTest(mutation=mutation):
                self.setUp()
                self.pending_callback("merge_group")
                if mutation == "target":
                    self.api.responses["pulls/17"]["base"]["ref"] = "release/next"
                else:
                    self.queue([self.node(source="9" * 40)])
                with (
                    tempfile.TemporaryDirectory() as temporary,
                    patch.object(subject, "evaluate") as evaluate,
                ):
                    output = Path(temporary) / subject.STATE_FILE
                    with self.assertRaises(subject.EligibilityError) as raised:
                        subject.service(
                            self.api,
                            event_name="workflow_run",
                            event={"workflow_run": {"id": 50}},
                            implementation=IMPLEMENTATION,
                            run_id=11,
                            output=output,
                            now=NOW,
                        )
                    self.assertNotIsInstance(raised.exception, subject.CandidateComputationPending)
                    self.assertFalse(output.exists())
                    self.assertEqual(self.api.posted, [])
                    evaluate.assert_not_called()

    def test_callback_readback_source_or_target_change_cannot_be_deferred(self) -> None:
        for mutation in ("source", "target"):
            with self.subTest(mutation=mutation):
                self.setUp()
                self.pending_callback("pull_request")
                self.api.responses["pulls/17"] = pr_metadata()
                get, reads = self.api.get, 0

                def moving(
                    suffix: str,
                    mutation: str = mutation,
                    get: Callable[[str], dict[str, Any]] = get,
                ) -> dict[str, Any]:
                    nonlocal reads
                    value = get(suffix)
                    if suffix == "pulls/17":
                        reads += 1
                        if reads == 2:
                            value.update(mergeable=None, merge_commit_sha=None)
                            if mutation == "source":
                                value["head"]["sha"] = "9" * 40
                            else:
                                value["base"]["ref"] = "release/next"
                    return value

                with (
                    tempfile.TemporaryDirectory() as temporary,
                    patch.object(self.api, "get", side_effect=moving),
                    patch.object(subject, "evaluate") as evaluate,
                ):
                    output = Path(temporary) / subject.STATE_FILE
                    with self.assertRaises(subject.EligibilityError) as raised:
                        subject.service(
                            self.api,
                            event_name="workflow_run",
                            event={"workflow_run": {"id": 50}},
                            implementation=IMPLEMENTATION,
                            run_id=11,
                            output=output,
                            now=NOW,
                        )
                    self.assertNotIsInstance(raised.exception, subject.CandidateComputationPending)
                    self.assertEqual(self.api.posted, [])
                    self.assertFalse(output.exists())
                    evaluate.assert_not_called()

    def test_service_persists_new_firstseen_even_when_dependency_policy_waits(self) -> None:
        self.api.responses["actions/runs/10"] = run_metadata()
        self.api.responses[
            "actions/workflows/dependency-eligibility.yml/runs?per_page=30&status=success"
        ] = {"workflow_runs": []}
        self.api.responses["pulls?state=open&per_page=100&sort=updated&direction=desc"] = [
            {"number": 17}
        ]
        self.api.responses["actions/workflows/ci.yml/runs?event=merge_group&per_page=20"] = {
            "workflow_runs": []
        }
        report = {
            "status": "waiting",
            "candidate_sha": HEAD,
            "pull_request": 17,
            "retry_at": "2026-11-06T12:00:00Z",
            "state": {"schema_version": 1, "first_seen": {"d" * 64: NOW.isoformat()}},
        }
        with (
            tempfile.TemporaryDirectory() as temporary,
            patch.object(subject, "evaluate", return_value=report),
        ):
            output = Path(temporary) / subject.STATE_FILE
            payload = subject.service(
                self.api,
                event_name="schedule",
                event={},
                implementation=IMPLEMENTATION,
                run_id=10,
                output=output,
                now=NOW,
            )
            self.assertTrue(output.is_file())
        self.assertEqual(payload["reports"][HEAD]["status"], "waiting")
        self.assertEqual(payload["dependency_state"]["first_seen"]["d" * 64], NOW.isoformat())
        self.assertEqual(self.api.posted[-1][1]["state"], "pending")

    def test_repository_metadata_uses_github_canonical_endpoint_without_trailing_slash(
        self,
    ) -> None:
        api = subject.GitHub(REPOSITORY, "token")
        with patch.object(api, "request", return_value={"default_branch": "main"}) as request:
            self.assertEqual(api.get(""), {"default_branch": "main"})
        request.assert_called_once_with("GET", f"/repos/{REPOSITORY}")

    def test_scheduled_scan_cursor_rotates_instead_of_starving_older_prs(self) -> None:
        self.api.responses["actions/runs/11"] = run_metadata(11)
        self.api.responses["pulls?state=open&per_page=100&sort=updated&direction=desc"] = [
            {"number": number} for number in (20, 19, 18, 17)
        ]
        self.api.responses["actions/workflows/ci.yml/runs?event=merge_group&per_page=20"] = {
            "workflow_runs": []
        }
        previous = state_payload()
        previous["scan_cursor"] = 18
        visited: list[int] = []

        def absent_candidate(api: subject.GitHub, number: int) -> subject.Candidate:
            visited.append(number)
            raise subject.EligibilityError("still computing merge candidate")

        with (
            tempfile.TemporaryDirectory() as temporary,
            patch.object(subject, "MAX_SCHEDULED_PRS", 2),
            patch.object(subject, "previous_state", return_value=previous),
            patch.object(subject, "pr_candidate", side_effect=absent_candidate),
        ):
            payload = subject.service(
                self.api,
                event_name="schedule",
                event={},
                implementation=IMPLEMENTATION,
                run_id=11,
                output=Path(temporary) / subject.STATE_FILE,
                now=NOW,
            )
            self.assertEqual(visited, [19, 20])
            self.assertEqual(payload["scan_cursor"], 20)
            previous["scan_cursor"] = payload["scan_cursor"]
            visited.clear()
            subject.service(
                self.api,
                event_name="schedule",
                event={},
                implementation=IMPLEMENTATION,
                run_id=11,
                output=Path(temporary) / subject.STATE_FILE,
                now=NOW,
            )
            self.assertEqual(visited, [17, 18])

    def test_budget_exhaustion_retains_cursor_at_last_evaluated_pr(self) -> None:
        self.api.responses["actions/runs/11"] = run_metadata(11)
        self.api.responses["pulls/18"] = pr_metadata(18, merge="d" * 40)
        self.bind_merge("d" * 40)
        self.api.responses["pulls?state=open&per_page=100&sort=updated&direction=desc"] = [
            {"number": 17},
            {"number": 18},
        ]
        self.api.responses["actions/workflows/ci.yml/runs?event=merge_group&per_page=20"] = {
            "workflow_runs": []
        }
        previous = state_payload()
        previous["scan_cursor"] = 16
        report = {
            "status": "waiting",
            "candidate_sha": HEAD,
            "pull_request": 17,
            "state": {"schema_version": 1, "first_seen": {}},
        }

        def evaluate(
            api: subject.GitHub, candidate: subject.Candidate, state: dict[str, Any], now: datetime
        ) -> dict[str, Any]:
            if candidate.number == 18:
                api.requests = 500
                raise subject.EligibilityError("network budget exhausted")
            return deepcopy(report)

        with (
            tempfile.TemporaryDirectory() as temporary,
            patch.object(subject, "previous_state", return_value=previous),
            patch.object(subject, "evaluate", side_effect=evaluate),
        ):
            payload = subject.service(
                self.api,
                event_name="schedule",
                event={},
                implementation=IMPLEMENTATION,
                run_id=11,
                output=Path(temporary) / subject.STATE_FILE,
                now=NOW,
            )
        self.assertEqual(payload["scan_cursor"], 17)

    def test_registry_budget_reserves_time_and_api_capacity_to_persist_state(self) -> None:
        self.api.requests = 450
        with self.assertRaises(URLError):
            subject.registry_fetch(self.api, "https://registry.npmjs.org/pkg/1.0.0")
        self.api.requests = 0
        with patch.object(subject.time, "monotonic", return_value=self.api.deadline - 10):
            with self.assertRaises(URLError):
                subject.registry_fetch(self.api, "https://registry.npmjs.org/pkg/1.0.0")

    def test_fixed_origin_allowlist_prevents_remote_expansion(self) -> None:
        with self.assertRaises(subject.EligibilityError):
            subject.GitHub("attacker/project", "token")
        with self.assertRaises(subject.EligibilityError):
            subject.GitHub(REPOSITORY, "token").request("GET", "https://attacker.invalid")


if __name__ == "__main__":
    unittest.main()
