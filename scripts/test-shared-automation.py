#!/usr/bin/env python3
"""BoxFerry caller contracts; native Renovate runs only in its prepared lane."""

from __future__ import annotations

import copy
import hashlib
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "automation/src"))

PIN = json.loads((ROOT / ".github/automation.json").read_text())["revision"]
OPERATIONAL_CALLERS = (
    "ci",
    "dependency-eligibility",
    "devcontainer-feature-qualification",
    "release-plz",
    "release",
)


def workflow() -> str:
    return (ROOT / ".github/workflows/ci.yml").read_text()


def job(name: str) -> str:
    match = re.search(rf"(?ms)^  {re.escape(name)}:\n.*?(?=^  [\w-]+:\n|\Z)", workflow())
    if match is None:
        raise AssertionError(f"Missing caller job: {name}")
    return match[0]


class ConsumerContracts(unittest.TestCase):
    def assert_shared_pin_coherence(
        self,
        manifest: dict[str, object],
        config: dict[str, object],
        callers: dict[str, str],
    ) -> None:
        pin = manifest["revision"]
        assert isinstance(pin, str)
        self.assertRegex(pin, r"^[a-f0-9]{40}$")
        self.assertEqual(set(callers), set(OPERATIONAL_CALLERS))
        self.assertEqual(
            config["extends"],
            [
                f"github>Strukturpiloten/boxferry//renovate-presets/{name}#{pin}"
                for name in ("common", "rust")
            ],
        )

        def one_pin(raw: str, filename: str) -> None:
            match = re.fullmatch(r"(?P<quote>['\"]?)(?P<sha>[a-f0-9]{40})(?P=quote)(?: +#.*)?", raw)
            self.assertIsNotNone(match, filename + " needs an immutable literal source")
            assert match is not None
            self.assertEqual(match["sha"], pin, filename + " selects another shared source")

        for name, text in callers.items():
            filename = ".github/workflows/" + name + ".yml"
            jobs = re.findall(r"(?ms)^  [\w-]+:\n.*?(?=^  [\w-]+:\n|\Z)", text)
            producer = next((block for block in jobs if block.startswith("  automation:\n")), "")
            shared_calls = 0
            for block in jobs:
                refs = re.findall(
                    r"(?m)^    uses: Strukturpiloten/boxferry/\.github/workflows/"
                    r"[\w-]+\.yml@([^ \n#]+)",
                    block,
                )
                if refs:
                    self.assertEqual(len(refs), 1, filename)
                    one_pin(refs[0], filename)
                    inputs = re.findall(r"(?m)^      automation[_-]sha: ([^\n]+)$", block)
                    self.assertEqual(
                        len(inputs), 1, filename + " shared call needs one source input"
                    )
                    one_pin(inputs[0], filename)
                    shared_calls += 1
            self.assertGreater(shared_calls, 0, filename)
            steps = re.findall(r"(?ms)^      - .*?(?=^      - |^  [\w-]+:\n|\Z)", text)
            for step in steps:
                annotation = (
                    "# renovate: datasource=github-digest "
                    "depName=Strukturpiloten/boxferry currentValue=automation-v1"
                )
                if annotation in step or (
                    "repository: Strukturpiloten/boxferry" in step
                    and re.search(r"(?m)^          path: _automation$", step)
                ):
                    self.assertIn("repository: Strukturpiloten/boxferry", step, filename)
                    self.assertIn(annotation, step, filename)
                    refs = re.findall(r"(?m)^          ref: ([^\n]+)$", step)
                    self.assertEqual(len(refs), 1, filename + " checkout needs one immutable ref")
                    one_pin(refs[0], filename)
                action = re.search(
                    r"uses: \./_automation/\.github/actions/"
                    r"(?P<name>shared-source|shared-crate-publish|shared-node)\b",
                    step,
                )
                if action:
                    inputs = re.findall(r"(?m)^          automation[_-]sha: ([^\n]+)$", step)
                    self.assertEqual(len(inputs), 1, filename + " action needs one source input")
                    if name == "ci" and action["name"] == "shared-node" and step in producer:
                        self.assertEqual(
                            inputs[0], "${{ github.sha }}", filename + " producer source"
                        )
                    else:
                        one_pin(inputs[0], filename)
                if "${AUTOMATION_SHA}" in step:
                    environment = re.findall(r"(?m)^ +AUTOMATION_SHA: ([^\n]+)$", step)
                    self.assertEqual(
                        len(environment), 1, filename + " command needs its held source"
                    )
                    one_pin(environment[0], filename)

    def caller_sources(self) -> dict[str, str]:
        return {
            name: (ROOT / f".github/workflows/{name}.yml").read_text()
            for name in OPERATIONAL_CALLERS
        }

    def test_manifest_loader_and_every_operational_caller_pin_align(self) -> None:
        self.assertEqual(
            (ROOT / "scripts/shared-ci.py").read_bytes(),
            (ROOT / "templates/shared-ci.py").read_bytes(),
        )
        self.assert_shared_pin_coherence(
            json.loads((ROOT / ".github/automation.json").read_text()),
            json.loads((ROOT / ".github/renovate.json").read_text()),
            self.caller_sources(),
        )

    def test_pin_coherence_refuses_each_changed_or_missing_caller_binding(self) -> None:
        manifest = json.loads((ROOT / ".github/automation.json").read_text())
        config = json.loads((ROOT / ".github/renovate.json").read_text())
        callers = self.caller_sources()
        other = "b" * 40 if PIN != "b" * 40 else "c" * 40
        for name, text in callers.items():
            for match in re.finditer(
                r"(?m)^ +(?:automation[_-]sha|AUTOMATION_SHA|ref): " + PIN + r"\n", text
            ):
                for replacement in (match[0].replace(PIN, other), ""):
                    changed = text[: match.start()] + replacement + text[match.end() :]
                    with self.subTest(caller=name, binding=match[0], removed=not replacement):
                        with self.assertRaises(AssertionError):
                            self.assert_shared_pin_coherence(
                                manifest, config, {**callers, name: changed}
                            )
            for match in re.finditer(
                r"Strukturpiloten/boxferry/\.github/workflows/[\w-]+\.yml@" + PIN, text
            ):
                changed = text[: match.start()] + match[0].replace(PIN, other) + text[match.end() :]
                with (
                    self.subTest(caller=name, workflow=match[0]),
                    self.assertRaises(AssertionError),
                ):
                    self.assert_shared_pin_coherence(manifest, config, {**callers, name: changed})
            for match in re.finditer(
                r"(?m)^ +# renovate: datasource=github-digest "
                r"depName=Strukturpiloten/boxferry currentValue=automation-v1\n",
                text,
            ):
                changed = text[: match.start()] + text[match.end() :]
                with self.subTest(caller=name, missing_checkout_owner=True):
                    with self.assertRaises(AssertionError):
                        self.assert_shared_pin_coherence(
                            manifest, config, {**callers, name: changed}
                        )
            with self.subTest(missing_caller=name), self.assertRaises(AssertionError):
                self.assert_shared_pin_coherence(
                    manifest,
                    config,
                    {key: value for key, value in callers.items() if key != name},
                )
        with self.subTest(manifest="wrong source"), self.assertRaises(AssertionError):
            self.assert_shared_pin_coherence({**manifest, "revision": other}, config, callers)
        for index in range(2):
            changed_config = copy.deepcopy(config)
            changed_config["extends"][index] = changed_config["extends"][index].replace(PIN, other)
            with self.subTest(preset=index), self.assertRaises(AssertionError):
                self.assert_shared_pin_coherence(manifest, changed_config, callers)

    def test_no_missing_trusted_policy_fallback_and_exact_revision_binding(
        self,
    ) -> None:
        for name in ("validation-plan", "pr-gate"):
            source = job(name)
            self.assertIn(".validation-base/scripts/shared-validation-policy.json", source)
            self.assertIn("ref: ${{ github.sha }}", source)
            self.assertIn("github.event.merge_group.base_sha", source)
            self.assertIn('--tested-sha "${GITHUB_SHA}"', source)
            self.assertNotIn("if [[ -f", source)
            self.assertNotIn("conservative complete bootstrap", source)
        self.assertIn('--plan-json "${PLAN_JSON}" --needs-json "${NEEDS_JSON}"', job("pr-gate"))

    def test_image_caller_preserves_trusted_plan_and_held_updater(self) -> None:
        policy = json.loads((ROOT / "scripts/shared-validation-policy.json").read_text())
        prior = json.loads(
            (
                ROOT / "automation/tests/fixtures/devcontainer_seed/boxferry-policy.json.txt"
            ).read_text()
        )
        expected = copy.deepcopy(prior)
        expected["jobs"].append("devcontainer-validation")
        self.assertEqual(policy, expected)
        image = job("devcontainer-validation")
        self.assertIn("    needs: validation-plan\n", image)
        self.assertIn(
            "    if: needs.validation-plan.outputs.select_devcontainer_validation != 'false'\n",
            image,
        )
        self.assertIn("    permissions:\n      contents: read\n", image)
        self.assertIn(
            "shared-devcontainer-validation.yml@" + PIN + " # automation-v1 interface v1",
            image,
        )
        self.assertEqual(
            set(re.findall(r"^      ([a-z_]+):", image.split("    with:\n", 1)[1], re.MULTILINE)),
            {"candidate_sha", "policy_sha", "automation_sha", "config_path"},
        )
        for binding in (
            "candidate_sha: ${{ github.sha }}",
            "policy_sha: ${{ github.event.pull_request.base.sha || "
            "github.event.merge_group.base_sha || github.sha }}",
            "automation_sha: " + PIN,
            "config_path: scripts/shared-validation-policy.json",
        ):
            self.assertIn(binding, image)
        self.assertIn(
            "select_devcontainer_validation: "
            "${{ steps.plan.outputs.select_devcontainer_validation }}",
            job("validation-plan"),
        )
        aggregate = job("pr-gate")
        self.assertIn("    if: always()\n", aggregate)
        self.assertIn("python -m boxferry_ci gate --devcontainer-validation", aggregate)
        needs = aggregate.split("    needs:\n", 1)[1].split("    runs-on:", 1)[0]
        self.assertEqual(
            re.findall(r"^        ([a-z-]+),$", needs, re.MULTILINE),
            ["validation-plan", *policy["jobs"]],
        )
        self.assertNotIn("shared-devcontainer-updates.yml@", workflow())

    def test_repository_native_suites_and_coverage_floors_remain_owned(self) -> None:
        policy = json.loads((ROOT / "scripts/shared-validation-policy.json").read_text())
        quality = policy["tasks"]["rust-quality"]["commands"]
        for alias in (
            "ci-check",
            "ci-core",
            "ci-compose",
            "ci-podman",
            "ci-quadlet",
            "ci-policy",
            "ci-clippy",
            "ci-test",
            "ci-doctest",
            "ci-doc",
        ):
            self.assertIn(["cargo", alias], quality)
        coverage = policy["tasks"]["coverage"]["commands"][-1]
        for flag, floor in (
            ("--fail-under-regions", "82"),
            ("--fail-under-functions", "87"),
            ("--fail-under-lines", "82"),
        ):
            self.assertEqual(coverage[coverage.index(flag) + 1], floor)
        self.assertIn("migration-readiness.py run", job("migration-readiness"))
        self.assertIn("--tier offline", job("migration-readiness"))

    def test_development_published_and_queue_purposes_remain_distinct(self) -> None:
        self.assertIn("default: published", workflow())
        self.assertIn("api_purpose: ${{ inputs.api_purpose || 'development' }}", job("semver"))
        self.assertIn("initial_api_contract: false", job("semver"))
        self.assertIn("classify-queue", job("semver-release-type"))
        self.assertIn('--queue-base-sha "${QUEUE_BASE_SHA}"', job("semver-release-type"))
        self.assertIn('"${PUBLISHED_API_CANDIDATE}" != "${CANDIDATE_SHA}"', job("pr-gate"))
        complete = (ROOT / "scripts/check-all.sh").read_text()
        self.assertIn("--api-purpose development --event local", complete)
        release = (ROOT / ".github/workflows/release.yml").read_text()
        self.assertIn("api_purpose: published", release)
        self.assertIn("environment: release", release)
        self.assertIn("!inputs.validation_only", release)

    def test_producer_self_test_uses_guarded_separate_candidate_fixture(self) -> None:
        source = job("automation")
        self.assertIn("timeout-minutes: 10", source)
        self.assertIn(
            "test ! -e dependency-action-fixture && cp -R "
            "_automation/automation/tests/fixtures/cargo-deny-action/source "
            "dependency-action-fixture",
            source,
        )
        self.assertIn("manifest-path: dependency-action-fixture/Cargo.toml", source)
        self.assertIn(
            "arguments: --all-features --config dependency-action-fixture/explicit-policy.toml",
            source,
        )
        self.assertIn("command: check\n", source)
        self.assertNotIn("command-arguments:", source)
        self.assertIn(
            "cargo-deny-action@3c6349835b2b7b196a839186cb8b78e02f7b5f25 # v2.1.1",
            source,
        )
        self.assertNotIn("cargo-deny-action@", job("dependencies"))

    def test_reporter_runs_once_and_native_parser_has_separate_lane(self) -> None:
        source = job("automation")
        self.assertEqual(source.count("python -m boxferry_ci.test_reporting"), 1)
        self.assertNotIn("python -m unittest", source)
        self.assertIn(
            "scripts/test-shared-automation.py --with-native-renovate",
            (ROOT / "scripts/check-all.sh").read_text(),
        )
        self.assertIn("scripts/test-shared-automation.py NativeRenovateContracts", source)
        self.assertIn("scripts/test-shared-automation.py --structural", job("documentation"))
        complete = (ROOT / "scripts/check-all.sh").read_text()
        self.assertEqual(complete.count("python -m boxferry_ci.test_reporting"), 1)

    def test_docs_node_uses_same_selected_source_after_locked_python(self) -> None:
        docs = job("documentation")
        checkout = docs.index("      - name: Check out immutable shared documentation tooling")
        python = docs.index("uses: ./_automation/.github/actions/shared-python")
        node = docs.index("uses: ./_automation/.github/actions/shared-node")
        npm = docs.index("npm ci --ignore-scripts")
        self.assertLess(checkout, python)
        self.assertLess(python, node)
        self.assertLess(node, npm)
        self.assertIn(f"ref: {PIN}\n          path: _automation", docs)
        self.assertIn(f"automation-sha: {PIN}", docs)
        self.assertEqual(docs.count("path: _automation"), 1)
        self.assertEqual(docs.count("uses: ./_automation/.github/actions/shared-python"), 1)
        self.assertIn(
            "          cache: npm\n          cache-dependency-path: package-lock.json",
            docs,
        )
        self.assertNotIn("node-version:", docs)
        self.assertNotIn("uses: actions/setup-node@", docs)
        producer = job("automation")
        declarations = re.findall(
            r"(?m)^          node-version: ([0-9]+\.[0-9]+\.[0-9]+)$", workflow()
        )
        self.assertEqual(len(declarations), 1, "only producer CI owns the canonical Node literal")
        self.assertIn(f"node-version: {declarations[0]}", producer)
        self.assertIn("uses: actions/setup-node@", producer)
        self.assertIn("Validate candidate shared Node composite", producer)
        self.assertIn('automation-sha: ${{ github.sha }}\n          cache: ""', producer)

    def test_complete_gate_has_no_contributor_image_or_sudo_requirement(self) -> None:
        source = (ROOT / "scripts/check-all.sh").read_text()
        self.assertNotIn("scripts/shared-ci.py devcontainer-local", source)
        self.assertNotIn("--outer-mode", source)
        self.assertIn('run_step "Check development API compatibility"', source)
        self.assertIn('run_step "Test shared automation adoption"', source)

    def test_image_exports_selected_python_and_installs_rust_before_cargo_tools(
        self,
    ) -> None:
        image = (ROOT / ".devcontainer/Dockerfile").read_text()
        self.assertNotIn("AS uv_toolchain", image)
        self.assertIn("COPY scripts/shared-ci.py", image)
        self.assertIn("COPY .github/automation.json", image)
        self.assertIn("--tools actionlint lychee --python-tools", image)
        self.assertLess(
            image.index("rustup toolchain install"),
            image.index('cargo "+${rust_channel}" install'),
        )
        self.assertIn("ARG CARGO_SEMVER_CHECKS_VERSION=0.50.0", image)
        self.assertIn("chown -R vscode:vscode /opt/boxferry-tooling/target", image)
        self.assertIn("UV_PYTHON_INSTALL_DIR=/opt/boxferry-tooling/python", image)
        self.assertNotIn("rm -r /opt/boxferry-tooling", image)

    def test_fixed_workloads_and_provider_remain_outside_updates(self) -> None:
        rules = json.loads((ROOT / ".github/renovate.json").read_text())["packageRules"]
        for phrase in (
            "Keep live workload, matrix and probe images fixed",
            "Keep the Docker Compose provider fixed",
        ):
            matches = [rule for rule in rules if rule.get("description") == phrase]
            self.assertEqual(len(matches), 1)
            self.assertIs(matches[0]["enabled"], False)


class SharedBehavior(unittest.TestCase):
    def setUp(self) -> None:
        from boxferry_ci import contracts, gate, planner, policy

        self.contracts = contracts
        self.gate = gate
        self.planner = planner
        self.policy_module = policy
        self.policy = policy.load_policy(ROOT / "scripts/shared-validation-policy.json")

    def test_box_seed_and_adopted_receipts_use_actual_approved_policy(self) -> None:
        from boxferry_ci.gate import verify_devcontainer_gate

        seed_path = ROOT / "automation/tests/fixtures/devcontainer_seed/boxferry-policy.json.txt"
        self.assertEqual(
            hashlib.sha256(seed_path.read_bytes()).hexdigest(),
            "b83045f7d6b17c8874f414536e9d8aa87ddb9e957df236c3f70501bab936cb52",
        )
        seed = self.policy_module.load_policy(seed_path)
        for event in ("pull_request", "merge_group"):
            comparison = self.planner.Comparison(event, "a" * 40, "b" * 40, "c" * 40)
            for policy in (seed, self.policy):
                selected = {name: name in policy.selected_jobs(("prose",)) for name in policy.jobs}
                plan: dict[str, object] = {
                    "schema": 3,
                    "repository": policy.repository,
                    "event": event,
                    "base_sha": comparison.base_sha,
                    "head_sha": comparison.head_sha,
                    "tested_sha": comparison.tested_sha,
                    "profile": "prose",
                    "profiles": ["prose"],
                    "jobs": selected,
                    "reasons": ["actual Box caller prose"],
                    "local_fingerprint": None,
                }
                needs: dict[str, dict[str, object]] = {
                    name: {"result": "success" if enabled else "skipped"}
                    for name, enabled in selected.items()
                }
                needs["validation-plan"] = {"result": "success"}
                needs["devcontainer-validation"] = {
                    "result": "success",
                    "outputs": {"candidate_sha": comparison.tested_sha},
                }
                original = copy.deepcopy(plan)
                verify_devcontainer_gate(plan, needs, policy, comparison, expected_plan=original)
                self.assertEqual(plan, original)
                for receipt in (
                    None,
                    comparison.base_sha,
                    comparison.head_sha,
                    "d" * 40,
                ):
                    wrong = copy.deepcopy(needs)
                    wrong["devcontainer-validation"]["outputs"] = {"candidate_sha": receipt}
                    with (
                        self.subTest(event=event, seed=policy is seed, receipt=receipt),
                        self.assertRaises(self.contracts.ContractError),
                    ):
                        verify_devcontainer_gate(
                            plan, wrong, policy, comparison, expected_plan=original
                        )
                for outcome in ("missing", "failure", "cancelled", "skipped"):
                    wrong = copy.deepcopy(needs)
                    if outcome == "missing":
                        del wrong["devcontainer-validation"]
                    else:
                        wrong["devcontainer-validation"] = {"result": outcome}
                    if policy is self.policy and outcome == "skipped":
                        verify_devcontainer_gate(
                            plan, wrong, policy, comparison, expected_plan=original
                        )
                    else:
                        with self.assertRaises(
                            (self.contracts.ContractError, self.gate.GateFailure)
                        ):
                            verify_devcontainer_gate(
                                plan, wrong, policy, comparison, expected_plan=original
                            )
                wrong = copy.deepcopy(needs)
                wrong["automation"] = {"result": "failure"}
                with self.assertRaises(self.gate.GateFailure):
                    verify_devcontainer_gate(
                        plan, wrong, policy, comparison, expected_plan=original
                    )

    def test_all_final_recipe_inputs_select_image_without_local_image_executor(
        self,
    ) -> None:
        image = (ROOT / ".devcontainer/Dockerfile").read_text()
        copied = {
            item
            for line in image.splitlines()
            if line.startswith("COPY ") and not line.startswith("COPY --from=")
            for item in line.split()[1:-1]
        }
        self.assertEqual(
            copied,
            {
                "rust-toolchain.toml",
                "scripts/install-file-tools.sh",
                "scripts/install-git.sh",
                "automation/src/boxferry_ci/tooling.py",
                "automation/src/boxferry_ci/git_source.py",
                "automation/tool-artifacts.json",
                "scripts/shared-ci.py",
                ".github/automation.json",
                "package.json",
                "package-lock.json",
                "scripts/install-kubernetes-tools.sh",
            },
        )
        inputs = copied | {
            ".devcontainer/Dockerfile",
            ".devcontainer/devcontainer.json",
            ".devcontainer/devcontainer-lock.json",
            ".devcontainer/verify-tools.sh",
            "scripts/git-worktree.py",
            "automation/pyproject.toml",
            "automation/uv.lock",
            "automation/src/boxferry_ci/bootstrap.py",
            "automation/src/boxferry_ci/runner.py",
            "scripts/shared-validation-policy.json",
            "unknown-input",
        }
        for name in inputs:
            with self.subTest(path=name):
                profiles = self.planner.path_profiles(name, (b"plain text\n",), self.policy, set())
                self.assertEqual(profiles, {"full"})
                self.assertIn(
                    "devcontainer-validation",
                    self.policy.selected_jobs(tuple(profiles)),
                )
        for profile, commands in self.policy.local_commands.items():
            with self.subTest(local_profile=profile):
                self.assertTrue(
                    all("devcontainer" not in " ".join(command) for command in commands)
                )
        for event in ("push", "workflow_dispatch", "workflow_call"):
            self.assertEqual(
                set(self.policy.selected_jobs(("full",))), set(self.policy.jobs), event
            )

    def test_box_hosted_source_closure_is_seven_exact_checkouts_only(self) -> None:
        from boxferry_ci import devcontainer_validation as validation

        repositories = (
            "boxferry",
            "compose-lens",
            "docker-lens",
            "kubernetes-lens",
            "podman-lens",
            "quadlet-lens",
            "boxferry-website",
        )
        sources = {name: str(index + 1) * 40 for index, name in enumerate(repositories)}
        candidate = sources["boxferry"]
        self.assertEqual(
            validation.parse_sources({"schema": 1, "repositories": sources}, "boxferry", candidate),
            sources,
        )
        script = validation.probe_script(
            validation.markdownlint_versions(ROOT), sources, "boxferry"
        )
        for name, revision in sources.items():
            target = (
                "/workspaces/boxferry"
                if name == "boxferry"
                else "/workspaces/boxferry/.boxferry-workspace/" + name
            )
            self.assertIn(f'git -C {target} rev-parse HEAD)" = {revision}', script)
        for changed in (
            {name: revision for name, revision in sources.items() if name != "compose-lens"},
            {**sources, "unscoped": "a" * 40},
            {**sources, "boxferry": "b" * 40},
            {**sources, "compose-lens": "main"},
        ):
            with (
                self.subTest(sources=changed),
                self.assertRaises(self.contracts.ContractError),
            ):
                validation.parse_sources(
                    {"schema": 1, "repositories": changed}, "boxferry", candidate
                )
        local = json.loads((ROOT / ".devcontainer/devcontainer.json").read_text())
        self.assertNotIn("workspaceMount", local)
        self.assertNotIn("workspaceFolder", local)
        self.assertEqual(len(local["mounts"]), 2)
        self.assertTrue(all(mount.endswith("type=volume") for mount in local["mounts"]))
        self.assertNotIn(
            ".boxferry-workspace/", (ROOT / ".devcontainer/verify-tools.sh").read_text()
        )

    def test_known_safe_union_retains_all_mandatory_checks(self) -> None:
        prose = self.planner.path_profiles(
            "docs/public/concepts/new-page.md",
            (b"Plain prose\n", b"Revised prose\n"),
            self.policy,
            set(),
            complete_pair=True,
        )
        rust = self.planner.path_profiles(
            "crates/boxferry/src/lib.rs",
            (b"old", b"new"),
            self.policy,
            set(),
            complete_pair=True,
        )
        self.assertEqual(prose, {"prose"})
        self.assertEqual(rust, {"rust-source"})
        prose_jobs = self.policy.selected_jobs(("prose",))
        self.assertNotIn("release-binaries", prose_jobs)
        self.assertNotIn("rust", prose_jobs)
        self.assertNotIn("devcontainer-validation", prose_jobs)
        selected = self.policy.selected_jobs(tuple(sorted(prose | rust)))
        for name in (
            "automation",
            "release-binaries",
            "rust",
            "semver",
            "coverage",
        ):
            self.assertIn(name, selected)

    def test_unknown_policy_and_producer_changes_select_full(self) -> None:
        for path in (
            "unknown/path",
            "scripts/shared-validation-policy.json",
            ".github/workflows/ci.yml",
            "automation/tests/fixtures/cargo-deny-action/source/Cargo.toml",
        ):
            with self.subTest(path=path):
                self.assertEqual(
                    self.planner.path_profiles(
                        path, (b"old", b"new"), self.policy, set(), complete_pair=True
                    ),
                    {"full"},
                )

    def test_push_manual_and_reusable_ci_are_complete(self) -> None:
        for event in ("push", "workflow_dispatch", "workflow_call"):
            with self.subTest(event=event):
                plan = self.planner.make_plan(ROOT, self.policy, event=event, tested_sha="c" * 40)
                self.assertEqual(plan["profiles"], ["full"])
                self.assertEqual(plan["jobs"], dict.fromkeys(self.policy.jobs, True))

    def ordinary_fixture(self) -> tuple:
        policy = self.policy
        comparison = self.planner.Comparison("pull_request", "a" * 40, "b" * 40, "c" * 40)
        plan = {
            "schema": 3,
            "repository": policy.repository,
            "event": comparison.event,
            "base_sha": comparison.base_sha,
            "head_sha": comparison.head_sha,
            "tested_sha": comparison.tested_sha,
            "profile": "full",
            "profiles": ["full"],
            "jobs": dict.fromkeys(policy.jobs, True),
            "reasons": ["independent complete ordinary inventory"],
            "local_fingerprint": None,
        }
        needs = {name: {"result": "success"} for name in ("validation-plan", *policy.jobs)}
        return policy, plan, needs, comparison

    def test_ordinary_inventory_requires_every_original_job(self) -> None:
        policy, plan, needs, comparison = self.ordinary_fixture()
        self.assertEqual(len(policy.jobs), 16)
        result = self.gate.verify_gate(plan, needs, policy, comparison, expected_plan=plan)
        self.assertEqual(len(result), 17)
        for name in policy.jobs:
            for outcome in ("failure", "cancelled", "skipped"):
                broken = copy.deepcopy(needs)
                broken[name]["result"] = outcome
                with (
                    self.subTest(name=name, outcome=outcome),
                    self.assertRaises((self.contracts.ContractError, self.gate.GateFailure)),
                ):
                    self.gate.verify_gate(plan, broken, policy, comparison, expected_plan=plan)

    def test_missing_jobs_and_unbound_ordinary_plans_fail(self) -> None:
        policy, plan, needs, comparison = self.ordinary_fixture()
        missing = copy.deepcopy(needs)
        del missing["automation"]
        with self.assertRaises(self.contracts.ContractError):
            self.gate.verify_gate(plan, missing, policy, comparison, expected_plan=plan)
        for field in ("base_sha", "head_sha", "tested_sha"):
            wrong = copy.deepcopy(plan)
            wrong[field] = "d" * 40
            with (
                self.subTest(field=field),
                self.assertRaises(self.contracts.ContractError),
            ):
                self.gate.verify_gate(wrong, needs, policy, comparison, expected_plan=plan)

    def test_missing_policy_is_never_candidate_bootstrap_authority(self) -> None:
        with (
            tempfile.TemporaryDirectory() as temporary,
            self.assertRaises((OSError, self.contracts.ContractError)),
        ):
            self.policy_module.load_policy(Path(temporary) / "missing-policy.json")

    def test_candidate_plan_cannot_weaken_trusted_job_selection(self) -> None:
        policy, plan, needs, comparison = self.ordinary_fixture()
        wrong = copy.deepcopy(plan)
        wrong["jobs"]["automation"] = False
        with self.assertRaises(self.contracts.ContractError):
            self.gate.verify_gate(wrong, needs, policy, comparison, expected_plan=plan)


def source_job(source: str, name: str) -> str:
    match = re.search(rf"(?ms)^  {re.escape(name)}:\n.*?(?=^  [\w-]+:\n|\Z)", source)
    if match is None:
        raise AssertionError(f"Missing release job: {name}")
    return match[0]


def require_release_boundaries(source: str) -> None:
    prepare = source_job(source, "release-prepare")
    publish = source_job(source, "publish")
    header, steps = prepare.split("    steps:\n", 1)
    if "published_api_candidate_sha" in header.split("    if:", 1)[1].split("    runs-on:", 1)[0]:
        raise AssertionError("missing published proof must fail read-only preparation, not skip it")
    if not steps.startswith("      - name: Require distinct exact published API evidence"):
        raise AssertionError("published proof must precede read-only package setup")
    binding = (
        "PUBLISHED_API_CANDIDATE: "
        "${{ needs.release-validation.outputs.published_api_candidate_sha }}"
    )
    if prepare.count(binding) != 2 or "secrets." in prepare:
        raise AssertionError("read-only preparation needs real published proof and no credentials")
    if "PUBLISHED_API_CANDIDATE: ${{ needs.release-validation.outputs.candidate_sha }}" in prepare:
        raise AssertionError("generic candidate is not published API proof")
    for required in [
        "workflow_dispatch:",
        "!inputs.validation_only",
        "environment: release",
        "id-token: write",
        "github.event_name == 'workflow_dispatch'",
        "github.ref == format('refs/heads/{0}', github.event.repository.default_branch)",
        "needs.release-validation.outputs.published_api_candidate_sha == github.sha",
        "needs.release-prepare.outputs.published_api_candidate_sha == github.sha",
        "needs.release-prepare.result == 'success'",
    ]:
        if required not in source:
            raise AssertionError(f"publication boundary missing: {required}")
    if "printf 'candidate_sha=%s\\n' \"${PUBLISHED_API_CANDIDATE}\"" not in prepare:
        raise AssertionError("read-only candidate binding must derive from published proof")
    guard = textwrap.dedent(steps.split("        run: |\n", 1)[1].split("\n      - ", 1)[0])
    for validated, published, expected in [
        ("a" * 40, "a" * 40, 0),
        ("a" * 40, "", 1),
        ("a" * 40, "b" * 40, 1),
        ("b" * 40, "a" * 40, 1),
    ]:
        result = subprocess.run(
            ["bash", "-c", guard],
            env={
                "PATH": os.defpath,
                "GITHUB_SHA": "a" * 40,
                "VALIDATED_CANDIDATE": validated,
                "PUBLISHED_API_CANDIDATE": published,
            },
            capture_output=True,
            check=False,
            timeout=5,
        )
        if result.returncode != expected:
            raise AssertionError(
                "read-only published-proof guard accepted a missing/stale candidate"
            )
    if publish.count('validation-only: "false"') != 6:
        raise AssertionError("protected publication must use the six configured common components")


class ReleaseConsumerContracts(unittest.TestCase):
    def setUp(self) -> None:
        self.release = (ROOT / ".github/workflows/release.yml").read_text()
        self.policy = json.loads((ROOT / "scripts/shared-release-policy.json").read_text())

    def test_distinct_api_proof_and_existing_publication_boundaries(self) -> None:
        require_release_boundaries(self.release)

    def test_missing_generic_or_stale_api_proof_is_rejected_as_source(self) -> None:
        mutations = [
            self.release.replace(
                "PUBLISHED_API_CANDIDATE: "
                "${{ needs.release-validation.outputs.published_api_candidate_sha }}",
                "PUBLISHED_API_CANDIDATE: ${{ needs.release-validation.outputs.candidate_sha }}",
            ),
            self.release.replace("environment: release", "environment: unrelated"),
            self.release.replace("!inputs.validation_only", "true"),
            self.release.replace("needs.release-prepare.result == 'success'", "true"),
            self.release.replace(
                "needs.release-validation.outputs.published_api_candidate_sha == github.sha",
                "true",
            ),
        ]
        for changed in mutations:
            with self.subTest(source=changed[-40:]), self.assertRaises(AssertionError):
                require_release_boundaries(changed)

    def test_complete_typed_order_and_initial_bootstrap_scope_are_preserved(
        self,
    ) -> None:
        packages = [
            "boxferry-model",
            "boxferry-engine",
            "boxferry-compose",
            "boxferry-podman",
            "boxferry-quadlet",
            "boxferry",
        ]
        self.assertEqual(self.policy["packages"], packages)
        self.assertEqual(self.policy["bootstrap_version"], "0.1.1")
        self.assertIs(self.policy["publication_enabled"], True)
        self.assertEqual(
            self.policy["required_jobs"],
            [
                "deterministic",
                "release-metadata",
                "migration-readiness",
                "migration-readiness-evidence",
                "release-binary-evidence",
            ],
        )
        for name, validation in (("release-prepare", "true"), ("publish", "false")):
            block = source_job(self.release, name)
            found = re.findall(r"(?m)^          package: (boxferry[\w-]*)$", block)
            self.assertEqual(found, packages)
            self.assertEqual(block.count(f'validation-only: "{validation}"'), 6)
            self.assertEqual(
                block.count("uses: ./_automation/.github/actions/shared-crate-publish"),
                6,
            )
        self.assertNotIn("secrets.", source_job(self.release, "release-prepare"))
        self.assertEqual(
            source_job(self.release, "publish").count(
                "bootstrap-token: ${{ secrets.CRATES_IO_BOOTSTRAP_TOKEN }}"
            ),
            6,
        )

    def test_complete_native_binary_and_same_run_evidence_remain_required(self) -> None:
        self.assertIn("api_purpose: published", source_job(self.release, "deterministic"))
        readiness = source_job(self.release, "migration-readiness")
        self.assertIn("uses: ./.github/workflows/migration-readiness.yml", readiness)
        self.assertIn("tier: pre-release", readiness)
        evidence = source_job(self.release, "migration-readiness-evidence")
        for expected in (
            "needs: [deterministic, release-metadata, migration-readiness]",
            "--require-aggregate",
            "--require-success",
            "needs.migration-readiness.outputs.evidence_artifact",
        ):
            self.assertIn(expected, evidence)
        binaries = source_job(self.release, "release-binary-evidence")
        for expected in (
            "release-binaries.py verify-set",
            '--revision "${GITHUB_SHA}" --run-id "${GITHUB_RUN_ID}"',
            "needs.deterministic.outputs.release_binary_artifact",
        ):
            self.assertIn(expected, binaries)
        validation = source_job(self.release, "release-validation")
        for name in self.policy["required_jobs"]:
            self.assertIn(name, validation)
        self.assertIn("needs_json: ${{ toJSON(needs) }}", validation)
        self.assertIn("shared-release-validation.yml@" + PIN, validation)
        publish = source_job(self.release, "publish")
        stage = publish.index("release-binaries.py stage")
        for marker in (
            "Create or verify annotated release tag",
            "Create or replace draft GitHub release",
            "Publish boxferry-model through the shared crate component",
        ):
            self.assertLess(stage, publish.index(marker))
        for forbidden in (
            "gh run list",
            "gh run download",
            "--workflow migration-readiness.yml",
        ):
            self.assertNotIn(forbidden, self.release)


class NativeRenovateContracts(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        # The complete gate can reach native tests before a consumer command has
        # prepared the selected immutable source. Always use its normal verifier,
        # including for warm caches; help dispatches no validation task.
        command = [sys.executable, str(ROOT / "scripts/shared-ci.py"), "--help"]

        def output(value: str | bytes | None) -> str:
            return value.decode(errors="replace") if isinstance(value, bytes) else value or ""

        try:
            result = subprocess.run(
                command,
                cwd=ROOT,
                capture_output=True,
                text=True,
                timeout=120,
                check=False,
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            stdout = output(getattr(error, "stdout", None))
            stderr = output(getattr(error, "stderr", None))
            raise AssertionError(
                f"Canonical selected-source bootstrap failed before native fixtures: {error}\n"
                f"Command: {command!r}\nstdout:\n{stdout}\nstderr:\n{stderr}"
            ) from error
        if result.returncode != 0:
            raise AssertionError(
                "Canonical selected-source bootstrap failed before native fixtures "
                f"(exit {result.returncode}).\nCommand: {command!r}\n"
                f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
            )

    def runtime(self) -> Path:
        runtime = Path(os.environ.get("BOXFERRY_RENOVATE_PACKAGE", ROOT / "target/tools/renovate"))
        expected = json.loads((ROOT / "automation/renovate-runtime.json").read_text())["image"]
        self.assertEqual((runtime / "source-image.txt").read_text().strip(), expected)
        return runtime.resolve()

    def run_native_fixture(
        self, runtime: Path, *, script: str = "", inline: str = "", fault: str = ""
    ) -> subprocess.CompletedProcess[str]:
        files = (
            ".github/automation.json",
            ".github/renovate.json",
            ".github/workflows/ci.yml",
            ".github/workflows/dependency-eligibility.yml",
            ".github/workflows/devcontainer-feature-qualification.yml",
            ".github/workflows/release-plz.yml",
            ".github/workflows/release.yml",
            ".github/workflows/shared-rust-dependencies.yml",
            "renovate-presets/common.json",
            "renovate-presets/rust.json",
            "scripts/test-shared-automation-policy.cjs",
            "scripts/test-cargo-deny-action.cjs",
            "scripts/test-git-source-renovate.cjs",
            "automation/tool-artifacts.json",
        )
        pin = json.loads((ROOT / ".github/automation.json").read_text())["revision"]
        selected = ROOT / "target/boxferry-automation" / pin / "renovate-presets"
        # Seed only the actual selected immutable source, never a local producer
        # projection or a fallback for missing source acquisition.
        self.assertTrue(selected.is_dir(), f"missing selected source cache: {selected}")
        # The unmodified Cargo helper also receives only a copied fixture. Its
        # bootstrap confines initialization before that helper imports the logger.
        bootstrap = r"""
{
  const os = require('node:os');
  const [runtime, root, , , script] = process.argv.slice(1);
  process.chdir(root);
  for (const key of Object.keys(process.env)) {
    if (key.startsWith('GIT_') || key === 'LOG_FILE' || key === 'RENOVATE_LOG_FILE')
      delete process.env[key];
  }
  Object.assign(process.env, {
    TMPDIR:root+'/tmp', XDG_CACHE_HOME:root+'/cache',
    RENOVATE_BASE_DIR:root, RENOVATE_CACHE_DIR:root+'/cache',
    GIT_CEILING_DIRECTORIES:root, GIT_TERMINAL_PROMPT:'0',
    GIT_CONFIG_NOSYSTEM:'1', GIT_CONFIG_GLOBAL:os.devNull, GIT_CONFIG_SYSTEM:os.devNull,
    GIT_CONFIG_COUNT:'0', GIT_OPTIONAL_LOCKS:'0',
    GIT_NO_REPLACE_OBJECTS:'1', GIT_NO_LAZY_FETCH:'1'
  });
  require(runtime + '/dist/config/global.js').GlobalConfig.set({
    localDir:root, baseDir:root, cacheDir:root+'/cache', presetCachePersistence:false
  });
  process.argv = [process.argv[0], script, runtime, root];
  require(script);
}
"""
        with tempfile.TemporaryDirectory(prefix="boxferry-native-fixture-") as directory:
            scratch = Path(directory)
            for name in files:
                source = ROOT / name
                self.assertFalse(source.is_symlink(), name)
                target = scratch / name
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(source, target)
            for name in ("common", "rust"):
                target = scratch / ".native-presets" / (name + ".json")
                target.parent.mkdir(parents=True, exist_ok=True)
                self.assertFalse((selected / (name + ".json")).is_symlink())
                shutil.copy2(selected / (name + ".json"), target)
            (scratch / "tmp").mkdir()
            environment = {
                **{
                    key: value
                    for key, value in os.environ.items()
                    if not key.startswith("GIT_") and key not in {"LOG_FILE", "RENOVATE_LOG_FILE"}
                },
                "GIT_CEILING_DIRECTORIES": directory,
                "GIT_TERMINAL_PROMPT": "0",
                "GIT_CONFIG_NOSYSTEM": "1",
                "GIT_CONFIG_GLOBAL": os.devnull,
                "GIT_CONFIG_SYSTEM": os.devnull,
                "GIT_CONFIG_COUNT": "0",
                "GIT_OPTIONAL_LOCKS": "0",
                "GIT_NO_REPLACE_OBJECTS": "1",
                "GIT_NO_LAZY_FETCH": "1",
                "TMPDIR": str(scratch / "tmp"),
                "XDG_CACHE_HOME": str(scratch / "cache"),
                "RENOVATE_BASE_DIR": directory,
                "RENOVATE_CACHE_DIR": str(scratch / "cache"),
            }
            if script == "scripts/test-shared-automation-policy.cjs":
                command = [
                    str(runtime / "node"),
                    str(scratch / script),
                    str(runtime),
                    directory,
                    str(scratch / ".native-presets"),
                ]
            else:
                command = [
                    str(runtime / "node"),
                    "-e",
                    bootstrap if script else inline,
                    str(runtime),
                    directory,
                    directory,
                    fault,
                    str(scratch / script) if script else "",
                ]
            return subprocess.run(
                command,
                cwd=directory,
                env=environment,
                capture_output=True,
                text=True,
                timeout=120,
                check=False,
            )

    def test_actual_local_native_policy_and_late_overrides(self) -> None:
        result = self.run_native_fixture(
            self.runtime(), script="scripts/test-shared-automation-policy.cjs"
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_native_git_source_owner_lookup_replacement_and_policy(self) -> None:
        result = self.run_native_fixture(
            self.runtime(), script="scripts/test-git-source-renovate.cjs"
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_native_action_owner_and_shared_reference_replacement(self) -> None:
        runtime = self.runtime()
        result = self.run_native_fixture(runtime, script="scripts/test-cargo-deny-action.cjs")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        result = self.run_native_source_replacement(runtime)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def run_native_source_replacement(
        self, runtime: Path, fault: str = ""
    ) -> subprocess.CompletedProcess[str]:
        source = r"""
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const [runtimeArg, sourceArg, , fault] = process.argv.slice(1);
const runtime = path.resolve(runtimeArg), sourceRoot = path.resolve(sourceArg);
const parent = fs.realpathSync(os.tmpdir());
const scratch = fs.mkdtempSync(path.join(parent,'boxferry-native-replacement-'));
(async () => {
try {
  process.chdir(scratch);
  fs.mkdirSync(scratch+'/tmp');
  for (const key of Object.keys(process.env)) {
    if (key.startsWith('GIT_') || key === 'LOG_FILE' || key === 'RENOVATE_LOG_FILE')
      delete process.env[key];
  }
  Object.assign(process.env, {
    TMPDIR:scratch+'/tmp', XDG_CACHE_HOME:scratch+'/cache',
    RENOVATE_BASE_DIR:scratch, RENOVATE_CACHE_DIR:scratch+'/cache',
    GIT_CEILING_DIRECTORIES:scratch, GIT_TERMINAL_PROMPT:'0',
    GIT_CONFIG_NOSYSTEM:'1', GIT_CONFIG_GLOBAL:os.devNull, GIT_CONFIG_SYSTEM:os.devNull,
    GIT_CONFIG_COUNT:'0', GIT_OPTIONAL_LOCKS:'0',
    GIT_NO_REPLACE_OBJECTS:'1', GIT_NO_LAZY_FETCH:'1'
  });
  for (const file of [
    '.github/automation.json','.github/renovate.json','.github/workflows/ci.yml',
    '.github/workflows/dependency-eligibility.yml',
    '.github/workflows/devcontainer-feature-qualification.yml',
    '.github/workflows/release-plz.yml','.github/workflows/release.yml','renovate-presets/common.json'
  ]) {
    assert.equal(fs.lstatSync(sourceRoot+'/'+file).isSymbolicLink(),false,file);
    fs.mkdirSync(path.dirname(scratch+'/'+file),{recursive:true});
    fs.copyFileSync(sourceRoot+'/'+file,scratch+'/'+file);
  }
  require(runtime+'/dist/config/global.js').GlobalConfig.set({
    localDir:scratch,baseDir:scratch,cacheDir:scratch+'/cache',presetCachePersistence:false
  });
  await require(runtime+'/dist/logger/index.js').init();
  const regex = require(runtime + '/dist/modules/manager/custom/regex/index.js');
  const actions = require(runtime + '/dist/modules/manager/github-actions/extract.js');
  const {doAutoReplace} =
    require(runtime + '/dist/workers/repository/update/branch/auto-replace.js');
  const common = JSON.parse(fs.readFileSync(scratch + '/renovate-presets/common.json','utf8'));
  const local = JSON.parse(fs.readFileSync(scratch + '/.github/renovate.json','utf8'));
  const pin = JSON.parse(fs.readFileSync(scratch + '/.github/automation.json','utf8')).revision;
  const next = 'a'.repeat(40);
  assert.notEqual(pin,next);
  // Independent literal-declaration inventory: [custom.regex, native github-actions].
  const expected = new Map([
    ['.github/automation.json',[1,0]],
    ['.github/renovate.json',[2,0]],
    ['.github/workflows/ci.yml',[15,7]],
    ['.github/workflows/dependency-eligibility.yml',[1,1]],
    ['.github/workflows/devcontainer-feature-qualification.yml',[1,1]],
    ['.github/workflows/release-plz.yml',[1,1]],
    ['.github/workflows/release.yml',[15,1]],
  ]);
  let count = 0;
  for (const [file,counts] of expected) {
    const content = fs.readFileSync(scratch + '/' + file,'utf8');
    const slots = [...content.matchAll(new RegExp(pin,'g'))].map(match => match.index);
    assert.equal(slots.length,counts[0]+counts[1],file+' literal declaration inventory');
    const rows = [];
    for (const manager of [...common.customManagers,...local.customManagers]) {
      if (!manager.managerFilePatterns.some(p => new RegExp(p.slice(1,-1)).test(file))) continue;
      const extracted = regex.extractPackageFile(content,file,manager);
      for (const [depIndex,dep] of (extracted?.deps || []).entries()) {
        if (dep.depName !== 'Strukturpiloten/boxferry') continue;
        rows.push({manager:'regex',definition:manager,extracted,depIndex,dep});
      }
    }
    if (file.endsWith('.yml')) {
      const extracted = await actions.extractPackageFile(content,file,{});
      for (const [depIndex,dep] of (extracted?.deps || []).entries()) {
        if (dep.currentDigest !== pin) continue;
        assert.equal(dep.depName,'Strukturpiloten/boxferry');
        rows.push({manager:'github-actions',definition:{},extracted,depIndex,dep});
      }
    }
    assert.deepEqual(
      ['regex','github-actions'].map(manager => rows.filter(row => row.manager === manager).length),
      counts,file+' exact native/custom ownership counts');
    const owners = new Map(slots.map(position => [position,[]]));
    for (const row of rows) {
      assert.equal(row.dep.currentDigest,pin);
      assert.equal(row.dep.currentValue,'automation-v1');
      fs.writeFileSync(scratch+'/'+file,content);
      const updated = await doAutoReplace({...row.definition,...row.extracted,...row.dep,
        manager:row.manager,depIndex:row.depIndex,packageFile:file,
        newValue:row.dep.currentValue,newDigest:next,autoReplaceGlobalMatch:false},content,false);
      const changed = slots.filter(position => updated ===
        content.slice(0,position)+next+content.slice(position+pin.length));
      assert.equal(changed.length,1,file+' replaces exactly one complete immutable declaration');
      owners.get(changed[0]).push(row.manager);
      const after = row.manager === 'regex'
        ? regex.extractPackageFile(updated,file,row.definition)
        : await actions.extractPackageFile(updated,file,{});
      assert.equal(after.deps[row.depIndex].currentDigest,next);
      assert.equal(after.deps[row.depIndex].currentValue,'automation-v1');
      assert.equal(fs.readFileSync(scratch+'/'+file,'utf8'),updated);
      count++;
      if (fault === 'assertion') assert.fail('injected failure after native write');
      if (fault === 'terminate') process.kill(process.pid,'SIGTERM');
    }
    for (const [position,managerNames] of owners)
      assert.equal(managerNames.length,1,file+':'+position+' needs exactly one Renovate owner');
    let coherent = content;
    for (const row of rows) {
      fs.writeFileSync(scratch+'/'+file,coherent);
      coherent = await doAutoReplace({...row.definition,...row.extracted,...row.dep,
        manager:row.manager,depIndex:row.depIndex,packageFile:file,
        newValue:row.dep.currentValue,newDigest:next,autoReplaceGlobalMatch:false},coherent,false);
    }
    assert.equal(coherent,content.split(pin).join(next),
      file+' coherent complete source replacement');
  }
  assert.equal(count,47,'every operational source declaration has an exclusive tested owner');

} finally {
  process.chdir(parent);
  fs.rmSync(scratch,{recursive:true,force:true});
}
})().catch(error => {console.error(error);process.exitCode=1;});
"""
        return self.run_native_fixture(runtime, inline=source, fault=fault)

    def test_native_replacement_failure_and_termination_preserve_real_inputs(
        self,
    ) -> None:
        def source_states() -> dict[str, tuple[str, int]]:
            result = {}
            for directory, dirs, names in os.walk(ROOT):
                dirs[:] = [
                    name
                    for name in dirs
                    if name
                    not in {
                        ".git",
                        "target",
                        ".venv",
                        "__pycache__",
                        ".ruff_cache",
                        "node_modules",
                    }
                ]
                for name in names:
                    path = Path(directory) / name
                    self.assertFalse(path.is_symlink(), str(path))
                    result[str(path.relative_to(ROOT))] = (
                        hashlib.sha256(path.read_bytes()).hexdigest(),
                        path.stat().st_mode,
                    )
            return result

        runtime = self.runtime()
        before = source_states()
        for fault in ("assertion", "terminate"):
            with self.subTest(fault=fault):
                result = self.run_native_source_replacement(runtime, fault)
                if fault == "assertion":
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn("injected failure after native write", result.stderr)
                else:
                    self.assertEqual(result.returncode, -signal.SIGTERM)
                self.assertEqual(source_states(), before)

    def test_hostile_child_environment_is_filtered_without_mutating_parent(
        self,
    ) -> None:
        from unittest.mock import patch

        with tempfile.TemporaryDirectory(prefix="boxferry-hostile-environment-") as directory:
            victim = Path(directory) / "victim"
            victim.mkdir()
            canary = victim / "canary"
            canary.write_text("unchanged")
            hostile = {
                **{
                    key: str(victim / key.lower())
                    for key in (
                        "GIT_DIR",
                        "GIT_WORK_TREE",
                        "GIT_INDEX_FILE",
                        "GIT_COMMON_DIR",
                        "GIT_CONFIG_GLOBAL",
                        "GIT_CONFIG_SYSTEM",
                        "GIT_CONFIG_PARAMETERS",
                        "GIT_CONFIG_COUNT",
                        "GIT_CONFIG_KEY_0",
                        "GIT_CONFIG_VALUE_0",
                        "GIT_INJECT_EXTRA",
                    )
                },
                "LOG_FILE": str(canary),
                "RENOVATE_LOG_FILE": str(canary),
            }
            probe = r"""
const assert=require('node:assert/strict'),os=require('node:os');
const root=process.argv[2];
assert.equal(process.cwd(),root);
for(const key of ['GIT_DIR','GIT_WORK_TREE','GIT_INDEX_FILE','GIT_COMMON_DIR',
'GIT_CONFIG_PARAMETERS','GIT_CONFIG_KEY_0','GIT_CONFIG_VALUE_0','GIT_INJECT_EXTRA',
'LOG_FILE','RENOVATE_LOG_FILE']) assert.equal(process.env[key],undefined,key);
assert.equal(process.env.GIT_CEILING_DIRECTORIES,root);
assert.equal(process.env.GIT_TERMINAL_PROMPT,'0');
assert.equal(process.env.GIT_CONFIG_NOSYSTEM,'1');
assert.equal(process.env.GIT_CONFIG_GLOBAL,os.devNull);
assert.equal(process.env.GIT_CONFIG_SYSTEM,os.devNull);
assert.equal(process.env.GIT_CONFIG_COUNT,'0');
assert.equal(process.env.TMPDIR,root+'/tmp');
assert.equal(process.env.XDG_CACHE_HOME,root+'/cache');
assert.equal(process.env.RENOVATE_BASE_DIR,root);
assert.equal(process.env.RENOVATE_CACHE_DIR,root+'/cache');
"""
            with patch.dict(os.environ, hostile):
                parent = dict(os.environ)
                result = self.run_native_fixture(self.runtime(), inline=probe)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(dict(os.environ), parent)
            self.assertEqual(canary.read_text(), "unchanged")


class NativeCachePreparationContracts(unittest.TestCase):
    def probe(
        self,
        *,
        create_cache: bool = False,
        warm: bool = False,
        returncode: int = 0,
        error: Exception | None = None,
        consume_cache: bool = False,
    ) -> tuple[unittest.TestResult, list[str], list[object]]:
        from unittest.mock import patch

        manifest = (ROOT / ".github/automation.json").read_bytes()
        with tempfile.TemporaryDirectory(prefix="boxferry-cache-preparation-") as directory:
            root = Path(directory)
            (root / ".github").mkdir()
            (root / ".github/automation.json").write_bytes(manifest)
            pin = json.loads(manifest)["revision"]
            selected = root / "target/boxferry-automation" / pin / "renovate-presets"
            events = []

            def cache() -> None:
                selected.mkdir(parents=True)
                for name in ("common", "rust"):
                    (selected / (name + ".json")).write_text("{}\n")

            if warm:
                cache()

            class Probe(NativeRenovateContracts):
                def runTest(self) -> None:
                    events.append("native")
                    if consume_cache:
                        self.run_native_fixture(root / "unused-runtime", inline="unused")
                    else:
                        self.assertTrue(selected.is_dir())
                        self.assertTrue((selected / "common.json").is_file())
                        self.assertTrue((selected / "rust.json").is_file())

            def frontloader(
                command: list[str], **options: object
            ) -> subprocess.CompletedProcess[str]:
                self.assertEqual(
                    command,
                    [sys.executable, str(root / "scripts/shared-ci.py"), "--help"],
                )
                self.assertEqual(
                    options,
                    {
                        "cwd": root,
                        "capture_output": True,
                        "text": True,
                        "timeout": 120,
                        "check": False,
                    },
                )
                events.append("bootstrap")
                if error is not None:
                    raise error
                if create_cache:
                    cache()
                return subprocess.CompletedProcess(
                    command,
                    returncode,
                    "captured bootstrap stdout",
                    "captured bootstrap stderr",
                )

            with (
                patch.object(sys.modules[__name__], "ROOT", root),
                patch.object(subprocess, "run", side_effect=frontloader) as run,
            ):
                result = unittest.TestResult()
                unittest.TestSuite([Probe()]).run(result)
            return result, events, list(run.call_args_list)

    def test_cold_cache_uses_canonical_preparation_before_native_work(self) -> None:
        result, events, calls = self.probe(create_cache=True)
        self.assertTrue(result.wasSuccessful(), result.errors + result.failures)
        self.assertEqual(events, ["bootstrap", "native"])
        self.assertEqual(len(calls), 1)

    def test_warm_cache_still_runs_canonical_verification(self) -> None:
        result, events, calls = self.probe(warm=True)
        self.assertTrue(result.wasSuccessful(), result.errors + result.failures)
        self.assertEqual(events, ["bootstrap", "native"])
        self.assertEqual(len(calls), 1)

    def test_bootstrap_failure_blocks_native_even_with_existing_cache(self) -> None:
        result, events, calls = self.probe(warm=True, returncode=2)
        self.assertEqual(result.testsRun, 0)
        self.assertEqual(events, ["bootstrap"])
        self.assertEqual(len(calls), 1)
        diagnostic = "".join(message for _, message in result.errors)
        self.assertIn("captured bootstrap stdout", diagnostic)
        self.assertIn("captured bootstrap stderr", diagnostic)
        self.assertIn("exit 2", diagnostic)

    def test_timeout_preserves_available_output_and_blocks_native(self) -> None:
        error = subprocess.TimeoutExpired(
            "canonical bootstrap",
            120,
            output=b"partial timeout stdout",
            stderr=b"partial timeout stderr",
        )
        result, events, calls = self.probe(error=error)
        self.assertEqual(result.testsRun, 0)
        self.assertEqual(events, ["bootstrap"])
        self.assertEqual(len(calls), 1)
        diagnostic = "".join(message for _, message in result.errors)
        self.assertIn("partial timeout stdout", diagnostic)
        self.assertIn("partial timeout stderr", diagnostic)
        self.assertIn("timed out", diagnostic)

    def test_oserror_is_actionable_and_blocks_native(self) -> None:
        error = FileNotFoundError("canonical frontloader unavailable")
        result, events, calls = self.probe(error=error)
        self.assertEqual(result.testsRun, 0)
        self.assertEqual(events, ["bootstrap"])
        self.assertEqual(len(calls), 1)
        diagnostic = "".join(message for _, message in result.errors)
        self.assertIn("canonical frontloader unavailable", diagnostic)
        self.assertIn("scripts/shared-ci.py", diagnostic)

    def test_zero_exit_without_cache_still_refuses_before_sdk_launch(self) -> None:
        result, events, calls = self.probe(consume_cache=True)
        self.assertEqual(result.testsRun, 1)
        self.assertEqual(events, ["bootstrap", "native"])
        self.assertEqual(len(calls), 1)
        self.assertEqual(len(result.failures), 1)
        self.assertIn("missing selected source cache", result.failures[0][1])


if __name__ == "__main__":
    if "--with-native-renovate" in sys.argv:
        sys.argv.remove("--with-native-renovate")
    if "--structural" in sys.argv:
        sys.argv.remove("--structural")
        suite = unittest.TestSuite(
            unittest.defaultTestLoader.loadTestsFromTestCase(case)
            for case in (
                ConsumerContracts,
                SharedBehavior,
                ReleaseConsumerContracts,
                NativeCachePreparationContracts,
            )
        )
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        raise SystemExit(not result.wasSuccessful())
    unittest.main(verbosity=2)
