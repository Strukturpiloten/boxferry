"""Independent graph, provider, crash and finite-effect recovery regressions."""

from __future__ import annotations

import base64
import hashlib
import io
import json
import tarfile
import tempfile
import unittest
import zipfile
from copy import deepcopy
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any
from unittest.mock import patch

from boxferry_ci import bootstrap
from boxferry_ci import eligibility as core
from boxferry_ci import recovery as subject

P, B, H, OLD_MERGE, N, C = (letter * 40 for letter in "abcdef")
REPOSITORY = "Strukturpiloten/compose-lens"
NOW = datetime(2026, 11, 4, 12, tzinfo=UTC)
CANDIDATE = core.Candidate(B, N, 17, H, "renovate[bot]", "Bot", "pull_request")
HISTORICAL = core.Candidate(P, OLD_MERGE, 17, H, "renovate[bot]", "Bot", "pull_request")

BASE_EXPRESSION = (
    "${{ github.event.pull_request.base.sha || github.event.merge_group.base_sha || github.sha }}"
)

# Fixed caller facts are independent expected inputs: the original run is H in
# REST, while the untrusted hint names OLD_MERGE. No fixture equates H, OLD_MERGE, N or actual X.
CALLER = f"""---
on:
  pull_request:
  merge_group:
    types: [checks_requested]
jobs:
  lockfile-release-age:
    name: Dependency eligibility
    needs: validation-plan
    if: needs.validation-plan.outputs.select_lockfile_release_age == 'true'
    permissions:
      actions: read
      contents: read
      statuses: read
    uses: {subject.READER_PATH}@{C}
    with:
      candidate_sha: ${{{{ github.sha }}}}
      automation_sha: {C}
      policy_sha: {BASE_EXPRESSION}
  pr-gate:
    name: Required CI
    if: always()
    needs: [validation-plan, lockfile-release-age]
    steps:
      - env:
          BASE_SHA: ${{{{ github.event.merge_group.base_sha }}}}
          HEAD_SHA: ${{{{ github.event.pull_request.head.sha || github.sha }}}}
          AUTOMATION_SHA: {C}
        run: |
          python -m boxferry_ci gate \\
            --tested-sha "${{GITHUB_SHA}}" --policy-sha "${{POLICY_SHA}}" \\
            --plan-json "${{PLAN_JSON}}" --needs-json "${{NEEDS_JSON}}"
""".encode()


def passing(candidate: core.Candidate) -> dict[str, Any]:
    return {
        "status": "pass",
        "candidate_sha": candidate.head,
        "base_sha": candidate.base,
        "source_head_sha": H,
        "pull_request": 17,
        "scope": "pull_request",
    }


def blob(data: bytes) -> str:
    return hashlib.sha1(
        b"blob " + str(len(data)).encode() + b"\0" + data, usedforsecurity=False
    ).hexdigest()


def archive(payload: dict[str, Any], name: str = subject.HINT_FILE) -> bytes:
    output = io.BytesIO()
    with zipfile.ZipFile(output, "w") as writer:
        writer.writestr(name, json.dumps(payload))
    return output.getvalue()


class API(core.GitHub):
    def __init__(self) -> None:
        super().__init__(REPOSITORY, "test-token")
        self.responses: dict[str, Any] = {}
        self.archives: dict[int, bytes] = {}
        self.posts: list[tuple[str, dict[str, Any]]] = []
        self.fail_post = False
        self.history: dict[str, Any] = {"workflow_runs": [], "total_count": 0}
        self.timeline: dict[str, Any] = {
            "data": {
                "repository": {
                    "nameWithOwner": REPOSITORY,
                    "pullRequest": {
                        "number": 17,
                        "state": "OPEN",
                        "headRefOid": H,
                        "timelineItems": {
                            "totalCount": 1000,
                            "nodes": [],
                            "pageInfo": {
                                "hasPreviousPage": False,
                                "hasNextPage": False,
                                "startCursor": None,
                                "endCursor": None,
                            },
                        },
                    },
                }
            }
        }

    def get(self, suffix: str) -> dict[str, Any]:
        if suffix.startswith("actions/workflows/dependency-eligibility.yml/runs?"):
            return deepcopy(self.history)
        value = self.responses.get(suffix)
        if not isinstance(value, dict):
            raise core.MissingResource(suffix)
        return deepcopy(value)

    def items(self, suffix: str) -> list[dict[str, Any]]:
        value = self.responses.get(suffix)
        if not isinstance(value, list):
            raise core.MissingResource(suffix)
        return deepcopy(value)

    def artifact(self, artifact_id: int) -> bytes:
        return self.archives[artifact_id]

    def request(self, method: str, path: str, data: dict[str, Any] | None = None) -> object:
        if method == "GET" and path == f"/repos/Strukturpiloten/boxferry/git/commits/{C}":
            return {"sha": C, "committer": {"date": "2026-11-03T12:00:00Z"}}
        if method != "POST" or path != "/graphql" or data is None:
            raise AssertionError("unexpected external request")
        if "itemTypes:[LABELED_EVENT,UNLABELED_EVENT]" not in data["query"]:
            raise AssertionError("label history must exclude unrelated timeline events")
        return deepcopy(self.timeline)

    def post(self, suffix: str, data: dict[str, Any]) -> None:
        self.posts.append((suffix, data))
        if self.fail_post and not suffix.startswith("statuses/"):
            raise core.EligibilityError("transport timeout after request acceptance")

    def files(self, sha: str, files: dict[str, bytes]) -> None:
        self.responses[f"git/trees/{sha}?recursive=1"] = {
            "truncated": False,
            "tree": [
                {"path": path, "type": "blob", "mode": "100644", "sha": blob(data)}
                for path, data in files.items()
            ],
        }
        for path, data in files.items():
            self.responses[f"contents/{path}?ref={sha}"] = {
                "type": "file",
                "size": len(data),
                "encoding": "base64",
                "content": base64.b64encode(data).decode(),
            }


class RecoveryTests(unittest.TestCase):
    def test_common_preset_uses_trusted_fixed_label_without_permanent_keep_updated(self) -> None:
        root = Path(__file__).resolve().parents[2]
        common = json.loads((root / "renovate-presets/common.json").read_text())
        self.assertEqual(common["rebaseLabel"], subject.REBASE_LABEL)
        self.assertFalse(common.get("keepUpdatedLabel"))

    def setUp(self) -> None:
        self.api = API()
        self.original: dict[str, Any] = {
            "id": 50,
            "run_attempt": 1,
            "name": "CI",
            "path": ".github/workflows/ci.yml",
            "repository": {"full_name": REPOSITORY},
            "head_repository": {"full_name": REPOSITORY},
            "head_sha": H,
            "event": "pull_request",
            "status": "completed",
            "conclusion": "failure",
            "created_at": "2026-11-01T12:00:00Z",
            "pull_requests": [{"number": 17, "head": {"sha": H}, "base": {"sha": P}}],
            "referenced_workflows": [{"path": f"{subject.READER_PATH}@{C}", "sha": C}],
        }
        self.api.responses["actions/runs/50"] = self.original
        self.jobs: list[dict[str, Any]] = [
            {
                "name": "Dependency eligibility / Dependency eligibility",
                "status": "completed",
                "conclusion": "failure",
                "run_id": 50,
                "run_attempt": 1,
            },
            {
                "name": "Required CI",
                "run_id": 50,
                "run_attempt": 1,
                "status": "completed",
                "conclusion": "failure",
                "check_run_url": f"https://api.github.com/repos/{REPOSITORY}/check-runs/800",
            },
            {"name": "Rust quality", "status": "completed", "conclusion": "success"},
        ]
        self.api.responses["actions/runs/50/attempts/1/jobs?per_page=100"] = {
            "jobs": self.jobs,
            "total_count": 3,
        }
        self.check: dict[str, Any] = {
            "id": 800,
            "name": "Required CI",
            "head_sha": H,
            "app": {"id": 15368, "slug": "github-actions"},
            "status": "completed",
            "conclusion": "failure",
        }
        self.api.responses["check-runs/800"] = self.check
        self.hint = {
            "schema": 1,
            "repository": REPOSITORY,
            "run_id": 50,
            "run_attempt": 1,
            "pull_request": 17,
            "source_head_sha": H,
            "candidate_sha": OLD_MERGE,
            "base_sha": P,
        }
        self.api.archives[1] = archive(self.hint)
        self.api.responses["actions/runs/50/artifacts?per_page=100"] = {
            "total_count": 1,
            "artifacts": [
                {
                    "id": 1,
                    "name": "dependency-candidate-hint-50-1",
                    "size_in_bytes": 500,
                    "expired": False,
                }
            ],
        }
        for head, base in ((OLD_MERGE, P), (N, B)):
            self.api.responses[f"git/commits/{head}"] = {
                "sha": head,
                "parents": [{"sha": base}, {"sha": H}],
            }
        self.api.responses[f"compare/{P}...{B}"] = {
            "status": "ahead",
            "merge_base_commit": {"sha": P},
        }
        manifest = json.dumps(
            {
                "schema": 1,
                "interface": 1,
                "repository": "Strukturpiloten/boxferry",
                "revision": C,
                "policy": "scripts/validation-policy.json",
            }
        ).encode()
        files = {
            ".github/workflows/ci.yml": CALLER,
            ".github/automation.json": manifest,
            "scripts/validation-policy.json": b'{"schema": 1}',
        }
        for sha in (P, B, H, OLD_MERGE, N):
            self.api.files(sha, files)
        self.rules: list[dict[str, Any]] = [
            {
                "type": "merge_queue",
                "ruleset_id": 9,
                "parameters": {
                    "max_entries_to_build": 1,
                    "max_entries_to_merge": 1,
                    "grouping_strategy": "ALLGREEN",
                },
            },
            {
                "type": "required_status_checks",
                "ruleset_id": 9,
                "parameters": {"required_status_checks": [{"context": "Required CI"}]},
            },
        ]
        self.api.responses["rules/branches/main?per_page=100&page=1"] = self.rules
        self.api.responses["rulesets/9?includes_parents=true"] = {
            "id": 9,
            "bypass_actors": [
                {"actor_type": "OrganizationAdmin", "actor_id": 1, "bypass_mode": "always"}
            ],
        }
        self.api.responses["labels/rebase"] = {"name": "rebase"}
        self.api.responses["issues/17/labels?per_page=100"] = []
        self.api.responses["issues/17/events?per_page=100"] = []
        self.reservations: dict[str, Any] = {}
        self.reports = {N: passing(CANDIDATE)}

    def recover(self, previous: dict[str, Any] | None = None) -> dict[str, Any]:
        with (
            patch.object(subject, "reader_closure"),
            patch.object(subject, "require_effect"),
            patch.object(core, "still_current", return_value=True),
            patch.object(
                core,
                "evaluate",
                side_effect=lambda api, candidate, state, now: {
                    **passing(candidate),
                    "state": state,
                },
            ),
        ):
            return subject.recover_run(
                self.api,
                50,
                CANDIDATE,
                passing(CANDIDATE),
                {},
                previous,
                self.reservations,
                self.reports,
                implementation=C,
                service_run=101 if previous else 100,
                branch="main",
                now=NOW,
                context=subject.ServiceContext(
                    previous,
                    {"fence": None},
                    NOW,
                    101 if previous else 100,
                    1,
                    True,
                    previous is not None,
                ),
            )

    def effects(self) -> list[tuple[str, dict[str, Any]]]:
        return [(path, data) for path, data in self.api.posts if not path.startswith("statuses/")]

    def prepared(self) -> dict[str, Any]:
        self.recover()
        return {"run_id": 100, "reports": deepcopy(self.reports)}

    def test_hint_is_bounded_and_run_attempt_scoped_without_claiming_actual_x(self) -> None:
        self.assertEqual(
            subject.candidate_hint(self.api, self.original, CANDIDATE)["candidate_sha"], OLD_MERGE
        )
        for key, value in (
            ("run_id", 51),
            ("run_attempt", 2),
            ("source_head_sha", N),
            ("schema", True),
        ):
            altered = {**self.hint, key: value}
            self.api.archives[1] = archive(altered)
            with self.subTest(key=key), self.assertRaises(ValueError):
                subject.candidate_hint(self.api, self.original, CANDIDATE)
        self.api.archives[1] = archive(self.hint, "../dependency-candidate-hint.json")
        with self.assertRaises(ValueError):
            subject.candidate_hint(self.api, self.original, CANDIDATE)

    def test_ordered_direct_parents_are_required_even_when_ancestry_would_pass(self) -> None:
        subject.ordered_merge(self.api, OLD_MERGE, P, H)
        for parents in ([H, P], [P], [P, H, B], [P, N]):
            self.api.responses[f"git/commits/{OLD_MERGE}"]["parents"] = [
                {"sha": sha} for sha in parents
            ]
            with self.subTest(parents=parents), self.assertRaises(ValueError):
                subject.ordered_merge(self.api, OLD_MERGE, P, H)

    def test_original_run_checks_actual_job_provider_and_never_equates_rest_h_to_o(self) -> None:
        self.assertEqual(subject.original_run(self.api, 50, CANDIDATE, C)["head_sha"], H)
        for field, value in (("head_sha", OLD_MERGE), ("id", 900), ("name", "Another CI")):
            with self.subTest(field=field):
                original = self.check[field]
                self.check[field] = value
                with self.assertRaises(ValueError):
                    subject.original_run(self.api, 50, CANDIDATE, C)
                self.check[field] = original
        self.check["app"] = {"id": 1, "slug": "github-actions"}
        with self.assertRaises(ValueError):
            subject.original_run(self.api, 50, CANDIDATE, C)

    def test_duplicate_aggregate_wrong_attempt_and_other_job_failure_refuse(self) -> None:
        for mutation in (
            lambda: self.jobs.append(deepcopy(self.jobs[1])),
            lambda: self.jobs[1].update(run_attempt=2),
            lambda: self.jobs[2].update(conclusion="failure"),
            lambda: self.jobs[1].update(check_run_url="https://attacker.invalid/check-runs/800"),
        ):
            saved = deepcopy(self.jobs)
            mutation()
            self.api.responses["actions/runs/50/attempts/1/jobs?per_page=100"]["total_count"] = len(
                self.jobs
            )
            with self.assertRaises(ValueError):
                subject.original_run(self.api, 50, CANDIDATE, C)
            self.jobs[:] = saved

    def test_unbound_rule_is_truthfully_recorded_but_wrong_provider_and_bypass_refuse(self) -> None:
        self.assertEqual(subject.protection(self.api, "main")["required_ci_binding"], "any-app")
        check = self.rules[1]["parameters"]["required_status_checks"][0]
        check["integration_id"] = 15368
        self.assertEqual(
            subject.protection(self.api, "main")["required_ci_binding"], "github-actions"
        )
        check["integration_id"] = 7
        with self.assertRaises(ValueError):
            subject.protection(self.api, "main")
        check.pop("integration_id")
        self.api.responses["rulesets/9?includes_parents=true"]["bypass_actors"] = [
            {"actor_type": "Integration", "actor_id": 1, "bypass_mode": "always"}
        ]
        with self.assertRaises(ValueError):
            subject.protection(self.api, "main")

    def test_workflow_added_changed_reader_or_truncated_inventory_refuse(self) -> None:
        with patch.object(subject, "reader_closure"):
            historical, kind, reader = subject.graph_proof(self.api, self.original, CANDIDATE, C)
        self.assertEqual((historical, kind, reader), (HISTORICAL, "retained-graph", C))
        tree = self.api.responses[f"git/trees/{H}?recursive=1"]
        tree["tree"].append(
            {
                "path": ".github/workflows/duplicate.yml",
                "type": "blob",
                "mode": "100644",
                "sha": "9" * 40,
            }
        )
        with self.assertRaises(ValueError):
            subject.graph_proof(self.api, self.original, CANDIDATE, C)
        tree["truncated"] = True
        with self.assertRaises(ValueError):
            subject.inventory(self.api, H)
        with self.assertRaises(ValueError):
            subject.reader_shape(
                CALLER.replace(b"candidate_sha: ${{ github.sha }}", b"candidate_sha: wrong"), C
            )

    def test_prepare_completed_reports_then_one_effect_and_no_second_attempt(self) -> None:
        previous = self.prepared()
        self.assertEqual(self.effects(), [])
        self.assertEqual(next(iter(self.reservations.values()))["phase"], "prepared")
        self.assertEqual(set(self.reports), {OLD_MERGE, N})
        self.assertNotIn(H, self.reports)
        self.recover(previous)
        self.assertEqual(self.effects(), [("actions/runs/50/rerun-failed-jobs", {})])
        self.recover(previous)
        self.assertEqual(len(self.effects()), 1)

    def test_current_base_churn_requires_a_new_completed_report_without_resetting_budget(
        self,
    ) -> None:
        previous = self.prepared()
        previous["reports"][N]["base_sha"] = "1" * 40
        self.recover(previous)
        self.assertEqual(self.effects(), [])
        self.assertEqual(len(self.reservations), 1)
        self.assertEqual(next(iter(self.reservations.values()))["phase"], "prepared")

    def test_transport_timeout_consumes_budget_even_if_attempt_did_not_yet_advance(self) -> None:
        previous = self.prepared()
        self.api.fail_post = True
        self.recover(previous)
        self.assertEqual(next(iter(self.reservations.values()))["phase"], "uncertain")
        self.recover(previous)
        self.assertEqual(len(self.effects()), 1)

    def test_incremented_original_attempt_cannot_receive_another_post(self) -> None:
        previous = self.prepared()
        self.original["run_attempt"] = 2
        with self.assertRaises(ValueError):
            self.recover(previous)
        self.assertEqual(self.effects(), [])
        self.assertEqual(next(iter(self.reservations.values()))["phase"], "suspended")

    def test_fixed_label_is_never_readded_after_bot_removal_or_when_missing(self) -> None:
        subject.label_unused(self.api, 17, NOW, source_head=H, since=self.original["created_at"])
        timeline = self.api.timeline["data"]["repository"]["pullRequest"]["timelineItems"]
        timeline["nodes"] = [
            {
                "__typename": "UnlabeledEvent",
                "label": {"name": "rebase"},
                "createdAt": "2026-11-04T11:00:00Z",
            }
        ]
        with self.assertRaisesRegex(ValueError, "already consumed"):
            subject.label_unused(
                self.api, 17, NOW, source_head=H, since=self.original["created_at"]
            )
        self.api.responses.pop("labels/rebase")
        with self.assertRaisesRegex(ValueError, "provision the fixed"):
            subject.label_unused(
                self.api, 17, NOW, source_head=H, since=self.original["created_at"]
            )

    def test_handoff_only_applies_one_fixed_label_and_never_claims_ci_success(self) -> None:
        with patch.object(subject, "graph_proof", return_value=(HISTORICAL, "handoff", C)):
            previous = self.prepared()
            self.recover(previous)
        self.assertEqual(self.effects(), [("issues/17/labels", {"labels": ["rebase"]})])
        self.assertNotIn(H, self.reports)
        self.assertFalse(
            any(
                path.startswith("statuses/") and data.get("context") == "Required CI"
                for path, data in self.api.posts
            )
        )

    def test_fixed_reader_closure_includes_recovery_and_refuses_missing_or_changed_code(
        self,
    ) -> None:
        canonical = API()
        root = Path(subject.__file__).resolve().parents[3]
        files = {path: (root / path).read_bytes() for path in subject.READER_CLOSURE}
        self.assertIn("automation/src/boxferry_ci/recovery.py", files)
        canonical.files(C, files)
        with patch.object(core, "GitHub", return_value=canonical):
            subject.reader_closure(self.api, C)
            files["automation/src/boxferry_ci/eligibility.py"] = b"ADMISSION_READER_CONTRACT = 1\n"
            canonical.files(C, files)
            with self.assertRaises(ValueError):
                subject.reader_closure(self.api, C)

    def test_all_seven_production_caller_shapes_and_changed_aggregate_refuse(self) -> None:
        path = Path(__file__).parent / "fixtures/recovery-callers.json"
        fixture = json.loads(path.read_text())
        self.assertEqual(len(fixture["repositories"]), 7)
        for repository, item in fixture["repositories"].items():
            caller = item["caller"].encode()
            with self.subTest(repository=repository):
                subject.reader_shape(caller, C)
                for changed in (
                    caller.replace(b"--tested-sha", b"--unbound-sha"),
                    caller.replace(b"    name: Required CI", b"    name: Unrequired CI"),
                    caller + b"  duplicate:\n    name: Required CI\n",
                    caller.replace(
                        b"      candidate_sha: ${{ github.sha }}",
                        b"      candidate_sha: ${{ github.event.pull_request.head.sha }}",
                    ),
                ):
                    with self.assertRaises(ValueError):
                        subject.reader_shape(changed, C)

    def test_forged_historical_hint_cannot_green_actual_x_or_source_h(self) -> None:
        actual_x = "9" * 40
        previous = self.prepared()
        self.recover(previous)
        self.assertNotIn(actual_x, self.reports)
        self.assertNotIn(H, self.reports)
        self.api.responses[f"commits/{actual_x}/statuses?per_page=100"] = []
        self.assertIsNone(
            core.candidate_receipt(
                self.api,
                actual_x,
                C,
                "main",
                NOW,
                expected_base_sha=P,
            )
        )
        self.assertTrue(
            all(path not in {f"statuses/{actual_x}", f"statuses/{H}"} for path, _ in self.api.posts)
        )

    def test_all_consumer_tokens_read_only_fixed_canonical_boxferry_closure(self) -> None:
        original = core.GitHub
        root = Path(subject.__file__).resolve().parents[3]
        contents = API()
        contents.files(C, {path: (root / path).read_bytes() for path in subject.READER_CLOSURE})
        reads: list[str] = []

        class Canonical(original):
            def request(self, method: str, path: str, data: dict[str, Any] | None = None) -> object:
                if method != "GET" or data is not None:
                    raise AssertionError("canonical source request must be read-only")
                prefix = "/repos/Strukturpiloten/boxferry/"
                if not path.startswith(prefix):
                    raise AssertionError("canonical source origin/repository changed")
                reads.append(path)
                return contents.get(path.removeprefix(prefix))

        for repository in sorted(core.REPOSITORIES):
            consumer = API()
            consumer.repository = repository
            consumer.token = "scoped-" + repository.rsplit("/", 1)[1]
            with (
                self.subTest(repository=repository),
                patch.object(
                    core,
                    "GitHub",
                    side_effect=Canonical,
                ) as factory,
            ):
                subject.reader_closure(consumer, C)
                factory.assert_called_once_with("Strukturpiloten/boxferry", consumer.token)
        self.assertEqual(len(reads), 7 * (len(subject.READER_CLOSURE) + 1))

    def test_source_archive_retains_new_recovery_module_without_additional_roots(self) -> None:
        source = Path(subject.__file__)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive_path = root / "source.tar"
            path = "automation/src/boxferry_ci/recovery.py"
            with tarfile.open(archive_path, "w") as output:
                info = tarfile.TarInfo(path)
                info.size = source.stat().st_size
                output.addfile(info, io.BytesIO(source.read_bytes()))
            destination = root / "prepared"
            bootstrap.extract_archive(archive_path, destination)
            self.assertEqual((destination / path).read_bytes(), source.read_bytes())

    def test_real_service_persists_two_truthful_reports_before_its_one_recovery_effect(
        self,
    ) -> None:
        self.api.responses[""] = {"default_branch": "main"}
        self.api.responses[f"compare/{B}...main"] = {"status": "ahead"}
        self.api.responses[f"actions/workflows/ci.yml/runs?head_sha={H}&per_page=20"] = {
            "workflow_runs": [self.original],
        }
        evaluated: list[str] = []

        def evaluate(
            api: core.GitHub, candidate: core.Candidate, state: dict[str, Any], now: datetime
        ) -> dict[str, Any]:
            evaluated.append(candidate.head)
            return {**passing(candidate), "state": state}

        previous = None
        with tempfile.TemporaryDirectory() as temporary:
            for identifier in (100, 101, 102):
                self.api.responses[f"actions/runs/{identifier}"] = {
                    "id": identifier,
                    "run_attempt": 1,
                    "created_at": "2026-11-04T11:00:00Z",
                    "event": "workflow_run",
                    "path": core.WORKFLOW_PATH,
                    "head_branch": "main",
                    "head_sha": B,
                    "status": "completed",
                    "conclusion": "success",
                    "repository": {"full_name": REPOSITORY},
                    "referenced_workflows": [
                        {"path": f"{core.SHARED_WORKFLOW_PATH}@{C}", "sha": C}
                    ],
                }
                self.api.history = {
                    "total_count": identifier - 99,
                    "workflow_runs": [{"id": run} for run in range(100, identifier + 1)],
                }
                output = Path(temporary) / f"state-{identifier}.json"
                with (
                    patch.object(core, "previous_state", return_value=previous),
                    patch.object(core, "pr_candidate", return_value=CANDIDATE),
                    patch.object(core, "still_current", return_value=True),
                    patch.object(core, "evaluate", side_effect=evaluate),
                    patch.object(subject, "reader_closure"),
                ):
                    payload = core.service(
                        self.api,
                        event_name="workflow_run",
                        event={"workflow_run": {"id": 50}},
                        implementation=C,
                        run_id=identifier,
                        output=output,
                        now=NOW,
                    )
                self.assertEqual(json.loads(output.read_text()), payload)
                self.assertEqual(payload["infrastructure_errors"], [])
                if identifier == 100:
                    self.assertEqual(set(payload["reports"]), {N, OLD_MERGE})
                    self.assertEqual(evaluated, [N, OLD_MERGE])
                    self.assertEqual(self.effects(), [])
                    self.api.archives[2] = archive(payload, core.STATE_FILE)
                    self.api.responses["actions/runs/100/artifacts?per_page=100"] = {
                        "artifacts": [
                            {
                                "id": 2,
                                "name": core.ARTIFACT_NAME,
                                "size_in_bytes": len(self.api.archives[2]),
                                "expired": False,
                            }
                        ],
                    }
                    self.api.history = {"total_count": 1, "workflow_runs": [{"id": 100}]}
                else:
                    self.assertEqual(self.effects(), [("actions/runs/50/rerun-failed-jobs", {})])
                artifact_id = identifier + 1000
                self.api.archives[artifact_id] = archive(payload, core.STATE_FILE)
                self.api.responses[f"actions/runs/{identifier}/artifacts?per_page=100"] = {
                    "artifacts": [
                        {
                            "id": artifact_id,
                            "name": core.ARTIFACT_NAME,
                            "size_in_bytes": len(self.api.archives[artifact_id]),
                            "expired": False,
                        }
                    ],
                }
                previous = payload
            self.assertEqual(next(iter(previous["recoveries"].values()))["phase"], "attempted")
        self.assertTrue(
            all(
                path != f"statuses/{H}" and data.get("context") != "Required CI"
                for path, data in self.api.posts
            )
        )

    def test_filtered_label_history_ignores_force_push_noise_but_refuses_missing_pages(
        self,
    ) -> None:
        timeline = self.api.timeline["data"]["repository"]["pullRequest"]["timelineItems"]
        # Observed GitHub behavior: totalCount describes unrelated timeline noise.
        timeline["totalCount"] = 10000
        subject.label_unused(self.api, 17, NOW, source_head=H, since=self.original["created_at"])
        timeline["nodes"] = [
            {
                "__typename": "LabeledEvent",
                "label": {"name": "dependencies"},
                "createdAt": "2026-11-04T11:00:00Z",
            }
        ]
        subject.label_unused(self.api, 17, NOW, source_head=H, since=self.original["created_at"])
        for field in ("hasPreviousPage", "hasNextPage"):
            timeline["pageInfo"][field] = True
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "incomplete"):
                subject.label_unused(
                    self.api, 17, NOW, source_head=H, since=self.original["created_at"]
                )
            timeline["pageInfo"][field] = False
        timeline["nodes"][0]["__typename"] = "HeadRefForcePushedEvent"
        with self.assertRaisesRegex(ValueError, "unsupported"):
            subject.label_unused(
                self.api, 17, NOW, source_head=H, since=self.original["created_at"]
            )

    def test_prior_equal_base_retry_consumes_original_run_budget_across_base_churn(self) -> None:
        previous = self.prepared()
        previous["reruns"] = {"8" * 40: 50}
        self.recover(previous)
        self.assertEqual(self.effects(), [])
        self.assertEqual(self.original["run_attempt"], 1)

    def test_oversized_ambiguous_and_capability_claiming_hints_cannot_supply_authority(
        self,
    ) -> None:
        listing = self.api.responses["actions/runs/50/artifacts?per_page=100"]
        original = deepcopy(listing)
        listing["artifacts"][0]["size_in_bytes"] = subject.HINT_LIMIT + 1
        with self.assertRaises(ValueError):
            subject.candidate_hint(self.api, self.original, CANDIDATE)
        listing.clear()
        listing.update(deepcopy(original))
        listing["artifacts"].append(deepcopy(listing["artifacts"][0]))
        listing["total_count"] = 2
        with self.assertRaises(ValueError):
            subject.candidate_hint(self.api, self.original, CANDIDATE)
        listing.clear()
        listing.update(original)
        self.api.archives[1] = archive({**self.hint, "reader_contract": 2})
        with self.assertRaises(ValueError):
            subject.candidate_hint(self.api, self.original, CANDIDATE)

    def test_earlier_created_service_cannot_prepare_a_future_original_run(self) -> None:
        self.original["created_at"] = (NOW + timedelta(hours=1)).isoformat()
        with self.assertRaisesRegex(ValueError, "future original CI"):
            self.recover()
        self.assertEqual(self.effects(), [])
        self.assertEqual(self.reservations, {})

    def test_real_authority_transition_uses_fixed_label_without_old_a_receipt(self) -> None:
        old_authority = "7" * 40
        self.original["referenced_workflows"] = [
            {
                "path": f"{subject.READER_PATH}@{old_authority}",
                "sha": old_authority,
            }
        ]
        manifest = json.dumps(
            {
                "schema": 1,
                "interface": 1,
                "repository": "Strukturpiloten/boxferry",
                "revision": old_authority,
                "policy": "scripts/validation-policy.json",
            }
        ).encode()
        old_files = {
            ".github/workflows/ci.yml": CALLER.replace(C.encode(), old_authority.encode()),
            ".github/automation.json": manifest,
            "scripts/validation-policy.json": b'{"schema": 1}',
        }
        for sha in (P, H, OLD_MERGE):
            self.api.files(sha, old_files)
        historical, kind, reader = subject.graph_proof(self.api, self.original, CANDIDATE, C)
        self.assertEqual((historical, kind, reader), (HISTORICAL, "handoff", old_authority))
        previous = self.prepared()
        self.recover(previous)
        self.assertEqual(self.effects(), [("issues/17/labels", {"labels": ["rebase"]})])
        self.assertTrue(
            all(
                path != f"statuses/{H}" and data.get("context") != "Required CI"
                for path, data in self.api.posts
            )
        )

    def test_hinted_recovery_rejects_unproved_skipped_job_without_old_report(self) -> None:
        self.jobs[2]["conclusion"] = "skipped"
        with self.assertRaisesRegex(ValueError, "unsupported job outcomes"):
            self.recover()
        self.assertEqual(set(self.reports), {N})
        self.assertEqual(self.effects(), [])
        self.assertEqual(self.reservations, {})


class CheckpointAPI(API):
    def __init__(self) -> None:
        super().__init__()
        self.runs: dict[int, dict[str, Any]] = {}
        self.queries: list[str] = []
        self.floor = datetime(2026, 11, 1, tzinfo=UTC)
        self.canonical_sha = C
        self.extra_total = 0

    def request(self, method: str, path: str, data: dict[str, Any] | None = None) -> object:
        if method == "GET" and path == f"/repos/Strukturpiloten/boxferry/git/commits/{C}":
            return {"sha": self.canonical_sha, "committer": {"date": self.floor.isoformat()}}
        return super().request(method, path, data)

    def get(self, suffix: str) -> dict[str, Any]:
        if suffix.startswith(
            "actions/workflows/dependency-eligibility.yml/runs?per_page=100&created="
        ):
            from urllib.parse import parse_qs

            self.queries.append(suffix)
            lower = core.utc(parse_qs(suffix.split("?", 1)[1])["created"][0][2:])
            matching = [run for run in self.runs.values() if core.utc(run["created_at"]) >= lower]
            # Deliberately unordered: correctness must not depend on API sorting.
            matching = sorted(matching, key=lambda run: run["id"] % 7)
            return {
                "total_count": len(matching) + self.extra_total,
                "workflow_runs": [{"id": run["id"]} for run in matching[:100]],
            }
        return super().get(suffix)

    def add(self, run: dict[str, Any], state: dict[str, Any] | None = None) -> None:
        identifier = run["id"]
        self.runs[identifier] = run
        self.responses[f"actions/runs/{identifier}"] = run
        if state is not None:
            artifact_id = identifier + 10000
            self.archives[artifact_id] = archive(state, core.STATE_FILE)
            self.responses[f"actions/runs/{identifier}/artifacts?per_page=100"] = {
                "artifacts": [
                    {
                        "id": artifact_id,
                        "name": core.ARTIFACT_NAME,
                        "size_in_bytes": len(self.archives[artifact_id]),
                        "expired": False,
                    }
                ],
            }


class ServiceCheckpointTests(unittest.TestCase):
    """Expected scenarios independently specified in the 25-case reviewed protocol."""

    def setUp(self) -> None:
        self.api = CheckpointAPI()
        self.now = NOW + timedelta(hours=6)
        self.api.responses[f"compare/{B}...main"] = {"status": "ahead"}
        self.current = self.service_run(2000, NOW, status="in_progress", conclusion=None)
        self.checkpoint = self.service_run(1800, NOW - timedelta(hours=1))
        self.previous = self.state(self.checkpoint)
        self.api.add(self.checkpoint, self.previous)
        self.api.add(self.current)

    def service_run(
        self,
        identifier: int,
        created: datetime,
        *,
        status: str = "completed",
        conclusion: str | None = "success",
        attempt: int = 1,
        implementation: str = C,
    ) -> dict[str, Any]:
        return {
            "id": identifier,
            "run_attempt": attempt,
            "created_at": created.isoformat(),
            "event": "schedule",
            "path": core.WORKFLOW_PATH,
            "head_branch": "main",
            "head_sha": B,
            "status": status,
            "conclusion": conclusion,
            "repository": {"full_name": REPOSITORY},
            "referenced_workflows": [
                {"path": f"{core.SHARED_WORKFLOW_PATH}@{implementation}", "sha": implementation}
            ],
        }

    def state(self, run: dict[str, Any], *, fence: str | None = None) -> dict[str, Any]:
        return {
            "schema": 1,
            "repository": REPOSITORY,
            "workflow_path": core.WORKFLOW_PATH,
            "implementation_revision": C,
            "run_id": run["id"],
            "run_attempt": run["run_attempt"],
            "observed_at": self.now.isoformat(),
            "dependency_state": {"schema_version": 1, "first_seen": {}},
            "reports": {},
            "reruns": {},
            "recoveries": {},
            "recovery_frontier": {
                "protocol": 1,
                "floor": self.api.floor.isoformat(),
                "service_created_at": core.utc(run["created_at"]).isoformat(),
                "authority": True,
                "fence": fence,
                "pending": [],
                "predecessor_run_id": None,
            },
        }

    def context(
        self, previous: dict[str, Any] | None = None, *, genesis: bool = False
    ) -> subject.ServiceContext:
        return subject.service_context(
            self.api,
            self.current,
            None if genesis else self.previous if previous is None else previous,
            C,
            "main",
            self.now,
        )

    def persist(self, context: subject.ServiceContext) -> dict[str, Any]:
        state = self.state(self.current)
        state["recovery_frontier"] = deepcopy(context.frontier)
        if context.previous is not None:
            state["reruns"] = deepcopy(context.previous["reruns"])
            state["recoveries"] = deepcopy(context.previous.get("recoveries", {}))
        completed = {**self.current, "status": "completed", "conclusion": "success"}
        self.api.add(completed, state)
        return state

    def next_run(self, identifier: int = 2200) -> None:
        self.current = self.service_run(
            identifier, NOW + timedelta(hours=2), status="in_progress", conclusion=None
        )
        self.api.add(self.current)

    def test_72_hour_hold_uses_recent_checkpoint_despite_150_archived_callbacks(self) -> None:
        for identifier in range(1, 151):
            self.api.add(self.service_run(identifier, NOW - timedelta(days=2)))
        context = self.context()
        self.assertTrue(context.can_effect)
        assert context.checkpoint_run is not None
        self.assertEqual(context.checkpoint_run["id"], 1800)
        self.assertEqual(len(self.api.queries), 1)
        from urllib.parse import parse_qs

        lower = parse_qs(self.api.queries[0].split("?", 1)[1])["created"][0]
        self.assertEqual(lower, ">=" + self.checkpoint["created_at"])
        assert context.previous is not None
        self.assertEqual(context.previous["run_id"], 1800)

    def test_clean_genesis_seeds_only_then_completed_checkpoint_may_authorize(self) -> None:
        self.api.runs.pop(1800)
        first = self.context(genesis=True)
        self.assertTrue(first.can_prepare)
        self.assertFalse(first.can_effect)
        self.assertIsNone(first.frontier["fence"])
        seeded = self.persist(first)
        self.next_run()
        later = self.context(seeded)
        self.assertTrue(later.can_effect)

    def test_both_creation_and_id_ordering_are_required_without_skipping_newer(self) -> None:
        for checkpoint_id, checkpoint_time in (
            (1800, NOW + timedelta(hours=1)),
            (2001, NOW - timedelta(hours=1)),
        ):
            with self.subTest(identifier=checkpoint_id):
                self.setUp()
                self.api.runs.pop(1800)
                self.checkpoint = self.service_run(checkpoint_id, checkpoint_time)
                self.previous = self.state(self.checkpoint)
                self.api.add(self.checkpoint, self.previous)
                context = self.context()
                self.assertFalse(context.can_effect)
                self.assertFalse(context.can_prepare)
                self.assertFalse(context.frontier["authority"])
                assert context.previous is not None
                self.assertEqual(context.previous["run_id"], checkpoint_id)

    def test_same_second_with_strictly_prior_id_is_supported(self) -> None:
        self.checkpoint["created_at"] = self.current["created_at"]
        self.previous = self.state(self.checkpoint)
        self.api.add(self.checkpoint, self.previous)
        self.assertTrue(self.context().can_effect)

    def test_older_created_queued_actor_cannot_recreate_budget(self) -> None:
        self.current["created_at"] = (NOW - timedelta(hours=2)).isoformat()
        self.current["id"] = 1700
        self.api.runs.pop(2000)
        self.api.add(self.current)
        context = self.context()
        self.assertFalse(context.can_prepare)
        self.assertFalse(context.can_effect)
        self.assertFalse(context.frontier["authority"])

    def test_provably_unstarted_future_queued_actor_is_neither_authority_nor_ambiguity(
        self,
    ) -> None:
        queued = self.service_run(2100, NOW + timedelta(hours=1), status="queued", conclusion=None)
        self.api.add(queued)
        self.api.responses["actions/runs/2100/attempts/1/jobs?per_page=100"] = {
            "total_count": 0,
            "jobs": [],
        }
        context = self.context()
        self.assertTrue(context.can_effect)
        self.assertIsNone(context.frontier["fence"])
        assert context.checkpoint_run is not None
        self.assertEqual(context.checkpoint_run["id"], 1800)

    def test_active_possible_effect_defers_frontier_until_valid_terminal_closure(self) -> None:
        active = self.service_run(
            2100, NOW + timedelta(hours=1), status="in_progress", conclusion=None
        )
        self.api.add(active)
        self.api.responses["actions/runs/2100/attempts/1/jobs?per_page=100"] = {
            "total_count": 1,
            "jobs": [
                {"status": "in_progress", "conclusion": None, "started_at": active["created_at"]}
            ],
        }
        pending = self.context()
        self.assertFalse(pending.can_effect)
        self.assertFalse(pending.frontier["authority"])
        self.assertIsNone(pending.frontier["fence"])
        observed = self.persist(pending)
        active.update(status="completed", conclusion="success")
        self.api.add(active, self.state(active))
        self.next_run()
        closed = self.context(observed)
        self.assertTrue(closed.can_effect)
        assert closed.checkpoint_run is not None
        self.assertEqual(closed.checkpoint_run["id"], 2100)

    def test_newer_created_failed_or_cancelled_actor_is_not_hidden_by_current_time(self) -> None:
        for conclusion in ("failure", "cancelled"):
            with self.subTest(conclusion=conclusion):
                self.setUp()
                failed = self.service_run(2100, NOW + timedelta(hours=1), conclusion=conclusion)
                self.api.add(failed)
                self.api.responses["actions/runs/2100/attempts/1/jobs?per_page=100"] = {
                    "total_count": 1,
                    "jobs": [
                        {
                            "status": "completed",
                            "conclusion": conclusion,
                            "started_at": failed["created_at"],
                        }
                    ],
                }
                context = self.context()
                self.assertFalse(context.can_effect)
                self.assertIsNotNone(context.frontier["fence"])
                durable = self.persist(context)
                self.next_run()
                self.assertFalse(self.context(durable).can_effect)

    def test_successful_artifact_gap_and_all_artifacts_lost_cannot_reseed_cleanly(self) -> None:
        for genesis in (False, True):
            with self.subTest(genesis=genesis):
                self.setUp()
                self.api.responses.pop("actions/runs/1800/artifacts?per_page=100")
                context = self.context(genesis=genesis)
                self.assertFalse(context.can_effect)
                self.assertIsNotNone(context.frontier["fence"])
                durable = self.persist(context)
                self.next_run()
                later = self.context(durable)
                self.assertFalse(later.can_effect)
                self.assertIsNotNone(later.frontier["fence"])

    def test_current_service_attempt_two_and_future_history_of_it_remain_uncertain(self) -> None:
        self.current["run_attempt"] = 2
        context = self.context()
        self.assertFalse(context.can_effect)
        self.assertIn("prior attempts", context.frontier["fence"])
        durable = self.persist(context)
        self.next_run()
        self.assertFalse(self.context(durable).can_effect)

    def test_leftover_checkpoint_attempt_one_cannot_authorize_server_attempt_two(self) -> None:
        self.checkpoint["run_attempt"] = 2
        context = self.context()
        self.assertFalse(context.can_effect)
        self.assertIn("attempt", context.frontier["fence"])

    def test_latest_failed_attempt_cannot_hide_prior_successful_artifact_loss(self) -> None:
        prior = self.service_run(1900, NOW - timedelta(minutes=30), conclusion="failure", attempt=2)
        self.api.add(prior)
        context = self.context(genesis=True)
        self.assertFalse(context.can_effect)
        self.assertIn("attempt", context.frontier["fence"])

    def test_truncated_genesis_and_backdated_floor_are_durably_uncertain(self) -> None:
        self.api.runs.pop(1800)
        self.api.floor = datetime(2000, 1, 1, tzinfo=UTC)
        self.api.extra_total = 101
        context = self.context(genesis=True)
        self.assertFalse(context.can_effect)
        self.assertIn("incomplete", context.frontier["fence"])
        durable = self.persist(context)
        self.next_run()
        self.api.extra_total = 0
        self.assertFalse(self.context(durable).can_effect)

    def test_rollback_does_not_stop_at_another_authority_and_mint_genesis(self) -> None:
        self.api.responses.pop("actions/runs/1800/artifacts?per_page=100")
        other = self.service_run(1900, NOW - timedelta(minutes=30), implementation=P)
        self.api.add(other)
        context = self.context(genesis=True)
        self.assertFalse(context.can_effect)
        self.assertIsNotNone(context.frontier["fence"])

    def test_floor_mismatch_future_floor_and_pre_floor_checkpoint_never_authorize(self) -> None:
        self.api.canonical_sha = P
        self.assertFalse(self.context().can_effect)
        self.setUp()
        self.api.floor = NOW + timedelta(days=1)
        self.assertFalse(self.context(genesis=True).can_effect)
        self.setUp()
        self.checkpoint["created_at"] = (self.api.floor - timedelta(seconds=1)).isoformat()
        self.previous = self.state(self.checkpoint)
        self.api.add(self.checkpoint, self.previous)
        self.assertFalse(self.context().can_effect)

    def test_actor_created_before_floor_cannot_effect_even_when_it_executes_later(self) -> None:
        self.current["created_at"] = (self.api.floor - timedelta(seconds=1)).isoformat()
        context = self.context(genesis=True)
        self.assertFalse(context.can_prepare)
        self.assertFalse(context.can_effect)
        with self.assertRaises(ValueError):
            subject.require_effect(self.api, context, C, "main")

    def test_checkpoint_advances_preserve_normal_and_terminal_recovery_budgets(self) -> None:
        key = "1" * 64
        entry = {
            "phase": "attempted",
            "kind": "retained-graph",
            "original_run_id": 50,
            "original_attempt": 1,
            "source_head_sha": H,
            "authority": C,
            "prepared_at": self.checkpoint["created_at"],
            "prepared_service_run": 1800,
            "hinted_candidate_sha": OLD_MERGE,
            "hinted_base_sha": P,
        }
        self.previous["reruns"] = {"8" * 40: 50}
        self.previous["recoveries"] = {key: entry}
        self.api.add(self.checkpoint, self.previous)
        context = self.context()
        self.assertTrue(context.can_effect)
        durable = self.persist(context)
        self.next_run()
        advanced = self.context(durable)
        assert advanced.previous is not None
        self.assertEqual(advanced.previous["reruns"], {"8" * 40: 50})
        assert advanced.previous is not None
        self.assertEqual(advanced.previous["recoveries"][key]["phase"], "attempted")

    def test_effect_time_attempt_change_suspends_instead_of_using_earlier_artifact(self) -> None:
        context = self.context()
        self.assertTrue(context.can_effect)
        self.checkpoint["run_attempt"] = 2
        with self.assertRaises(ValueError):
            subject.require_effect(self.api, context, C, "main")
        self.assertFalse(context.can_effect)
        self.assertIsNotNone(context.frontier["fence"])

    def test_normal_equal_base_path_cannot_bypass_floor_or_newer_checkpoint(self) -> None:
        for before_floor in (True, False):
            with self.subTest(before_floor=before_floor):
                self.setUp()
                self.current["created_at"] = (
                    self.api.floor - timedelta(seconds=1)
                    if before_floor
                    else core.utc(self.checkpoint["created_at"]) - timedelta(seconds=1)
                ).isoformat()
                self.previous["reports"] = {N: passing(CANDIDATE)}
                self.api.add(self.checkpoint, self.previous)
                self.api.responses[""] = {"default_branch": "main"}
                self.api.responses["pulls?state=open&per_page=100&sort=updated&direction=desc"] = [
                    {"number": 17}
                ]
                self.api.responses[
                    "actions/workflows/ci.yml/runs?event=merge_group&per_page=20"
                ] = {
                    "workflow_runs": [],
                }
                self.api.responses[f"actions/workflows/ci.yml/runs?head_sha={H}&per_page=20"] = {
                    "workflow_runs": [],
                }
                with (
                    tempfile.TemporaryDirectory() as temporary,
                    patch.object(core, "previous_state", return_value=self.previous),
                    patch.object(core, "pr_candidate", return_value=CANDIDATE),
                    patch.object(core, "still_current", return_value=True),
                    patch.object(
                        core,
                        "evaluate",
                        side_effect=lambda api, candidate, state, now: {
                            **passing(candidate),
                            "state": state,
                        },
                    ),
                    patch.object(core, "rerun_ready") as normal,
                ):
                    payload = core.service(
                        self.api,
                        event_name="schedule",
                        event={},
                        implementation=C,
                        run_id=2000,
                        run_attempt=1,
                        output=Path(temporary) / "state.json",
                        now=self.now,
                    )
                normal.assert_not_called()
                self.assertFalse(payload["recovery_frontier"]["authority"])
                self.assertTrue(all(path.startswith("statuses/") for path, _ in self.api.posts))

    def test_normal_retry_transport_ambiguity_consumes_its_original_budget_before_post(
        self,
    ) -> None:
        fixture = RecoveryTests()
        fixture.setUp()
        api = fixture.api
        original = deepcopy(fixture.original)
        original["pull_requests"][0]["base"]["sha"] = B
        api.responses[f"actions/workflows/ci.yml/runs?head_sha={H}&per_page=20"] = {
            "workflow_runs": [original],
        }
        api.responses["actions/runs/50/jobs?per_page=100"] = {
            "total_count": len(fixture.jobs),
            "jobs": fixture.jobs,
        }
        for revision in ("main", N):
            api.files(revision, {".github/workflows/ci.yml": CALLER})
        ledger: dict[str, int] = {}
        api.fail_post = True
        with patch.object(core, "still_current", return_value=True):
            with self.assertRaises(ValueError):
                core.rerun_ready(api, CANDIDATE, ledger, "main", implementation=C)
            self.assertEqual(ledger, {N: 50})
            core.rerun_ready(api, CANDIDATE, ledger, "main", implementation=C)
        self.assertEqual(api.posts, [("actions/runs/50/rerun-failed-jobs", {})])


SKIPPED_FIXTURES = Path(__file__).parent / "fixtures/recovery/podman-skipped-20261106"


def skipped_fixture(name: str) -> dict[str, Any]:
    return json.loads((SKIPPED_FIXTURES / (name + ".json.txt")).read_text())


class RecordedHistoryAPI(CheckpointAPI):
    def __init__(self) -> None:
        super().__init__()
        self.repository = "Strukturpiloten/podman-lens"
        self.history_mutation = ""
        self.canonical_sha = "41d7f8b868351621c4e60fbeedd9fce5955f7897"
        self.floor = datetime(2026, 11, 6, 21, 40, 3, tzinfo=UTC)
        for run in skipped_fixture("history-cutoff")["workflow_runs"]:
            self.add(run)
        self.prior = self.runs[42305732516]
        self.current = self.runs[42306033057]
        for prefix, identifier in (("prior", 42305732516), ("later-skipped", 42306504865)):
            self.responses[f"actions/runs/{identifier}/attempts/1/jobs?per_page=100"] = (
                skipped_fixture(prefix + "-attempt1-jobs")
            )
            self.responses[f"actions/runs/{identifier}/artifacts?per_page=100"] = skipped_fixture(
                prefix + "-artifacts"
            )
        self.responses["actions/workflows/dependency-eligibility.yml"] = skipped_fixture(
            "workflow-metadata"
        )
        head = self.current["head_sha"]
        self.responses[f"compare/{head}...main"] = {"status": "identical"}
        self.responses[f"contents/{core.WORKFLOW_PATH}?ref={head}"] = skipped_fixture(
            "caller-contents"
        )

    def request(self, method: str, path: str, data: dict[str, Any] | None = None) -> object:
        if (
            method == "GET"
            and path == f"/repos/Strukturpiloten/boxferry/git/commits/{self.canonical_sha}"
        ):
            return {"sha": self.canonical_sha, "committer": {"date": self.floor.isoformat()}}
        return super().request(method, path, data)

    def get(self, suffix: str) -> dict[str, Any]:
        if suffix.startswith("actions/workflows/dependency-eligibility.yml/runs?"):
            from urllib.parse import parse_qs

            self.queries.append(suffix)
            lower = core.utc(parse_qs(suffix.split("?", 1)[1])["created"][0][2:])
            matching = []
            for run in self.runs.values():
                try:
                    include = core.utc(run.get("created_at")) >= lower
                except ValueError:
                    include = True  # Return malformed server data to the real collector.
                if include:
                    matching.append(deepcopy(run))
            listing = {"total_count": len(matching), "workflow_runs": list(reversed(matching))}
            if self.history_mutation == "float-count":
                listing["total_count"] = float(len(matching))
            elif self.history_mutation == "bool-count":
                listing["total_count"] = True
            elif self.history_mutation == "truncated":
                listing["total_count"] += 1
            elif self.history_mutation == "duplicate":
                listing["workflow_runs"][-1] = deepcopy(listing["workflow_runs"][0])
            elif self.history_mutation == "omitted-current":
                listing["workflow_runs"] = [
                    run for run in matching if run["id"] != self.current["id"]
                ]
                listing["total_count"] -= 1
            return listing
        return super().get(suffix)


class RecordedSkippedServiceTests(unittest.TestCase):
    def setUp(self) -> None:
        self.api = RecordedHistoryAPI()
        self.source = self.api.canonical_sha
        self.now = datetime(2026, 11, 6, 23, tzinfo=UTC)

    def context(self, previous: dict[str, Any] | None = None) -> subject.ServiceContext:
        return subject.service_context(
            self.api, self.api.current, previous, self.source, "main", self.now
        )

    def persist(self, context: subject.ServiceContext) -> dict[str, Any]:
        run = self.api.current
        state = {
            "schema": 1,
            "repository": self.api.repository,
            "workflow_path": core.WORKFLOW_PATH,
            "implementation_revision": self.source,
            "run_id": run["id"],
            "run_attempt": 1,
            "observed_at": self.now.isoformat(),
            "dependency_state": {"schema_version": 1, "first_seen": {}},
            "reports": {},
            "reruns": {"8" * 40: 50},
            "recoveries": {},
            "recovery_frontier": deepcopy(context.frontier),
        }
        self.api.add({**run, "status": "completed", "conclusion": "success"}, state)
        return state

    def test_full_raw_fixture_integrity_and_pagination_provenance(self) -> None:
        import hashlib

        index = json.loads((SKIPPED_FIXTURES / "index.json").read_text())
        for name, record in index["files"].items():
            data = (SKIPPED_FIXTURES / name).read_bytes()
            self.assertEqual(hashlib.sha256(data).hexdigest(), record["sha256"])
            self.assertFalse(record["projection"])
            if "acquisition" in record:
                self.assertTrue(record["acquisition"]["all_pages"])
                self.assertEqual(record["acquisition"]["headers"]["link"], [])
        history = skipped_fixture("history-cutoff")
        self.assertEqual(history["total_count"], 10)
        self.assertEqual(len(history["workflow_runs"]), 10)
        self.assertEqual(
            skipped_fixture("prior-attempt1-jobs")["jobs"][0]["runner_group_name"], None
        )

    def test_recorded_guarded_skip_seeds_only_then_real_checkpoint_data_advances(self) -> None:
        self.assertTrue(subject.skipped_service(self.api, self.api.prior, self.source, "main"))
        context = self.context()
        self.assertTrue(context.can_prepare)
        self.assertFalse(context.can_effect)
        self.assertIsNone(context.checkpoint_run)
        self.assertIsNone(context.frontier["fence"])
        self.assertEqual(context.frontier["pending"], [])
        self.assertEqual(len(self.api.queries), 1)  # Real unordered history_runs collector.
        state = self.persist(context)
        # Synthetic next server actor; the actual completed first actor's state/artifact
        # follows the real checkpoint_payload and validate_state paths, never a mocked proof.
        later = deepcopy(self.api.current)
        later.update(
            id=42306033058, created_at="2026-11-06T22:39:00Z", status="in_progress", conclusion=None
        )
        self.api.current = later
        self.api.add(later)
        advanced = self.context(state)
        self.assertTrue(advanced.can_effect)
        assert advanced.previous is not None
        self.assertEqual(advanced.previous["reruns"], {"8" * 40: 50})
        subject.require_effect(self.api, advanced, self.source, "main")
        self.assertEqual(self.api.posts, [])

    def test_actual_nonmonotonic_placeholder_timestamps_are_not_dispatch_proof(self) -> None:
        run = skipped_fixture("later-skipped-run")
        job = skipped_fixture("later-skipped-attempt1-jobs")["jobs"][0]
        self.assertGreater(core.utc(job["started_at"]), core.utc(job["completed_at"]))
        self.assertTrue(subject.skipped_service(self.api, run, self.source, "main"))
        self.assertEqual(self.api.posts, [])

    def test_each_observed_missing_or_wrong_typed_field_retains_uncertainty(self) -> None:
        run_fields = (
            "id",
            "run_attempt",
            "status",
            "conclusion",
            "repository",
            "event",
            "path",
            "head_sha",
            "head_branch",
            "name",
            "workflow_id",
            "referenced_workflows",
            "created_at",
            "updated_at",
            "run_started_at",
        )
        job_fields = (
            "id",
            "run_id",
            "run_attempt",
            "name",
            "workflow_name",
            "head_sha",
            "head_branch",
            "status",
            "conclusion",
            "steps",
            "labels",
            "runner_id",
            "runner_name",
            "runner_group_id",
            "runner_group_name",
            "started_at",
            "completed_at",
        )
        for target, fields in (("run", run_fields), ("job", job_fields)):
            for field in fields:
                for value in ("MISSING", False, True, 1.0, {}, ["unexpected"]):
                    with self.subTest(target=target, field=field, value=value):
                        self.setUp()
                        mapping = (
                            self.api.prior
                            if target == "run"
                            else self.api.responses[
                                "actions/runs/42305732516/attempts/1/jobs?per_page=100"
                            ]["jobs"][0]
                        )
                        if value == "MISSING":
                            mapping.pop(field)
                        else:
                            mapping[field] = value
                        context = self.context()
                        self.assertFalse(context.can_prepare)
                        self.assertFalse(context.can_effect)
                        self.assertTrue(context.frontier["fence"] or context.frontier["pending"])
                        self.assertEqual(self.api.posts, [])

    def test_lists_artifacts_caller_and_exact_source_ancestry_fail_closed(self) -> None:
        for mutation in (
            "count-bool",
            "count-float",
            "extra-job",
            "no-jobs",
            "artifact",
            "artifact-count-bool",
            "artifact-lookup-missing",
            "duplicate-ref",
            "foreign-ref",
            "wrong-source",
            "caller",
            "workflow-id",
            "ancestry",
            "unknown-runner",
            "run-attempt2",
            "source-sha-mismatch",
        ):
            with self.subTest(mutation=mutation):
                self.setUp()
                listing = self.api.responses[
                    "actions/runs/42305732516/attempts/1/jobs?per_page=100"
                ]
                artifacts = self.api.responses["actions/runs/42305732516/artifacts?per_page=100"]
                if mutation == "count-bool":
                    listing["total_count"] = True
                elif mutation == "count-float":
                    listing["total_count"] = 1.0
                elif mutation == "extra-job":
                    listing["jobs"].append(deepcopy(listing["jobs"][0]))
                    listing["total_count"] = 2
                elif mutation == "no-jobs":
                    listing.update(total_count=0, jobs=[])
                elif mutation == "artifact":
                    artifacts.update(total_count=1, artifacts=[{"id": 1}])
                elif mutation == "artifact-count-bool":
                    artifacts["total_count"] = False
                elif mutation == "artifact-lookup-missing":
                    del self.api.responses["actions/runs/42305732516/artifacts?per_page=100"]
                elif mutation == "duplicate-ref":
                    self.api.prior["referenced_workflows"] *= 2
                elif mutation == "foreign-ref":
                    self.api.prior["referenced_workflows"].append({"path": "foreign", "sha": C})
                elif mutation == "wrong-source":
                    # Keep same-C classification while contradicting its exact reference.
                    self.api.prior["referenced_workflows"][0]["extra"] = True
                elif mutation == "caller":
                    self.api.files(
                        self.api.prior["head_sha"], {core.WORKFLOW_PATH: b"---\njobs: {}\n"}
                    )
                elif mutation == "workflow-id":
                    self.api.prior["workflow_id"] += 1
                elif mutation == "ancestry":
                    self.api.responses[f"compare/{self.api.prior['head_sha']}...main"] = {
                        "status": "diverged"
                    }
                elif mutation == "unknown-runner":
                    listing["jobs"][0]["runner_new_identity"] = "runner"
                elif mutation == "run-attempt2":
                    self.api.prior["run_attempt"] = 2
                elif mutation == "source-sha-mismatch":
                    self.api.prior["referenced_workflows"][0]["sha"] = C
                context = self.context()
                self.assertFalse(context.can_prepare)
                self.assertIsNotNone(context.frontier["fence"])
                self.assertEqual(self.api.posts, [])

    def test_real_history_collector_refuses_incomplete_duplicate_and_aliased_counts(self) -> None:
        for mutation in ("float-count", "bool-count", "truncated", "duplicate", "omitted-current"):
            with self.subTest(mutation=mutation):
                self.setUp()
                self.api.history_mutation = mutation
                context = self.context()
                self.assertFalse(context.can_prepare)
                self.assertFalse(context.can_effect)
                self.assertIsNotNone(context.frontier["fence"])
                self.assertEqual(self.api.posts, [])

    def test_separate_genuinely_queued_job_schema_and_numeric_aliases(self) -> None:
        self.api.prior.update(status="queued", conclusion=None)
        listing = self.api.responses["actions/runs/42305732516/attempts/1/jobs?per_page=100"]
        job = listing["jobs"][0]
        job.update(status="queued", conclusion=None, started_at=None)
        self.assertTrue(subject.unstarted_service(self.api, self.api.prior))
        for field in ("id", "run_id", "run_attempt"):
            original = job[field]
            job[field] = float(original)
            self.assertFalse(subject.unstarted_service(self.api, self.api.prior))
            job[field] = original
        listing["total_count"] = True
        self.assertFalse(subject.unstarted_service(self.api, self.api.prior))
        self.assertEqual(self.api.posts, [])

    def test_disjoint_no_start_branch_cannot_accept_terminal_or_active_empty_jobs(self) -> None:
        for status, conclusion in (
            ("completed", "failure"),
            ("completed", "cancelled"),
            ("completed", "skipped"),
            ("completed", "success"),
            ("in_progress", None),
            ("queued", "failure"),
        ):
            self.setUp()
            self.api.prior.update(status=status, conclusion=conclusion)
            self.api.responses["actions/runs/42305732516/attempts/1/jobs?per_page=100"] = {
                "total_count": 0,
                "jobs": [],
            }
            self.assertFalse(subject.unstarted_service(self.api, self.api.prior))
            context = self.context()
            self.assertFalse(context.can_prepare)
            self.assertTrue(context.frontier["fence"] or context.frontier["pending"])
            self.assertEqual(self.api.posts, [])

    def test_inherited_fence_is_not_cleared_by_no_dispatch_evidence(self) -> None:
        state = self.persist(self.context())
        state["recovery_frontier"]["fence"] = "prior uncertainty"
        self.api.add(self.api.current, state)
        self.api.current = {
            **self.api.current,
            "id": 42306033058,
            "created_at": "2026-11-06T22:39:00Z",
        }
        self.api.add(self.api.current)
        context = self.context(state)
        self.assertEqual(context.frontier["fence"], "prior uncertainty")
        self.assertFalse(context.can_prepare)
        self.assertFalse(context.can_effect)
        self.assertEqual(self.api.posts, [])
