"""Independent source histories and oracle stubs; no compilers, registry calls or builds."""

from __future__ import annotations

import contextlib
import io
import json
import os
import re
import signal
import subprocess
import sys
import tempfile
import time
import tomllib
import unittest
from collections.abc import Mapping
from dataclasses import replace
from email.message import Message
from pathlib import Path
from unittest.mock import patch
from urllib.error import HTTPError, URLError

from boxferry_ci import api_baselines as api
from boxferry_ci import bootstrap as bootstrap_module
from boxferry_ci import cargo_tools
from boxferry_ci.__main__ import main as policy_main
from boxferry_ci.contracts import ContractError
from boxferry_ci.runner import Diagnostic, RunCancelled, RunResult


class GitHistories(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.git("init", "-q", "-b", "main")
        self.git("config", "user.email", "test@example.invalid")
        self.git("config", "user.name", "API policy tests")
        (self.root / "Cargo.toml").write_text('[package]\nname="example"\nversion="0.1.0"\n')
        (self.root / ".gitignore").write_text("target/\n")
        (self.root / "api.rs").write_text("pub fn original() {}\n")
        self.base = self.commit("published API")
        (self.root / "api.rs").write_text("pub fn already_changed_on_main() {}\n")
        self.main = self.commit("existing main API break")
        self.git("checkout", "-qb", "proposal")
        (self.root / "workflow.yml").write_text("workflow-only adoption\n")
        self.first = self.commit("workflow change")
        (self.root / "other.txt").write_text("second proposed commit\n")
        self.candidate = self.commit("whole branch")
        self.context = api.Context(
            "development",
            "pull_request",
            self.candidate,
            self.main,
            "a" * 40,
            "Strukturpiloten/boxferry",
        )
        self.event = {"pull_request": {"base": {"sha": self.main}}}
        self.tool = cargo_tools.Selection("cargo-semver-checks", "1.2.3", "1.98.1")
        self.packages = {"example": {"version": "0.1.0"}}
        # These tests isolate baseline selection. The real archive/CLI/source-bound
        # positive contract is exercised by ArchiveIdentityCli in test_cli.py.
        identity = patch.object(
            api,
            "automation_identity",
            side_effect=lambda _root, expected=None, **_kwargs: {
                "revision": expected or "b" * 40,
                "kind": "baseline-test-fixture",
            },
        )
        self.identity = identity.start()
        self.addCleanup(identity.stop)

    def git(self, *args: str) -> str:
        return subprocess.run(
            ["git", *args],
            cwd=self.root,
            check=True,
            capture_output=True,
            text=True,
            env=dict(os.environ, GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull),
        ).stdout.strip()

    def commit(self, message: str) -> str:
        self.git("add", ".")
        self.git("commit", "-qm", message)
        return self.git("rev-parse", "HEAD")

    def test_pr_and_queue_use_exact_base_and_actual_tested_merge(self) -> None:
        for event, body in (
            ("pull_request", self.event),
            ("merge_group", {"merge_group": {"base_sha": self.main}}),
        ):
            with self.subTest(event=event):
                base, authority = api.select_git(
                    self.root, replace(self.context, event=event), body
                )
                self.assertEqual(base, self.main)
                self.assertEqual(authority, {"event_base": self.main})
        for wrong in (self.base, self.candidate, "f" * 40, None):
            with self.subTest(wrong=wrong), self.assertRaises(ContractError):
                api.select_git(self.root, self.context, {"pull_request": {"base": {"sha": wrong}}})

    def test_base_self_and_forged_candidate_fail(self) -> None:
        with self.assertRaises(ContractError):
            api.select_git(
                self.root,
                replace(self.context, policy=self.candidate),
                {"pull_request": {"base": {"sha": self.candidate}}},
            )
        with self.assertRaises(ContractError):
            api.select_git(self.root, replace(self.context, candidate=self.first), self.event)

    def test_main_push_covers_all_commits_since_exact_before(self) -> None:
        event = {
            "before": self.main,
            "ref": "refs/heads/main",
            "repository": {"default_branch": "main"},
        }
        context = replace(self.context, event="push", policy=self.candidate)
        self.assertEqual(api.select_git(self.root, context, event)[0], self.main)
        for mutation in (
            {"before": "0" * 40},
            {"before": self.candidate},
            {"ref": "refs/heads/proposal"},
            {"before": "f" * 40},
        ):
            with self.subTest(mutation=mutation), self.assertRaises(ContractError):
                api.select_git(self.root, context, {**event, **mutation})

    def test_local_and_manual_cover_entire_multicommit_branch(self) -> None:
        with patch.object(api, "default_authority", return_value=("main", self.main)):
            for event in ("local", "workflow_dispatch"):
                with self.subTest(event=event):
                    self.assertEqual(
                        api.select_git(self.root, replace(self.context, event=event), {})[0],
                        self.main,
                    )
                    self.assertNotEqual(self.main, self.first)

    def test_clean_main_uses_previous_main_and_dirty_main_can_use_head(self) -> None:
        self.git("checkout", "-q", "main")
        context = replace(self.context, event="local", candidate=self.main, policy=self.main)
        with patch.object(api, "default_authority", return_value=("main", self.main)):
            self.assertEqual(api.select_git(self.root, context, {})[0], self.base)
            (self.root / "api.rs").write_text("pub fn local_addition() {}\n")
            self.assertEqual(api.select_git(self.root, context, {})[0], self.main)
            with (
                patch.object(cargo_tools, "select", return_value=self.tool),
                patch.object(api, "metadata", return_value=self.packages),
            ):
                evidence = api.prepare(self.root, context, {}, None, "", False)
            self.assertEqual(evidence["checks"][0]["baseline_rev"], self.main)
            self.assertEqual(len(evidence["source_fingerprint"]), 64)

    def test_index_only_changes_cannot_authorize_self_comparison(self) -> None:
        self.git("checkout", "-q", "main")
        original = (self.root / "api.rs").read_text()
        (self.root / "api.rs").write_text("staged only\n")
        self.git("add", "api.rs")
        (self.root / "api.rs").write_text(original)
        context = replace(self.context, event="local", candidate=self.main)
        with (
            patch.object(api, "default_authority", return_value=("main", self.main)),
            patch.object(cargo_tools, "select", return_value=self.tool),
            patch.object(api, "metadata", return_value=self.packages),
            self.assertRaisesRegex(ContractError, "without actual source"),
        ):
            api.prepare(self.root, context, {}, None, "", False)

    def test_phantom_staged_file_absent_from_source_does_not_authorize_self_comparison(
        self,
    ) -> None:
        self.git("checkout", "-q", "main")
        (self.root / "phantom.rs").write_text("staged but absent source\n")
        self.git("add", "phantom.rs")
        (self.root / "phantom.rs").unlink()
        context = replace(self.context, event="local", candidate=self.main)
        with (
            patch.object(api, "default_authority", return_value=("main", self.main)),
            patch.object(cargo_tools, "select", return_value=self.tool),
            patch.object(api, "metadata", return_value=self.packages),
            self.assertRaisesRegex(ContractError, "without actual source"),
        ):
            api.prepare(self.root, context, {}, None, "", False)

    def test_shallow_graft_partial_and_replace_history_fail_closed(self) -> None:
        for path in ("shallow", "info/grafts"):
            target = self.root / ".git" / path
            target.parent.mkdir(exist_ok=True)
            target.write_text(self.main + "\n")
            with self.subTest(path=path), self.assertRaises(ContractError):
                api.select_git(self.root, self.context, self.event)
            target.unlink()
        self.git("config", "remote.origin.promisor", "true")
        with self.assertRaisesRegex(ContractError, "partial"):
            api.select_git(self.root, self.context, self.event)
        self.git("config", "--unset", "remote.origin.promisor")
        self.git("replace", self.main, self.base)
        self.assertEqual(api.select_git(self.root, self.context, self.event)[0], self.main)

    def test_remote_movement_and_missing_fork_fail(self) -> None:
        with (
            patch.object(
                api,
                "default_authority",
                side_effect=[("main", self.main), ("main", self.candidate)],
            ),
            self.assertRaisesRegex(ContractError, "moved"),
        ):
            api.select_git(self.root, replace(self.context, event="local"), {})
        with (
            patch.object(api, "default_authority", return_value=("main", "f" * 40)),
            self.assertRaises(ContractError),
        ):
            api.select_git(self.root, replace(self.context, event="local"), {})

    def test_source_fingerprint_covers_untracked_deletions_and_bytes(self) -> None:
        before = api.fingerprint(self.root)
        (self.root / "untracked.txt").write_text("protected source\n")
        self.assertNotEqual(api.fingerprint(self.root), before)
        (self.root / "untracked.txt").unlink()
        self.assertEqual(api.fingerprint(self.root), before)
        (self.root / "api.rs").unlink()
        self.assertNotEqual(api.fingerprint(self.root), before)
        with patch.object(api, "MAX_SOURCE_BYTES", 1), self.assertRaises(ContractError):
            api.fingerprint(self.root)

    def test_development_uses_git_despite_preexisting_published_break(self) -> None:
        with (
            patch.object(cargo_tools, "select", return_value=self.tool),
            patch.object(api, "metadata", return_value=self.packages),
            patch.object(
                api,
                "registry_version",
                side_effect=AssertionError("existing Git crate must not query registry"),
            ),
        ):
            evidence = api.prepare(self.root, self.context, self.event, None, "", False)
        check = evidence["checks"][0]
        self.assertEqual(check["baseline_rev"], self.main)
        self.assertEqual(check["release_type"], "minor")
        self.assertIn("--baseline-rev", api.argv_for(self.root, check))

    def private_library(self) -> tuple[api.Context, dict[str, object]]:
        (self.root / "Cargo.toml").write_text(
            '[package]\nname="kubernetes-lens"\nversion="0.1.0"\npublish=false\n'
        )
        baseline = self.commit("existing private workspace library")
        (self.root / "workflow.yml").write_text("private library workflow adoption\n")
        candidate = self.commit("validate private library without publishing")
        return (
            replace(
                self.context,
                candidate=candidate,
                policy=baseline,
                repository="Strukturpiloten/kubernetes-lens",
            ),
            {"pull_request": {"base": {"sha": baseline}}},
        )

    def private_metadata_command(
        self,
        root: Path,
        argv: tuple[str, ...],
        *,
        timeout: int = 30,
        environment: Mapping[str, str] | None = None,
    ) -> bytes:
        if argv[:3] == (sys.executable, "-m", "boxferry_ci"):
            selected = argv[argv.index("--automation-sha") + 1]
            with (
                patch.object(
                    bootstrap_module,
                    "automation_identity",
                    return_value={
                        "revision": selected,
                        "kind": "baseline-test-fixture",
                    },
                ),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                status = policy_main(list(argv[3:]))
            if status:
                raise ContractError("fixture positive API task failed")
            return b"positive task executed\n"
        if argv[0] != "cargo" or "metadata" not in argv:
            return self.original_private_command(
                root, argv, timeout=timeout, environment=environment
            )
        package = tomllib.loads((root / "Cargo.toml").read_text())["package"]
        return json.dumps(
            {
                "workspace_members": ["private-id"],
                "packages": [
                    {
                        "id": "private-id",
                        "name": package["name"],
                        "version": package["version"],
                        "publish": [] if package.get("publish") is False else None,
                        "targets": [{"kind": ["lib"]}],
                    }
                ],
            }
        ).encode()

    def test_explicit_private_library_keeps_its_existing_development_git_baseline(self) -> None:
        context, event = self.private_library()
        self.original_private_command = api.command
        with (
            patch.object(api, "command", side_effect=self.private_metadata_command),
            patch.object(cargo_tools, "select", return_value=self.tool),
            patch.object(
                api,
                "registry_version",
                side_effect=AssertionError(
                    "private existing Git baseline must not fall back to registry"
                ),
            ),
        ):
            libraries = api.metadata(self.root, self.tool.rust_channel)
            self.assertEqual(libraries["kubernetes-lens"]["publish"], [])
            evidence = api.prepare(self.root, context, event, "kubernetes-lens", "", False)
        check = evidence["checks"][0]
        self.assertEqual(check["kind"], "git")
        self.assertEqual(check["baseline_rev"], context.policy)
        command = api.argv_for(self.root, check)
        self.assertEqual(command[command.index("--package") + 1], "kubernetes-lens")
        self.assertNotIn("--workspace", command)

    def test_default_workspace_selection_excludes_private_support_but_explicit_selection_accepts_it(
        self,
    ) -> None:
        packages = {
            "example": {"version": "0.1.0", "publish": None},
            "private-support": {"version": "0.1.0", "publish": []},
        }
        with (
            patch.object(cargo_tools, "select", return_value=self.tool),
            patch.object(api, "metadata", return_value=packages),
            patch.object(
                api,
                "registry_version",
                side_effect=AssertionError("both library baselines already exist"),
            ),
        ):
            selected = api.prepare(self.root, self.context, self.event, None, "", False)
            explicit = api.prepare(
                self.root, self.context, self.event, "private-support", "", False
            )
        self.assertEqual([check["package"] for check in selected["checks"]], ["example"])
        self.assertEqual(explicit["checks"][0]["package"], "private-support")
        self.assertEqual(explicit["checks"][0]["kind"], "git")

    def test_private_library_with_canonical_publication_uses_published_comparison(self) -> None:
        context, _event = self.private_library()
        self.original_private_command = api.command
        with (
            patch.object(api, "command", side_effect=self.private_metadata_command),
            patch.object(cargo_tools, "select", return_value=self.tool),
            patch.object(api, "registry_version", return_value="0.1.0") as canonical,
        ):
            evidence = api.prepare(
                self.root,
                replace(context, purpose="published", event="local"),
                {},
                "kubernetes-lens",
                "",
                False,
            )
        canonical.assert_called_once_with("kubernetes-lens")
        check = evidence["checks"][0]
        self.assertEqual(check["kind"], "published")
        self.assertEqual(check["baseline_version"], "0.1.0")
        command = api.argv_for(self.root, check)
        self.assertEqual(command[command.index("--package") + 1], "kubernetes-lens")
        self.assertIn("--baseline-version", command)
        self.assertNotIn("--release-type", command)

    def test_private_published_absence_requires_and_executes_successful_positive_task(self) -> None:
        context, _event = self.private_library()
        self.original_private_command = api.command
        config = self.root / "target/private-policy.json"
        config.parent.mkdir()
        marker = self.root / "target/positive-private-api"
        positive = [
            sys.executable,
            "-c",
            "from pathlib import Path; import tomllib; "
            "p=tomllib.loads(Path('Cargo.toml').read_text())['package']; "
            "assert p['publish'] is False; "
            "assert 'pub fn already_changed_on_main' in Path('api.rs').read_text(); "
            "Path('target/positive-private-api').write_text('validated')",
        ]
        policy = {
            "schema": 2,
            "repository": context.repository,
            "jobs": ["initial-api-contract"],
            "profile_jobs": {"maintenance": ["initial-api-contract"]},
            "public_documentation_root": "docs/",
            "full_documentation_roots": [],
            "executable_documentation_roots": [],
            "executable_documentation_paths": [],
            "documentation_examples_manifest": None,
            "path_profiles": {"maintenance": ["README.md"]},
            "local_commands": {"maintenance": [["true"]], "full": [["true"]]},
            "tasks": {"initial-api-contract": {"commands": [positive], "timeout_seconds": 5}},
        }
        config.write_text(json.dumps(policy))
        report = self.root / "target/private-api-report.json"
        arguments = [
            "run",
            "--repository-root",
            str(self.root),
            "--repository",
            context.repository,
            "--api-purpose",
            "published",
            "--event",
            "local",
            "--package",
            "kubernetes-lens",
            "--automation-sha",
            api.git(Path(api.__file__).resolve().parents[3], "rev-parse", "HEAD"),
            "--config",
            str(config),
            "--report-json",
            str(report),
        ]
        absent = HTTPError(
            "https://crates.io/api/v1/crates/kubernetes-lens", 404, "not found", Message(), None
        )
        with (
            patch.object(api, "command", side_effect=self.private_metadata_command),
            patch.object(cargo_tools, "select", return_value=self.tool),
            patch.object(cargo_tools, "verify", return_value=Path("/verified/cargo-semver-checks")),
            patch.object(api, "urlopen", side_effect=absent),
            contextlib.redirect_stdout(io.StringIO()),
            contextlib.redirect_stderr(io.StringIO()),
        ):
            self.assertEqual(
                api.main(arguments), 1, "canonical absence alone cannot waive positive API contract"
            )
            self.assertFalse(marker.exists())
            self.assertEqual(api.main([*arguments, "--allow-initial"]), 0)
            self.assertEqual(marker.read_text(), "validated")
            self.assertEqual(
                json.loads((self.root / "target/automation-initial-api-contract.json").read_text())[
                    "status"
                ],
                "success",
            )
            policy["tasks"]["initial-api-contract"]["commands"] = [
                [sys.executable, "-c", "raise SystemExit(7)"]
            ]
            config.write_text(json.dumps(policy))
            self.assertEqual(api.main([*arguments, "--allow-initial"]), 1)
            self.assertEqual(json.loads(report.read_text())["status"], "failure")
            self.assertNotEqual(
                json.loads((self.root / "target/automation-initial-api-contract.json").read_text())[
                    "status"
                ],
                "success",
            )

    def test_initial_package_requires_both_genuine_absence_and_positive_contract(self) -> None:
        for canonical, allow, expected in (
            ("0.1.0", False, "published"),
            (None, True, "initial-contract"),
        ):
            with (
                self.subTest(canonical=canonical),
                patch.object(cargo_tools, "select", return_value=self.tool),
                patch.object(api, "metadata", side_effect=[self.packages, {}]),
                patch.object(api, "registry_version", return_value=canonical),
            ):
                evidence = api.prepare(self.root, self.context, self.event, None, "", allow)
                self.assertEqual(evidence["checks"][0]["kind"], expected)
        with (
            patch.object(cargo_tools, "select", return_value=self.tool),
            patch.object(api, "metadata", side_effect=[self.packages, {}]),
            patch.object(api, "registry_version", return_value=None),
            self.assertRaisesRegex(ContractError, "positive"),
        ):
            api.prepare(self.root, self.context, self.event, None, "", False)

    def test_finish_rejects_source_mutation_and_development_release_substitution(self) -> None:
        with (
            patch.object(cargo_tools, "select", return_value=self.tool),
            patch.object(api, "metadata", return_value=self.packages),
            patch.object(cargo_tools, "verify", return_value=Path("/verified/cargo-semver-checks")),
        ):
            evidence = api.prepare(self.root, self.context, self.event, None, "", False)
            self.assertEqual(
                api.finish(self.root, evidence, self.root)["published_api_candidate_sha"], ""
            )
            (self.root / "api.rs").write_text("mutation during check")
            with self.assertRaisesRegex(ContractError, "changed"):
                api.finish(self.root, evidence, self.root)

    def test_completion_rechecks_the_same_verified_automation_source(self) -> None:
        with (
            patch.object(cargo_tools, "select", return_value=self.tool),
            patch.object(
                api,
                "metadata",
                return_value=self.packages,
            ),
        ):
            evidence = api.prepare(self.root, self.context, self.event, None, "", False)
        evidence["automation_source"] = {
            "revision": self.context.automation,
            "kind": "interface-1-archive",
        }
        with (
            patch.object(
                api,
                "automation_identity",
                return_value={
                    "revision": self.context.automation,
                    "kind": "producer-working-tree",
                },
            ),
            self.assertRaisesRegex(ContractError, "implementation changed"),
        ):
            api.finish(self.root, evidence, self.root)

    def test_real_cli_run_backend_reports_additions_and_breaks_independent_of_version(self) -> None:
        original_command = api.command
        versions = {"example": {"version": "1.0.0"}}

        def backend(
            root: Path,
            argv: tuple[str, ...],
            *,
            timeout: int = 30,
            environment: Mapping[str, str] | None = None,
        ) -> bytes:
            if argv[:3] != ("cargo", "semver-checks", "check-release"):
                return original_command(root, argv, timeout=timeout, environment=environment)
            self.assertIsNotNone(environment)
            self.assertEqual(environment.get("RUSTUP_TOOLCHAIN") if environment else None, "1.98.1")
            base = argv[argv.index("--baseline-rev") + 1]
            old = self.git("show", f"{base}:api.rs")
            new = (root / "api.rs").read_text()
            removed = set(re.findall(r"pub fn (\w+)", old)) - set(re.findall(r"pub fn (\w+)", new))
            allowance = argv[argv.index("--release-type") + 1]
            if removed and allowance != "major":
                raise api.APICommandError(
                    "removed public API sentinel",
                    {"stdout": "removed public API sentinel", "status": "failed"},
                )
            return b"compatible additions preserved\n"

        with (
            patch.object(api, "command", side_effect=backend),
            patch.object(api, "default_authority", return_value=("main", self.main)),
            patch.object(cargo_tools, "select", return_value=self.tool),
            patch.object(api, "metadata", return_value=versions),
            patch.object(cargo_tools, "verify", return_value=Path("/verified/cargo-semver-checks")),
            contextlib.redirect_stdout(io.StringIO()),
            contextlib.redirect_stderr(io.StringIO()),
        ):
            arguments = [
                "run",
                "--repository-root",
                str(self.root),
                "--api-purpose",
                "development",
                "--event",
                "local",
                "--report-json",
                str(self.root / "target/report.json"),
            ]
            self.assertEqual(
                api.main(arguments),
                0,
                "preexisting published break must not block workflow adoption",
            )
            (self.root / "api.rs").write_text(
                "pub fn already_changed_on_main() {}\npub fn additive() {}\n"
            )
            self.assertEqual(api.main(arguments), 0, "compatible additions are allowed by minor")
            (self.root / "Cargo.toml").write_text('[package]\nname="example"\nversion="1.0.0"\n')
            (self.root / "api.rs").write_text("pub fn replaced() {}\n")
            self.assertEqual(
                api.main(arguments), 1, "candidate manifest bump must not waive a new break"
            )
            report = json.loads((self.root / "target/report.json").read_text())
            self.assertEqual(report["command_failure"]["stdout"], "removed public API sentinel")
            self.assertEqual(api.main([*arguments, "--release-type", "major"]), 0)

    def test_positive_initial_task_is_executed_and_failure_cannot_emit_success(self) -> None:
        original_command = api.command
        config = self.root / "target/policy.json"
        config.parent.mkdir()
        config.write_text("{}")
        report = self.root / "target/initial-report.json"
        arguments = [
            "run",
            "--repository-root",
            str(self.root),
            "--api-purpose",
            "published",
            "--event",
            "local",
            "--allow-initial",
            "--config",
            str(config),
            "--report-json",
            str(report),
        ]
        seen: list[tuple[str, ...]] = []
        outcome = {"succeeds": True}
        for succeeds in (True, False):
            seen.clear()
            outcome["succeeds"] = succeeds

            def backend(
                root: Path,
                argv: tuple[str, ...],
                *,
                timeout: int = 30,
                environment: Mapping[str, str] | None = None,
            ) -> bytes:
                if "initial-api-contract" not in argv:
                    return original_command(root, argv, timeout=timeout, environment=environment)
                seen.append(argv)
                self.assertEqual(argv[argv.index("--task") + 1], "initial-api-contract")
                self.assertIn(str(config), argv)
                if not outcome["succeeds"]:
                    raise ContractError("positive initial API contract failed")
                return b"positive contract evidence\n"

            with (
                self.subTest(succeeds=succeeds),
                patch.object(api, "command", side_effect=backend),
                patch.object(cargo_tools, "select", return_value=self.tool),
                patch.object(api, "metadata", return_value=self.packages),
                patch.object(api, "registry_version", return_value=None),
                patch.object(
                    cargo_tools, "verify", return_value=Path("/verified/cargo-semver-checks")
                ),
                contextlib.redirect_stdout(io.StringIO()),
                contextlib.redirect_stderr(io.StringIO()),
            ):
                self.assertEqual(api.main(arguments), 0 if succeeds else 1)
            self.assertEqual(len(seen), 1)
            self.assertEqual(
                json.loads(report.read_text())["status"], "success" if succeeds else "failure"
            )

    def test_published_success_requires_clean_exact_source_and_chosen_manifest(self) -> None:
        context = replace(self.context, purpose="published", event="local")
        with (
            patch.object(cargo_tools, "select", return_value=self.tool),
            patch.object(api, "metadata", return_value=self.packages),
            patch.object(api, "registry_version", return_value="0.1.0"),
            patch.object(cargo_tools, "verify", return_value=Path("/verified/cargo-semver-checks")),
        ):
            evidence = api.prepare(self.root, context, {}, None, "", False)
            pristine = json.loads(json.dumps(evidence))
            self.assertEqual(
                api.finish(self.root, evidence, self.root)["published_api_candidate_sha"],
                self.candidate,
            )
            (self.root / "Cargo.toml").write_text('[package]\nname="example"\nversion="1.0.0"\n')
            with self.assertRaisesRegex(ContractError, "source changed"):
                api.finish(self.root, pristine, self.root)
            with self.assertRaisesRegex(ContractError, "clean exact"):
                api.prepare(self.root, context, {}, None, "", False)

    def test_hosted_selection_digest_cannot_be_rewritten_by_candidate_build(self) -> None:
        report = self.root / "target/report.json"
        report.parent.mkdir()
        evidence = {"context": {"purpose": "development"}, "status": "selected"}
        expected = api.selection_digest(evidence)
        evidence["context"]["purpose"] = "published"
        report.write_text(json.dumps(evidence))
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(
                api.main(
                    [
                        "finish",
                        "--repository-root",
                        str(self.root),
                        "--report-json",
                        str(report),
                        "--selection-sha256",
                        expected,
                    ]
                ),
                1,
            )
        self.assertIn("immutable pre-check", json.loads(report.read_text())["error"])

    def test_sigterm_reaps_actual_owned_child_group_and_retains_cancellation(self) -> None:
        pidfile = self.root / "target/owned-child.pid"
        pidfile.parent.mkdir()
        report = self.root / "target/terminated.json"
        child = (
            "import os,signal,time; "
            "signal.signal(signal.SIGTERM,signal.SIG_IGN); "
            f"open({str(pidfile)!r},'w').write(str(os.getpid())); time.sleep(10)"
        )
        script = (
            "from pathlib import Path; import sys; "
            "from boxferry_ci import api_baselines as api; "
            "api.automation_identity=lambda *a,**k: {'revision':'a'*40,'kind':'cancel-fixture'}; "
            f"api.prepare=lambda *a,**k: api.command(Path({str(self.root)!r}),"
            f"(sys.executable,'-c',{child!r}),timeout=10); "
            f"raise SystemExit(api.main(['run','--repository-root',{str(self.root)!r},"
            f"'--api-purpose','development','--event','local','--report-json',{str(report)!r}]))"
        )
        environment = dict(os.environ, PYTHONPATH=str(Path(api.__file__).resolve().parents[1]))
        process = subprocess.Popen(
            [sys.executable, "-c", script],
            env=environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        child_pid = None
        try:
            deadline = time.monotonic() + 3
            while not pidfile.exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            self.assertTrue(
                pidfile.exists(), "owned child did not start within bounded test budget"
            )
            child_pid = int(pidfile.read_text())
            process.send_signal(signal.SIGTERM)
            process.communicate(timeout=5)
            self.assertEqual(process.returncode, 130)
            evidence = json.loads(report.read_text())
            self.assertEqual(evidence["status"], "cancelled")
            self.assertEqual(evidence["command_failure"]["status"], "cancelled")
            with self.assertRaises(ProcessLookupError):
                os.kill(child_pid, 0)
        finally:
            if process.poll() is None:
                process.kill()
                process.communicate(timeout=5)
            if child_pid is not None:
                try:
                    os.kill(child_pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass

    def test_cancelled_or_failed_compatibility_produces_no_success_binding(self) -> None:
        for status in ("failed", "cancelled", "timeout"):
            result = RunResult(
                status,
                1,
                "actionable stdout API finding",
                "build progress",
                0.1,
                Diagnostic("API", status, "candidate", "report", "inspect API"),
                ("cargo", "semver-checks"),
            )
            with (
                self.subTest(status=status),
                patch.object(api, "run", return_value=result),
                self.assertRaises(api.APICommandError) as raised,
            ):
                api.command(self.root, ("cargo", "semver-checks"))
            self.assertIn("actionable stdout API finding", str(raised.exception))
            self.assertNotIn("git fetch", str(raised.exception))
            self.assertEqual(raised.exception.diagnostic["status"], status)
        with (
            patch.object(api, "prepare", side_effect=RunCancelled()),
            contextlib.redirect_stderr(io.StringIO()),
        ):
            report = self.root / "target/cancelled.json"
            self.assertEqual(
                api.main(
                    [
                        "run",
                        "--repository-root",
                        str(self.root),
                        "--api-purpose",
                        "development",
                        "--event",
                        "local",
                        "--report-json",
                        str(report),
                    ]
                ),
                130,
            )
            self.assertEqual(json.loads(report.read_text())["status"], "cancelled")

    def test_cancellation_fixture_is_local_but_default_hosted_candidate_stays_exact(self) -> None:
        hosted = {
            "PATH": os.defpath,
            "GITHUB_ACTIONS": "true",
            "GITHUB_EVENT_NAME": "pull_request",
            "GITHUB_SHA": "0" * 40,
            "GITHUB_REPOSITORY": "Strukturpiloten/boxferry",
            "GITHUB_EVENT_PATH": str(self.root / "must-not-read-event.json"),
        }
        self.assertNotEqual(self.candidate, hosted["GITHUB_SHA"])
        for explicit_local in (True, False):
            report = self.root / f"target/hosted-cancellation-{explicit_local}.json"
            arguments = [
                "run",
                "--repository-root",
                str(self.root),
                "--api-purpose",
                "development",
                "--report-json",
                str(report),
            ]
            if explicit_local:
                arguments.extend(("--event", "local"))
            with (
                self.subTest(explicit_local=explicit_local),
                patch.dict(os.environ, hosted, clear=True),
                patch.object(api, "prepare", side_effect=RunCancelled()) as prepare,
                patch.object(api, "finish") as finish,
                patch.object(api, "write_outputs") as write_outputs,
                contextlib.redirect_stderr(io.StringIO()) as stderr,
            ):
                result = api.main(arguments)
                evidence = json.loads(report.read_text())
                self.assertEqual(
                    result,
                    130 if explicit_local else 1,
                    f"report={str(evidence)[:2000]}; stderr={stderr.getvalue()[:2000]}",
                )
                finish.assert_not_called()
                write_outputs.assert_not_called()
                if explicit_local:
                    prepare.assert_called_once()
                    self.assertEqual(evidence["status"], "cancelled")
                    self.assertNotIn("error", evidence)
                else:
                    prepare.assert_not_called()
                    self.assertEqual(evidence["status"], "failure")
                    self.assertEqual(
                        evidence["error"], "hosted API candidate must equal exact GITHUB_SHA"
                    )


class PolicyCases(unittest.TestCase):
    def test_development_minor_allows_additions_but_never_infers_break_from_version(self) -> None:
        for candidate_version in ("0.1.0", "0.2.0", "1.0.0"):
            check = {
                "package": "example",
                "candidate_version": candidate_version,
                "kind": "git",
                "baseline_rev": "a" * 40,
                "baseline_version": "",
                "release_type": api.release_allowance("development", ""),
            }
            argv = api.argv_for(Path("/candidate"), check)
            self.assertEqual(argv[-2:], ("--release-type", "minor"))
            # Independent rule: additions need minor; public API removal needs major.
            for required, succeeds in (("minor", True), ("major", False)):
                self.assertEqual(
                    {"patch": 0, "minor": 1, "major": 2}[argv[-1]]
                    >= {"minor": 1, "major": 2}[required],
                    succeeds,
                )
        self.assertEqual(api.release_allowance("development", "major"), "major")

    def test_published_uses_manifest_inference_and_rejects_all_intent_overrides(self) -> None:
        self.assertEqual(api.release_allowance("published", ""), "")
        for override in ("major", "minor", "patch", "unexpected"):
            with self.subTest(override=override), self.assertRaises(ContractError):
                api.release_allowance("published", override)
        check = {
            "package": "example",
            "kind": "published",
            "baseline_version": "0.1.0",
            "release_type": "",
        }
        argv = api.argv_for(Path("/candidate"), check)
        self.assertIn("--baseline-version", argv)
        self.assertNotIn("--release-type", argv)

    def test_wrong_purpose_repository_identity_and_initial_shortcut_fail(self) -> None:
        for change in (
            {"purpose": "release-ish"},
            {"candidate": "main"},
            {"repository": "other/repo"},
        ):
            with self.subTest(change=change), self.assertRaises(ContractError):
                replace(
                    api.Context(
                        "published",
                        "local",
                        "a" * 40,
                        "b" * 40,
                        "c" * 40,
                        "Strukturpiloten/boxferry",
                    ),
                    **change,
                ).validate()
        with self.assertRaisesRegex(ContractError, "positive"):
            api.argv_for(Path("/candidate"), {"kind": "initial-contract"})

    def registry(self, rows: list[dict[str, str | bool]]) -> str | None:
        class Response:
            status = 200
            url = "https://crates.io/api/v1/crates/example"

            def __enter__(self) -> Response:
                return self

            def __exit__(self, *_args: object) -> None:
                pass

            def read(self, _limit: int) -> bytes:
                return json.dumps({"crate": {"name": "example"}, "versions": rows}).encode()

        with patch.object(api, "urlopen", return_value=Response()):
            return api.registry_version("example")

    def test_registry_selects_exact_canonical_latest_usable_version(self) -> None:
        rows = [
            {"crate": "example", "num": version, "yanked": yanked}
            for version, yanked in (
                ("0.1.0", False),
                ("0.2.0", False),
                ("1.0.0", True),
                ("2.0.0-alpha.1", False),
            )
        ]
        self.assertEqual(self.registry(rows), "0.2.0")
        for history in (
            [],
            [rows[2]],
            [rows[3]],
            [{"crate": "forged", "num": "1.0.0", "yanked": False}],
        ):
            with self.subTest(history=history), self.assertRaises(ContractError):
                self.registry(history)

    def test_only_canonical_404_establishes_absence(self) -> None:
        url = "https://crates.io/api/v1/crates/example"
        with patch.object(
            api, "urlopen", side_effect=HTTPError(url, 404, "not found", Message(), None)
        ):
            self.assertIsNone(api.registry_version("example"))
        for error in (
            HTTPError(url, 500, "failed", Message(), None),
            HTTPError("https://other.invalid", 404, "forged", Message(), None),
            URLError("offline"),
            TimeoutError(),
        ):
            with (
                self.subTest(error=error),
                patch.object(api, "urlopen", side_effect=error),
                self.assertRaises(ContractError),
            ):
                api.registry_version("example")

    def test_canonical_manual_authority_does_not_need_origin_head_alias(self) -> None:
        context = api.Context(
            "development",
            "workflow_dispatch",
            "a" * 40,
            "b" * 40,
            "c" * 40,
            "Strukturpiloten/boxferry",
        )
        root = Path("/candidate")

        def authority(cwd: Path, *args: str) -> str:
            if args == ("remote", "get-url", "origin"):
                return "https://github.com/Strukturpiloten/boxferry.git"
            if "ls-remote" in args:
                self.assertNotEqual(
                    cwd, root, "candidate Git config must not control canonical authority"
                )
                self.assertIn("https://github.com/Strukturpiloten/boxferry.git", args)
                return "ref: refs/heads/main\tHEAD\n" + "d" * 40 + "\tHEAD"
            if args == ("rev-parse", "--verify", "refs/remotes/origin/main"):
                return "d" * 40
            if args[0] == "rev-list":
                return "complete graph"
            raise AssertionError(f"unexpected authority command (including symbolic alias): {args}")

        with patch.object(api, "git", side_effect=authority):
            self.assertEqual(api.default_authority(root, context), ("main", "d" * 40))
        for missing in ("", "ref: refs/heads/main\tHEAD"):
            with (
                patch.object(
                    api,
                    "git",
                    side_effect=["https://github.com/Strukturpiloten/boxferry.git", missing],
                ),
                self.assertRaisesRegex(ContractError, "authority"),
            ):
                api.default_authority(root, context)
        with (
            patch.object(
                api,
                "git",
                side_effect=[
                    "https://github.com/Strukturpiloten/boxferry.git",
                    "ref: refs/heads/main\tHEAD\n" + "d" * 40 + "\tHEAD",
                    "e" * 40,
                ],
            ),
            self.assertRaisesRegex(ContractError, "git fetch"),
        ):
            api.default_authority(root, context)

    def test_original_workflow_dispatch_context_retains_explicit_reusable_purpose(self) -> None:
        # GitHub calls keep the original event; only the call input distinguishes these cases.
        for event, supplied, expected in (
            ("workflow_dispatch", "", "development"),
            ("workflow_dispatch", "published", "published"),
            ("workflow_dispatch", "development", "development"),
            ("push", "published", "published"),
        ):
            with self.subTest(event=event, supplied=supplied):
                self.assertEqual(supplied or "development", expected)
        root = Path(__file__).resolve().parents[2]
        ci = (root / ".github/workflows/ci.yml").read_text()
        self.assertIn("api_purpose: ${{ inputs.api_purpose || 'development' }}", ci)
        self.assertNotIn("github.event_name == 'workflow_call'", ci)
        shared = (root / ".github/workflows/shared-rust-semver.yml").read_text()
        self.assertIn("SELECTION_SHA256: ${{ steps.baseline.outputs.selection_sha256 }}", shared)
        self.assertIn('--selection-sha256 "${SELECTION_SHA256}"', shared)

    def test_workflow_release_guards_and_legacy_default_are_explicit(self) -> None:
        root = Path(__file__).resolve().parents[2]
        shared = (root / ".github/workflows/shared-rust-semver.yml").read_text()
        ci = (root / ".github/workflows/ci.yml").read_text()
        release = (root / ".github/workflows/release.yml").read_text()
        self.assertIn("default: published", shared)
        self.assertIn("baseline-rev: ${{ steps.baseline.outputs.baseline_rev }}", shared)
        self.assertIn("baseline-version: ${{ steps.baseline.outputs.baseline_version }}", shared)
        self.assertIn("api_purpose: ${{ inputs.api_purpose || 'development' }}", ci)
        self.assertIn("api_purpose: published", release)
        self.assertIn('"${PUBLISHED_API_CANDIDATE}" != "${GITHUB_SHA}"', release)
        guard = release[release.index("  release-validation:") : release.index("  publish:")]
        self.assertNotIn("validation_only", guard)
        aggregate = guard.split("  release-prepare:", 1)[0]
        manifest = json.loads((root / ".github/automation.json").read_text())
        self.assertRegex(manifest["revision"], r"^[a-f0-9]{40}$")
        self.assertIn(
            "uses: Strukturpiloten/boxferry/.github/workflows/shared-release-validation.yml@"
            + manifest["revision"],
            aggregate,
        )
        self.assertIn(f"automation_sha: {manifest['revision']}", aggregate)
        self.assertIn("needs_json: ${{ toJSON(needs) }}", aggregate)
        self.assertIn("config_path: scripts/shared-release-policy.json", aggregate)
        self.assertIn("needs.release-validation.outputs.published_api_candidate_sha", guard)
        self.assertIn("needs.release-validation.outputs.candidate_sha", guard)
        self.assertNotIn("published_api_candidate_sha: ${{ github.sha }}", guard)


if __name__ == "__main__":
    unittest.main()
