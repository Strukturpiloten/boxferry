#!/usr/bin/env python3
"""Deterministic caller regressions; --native requires a verified Renovate cache."""

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
import tomllib
import unittest
from collections.abc import Generator
from contextlib import contextmanager
from pathlib import Path
from typing import Any
from unittest.mock import patch

from boxferry_ci import release as shared_release
from boxferry_ci.contracts import ContractError
from boxferry_ci.gate import verify_gate
from boxferry_ci.planner import Comparison, make_plan, path_profiles
from boxferry_ci.policy import Policy, load_policy

ROOT = Path(__file__).resolve().parent.parent
POLICY = load_policy(ROOT / "scripts/validation-policy.json")


def shared_archive_fixture(
    metadata_path: str | None = ".devcontainer/Dockerfile", metadata_kind: bytes = tarfile.REGTYPE
) -> bytes:
    """Author the interface-1 closure without repository Git objects or network."""
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w:") as archive:
        for path, data in (
            ("automation/pyproject.toml", b"[project]\nname = 'fixture'\n"),
            ("automation/uv.lock", b"version = 1\n"),
            ("renovate-presets/common.json", b"{}\n"),
        ):
            entry = tarfile.TarInfo(path)
            entry.size = len(data)
            archive.addfile(entry, io.BytesIO(data))
        if metadata_path is not None:
            directory = tarfile.TarInfo(".devcontainer/")
            directory.type = tarfile.DIRTYPE
            archive.addfile(directory)
            entry = tarfile.TarInfo(metadata_path)
            entry.type = metadata_kind
            entry.mode = 0o755
            data = b"ARG CARGO_DENY_VERSION=9.8.7\nRUN false\n"
            if entry.isfile():
                entry.size = len(data)
            if entry.issym():
                entry.linkname = "../automation/pyproject.toml"
            archive.addfile(entry, io.BytesIO(data) if entry.isfile() else None)
    return output.getvalue()


def hosted_gate_case(
    policy: Policy, event: str, profile: str
) -> tuple[dict[str, object], dict[str, dict[str, object]], Comparison]:
    """Author a caller plan with a separate exact tested merge candidate."""
    tested = "c" * 40
    pull = event in {"pull_request", "merge_group"}
    comparison = Comparison(
        event, "a" * 40 if pull else tested, "b" * 40 if pull else tested, tested
    )
    selected = policy.selected_jobs((profile,))
    plan: dict[str, object] = {
        "schema": 3,
        "repository": policy.repository,
        "event": event,
        "base_sha": comparison.base_sha,
        "head_sha": comparison.head_sha,
        "tested_sha": tested,
        "profile": profile,
        "profiles": [profile],
        "jobs": {job: job in selected for job in policy.jobs},
        "reasons": ["independent caller expectation"],
        "local_fingerprint": None,
    }
    needs: dict[str, dict[str, object]] = {
        job: {"result": "success" if job in selected else "skipped"} for job in policy.jobs
    }
    needs["validation-plan"] = {"result": "success"}
    return plan, needs, comparison


def workflow_job(workflow: str, name: str) -> str:
    match = re.search(rf"(?ms)^  {re.escape(name)}:\n.*?(?=^  [\w-]+:\n|\Z)", workflow)
    if match is None:
        raise ValueError(f"missing workflow job {name}")
    return match[0]


def require_release_ci_permission_ceiling(release: str, ci: str) -> None:
    """A reusable workflow cannot elevate any explicitly declared callee job scope."""
    caller = workflow_job(release, "deterministic")
    declared = re.search(r"(?m)^    permissions:\n((?:      [^\n]+\n)+)", caller)
    if declared is None:
        raise ValueError("release caller must declare the reusable CI token ceiling")
    granted = dict(re.findall(r"^      ([a-z-]+): (read|write|none)$", declared[1], re.M))
    levels = {"none": 0, "read": 1, "write": 2}
    for name in re.findall(r"^  ([a-z][a-z-]*):$", ci, re.M):
        job = workflow_job(ci, name)
        requested = re.search(r"(?m)^    permissions:\n((?:      [^\n]+\n)+)", job)
        if requested is None:
            continue
        scopes = re.findall(r"^      ([a-z-]+): (read|write|none)$", requested[1], re.M)
        for scope, permission in scopes:
            if levels[permission] > levels[granted.get(scope, "none")]:
                raise ValueError(f"CI job {name} exceeds release caller permission for {scope}")


def require_eligibility_service_permissions(workflow: str) -> None:
    """The trusted service job grants exactly the immutable callee's required scopes."""
    if re.findall(r"(?m)^permissions:[^\n]*$", workflow) != ["permissions: {}"]:
        raise ValueError("eligibility workflow must grant no default token scopes")
    job = workflow_job(workflow, "eligibility")
    declared = re.search(r"(?ms)^    permissions:\n(.*?)(?=^    [a-z][a-z-]*:|\Z)", job)
    if declared is None or job.count("    permissions:\n") != 1:
        raise ValueError("eligibility service must declare its job-level token grant")
    rows = [
        line
        for line in declared[1].splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    ]
    scopes = re.findall(r"^      ([a-z-]+): (read|write|none)$", "\n".join(rows), re.M)
    required = {
        "actions": "write",
        "checks": "read",
        "contents": "read",
        "pull-requests": "write",
        "statuses": "write",
    }
    if len(scopes) != len(rows) or len(scopes) != len(required) or dict(scopes) != required:
        raise ValueError("eligibility service job must retain exactly the callee's five scopes")


def require_selected_shared_node_caller(workflow: str, automation_sha: str) -> dict[str, str]:
    """Validate caller source binding/order/cache without inventing a Node authority."""
    if re.fullmatch(r"[a-f0-9]{40}", automation_sha) is None:
        raise ValueError("shared Node caller requires the exact selected immutable source")
    job = workflow_job(workflow, "documentation")
    steps = re.split(r"(?m)(?=^      - )", job)[1:]

    def one(token: str) -> tuple[int, str]:
        matches = [(index, step) for index, step in enumerate(steps) if token in step]
        if len(matches) != 1:
            raise ValueError(f"shared Node flow needs exactly one {token}")
        return matches[0]

    checkout_index, checkout = one("repository: Strukturpiloten/boxferry")
    python_index, python_step = one("uses: ./_automation/.github/actions/shared-python")
    node_index, node_step = one("uses: ./_automation/.github/actions/shared-node")
    npm_index, _npm = one("npm ci --ignore-scripts")
    if not checkout_index < python_index < node_index < npm_index:
        raise ValueError("selected checkout and shared Python must precede shared Node and npm")
    if (
        f"          ref: {automation_sha}\n" not in checkout
        or "          path: _automation\n" not in checkout
    ):
        raise ValueError("shared Node must use the same immutable _automation checkout")
    field_rows = re.findall(r"(?m)^          ([a-z-]+): ([^\n]+)$", node_step)
    fields = dict(field_rows)
    if len(field_rows) != len(fields):
        raise ValueError("shared Node inputs must not repeat keys")
    expected = {
        "automation-sha": automation_sha,
        "cache": "npm",
        "cache-dependency-path": "package-lock.json",
    }
    if fields != expected:
        raise ValueError(
            "shared Node must preserve only selected source, existing npm cache and root lock"
        )
    if re.search(r"(?m)^\s+node-version:", job) or "uses: actions/setup-node@" in job:
        raise ValueError("consumer must not retain an independent Node version or setup route")
    return {"checkout_step": checkout, "python_step": python_step, "node_step": node_step}


def verified_renovate_cache(shared: Path, provided: str | None) -> Path:
    """Verify an explicitly supplied cache with the selected canonical verifier only."""
    from boxferry_ci import renovate_runtime

    if not provided:
        raise ValueError(
            "--native requires BOXFERRY_RENOVATE_PACKAGE; no runtime is acquired implicitly"
        )
    destination = Path(provided)
    if destination.is_symlink() or not destination.is_dir():
        raise ValueError("BOXFERRY_RENOVATE_PACKAGE must be an existing verified cache directory")
    image, version = renovate_runtime._pin(shared / "renovate-runtime.json")
    marker = destination / "source-image.txt"
    if marker.is_symlink() or not marker.is_file():
        raise ValueError("native Renovate cache lacks its canonical immutable image identity")
    with marker.open("rb") as stream:
        identity = stream.read(4097)
    if len(identity) > 4096 or identity.decode("utf-8").strip() != image:
        raise ValueError("native Renovate cache differs from the selected immutable image")
    renovate_runtime._verify(destination, version, renovate_runtime.run_command)
    return destination.absolute()


def declared_devcontainer_images(source: str) -> list[tuple[str, str, str, str | None]]:
    """Validate retained image release annotations and immutable digest declarations."""
    blocks = re.findall(
        r"(?m)^# renovate: datasource=docker depName=(\S+)\n"
        r"# release: (\S+)\n(FROM [^\n]+)$",
        source,
    )
    if len(blocks) != len(re.findall(r"(?m)^FROM ", source)):
        raise ValueError("every image needs an annotated tagged declaration")
    images = []
    for name, tag, declaration in blocks:
        words = declaration.split()
        if len(words) not in {2, 4} or (len(words) == 4 and words[2] != "AS"):
            raise ValueError("invalid image stage declaration")
        prefix = f"{name}@"
        if not words[1].startswith(prefix):
            raise ValueError("legacy image declaration must retain its digest-only form")
        digest = words[1][len(prefix) :]
        if re.fullmatch(r"sha256:[0-9a-f]{64}", digest) is None:
            raise ValueError("image digest must be exact")
        images.append((name, tag, digest, words[3] if len(words) == 4 else None))
    return images


def devcontainer_image_owners(configuration: dict, source: str) -> list[tuple[dict, re.Match[str]]]:
    """Require one regex owner per declared image and disable competing native ownership."""
    if configuration.get("dockerfile", {}).get("enabled") is not False:
        raise ValueError("native Dockerfile extraction would duplicate ownership")
    images = {
        name: (tag, digest) for name, tag, digest, _alias in declared_devcontainer_images(source)
    }
    owners = []
    for manager in configuration["customManagers"]:
        if manager["customType"] != "regex" or not any(
            re.search(pattern[1:-1], ".devcontainer/Dockerfile")
            for pattern in manager["managerFilePatterns"]
        ):
            continue
        for pattern in manager["matchStrings"]:
            translated = re.sub(r"\(\?<([A-Za-z][A-Za-z0-9_]*)>", r"(?P<\1>", pattern)
            for match in re.finditer(translated, source):
                if match.groupdict().get("datasource") != "docker":
                    continue
                if images.get(match["depName"]) != (match["currentValue"], match["currentDigest"]):
                    raise ValueError("extractor identity differs from literal image declaration")
                if manager.get("versioningTemplate") != "docker":
                    raise ValueError("image versions require Docker versioning")
                owners.append((manager, match))
    names = [match["depName"] for _manager, match in owners]
    if len(names) != len(images) or set(names) != set(images):
        raise ValueError("each image must have exactly one owner")
    return owners


class ConsumerContracts(unittest.TestCase):
    def test_release_ci_permission_ceiling_covers_all_explicit_callee_scopes(self) -> None:
        release = (ROOT / ".github/workflows/release.yml").read_text()
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        classifier = workflow_job(ci, "api-release-type")
        self.assertIn("      pull-requests: read\n", classifier)
        require_release_ci_permission_ceiling(release, ci)
        caller = workflow_job(release, "deterministic")
        for replacement in ("", "      pull-requests: none\n"):
            narrowed = release.replace(
                caller, caller.replace("      pull-requests: read\n", replacement, 1), 1
            )
            with self.subTest(replacement=replacement), self.assertRaises(ValueError):
                require_release_ci_permission_ceiling(narrowed, ci)
        expanded_classifier = classifier.replace(
            "    permissions:\n", "    permissions:\n      issues: read\n", 1
        )
        with self.assertRaises(ValueError):
            require_release_ci_permission_ceiling(
                release, ci.replace(classifier, expanded_classifier, 1)
            )

    def test_release_gate_consumes_every_real_prerequisite_receipt(self) -> None:
        policy = shared_release.load_policy(
            ROOT / ".github/release-policy.json", "Strukturpiloten/podman-lens"
        )
        jobs = ["deterministic", "release-metadata", "native-conformance", "semver", "release-gate"]
        self.assertEqual(policy["required_jobs"], jobs)
        self.assertIs(policy["publication_enabled"], True)
        self.assertNotIn("bootstrap_version", policy)
        candidate = "a" * 40
        needs: dict[str, Any] = {
            job: {"result": "success", "outputs": {"candidate_sha": candidate}} for job in jobs
        }
        needs["deterministic"]["outputs"]["deterministic_published_api_candidate_sha"] = candidate
        needs["semver"]["outputs"]["published_api_candidate_sha"] = candidate
        self.assertEqual(shared_release.verify_gate(policy, needs, candidate), candidate)
        for job in jobs:
            broken = dict(needs)
            broken.pop(job)
            with (
                self.subTest(job=job, missing=True),
                self.assertRaises(shared_release.ReleaseError),
            ):
                shared_release.verify_gate(policy, broken, candidate)
            for result in ("failure", "cancelled", "skipped", "", None, "unknown"):
                broken = {**needs, job: {"result": result, "outputs": {"candidate_sha": candidate}}}
                with (
                    self.subTest(job=job, result=result),
                    self.assertRaises(shared_release.ReleaseError),
                ):
                    shared_release.verify_gate(policy, broken, candidate)
            for outputs in (
                None,
                {},
                {"candidate_sha": ""},
                {"candidate_sha": "b" * 40},
                {"candidate_sha": "main"},
                {"candidate_sha": "a" * 39},
                {"candidate_sha": "A" * 40},
                {"candidate_sha": None},
            ):
                broken = {**needs, job: {"result": "success", "outputs": outputs}}
                with (
                    self.subTest(job=job, outputs=outputs),
                    self.assertRaises(shared_release.ReleaseError),
                ):
                    shared_release.verify_gate(policy, broken, candidate)
        with self.assertRaises(shared_release.ReleaseError):
            shared_release.verify_gate(policy, {**needs, "unknown": needs[jobs[0]]}, candidate)

    def test_release_direct_gate_and_read_only_prepare_bind_the_actual_shared_output(self) -> None:
        release = (ROOT / ".github/workflows/release.yml").read_text()
        gate = workflow_job(release, "shared-release-gate")
        self.assertIn(
            "needs: [deterministic, release-metadata, native-conformance, semver, release-gate]",
            gate,
        )
        self.assertIn("if: always()", gate)
        self.assertIn("needs_json: ${{ toJSON(needs) }}", gate)
        self.assertIn("config_path: .github/release-policy.json", gate)
        evidence = workflow_job(release, "release-gate")
        self.assertIn("needs: [deterministic, native-conformance, semver]", evidence)
        self.assertIn("if: needs.native-conformance.result == 'success'", evidence)
        self.assertNotIn("Require every validation prerequisite", evidence)
        self.assertNotIn("DETERMINISTIC:", evidence)
        self.assertLess(
            evidence.index("scripts/validate-native-evidence.sh"), evidence.index("id: binding")
        )
        metadata = workflow_job(release, "release-metadata")
        self.assertIn("candidate_sha: ${{ steps.metadata.outputs.candidate_sha }}", metadata)
        self.assertLess(
            metadata.index("scripts/check-release-metadata.sh --release"),
            metadata.index("printf 'candidate_sha="),
        )
        self.assertLess(
            metadata.index("scripts/extract-release-notes.sh"),
            metadata.index("printf 'candidate_sha="),
        )
        preparation = workflow_job(release, "release-preparation")
        self.assertIn("needs: shared-release-gate", preparation)
        self.assertIn("permissions:\n      contents: read\n", preparation)
        self.assertRegex(preparation, r"validation-only: ['\"]true['\"]")
        self.assertNotIn("inputs.validation_only", preparation)
        for forbidden in (
            "id-token:",
            "secrets.",
            "bootstrap-token:",
            "CARGO_REGISTRY_TOKEN",
            "environment:",
        ):
            self.assertNotIn(forbidden, preparation)
        publish = workflow_job(release, "publish")
        self.assertIn("needs: [shared-release-gate, release-preparation]", publish)
        for required in (
            "github.event_name == 'workflow_dispatch'",
            "github.ref == format('refs/heads/{0}', github.event.repository.default_branch)",
            "inputs.validation_only != true",
            "needs.release-preparation.result == 'success'",
            "environment: release",
            "id-token: write",
            "validation-only: ${{ inputs.validation_only }}",
        ):
            self.assertIn(required, publish)
        for job in (preparation, publish):
            for required in (
                "needs.shared-release-gate.result == 'success'",
                "needs.shared-release-gate.outputs.candidate_sha == github.sha",
                "uses: ./_automation/.github/actions/shared-crate-publish",
                "validation-sha: ${{ needs.shared-release-gate.outputs.candidate_sha }}",
                "validation-result: ${{ needs.shared-release-gate.result }}",
            ):
                self.assertIn(required, job)
            self.assertNotIn("bootstrap-token:", job)
        self.assertIn("release-config: ${{ steps.source.outputs.config }}", preparation)
        self.assertIn("policy-sha: ${{ github.sha }}", preparation)
        self.assertIn("candidate-sha: ${{ github.sha }}", preparation)
        revision = json.loads((ROOT / ".github/automation.json").read_text())["revision"]
        self.assertIn(f"shared-release-validation.yml@{revision}", gate)
        self.assertIn(f"automation_sha: {revision}", gate)
        self.assertIn(f"ref: {revision}", preparation)
        self.assertEqual(preparation.count(f"automation-sha: {revision}"), 2)
        self.assertIn(f"automation-sha: {revision}", publish)

    def test_release_validation_only_inventory_never_contacts_registry_or_mutates(self) -> None:
        candidate = "a" * 40
        manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
        metadata = {
            "workspace_members": ["podman-lens"],
            "packages": [
                {
                    "id": "podman-lens",
                    "name": "podman-lens",
                    "version": manifest["workspace"]["package"]["version"],
                    "manifest_path": str(ROOT / "Cargo.toml"),
                    "publish": ["crates-io"],
                    "dependencies": [],
                }
            ],
        }
        args = [
            "prepare-crate",
            "--repository-root",
            str(ROOT),
            "--config",
            str(ROOT / ".github/release-policy.json"),
            "--candidate-sha",
            candidate,
            "--automation-sha",
            "b" * 40,
            "--package",
            "podman-lens",
            "--validation-sha",
            candidate,
            "--validation-result",
            "success",
            "--validation-only",
            "true",
        ]
        with (
            patch.dict(
                os.environ, {"GITHUB_REPOSITORY": "Strukturpiloten/podman-lens"}, clear=True
            ),
            patch.object(shared_release, "verify_bindings"),
            patch.object(
                shared_release.subprocess, "check_output", return_value=json.dumps(metadata)
            ) as read_metadata,
            patch.object(shared_release, "registry_status") as registry,
            patch.object(shared_release, "publish_crate") as publish,
            patch.object(shared_release.subprocess, "run") as mutate,
            patch("sys.stderr", new_callable=io.StringIO),
        ):
            self.assertEqual(shared_release.main(args), 0)
            self.assertEqual(
                read_metadata.call_args.args[0],
                ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
            )
            for extra in (
                ["--validation-sha", ""],
                ["--validation-sha", "main"],
                ["--validation-sha", "c" * 40],
                ["--validation-result", "failure"],
                ["--validation-result", "skipped"],
                ["--validation-result", "cancelled"],
            ):
                with self.subTest(extra=extra), self.assertRaises(SystemExit):
                    shared_release.main([*args, *extra])
            with self.assertRaises(SystemExit):
                shared_release.main(["publish-crate", *args[1:]])
            registry.assert_not_called()
            publish.assert_not_called()
            mutate.assert_not_called()

    def test_native_reusable_receipt_follows_matrix_cleanup_and_exact_admission(self) -> None:
        native = (ROOT / ".github/workflows/native-podman-conformance.yml").read_text()
        self.assertIn("value: ${{ jobs.native-gate.outputs.candidate_sha }}", native)
        gate = workflow_job(native, "native-gate")
        self.assertIn("needs: [admit-candidate, native-api]", gate)
        self.assertIn("if: always()", gate)
        self.assertIn("permissions: {}", gate)
        self.assertIn("CANDIDATE_SHA: ${{ needs.admit-candidate.outputs.candidate_sha }}", gate)
        matrix = workflow_job(native, "native-api")
        self.assertIn(
            "if: always()", matrix[matrix.index("- name: Remove disposable native service") :]
        )
        self.assertIn('bash scripts/cleanup-native-runtime.sh "${state_key}"', matrix)
        script = gate.split("        run: |\n", 1)[1]
        script = textwrap.dedent(script)
        candidate = "a" * 40
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "output"
            for admission, outcome, receipt, succeeds in (
                ("success", "success", candidate, True),
                ("failure", "success", candidate, False),
                ("success", "failure", candidate, False),
                ("success", "cancelled", candidate, False),
                ("success", "skipped", candidate, False),
                ("success", "", candidate, False),
                ("success", "success", "", False),
                ("success", "success", "b" * 40, False),
                ("success", "success", "main", False),
            ):
                output.write_text("")
                result = subprocess.run(
                    ["bash", "-c", script],
                    capture_output=True,
                    text=True,
                    env={
                        **os.environ,
                        "ADMISSION_RESULT": admission,
                        "NATIVE_RESULT": outcome,
                        "CANDIDATE_SHA": receipt,
                        "GITHUB_SHA": candidate,
                        "GITHUB_OUTPUT": str(output),
                    },
                )
                with self.subTest(admission=admission, outcome=outcome, receipt=receipt):
                    self.assertEqual(result.returncode == 0, succeeds, result.stderr)
                    self.assertEqual(
                        output.read_text(), f"candidate_sha={candidate}\n" if succeeds else ""
                    )

    def test_prose_examples_native_and_unknown_paths(self) -> None:
        for path, content, expected in (
            ("docs/public/acquisition/index.md", b"Plain prose.\n", {"executable-docs"}),
            (
                "docs/public/acquisition/index.md",
                b"```rust\nfn main() {}\n```",
                {"executable-docs"},
            ),
            ("docs/public/acquisition/index.md", b"Call `acquire_inventory`.", {"executable-docs"}),
            ("fixtures/native-regressions/capture.json", b"{}", {"full"}),
            ("scripts/native-runtime-pins.sh", b"changed", {"full"}),
            ("src/inventory.rs", b"changed", {"full"}),
            ("tests/native_release_contract.rs", b"changed", {"full"}),
            ("scripts/validation-policy.json", b"changed", {"full"}),
            (".devcontainer/Dockerfile", b"COPY unknown /new", {"full"}),
            (".devcontainer/devcontainer.json", b"{}", {"full"}),
            (".devcontainer/devcontainer-lock.json", b"{}", {"full"}),
            (".dockerignore", b"changed", {"full"}),
            (".github/automation.json", b"{}", {"full"}),
            ("scripts/install-file-tools.sh", b"changed", {"full"}),
            ("package-lock.json", b"{}", {"full"}),
            ("rust-toolchain.toml", b"changed", {"full"}),
            ("docs/internal/unknown.md", b"prose", {"full"}),
        ):
            with self.subTest(path=path, expected=expected):
                self.assertEqual(path_profiles(path, (content,), POLICY, set()), expected)
        self.assertEqual(POLICY.selected_jobs(("full",)), set(POLICY.jobs))
        self.assertEqual(
            POLICY.selected_jobs(("prose",)), {"documentation", "lockfile-release-age"}
        )
        self.assertFalse(any("cargo" in call for call in POLICY.local_commands["prose"]))
        self.assertIn(
            ("cargo", "test", "--locked", "--test", "public_guides"),
            POLICY.local_commands["executable-docs"],
        )

    def test_public_guide_inventory_changes_keep_the_targeted_contract(self) -> None:
        self.assertEqual(POLICY.executable_documentation_roots, ("docs/public/",))
        for evidence in ((b"New plain guide.\n",), (b"Deleted plain guide.\n",), ()):
            with self.subTest(evidence=evidence):
                self.assertEqual(
                    path_profiles("docs/public/new-guide/index.md", evidence, POLICY, set()),
                    {"executable-docs"},
                )
        self.assertEqual(
            POLICY.selected_jobs(("executable-docs",)),
            {"documentation", "lockfile-release-age"},
        )
        self.assertIn(
            ("cargo", "test", "--locked", "--test", "public_guides"),
            POLICY.local_commands["executable-docs"],
        )
        for command in POLICY.local_commands["executable-docs"]:
            self.assertFalse(
                {"ci-test", "ci-clippy", "llvm-cov", "ci-generators", "devcontainer-local"}
                & set(command)
            )
        hosted = workflow_job((ROOT / ".github/workflows/ci.yml").read_text(), "documentation")
        self.assertIn(".profile == 'executable-docs'", hosted)
        self.assertIn("run: cargo test --locked --test public_guides", hosted)

    def test_ordinary_adoption_preserves_every_main_job_without_image_opt_in(self) -> None:
        expected_jobs = (
            "rust",
            "coverage",
            "msrv",
            "dependencies",
            "api-release-type",
            "api",
            "documentation",
            "lockfile-release-age",
        )
        self.assertEqual(POLICY.jobs, expected_jobs)
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        gate = workflow_job(ci, "pr-gate")
        dependencies = re.search(r"    needs:\n((?:      - [^\n]+\n)+)", gate)
        assert dependencies is not None
        self.assertEqual(
            dependencies[1].splitlines(),
            [f"      - {job}" for job in ("validation-plan", *expected_jobs)],
        )
        self.assertIn("    if: always()\n", gate)
        self.assertIn('--plan-json "${PLAN_JSON}" --needs-json "${NEEDS_JSON}"', gate)
        self.assertNotIn("continue-on-error", gate)
        self.assertNotIn("devcontainer-validation", ci)
        local = (ROOT / "scripts/check-all.sh").read_text()
        self.assertNotIn("devcontainer-local", local)
        self.assertNotIn("--outer-mode", local)
        self.assertIn("readonly total_steps=24", local)
        for event in ("pull_request", "merge_group", "push", "workflow_dispatch", "workflow_call"):
            for profile in (
                ("full", "prose", "executable-docs")
                if event in {"pull_request", "merge_group"}
                else ("full",)
            ):
                plan, needs, comparison = hosted_gate_case(POLICY, event, profile)
                verify_gate(plan, needs, POLICY, comparison, expected_plan=plan)
                for job in ("validation-plan", *expected_jobs):
                    broken = copy.deepcopy(needs)
                    del broken[job]
                    with (
                        self.subTest(event=event, profile=profile, missing=job),
                        self.assertRaises(ContractError),
                    ):
                        verify_gate(plan, broken, POLICY, comparison, expected_plan=plan)
                if profile == "full":
                    for job in expected_jobs:
                        for outcome in ("failure", "cancelled", "skipped"):
                            broken = {**needs, job: {"result": outcome}}
                            with (
                                self.subTest(event=event, job=job, outcome=outcome),
                                self.assertRaises(ContractError),
                            ):
                                verify_gate(plan, broken, POLICY, comparison, expected_plan=plan)
                with self.assertRaises(ContractError):
                    verify_gate(
                        plan,
                        {**needs, "unexpected-image-job": {"result": "success"}},
                        POLICY,
                        comparison,
                        expected_plan=plan,
                    )

    def test_direct_image_inputs_are_held_without_freezing_ordinary_dependencies(self) -> None:
        import boxferry_ci

        shared = Path(boxferry_ci.__file__).resolve().parents[2]
        spec = importlib.util.spec_from_file_location(
            "held_image_presets", shared / "tests/test_presets.py"
        )
        assert spec is not None and spec.loader is not None
        presets = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(presets)
        config = presets.consumer_config(ROOT / ".github/renovate.json")
        source = (ROOT / ".devcontainer/Dockerfile").read_text()
        owners = devcontainer_image_owners(config, source)
        self.assertEqual(len(owners), 2)
        duplicate = copy.deepcopy(config)
        duplicate["customManagers"].append(copy.deepcopy(owners[0][0]))
        with self.assertRaisesRegex(ValueError, "exactly one"):
            devcontainer_image_owners(duplicate, source)
        installer = (ROOT / "scripts/install-file-tools.sh").read_text()
        legacy = []
        for manager in config["customManagers"]:
            if not any(
                re.search(pattern[1:-1], "scripts/install-file-tools.sh")
                for pattern in manager["managerFilePatterns"]
            ):
                continue
            for expression in manager["matchStrings"]:
                translated = re.sub(r"\(\?<([A-Za-z][A-Za-z0-9_]*)>", r"(?P<\1>", expression)
                legacy.extend(re.finditer(translated, installer))
        self.assertEqual(
            {match["depName"] for match in legacy},
            {"tombi-toml/tombi", "mvdan/sh", "koalaman/shellcheck", "hadolint/hadolint"},
        )
        self.assertEqual(len(legacy), 4)
        held = (
            [
                ("custom.regex", "docker", match["depName"], ".devcontainer/Dockerfile")
                for _manager, match in owners
            ]
            + [
                ("custom.regex", "crate", "cargo-deny", ".devcontainer/Dockerfile"),
                (
                    "devcontainer",
                    "github-tags",
                    "ghcr.io/devcontainers/features/github-cli",
                    ".devcontainer/devcontainer.json",
                ),
                ("custom.regex", "github-releases", "cli/cli", ".devcontainer/devcontainer.json"),
            ]
            + [
                (
                    "custom.regex",
                    "github-releases",
                    match["depName"],
                    "scripts/install-file-tools.sh",
                )
                for match in legacy
            ]
        )
        for manager, datasource, package, filename in held:
            effective = presets.effective(
                config,
                {
                    "manager": manager,
                    "datasource": datasource,
                    "packageName": package,
                    "fileName": filename,
                    "updateType": "major",
                },
            )
            with self.subTest(held=package, file=filename):
                self.assertFalse(effective["enabled"])
                self.assertFalse(effective.get("dependencyDashboardApproval", False))
        for manager, datasource, package, filename in (
            (
                "github-actions",
                "github-tags",
                "Strukturpiloten/boxferry",
                ".github/workflows/ci.yml",
            ),
            (
                "custom.regex",
                "github-digest",
                "Strukturpiloten/boxferry",
                ".github/automation.json",
            ),
            ("npm", "npm", "prettier", "package.json"),
            ("custom.regex", "github-tags", "rust-lang/rust", "rust-toolchain.toml"),
            ("cargo", "crate", "serde", "Cargo.toml"),
        ):
            effective = presets.effective(
                config,
                {
                    "manager": manager,
                    "datasource": datasource,
                    "packageName": package,
                    "fileName": filename,
                    "updateType": "major",
                },
            )
            with self.subTest(ordinary=package):
                self.assertNotEqual(effective.get("enabled"), False)
                self.assertTrue(effective["automerge"])
                self.assertFalse(effective["dependencyDashboardApproval"])
        self.assertIn("COPY scripts/install-file-tools.sh /tmp/install-file-tools.sh", source)
        self.assertNotIn("shared-ci.py", source)
        self.assertNotIn("UV_PYTHON_INSTALL_DIR", source)
        self.assertFalse((ROOT / ".devcontainer/devcontainer-lock.json").exists())
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertIn("scripts/install-host-tools.sh", ci)
        for workflow in (ROOT / ".github/workflows").glob("*.yml"):
            self.assertNotIn("scripts/install-file-tools.sh", workflow.read_text())

    def test_rust_coverage_and_complete_local_contract_are_preserved(self) -> None:
        calls = POLICY.task("rust-quality").commands
        for alias in (
            "ci-check",
            "ci-policy",
            "ci-clippy",
            "ci-test",
            "ci-doctest",
            "ci-doc",
            "ci-package",
        ):
            self.assertIn(("cargo", alias), calls)
        self.assertIn(("bash", "scripts/check-native-release-contract.sh"), calls)
        self.assertEqual(
            POLICY.task("coverage").commands,
            (
                ("cargo", "llvm-cov", "clean", "--locked"),
                (
                    "cargo",
                    "llvm-cov",
                    "--locked",
                    "--no-clean",
                    "--workspace",
                    "--all-features",
                    "--all-targets",
                    "--summary-only",
                ),
            ),
        )
        self.assertEqual(
            POLICY.local_commands["full"], (("bash", "scripts/check-all.sh", "--check"),)
        )
        local = (ROOT / "scripts/check-all.sh").read_text()
        self.assertIn("readonly total_steps=24", local)
        for token in (
            "CARGO_TARGET_DIR must be inside this worktree",
            "coverage_target_dir=",
            "semver_target_dir=",
            "cargo ci-clippy",
            "cargo ci-test",
            "cargo package --locked --allow-dirty",
            "cargo deny --all-features check",
            "bash scripts/check-native-release-contract.sh",
            "scripts/shared-ci.py run --task python-quality",
        ):
            self.assertIn(token, local)

    def test_effective_renovate_policy_preserves_automatic_updates(self) -> None:
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
        for dependency in (
            {
                "manager": "custom.regex",
                "datasource": "github-releases",
                "packageName": "containers/podman",
                "fileName": "scripts/native-runtime-pins.sh",
            },
            {
                "manager": "custom.regex",
                "datasource": "docker",
                "packageName": "registry.fedoraproject.org/fedora",
                "fileName": "scripts/native-runtime-pins.sh",
            },
            {
                "manager": "custom.regex",
                "datasource": "github-releases",
                "packageName": "astral-sh/uv",
                "fileName": "scripts/install-host-tools.sh",
            },
            {
                "manager": "devcontainer",
                "datasource": "github-tags",
                "packageName": "ghcr.io/devcontainers/features/github-cli",
                "depType": "feature",
                "fileName": ".devcontainer/devcontainer.json",
            },
        ):
            result = presets.effective(config, {**dependency, "updateType": "major"})
            if (
                dependency["fileName"] == "scripts/native-runtime-pins.sh"
                or dependency.get("depType") == "feature"
            ):
                self.assertFalse(result["enabled"])
            else:
                self.assertTrue(result.get("enabled", True))
                self.assertTrue(result["automerge"])
            self.assertFalse(result["dependencyDashboardApproval"])
            self.assertEqual(result["minimumReleaseAge"], "3 days")

    def test_fixed_native_tuple_extraction_and_exclusion_are_exact(self) -> None:
        import boxferry_ci

        shared = Path(boxferry_ci.__file__).resolve().parents[2]
        spec = importlib.util.spec_from_file_location(
            "fixed_tuple_presets", shared / "tests/test_presets.py"
        )
        assert spec is not None and spec.loader is not None
        presets = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(presets)
        local = json.loads((ROOT / ".github/renovate.json").read_text())
        config = presets.consumer_config(ROOT / ".github/renovate.json")
        fixed = [
            rule
            for rule in local["packageRules"]
            if rule.get("description")
            == ["Keep the coupled Podman native qualification tuple fixed"]
        ]
        self.assertEqual(len(fixed), 1)
        self.assertEqual(
            fixed[0],
            {
                "description": ["Keep the coupled Podman native qualification tuple fixed"],
                "matchManagers": ["custom.regex"],
                "matchFileNames": ["scripts/native-runtime-pins.sh"],
                "matchDatasources": ["github-releases", "docker"],
                "matchPackageNames": ["containers/podman", "registry.fedoraproject.org/fedora"],
                "enabled": False,
            },
        )
        pins = (ROOT / "scripts/native-runtime-pins.sh").read_text()
        extracted = []
        for manager in local["customManagers"]:
            if manager.get("managerFilePatterns") != ["/^scripts/native-runtime-pins\\.sh$/"]:
                continue
            for regex in manager["matchStrings"]:
                pattern = re.sub(r"\(\?<([a-zA-Z][a-zA-Z0-9_]*)>", r"(?P<\1>", regex)
                extracted.extend(match.groupdict() for match in re.finditer(pattern, pins))
        self.assertEqual(len(extracted), 2)
        self.assertEqual(
            {item["depName"] for item in extracted},
            {"containers/podman", "registry.fedoraproject.org/fedora"},
        )
        for item in extracted:
            dependency = {
                "manager": "custom.regex",
                "datasource": item["datasource"],
                "packageName": item["depName"],
                "fileName": "scripts/native-runtime-pins.sh",
                "updateType": "digest",
            }
            self.assertFalse(presets.effective(config, dependency)["enabled"])
            self.assertTrue(
                presets.effective(
                    config, {**dependency, "fileName": "scripts/unrelated-runtime.sh"}
                ).get("enabled", True)
            )
            self.assertTrue(
                presets.effective(config, {**dependency, "packageName": "unrelated/runtime"}).get(
                    "enabled", True
                )
            )
            self.assertTrue(
                presets.effective(config, {**dependency, "datasource": "github-tags"}).get(
                    "enabled", True
                )
            )

    def test_shared_node_caller_uses_same_selected_python_and_existing_cache(self) -> None:
        revision = json.loads((ROOT / ".github/automation.json").read_text())["revision"]
        flow = require_selected_shared_node_caller(
            (ROOT / ".github/workflows/ci.yml").read_text(), revision
        )
        self.assertIn("cache: npm", flow["node_step"])
        self.assertIn("cache-dependency-path: package-lock.json", flow["node_step"])
        self.assertTrue((ROOT / "package-lock.json").is_file())
        self.assertNotIn("node-version:", flow["node_step"])

    def test_shared_node_caller_rejects_source_order_version_and_cache_drift(self) -> None:
        revision = json.loads((ROOT / ".github/automation.json").read_text())["revision"]
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        flow = require_selected_shared_node_caller(workflow, revision)
        node_step = flow["node_step"]
        for changed in (
            node_step.replace(revision, "b" * 40),
            node_step.replace(f"          automation-sha: {revision}\n", ""),
            node_step.replace("        with:\n", "        with:\n          node-version: 99.0.0\n"),
            node_step.replace("cache: npm", 'cache: ""'),
            node_step.replace(
                "package-lock.json", "_automation/automation/devcontainer/package-lock.json"
            ),
            node_step + node_step,
        ):
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                require_selected_shared_node_caller(
                    workflow.replace(node_step, changed, 1), revision
                )
        adjacent = flow["python_step"] + node_step
        self.assertIn(adjacent, workflow)
        with self.assertRaises(ValueError):
            require_selected_shared_node_caller(
                workflow.replace(adjacent, node_step + flow["python_step"], 1), revision
            )
        job = workflow_job(workflow, "documentation")
        changed_job = job.replace(
            flow["checkout_step"], flow["checkout_step"].replace(revision, "b" * 40), 1
        )
        with self.assertRaises(ValueError):
            require_selected_shared_node_caller(workflow.replace(job, changed_job, 1), revision)

    def test_unique_native_and_local_python_pin_ownership(self) -> None:
        config = json.loads((ROOT / ".github/renovate.json").read_text())
        managers = config["customManagers"]
        for marker in ("PODMAN_NATIVE_VERSION", "PODMAN_NATIVE_BASE_IMAGE"):
            self.assertEqual(sum(marker in json.dumps(manager) for manager in managers), 1)
        installer = (ROOT / "scripts/install-host-tools.sh").read_text()
        self.assertNotIn("uv_version", installer)
        with (ROOT / "pyproject.toml").open("rb") as stream:
            tools = tomllib.load(stream)
        self.assertEqual(set(tools), {"tool"})
        self.assertEqual(set(tools["tool"]), {"ruff", "ty"})
        self.assertNotIn("mypy", (ROOT / "pyproject.toml").read_text())

    def check_shared_node_caller_has_no_independent_node_version_owner(self) -> None:
        """Explicit --native check; ordinary caller tests never acquire this runtime."""
        import boxferry_ci

        shared = Path(boxferry_ci.__file__).resolve().parents[2]
        runtime = verified_renovate_cache(shared, os.environ.get("BOXFERRY_RENOVATE_PACKAGE"))
        node = shutil.which("node")
        assert node is not None
        script = r"""
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
(async () => {
  const [runtime, root, shared] = process.argv.slice(1);
  const pin = JSON.parse(fs.readFileSync(path.join(shared, "automation/renovate-runtime.json")));
  const expected = pin.image.match(/^ghcr\.io\/renovatebot\/renovate:([0-9]+\.[0-9]+\.[0-9]+)@sha256:[a-f0-9]{64}$/);
  assert(expected);
  assert.equal(JSON.parse(fs.readFileSync(path.join(runtime, "package.json"))).version, expected[1]);
  const base = path.join(runtime, "dist");
  await require(path.join(base, "logger/index.js")).init();
  assert.equal(require(path.join(base, "util/regex.js")).regexEngineStatus.type, "available");
  require(path.join(base, "config/global.js")).GlobalConfig.set({ localDir: root });
  const memory = require(path.join(base, "util/cache/memory/index.js")); memory.init();
  const raw = JSON.parse(fs.readFileSync(path.join(root, ".github/renovate.json")));
  const revision = JSON.parse(fs.readFileSync(path.join(root, ".github/automation.json"))).revision;
  for (const reference of raw.extends) {
    const match = reference.match(/^github>Strukturpiloten\/boxferry\/\/renovate-presets\/(common|rust)#([a-f0-9]{40})$/);
    assert(match && match[2] === revision);
    memory.set("preset:" + reference, JSON.parse(fs.readFileSync(path.join(shared, "renovate-presets", match[1] + ".json"))));
  }
  const { validateConfig } = require(path.join(base, "config/validation.js"));
  const validation = await validateConfig("repository", raw);
  assert.deepEqual(validation.errors, []);
  assert.deepEqual(validation.warnings, []);
  const { config } = await require(path.join(base, "config/presets/index.js")).resolveConfigPresets(raw);
  const native = require(path.join(base, "modules/manager/github-actions/index.js"));
  const regex = require(path.join(base, "modules/manager/custom/regex/index.js"));
  const { matchRegexOrGlobList } = require(path.join(base, "util/string-match.js"));
  const { applyPackageRules } = require(path.join(base, "util/package-rules/index.js"));
  const file = ".github/workflows/ci.yml";
  const source = fs.readFileSync(path.join(root, file), "utf8");
  const isNode = (dep) => dep.depName === "node" || dep.packageName === "actions/node-versions";
  async function verify(content) {
    assert(content.includes("uses: ./_automation/.github/actions/shared-node"));
    assert(!/^\s+node-version:/m.test(content));
    const extracted = await native.extractPackageFile(content, file, config["github-actions"] ?? {});
    const nativeRows = (extracted?.deps ?? []).filter(isNode);
    const customRows = [];
    for (const manager of config.customManagers ?? []) {
      if (manager.customType !== "regex" || !matchRegexOrGlobList(file, manager.managerFilePatterns)) continue;
      for (const dep of regex.extractPackageFile(content, file, manager)?.deps ?? []) {
        if (!isNode(dep)) continue;
        const effective = await applyPackageRules({ ...config, ...manager, ...dep, manager: "custom.regex", packageFile: file, updateType: "patch" }, "consumer Node inactivity");
        customRows.push({ dependency: dep, enabled: effective.enabled !== false });
      }
    }
    assert.equal(nativeRows.length, 0, "local composite caller must have no independent native Node version row");
    assert.equal(customRows.length, 0, "local composite caller must have no independent custom Node version row");
    return { nativeRows, customRows };
  }
  const actual = await verify(source);
  const literal = source + "\n  forbidden-node-fixture:\n    runs-on: ubuntu-24.04\n    steps:\n      - uses: actions/setup-node@" + "a".repeat(40) + "\n        with:\n          node-version: 99.0.0\n";
  await assert.rejects(verify(literal));
  console.log(JSON.stringify({ runtime: pin.image, consumerRevision: revision, declaration: file, consumerNodeRows: actual, producerCompositeLookupProof: "separate central593 qualification; no fabricated skipReason or consumer dynamic row" }));
})().catch((error) => { console.error(error); process.exit(1); });
"""
        result = subprocess.run(
            [node, "-e", script, str(runtime), str(ROOT), str(shared.parent)],
            capture_output=True,
            text=True,
            check=False,
            timeout=120,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        proof = json.loads(result.stdout)
        self.assertEqual(proof["consumerNodeRows"], {"nativeRows": [], "customRows": []})

    def test_native_owner_check_requires_an_explicit_existing_cache(self) -> None:
        import boxferry_ci
        from boxferry_ci import renovate_runtime

        shared = Path(boxferry_ci.__file__).resolve().parents[2]
        with (
            tempfile.TemporaryDirectory(prefix="podman native cache boundary ") as temporary,
            patch.object(renovate_runtime, "_verify") as verify,
            patch.object(renovate_runtime, "run_command") as command,
        ):
            for provided in (None, str(Path(temporary) / "absent"), temporary):
                with self.subTest(provided=provided), self.assertRaises(ValueError):
                    verified_renovate_cache(shared, provided)
            (Path(temporary) / "source-image.txt").write_text("untrusted image identity\n")
            with self.assertRaises(ValueError):
                verified_renovate_cache(shared, temporary)
            verify.assert_not_called()
            command.assert_not_called()
        environment = dict(os.environ)
        environment.pop("BOXFERRY_RENOVATE_PACKAGE", None)
        result = subprocess.run(
            [sys.executable, str(Path(__file__).resolve()), "--native"],
            env=environment,
            capture_output=True,
            text=True,
            check=False,
            timeout=15,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--native requires BOXFERRY_RENOVATE_PACKAGE", result.stderr)

    def test_file_tool_wrapper_forwards_destination_and_preserves_failure(self) -> None:
        with tempfile.TemporaryDirectory(prefix="podman tools ") as temporary:
            root = Path(temporary)
            scripts = root / "scripts"
            scripts.mkdir()
            wrapper = scripts / "install-host-tools.sh"
            shutil.copyfile(ROOT / "scripts/install-host-tools.sh", wrapper)
            fake_bin = root / "bin"
            fake_bin.mkdir()
            interpreter = fake_bin / "python3"
            interpreter.write_text(
                f"#!{sys.executable}\n"
                "import json, os, sys\n"
                "print(json.dumps(sys.argv[1:]))\n"
                "raise SystemExit(int(os.environ.get('TOOL_STATUS', '0')))\n"
            )
            interpreter.chmod(0o755)
            environment = {**os.environ, "PATH": str(fake_bin) + os.pathsep + os.environ["PATH"]}
            for arguments, destination in (
                ([], "/usr/local/bin"),
                ([str(root / "destination spaces")], str(root / "destination spaces")),
            ):
                with self.subTest(arguments=arguments):
                    result = subprocess.run(
                        ["bash", str(wrapper), *arguments],
                        env=environment,
                        capture_output=True,
                        text=True,
                        check=False,
                        timeout=10,
                    )
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(
                        json.loads(result.stdout),
                        [
                            str(scripts / "shared-ci.py"),
                            "tools",
                            "--destination",
                            destination,
                            "--profile",
                            "core",
                        ],
                    )
            failed = subprocess.run(
                ["bash", str(wrapper)],
                env={**environment, "TOOL_STATUS": "23"},
                capture_output=True,
                text=True,
                check=False,
                timeout=10,
            )
            self.assertEqual(failed.returncode, 23)
            invalid = subprocess.run(
                ["bash", str(wrapper), "one", "two"],
                env=environment,
                capture_output=True,
                text=True,
                check=False,
                timeout=10,
            )
            self.assertEqual(invalid.returncode, 2)
            self.assertEqual(invalid.stdout, "")

    def test_trusted_service_and_candidate_receipt_reader_have_separate_authority(self) -> None:
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        reader = workflow.split("  lockfile-release-age:\n", 1)[1].split("  pr-gate:\n", 1)[0]
        for required in (
            "actions: read",
            "contents: read",
            "statuses: read",
            "shared-dependency-wait.yml@",
            "candidate_sha: ${{ github.sha }}",
            "automation_sha:",
            "policy_sha: ${{ github.event.pull_request.base.sha || github.event.merge_group.base_sha || github.sha }}",
        ):
            self.assertIn(required, reader)
        for forbidden in (
            "actions: write",
            "statuses: write",
            "steps:",
            "boxferry_ci dependencies",
            "boxferry_ci.eligibility service",
            "boxferry_ci.eligibility wait",
        ):
            self.assertNotIn(forbidden, reader)
        service = (ROOT / ".github/workflows/dependency-eligibility.yml").read_text()
        require_eligibility_service_permissions(service)
        for before, after in (
            ("permissions: {}\n", "permissions: write-all\n"),
            ("permissions: {}\n", "permissions:\n  contents: read\n"),
            ("      pull-requests: write\n", "      pull-requests: read\n"),
            ("      checks: read\n", ""),
            ("      statuses: write\n", "      statuses: write\n\n      issues: write\n"),
        ):
            changed = service.replace(before, after, 1)
            self.assertNotEqual(changed, service)
            with self.subTest(before=before), self.assertRaises(ValueError):
                require_eligibility_service_permissions(changed)
        for required in (
            "schedule:",
            "workflow_run:",
            "workflows: [CI]",
            "types: [requested, completed]",
            "actions: write",
            "contents: read",
            "checks: read",
            "pull-requests: write",
            "statuses: write",
            "shared-dependency-eligibility.yml@",
            "automation-sha:",
        ):
            self.assertIn(required, service)
        for forbidden in ("steps:", "pull_request:", "pull_request_target:"):
            self.assertNotIn(forbidden, service)

    def test_admission_uses_trusted_base_A_while_candidate_proposes_B(self) -> None:
        import boxferry_ci
        from boxferry_ci.admission import AdmissionContext, AdmissionError, resolve_admission

        shared = Path(boxferry_ci.__file__).resolve().parents[2]
        spec = importlib.util.spec_from_file_location(
            "shared_admission", shared / "tests/test_admission.py"
        )
        assert spec is not None and spec.loader is not None
        fixture = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(fixture)
        implementation, trusted = Path("/fixture/proposed-B"), Path("/fixture/trusted-base")
        for event in ("pull_request", "merge_group"):
            git = fixture.GitFixture(implementation, trusted)
            context = AdmissionContext(
                event=event,
                repository="Strukturpiloten/podman-lens",
                candidate_sha=fixture.CANDIDATE,
                automation_sha=fixture.PROPOSED_AUTOMATION,
                policy_sha=fixture.POLICY,
                tested_sha=fixture.CANDIDATE,
                pr_base_sha=fixture.POLICY if event == "pull_request" else "",
                queue_base_sha=fixture.POLICY if event == "merge_group" else "",
            )
            # The candidate's proposed manifest is deliberately B; only the trusted Git blob authorizes A.
            git.files[implementation] = {
                fixture.MANIFEST_PATH: (
                    b"100644",
                    fixture.manifest(revision=fixture.PROPOSED_AUTOMATION),
                )
            }
            selection = resolve_admission(context, implementation, trusted, reader=git)
            self.assertEqual(selection.status, "required")
            self.assertEqual(selection.policy_automation_sha, fixture.OLD_AUTOMATION)
            self.assertEqual(selection.proposed_automation_sha, fixture.PROPOSED_AUTOMATION)
            self.assertEqual(selection.candidate_sha, fixture.CANDIDATE)
            self.assertFalse(
                any(root == implementation and args[0] != "rev-parse" for root, args in git.calls)
            )
            del git.files[trusted][fixture.MANIFEST_PATH]
            with self.assertRaisesRegex(AdmissionError, "prerequisite"):
                resolve_admission(context, implementation, trusted, reader=git)
            git.files[trusted][fixture.MANIFEST_PATH] = (b"120000", fixture.manifest())
            with self.assertRaisesRegex(AdmissionError, "regular tracked file"):
                resolve_admission(context, implementation, trusted, reader=git)
        for event in ("push", "workflow_dispatch", "workflow_call"):
            git = fixture.GitFixture(implementation, trusted)
            context = AdmissionContext(
                event=event,
                repository="Strukturpiloten/podman-lens",
                candidate_sha=fixture.CANDIDATE,
                automation_sha=fixture.PROPOSED_AUTOMATION,
                policy_sha=fixture.CANDIDATE,
                tested_sha=fixture.CANDIDATE,
            )
            selection = resolve_admission(context, implementation, trusted, reader=git)
            self.assertEqual(selection.status, "not-applicable")
            self.assertIsNone(selection.policy_automation_sha)
            self.assertFalse(any(root == trusted for root, _ in git.calls))

    def test_interface1_frontloader_A_delegates_to_selected_core_B(self) -> None:
        spec = importlib.util.spec_from_file_location(
            "podman_frontloader_A", ROOT / "scripts/shared-ci.py"
        )
        assert spec is not None and spec.loader is not None
        loader = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(loader)
        self.assertEqual(loader.INTERFACE, 1)
        self.assertEqual(loader.REPOSITORY, "Strukturpiloten/boxferry")
        with tempfile.TemporaryDirectory(prefix="podman-interface1-") as temporary:
            consumer = Path(temporary) / "consumer"
            (consumer / ".github").mkdir(parents=True)
            manifest = {
                "schema": 1,
                "repository": "Strukturpiloten/boxferry",
                "revision": "b" * 40,
                "policy": "scripts/validation-policy.json",
                "interface": 1,
            }
            config = consumer / ".github/automation.json"
            config.write_text(json.dumps(manifest))
            project = Path(temporary) / "verified-project-B/automation"
            core = project / "src/boxferry_ci/bootstrap.py"
            core.parent.mkdir(parents=True)
            core.write_text(
                "import json\n"
                "def run_project(repository_root, project, arguments):\n"
                "    record = {'selected_core': 'B', 'project': str(project), 'argv': arguments}\n"
                "    (repository_root / 'delegation.json').write_text(json.dumps(record))\n"
                "    return 29\n"
            )
            with (
                patch.object(loader, "__file__", str(consumer / "scripts/shared-ci.py")),
                patch.object(loader, "prepare_implementation", return_value=project) as verified,
                patch("sys.stderr", new_callable=io.StringIO) as diagnostic,
            ):
                self.assertEqual(loader.main(["plan"]), 29)
                verified.assert_called_once_with(consumer, "b" * 40)
                record = json.loads((consumer / "delegation.json").read_text())
                self.assertEqual(record["selected_core"], "B")
                self.assertEqual(record["project"], str(project))
                self.assertEqual(record["argv"], ["plan"])
                core.write_text("# This proposed core does not implement interface 1.\n")
                self.assertEqual(loader.main(["plan"]), 2)
                verified.reset_mock()
                manifest["interface"] = 2
                config.write_text(json.dumps(manifest))
                self.assertEqual(loader.main(["plan"]), 2)
                verified.assert_not_called()
                self.assertIn("lacks interface-1 run_project", diagnostic.getvalue())
                self.assertIn("unsupported shared automation interface", diagnostic.getvalue())

    def test_interface1_archive_retains_exact_cargo_metadata_as_nonexecuting_data(self) -> None:
        spec = importlib.util.spec_from_file_location(
            "podman_archive_frontloader", ROOT / "scripts/shared-ci.py"
        )
        assert spec is not None and spec.loader is not None
        loader = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(loader)
        revision = "b" * 40
        with tempfile.TemporaryDirectory(prefix="podman-archive-metadata-") as temporary:
            root = Path(temporary)
            archive = shared_archive_fixture()

            def git(_directory: Path, *arguments: str, binary: bool = False) -> bytes | str:
                if arguments[0] == "archive":
                    self.assertTrue(binary)
                    self.assertEqual(
                        arguments,
                        (
                            "archive",
                            "--format=tar",
                            revision,
                            "automation",
                            "renovate-presets",
                            ".devcontainer/Dockerfile",
                        ),
                    )
                    return archive
                return revision if arguments[0] == "rev-parse" else ""

            with patch.object(loader, "run_git", side_effect=git):
                project = loader.prepare_implementation(root, revision)
            self.assertTrue((project / "pyproject.toml").is_file())
            self.assertTrue((project / "uv.lock").is_file())
            self.assertTrue((project.parent / "renovate-presets/common.json").is_file())
            metadata = project.parent / ".devcontainer/Dockerfile"
            self.assertEqual(metadata.read_bytes(), b"ARG CARGO_DENY_VERSION=9.8.7\nRUN false\n")
            self.assertEqual(metadata.stat().st_mode & 0o777, 0o644)
            self.assertEqual(list(metadata.parent.iterdir()), [metadata])

    def test_interface1_archive_rejects_missing_foreign_and_linked_metadata(self) -> None:
        spec = importlib.util.spec_from_file_location(
            "podman_archive_negative_frontloader", ROOT / "scripts/shared-ci.py"
        )
        assert spec is not None and spec.loader is not None
        loader = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(loader)
        revision = "b" * 40
        for path, kind, expected in (
            (None, tarfile.REGTYPE, "no regular Cargo tool metadata"),
            (".devcontainer/devcontainer.json", tarfile.REGTYPE, "unsafe member"),
            (".devcontainer/Dockerfile", tarfile.SYMTYPE, "unsafe member"),
            (".devcontainer/../Dockerfile", tarfile.REGTYPE, "unsafe member"),
        ):
            with self.subTest(path=path, kind=kind), tempfile.TemporaryDirectory() as temporary:
                archive = shared_archive_fixture(path, kind)

                def git(_directory: Path, *arguments: str, binary: bool = False) -> bytes | str:
                    if binary:
                        return archive
                    return revision if arguments[0] == "rev-parse" else ""

                with (
                    patch.object(loader, "run_git", side_effect=git),
                    self.assertRaisesRegex(loader.BootstrapError, expected),
                ):
                    loader.prepare_implementation(Path(temporary), revision)

    def test_selected_core_owns_its_uv_and_locked_project_execution(self) -> None:
        from boxferry_ci import bootstrap

        consumer, selected = Path("/fixture/consumer"), Path("/fixture/implementation-B/automation")
        with (
            patch.object(
                bootstrap, "load_config", return_value={"policy": "scripts/validation-policy.json"}
            ),
            patch.object(bootstrap, "ensure_uv", return_value=Path("/fixture/verified-B/uv")) as uv,
            patch.object(bootstrap.subprocess, "call", return_value=31) as execute,
        ):
            self.assertEqual(bootstrap.run_project(consumer, selected, ["plan"]), 31)
        uv.assert_called_once_with(selected, consumer / "target/boxferry-automation/tools")
        argv = execute.call_args.args[0]
        self.assertEqual(argv[0], "/fixture/verified-B/uv")
        self.assertEqual(argv[argv.index("--project") + 1], str(selected))
        self.assertIn("--locked", argv)
        self.assertEqual(
            argv[-5:],
            [
                "plan",
                "--repository-root",
                str(consumer),
                "--config",
                str(consumer / "scripts/validation-policy.json"),
            ],
        )

    def test_all_shared_references_bind_one_immutable_revision(self) -> None:
        manifest = json.loads((ROOT / ".github/automation.json").read_text())
        self.assertEqual(set(manifest), {"schema", "repository", "revision", "policy", "interface"})
        self.assertEqual(manifest["repository"], "Strukturpiloten/boxferry")
        self.assertEqual(manifest["schema"], 1)
        self.assertEqual(manifest["interface"], 1)
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
        for path in (ROOT / ".github/workflows").glob("*.yml"):
            references.extend(
                re.findall(
                    r"Strukturpiloten/boxferry/\.github/workflows/shared-[^@\s]+@([^\s]+)",
                    path.read_text(),
                )
            )
        self.assertEqual(
            {
                match
                for path in (ROOT / ".github/workflows").glob("*.yml")
                for match in re.findall(
                    r"boxferry/\.github/workflows/(shared-[^@\s]+)@", path.read_text()
                )
            },
            {
                "shared-rust-ci.yml",
                "shared-rust-dependencies.yml",
                "shared-rust-semver.yml",
                "shared-dependency-wait.yml",
                "shared-dependency-eligibility.yml",
                "shared-release-prepare.yml",
                "shared-release-validation.yml",
            },
        )
        self.assertGreaterEqual(len(references), 8)
        self.assertTrue(all(reference == manifest["revision"] for reference in references))
        # Checkout refs and explicit callee inputs must advance with the manifest/presets.
        # Reusable workflow uses alone do not bind the code executed by local/shared loaders.
        for path in (ROOT / ".github/workflows").glob("*.yml"):
            source = path.read_text()
            bindings = re.findall(
                r"(?m)^\s*(?:automation_sha|automation-sha|AUTOMATION_SHA|ref): "
                r"([a-f0-9]{40})(?:\s|$)",
                source,
            )
            with self.subTest(workflow=path.name):
                self.assertTrue(all(binding == manifest["revision"] for binding in bindings))


class MaintenanceProsePlanningContract(unittest.TestCase):
    """Real temporary Git evidence; no operational Git writes, tools or runtimes."""

    @staticmethod
    def fixture_git(root: Path, *arguments: str) -> str:
        if not root.name.startswith("podman-maintenance-prose-"):
            raise ValueError("Git fixture commands must stay in their owned temporary repository")
        environment = {
            key: value for key, value in os.environ.items() if not key.startswith("GIT_")
        }
        environment.update(
            GIT_CONFIG_NOSYSTEM="1",
            GIT_CONFIG_GLOBAL=os.devnull,
            GIT_CONFIG_SYSTEM=os.devnull,
            GIT_TERMINAL_PROMPT="0",
            GIT_AUTHOR_NAME="Podman fixture",
            GIT_AUTHOR_EMAIL="fixture@example.invalid",
            GIT_COMMITTER_NAME="Podman fixture",
            GIT_COMMITTER_EMAIL="fixture@example.invalid",
            GIT_AUTHOR_DATE="2000-01-01T00:00:00+00:00",
            GIT_COMMITTER_DATE="2000-01-01T00:00:00+00:00",
        )
        result = subprocess.run(
            [
                "git",
                "--no-replace-objects",
                "--literal-pathspecs",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "commit.gpgSign=false",
                "-c",
                "gc.auto=0",
                "-c",
                "maintenance.auto=false",
                "-c",
                "protocol.file.allow=never",
                *arguments,
            ],
            cwd=root,
            env=environment,
            capture_output=True,
            text=True,
            check=True,
            timeout=10,
        )
        return result.stdout.strip()

    @contextmanager
    def repository(self, *, omit: str | None = None) -> Generator[tuple[Path, Policy, str]]:
        with tempfile.TemporaryDirectory(prefix="podman-maintenance-prose-") as directory:
            root = Path(directory)
            for name in (
                "README.md",
                "docs/README.md",
                "CONTRIBUTING.md",
                "src/lib.rs",
                "docs/public/acquisition/index.md",
                "scripts/validation-policy.json",
            ):
                if name == omit:
                    continue
                target = root / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes((ROOT / name).read_bytes())
            self.fixture_git(root, "init", "--template=", "--initial-branch=main")
            base = self.commit(root)
            policy = load_policy(root / "scripts/validation-policy.json")
            yield root, policy, base

    def commit(self, root: Path) -> str:
        self.fixture_git(root, "add", "--", ".")
        self.fixture_git(root, "commit", "--no-gpg-sign", "-m", "Authored planner fixture")
        return self.fixture_git(root, "rev-parse", "HEAD")

    def candidate(self, root: Path, base: str, event: str) -> tuple[str, str]:
        head = self.commit(root)
        if event in {"pull_request", "merge_group"}:
            self.fixture_git(root, "switch", "--detach", base)
            self.fixture_git(root, "merge", "--no-ff", "--no-edit", head)
        return head, self.fixture_git(root, "rev-parse", "HEAD")

    @staticmethod
    def plan(
        root: Path,
        policy: Policy,
        base: str,
        head: str,
        tested: str,
        event: str,
        *,
        complete: bool = False,
    ) -> dict[str, object]:
        return make_plan(
            root,
            policy,
            event=event,
            tested_sha=tested,
            base_sha=base,
            head_sha=head,
            policy_sha=base,
            base_ref=base,
            force_full=complete,
        )

    @staticmethod
    def outcomes(plan: dict[str, object], policy: Policy) -> dict[str, dict[str, Any]]:
        jobs = plan["jobs"]
        if not isinstance(jobs, dict):
            raise ValueError("real planner did not return a job map")
        needs: dict[str, dict[str, Any]] = {"validation-plan": {"result": "success"}}
        for job in policy.jobs:
            needs[job] = {"result": "success" if jobs[job] else "skipped"}
        return needs

    def require_real_gate(
        self,
        root: Path,
        policy: Policy,
        base: str,
        head: str,
        tested: str,
        event: str,
        plan: dict[str, object],
    ) -> None:
        expected = self.plan(root, policy, base, head, tested, event)
        self.assertIsNot(expected, plan)
        comparison = Comparison(event, str(plan["base_sha"]), str(plan["head_sha"]), tested)
        verify_gate(plan, self.outcomes(plan, policy), policy, comparison, expected_plan=expected)

    def test_actual_maintenance_prose_reaches_pr_queue_and_local_narrow_plans(self) -> None:
        self.assertEqual(POLICY.path_profiles, {"prose": ("README.md", "docs/README.md")})
        for name in ("README.md", "docs/README.md"):
            for event in ("pull_request", "merge_group", "local"):
                modes = ("committed", "staged", "unstaged") if event == "local" else ("committed",)
                for mode in modes:
                    with (
                        self.subTest(name=name, event=event, mode=mode),
                        self.repository() as fixture,
                    ):
                        root, policy, base = fixture
                        path = root / name
                        path.write_bytes(
                            path.read_bytes() + b"\nAuthored maintenance prose sentence.\n"
                        )
                        if mode == "committed":
                            head, tested = self.candidate(root, base, event)
                        else:
                            if mode == "staged":
                                self.fixture_git(root, "add", "--", name)
                            head = tested = base
                        plan = self.plan(root, policy, base, head, tested, event)
                        self.assertEqual(plan["profiles"], ["prose"])
                        jobs = plan["jobs"]
                        assert isinstance(jobs, dict)
                        self.assertEqual(
                            {job for job, selected in jobs.items() if selected},
                            {"documentation", "lockfile-release-age"},
                        )
                        self.assertFalse(jobs["rust"])
                        self.assertFalse(jobs["coverage"])
                        self.assertNotIn("devcontainer-validation", jobs)
                        self.require_real_gate(root, policy, base, head, tested, event, plan)

    def test_changed_code_unknown_mixed_and_ambiguous_content_select_full(self) -> None:
        for case in ("changed-code", "ambiguous", "unknown", "unlisted", "mixed"):
            for event in ("pull_request", "merge_group", "local"):
                with self.subTest(case=case, event=event), self.repository() as fixture:
                    root, policy, base = fixture
                    path = root / "README.md"
                    before = path.read_bytes()
                    if case == "changed-code":
                        path.write_bytes(before + b"\n```sh\nprintf fixture\n```\n")
                    elif case == "ambiguous":
                        path.write_bytes(before + b"\n> Ambiguous nested maintenance prose.\n")
                    elif case == "unknown":
                        unknown = root / "docs/internal/unknown.md"
                        unknown.parent.mkdir(parents=True)
                        unknown.write_text("Unknown prose path.\n")
                    elif case == "unlisted":
                        path = root / "CONTRIBUTING.md"
                        path.write_bytes(path.read_bytes() + b"\nUnlisted maintenance prose.\n")
                    else:
                        path.write_bytes(before + b"\nMaintenance prose.\n")
                        code = root / "src/lib.rs"
                        code.write_bytes(code.read_bytes() + b"\n// Mixed source fixture.\n")
                    head, tested = self.candidate(root, base, event)
                    plan = self.plan(root, policy, base, head, tested, event)
                    self.assertEqual(plan["profiles"], ["full"])
                    self.assertEqual(plan["jobs"], dict.fromkeys(policy.jobs, True))
                    self.require_real_gate(root, policy, base, head, tested, event, plan)

    def test_missing_before_or_after_maintenance_blob_remains_full(self) -> None:
        for operation in ("add", "delete"):
            for event in ("pull_request", "merge_group", "local"):
                omitted = "docs/README.md" if operation == "add" else None
                with (
                    self.subTest(operation=operation, event=event),
                    self.repository(omit=omitted) as fixture,
                ):
                    root, policy, base = fixture
                    target = root / "docs/README.md"
                    if operation == "add":
                        target.write_bytes((ROOT / "docs/README.md").read_bytes())
                    else:
                        target.unlink()
                    head, tested = self.candidate(root, base, event)
                    plan = self.plan(root, policy, base, head, tested, event)
                    self.assertEqual(plan["profiles"], ["full"])
                    self.require_real_gate(root, policy, base, head, tested, event, plan)

    def test_local_staged_code_cannot_hide_behind_prose_worktree(self) -> None:
        with self.repository() as (root, policy, base):
            path = root / "README.md"
            before = path.read_bytes()
            path.write_bytes(before + b"\n```sh\nprintf staged_code\n```\n")
            self.fixture_git(root, "add", "--", "README.md")
            path.write_bytes(before + b"\nOnly worktree prose remains visible.\n")
            plan = self.plan(root, policy, base, base, base, "local")
            self.assertEqual(plan["profiles"], ["full"])
            self.require_real_gate(root, policy, base, base, base, "local", plan)

    def test_queue_plan_uses_combined_tested_tree_and_public_guides_stay_targeted(self) -> None:
        with self.repository() as (root, policy, base):
            path = root / "README.md"
            path.write_bytes(path.read_bytes() + b"\nProposed maintenance prose.\n")
            head = self.commit(root)
            self.fixture_git(root, "switch", "--detach", base)
            code = root / "src/lib.rs"
            code.write_bytes(code.read_bytes() + b"\n// Other queued source fixture.\n")
            self.commit(root)
            self.fixture_git(root, "merge", "--no-ff", "--no-edit", head)
            tested = self.fixture_git(root, "rev-parse", "HEAD")
            plan = self.plan(root, policy, base, head, tested, "merge_group")
            self.assertEqual(plan["profiles"], ["full"])
            self.require_real_gate(root, policy, base, head, tested, "merge_group", plan)
        for event in ("pull_request", "merge_group", "local"):
            with self.subTest(event=event), self.repository() as (root, policy, base):
                guide = root / "docs/public/acquisition/index.md"
                guide.write_bytes(guide.read_bytes() + b"\nPublic guide prose.\n")
                path = root / "README.md"
                path.write_bytes(path.read_bytes() + b"\nMaintenance prose.\n")
                head, tested = self.candidate(root, base, event)
                plan = self.plan(root, policy, base, head, tested, event)
                self.assertEqual(plan["profiles"], ["executable-docs"])
                self.assertIn(
                    ("cargo", "test", "--locked", "--test", "public_guides"),
                    policy.local_commands["executable-docs"],
                )
                self.require_real_gate(root, policy, base, head, tested, event, plan)

    def test_real_narrow_gate_rejects_unexpected_outcomes_skips_and_candidate(self) -> None:
        for event in ("pull_request", "merge_group", "local"):
            with self.subTest(event=event), self.repository() as (root, policy, base):
                path = root / "README.md"
                path.write_bytes(path.read_bytes() + b"\nMaintenance prose.\n")
                head, tested = self.candidate(root, base, event)
                plan = self.plan(root, policy, base, head, tested, event)
                expected = self.plan(root, policy, base, head, tested, event)
                comparison = Comparison(event, str(plan["base_sha"]), str(plan["head_sha"]), tested)
                needs = self.outcomes(plan, policy)
                verify_gate(plan, needs, policy, comparison, expected_plan=expected)
                for missing in ("validation-plan", *policy.jobs):
                    broken = copy.deepcopy(needs)
                    del broken[missing]
                    with self.subTest(missing=missing), self.assertRaises(ContractError):
                        verify_gate(plan, broken, policy, comparison, expected_plan=expected)
                for mutation in (
                    "extra",
                    "failure",
                    "skip",
                    "cancelled",
                    "unknown",
                    "unselected-failure",
                ):
                    broken = copy.deepcopy(needs)
                    if mutation == "extra":
                        broken["unexpected-job"] = {"result": "success"}
                    elif mutation == "unselected-failure":
                        broken["rust"]["result"] = "failure"
                    else:
                        broken["documentation"]["result"] = {
                            "failure": "failure",
                            "skip": "skipped",
                            "cancelled": "cancelled",
                            "unknown": None,
                        }[mutation]
                    with self.subTest(mutation=mutation), self.assertRaises(ContractError):
                        verify_gate(plan, broken, policy, comparison, expected_plan=expected)
                for mutation in ("forged-skip", "stale-candidate"):
                    broken_plan = copy.deepcopy(plan)
                    if mutation == "forged-skip":
                        jobs = broken_plan["jobs"]
                        assert isinstance(jobs, dict)
                        jobs["documentation"] = False
                    else:
                        broken_plan["tested_sha"] = base
                    with self.subTest(mutation=mutation), self.assertRaises(ContractError):
                        verify_gate(broken_plan, needs, policy, comparison, expected_plan=expected)

    def test_main_manual_release_and_explicit_local_complete_remain_full(self) -> None:
        with self.repository() as (root, policy, base):
            path = root / "README.md"
            path.write_bytes(path.read_bytes() + b"\nMaintenance prose.\n")
            head = tested = self.commit(root)
            for event in ("push", "workflow_dispatch", "workflow_call", "local"):
                with self.subTest(event=event):
                    plan = self.plan(
                        root, policy, base, head, tested, event, complete=event == "local"
                    )
                    self.assertEqual(plan["profiles"], ["full"])
                    self.assertEqual(plan["jobs"], dict.fromkeys(policy.jobs, True))


class ApiPurposeContract(unittest.TestCase):
    """Caller purpose and proof contracts without compiler or network execution."""

    @staticmethod
    def step_script(workflow: str, name: str) -> str:
        step = workflow.split(f"      - name: {name}\n", 1)[1]
        body = step.split("        run: |\n", 1)[1]
        lines = []
        for line in body.splitlines():
            if line and not line.startswith("          "):
                break
            lines.append(line[10:] if line else "")
        return "\n".join(lines) + "\n"

    def test_local_wrapper_dispatch_and_failure_propagation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            scripts = root / "scripts"
            scripts.mkdir()
            shutil.copy2(ROOT / "scripts/check-public-api.sh", scripts)
            (scripts / "shared-ci.py").write_text(
                "import os, sys\n"
                "print('|'.join(sys.argv[1:]))\n"
                "raise SystemExit(int(os.environ.get('API_STUB_STATUS', '0')))\n"
            )
            for arguments, task in (
                ([], "api-development"),
                (["--development"], "api-development"),
                (["--published"], "api-published"),
            ):
                for status in (0, 17):
                    with self.subTest(arguments=arguments, status=status):
                        result = subprocess.run(
                            ["bash", str(scripts / "check-public-api.sh"), *arguments],
                            env={
                                **os.environ,
                                "API_STUB_STATUS": str(status),
                                "PODMAN_LENS_SEMVER_CHECK": "0",
                                "BOXFERRY_SEMVER_RELEASE_TYPE": "major",
                            },
                            text=True,
                            capture_output=True,
                            check=False,
                        )
                        self.assertEqual(result.returncode, status, result.stderr)
                        self.assertEqual(result.stdout.strip(), f"run|--task|{task}")
            for arguments in (
                ["--unknown"],
                ["--release-type-only"],
                ["--development", "extra"],
                ["--published", "--release-type", "major"],
            ):
                result = subprocess.run(
                    ["bash", str(scripts / "check-public-api.sh"), *arguments],
                    text=True,
                    capture_output=True,
                    check=False,
                )
                self.assertEqual(result.returncode, 2)
                self.assertEqual(result.stdout, "")

    def test_policy_tasks_use_shared_selector_and_explicit_purpose(self) -> None:
        policy = json.loads((ROOT / "scripts/validation-policy.json").read_text())
        for purpose in ("development", "published"):
            command = policy["tasks"][f"api-{purpose}"]["commands"][0]
            self.assertEqual(command[:4], ["python", "-m", "boxferry_ci.api_baselines", "run"])
            self.assertEqual(command[command.index("--api-purpose") + 1], purpose)
            self.assertEqual(command[command.index("--event") + 1], "local")
            self.assertEqual(command[command.index("--package") + 1], "podman-lens")
            self.assertIn("--allow-initial", command)
            self.assertIn("--config", command)
            self.assertNotIn("--release-type", command)
        local = (ROOT / "scripts/check-all.sh").read_text()
        self.assertNotIn("PODMAN_LENS_SEMVER_CHECK", local)
        self.assertIn('run_step "Check initial public API contract" cargo test', local)
        self.assertIn('run_step "Check development public API compatibility"', local)

    def test_ci_distinguishes_development_and_exact_published_proof(self) -> None:
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        api = workflow_job(ci, "api")
        self.assertIn("api_purpose: ${{ inputs.api_purpose || 'development' }}", api)
        self.assertIn(
            "release_type: ${{ inputs.api_purpose != 'published' && "
            "needs.api-release-type.outputs.release_type || '' }}",
            api,
        )
        self.assertIn("default: published", ci.split("  push:", 1)[0])
        self.assertIn("value: ${{ jobs.pr-gate.outputs.published_api_candidate_sha }}", ci)
        gate = workflow_job(ci, "pr-gate")
        self.assertIn("needs.api.outputs.published_api_candidate_sha", gate)
        self.assertIn("published_api_candidate_sha: ${{ steps.binding.outputs.", gate)
        script = self.step_script(gate, "Expose successful candidate binding")
        candidate = "a" * 40
        for purpose, receipt, status in (
            ("development", "", 0),
            ("development", candidate, 0),
            ("published", candidate, 0),
            ("published", "", 1),
            ("published", "b" * 40, 1),
            ("published", "main", 1),
            ("unknown", candidate, 1),
        ):
            with self.subTest(purpose=purpose, receipt=receipt):
                with tempfile.TemporaryDirectory() as directory:
                    output = Path(directory) / "output"
                    result = subprocess.run(
                        ["bash", "-c", script],
                        env={
                            **os.environ,
                            "API_PURPOSE": purpose,
                            "PUBLISHED_API_CANDIDATE_SHA": receipt,
                            "GITHUB_SHA": candidate,
                            "GITHUB_OUTPUT": str(output),
                        },
                        capture_output=True,
                        text=True,
                        check=False,
                    )
                    self.assertEqual(result.returncode, status, result.stderr)
                    exposed = output.read_text() if output.exists() else ""
                    if status:
                        self.assertEqual(exposed, "")
                    else:
                        self.assertIn(f"candidate_sha={candidate}\n", exposed)
                        self.assertEqual(
                            f"published_api_candidate_sha={candidate}\n" in exposed,
                            purpose == "published",
                        )

    def test_ci_intent_is_current_event_only_and_queue_stays_authoritative(self) -> None:
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        classifier = workflow_job(ci, "api-release-type")
        self.assertNotIn("--release-type-only", classifier)
        script = self.step_script(classifier, "Preserve pre-1.0 breaking-change intent")
        candidate, automation, base = "a" * 40, "b" * 40, "c" * 40
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output, calls = root / "output", root / "calls"
            for name, body in (
                (
                    "git",
                    'printf "git:%s\\n" "$*" >> "$CLASSIFIER_CALLS"\n'
                    'if [[ "$*" == "-C source log -1 --format=%s" ]]; then\n'
                    '  printf "%s\\n" "$CLASSIFIER_CURRENT_SUBJECT"\n'
                    'else printf "feat!: old trusted-main break\\n"; fi\n',
                ),
                (
                    "uv",
                    'printf "uv:%s\\n" "$*" >> "$CLASSIFIER_CALLS"\n'
                    'printf "release_type=major\\n" >> "$GITHUB_OUTPUT"\n'
                    'exit "${CLASSIFIER_QUEUE_STATUS:-0}"\n',
                ),
            ):
                executable = root / name
                executable.write_text("#!/bin/bash\nset -euo pipefail\n" + body)
                executable.chmod(0o755)
            cases = (
                (
                    "pull_request",
                    "fix: new unmarked change",
                    "feat!: old trusted-main break",
                    "",
                    0,
                ),
                ("pull_request", "feat!: current break", "fix: current subject", "major", 0),
                ("push", "feat!: unrelated title", "fix: current subject", "", 0),
                ("push", "", "fix!: current break", "major", 0),
                ("workflow_dispatch", "", "feat!: current break", "major", 0),
                ("pull_request", "fix: first line\nfeat!: injected second line", "", "", 0),
                ("merge_group", "fix: irrelevant", "", "major", 0),
                ("merge_group", "", "", "major", 17),
                ("unknown", "feat!: ignored", "", "", 1),
            )
            for event, title, subject, expected, status in cases:
                with self.subTest(event=event, title=title, subject=subject, status=status):
                    output.unlink(missing_ok=True)
                    calls.unlink(missing_ok=True)
                    result = subprocess.run(
                        ["bash", "-c", script],
                        env={
                            **os.environ,
                            "PATH": str(root) + os.pathsep + os.environ["PATH"],
                            "EVENT_NAME": event,
                            "SEMVER_CHANGE_SUBJECT": title,
                            "GITHUB_SHA": candidate,
                            "AUTOMATION_SHA": automation,
                            "QUEUE_BASE_SHA": base,
                            "DEFAULT_BRANCH": "main",
                            "GITHUB_OUTPUT": str(output),
                            "CLASSIFIER_CALLS": str(calls),
                            "CLASSIFIER_CURRENT_SUBJECT": subject,
                            "CLASSIFIER_QUEUE_STATUS": str(status),
                        },
                        capture_output=True,
                        text=True,
                        check=False,
                    )
                    self.assertEqual(result.returncode, status, result.stderr)
                    if status == 0:
                        self.assertEqual(output.read_text(), f"release_type={expected}\n")
                    invoked = calls.read_text() if calls.exists() else ""
                    if event in {"push", "workflow_dispatch"}:
                        self.assertEqual(invoked, "git:-C source log -1 --format=%s\n")
                    elif event == "merge_group":
                        self.assertIn("boxferry_ci.release classify-queue", invoked)
                        self.assertIn(f"--candidate-sha {candidate}", invoked)
                        self.assertIn(f"--automation-sha {automation}", invoked)
                        self.assertIn(f"--queue-base-sha {base}", invoked)
                        self.assertIn("--default-branch main", invoked)
                        self.assertNotIn("git:", invoked)
                    else:
                        self.assertEqual(invoked, "")

    def test_release_has_one_standard_api_carrier_without_losing_other_receipts(self) -> None:
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        header = ci.split("  push:", 1)[0].split("    outputs:\n", 1)[1]
        self.assertEqual(
            set(re.findall(r"(?m)^      ([a-z_]+):$", header)),
            {"candidate_sha", "deterministic_published_api_candidate_sha"},
        )
        self.assertIn("value: ${{ jobs.pr-gate.outputs.published_api_candidate_sha }}", header)
        release = (ROOT / ".github/workflows/release.yml").read_text()
        self.assertNotIn("needs.deterministic.outputs.published_api_candidate_sha", release)
        self.assertIn(
            "needs.deterministic.outputs.deterministic_published_api_candidate_sha", release
        )
        aggregate = workflow_job(release, "shared-release-gate")
        self.assertIn(
            "needs: [deterministic, release-metadata, native-conformance, semver, release-gate]",
            aggregate,
        )
        self.assertIn("needs_json: ${{ toJSON(needs) }}", aggregate)
        policy = json.loads((ROOT / ".github/release-policy.json").read_text())
        self.assertEqual(
            policy["required_jobs"],
            ["deterministic", "release-metadata", "native-conformance", "semver", "release-gate"],
        )
        candidate = "a" * 40
        needs: dict[str, Any] = {
            job: {"result": "success", "outputs": {"candidate_sha": candidate}}
            for job in policy["required_jobs"]
        }
        needs["deterministic"]["outputs"]["deterministic_published_api_candidate_sha"] = candidate
        needs["semver"]["outputs"]["published_api_candidate_sha"] = candidate
        self.assertEqual(shared_release.verify_gate(policy, needs, candidate), candidate)
        for job in policy["required_jobs"]:
            for mutation in ("failure", "wrong-candidate"):
                broken = copy.deepcopy(needs)
                if mutation == "failure":
                    broken[job]["result"] = "failure"
                else:
                    broken[job]["outputs"]["candidate_sha"] = "b" * 40
                with (
                    self.subTest(job=job, mutation=mutation),
                    self.assertRaises(shared_release.ReleaseError),
                ):
                    shared_release.verify_gate(policy, broken, candidate)
        for receipt in (None, "", "b" * 40, "main"):
            broken = copy.deepcopy(needs)
            if receipt is None:
                del broken["semver"]["outputs"]["published_api_candidate_sha"]
            else:
                broken["semver"]["outputs"]["published_api_candidate_sha"] = receipt
            with self.subTest(receipt=receipt), self.assertRaises(shared_release.ReleaseError):
                shared_release.verify_gate(policy, broken, candidate)
        duplicate = copy.deepcopy(needs)
        duplicate["deterministic"]["outputs"]["published_api_candidate_sha"] = candidate
        with self.assertRaises(shared_release.ReleaseError):
            shared_release.verify_gate(policy, duplicate, candidate)

    def test_release_requires_both_exact_published_receipts_before_binding(self) -> None:
        release = (ROOT / ".github/workflows/release.yml").read_text()
        for job in ("deterministic", "semver"):
            self.assertIn("api_purpose: published", workflow_job(release, job))
        semver = workflow_job(release, "semver")
        self.assertNotIn("release_type:", semver)
        self.assertIn("    needs: deterministic\n", semver)
        self.assertNotIn("api-release-type", release)
        self.assertNotIn("--release-type-only", release)
        gate = workflow_job(release, "release-gate")
        self.assertIn("needs: [deterministic, native-conformance, semver]", gate)
        for job, output in (
            ("deterministic", "deterministic_published_api_candidate_sha"),
            ("semver", "published_api_candidate_sha"),
        ):
            self.assertIn(f"needs.{job}.outputs.{output}", gate)
        self.assertNotIn("validation_only", gate)
        self.assertLess(
            gate.index("Require exact-candidate published API evidence"), gate.index("id: binding")
        )
        self.assertLess(
            gate.index("scripts/validate-native-evidence.sh"), gate.index("id: binding")
        )
        script = self.step_script(gate, "Require exact-candidate published API evidence")
        candidate = "a" * 40
        for first, second in (
            (candidate, candidate),
            ("", candidate),
            (candidate, ""),
            ("b" * 40, candidate),
            (candidate, "b" * 40),
            ("", ""),
            ("main", candidate),
        ):
            with self.subTest(deterministic=first, semver=second):
                result = subprocess.run(
                    ["bash", "-c", script],
                    env={
                        **os.environ,
                        "GITHUB_SHA": candidate,
                        "DETERMINISTIC_PUBLISHED_API_SHA": first,
                        "SEMVER_PUBLISHED_API_SHA": second,
                    },
                    capture_output=True,
                    text=True,
                    check=False,
                )
                self.assertEqual(result.returncode, 0 if first == second == candidate else 1)


if __name__ == "__main__":
    if sys.argv[1:] == ["--native"]:
        suite = unittest.TestSuite(
            [ConsumerContracts("check_shared_node_caller_has_no_independent_node_version_owner")]
        )
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        raise SystemExit(0 if result.wasSuccessful() else 1)
    unittest.main()
