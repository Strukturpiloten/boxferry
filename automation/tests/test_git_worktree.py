#!/usr/bin/env python3
"""Real-repository source/index, mounted-metadata and explicit fallback contracts."""

from __future__ import annotations

import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from typing import Any
from unittest.mock import patch

from boxferry_ci import git_worktree as subject


class GitWorktreeContracts(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="boxferry-git-contract-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "primary"
        self.source.mkdir()
        self.environment = {
            key: value for key, value in os.environ.items() if not key.startswith("GIT_")
        }
        self.environment.update(
            GIT_CONFIG_NOSYSTEM="1",
            GIT_CONFIG_GLOBAL="/dev/null",
            GIT_CONFIG_SYSTEM="/dev/null",
            GIT_CONFIG_COUNT="0",
        )
        self.command(self.source, "init", "--initial-branch=main")
        self.command(self.source, "config", "user.name", "Git fixture")
        self.command(self.source, "config", "user.email", "fixture@example.invalid")
        (self.source / "file").write_bytes(b"base\n")
        (self.source / "executable").write_bytes(b"#!/bin/sh\nexit 0\n")
        (self.source / "executable").chmod(0o755)
        (self.source / "removed").write_text("remove me\n")
        (self.source / "removed-staged").write_text("remove from index\n")
        self.command(self.source, "add", ".")
        self.command(self.source, "commit", "-m", "baseline")

    def command(self, root: Path, *args: str) -> bytes:
        return subprocess.run(
            ["git", "-C", str(root), *args],
            env=self.environment,
            check=True,
            capture_output=True,
            timeout=30,
        ).stdout

    def dirty(self, root: Path) -> None:
        (root / "file").write_bytes(b"staged\0bytes\n")
        (root / "new").write_bytes(b"new staged blob\n")
        self.command(root, "add", "file", "new")
        (root / "file").write_bytes(b"working\0bytes\n")
        self.command(root, "rm", "removed-staged")
        (root / "removed").unlink()
        (root / "untracked").write_bytes(b"untracked\n")
        (root / "link").symlink_to("file")
        (root / "executable").chmod(0o644)

    def test_primary_relative_legacy_and_standalone_preserve_exact_identity(self) -> None:
        for relative in (True, False):
            source = self.root / ("relative" if relative else "legacy")
            self.command(
                self.source,
                "worktree",
                "add",
                "--relative-paths" if relative else "--no-relative-paths",
                "-b",
                source.name,
                str(source),
            )
            self.dirty(source)
            metadata_before = (source / ".git").read_bytes()
            config_before = (self.source / ".git/config").read_bytes()
            expected_head = self.command(source, "rev-parse", "HEAD").strip()
            before = subject.inspect(source)
            destination = self.root / (source.name + "-standalone")
            receipt = subject.standalone(source, destination)
            self.assertEqual(subject.identity(before), subject.identity(receipt["standalone"]))
            self.assertEqual(subject.identity(before), subject.identity(subject.inspect(source)))
            self.assertEqual(self.command(destination, "rev-parse", "HEAD").strip(), expected_head)
            self.assertEqual(self.command(destination, "show", ":file"), b"staged\0bytes\n")
            self.assertEqual(self.command(destination, "show", ":new"), b"new staged blob\n")
            self.assertNotIn(b"removed-staged", self.command(destination, "ls-files"))
            self.assertTrue(
                self.command(destination, "ls-files", "--stage", "executable").startswith(
                    b"100755 "
                )
            )
            self.assertEqual((source / ".git").read_bytes(), metadata_before)
            self.assertEqual((self.source / ".git/config").read_bytes(), config_before)
            self.assertTrue((destination / ".git").is_dir())
            self.assertNotIn(b"relativeWorktrees", (destination / ".git/config").read_bytes())
            self.assertFalse((destination / ".git/objects/info/alternates").exists())
            self.assertEqual((destination / "file").read_bytes(), b"working\0bytes\n")
            self.assertEqual((destination / "executable").stat().st_mode & 0o777, 0o644)
            self.assertTrue((destination / "link").is_symlink())
        self.assertEqual(subject.inspect(self.source)["head"], before["head"])

    def test_missing_relocated_metadata_has_actionable_distinct_diagnostics(self) -> None:
        for relative in (True, False):
            leaf = self.root / ("relative" if relative else "legacy")
            self.command(
                self.source,
                "worktree",
                "add",
                "--relative-paths" if relative else "--no-relative-paths",
                "-b",
                leaf.name,
                str(leaf),
            )
            # A leaf-only relocated mount cannot see its original metadata.
            relocated = self.root / (leaf.name + "-mounted")
            shutil.copytree(leaf, relocated)
            if not relative:
                (relocated / ".git").write_text(
                    "gitdir: /unmounted-host/primary/.git/worktrees/legacy\n"
                )
            else:
                (relocated / ".git").write_text(
                    "gitdir: ../unmounted-primary/.git/worktrees/relative\n"
                )
            with self.assertRaisesRegex(
                subject.SourceError,
                "experimentalMountGitWorktreeCommonDir"
                if relative
                else "standalone --source SOURCE",
            ):
                subject.inspect(relocated)

    def test_existing_destination_and_routing_overrides_refuse_without_mutation(self) -> None:
        destination = self.root / "existing"
        destination.mkdir()
        marker = destination / "keep"
        marker.write_text("unrelated")
        with self.assertRaisesRegex(subject.SourceError, "already exists"):
            subject.standalone(self.source, destination)
        self.assertEqual(marker.read_text(), "unrelated")
        for key in subject.ROUTING:
            with (
                patch.dict(os.environ, {key: "unrelated"}),
                self.assertRaisesRegex(subject.SourceError, "unset"),
            ):
                subject.inspect(self.source)

    def test_qualifier_exact_path_trust_is_accepted_but_overrides_are_refused(self) -> None:
        entries = [str(self.source)] + [
            str(self.source / ".boxferry-workspace" / name)
            for name in (
                "compose-lens",
                "podman-lens",
                "quadlet-lens",
                "docker-lens",
                "kubernetes-lens",
                "boxferry-website",
            )
        ]
        environment = {"GIT_CONFIG_COUNT": "7"}
        for index, value in enumerate(entries):
            environment[f"GIT_CONFIG_KEY_{index}"] = "safe.directory"
            environment[f"GIT_CONFIG_VALUE_{index}"] = value
        with patch.dict(os.environ, environment):
            subject.inspect(self.source)
            for key, value in (
                ("GIT_CONFIG_KEY_0", "core.worktree"),
                ("GIT_CONFIG_VALUE_0", "*"),
                ("GIT_CONFIG_VALUE_0", "/unrelated"),
                ("GIT_CONFIG_COUNT", "8"),
            ):
                with patch.dict(os.environ, {key: value}), self.assertRaises(subject.SourceError):
                    subject.inspect(self.source)

    def test_regular_to_symlink_race_cannot_change_an_outside_file(self) -> None:
        outside = self.root / "outside"
        outside.write_bytes(b"unrelated")
        outside.chmod(0o600)
        destination = self.root / "race"
        original = subject.member
        changed = False

        def swap(root: Path, name: str) -> tuple[bytes, int, str]:
            nonlocal changed
            if (
                destination.exists()
                and root == self.source
                and name == "executable"
                and not changed
            ):
                changed = True
                (root / name).unlink()
                (root / name).symlink_to(outside)
            return original(root, name)

        with (
            patch.object(subject, "member", side_effect=swap),
            self.assertRaisesRegex(subject.SourceError, "source changed"),
        ):
            subject.standalone(self.source, destination)
        self.assertTrue(changed)
        self.assertEqual(outside.read_bytes(), b"unrelated")
        self.assertEqual(outside.stat().st_mode & 0o777, 0o600)
        self.assertFalse(destination.exists())

    def test_failed_clone_and_changed_source_remove_only_owned_destination(self) -> None:
        destination = self.root / "failed"
        original = subject.git

        def fail(root: Path, *args: str) -> bytes:
            if "clone" in args:
                raise subject.SourceError("controlled clone failure")
            return original(root, *args)

        with (
            patch.object(subject, "git", side_effect=fail),
            self.assertRaisesRegex(subject.SourceError, "controlled"),
        ):
            subject.standalone(self.source, destination)
        self.assertFalse(destination.exists())
        real = subject.inspect
        count = 0

        def changed(root: Path) -> dict[str, Any]:
            nonlocal count
            count += 1
            if count == 3:
                (self.source / "file").write_text("changed during snapshot")
            return real(root)

        with (
            patch.object(subject, "inspect", side_effect=changed),
            self.assertRaisesRegex(subject.SourceError, "source changed"),
        ):
            subject.standalone(self.source, destination)
        self.assertFalse(destination.exists())
        self.assertTrue(self.source.exists())

    def test_primary_clone_preserves_remote_only_baseline_without_network(self) -> None:
        self.command(
            self.source,
            "remote",
            "add",
            "origin",
            "https://github.com/Strukturpiloten/boxferry.git",
        )
        # A freshly fetched main can be newer than every remaining local branch.
        self.command(self.source, "checkout", "-b", "upstream-main")
        (self.source / "upstream-only").write_bytes(b"newer upstream baseline\n")
        self.command(self.source, "add", "upstream-only")
        self.command(self.source, "commit", "-m", "upstream only")
        head = self.command(self.source, "rev-parse", "HEAD").strip().decode()
        self.command(self.source, "checkout", "main")
        self.command(self.source, "branch", "-D", "upstream-main")
        self.command(self.source, "update-ref", "refs/remotes/origin/main", head)
        self.dirty(self.source)
        destination = self.root / "validation-clone"
        result = subject.standalone(self.source, destination)
        self.assertEqual(
            result["baseline"],
            {"origin": "https://github.com/Strukturpiloten/boxferry.git", "origin_main": head},
        )
        self.assertEqual(
            self.command(destination, "remote", "get-url", "origin"),
            b"https://github.com/Strukturpiloten/boxferry.git\n",
        )
        self.assertEqual(
            self.command(destination, "rev-parse", "refs/remotes/origin/main").strip().decode(),
            head,
        )
        self.assertEqual(
            self.command(destination, "show", head + ":upstream-only"), b"newer upstream baseline\n"
        )
        self.assertNotEqual(self.command(destination, "rev-parse", "HEAD").strip().decode(), head)

    def fake_git(self, mode: str) -> tuple[Path, Path]:
        binary = self.root / "shim"
        binary.mkdir(exist_ok=True)
        pids = self.root / "owned-processes.json"
        real = shutil.which("git")
        assert real is not None
        script = binary / "git"
        script.write_text(
            f"#!{sys.executable}\n"
            "import json, os, subprocess, sys, time\n"
            f"mode = {mode!r}\n"
            "if mode == 'clone' and 'clone' not in sys.argv[1:]:\n"
            f" os.execv({real!r}, [{real!r}, *sys.argv[1:]])\n"
            "child = subprocess.Popen([sys.executable, '-c', 'import time;time.sleep(120)'])\n"
            f"with open({str(pids)!r}, 'w') as stream:\n"
            " json.dump([os.getpid(), child.pid], stream)\n"
            "if mode == 'output':\n"
            " sys.stdout.buffer.write(b'x' * 3000000);sys.stdout.buffer.flush()\n"
            "time.sleep(120)\n"
        )
        script.chmod(0o755)
        return binary, pids

    def assert_processes_stopped(self, pids: Path) -> None:
        self.assertTrue(pids.is_file(), "controlled child must really execute")
        identities = json.loads(pids.read_text())

        def running(pid: int) -> bool:
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                return False
            state = Path(f"/proc/{pid}/stat")
            return not (state.exists() and state.read_text().split(") ", 1)[1].startswith("Z"))

        try:
            deadline = time.monotonic() + 3
            while any(running(pid) for pid in identities) and time.monotonic() < deadline:
                time.sleep(0.01)
            self.assertFalse(
                any(running(pid) for pid in identities), "owned Git descendants survived"
            )
        finally:
            for pid in identities:
                if running(pid):
                    os.kill(pid, signal.SIGKILL)

    def test_canonical_runner_bounds_output_timeout_and_owned_descendants(self) -> None:
        for mode, seconds in (("timeout", 0.2), ("output", 5)):
            binary, pids = self.fake_git(mode)
            with (
                patch.dict(os.environ, {"PATH": str(binary) + ":" + os.environ["PATH"]}),
                patch.object(subject, "COMMAND_SECONDS", seconds),
                self.subTest(mode=mode),
                self.assertRaises(subject.SourceError),
            ):
                subject.git(self.source, "--version")
            self.assert_processes_stopped(pids)
        self.assertTrue(self.source.is_dir())

    def test_sigterm_cleans_nested_git_group_and_only_owned_fallback_destination(self) -> None:
        binary, pids = self.fake_git("clone")
        destination = self.root / "cancelled-clone"
        outside = self.root / "protected"
        outside.write_bytes(b"unrelated")
        outside.chmod(0o600)
        environment = {
            **self.environment,
            "PATH": str(binary) + ":" + os.environ["PATH"],
            "PYTHONPATH": str(Path(subject.__file__).parents[1]),
        }
        program = (
            "from pathlib import Path;from boxferry_ci.git_worktree import standalone;"
            f"standalone(Path({str(self.source)!r}),Path({str(destination)!r}))"
        )
        process = subprocess.Popen(
            [sys.executable, "-c", program],
            env=environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )
        try:
            deadline = time.monotonic() + 10
            while not pids.exists() and process.poll() is None and time.monotonic() < deadline:
                time.sleep(0.01)
            self.assertTrue(pids.exists(), "clone must reach the controlled running Git child")
            self.assertTrue(destination.is_dir())
            process.send_signal(signal.SIGTERM)
            process.communicate(timeout=10)
            self.assertNotEqual(process.returncode, 0)
            self.assert_processes_stopped(pids)
            self.assertFalse(destination.exists())
            self.assertEqual(outside.read_bytes(), b"unrelated")
            self.assertEqual(outside.stat().st_mode & 0o777, 0o600)
            self.assertEqual(self.command(self.source, "show", "HEAD:file"), b"base\n")
        finally:
            if process.poll() is None:
                process.kill()
                process.communicate(timeout=10)

    def test_sparse_index_and_escaping_parent_refuse(self) -> None:
        self.command(self.source, "update-index", "--assume-unchanged", "file")
        with self.assertRaisesRegex(subject.SourceError, "assume-unchanged"):
            subject.inspect(self.source)
        self.command(self.source, "update-index", "--no-assume-unchanged", "file")
        with (
            patch.object(subject, "MAX_BYTES", 1),
            self.assertRaisesRegex(subject.SourceError, "byte bound"),
        ):
            subject.inspect(self.source)


if __name__ == "__main__":
    unittest.main()
