"""Authored Git/tool doubles: local identity and failure contracts, never native evidence."""

from __future__ import annotations

import contextlib
import hashlib
import io
import json
import os
import stat
import sys
import tempfile
import time
import unittest
from pathlib import Path
from typing import Any
from unittest.mock import patch

from boxferry_ci import devcontainer_local as subject
from boxferry_ci.__main__ import main as dispatch

CANDIDATE, AUTOMATION, SIBLING = "a" * 40, "b" * 40, "c" * 40
REAL_COMMANDS = subject.validation.Commands

NODE = "24.9.9"  # Synthetic authority; no installation or executable is used.


def policy(repository: str) -> dict[str, object]:
    return {
        "schema": 2,
        "repository": "Strukturpiloten/" + repository,
        "jobs": ["quality"],
        "profile_jobs": {"maintenance": ["quality"]},
        "public_documentation_root": "docs/",
        "full_documentation_roots": [],
        "executable_documentation_roots": [],
        "executable_documentation_paths": [],
        "documentation_examples_manifest": None,
        "path_profiles": {"maintenance": ["README.md"]},
        "local_commands": {"maintenance": [["true"]], "full": [["true"]]},
        "tasks": {"quality": {"commands": [["true"]], "timeout_seconds": 1}},
    }


class LocalEntry(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "task-worktrees" / "candidate"
        self.source.mkdir(parents=True)
        (self.source / ".github").mkdir()
        (self.source / "scripts").mkdir()
        self.output = self.source / "target/local/report.json"
        self.output.parent.mkdir(parents=True)
        self.repository = "compose-lens"
        self.primary = self.root / "real-workspace" / "boxferry"
        self.primary.mkdir(parents=True)
        self.heads = {self.source: CANDIDATE, self.primary: CANDIDATE}
        self.origins: dict[Path, str] = {}
        self.dirty: set[Path] = set()
        self.wrong_top: set[Path] = set()
        self.events: list[tuple[str, Any]] = []
        self.cache_deadline_receipts: list[dict[str, object]] = []
        self.commands: list[subject.validation.Commands] = []
        self.node_version = "v" + NODE
        self.install_error = False
        self.install_cancel = False
        self.mutate_install = False
        self.npm_config_paths: list[Path] = []
        self.snapshot_policy = policy(self.repository)
        self.configure(self.repository)
        test = self

        class FakeCommands(subject.validation.Commands):
            """Models bounded Git argv, not fabricated .git directories or real Git proof."""

            def __init__(self, private: Path) -> None:
                self.private = private
                test.commands.append(self)
                self.env = subject.validation.environment(private)
                self.outcomes: list[dict[str, object]] = []
                self.budget: Any = None

            def git(self, root: Path, *argv: str) -> str:
                test.events.append(("git", (root, argv)))
                if argv == ("rev-parse", "--show-toplevel"):
                    return str(root / "nested" if root in test.wrong_top else root)
                if argv == ("rev-parse", "HEAD"):
                    if root not in test.heads:
                        raise subject.validation.ValidationError("missing independent Git checkout")
                    return test.heads[root]
                if argv == ("remote", "get-url", "origin"):
                    return test.origins[root]
                if argv[:3] == ("remote", "add", "origin"):
                    test.origins[root] = argv[3]
                    return ""
                if argv == ("status", "--porcelain", "--untracked-files=all"):
                    return " M changed" if root in test.dirty else ""
                if argv == ("rev-parse", "--path-format=absolute", "--git-common-dir"):
                    return str(test.primary / ".git")
                if argv == ("rev-parse", "--path-format=absolute", "--git-dir"):
                    return str(test.primary / ".git")
                if argv == ("ls-files", "--cached", "--others", "--exclude-standard", "-z"):
                    return ".github/automation.json\0scripts/validation-policy.json\0README.md\0"
                if argv[:1] == ("check-ignore",):
                    return ""
                raise AssertionError((root, argv))

            def call(
                self,
                argv: tuple[str, ...],
                stage: str,
                timeout: int = 30,
                *,
                cwd: Path | None = None,
            ) -> str:
                test.events.append((stage, {"argv": argv, "cwd": cwd, "env": dict(self.env)}))
                self.outcomes.append({"stage": stage, "status": "success"})
                if stage == "producer-cache-fsck":
                    return ""
                if stage == "producer-cache-revision":
                    return AUTOMATION
                if stage == "node-version":
                    return test.node_version
                if stage == "install-official-cli":
                    user_config = Path(self.env["NPM_CONFIG_USERCONFIG"])
                    global_config = Path(self.env["NPM_CONFIG_GLOBALCONFIG"])
                    test.npm_config_paths.extend((user_config, global_config))
                    test.assertNotEqual(user_config, global_config)
                    for config in (user_config, global_config):
                        test.assertTrue(config.is_relative_to(self.private))
                        test.assertTrue(config.is_file())
                        test.assertEqual(config.read_bytes(), b"")
                        test.assertEqual(stat.S_IMODE(config.stat().st_mode), 0o600)
                    test.assertEqual(self.env["NPM_CONFIG_CACHE"], str(self.private / "npm-cache"))
                    test.assertEqual(self.env["HOME"], str(self.private / "home"))
                    test.assertNotIn("NPM_CONFIG_TOKEN", self.env)
                    test.assertNotIn("NODE_AUTH_TOKEN", self.env)
                    test.assertNotEqual(self.env["NPM_CONFIG_USERCONFIG"], os.devnull)
                    test.assertNotEqual(self.env["NPM_CONFIG_GLOBALCONFIG"], os.devnull)
                    if test.install_cancel:
                        raise subject.RunCancelled("official install cancelled")
                    if test.install_error:
                        raise subject.validation.ValidationError("official install failed")
                    if test.mutate_install:
                        (test.source / "README.md").write_text("changed during preparation")
                    return ""
                raise AssertionError((argv, stage, timeout))

        def checkout(
            target: Path, revision: str, commands: subject.validation.Commands, remote: str
        ) -> None:
            self.events.append(("checkout", (target, revision, remote)))
            target.mkdir()
            self.heads[target] = revision
            if revision == CANDIDATE:
                (target / "scripts").mkdir()
                (target / "scripts/validation-policy.json").write_text(
                    json.dumps(self.snapshot_policy)
                )
            else:
                (target / "automation/devcontainer").mkdir(parents=True)
                (target / "automation/pyproject.toml").write_text(
                    '[dependency-groups]\ndev=["ruff==1.2.3","ty==2.3.4"]\n'
                    '[tool.uv]\nrequired-version="==0.1.2"\n'
                )
                (target / ".github/workflows").mkdir(parents=True)
                (target / ".github/workflows/ci.yml").write_text(f"    node-version: {NODE}\n")

        def qualify(**kwargs: object) -> dict[str, object]:
            self.events.append(("qualify", kwargs))
            source, trusted, manifest = kwargs["source"], kwargs["policy"], kwargs["manifest"]
            assert isinstance(source, Path) and isinstance(trusted, Path)
            assert isinstance(manifest, dict)
            self.assertNotEqual(kwargs["source"], kwargs["policy"])
            self.assertNotEqual(kwargs["source"], kwargs["automation"])
            self.assertEqual(
                json.loads((trusted / "scripts/validation-policy.json").read_text()),
                self.snapshot_policy,
            )
            return {
                "schema": 1,
                "status": "success",
                "cleanup": "success",
                "candidate_sha": CANDIDATE,
                "policy_sha": CANDIDATE,
                "automation_sha": AUTOMATION,
                "engine": "podman",
                "architecture": kwargs["expected_architecture"],
                "input_sha256": subject.validation.fingerprint(source, FakeCommands(self.root)),
                "source_manifest_sha256": subject.validation.manifest_digest(manifest),
            }

        self.qualify = qualify
        patches = (
            patch.object(subject.validation, "Commands", FakeCommands),
            patch.object(subject.validation.Budget, "start"),
            patch.object(subject.validation, "_checkout", side_effect=checkout),
            patch.object(subject.validation, "qualify", side_effect=qualify),
            patch.object(subject.shutil, "which", return_value=sys.executable),
            patch.object(subject.platform, "system", return_value="Linux"),
            patch.object(subject.platform, "machine", return_value="x86_64"),
        )
        self.mocks: list[Any] = [self.enterContext(item) for item in patches]

    def configure(self, repository: str) -> None:
        self.repository = repository
        self.snapshot_policy = policy(repository)
        self.origins[self.source] = f"https://github.com/Strukturpiloten/{repository}.git"
        self.origins[self.primary] = "https://github.com/Strukturpiloten/boxferry.git"
        (self.source / ".github/automation.json").write_text(
            json.dumps(
                {
                    "schema": 1,
                    "repository": "Strukturpiloten/boxferry",
                    "revision": AUTOMATION,
                    "policy": "scripts/validation-policy.json",
                    "interface": 1,
                }
            )
        )
        (self.source / "README.md").write_text("dirty local candidate")
        (self.source / "scripts/validation-policy.json").write_text(
            "dirty policy must not be parsed"
        )
        for name in subject.validation.SIBLINGS:
            sibling = self.primary.parent / name
            sibling.mkdir(exist_ok=True)
            self.heads[sibling] = SIBLING
            self.origins[sibling] = f"https://github.com/Strukturpiloten/{name}.git"

    def execute(self) -> dict[str, object]:
        return subject.execute(self.source, Path("scripts/validation-policy.json"), self.output)

    def test_nonbox_binds_dirty_candidate_and_clean_HEAD_policy_and_full_automation(self) -> None:
        result = self.execute()
        self.assertEqual(result["status"], "success", result)
        self.assertEqual(result["preparation_cleanup"], "success")
        self.assertEqual(result["policy_sha"], CANDIDATE)
        self.assertEqual(result["automation_sha"], AUTOMATION)
        self.assertEqual(len([event for event in self.events if event[0] == "qualify"]), 1)
        invocation = self.mocks[3].call_args.kwargs
        self.assertEqual(invocation["mode"], "local")
        self.assertEqual(invocation["engine"], "podman")
        self.assertEqual(invocation["expected_architecture"], "amd64")
        self.assertIsNone(invocation["local_sources"])
        clones = [event[1] for event in self.events if event[0] == "checkout"]
        self.assertEqual(clones[0][1:], (CANDIDATE, str(self.source)))
        self.assertEqual(clones[1][1:], (AUTOMATION, subject.bootstrap.REMOTE))
        install = next(event[1] for event in self.events if event[0] == "install-official-cli")
        self.assertEqual(install["argv"][1:], ("ci", "--ignore-scripts", "--no-audit", "--no-fund"))
        self.assertTrue(str(install["cwd"]).endswith(subject.validation.CLI_PROJECT))
        self.assertEqual(install["env"]["NPM_CONFIG_USERCONFIG"], str(self.npm_config_paths[0]))
        self.assertEqual(install["env"]["NPM_CONFIG_GLOBALCONFIG"], str(self.npm_config_paths[1]))
        self.assertTrue(all(not config.exists() for config in self.npm_config_paths))
        self.assertEqual(
            result["policy_config_sha256"],
            hashlib.sha256(json.dumps(self.snapshot_policy).encode()).hexdigest(),
        )

    def test_box_worktree_uses_verified_primary_mapping_for_six_clean_real_siblings(self) -> None:
        self.configure("boxferry")
        self.dirty.add(self.source)
        result = self.execute()
        self.assertEqual(result["status"], "success", result)
        invocation = self.mocks[3].call_args.kwargs
        self.assertEqual(set(invocation["local_sources"]), set(subject.validation.SIBLINGS))
        self.assertTrue(
            all(path.parent == self.primary.parent for path in invocation["local_sources"].values())
        )
        self.assertNotEqual(self.source.parent, self.primary.parent)
        self.assertEqual(invocation["manifest"]["repositories"]["boxferry"], CANDIDATE)
        self.assertTrue(
            all(
                invocation["manifest"]["repositories"][name] == SIBLING
                for name in subject.validation.SIBLINGS
            )
        )

    def test_sibling_missing_dirty_wrong_origin_and_nested_checkout_fail_closed(self) -> None:
        for failure in ("missing", "dirty", "origin", "nested"):
            with self.subTest(failure=failure):
                self.configure("boxferry")
                sibling = self.primary.parent / subject.validation.SIBLINGS[0]
                if failure == "missing":
                    self.heads.pop(sibling)
                elif failure == "dirty":
                    self.dirty.add(sibling)
                elif failure == "origin":
                    self.origins[sibling] = "https://invalid.example/foreign.git"
                else:
                    self.wrong_top.add(sibling)
                self.assertEqual(self.execute()["status"], "failure")
                self.assertFalse(self.mocks[3].called)
                self.dirty.clear()
                self.wrong_top.clear()

    def test_partial_loader_archive_is_never_passed_as_the_automation_checkout(self) -> None:
        partial = self.source / f"target/boxferry-automation/{AUTOMATION}/automation"
        partial.mkdir(parents=True)
        result = self.execute()
        self.assertEqual(result["status"], "success", result)
        self.assertNotEqual(self.mocks[3].call_args.kwargs["automation"], partial.parent)

    def test_verified_cache_is_reused_but_invalid_cache_cannot_fall_back(self) -> None:
        cache = self.source / f"target/boxferry-automation/{AUTOMATION}.git"
        cache.mkdir(parents=True)
        with (
            patch.object(subject.bootstrap, "validate_cache") as verify,
            patch.object(subject.bootstrap, "run_git") as unbudgeted_git,
        ):
            self.assertEqual(self.execute()["status"], "success")
        verify.assert_called_once_with(cache)
        unbudgeted_git.assert_not_called()
        stages = [event[0] for event in self.events]
        self.assertLess(
            stages.index("producer-cache-fsck"), stages.index("producer-cache-revision")
        )
        clone = [event[1] for event in self.events if event[0] == "checkout"][-1]
        self.assertEqual(clone[2], str(cache))
        self.mocks[3].reset_mock()
        with patch.object(subject.bootstrap, "validate_cache", side_effect=ValueError("bad cache")):
            self.assertEqual(self.execute()["status"], "failure")
        self.assertFalse(self.mocks[3].called)

    def test_cache_verification_obeys_expired_and_near_deadline_with_owned_cleanup(self) -> None:
        cache = self.source / f"target/boxferry-automation/{AUTOMATION}.git"
        cache.mkdir(parents=True)
        original_call = self.mocks[0].call
        for phase in ("expired", "fsck", "revision"):
            with self.subTest(phase=phase):
                binary = self.root / ("deadline-" + phase)
                binary.mkdir()
                marker, trace = binary / "pid", binary / "trace"
                executable = binary / "git"
                executable.write_text(
                    f"#!{sys.executable}\n"
                    "import json, os, sys, time\n"
                    "from pathlib import Path\n"
                    f"with Path({str(trace)!r}).open('a') as output:\n"
                    "    output.write(json.dumps(sys.argv[1:]) + '\\n')\n"
                    f"if {phase!r} == 'revision' and 'fsck' in sys.argv: raise SystemExit(0)\n"
                    f"Path({str(marker)!r}).write_text(str(os.getpid()))\n"
                    "time.sleep(30)\n"
                )
                executable.chmod(0o755)
                budget = subject.validation.Budget(
                    self.output.parent, 0, time.monotonic() + 60, check_disk=False
                )
                self.mocks[1].return_value = budget

                def arm(
                    _cache: Path,
                    _phase: str = phase,
                    _binary: Path = binary,
                    _budget: subject.validation.Budget = budget,
                ) -> None:
                    self.commands[-1].env["PATH"] = str(_binary)
                    _budget.deadline = time.monotonic() + (-1 if _phase == "expired" else 0.4)

                def call(
                    instance: subject.validation.Commands,
                    argv: tuple[str, ...],
                    stage: str,
                    timeout: int = 30,
                    *,
                    cwd: Path | None = None,
                ) -> str:
                    if stage.startswith("producer-cache-"):
                        return REAL_COMMANDS.call(instance, argv, stage, timeout, cwd=cwd)
                    return original_call(instance, argv, stage, timeout, cwd=cwd)

                with (
                    patch.object(subject.bootstrap, "validate_cache", side_effect=arm),
                    patch.object(subject.bootstrap, "run_git") as unbudgeted_git,
                    patch.object(self.mocks[0], "call", new=call),
                ):
                    started = time.monotonic()
                    result = self.execute()
                self.assertLess(time.monotonic() - started, 3)
                self.assertEqual(result["status"], "failure")
                self.assertEqual(result["preparation_cleanup"], "success")
                self.assertFalse(self.mocks[3].called)
                unbudgeted_git.assert_not_called()
                outcome = self.commands[-1].outcomes[-1]
                failed_stage = (
                    "producer-cache-revision" if phase == "revision" else "producer-cache-fsck"
                )
                self.assertEqual(outcome["stage"], failed_stage)
                self.assertIn(
                    "rev-parse" if phase == "revision" else "fsck", str(outcome["command"])
                )
                self.assertIn("deadline", str(outcome["cause"]))
                self.assertTrue(outcome["next_action"])
                if phase != "revision":
                    self.assertFalse(
                        any(
                            item["stage"] == "producer-cache-revision"
                            for item in self.commands[-1].outcomes
                        )
                    )
                if phase == "expired":
                    self.assertFalse(marker.exists())
                    self.assertFalse(trace.exists())
                else:
                    self.assertTrue(
                        marker.exists(), "synthetic fsck must have started before expiry"
                    )
                    with self.assertRaises(ProcessLookupError):
                        os.kill(int(marker.read_text()), 0)
                    self.assertEqual(
                        len(trace.read_text().splitlines()), 2 if phase == "revision" else 1
                    )
                self.cache_deadline_receipts.append(
                    {
                        "phase": phase,
                        "status": result["status"],
                        "preparation_cleanup": result["preparation_cleanup"],
                        "cache_outcomes": [
                            dict(item)
                            for item in self.commands[-1].outcomes
                            if str(item["stage"]).startswith("producer-cache-")
                        ],
                        "child_reaped": phase != "expired",
                        "actual_Git_or_native_execution": False,
                    }
                )

    def test_manifest_requires_fixed_producer_full_SHA_and_interface(self) -> None:
        for key, value in (
            ("revision", "main"),
            ("revision", "b" * 39),
            ("repository", "Strukturpiloten/foreign"),
            ("interface", 2),
        ):
            self.configure("compose-lens")
            path = self.source / ".github/automation.json"
            document = json.loads(path.read_text())
            document[key] = value
            path.write_text(json.dumps(document))
            self.assertEqual(self.execute()["status"], "failure")
            self.assertFalse(self.mocks[3].called)

    def test_primary_requires_absolute_common_directory_and_matching_real_git_directory(
        self,
    ) -> None:
        self.configure("boxferry")
        original = self.mocks[0].git
        for failure in ("relative-common", "wrong-primary"):

            def git(
                instance: subject.validation.Commands,
                root: Path,
                *argv: str,
                _failure: str = failure,
            ) -> str:
                if _failure == "relative-common" and argv[-1:] == ("--git-common-dir",):
                    return ".git"
                if _failure == "wrong-primary" and argv[-1:] == ("--git-dir",):
                    return str(self.primary / ".git/worktrees/another-task")
                return original(instance, root, *argv)

            with patch.object(self.mocks[0], "git", new=git):
                self.assertEqual(self.execute()["status"], "failure")
            self.assertFalse(self.mocks[3].called)

    def test_wrong_node_or_failed_install_stops_before_qualifier(self) -> None:
        for node, install in (("v0.0.1", False), ("v" + NODE, True)):
            self.node_version, self.install_error = node, install
            result = self.execute()
            self.assertEqual(result["status"], "failure", result)
            self.assertEqual(result["preparation_cleanup"], "success")
            self.assertFalse(self.mocks[3].called)
            if install:
                self.assertEqual(len(self.npm_config_paths), 2)
                self.assertTrue(all(not config.exists() for config in self.npm_config_paths))

    def test_cancelled_cli_install_removes_private_config_files(self) -> None:
        self.install_cancel = True
        result = self.execute()
        self.assertEqual(result["status"], "failure")
        self.assertEqual(result["preparation_cleanup"], "success")
        self.assertEqual(len(self.npm_config_paths), 2)
        self.assertTrue(all(not config.exists() for config in self.npm_config_paths))
        self.assertFalse(self.mocks[3].called)

    def test_preparation_input_drift_does_not_rebase_the_held_fingerprint(self) -> None:
        self.mutate_install = True
        result = self.execute()
        self.assertEqual(result["status"], "failure")
        cause = result["cause"]
        assert isinstance(cause, str)
        self.assertIn("changed during local preparation", cause)
        self.assertFalse(self.mocks[3].called)

    def test_qualifier_failure_cleanup_failure_and_identity_mismatch_propagate(self) -> None:
        for change in (
            {"status": "failure", "cleanup": "failure", "cleanup_cause": "held image remains"},
            {"automation_sha": "d" * 40},
            {"cleanup": "failure"},
        ):
            with self.subTest(change=change):

                def qualify(
                    _change: dict[str, str] = change, **kwargs: object
                ) -> dict[str, object]:
                    return {**self.qualify(**kwargs), **_change}

                self.mocks[3].side_effect = qualify
                result = self.execute()
                self.assertEqual(result["status"], "failure")
                if "cleanup_cause" in change:
                    self.assertEqual(result["cleanup_cause"], change["cleanup_cause"])

    def test_candidate_drift_after_qualification_fails(self) -> None:
        def qualify(**kwargs: object) -> dict[str, object]:
            result = self.qualify(**kwargs)
            (self.source / "README.md").write_text("late drift")
            return result

        self.mocks[3].side_effect = qualify
        self.assertEqual(self.execute()["status"], "failure")

    def test_budget_start_failure_and_private_cleanup_failure_remain_failures(self) -> None:
        with patch.object(
            subject.validation.Budget, "start", side_effect=ValueError("disk reserve")
        ):
            result = self.execute()
        self.assertEqual(result["status"], "failure")
        self.assertEqual(result["preparation_cleanup"], "success")
        with patch.object(
            subject.shutil, "rmtree", side_effect=OSError("cannot remove private inputs")
        ):
            result = self.execute()
        self.assertEqual(result["status"], "failure")
        self.assertEqual(result["preparation_cleanup"], "failure")

    def test_policy_owner_and_policy_path_are_bound_to_HEAD_and_manifest(self) -> None:
        self.snapshot_policy = policy("podman-lens")
        self.assertEqual(self.execute()["status"], "failure")
        self.assertFalse(self.mocks[3].called)
        result = subject.execute(
            self.source, Path("../scripts/validation-policy.json"), self.output
        )
        self.assertEqual(result["status"], "failure")
        result = subject.execute(self.source, Path("scripts/other-policy.json"), self.output)
        self.assertEqual(result["status"], "failure")

    def test_native_arm64_identity_is_forwarded_without_emulation(self) -> None:
        with patch.object(subject.platform, "machine", return_value="aarch64"):
            result = self.execute()
        self.assertEqual(result["status"], "success", result)
        self.assertEqual(result["architecture"], "arm64")
        self.assertEqual(self.mocks[3].call_args.kwargs["expected_architecture"], "arm64")

    def test_platform_mismatch_stops_before_any_install(self) -> None:
        with patch.object(subject.platform, "system", return_value="Darwin"):
            self.assertEqual(self.execute()["status"], "failure")
        self.assertFalse(any(event[0] == "install-official-cli" for event in self.events))

    def test_dispatch_forwards_raw_arguments_and_main_writes_receipt(self) -> None:
        arguments = [
            "--repository-root",
            str(self.source),
            "--config",
            "scripts/validation-policy.json",
            "--output",
            str(self.output),
        ]
        with patch.object(subject, "main", return_value=7) as called:
            self.assertEqual(dispatch(["devcontainer-local", *arguments]), 7)
        called.assert_called_once_with(arguments)
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(subject.main(arguments), 0)
        self.assertEqual(json.loads(self.output.read_text())["status"], "success")

    def test_source_wrong_origin_or_nested_root_is_rejected(self) -> None:
        for failure in ("origin", "nested"):
            self.configure("compose-lens")
            if failure == "origin":
                self.origins[self.source] = "https://invalid.example/repository.git"
            else:
                self.wrong_top.add(self.source)
            result = self.execute()
            self.assertEqual(result["status"], "failure")
            self.assertFalse(self.mocks[3].called)

    def test_wrong_primary_mapping_cannot_fall_back_to_task_siblings(self) -> None:
        self.configure("boxferry")
        self.wrong_top.add(self.primary)
        result = self.execute()
        self.assertEqual(result["status"], "failure")
        self.assertFalse(self.mocks[3].called)

    def test_sibling_drift_after_qualification_invalidates_success(self) -> None:
        self.configure("boxferry")

        def qualify(**kwargs: object) -> dict[str, object]:
            result = self.qualify(**kwargs)
            self.heads[self.primary.parent / subject.validation.SIBLINGS[0]] = "d" * 40
            return result

        self.mocks[3].side_effect = qualify
        self.assertEqual(self.execute()["status"], "failure")

    def test_missing_backend_and_cancelled_preparation_produce_bounded_failure(self) -> None:
        self.mocks[3].side_effect = subject.validation.ValidationError(
            "required Podman unavailable"
        )
        result = self.execute()
        self.assertEqual(result["status"], "failure")
        self.assertEqual(result["engine"], "podman")
        self.assertEqual(result["preparation_cleanup"], "success")
        self.install_cancel = True
        result = self.execute()
        self.assertEqual(result["status"], "failure")
        self.assertEqual(result["preparation_cleanup"], "success")
        self.assertTrue(all(not config.exists() for config in self.npm_config_paths))

    def test_main_replaces_stale_receipt_on_failed_preparation(self) -> None:
        self.output.write_text('{"status":"success"}')
        arguments = [
            "--repository-root",
            str(self.source),
            "--config",
            "scripts/validation-policy.json",
            "--output",
            str(self.output),
        ]
        with (
            patch.object(subject, "execute", side_effect=OSError("private directory unavailable")),
            contextlib.redirect_stderr(io.StringIO()),
        ):
            self.assertEqual(subject.main(arguments), 1)
        self.assertEqual(json.loads(self.output.read_text())["status"], "failure")

    def test_failure_report_redacts_protected_values_and_rejects_oversized_receipts(self) -> None:
        with (
            patch.dict(os.environ, {"TEST_PRIVATE_TOKEN": "private-test-value"}),
            patch.object(subject, "_install_cli", side_effect=ValueError("private-test-value")),
        ):
            result = self.execute()
        self.assertNotIn("private-test-value", json.dumps(result))
        arguments = [
            "--repository-root",
            str(self.source),
            "--config",
            "scripts/validation-policy.json",
            "--output",
            str(self.output),
        ]
        with (
            patch.object(
                subject,
                "execute",
                return_value={"status": "success", "large": "x" * subject.validation.MAX_REPORT},
            ),
            contextlib.redirect_stdout(io.StringIO()),
        ):
            self.assertEqual(subject.main(arguments), 1)
        self.assertLess(self.output.stat().st_size, subject.validation.MAX_REPORT)

    def test_output_outside_owned_target_and_symlink_are_rejected(self) -> None:
        for output in (self.source / "README.md", self.source / "target/../outside.json"):
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(
                    subject.main(
                        [
                            "--repository-root",
                            str(self.source),
                            "--config",
                            "scripts/validation-policy.json",
                            "--output",
                            str(output),
                        ]
                    ),
                    1,
                )
        self.output.symlink_to(self.source / "README.md")
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(
                subject.main(
                    [
                        "--repository-root",
                        str(self.source),
                        "--config",
                        "scripts/validation-policy.json",
                        "--output",
                        str(self.output),
                    ]
                ),
                1,
            )


if __name__ == "__main__":
    unittest.main()
