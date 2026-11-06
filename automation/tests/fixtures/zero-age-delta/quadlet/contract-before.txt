#!/usr/bin/env python3
"""QuadletLens consumer contracts; shared tests own classifier implementation."""

from __future__ import annotations

import copy
import importlib.util
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import textwrap
import unittest
from pathlib import Path
from unittest.mock import patch

from boxferry_ci.contracts import ContractError
from boxferry_ci.gate import verify_gate
from boxferry_ci.planner import Comparison, make_plan, path_profiles
from boxferry_ci.policy import Policy, load_policy

ROOT = Path(__file__).resolve().parent.parent
POLICY = load_policy(ROOT / "scripts/validation-policy.json")


def require_release_ci_read_permissions(release: str, ci: str) -> None:
    """The outer reusable caller must supply the nested reader's exact read-only token."""
    required = {"actions": "read", "contents": "read", "statuses": "read"}
    for workflow, name, callee in (
        (release, "deterministic", "uses: ./.github/workflows/ci.yml"),
        (ci, "lockfile-release-age", "shared-dependency-wait.yml@"),
    ):
        job = re.search(rf"(?ms)^  {re.escape(name)}:\n.*?(?=^  [\w-]+:\n|\Z)", workflow)
        if job is None or callee not in job[0]:
            raise ValueError(f"missing expected reusable call in {name}")
        permissions = re.search(r"(?m)^    permissions:\n((?:      [^\n]+\n)+)", job[0])
        if permissions is None:
            raise ValueError(f"missing explicit permissions in {name}")
        scopes: dict[str, str] = {}
        for line in permissions[1].splitlines():
            scope = re.fullmatch(r"      ([a-z-]+): (read|write|none)", line)
            if scope is None or scope[1] in scopes:
                raise ValueError(f"malformed or duplicate permission in {name}")
            scopes[scope[1]] = scope[2]
        if scopes != required:
            raise ValueError(f"{name} must grant only actions, contents and statuses read")


ORIGINAL_JOBS = (
    "rust",
    "coverage",
    "msrv",
    "dependencies",
    "api",
    "documentation",
    "lockfile-release-age",
)


def ordinary_gate_fixture(
    policy: Policy, event: str, profile: str
) -> tuple[dict[str, object], dict[str, dict[str, object]], Comparison]:
    """Synthetic outcomes for an independently specified ordinary policy/profile."""
    candidate = "c" * 40
    comparison = Comparison(
        event,
        "a" * 40 if event in {"pull_request", "merge_group", "local"} else candidate,
        "b" * 40 if event in {"pull_request", "merge_group", "local"} else candidate,
        candidate,
    )
    selected = policy.selected_jobs((profile,))
    plan: dict[str, object] = {
        "schema": 3,
        "repository": policy.repository,
        "event": event,
        "base_sha": comparison.base_sha,
        "head_sha": comparison.head_sha,
        "tested_sha": candidate,
        "profile": profile,
        "profiles": [profile],
        "jobs": {job: job in selected for job in policy.jobs},
        "reasons": ["independently specified consumer contract"],
        "local_fingerprint": None,
    }
    needs: dict[str, dict[str, object]] = {
        job: {"result": "success" if job in selected else "skipped"} for job in policy.jobs
    }
    needs["validation-plan"] = {"result": "success"}
    return plan, needs, comparison


class OrdinaryValidationContractTests(unittest.TestCase):
    def test_adoption_preserves_original_jobs_profiles_and_local_commands(self) -> None:
        self.assertEqual(POLICY.jobs, ORIGINAL_JOBS)
        self.assertEqual(
            POLICY.selected_jobs(("prose",)), {"documentation", "lockfile-release-age"}
        )
        for profile in ("executable-docs", "full"):
            self.assertEqual(POLICY.selected_jobs((profile,)), set(ORIGINAL_JOBS))
            self.assertEqual(
                POLICY.local_commands[profile], (("bash", "scripts/check-all.sh", "--check"),)
            )
        for path in (
            ".devcontainer/Dockerfile",
            ".devcontainer/devcontainer.json",
            ".devcontainer/devcontainer-lock.json",
            ".dockerignore",
            ".containerignore",
            "scripts/shared-ci.py",
            "scripts/install-file-tools.sh",
            ".devcontainer/verify-tools.sh",
            ".github/automation.json",
            "package-lock.json",
            "rust-toolchain.toml",
            "scripts/validation-policy.json",
            "new-input-shape/unknown",
        ):
            with self.subTest(path=path):
                self.assertEqual(path_profiles(path, (b"changed",), POLICY, set()), {"full"})

    def test_generic_gate_rejects_missing_failed_cancelled_skipped_and_stale_results(self) -> None:
        for event in ("pull_request", "merge_group"):
            for profile in ("prose", "executable-docs", "full"):
                plan, needs, comparison = ordinary_gate_fixture(POLICY, event, profile)
                verify_gate(plan, needs, POLICY, comparison, expected_plan=plan)
                for job in ("validation-plan", *ORIGINAL_JOBS):
                    missing = copy.deepcopy(needs)
                    missing.pop(job)
                    with self.subTest(event=event, profile=profile, job=job, missing=True):
                        with self.assertRaises(ContractError):
                            verify_gate(plan, missing, POLICY, comparison, expected_plan=plan)
                    outcomes = ["failure", "cancelled", None]
                    if needs[job]["result"] == "success":
                        outcomes.append("skipped")
                    for result in outcomes:
                        broken = copy.deepcopy(needs)
                        broken[job]["result"] = result
                        with self.subTest(event=event, profile=profile, job=job, result=result):
                            with self.assertRaises(ContractError):
                                verify_gate(plan, broken, POLICY, comparison, expected_plan=plan)
                for key in ("base_sha", "head_sha", "tested_sha"):
                    with self.assertRaises(ContractError):
                        verify_gate(
                            {**plan, key: "d" * 40}, needs, POLICY, comparison, expected_plan=plan
                        )
                untrusted = copy.deepcopy(plan)
                untrusted["jobs"] = dict.fromkeys(POLICY.jobs, False)
                with self.assertRaises(ContractError):
                    verify_gate(untrusted, needs, POLICY, comparison, expected_plan=plan)

    def test_main_manual_release_and_explicit_local_full_select_every_original_job(self) -> None:
        tested = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
        for event in ("push", "workflow_dispatch", "workflow_call", "local"):
            with self.subTest(event=event):
                plan = make_plan(ROOT, POLICY, event=event, tested_sha=tested, force_full=True)
                self.assertEqual(plan["profile"], "full")
                self.assertEqual(plan["jobs"], dict.fromkeys(ORIGINAL_JOBS, True))
                expected, needs, comparison = ordinary_gate_fixture(POLICY, event, "full")
                verify_gate(expected, needs, POLICY, comparison, expected_plan=expected)

    def test_caller_keeps_strict_original_aggregate_without_image_activation(self) -> None:
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        for held in ("devcontainer-validation", "select_devcontainer_validation"):
            self.assertNotIn(held, ci)
        gate = ci.split("  pr-gate:\n", 1)[1]
        self.assertIn("    name: Required CI\n    if: always()\n", gate)
        dependencies = re.search(r"    needs:\n((?:      - [^\n]+\n)+)", gate)
        assert dependencies is not None
        self.assertEqual(
            [line.strip().removeprefix("- ") for line in dependencies[1].splitlines()],
            ["validation-plan", *ORIGINAL_JOBS],
        )
        self.assertIn("candidate_sha: ${{ steps.gate.outputs.candidate_sha }}", gate)
        self.assertIn("value: ${{ jobs.pr-gate.outputs.candidate_sha }}", ci)

    def test_candidate_output_is_written_only_after_successful_ordinary_gate(self) -> None:
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        block = ci.split("      - name: Require selected jobs and planned skips\n", 1)[1]
        script = block.split("        run: |\n", 1)[1].split("\non:", 1)[0]
        script = "\n".join(line.removeprefix("          ") for line in script.splitlines())
        bash = shutil.which("bash")
        assert bash is not None
        for status in (0, 17):
            with self.subTest(status=status), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                uv = directory / "uv"
                uv.write_text(
                    f"#!{sys.executable}\nimport json, os, sys\nfrom pathlib import Path\n"
                    "Path(os.environ['TRACE']).write_text(json.dumps(sys.argv[1:]))\n"
                    "raise SystemExit(int(os.environ['GATE_STATUS']))\n"
                )
                uv.chmod(0o755)
                output = directory / "outputs"
                plan, needs, comparison = ordinary_gate_fixture(POLICY, "merge_group", "full")
                encoded_plan = json.dumps(plan)
                result = subprocess.run(
                    [bash, "-c", script],
                    capture_output=True,
                    text=True,
                    check=False,
                    env={
                        **os.environ,
                        "PATH": str(directory),
                        "GATE_STATUS": str(status),
                        "TRACE": str(directory / "trace"),
                        "GITHUB_OUTPUT": str(output),
                        "GITHUB_SHA": comparison.tested_sha,
                        "EVENT_NAME": comparison.event,
                        "BASE_SHA": comparison.base_sha,
                        "HEAD_SHA": comparison.head_sha,
                        "POLICY_SHA": comparison.base_sha,
                        "AUTOMATION_SHA": "e" * 40,
                        "POLICY_CONFIG": "held-policy.json",
                        "PLAN_JSON": encoded_plan,
                        "NEEDS_JSON": json.dumps(needs),
                        "API_PURPOSE": "development",
                    },
                )
                self.assertEqual(result.returncode, status, result.stderr)
                arguments = json.loads((directory / "trace").read_text())
                self.assertEqual(arguments[arguments.index("--plan-json") + 1], encoded_plan)
                self.assertNotIn("--devcontainer-validation", arguments)
                if status == 0:
                    self.assertEqual(output.read_text(), f"candidate_sha={comparison.tested_sha}\n")
                else:
                    self.assertFalse(output.exists())


RELEASE_JOBS = (
    "deterministic",
    "native-generator-validation",
    "native-generator-evidence",
    "release-metadata",
)


def workflow_job(workflow: str, name: str) -> str:
    match = re.search(rf"(?ms)^  {re.escape(name)}:\n.*?(?=^  [\w-]+:|\Z)", workflow)
    if match is None:
        raise AssertionError(f"missing workflow job {name}")
    return match[0]


def require_shared_release_preparation(workflow: str, revision: str) -> str:
    """Bind the thin caller, leaving App/token mechanics with the selected producer."""
    if not re.fullmatch(r"[a-f0-9]{40}", revision):
        raise ValueError("shared release preparation needs an immutable source")
    job = workflow_job(workflow, "prepare-release-pr")
    reference = (
        "uses: Strukturpiloten/boxferry/.github/workflows/shared-release-prepare.yml@"
        + revision
        + " # automation-v1 interface v1"
    )
    required = (
        "if: github.repository == 'Strukturpiloten/quadlet-lens'",
        "permissions:\n      contents: read",
        "candidate_sha: ${{ github.sha }}",
        "automation_sha: " + revision,
        "policy_sha: ${{ github.sha }}",
        "config_path: scripts/validation-policy.json",
        "app_client_id: ${{ vars.RELEASE_PLZ_APP_CLIENT_ID }}",
        "app_private_key: ${{ secrets.RELEASE_PLZ_APP_PRIVATE_KEY }}",
    )
    if job.count(reference) != 1 or any(value not in job for value in required):
        raise ValueError("shared release preparation source/permission/App contract drifted")
    for forbidden in (
        "steps:",
        "runs-on:",
        "version:",
        "release-plz/action@",
        "actions/create-github-app-token@",
        "secrets: inherit",
        "contents: write",
        "actions: write",
        "pull-requests: write",
        "id-token: write",
    ):
        if forbidden in job:
            raise ValueError("shared release preparation must stay a read-only thin caller")
    return job


class ReleaseContractTests(unittest.TestCase):
    def release_policy(self) -> dict[str, object]:
        from boxferry_ci import release

        return release.load_policy(ROOT / ".github/release-policy.json", POLICY.repository)

    def test_shared_release_preparation_keeps_source_permissions_and_existing_App_inputs(
        self,
    ) -> None:
        workflow = (ROOT / ".github/workflows/release-plz.yml").read_text()
        revision = json.loads((ROOT / ".github/automation.json").read_text())["revision"]
        require_shared_release_preparation(workflow, revision)
        for old, new in (
            ("shared-release-prepare.yml@" + revision, "shared-release-prepare.yml@" + "a" * 40),
            ("automation_sha: " + revision, "automation_sha: " + "a" * 40),
            ("candidate_sha: ${{ github.sha }}", "candidate_sha: main"),
            ("policy_sha: ${{ github.sha }}", "policy_sha: main"),
            ("app_client_id: ${{ vars.RELEASE_PLZ_APP_CLIENT_ID }}", "app_client_id: missing"),
            (
                "app_private_key: ${{ secrets.RELEASE_PLZ_APP_PRIVATE_KEY }}",
                "app_private_key: missing",
            ),
            ("permissions:\n      contents: read", "permissions:\n      contents: write"),
            (
                "    uses: Strukturpiloten/boxferry/",
                "    steps: []\n    uses: Strukturpiloten/boxferry/",
            ),
        ):
            changed = workflow.replace(old, new, 1)
            self.assertNotEqual(changed, workflow)
            with self.subTest(old=old), self.assertRaises(ValueError):
                require_shared_release_preparation(changed, revision)

    def test_typed_release_policy_and_exact_original_prerequisites(self) -> None:
        from boxferry_ci import release

        policy = self.release_policy()
        self.assertEqual(
            policy,
            {
                "schema": 1,
                "repository": "Strukturpiloten/quadlet-lens",
                "packages": ["quadlet-lens"],
                "required_jobs": list(RELEASE_JOBS),
                "publication_enabled": True,
            },
        )
        candidate = "c" * 40
        deterministic = workflow_job(
            (ROOT / ".github/workflows/release.yml").read_text(), "deterministic"
        )
        self.assertIn("api_purpose: published", deterministic)
        self.assertIn(
            "value: ${{ jobs.pr-gate.outputs.published_api_candidate_sha }}",
            (ROOT / ".github/workflows/ci.yml").read_text(),
        )
        # Match the actual deterministic published-purpose output explicitly;
        # a generic successful candidate receipt does not supply API evidence.
        needs = {
            job: {
                "result": "success",
                "outputs": {
                    "candidate_sha": candidate,
                    **(
                        {"published_api_candidate_sha": candidate} if job == "deterministic" else {}
                    ),
                },
            }
            for job in RELEASE_JOBS
        }
        self.assertEqual(release.verify_gate(policy, needs, candidate), candidate)
        for job in RELEASE_JOBS:
            published = {"published_api_candidate_sha": candidate} if job == "deterministic" else {}
            for outcome in (
                None,
                {},
                {"result": "success"},
                *(
                    {"result": result, "outputs": {"candidate_sha": candidate, **published}}
                    for result in ("failure", "cancelled", "skipped", "neutral")
                ),
                *(
                    {"result": "success", "outputs": {"candidate_sha": receipt, **published}}
                    for receipt in (None, "", "a" * 40, "A" * 40, "c" * 39, candidate + "\n")
                ),
            ):
                with (
                    self.subTest(job=job, outcome=outcome),
                    self.assertRaises(release.ReleaseError),
                ):
                    release.verify_gate(policy, {**needs, job: outcome}, candidate)
            missing = dict(needs)
            missing.pop(job)
            with self.assertRaises(release.ReleaseError):
                release.verify_gate(policy, missing, candidate)
        missing_api = {
            **needs,
            "deterministic": {"result": "success", "outputs": {"candidate_sha": candidate}},
        }
        with self.assertRaisesRegex(release.ReleaseError, "exactly one explicit published API"):
            release.verify_gate(policy, missing_api, candidate)
        for invalid in (None, "", "a" * 40, "A" * 40, "c" * 39, candidate + "\n", True, {}):
            malformed = {
                **needs,
                "deterministic": {
                    "result": "success",
                    "outputs": {"candidate_sha": candidate, "published_api_candidate_sha": invalid},
                },
            }
            with self.subTest(proof=invalid), self.assertRaises(release.ReleaseError):
                release.verify_gate(policy, malformed, candidate)
        for job in RELEASE_JOBS[1:]:
            duplicate = {
                **needs,
                job: {
                    "result": "success",
                    "outputs": {
                        "candidate_sha": candidate,
                        "published_api_candidate_sha": candidate,
                    },
                },
            }
            with (
                self.subTest(duplicate=job),
                self.assertRaisesRegex(release.ReleaseError, "exactly one explicit published API"),
            ):
                release.verify_gate(policy, duplicate, candidate)
        with self.assertRaises(release.ReleaseError):
            release.verify_gate(policy, {**needs, "unknown": needs[RELEASE_JOBS[0]]}, candidate)
        for candidate in ("", "A" * 40, "c" * 39, "g" * 40, "c" * 40 + "\n"):
            with self.subTest(candidate=candidate), self.assertRaises(release.ReleaseError):
                release.verify_gate(policy, needs, candidate)
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "release.json"
            for malformed in (
                {**policy, "unknown": True},
                {key: value for key, value in policy.items() if key != "required_jobs"},
                {**policy, "required_jobs": []},
                {**policy, "required_jobs": [*RELEASE_JOBS, RELEASE_JOBS[0]]},
                {**policy, "repository": "Strukturpiloten/compose-lens"},
                {**policy, "publication_enabled": "true"},
            ):
                path.write_text(json.dumps(malformed))
                with self.subTest(policy=malformed), self.assertRaises(release.ReleaseError):
                    release.load_policy(path, POLICY.repository)

    def test_read_only_shared_preparation_rejects_bad_gate_without_registry_or_token(self) -> None:
        from boxferry_ci import release

        candidate = "c" * 40
        metadata = {
            "workspace_members": ["quadlet-lens"],
            "packages": [
                {
                    "id": "quadlet-lens",
                    "name": "quadlet-lens",
                    "version": "1.2.3",
                    "publish": None,
                    "manifest_path": str(ROOT / "Cargo.toml"),
                    "dependencies": [],
                }
            ],
        }
        arguments = [
            "prepare-crate",
            "--repository-root",
            str(ROOT),
            "--config",
            str(ROOT / ".github/release-policy.json"),
            "--package",
            "quadlet-lens",
            "--candidate-sha",
            candidate,
            "--automation-sha",
            "e" * 40,
            "--validation-sha",
            candidate,
            "--validation-result",
            "success",
            "--validation-only",
            "true",
        ]
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "outputs"
            with (
                patch.dict(
                    os.environ,
                    {
                        "GITHUB_REPOSITORY": POLICY.repository,
                        "GITHUB_OUTPUT": str(output),
                    },
                    clear=True,
                ),
                patch.object(release, "verify_bindings") as bindings,
                patch.object(
                    release.subprocess, "check_output", return_value=json.dumps(metadata)
                ) as cargo,
                patch.object(release, "registry_status") as registry,
                patch.object(release, "publish_crate") as publish,
                patch.object(release, "verify_publication_context") as auth_context,
            ):
                self.assertEqual(release.main(arguments), 0)
                bindings.assert_called_once_with(ROOT, candidate, "e" * 40)
                cargo.assert_called_once_with(
                    ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
                    cwd=ROOT,
                    text=True,
                    timeout=120,
                )
                self.assertIn("version=1.2.3\n", output.read_text())
                registry.assert_not_called()
                publish.assert_not_called()
                auth_context.assert_not_called()
                for option, bad_values in (
                    ("--validation-sha", ("", "a" * 40, "A" * 40, candidate + "\n")),
                    ("--validation-result", ("", "failure", "cancelled", "skipped")),
                ):
                    for bad in bad_values:
                        broken = list(arguments)
                        broken[broken.index(option) + 1] = bad
                        cargo.reset_mock()
                        output.unlink(missing_ok=True)
                        with (
                            self.subTest(option=option, bad=bad),
                            patch("sys.stderr", io.StringIO()),
                        ):
                            with self.assertRaises(SystemExit):
                                release.main(broken)
                        cargo.assert_not_called()
                        self.assertFalse(output.exists())
                with self.assertRaises(SystemExit), patch("sys.stderr", io.StringIO()):
                    release.main(["publish-crate", *arguments[1:]])
                registry.assert_not_called()
                publish.assert_not_called()
                auth_context.assert_not_called()

    def test_publication_requires_existing_enabled_product_and_explicit_default_branch(
        self,
    ) -> None:
        from boxferry_ci import release

        candidate = "c" * 40
        valid = {
            "GITHUB_EVENT_NAME": "workflow_dispatch",
            "GITHUB_REF": "refs/heads/main",
            "BOXFERRY_DEFAULT_BRANCH": "main",
            "GITHUB_SHA": candidate,
        }
        policy = self.release_policy()
        with patch.dict(os.environ, valid, clear=True):
            release.verify_publication_context(policy, candidate)
            with self.assertRaises(release.ReleaseError):
                release.verify_publication_context(
                    {**policy, "publication_enabled": False}, candidate
                )
        for key, value in (
            ("GITHUB_EVENT_NAME", "pull_request"),
            ("GITHUB_EVENT_NAME", "push"),
            ("GITHUB_REF", "refs/heads/release"),
            ("BOXFERRY_DEFAULT_BRANCH", ""),
            ("GITHUB_SHA", "a" * 40),
        ):
            with (
                self.subTest(key=key, value=value),
                patch.dict(os.environ, {**valid, key: value}, clear=True),
            ):
                with self.assertRaises(release.ReleaseError):
                    release.verify_publication_context(policy, candidate)

    def test_release_callers_bind_shared_gate_prepare_and_protected_publication(self) -> None:
        release = (ROOT / ".github/workflows/release.yml").read_text()
        sha = json.loads((ROOT / ".github/automation.json").read_text())["revision"]
        gate = workflow_job(release, "release-gate")
        self.assertIn("needs: [" + ", ".join(RELEASE_JOBS) + "]", gate)
        for expected in (
            f"shared-release-validation.yml@{sha} # automation-v1 interface v1",
            "candidate_sha: ${{ github.sha }}",
            "policy_sha: ${{ github.sha }}",
            f"automation_sha: {sha}",
            "config_path: .github/release-policy.json",
            "needs_json: ${{ toJSON(needs) }}",
            "if: always()",
            "contents: read",
        ):
            self.assertIn(expected, gate)
        self.assertNotIn("steps:", gate)
        self.assertNotIn("run:", gate)
        preparation = workflow_job(release, "crate-preparation")
        publisher = workflow_job(release, "release")
        self.assertIn("needs: release-gate", preparation)
        self.assertNotIn("validation_only", preparation)
        self.assertNotIn(": write", preparation)
        self.assertNotIn("environment:", preparation)
        self.assertIn('validation-only: "true"', preparation)
        self.assertIn('validation-only: "false"', publisher)
        self.assertIn("!inputs.validation_only", publisher)
        self.assertIn("environment: release", publisher)
        self.assertIn("id-token: write", publisher)
        self.assertIn("crate-preparation,", publisher)
        for job in (preparation, publisher):
            for expected in (
                "uses: ./_automation/.github/actions/shared-crate-publish",
                "uses: ./_automation/.github/actions/shared-source",
                "config-path: .github/release-policy.json",
                "release-config: ${{ steps.source.outputs.config }}",
                "validation-sha: ${{ needs.release-gate.outputs.candidate_sha }}",
                "validation-result: ${{ needs.release-gate.result }}",
                "candidate-sha: ${{ github.sha }}",
                f"automation-sha: {sha}",
                "package: quadlet-lens",
                f"ref: {sha}",
                "path: _automation",
                "policy-sha: ${{ github.sha }}",
            ):
                self.assertIn(expected, job)
            self.assertNotIn("bootstrap-token", job)
            self.assertNotIn("cargo publish", job)
            self.assertNotIn("crates-io-auth-action", job)
        hooks = [
            "Derive release publication metadata",
            "Build release package and checksum",
            "Attest release package",
            "Create or verify annotated release tag",
            "Create or replace draft GitHub release",
            "Upload draft GitHub release assets",
            "Publish crate through shared trusted publishing",
            "Publish immutable GitHub release",
        ]
        self.assertEqual(
            [publisher.index(hook) for hook in hooks],
            sorted(publisher.index(hook) for hook in hooks),
        )
        self.assertIn("subject-path: source/${{ steps.metadata.outputs.crate }}", publisher)
        for step in publisher.split("      - name: ")[1:]:
            if "\n        run:" in step:
                self.assertIn("working-directory: source", step)

    def test_native_receipts_follow_checks_cleanup_and_evidence_persistence(self) -> None:
        generators = (ROOT / ".github/workflows/generator-matrix.yml").read_text()
        final = generators.split(
            "      - name: Expose successfully persisted native candidate\n", 1
        )[1]
        self.assertNotIn("if:", final)
        self.assertIn('test "$(git rev-parse HEAD)" = "${CANDIDATE_SHA}"', final)
        self.assertIn('[[ "${CANDIDATE_SHA}" =~ ^[0-9a-f]{40}$ ]]', final)
        for required in (
            "run: cargo ci-generators",
            "run: cargo ci-application-generators",
            "run: bash ./scripts/cleanup-generator-containers.sh",
            "name: Upload exact native-generator evidence",
        ):
            self.assertLess(
                generators.index(required),
                generators.index("Expose successfully persisted native candidate"),
            )
        self.assertIn("value: ${{ jobs.supported-generators.outputs.candidate_sha }}", generators)
        release = (ROOT / ".github/workflows/release.yml").read_text()
        evidence = workflow_job(release, "native-generator-evidence")
        for required in (
            'test "${NATIVE_RESULT}" = success',
            'test "${NATIVE_CANDIDATE}" = "${GITHUB_SHA}"',
            ".revision == $revision",
            ".run_id == $run_id",
            ".task == $task",
            '.generator_lane == "full"',
            '.application_lane == "tracked-current"',
            '.outer_runtime.engine == "podman"',
            ".outer_runtime.rootless == false",
            ".outer_runtime.remote == false",
            '.outer_runtime.timeout_self_control.result == "passed"',
            ".outer_runtime.timeout_self_control.uid == 0",
            '.outer_runtime.timeout_self_control.descendants == "sigkill-reaped"',
        ):
            self.assertIn(required, evidence)
        self.assertLess(
            evidence.index("Retain release native-generator evidence"),
            evidence.index("Expose verified native evidence candidate"),
        )
        self.assertNotIn("if:", evidence.split("Expose verified native evidence candidate", 1)[1])
        metadata = workflow_job(release, "release-metadata")
        self.assertLess(
            metadata.index("bash scripts/check-release-metadata.sh --release"),
            metadata.index("printf 'candidate_sha="),
        )
        self.assertIn('if [[ "${GITHUB_REF}" != "refs/heads/${DEFAULT_BRANCH}" ]]', metadata)
        for job in (metadata, evidence):
            self.assertIn("candidate_sha: ${{ steps.verified.outputs.candidate_sha }}", job)


class ConsumerContractTests(unittest.TestCase):
    def test_api_purpose_uses_existing_lane_and_keeps_release_published(self) -> None:
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        api = ci.split("\n  api:\n", 1)[1].split("\n  documentation:\n", 1)[0]
        self.assertIn("needs: validation-plan", api)
        self.assertIn("if: needs.validation-plan.outputs.select_api == 'true'", api)
        self.assertIn("api_purpose: ${{ inputs.api_purpose || 'development' }}", api)
        self.assertIn("package: quadlet-lens", api)
        self.assertNotIn("initial_api_contract:", api)
        self.assertNotIn("release_type:", api)
        self.assertIn("        default: published", ci)
        self.assertIn("value: ${{ jobs.pr-gate.outputs.published_api_candidate_sha }}", ci)
        self.assertIn(
            "published_api_candidate_sha: ${{ steps.gate.outputs.published_api_candidate_sha }}", ci
        )
        self.assertIn(
            "PUBLISHED_API_CANDIDATE_SHA: ${{ needs.api.outputs.published_api_candidate_sha }}", ci
        )
        self.assertIn(
            "printf 'published_api_candidate_sha=%s\\n' \"${PUBLISHED_API_CANDIDATE_SHA}\"", ci
        )
        self.assertNotIn("printf 'published_api_candidate_sha=%s\\n' \"${GITHUB_SHA}\"", ci)
        release = (ROOT / ".github/workflows/release.yml").read_text()
        caller = release.split("\n  deterministic:\n", 1)[1].split(
            "\n  native-generator-validation:\n", 1
        )[0]
        self.assertIn(
            "uses: ./.github/workflows/ci.yml\n    with:\n      api_purpose: published", caller
        )
        self.assertNotIn("validation_only", caller)
        self.assertIn("!inputs.validation_only", release)
        self.assertEqual(ci.count("shared-rust-semver.yml@"), 1)

    @staticmethod
    def run_api_aggregate(**changes: str) -> tuple[int, str]:
        ci = (ROOT / ".github/workflows/ci.yml").read_text().split("\n  pr-gate:\n", 1)[1]
        blocks = re.findall(r"        run: \|\n((?:          [^\n]*\n)+)", ci)
        assert len(blocks) == 1, "exercise actual canonical gate and purpose-bound output block"
        script = "\n".join(textwrap.dedent(block) for block in blocks)
        candidate = "a" * 40
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            output = directory / "output"
            uv = directory / "uv"
            uv.write_text('#!/bin/sh\nexit "${GATE_STATUS:-0}"\n')
            uv.chmod(0o755)
            environment = {
                **os.environ,
                "PATH": f"{directory}:/usr/bin:/bin",
                "GITHUB_SHA": candidate,
                "GITHUB_OUTPUT": str(output),
                "API_PURPOSE": "published",
                "PUBLISHED_API_CANDIDATE_SHA": candidate,
                "EVENT_NAME": "workflow_dispatch",
                "BASE_SHA": "",
                "HEAD_SHA": "",
                "POLICY_SHA": candidate,
                "AUTOMATION_SHA": "b" * 40,
                "POLICY_CONFIG": "policy.json",
                "PLAN_JSON": "{}",
                "NEEDS_JSON": "{}",
                **changes,
            }
            result = subprocess.run(
                ["bash", "-euo", "pipefail", "-c", script],
                env=environment,
                capture_output=True,
                text=True,
                check=False,
            )
            return result.returncode, output.read_text() if output.exists() else ""

    def test_api_aggregate_requires_exact_published_output_after_canonical_gate(self) -> None:
        candidate = "a" * 40
        self.assertEqual(
            self.run_api_aggregate(),
            (
                0,
                f"published_api_candidate_sha={candidate}\ncandidate_sha={candidate}\n",
            ),
        )
        for purpose in ("development", "published"):
            for failure in ("1", "17"):
                with self.subTest(purpose=purpose, failure=failure):
                    status, output = self.run_api_aggregate(
                        API_PURPOSE=purpose, GATE_STATUS=failure
                    )
                    self.assertNotEqual(status, 0)
                    self.assertEqual(output, "")
        for published in ("", "b" * 40, "bad", "A" * 40, candidate + "\n"):
            with self.subTest(published=published):
                status, output = self.run_api_aggregate(PUBLISHED_API_CANDIDATE_SHA=published)
                self.assertNotEqual(status, 0)
                self.assertEqual(output, "")
                # A trusted prose plan may skip API in development; it emits no release binding.
                self.assertEqual(
                    self.run_api_aggregate(
                        API_PURPOSE="development",
                        PUBLISHED_API_CANDIDATE_SHA=published,
                    ),
                    (0, f"candidate_sha={candidate}\n"),
                )
        for purpose in ("", "unknown", "Published"):
            with self.subTest(purpose=purpose):
                status, output = self.run_api_aggregate(API_PURPOSE=purpose)
                self.assertNotEqual(status, 0)
                self.assertEqual(output, "")

    def test_selected_api_outcomes_fail_the_existing_complete_gate(self) -> None:
        candidate = "a" * 40
        comparison = Comparison("workflow_dispatch", candidate, candidate, candidate)
        plan: dict[str, object] = {
            "schema": 3,
            "repository": POLICY.repository,
            "event": comparison.event,
            "base_sha": candidate,
            "head_sha": candidate,
            "tested_sha": candidate,
            "profile": "full",
            "profiles": ["full"],
            "jobs": dict.fromkeys(POLICY.jobs, True),
            "reasons": ["complete candidate"],
            "local_fingerprint": None,
        }
        needs: dict[str, dict[str, object]] = {
            job: {"result": "success"} for job in ("validation-plan", *POLICY.jobs)
        }
        verify_gate(plan, needs, POLICY, comparison, expected_plan=plan)
        for outcome in ("failure", "cancelled", "skipped", "", "unknown", None):
            with self.subTest(outcome=outcome):
                broken = copy.deepcopy(needs)
                broken["api"]["result"] = outcome
                with self.assertRaises(ContractError):
                    verify_gate(plan, broken, POLICY, comparison, expected_plan=plan)
        missing = copy.deepcopy(needs)
        del missing["api"]
        with self.assertRaises(ContractError):
            verify_gate(plan, missing, POLICY, comparison, expected_plan=plan)

    def test_local_api_task_uses_selected_identity_and_preserves_isolation(self) -> None:
        policy = json.loads((ROOT / "scripts/validation-policy.json").read_text())
        self.assertEqual(
            policy["tasks"]["api-development"],
            {
                "commands": [
                    [
                        "python",
                        "-m",
                        "boxferry_ci.api_baselines",
                        "run",
                        "--repository-root",
                        ".",
                        "--repository",
                        "Strukturpiloten/quadlet-lens",
                        "--api-purpose",
                        "development",
                        "--event",
                        "local",
                        "--package",
                        "quadlet-lens",
                        "--report-json",
                        "target/api-baseline-local.json",
                    ]
                ],
                "timeout_seconds": 1200,
            },
        )
        self.assertNotIn("initial-api-contract", policy["tasks"])
        local = (ROOT / "scripts/check-all.sh").read_text()
        self.assertIn(
            'run_step "Check development API compatibility" env CARGO_HOME="${semver_cargo_home}"',
            local,
        )
        self.assertIn(
            'CARGO_TARGET_DIR="${semver_target_dir}" RUSTUP_TOOLCHAIN="${semver_rust_channel}"',
            local,
        )
        self.assertIn(
            "python3 scripts/shared-ci.py run --event local --task api-development", local
        )
        self.assertIn("--report-json target/api-development-task.json", local)
        self.assertNotIn("cargo semver-checks check-release", local)
        self.assertIn("scripts/shared-ci.py cargo-tools verify", local)
        self.assertNotIn("--automation-sha", json.dumps(policy["tasks"]["api-development"]))

    def test_release_ci_supplies_the_nested_dependency_read_scopes(self) -> None:
        release = (ROOT / ".github/workflows/release.yml").read_text()
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        require_release_ci_read_permissions(release, ci)
        for position, source, name in (
            (0, release, "deterministic"),
            (1, ci, "lockfile-release-age"),
        ):
            match = re.search(rf"(?ms)^  {re.escape(name)}:\n.*?(?=^  [\w-]+:\n|\Z)", source)
            assert match is not None
            body = match[0]
            for scope in ("actions", "contents", "statuses"):
                declaration = f"      {scope}: read\n"
                for replacement in ("", f"      {scope}: write\n"):
                    with self.subTest(job=name, scope=scope, replacement=replacement):
                        mutated = source.replace(body, body.replace(declaration, replacement, 1), 1)
                        arguments = [release, ci]
                        arguments[position] = mutated
                        with self.assertRaises(ValueError):
                            require_release_ci_read_permissions(*arguments)
        for extra in ("      pull-requests: write\n", "      id-token: write\n"):
            mutated = release.replace("    permissions:\n", "    permissions:\n" + extra, 1)
            with self.subTest(extra=extra), self.assertRaises(ValueError):
                require_release_ci_read_permissions(mutated, ci)

    def test_prose_examples_native_and_unknown_paths(self) -> None:
        for path, content, expected in (
            ("docs/public/model/index.md", b"Plain prose.\n", {"prose"}),
            ("docs/public/model/index.md", b"```rust\nfn main() {}\n```", {"executable-docs"}),
            ("docs/public/model/index.md", b"Call `QuadletDocument::parse`.", {"executable-docs"}),
            ("fixtures/generators/example/fixture.toml", b"schema=1", {"full"}),
            ("tools/generator-matrix.toml", b"changed", {"full"}),
            ("tools/generator-history.toml", b"changed", {"full"}),
            ("src/model.rs", b"changed", {"full"}),
            ("scripts/validation-policy.json", b"changed", {"full"}),
            ("docs/internal/unknown.md", b"prose", {"full"}),
        ):
            with self.subTest(path=path, expected=expected):
                self.assertEqual(path_profiles(path, (content,), POLICY, set()), expected)
        self.assertEqual(POLICY.selected_jobs(("full",)), set(POLICY.jobs))
        self.assertEqual(
            POLICY.selected_jobs(("prose",)), {"documentation", "lockfile-release-age"}
        )
        self.assertEqual(
            POLICY.selected_jobs(("executable-docs",)),
            set(ORIGINAL_JOBS),
        )

    def test_pr_and_merge_queue_require_selected_jobs_and_planned_skips(self) -> None:
        for event in ("pull_request", "merge_group"):
            comparison = Comparison(event, "a" * 40, "b" * 40, "c" * 40)
            selected = POLICY.selected_jobs(("prose",))
            plan: dict[str, object] = {
                "schema": 3,
                "repository": POLICY.repository,
                "event": event,
                "base_sha": comparison.base_sha,
                "head_sha": comparison.head_sha,
                "tested_sha": comparison.tested_sha,
                "profile": "prose",
                "profiles": ["prose"],
                "jobs": {job: job in selected for job in POLICY.jobs},
                "reasons": ["public prose"],
                "local_fingerprint": None,
            }
            needs: dict[str, dict[str, object]] = {
                job: {"result": "success" if job in selected else "skipped"} for job in POLICY.jobs
            }
            needs["validation-plan"] = {"result": "success"}
            verify_gate(plan, needs, POLICY, comparison, expected_plan=plan)
            for result in ("failure", "cancelled", "skipped", None):
                broken = copy.deepcopy(needs)
                broken["documentation"]["result"] = result
                with self.assertRaises(ContractError):
                    verify_gate(plan, broken, POLICY, comparison, expected_plan=plan)
            unexpected = copy.deepcopy(needs)
            unexpected.pop("rust")
            with self.assertRaises(ContractError):
                verify_gate(plan, unexpected, POLICY, comparison, expected_plan=plan)
            unselected_failure = copy.deepcopy(needs)
            unselected_failure["rust"]["result"] = "failure"
            with self.assertRaises(ContractError):
                verify_gate(plan, unselected_failure, POLICY, comparison, expected_plan=plan)
            wrong_candidate = {**plan, "tested_sha": "d" * 40}
            with self.assertRaises(ContractError):
                verify_gate(wrong_candidate, needs, POLICY, comparison, expected_plan=plan)
            mixed = {
                **plan,
                "profiles": ["full"],
                "profile": "full",
                "jobs": dict.fromkeys(POLICY.jobs, True),
            }
            with self.assertRaises(ContractError):
                verify_gate(plan, needs, POLICY, comparison, expected_plan=mixed)

    def test_rust_and_coverage_tasks_preserve_product_contract(self) -> None:
        calls = POLICY.task("rust-quality").commands
        for alias in (
            "ci-check",
            "ci-application",
            "ci-policy",
            "ci-clippy",
            "ci-test",
            "ci-doctest",
            "ci-doc",
            "ci-package",
        ):
            self.assertIn(("cargo", alias), calls)
        coverage = POLICY.task("coverage").commands[-1]
        for option, expected in (
            ("--fail-under-regions", "91"),
            ("--fail-under-functions", "92"),
            ("--fail-under-lines", "92"),
        ):
            self.assertEqual(coverage[coverage.index(option) + 1], expected)
        for calls in POLICY.local_commands.values():
            for call in calls:
                self.assertNotIn("ci-generators", call)
                self.assertNotIn("ci-real-world-quadlet", call)

    def test_effective_renovate_rules_keep_discovery_atomic_and_automatic(self) -> None:
        import boxferry_ci

        shared = Path(boxferry_ci.__file__).resolve().parents[2]
        spec = importlib.util.spec_from_file_location(
            "shared_presets", shared / "tests/test_presets.py"
        )
        assert spec is not None and spec.loader is not None
        presets = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(presets)
        config = presets.consumer_config(ROOT / ".github/renovate.json")
        presets.assert_automatic_policy(config)
        for name in ("podman-upstream", "podman-5.8", "podman-6.1"):
            result = presets.effective(
                config,
                {
                    "manager": "custom.regex",
                    "datasource": "github-releases",
                    "packageName": "podman-container-tools/podman",
                    "depName": name,
                    "fileName": "tools/generator-matrix.toml",
                    "updateType": "patch",
                },
            )
            self.assertEqual(result["groupName"], "Podman generator discovery")
            self.assertTrue(result["automerge"])
            self.assertFalse(result["dependencyDashboardApproval"])
            self.assertEqual(result["minimumReleaseAge"], "3 days")

    def test_direct_image_proposals_are_held_without_blocking_ordinary_inputs(self) -> None:
        import boxferry_ci

        shared = Path(boxferry_ci.__file__).resolve().parents[2]
        spec = importlib.util.spec_from_file_location(
            "image_holds", shared / "tests/test_presets.py"
        )
        assert spec is not None and spec.loader is not None
        presets = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(presets)
        local = json.loads((ROOT / ".github/renovate.json").read_text())
        holds = local["packageRules"][-4:]
        self.assertTrue(all(rule.get("enabled") is False for rule in holds))
        self.assertTrue(all("#563" in rule["description"] for rule in holds))
        self.assertTrue(all(len(rule["matchFileNames"]) == 1 for rule in holds))
        self.assertEqual(
            {tuple(rule["matchFileNames"]) for rule in holds},
            {(".devcontainer/Dockerfile",), (".devcontainer/devcontainer.json",)},
        )
        self.assertFalse(
            any("dependencyDashboardApproval" in rule or "automerge" in rule for rule in holds)
        )
        config = presets.consumer_config(ROOT / ".github/renovate.json")
        self.assertFalse(config["dockerfile"]["enabled"])
        direct = (
            [
                ("custom.regex", "docker", name, ".devcontainer/Dockerfile", "")
                for name in ("docker.io/library/node", "mcr.microsoft.com/devcontainers/rust")
            ]
            + [
                ("custom.regex", "crate", name, ".devcontainer/Dockerfile", "")
                for name in ("cargo-deny", "cargo-llvm-cov", "cargo-semver-checks", "zizmor")
            ]
            + [
                (
                    "devcontainer",
                    "docker",
                    "ghcr.io/devcontainers/features/github-cli",
                    ".devcontainer/devcontainer.json",
                    "feature",
                ),
                (
                    "custom.regex",
                    "github-releases",
                    "cli/cli",
                    ".devcontainer/devcontainer.json",
                    "",
                ),
            ]
        )
        for manager, datasource, name, filename, dep_type in direct:
            dependency = {
                "manager": manager,
                "datasource": datasource,
                "packageName": name,
                "depName": name,
                "fileName": filename,
                "depType": dep_type,
                "updateType": "patch",
            }
            with self.subTest(dependency=dependency):
                result = presets.effective(config, dependency)
                self.assertIs(result.get("enabled"), False)
                self.assertFalse(result["dependencyDashboardApproval"])
            if manager != "devcontainer":
                source = (ROOT / filename).read_text()
                owners = []
                for owner in config["customManagers"]:
                    if not presets._matches(filename, owner["managerFilePatterns"], file=True):
                        continue
                    for pattern in owner["matchStrings"]:
                        pattern = re.sub(r"\(\?<([A-Za-z][A-Za-z0-9_]*)>", r"(?P<\1>", pattern)
                        for match in re.finditer(pattern, source):
                            package = match.groupdict().get("depName", owner.get("depNameTemplate"))
                            if package == name:
                                owners.append(owner["description"])
                self.assertEqual(len(owners), 1, (name, owners))
        ordinary = [
            {
                "manager": "cargo",
                "datasource": "crate",
                "packageName": name,
                "fileName": "Cargo.toml",
                "updateType": "patch",
            }
            for name in ("serde", "cargo-deny", "cargo-llvm-cov", "cargo-semver-checks", "zizmor")
        ] + [
            {
                "manager": "npm",
                "datasource": "npm",
                "packageName": "prettier",
                "fileName": "package.json",
                "updateType": "major",
            },
            {
                "manager": "rust-toolchain",
                "datasource": "rust-version",
                "packageName": "rust",
                "fileName": "rust-toolchain.toml",
                "updateType": "minor",
            },
            {
                "manager": "custom.regex",
                "datasource": "github-digest",
                "packageName": "Strukturpiloten/boxferry",
                "fileName": ".github/automation.json",
                "updateType": "digest",
            },
            {
                "manager": "custom.regex",
                "datasource": "crate",
                "packageName": "cargo-llvm-cov",
                "fileName": ".github/workflows/ci.yml",
                "updateType": "patch",
            },
        ]
        for dependency in ordinary:
            with self.subTest(ordinary=dependency):
                result = presets.effective(config, dependency)
                self.assertIsNot(result.get("enabled"), False)
                self.assertTrue(result["automerge"])
                self.assertFalse(result["dependencyDashboardApproval"])
                expected_age = (
                    "0 days"
                    if dependency["packageName"] == "Strukturpiloten/boxferry"
                    else "3 days"
                )
                self.assertEqual(result["minimumReleaseAge"], expected_age)

    def test_shared_age_policy_keeps_exact_whole_lens_release_exceptions(self) -> None:
        import boxferry_ci

        shared = Path(boxferry_ci.__file__).resolve().parents[2]
        spec = importlib.util.spec_from_file_location(
            "shared_age_presets", shared / "tests/test_presets.py"
        )
        assert spec is not None and spec.loader is not None
        presets = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(presets)
        configuration = presets.consumer_config(ROOT / ".github/renovate.json")
        for name in (
            "compose-lens",
            "podman-lens",
            "quadlet-lens",
            "docker-lens",
            "kubernetes-lens",
        ):
            dependency = {
                "manager": "cargo",
                "datasource": "crate",
                "depName": name,
                "packageName": name,
                "updateType": "minor",
                "fileName": "Cargo.toml",
            }
            with self.subTest(name=name):
                result = presets.effective(configuration, dependency)
                self.assertEqual(result["minimumReleaseAge"], "0 days")
                self.assertEqual(result["groupName"], "First-party Rust releases")
                self.assertTrue(result["automerge"])
                self.assertFalse(result["dependencyDashboardApproval"])
            for changed in (
                {"depName": name + "-untrusted", "packageName": name + "-untrusted"},
                {"manager": "npm", "datasource": "npm"},
                {"depName": "serde", "packageName": "serde"},
            ):
                with self.subTest(name=name, changed=changed):
                    result = presets.effective(configuration, {**dependency, **changed})
                    self.assertEqual(result["minimumReleaseAge"], "3 days")

    def test_eligibility_service_and_candidate_reader_keep_separate_authority(self) -> None:
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        reader = workflow.split("  lockfile-release-age:\n", 1)[1].split("  pr-gate:\n", 1)[0]
        for required in (
            "name: Dependency eligibility",
            "actions: read",
            "contents: read",
            "statuses: read",
            "shared-dependency-wait.yml@",
            "candidate_sha: ${{ github.sha }}",
            "automation_sha:",
            "policy_sha: ${{ github.event.pull_request.base.sha || github.event.merge_group.base_sha || github.sha }}",
        ):
            self.assertIn(required, reader)
        for forbidden in ("actions: write", "statuses: write", "steps:", "runs-on:"):
            self.assertNotIn(forbidden, reader)
        service = (ROOT / ".github/workflows/dependency-eligibility.yml").read_text()
        for required in (
            "schedule:",
            'cron: "23 * * * *"',
            "workflow_run:",
            "workflows: [CI]",
            "types: [requested, completed]",
            "actions: write",
            "contents: read",
            "pull-requests: read",
            "statuses: write",
            "shared-dependency-eligibility.yml@",
            "automation-sha:",
        ):
            self.assertIn(required, service)
        for forbidden in ("steps:", "pull_request:", "pull_request_target:"):
            self.assertNotIn(forbidden, service)

    def test_waiter_and_release_have_unique_shared_revision_renovate_owners(self) -> None:
        import boxferry_ci

        shared = Path(boxferry_ci.__file__).resolve().parents[2]
        spec = importlib.util.spec_from_file_location(
            "shared_presets", shared / "tests/test_presets.py"
        )
        assert spec is not None and spec.loader is not None
        presets = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(presets)
        config = presets.consumer_config(ROOT / ".github/renovate.json")
        for workflow, filename in (
            ("shared-dependency-wait.yml", ".github/workflows/ci.yml"),
            ("shared-rust-semver.yml", ".github/workflows/ci.yml"),
            ("shared-release-validation.yml", ".github/workflows/release.yml"),
        ):
            result = presets.effective(
                config,
                {
                    "manager": "github-actions",
                    "datasource": "github-tags",
                    "packageName": "Strukturpiloten/boxferry",
                    "depName": f"Strukturpiloten/boxferry/.github/workflows/{workflow}",
                    "fileName": filename,
                    "updateType": "digest",
                },
            )
            self.assertTrue(result["automerge"])
            self.assertFalse(result["dependencyDashboardApproval"])
            self.assertEqual(result["groupName"], "Shared BoxFerry automation")
            # Native github-actions owns the workflow reference; the shared custom
            # manager owns only the explicit input, so adoption has no duplicate owner.
            pin = "a" * 40
            sample = (
                "    uses: Strukturpiloten/boxferry/.github/workflows/"
                f"{workflow}@{pin} # automation-v1 interface v1\n"
                f"    with:\n      automation_sha: {pin}\n"
            )
            owners = []
            for manager in config["customManagers"]:
                if not presets._matches(filename, manager["managerFilePatterns"], file=True):
                    continue
                for pattern in manager["matchStrings"]:
                    pattern = re.sub(r"\(\?<([A-Za-z][A-Za-z0-9_]*)>", r"(?P<\1>", pattern)
                    for match in re.finditer(pattern, sample):
                        if match.groupdict().get("currentDigest") == pin:
                            owners.append(match.start("currentDigest"))
            self.assertEqual(owners, [sample.rindex(pin)])

        filename = ".github/workflows/release.yml"
        source = (ROOT / filename).read_text()
        revision = json.loads((ROOT / ".github/automation.json").read_text())["revision"]
        extracted = []
        for manager in config["customManagers"]:
            if not presets._matches(filename, manager["managerFilePatterns"], file=True):
                continue
            for pattern in manager["matchStrings"]:
                pattern = re.sub(r"\(\?<([A-Za-z][A-Za-z0-9_]*)>", r"(?P<\1>", pattern)
                extracted.extend(
                    match.start("currentDigest")
                    for match in re.finditer(pattern, source)
                    if match.groupdict().get("currentDigest") == revision
                )
        expected = [
            match.start(1)
            for match in re.finditer(
                r"(?m)^\s*(?:automation_sha|automation-sha|ref): (" + revision + ")$", source
            )
        ]
        self.assertEqual(sorted(extracted), sorted(expected))
        self.assertEqual(len(extracted), len(set(extracted)))

    def test_interface_one_loader_a_delegates_to_selected_implementation_b(self) -> None:
        spec = importlib.util.spec_from_file_location(
            "consumer_loader", ROOT / "scripts/shared-ci.py"
        )
        assert spec is not None and spec.loader is not None
        loader = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(loader)
        self.assertEqual(loader.INTERFACE, 1)
        self.assertEqual(loader.REPOSITORY, "Strukturpiloten/boxferry")
        with tempfile.TemporaryDirectory(prefix="quadlet-loader-contract-") as temporary:
            consumer = Path(temporary)
            manifest = consumer / ".github/automation.json"
            manifest.parent.mkdir()
            config = {
                "schema": 1,
                "repository": loader.REPOSITORY,
                "revision": "a" * 40,
                "policy": "scripts/validation-policy.json",
                "interface": 1,
            }
            manifest.write_text(json.dumps(config))
            self.assertEqual(loader.load_config(consumer)["revision"], "a" * 40)
            # The already loaded A frontloader follows the manifest's upgrade to B.
            config["revision"] = "b" * 40
            manifest.write_text(json.dumps(config))
            project = consumer / "selected-b/automation"
            implementation = project / "src/boxferry_ci/bootstrap.py"
            implementation.parent.mkdir(parents=True)
            implementation.write_text(
                "import json\n"
                "def run_project(repository_root, project, arguments):\n"
                "    (repository_root / 'delegated.json').write_text(json.dumps({\n"
                "        'project': str(project), 'arguments': arguments}))\n"
                "    return 23\n"
            )
            with (
                patch.object(loader, "__file__", str(consumer / "scripts/shared-ci.py")),
                patch.object(loader, "prepare_implementation", return_value=project) as prepare,
            ):
                self.assertEqual(loader.main(["plan", "--event", "local"]), 23)
            prepare.assert_called_once_with(consumer, "b" * 40)
            evidence = json.loads((consumer / "delegated.json").read_text())
            self.assertEqual(evidence["project"], str(project))
            self.assertEqual(
                evidence["arguments"],
                ["plan", "--event", "local"],
            )

    def test_core_tool_wrapper_preserves_arguments_and_shared_failure_status(self) -> None:
        with tempfile.TemporaryDirectory(prefix="quadlet-tool-wrapper-") as directory:
            temporary = Path(directory)
            script = temporary / "install-file-tools.sh"
            script.write_text((ROOT / "scripts/install-file-tools.sh").read_text())
            (temporary / "shared-ci.py").write_text(
                "import json, sys\nfrom pathlib import Path\n"
                "Path(__file__).with_name('arguments.json').write_text(json.dumps(sys.argv[1:]))\n"
                "raise SystemExit(17)\n"
            )
            destination = temporary / "installed tools"
            result = subprocess.run(
                ["bash", str(script), str(destination)], capture_output=True, check=False
            )
            self.assertEqual(result.returncode, 17)
            self.assertEqual(
                json.loads((temporary / "arguments.json").read_text()),
                ["tools", "--destination", str(destination), "--profile", "core"],
            )
            (temporary / "arguments.json").unlink()
            result = subprocess.run(
                ["bash", str(script), str(destination), "extra"], capture_output=True, check=False
            )
            self.assertEqual(result.returncode, 2)
            self.assertFalse((temporary / "arguments.json").exists())

    def test_canonical_tool_inventory_rejects_corruption_before_installation(self) -> None:
        from boxferry_ci import tooling

        self.assertEqual(set(tooling.CORE), {"tombi", "shfmt", "shellcheck", "hadolint"})
        inventory = tooling.load_manifest()
        self.assertIn(("actionlint", "x86_64"), inventory)
        self.assertIn(("lychee", "x86_64"), inventory)

        def corrupt(_asset: tooling.Asset, destination: Path) -> None:
            destination.write_bytes(b"corrupt")

        with tempfile.TemporaryDirectory(prefix="quadlet-tool-integrity-") as directory:
            destination = Path(directory).resolve() / "tools"
            with self.assertRaisesRegex(tooling.ToolError, "SHA256"):
                tooling.install(destination, ("lychee",), architecture="x86_64", downloader=corrupt)
            self.assertFalse((destination / "lychee").exists())

    def test_devcontainer_closure_and_ci_use_the_same_canonical_tool_inventory(self) -> None:
        dockerfile = (ROOT / ".devcontainer/Dockerfile").read_text()
        for required in (
            "COPY .github/automation.json /tmp/quadlet-lens-tooling/.github/automation.json",
            "COPY scripts/shared-ci.py scripts/install-file-tools.sh /tmp/quadlet-lens-tooling/scripts/",
            "bash /tmp/quadlet-lens-tooling/scripts/install-file-tools.sh /usr/local/bin",
            "--destination /usr/local/bin --tools actionlint lychee",
        ):
            self.assertIn(required, dockerfile)
        for obsolete in ("ACTIONLINT_VERSION", "LYCHEE_VERSION", "checksums.txt", "lychee_version"):
            self.assertNotIn(obsolete, dockerfile)
        installer = (ROOT / "scripts/install-file-tools.sh").read_text()
        self.assertIn("--profile core", installer)
        self.assertNotIn("readonly tombi_version", installer)
        config = json.loads((ROOT / ".github/renovate.json").read_text())
        self.assertFalse(
            any(
                "install-file-tools" in pattern
                for manager in config["customManagers"]
                for pattern in manager["managerFilePatterns"]
            )
        )
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertIn("--tools actionlint lychee", ci)
        self.assertIn("run: actionlint", ci)
        self.assertNotIn("sudo bash scripts/install-file-tools.sh", ci)
        self.assertNotIn("lycheeverse/lychee-action@", ci)
        external = (ROOT / ".github/workflows/documentation-links.yml").read_text()
        for required in (
            "--tools lychee",
            "path: .lycheecache",
            "--cache",
            "GITHUB_TOKEN: ${{ github.token }}",
        ):
            self.assertIn(required, external)

    def test_loader_archives_only_exact_inert_cargo_metadata(self) -> None:
        spec = importlib.util.spec_from_file_location(
            "archive_loader", ROOT / "scripts/shared-ci.py"
        )
        assert spec is not None and spec.loader is not None
        loader = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(loader)
        with tempfile.TemporaryDirectory(prefix="quadlet-loader-archive-") as directory:
            root = Path(directory)
            marker = root / "must-not-execute"
            payload = f"FROM scratch\nRUN touch {marker}\n".encode()
            archive_path = root / "metadata.tar"

            def write_archive(name: str, kind: bytes = tarfile.REGTYPE) -> None:
                with tarfile.open(archive_path, "w") as archive:
                    member = tarfile.TarInfo(name)
                    member.type = kind
                    member.size = len(payload) if kind == tarfile.REGTYPE else 0
                    member.linkname = "outside" if kind == tarfile.SYMTYPE else ""
                    archive.addfile(member, io.BytesIO(payload) if member.size else None)

            write_archive(".devcontainer/Dockerfile")
            output = root / "valid"
            output.mkdir()
            loader.extract_archive(archive_path, output)
            self.assertEqual((output / ".devcontainer/Dockerfile").read_bytes(), payload)
            self.assertFalse(marker.exists())
            for index, (name, kind) in enumerate(
                (
                    (".devcontainer/payload.sh", tarfile.REGTYPE),
                    (".devcontainer/Dockerfile/extra", tarfile.REGTYPE),
                    (".devcontainer/Dockerfile", tarfile.SYMTYPE),
                    (".devcontainer", tarfile.REGTYPE),
                )
            ):
                with self.subTest(name=name, kind=kind), self.assertRaises(loader.BootstrapError):
                    write_archive(name, kind)
                    rejected = root / f"rejected-{index}"
                    rejected.mkdir()
                    loader.extract_archive(archive_path, rejected)

    def test_uv_is_owned_by_selected_shared_implementation(self) -> None:
        installer = (ROOT / "scripts/install-file-tools.sh").read_text()
        for removed in ("uv_version", "uv_checksum", "astral-sh/uv", "uv_archive"):
            self.assertNotIn(removed, installer)
        self.assertNotIn("tombi uv)", (ROOT / "scripts/check-files.sh").read_text())

    def test_shared_Node_caller_binds_selected_authority_order_and_existing_cache(self) -> None:
        revision = json.loads((ROOT / ".github/automation.json").read_text())["revision"]
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        match = re.search(r"(?ms)^  documentation:\n.*?(?=^  [\w-]+:\n|\Z)", workflow)
        assert match is not None
        job = match[0]
        checkout = job.index("      - name: Check out immutable shared automation\n")
        python = job.index("uses: ./_automation/.github/actions/shared-python")
        node = job.index("uses: ./_automation/.github/actions/shared-node")
        npm = job.index("npm ci --ignore-scripts")
        self.assertLess(checkout, python)
        self.assertLess(python, node)
        self.assertLess(node, npm)
        self.assertEqual(job.count("uses: ./_automation/.github/actions/shared-python"), 1)
        self.assertEqual(job.count("uses: ./_automation/.github/actions/shared-node"), 1)
        self.assertIn(f"ref: {revision}", job[checkout:python])
        block = re.search(
            r"(?ms)^      - name: Install canonical Node.js from selected automation\n.*?(?=^      - name:|\Z)",
            job,
        )
        assert block is not None
        self.assertIn(f"automation-sha: {revision}", block[0])
        self.assertIn("          cache: npm\n", block[0])
        self.assertIn("          cache-dependency-path: package-lock.json\n", block[0])
        self.assertTrue((ROOT / "package-lock.json").is_file())
        self.assertNotRegex(job, r"(?m)^\s*node-version:")
        self.assertNotIn("actions/setup-node@", job)
        self.assertNotIn("# renovate: datasource=node-version depName=node", job)
        self.assertNotRegex(block[0], r"(?m)^\s*(?:version|node-version):")

    def test_reference_manifest_binds_preset_and_workflow_revisions(self) -> None:
        manifest = json.loads((ROOT / ".github/automation.json").read_text())
        self.assertEqual(set(manifest), {"schema", "repository", "revision", "policy", "interface"})
        self.assertEqual(manifest["schema"], 1)
        self.assertEqual(manifest["interface"], 1)
        self.assertEqual(manifest["repository"], "Strukturpiloten/boxferry")
        self.assertEqual(manifest["policy"], "scripts/validation-policy.json")
        self.assertRegex(manifest["revision"], r"^[a-f0-9]{40}$")
        renovate = json.loads((ROOT / ".github/renovate.json").read_text())
        self.assertEqual(
            renovate["extends"],
            [
                f"github>Strukturpiloten/boxferry//renovate-presets/{name}#{manifest['revision']}"
                for name in ("common", "rust")
            ],
        )
        references = []
        for workflow in (ROOT / ".github/workflows").glob("*.yml"):
            references.extend(
                re.findall(
                    r"Strukturpiloten/boxferry/\.github/workflows/shared-[^@\s]+@([^\s]+)",
                    workflow.read_text(),
                )
            )
        self.assertGreaterEqual(len(references), 5)
        self.assertTrue(all(reference == manifest["revision"] for reference in references))
        # The checkout and action/workflow input bindings select executable code too.
        # A matching reusable reference alone cannot make a mixed-source upgrade valid.
        for workflow in (ROOT / ".github/workflows").glob("*.yml"):
            bindings = re.findall(
                r"(?m)^\s*(?:automation_sha|automation-sha|AUTOMATION_SHA|ref): ([a-f0-9]{40})$",
                workflow.read_text(),
            )
            with self.subTest(workflow=workflow.name):
                self.assertTrue(all(binding == manifest["revision"] for binding in bindings))

    def test_api_source_binding_remains_a_devcontainer_build_input(self) -> None:
        dockerfile = (ROOT / ".devcontainer/Dockerfile").read_text()
        installer = (ROOT / "scripts/install-file-tools.sh").read_text()
        self.assertIn(
            "COPY .github/automation.json /tmp/quadlet-lens-tooling/.github/automation.json",
            dockerfile,
        )
        self.assertIn(
            "COPY scripts/shared-ci.py scripts/install-file-tools.sh "
            "/tmp/quadlet-lens-tooling/scripts/",
            dockerfile,
        )
        self.assertIn(
            "bash /tmp/quadlet-lens-tooling/scripts/install-file-tools.sh /usr/local/bin",
            dockerfile,
        )
        self.assertIn('exec python3 "${script_directory}/shared-ci.py" tools', installer)
        self.assertIn("rm -r /tmp/quadlet-lens-tooling", dockerfile)
        self.assertEqual(
            path_profiles(".github/automation.json", (b"changed",), POLICY, set()), {"full"}
        )


if __name__ == "__main__":
    unittest.main()
