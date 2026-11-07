"""Exact declarations and independent Cargo dispatch/failure expectations; no builds."""

from __future__ import annotations

import contextlib
import io
import json
import os
import shutil
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path
from unittest.mock import patch

from boxferry_ci import cargo_tools as subject
from boxferry_ci.contracts import ContractError


class CargoTools(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.repo = self.root / "candidate"
        (self.repo / ".devcontainer").mkdir(parents=True)
        (self.repo / "rust-toolchain.toml").write_text('[toolchain]\nchannel="1.98.1"\n')
        self.metadata = self.repo / subject.DOCKERFILE
        self.metadata.write_text(
            "# renovate: datasource=crate depName=cargo-semver-checks\n"
            "ARG CARGO_SEMVER_CHECKS_VERSION=1.2.3\n"
        )
        self.selection = subject.select(self.repo, "cargo-semver-checks")
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.cargo = self.bin / "cargo"
        self.cargo.write_text("#!/bin/sh\nexit 1\n")
        self.cargo.chmod(0o755)
        self.plugin = self.bin / "cargo-semver-checks"
        self.plugin.write_text("#!/bin/sh\nprintf 'cargo-semver-checks 1.2.3\\n'\n")
        self.plugin.chmod(0o755)
        self.rustup = self.bin / "rustup"
        self.rustup.write_text("#!/bin/sh\nexit 1\n")
        self.rustup.chmod(0o755)
        self.rustc = self.bin / "rustc"
        self.rustc.write_text("#!/bin/sh\nexit 1\n")
        self.rustc.chmod(0o755)
        self.environment = {
            **os.environ,
            "PATH": str(self.bin),
            "CARGO_HOME": str(self.root / "cargo-home"),
            "RUSTUP_TOOLCHAIN": "stable",
        }
        self.releases = [
            "cargo 1.98.1 (fixture 2026-10-01)\n",
            "rustc 1.98.1 (fixture 2026-10-01)\n",
        ]

    def test_candidate_versions_are_not_replaced_by_shared_literal(self) -> None:
        for version in ("0.50.0", "0.51.0", "2.0.0"):
            self.metadata.write_text(
                self.metadata.read_text().split("ARG")[0]
                + f"ARG CARGO_SEMVER_CHECKS_VERSION={version}\n"
            )
            self.assertEqual(subject.select(self.repo, "cargo-semver-checks").version, version)

    def test_only_absent_dockerfile_can_use_held_shared_fallback(self) -> None:
        fallback = self.root / "shared"
        (fallback / ".devcontainer").mkdir(parents=True)
        shutil.copyfile(self.metadata, fallback / subject.DOCKERFILE)
        self.metadata.unlink()
        self.assertEqual(subject.select(self.repo, "cargo-semver-checks", fallback), self.selection)
        for text in ("FROM example\n", "ARG CARGO_SEMVER_CHECKS_VERSION=latest\n", ""):
            self.metadata.write_text(text)
            with self.subTest(text=text), self.assertRaises(ContractError):
                subject.select(self.repo, "cargo-semver-checks", fallback)

    def test_invalid_duplicate_unowned_and_linked_declarations_fail(self) -> None:
        valid = self.metadata.read_text()
        for text in (
            valid + valid,
            valid.replace("1.2.3", "^1.2.3"),
            valid.replace("1.2.3", "${VERSION}"),
            valid.replace("1.2.3", "01.2.3"),
            valid.replace("depName=cargo-semver-checks", "depName=other"),
            valid + "ARG CARGO_SEMVER_CHECKS_VERSION=1.2.4\n",
        ):
            self.metadata.write_text(text)
            with self.subTest(text=text), self.assertRaises(ContractError):
                subject.select(self.repo, "cargo-semver-checks")
        self.metadata.unlink()
        self.metadata.symlink_to(self.plugin)
        with self.assertRaises(ContractError):
            subject.select(self.repo, "cargo-semver-checks")

    def test_floating_or_nonstring_rust_channel_is_not_accepted(self) -> None:
        for value in ('"stable"', '"nightly-2026-01-01"', '"1.98"', "true", '"1.98.1\\n"'):
            (self.repo / "rust-toolchain.toml").write_text(f"[toolchain]\nchannel={value}\n")
            with self.subTest(channel=value), self.assertRaises(ContractError):
                subject.select(self.repo, "cargo-semver-checks")

    def test_compiler_and_invoked_plugin_versions_have_to_match(self) -> None:
        with patch.object(
            subject, "command", side_effect=[*self.releases, "cargo-semver-checks 1.2.3\n"]
        ) as command:
            self.assertEqual(
                subject.verify(self.selection, self.repo, self.environment), self.plugin
            )
            self.assertEqual(
                command.call_args.args[0], (str(self.cargo), "semver-checks", "--version")
            )
            self.assertEqual(command.call_args.args[2]["RUSTUP_TOOLCHAIN"], "1.98.1")
        for output in ("cargo-semver-checks 1.2.4\n", "cargo-semver-checks 1.2.3\nextra\n", ""):
            with (
                patch.object(subject, "command", side_effect=[*self.releases, output]),
                self.assertRaises(ContractError),
            ):
                subject.verify(self.selection, self.repo, self.environment)
        for releases in (
            ["cargo 1.99.0 (fixture)\n"],
            [self.releases[0], "rustc 1.97.0 (fixture)\n"],
        ):
            with (
                patch.object(subject, "command", side_effect=releases),
                self.assertRaisesRegex(ContractError, "candidate Rust"),
            ):
                subject.verify(self.selection, self.repo, self.environment)

    def test_missing_rustup_cannot_enter_action_bootstrap_path(self) -> None:
        self.rustup.unlink()
        with patch.object(subject, "command") as command, self.assertRaises(ContractError):
            subject.verify(self.selection, self.repo, self.environment)
        command.assert_not_called()

    def test_cargo_home_shadow_and_wrong_expected_installation_are_rejected(self) -> None:
        other = self.root / "other-plugin"
        other.write_text("different plugin")
        with self.assertRaises(ContractError):
            subject.verify(self.selection, self.repo, self.environment, other)
        home = self.root / "cargo-home"
        (home / "bin").mkdir(parents=True)
        shutil.copyfile(self.plugin, home / "bin/cargo-semver-checks")
        with self.assertRaisesRegex(ContractError, "Cargo home shadows"):
            subject.verify(self.selection, self.repo, {**self.environment, "CARGO_HOME": str(home)})

    def test_command_enumeration_is_not_used_as_dispatch_proof(self) -> None:
        with patch.object(
            subject, "command", side_effect=[*self.releases, "cargo-semver-checks 1.2.3\n"]
        ) as command:
            self.assertEqual(
                subject.verify(self.selection, self.repo, self.environment), self.plugin
            )
        self.assertTrue(all("--list" not in call.args[0] for call in command.call_args_list))

    def test_environment_and_config_overrides_fail_before_invoking_cargo(self) -> None:
        for variable in ("CARGO_ALIAS_SEMVER_CHECKS", *subject.COMPILER_ENV):
            with patch.object(subject, "command") as command, self.assertRaises(ContractError):
                subject.verify(
                    self.selection, self.repo, {**self.environment, variable: "replacement"}
                )
            command.assert_not_called()
        (self.repo / ".cargo").mkdir()
        for name in ("config", "config.toml"):
            for key in subject.COMPILER_KEYS:
                for directory in (self.repo, self.root):
                    (directory / ".cargo").mkdir(exist_ok=True)
                    path = directory / ".cargo" / name
                    path.write_text(f'[build]\n{key}="replacement"\n')
                    with (
                        patch.object(subject, "command") as command,
                        self.assertRaises(ContractError),
                    ):
                        subject.verify(self.selection, self.repo, self.environment)
                    command.assert_not_called()
                    path.unlink()

    def test_config_environment_cannot_replace_tools_after_preflight(self) -> None:
        (self.repo / ".cargo").mkdir()
        path = self.repo / ".cargo/config.toml"
        for variable in (
            *subject.COMPILER_ENV,
            "PATH",
            "CARGO_HOME",
            "RUSTUP_TOOLCHAIN",
            "CARGO_ALIAS_SEMVER_CHECKS",
            "CARGO_ALIAS_SEMVER-CHECKS",
        ):
            for value in ('"replacement"', '{value="replacement",force=true}'):
                path.write_text(f"[env]\n{variable}={value}\n")
                with patch.object(subject, "command") as command, self.assertRaises(ContractError):
                    subject.verify(self.selection, self.repo, self.environment)
                command.assert_not_called()
        path.write_text('include=["hidden.toml"]\n')
        with patch.object(subject, "command") as command, self.assertRaises(ContractError):
            subject.verify(self.selection, self.repo, self.environment)
        command.assert_not_called()

    def test_relative_path_and_home_use_the_invocation_cwd(self) -> None:
        environment = {**self.environment, "PATH": "../bin", "CARGO_HOME": "../cargo-home"}
        with patch.object(
            subject, "command", side_effect=[*self.releases, "cargo-semver-checks 1.2.3\n"]
        ) as command:
            self.assertEqual(subject.verify(self.selection, self.repo, environment), self.plugin)
        self.assertTrue(all(call.args[1] == self.repo for call in command.call_args_list))
        # Candidate metadata need not be the Action's execution directory.
        with (
            patch.object(subject, "verify") as verify,
            patch.object(subject.Path, "cwd", return_value=self.root),
            contextlib.redirect_stdout(io.StringIO()),
        ):
            self.assertEqual(
                subject.main(
                    [
                        "verify",
                        "--repository-root",
                        str(self.repo),
                        "--package",
                        "cargo-semver-checks",
                    ]
                ),
                0,
            )
        self.assertEqual(verify.call_args.args[1], self.root)

    def test_exact_locked_install_never_uses_candidate_registry_configuration(self) -> None:
        (self.repo / ".cargo").mkdir()
        (self.repo / ".cargo/config.toml").write_text(
            '[source.crates-io]\nreplace-with="untrusted"\n'
            '[source.untrusted]\nregistry="https://invalid.example/index"\n'
        )
        calls: list[tuple[tuple[str, ...], Path, dict[str, str], int]] = []

        def command(
            argv: tuple[str, ...], cwd: Path, environment: dict[str, str], timeout: int = 30
        ) -> str:
            calls.append((argv, cwd, dict(environment), timeout))
            home = Path(environment["CARGO_HOME"])
            binary = home / "bin/cargo-semver-checks"
            if "install" in argv:
                binary.parent.mkdir()
                binary.write_text("installed fixture")
                binary.chmod(0o755)
                return ""
            if Path(argv[0]).name == "cargo" and argv[1:] == ("--version",):
                return self.releases[0]
            if Path(argv[0]).name == "rustc":
                return self.releases[1]
            return "cargo-semver-checks 1.2.3\n"

        with (
            patch.dict(os.environ, self.environment),
            patch.object(subject, "command", side_effect=command),
        ):
            result = subject.install(self.selection, self.root / "owned", self.root)
        install, cwd, environment, timeout = calls[0]
        self.assertEqual(
            install[1:],
            (
                "install",
                "--locked",
                "--force",
                "--version",
                "=1.2.3",
                "--registry",
                "crates-io",
                "--root",
                result["CARGO_HOME"],
                "cargo-semver-checks",
            ),
        )
        self.assertFalse(cwd.is_relative_to(self.repo))
        self.assertEqual(environment["RUSTUP_TOOLCHAIN"], "1.98.1")
        self.assertEqual(timeout, 900)
        self.assertEqual(calls[1][2]["CARGO_HOME"], result["CARGO_HOME"])
        self.assertTrue(all(call[1] == self.root for call in calls[1:]))
        self.assertEqual(calls[-1][2]["PATH"].split(os.pathsep)[0], result["bin_directory"])

    def test_install_failure_cannot_publish_action_setup_outputs(self) -> None:
        output = self.root / "github-output"
        with (
            patch.object(subject, "install", side_effect=ContractError("install failed")),
            contextlib.redirect_stderr(io.StringIO()),
        ):
            self.assertEqual(
                subject.main(
                    [
                        "install",
                        "--repository-root",
                        str(self.repo),
                        "--package",
                        "cargo-semver-checks",
                        "--destination",
                        str(self.root / "owned"),
                        "--github-output",
                        str(output),
                    ]
                ),
                1,
            )
        self.assertFalse(output.exists())

    def test_failed_dispatch_verification_cannot_publish_action_setup_outputs(self) -> None:
        output = self.root / "github-output"
        with (
            patch.dict(os.environ, self.environment),
            patch.object(subject, "command", return_value=""),
            contextlib.redirect_stderr(io.StringIO()),
        ):
            self.assertEqual(
                subject.main(
                    [
                        "install",
                        "--repository-root",
                        str(self.repo),
                        "--package",
                        "cargo-semver-checks",
                        "--destination",
                        str(self.root / "owned"),
                        "--github-output",
                        str(output),
                    ]
                ),
                1,
            )
        self.assertFalse(output.exists())

    def test_real_cargo_resolver_rejects_alias_and_cargo_home_shadow_without_building(self) -> None:
        real_cargo = shutil.which("cargo")
        if real_cargo is None:
            self.fail("version-resolution regression requires Cargo, not a compiler/build")
        # Use the installed repository toolchain, so future pin updates need no old compiler.
        (self.repo / "rust-toolchain.toml").write_bytes(
            (Path(__file__).resolve().parents[2] / "rust-toolchain.toml").read_bytes()
        )
        self.selection = subject.select(self.repo, "cargo-semver-checks")
        self.cargo.unlink()
        self.rustc.unlink()
        self.rustup.unlink()
        environment = {
            **os.environ,
            "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
            "CARGO_HOME": str(self.root / "cargo-home"),
        }
        self.assertEqual(subject.verify(self.selection, self.repo, environment), self.plugin)
        later = self.root / "later"
        later.mkdir()
        wrong = later / "cargo-semver-checks"
        wrong.write_text("#!/bin/sh\nprintf 'cargo-semver-checks 9.9.9\\n'\n")
        wrong.chmod(0o755)
        environment["PATH"] += os.pathsep + str(later)
        # Real Cargo's listing may report the later entry: invocation still uses first PATH.
        self.assertEqual(subject.verify(self.selection, self.repo, environment), self.plugin)
        with self.assertRaisesRegex(ContractError, "declared"):
            subject.verify(
                self.selection,
                self.repo,
                {**environment, "PATH": str(later) + os.pathsep + environment["PATH"]},
            )
        (self.repo / ".cargo").mkdir()
        config = self.repo / ".cargo/config.toml"
        config.write_text('[alias]\nsemver-checks="!false"\n')
        with self.assertRaises(ContractError):
            subject.verify(self.selection, self.repo, environment)
        config.unlink()
        shadow = Path(environment["CARGO_HOME"]) / "bin/cargo-semver-checks"
        shadow.parent.mkdir(parents=True)
        shutil.copyfile(self.plugin, shadow)
        shadow.chmod(0o755)
        with self.assertRaises(ContractError):
            subject.verify(self.selection, self.repo, environment)

    def test_shared_workflow_binds_compiler_and_runs_verified_install_before_action(self) -> None:
        root = Path(__file__).resolve().parents[2]
        workflow = (root / ".github/workflows/shared-rust-semver.yml").read_text()
        self.assertLess(
            workflow.index("boxferry_ci.cargo_tools install"), workflow.index("uses: obi1kenobi/")
        )
        self.assertIn("rust-toolchain: ${{ steps.cargo-tool.outputs.rust_channel }}", workflow)
        self.assertIn('--github-env "${GITHUB_ENV}"', workflow)
        ci = (
            (root / ".github/workflows/ci.yml")
            .read_text()
            .split("  semver:\n", 1)[1]
            .split("  migration-readiness:", 1)[0]
        )
        manifest = json.loads((root / ".github/automation.json").read_text())
        self.assertEqual(manifest["repository"], "Strukturpiloten/boxferry")
        self.assertRegex(manifest["revision"], r"^[a-f0-9]{40}$")
        self.assertIn(
            "uses: Strukturpiloten/boxferry/.github/workflows/shared-rust-semver.yml@"
            + manifest["revision"],
            ci,
        )
        self.assertNotIn("uses: ./.github/workflows/shared-rust-semver.yml", ci)
        for token in (
            "candidate_sha: ${{ github.sha }}",
            f"automation_sha: {manifest['revision']}",
            "config_path: scripts/shared-validation-policy.json",
            "policy_sha: ${{ github.event.pull_request.base.sha || "
            "github.event.merge_group.base_sha || github.sha }}",
            "api_purpose: ${{ inputs.api_purpose || 'development' }}",
            "release_type: ${{ inputs.api_purpose != 'published' && "
            "needs.semver-release-type.outputs.release_type || '' }}",
        ):
            self.assertIn(token, ci)

    def test_actual_source_admission_binds_pr_queue_and_complete_candidates(self) -> None:
        root = Path(__file__).resolve().parents[2]
        action = (root / ".github/actions/shared-source/action.yml").read_text()
        script = textwrap.dedent(action.split("      run: |\n", 1)[1].split("    - name:", 1)[0])
        git = self.bin / "git"
        git.write_text('#!/bin/sh\nprintf "%s\\n" "$TEST_AUTOMATION_HEAD"\n')
        git.chmod(0o755)
        for event in ("pull_request", "merge_group", "push", "workflow_dispatch"):
            expected = (
                "a" * 40
                if event == "pull_request"
                else "b" * 40
                if event == "merge_group"
                else "c" * 40
            )
            environment = dict(
                os.environ,
                PATH=str(self.bin) + os.pathsep + os.defpath,
                GITHUB_REPOSITORY="Strukturpiloten/boxferry",
                GITHUB_SHA="c" * 40,
                CANDIDATE_SHA="c" * 40,
                AUTOMATION_SHA="d" * 40,
                TEST_AUTOMATION_HEAD="d" * 40,
                POLICY_SHA=expected,
                PR_BASE_SHA="a" * 40,
                QUEUE_BASE_SHA="b" * 40,
                EVENT_NAME=event,
            )
            with self.subTest(event=event):
                self.assertEqual(
                    subprocess.run(
                        ["bash", "-c", script], env=environment, capture_output=True
                    ).returncode,
                    0,
                )
                for field in ("CANDIDATE_SHA", "POLICY_SHA", "TEST_AUTOMATION_HEAD"):
                    result = subprocess.run(
                        ["bash", "-c", script],
                        env={**environment, field: "e" * 40},
                        capture_output=True,
                    )
                    self.assertNotEqual(result.returncode, 0, field)


if __name__ == "__main__":
    unittest.main()
