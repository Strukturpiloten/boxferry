"""Adversarial metadata and branch-race examples for automatic distribution."""

from __future__ import annotations

import tempfile
import unittest
from copy import deepcopy
from pathlib import Path
from typing import Any
from unittest.mock import patch

from boxferry_ci import distribution as subject

BASE, CANDIDATE, MAIN, IMPLEMENTATION, FOREIGN = (letter * 40 for letter in "abcde")
REPO = {"full_name": subject.REPOSITORY}


def ref(branch: str, sha: str) -> dict[str, Any]:
    return {"ref": f"refs/heads/{branch}", "object": {"type": "commit", "sha": sha}}


class FakeAPI(subject.API):
    def __init__(self) -> None:
        super().__init__("never-sent")
        self.source: dict[str, Any] = {
            "id": 12,
            "run_attempt": 2,
            "workflow_id": 44,
            "name": "CI",
            "path": subject.CI_WORKFLOW,
            "repository": REPO,
            "head_repository": REPO,
            "head_branch": "main",
            "head_sha": CANDIDATE,
            "event": "push",
            "status": "completed",
            "conclusion": "success",
            "pull_requests": [],
            "actor": {"login": "legitimate-user"},
        }
        self.service: dict[str, Any] = {
            "id": 99,
            "run_attempt": 1,
            "event": "workflow_run",
            "path": subject.WORKFLOW,
            "head_branch": "main",
            "head_sha": IMPLEMENTATION,
            "repository": REPO,
            "head_repository": REPO,
        }
        self.jobs = [
            {
                "name": name,
                "run_id": 12,
                "run_attempt": 2,
                "head_sha": CANDIDATE,
                "status": "completed",
                "conclusion": "success",
            }
            for name in ("Rust quality", "Shared automation package", "Required CI")
        ]
        self.default_branch = "main"
        self.branch = BASE
        self.main = MAIN
        self.main_reads = 0
        self.repository_reads = 0
        self.change_main: str | None = None
        self.rename_default = False
        self.files: list[Any] = [{"filename": "automation/src/boxferry_ci/planner.py"}]
        self.statuses: dict[tuple[str, str], str] = {}
        self.reads: list[str] = []
        self.writes: list[tuple[str, str, dict[str, Any] | None]] = []
        self.branch_reads = 0
        self.change_before_write: str | None = None
        self.patch_race: str | None = None
        self.bad_readback: str | None = None
        self.source_reads = 0
        self.supersede = False
        self.missing_branch = False
        self.total_count: int | None = None

    def get(self, suffix: str) -> dict[str, Any]:
        self.reads.append(suffix)
        if suffix == "":
            self.repository_reads += 1
            branch = (
                "renamed"
                if self.rename_default and self.repository_reads > 1
                else self.default_branch
            )
            return {"default_branch": branch}
        if suffix == "actions/runs/99":
            return deepcopy(self.service)
        if suffix == "actions/runs/12":
            self.source_reads += 1
            result = deepcopy(self.source)
            if self.supersede and self.source_reads > 1:
                result["run_attempt"] += 1
            return result
        if suffix == "actions/workflows/ci.yml":
            return {"id": 44, "path": subject.CI_WORKFLOW, "state": "active"}
        if suffix == "actions/runs/12/attempts/2/jobs?per_page=100":
            return {
                "total_count": len(self.jobs) if self.total_count is None else self.total_count,
                "jobs": deepcopy(self.jobs),
            }
        if suffix == "git/ref/heads/main":
            self.main_reads += 1
            if self.change_main and self.main_reads > 1:
                self.main = self.change_main
            return ref("main", self.main)
        if suffix == "git/ref/heads/automation-v1":
            if self.missing_branch:
                raise subject.MissingResource("absent")
            self.branch_reads += 1
            if self.change_before_write and self.branch_reads > 1:
                self.branch = self.change_before_write
            return ref(subject.BRANCH, self.branch)
        if suffix.startswith("compare/"):
            base, head = suffix[len("compare/") :].split("?")[0].split("...")
            chain = [BASE, IMPLEMENTATION, CANDIDATE, MAIN]
            status = self.statuses.get((base, head))
            if status is None:
                status = (
                    "diverged"
                    if base not in chain or head not in chain
                    else (
                        "ahead"
                        if chain.index(base) < chain.index(head)
                        else ("identical" if base == head else "behind")
                    )
                )
            return {"status": status, "files": deepcopy(self.files)}
        raise AssertionError(f"unexpected API read {suffix}")

    def request(self, method: str, path: str, data: dict[str, Any] | None = None) -> object:
        self.writes.append((method, path, deepcopy(data)))
        if self.patch_race:
            self.branch = self.patch_race
            raise subject.EligibilityError("GitHub metadata request failed with HTTP 422")
        self.branch = self.bad_readback or CANDIDATE
        return ref(subject.BRANCH, self.branch)


def event(api: FakeAPI) -> dict[str, Any]:
    return {"action": "completed", "repository": REPO, "workflow_run": deepcopy(api.source)}


def run(
    api: FakeAPI, payload: dict[str, Any] | None = None, *, event_name: str = "workflow_run"
) -> subject.Result:
    return subject.distribute(
        api,
        event_name=event_name,
        event=event(api) if payload is None else payload,
        implementation=IMPLEMENTATION,
        run_id=99,
        run_attempt=1,
    )


class DistributionTests(unittest.TestCase):
    def test_exact_successful_mixed_candidate_advances_only_nonforced_branch(self) -> None:
        api = FakeAPI()
        api.files.append({"filename": "crates/boxferry/src/lib.rs"})
        result = run(api)
        self.assertEqual(result.status, "advanced")
        self.assertEqual(result.candidate, CANDIDATE)
        self.assertEqual(
            api.writes,
            [
                (
                    "PATCH",
                    f"/repos/{subject.REPOSITORY}/{subject.REF}",
                    {"sha": CANDIDATE, "force": False},
                )
            ],
        )
        self.assertEqual(api.branch, CANDIDATE)
        self.assertEqual(api.source_reads, 2)
        self.assertGreaterEqual(api.branch_reads, 3)
        self.assertFalse(any("contents/" in path or "artifacts" in path for path in api.reads))

    def test_foreign_repository_and_unsupported_events_never_write(self) -> None:
        for name in ("push", "pull_request", "merge_group", "workflow_dispatch", "schedule"):
            api = FakeAPI()
            with self.subTest(event=name), self.assertRaises(subject.DistributionError):
                run(api, event_name=name)
            self.assertFalse(api.writes)
        for key, value in (("action", "requested"), ("repository", {"full_name": "evil/fork"})):
            api = FakeAPI()
            payload = event(api)
            payload[key] = value
            with self.subTest(key=key), self.assertRaises(subject.DistributionError):
                run(api, payload)
            self.assertFalse(api.writes)
        api = FakeAPI()
        api.repository = "Strukturpiloten/compose-lens"
        with self.assertRaises(subject.DistributionError):
            run(api)
        self.assertFalse(api.reads)

    def test_failed_foreign_manual_pr_queue_or_stale_source_is_rejected(self) -> None:
        mutations = {
            "repository": {"full_name": "evil/fork"},
            "head_repository": {"full_name": "evil/fork"},
            "event": "pull_request",
            "head_branch": "feature",
            "path": "evil.yml",
            "workflow_id": 123,
            "name": "Fake CI",
            "status": "in_progress",
            "conclusion": "failure",
            "pull_requests": [{"number": 1}],
        }
        for key, value in mutations.items():
            api = FakeAPI()
            api.source[key] = value
            with self.subTest(key=key), self.assertRaises(subject.DistributionError):
                run(api)
            self.assertFalse(api.writes)
        for value in ("merge_group", "workflow_dispatch", "push"):
            api = FakeAPI()
            payload = event(api)
            api.source["event"] = value
            api.source["conclusion"] = "cancelled"
            with self.subTest(cancelled=value), self.assertRaises(subject.DistributionError):
                run(api, payload)
        for key, value in (("run_attempt", 1), ("head_sha", BASE)):
            api = FakeAPI()
            payload = event(api)
            payload["workflow_run"][key] = value
            with self.subTest(stale=key), self.assertRaises(subject.DistributionError):
                run(api, payload)
            self.assertFalse(api.writes)

    def test_trusted_service_and_default_branch_identity_are_required(self) -> None:
        for key, value in (
            ("head_sha", BASE),
            ("path", "evil.yml"),
            ("event", "push"),
            ("head_repository", {"full_name": "evil/fork"}),
            ("head_branch", "feature"),
            ("run_attempt", 2),
        ):
            api = FakeAPI()
            api.service[key] = value
            with self.subTest(key=key), self.assertRaises(subject.DistributionError):
                run(api)
            self.assertFalse(api.writes)
        api = FakeAPI()
        api.default_branch = "renamed-main"
        with self.assertRaisesRegex(subject.DistributionError, "default branch changed"):
            run(api)

    def test_current_attempt_full_jobs_and_unique_required_ci_must_succeed(self) -> None:
        for key, value in (
            ("run_id", 1),
            ("run_attempt", 1),
            ("head_sha", BASE),
            ("status", "queued"),
            ("conclusion", "failure"),
            ("conclusion", "skipped"),
            ("conclusion", "cancelled"),
        ):
            api = FakeAPI()
            api.jobs[0][key] = value
            with self.subTest(key=key, value=value), self.assertRaises(subject.DistributionError):
                run(api)
            self.assertFalse(api.writes)
        for count in (0, 2):
            api = FakeAPI()
            api.jobs = api.jobs[:-1] + [deepcopy(api.jobs[-1]) for _ in range(count)]
            with self.subTest(required=count), self.assertRaises(subject.DistributionError):
                run(api)
        for jobs, total in (([], 0), (FakeAPI().jobs, 101), (FakeAPI().jobs * 34, 102)):
            api = FakeAPI()
            api.jobs, api.total_count = jobs, total
            with self.subTest(total=total), self.assertRaises(subject.DistributionError):
                run(api)

    def test_cargo_metadata_only_main_change_advances_exact_nonforced_branch(self) -> None:
        api = FakeAPI()
        api.files = [{"filename": ".devcontainer/Dockerfile"}]
        result = run(api)
        self.assertEqual(result.status, "advanced")
        self.assertEqual(result.candidate, CANDIDATE)
        self.assertEqual(
            api.writes,
            [
                (
                    "PATCH",
                    f"/repos/{subject.REPOSITORY}/{subject.REF}",
                    {"sha": CANDIDATE, "force": False},
                )
            ],
        )
        self.assertEqual(api.branch, CANDIDATE)

    def test_unrelated_devcontainer_paths_never_advance_distribution(self) -> None:
        for name in (
            ".devcontainer/devcontainer.json",
            ".devcontainer/verify-tools.sh",
            ".devcontainer/Dockerfile.bak",
            ".devcontainer/nested/Dockerfile",
            ".devcontainer/Dockerfile/child",
        ):
            api = FakeAPI()
            api.files = [{"filename": name}]
            with self.subTest(path=name):
                self.assertFalse(subject.shared_path(name))
                self.assertEqual(run(api).status, "unchanged")
                self.assertFalse(api.writes)

    def test_unrelated_and_consumer_adoption_only_changes_do_not_distribute(self) -> None:
        api = FakeAPI()
        api.files = [
            {"filename": name}
            for name in (
                "crates/boxferry/src/lib.rs",
                ".github/automation.json",
                ".github/workflows/ci.yml",
                ".github/renovate.json",
                "docs/automation.md",
            )
        ]
        self.assertEqual(run(api).status, "unchanged")
        self.assertFalse(api.writes)
        self.assertEqual(api.branch, BASE)

    def test_all_producer_roots_and_rename_away_are_relevant(self) -> None:
        for path in (
            "automation/uv.lock",
            "renovate-presets/default.json",
            "templates/caller.yml",
            ".github/workflows/shared-ci.yml",
            ".github/actions/shared-python/action.yml",
            subject.WORKFLOW,
        ):
            api = FakeAPI()
            api.files = [{"filename": path}]
            with self.subTest(path=path):
                self.assertEqual(run(api).status, "advanced")
        api = FakeAPI()
        api.files = [{"filename": "archive/old.py", "previous_filename": "automation/old.py"}]
        self.assertEqual(run(api).status, "advanced")

    def test_file_evidence_cap_and_malformed_paths_fail_closed(self) -> None:
        for files in (
            [{}],
            [{"filename": 3}],
            [{"filename": "archive", "previous_filename": 3}],
            [{"filename": "product.txt"}] * 300,
        ):
            api = FakeAPI()
            api.files = files
            with self.subTest(files=len(files)), self.assertRaises(subject.DistributionError):
                run(api)
            self.assertFalse(api.writes)

    def test_missing_initial_branch_requires_explicit_verified_bootstrap(self) -> None:
        api = FakeAPI()
        api.missing_branch = True
        with self.assertRaisesRegex(subject.DistributionError, "maintainer must bootstrap"):
            run(api)
        self.assertFalse(api.writes)

    def test_outside_main_or_divergent_source_service_and_branch_are_rejected(self) -> None:
        for base, head in (
            (IMPLEMENTATION, MAIN),
            (CANDIDATE, MAIN),
            (BASE, MAIN),
            (BASE, CANDIDATE),
        ):
            api = FakeAPI()
            api.statuses[base, head] = "diverged"
            with self.subTest(base=base), self.assertRaises(subject.DistributionError):
                run(api)
            self.assertFalse(api.writes)

    def test_equal_or_newer_main_branch_never_regresses(self) -> None:
        for branch in (CANDIDATE, MAIN):
            api = FakeAPI()
            api.branch = branch
            with self.subTest(branch=branch):
                self.assertEqual(run(api).status, "unchanged")
            self.assertFalse(api.writes)

    def test_main_history_and_default_branch_are_rechecked_before_write(self) -> None:
        api = FakeAPI()
        api.change_main = BASE
        with self.assertRaisesRegex(subject.DistributionError, "left canonical main"):
            run(api)
        self.assertFalse(api.writes)
        api = FakeAPI()
        api.rename_default = True
        with self.assertRaisesRegex(subject.DistributionError, "default branch changed before"):
            run(api)
        self.assertFalse(api.writes)

    def test_legitimate_main_push_actor_does_not_need_a_login_allowlist(self) -> None:
        api = FakeAPI()
        api.source["actor"] = {"login": "renovate[bot]"}
        self.assertEqual(run(api).status, "advanced")

    def test_source_retry_during_evaluation_never_writes(self) -> None:
        api = FakeAPI()
        api.supersede = True
        with self.assertRaisesRegex(subject.DistributionError, "superseded"):
            run(api)
        self.assertFalse(api.writes)

    def test_branch_reread_retains_newer_descendant_but_rejects_foreign_change(self) -> None:
        for changed in (CANDIDATE, MAIN):
            api = FakeAPI()
            api.change_before_write = changed
            with self.subTest(changed=changed):
                self.assertEqual(run(api).status, "unchanged")
            self.assertFalse(api.writes)
        api = FakeAPI()
        api.change_before_write = FOREIGN
        with self.assertRaises(subject.DistributionError):
            run(api)
        self.assertFalse(api.writes)

    def test_nonfastforward_patch_race_accepts_only_verified_newer_main_ref(self) -> None:
        for changed in (CANDIDATE, MAIN):
            api = FakeAPI()
            api.patch_race = changed
            with self.subTest(changed=changed):
                self.assertEqual(run(api).status, "unchanged")
            self.assertEqual(len(api.writes), 1)
        for changed in (BASE, FOREIGN):
            api = FakeAPI()
            api.patch_race = changed
            with self.subTest(changed=changed), self.assertRaises(subject.EligibilityError):
                run(api)
            self.assertEqual(len(api.writes), 1)

    def test_readback_outside_main_or_without_candidate_fails(self) -> None:
        for changed in (BASE, FOREIGN):
            api = FakeAPI()
            api.bad_readback = changed
            with self.subTest(changed=changed), self.assertRaises(subject.DistributionError):
                run(api)


class APIBoundaryTests(unittest.TestCase):
    def test_client_rejects_arbitrary_repo_mutation_force_and_budget(self) -> None:
        api = subject.API("never-sent")
        for method, path, body in (
            ("GET", "/repos/foreign/repo", None),
            ("POST", f"/repos/{subject.REPOSITORY}/git/refs", {"sha": CANDIDATE}),
            (
                "PATCH",
                f"/repos/{subject.REPOSITORY}/{subject.REF}",
                {"sha": CANDIDATE, "force": True},
            ),
            (
                "PATCH",
                f"/repos/{subject.REPOSITORY}/git/refs/heads/main",
                {"sha": CANDIDATE, "force": False},
            ),
        ):
            with self.subTest(path=path), self.assertRaises(subject.DistributionError):
                api.request(method, path, body)
        api.requests = subject.MAX_REQUESTS
        with self.assertRaisesRegex(subject.DistributionError, "budget"):
            api.get("")

    def test_missing_token_and_oversized_event_have_actionable_failures(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            event_path = Path(temporary) / "event.json"
            event_path.write_text("{}", encoding="utf-8")
            arguments = [
                "--event-name",
                "workflow_run",
                "--event-path",
                str(event_path),
                "--implementation-revision",
                IMPLEMENTATION,
                "--run-id",
                "99",
                "--run-attempt",
                "1",
            ]
            with patch.dict("os.environ", {}, clear=True):
                self.assertEqual(subject.main(arguments), 2)
            event_path.write_bytes(b" " * (subject.MAX_EVENT_BYTES + 1))
            with patch.dict("os.environ", {"GH_TOKEN": "never-sent"}, clear=True):
                self.assertEqual(subject.main(arguments), 2)

    def test_workflow_has_only_required_permissions_and_trusted_checkout(self) -> None:
        root = Path(__file__).resolve().parents[2]
        workflow = (root / subject.WORKFLOW).read_text(encoding="utf-8")
        self.assertTrue(workflow.startswith("---\n"))
        for expected in (
            "permissions: {}",
            "contents: write",
            "actions: read",
            "cancel-in-progress: false",
            "workflows: [CI]",
            "types: [completed]",
            "branches: [main]",
            "ref: ${{ github.sha }}",
            "persist-credentials: false",
            "uses: ./_automation/.github/actions/shared-python",
            "timeout-minutes: 5",
            "github.event.workflow_run.event == 'push'",
        ):
            self.assertIn(expected, workflow)
        for forbidden in (
            "head_sha }}",
            "statuses: write",
            "pull-requests: write",
            "id-token:",
            "workflow_dispatch:",
            "push:",
            "git push",
            "force:",
            "upload-artifact",
        ):
            self.assertNotIn(forbidden, workflow)
        self.assertEqual(workflow.count("uses: actions/checkout@"), 1)


if __name__ == "__main__":
    unittest.main()
