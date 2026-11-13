"""Independent authored Git objects, native mount maps and refusal/receipt adversaries."""

from __future__ import annotations

import importlib.util
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr
from copy import deepcopy
from pathlib import Path
from unittest.mock import patch

from git_worktree_fixture import CASES, HEAD, expected_files, expected_index, fixtures, receipt

from boxferry_ci import git_worktree as inspector
from boxferry_ci import git_worktree_validation as subject
from boxferry_ci.devcontainer_validation import Commands

ROOT = Path(__file__).resolve().parents[2]


class HostFixtureTests(unittest.TestCase):
    def test_actual_host_creates_untouched_layouts_with_independent_git_objects(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            private = Path(temporary)
            fixture = subject.prepare(private, ROOT, ROOT, Commands(private))
            self.assertEqual(subject.baseline_head(), HEAD)
            self.assertEqual(subject.entries(subject.STAGED), expected_index())
            self.assertEqual(subject.working_files(), expected_files())
            self.assertEqual(subject.CASES, CASES)
            self.assertEqual(
                (fixture.root / "relative/.git").read_text(),
                "gitdir: ../primary/.git/worktrees/relative\n",
            )
            self.assertTrue((fixture.root / "legacy/.git").read_text().startswith("gitdir: /"))
            frozen = subject.fingerprint(fixture.root)
            for name in ("primary", "relative", "legacy", "standalone"):
                observation = inspector.inspect(fixture.root / name)
                self.assertEqual(observation["head"], HEAD)
                self.assertEqual(observation["files"], expected_files())
                self.assertEqual(observation["index_sha256"], fixture.expected["index_sha256"])
            self.assertEqual(subject.fingerprint(fixture.root), frozen)
            self.assertEqual(
                fixture.expected["host_repository_formats"],
                {"primary": 1, "relative": 1, "legacy": 0, "standalone": 0},
            )
            self.assertTrue((fixture.root / "standalone/.git").is_dir())
            self.assertFalse((fixture.root / "standalone/.git/objects/info/alternates").exists())
            binds = subject.mounts(fixture, ROOT)
            self.assertEqual(binds.count("--mount"), 9)
            self.assertTrue(
                all(value.endswith(",readonly") for value in binds if value.startswith("type="))
            )
            self.assertIn(
                f"type=bind,source={fixture.root}/primary/.git,target=/workspaces/.boxferry-git-contract/topology/primary/.git,readonly",
                binds,
            )
            self.assertFalse(
                any(
                    "target=/workspaces/.boxferry-git-contract/leaf/primary" in value
                    for value in binds
                )
            )
            self.assertFalse(any(f"source={fixture.root}," in value for value in binds))

    def test_changed_immutable_source_or_unsupported_native_git_cannot_emulate_metadata(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            private = Path(temporary)
            commands = Commands(private)
            with self.assertRaisesRegex(subject.FixtureError, "explicit immutable"):
                subject.prepare(private, private / "wrong", ROOT, commands)
            original = commands.git

            def unsupported(root: Path, *args: str) -> str:
                if "--relative-paths" in args:
                    raise ValueError("unsupported real host option")
                return original(root, *args)

            with (
                patch.object(commands, "git", side_effect=unsupported),
                self.assertRaisesRegex(subject.FixtureError, "never emulate"),
            ):
                subject.prepare(private, ROOT, ROOT, commands)
            self.assertFalse((private / "git-worktree-contract/relative").exists())


class NativeReceiptTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.fixtures = fixtures(self.root / "fixtures", ROOT / "automation/src/boxferry_ci")
        self.valid = receipt(self.fixtures)

    def verify(self, value: object) -> dict[str, object]:
        return subject.verify_receipt(json.dumps(value), self.fixtures, "x86_64", 1000, 1000)

    def test_success_preserves_every_case_and_both_actual_native_machine_identities(self) -> None:
        for machine in ("x86_64", "aarch64"):
            result = subject.verify_receipt(
                json.dumps(receipt(self.fixtures, machine)), self.fixtures, machine, 1000, 1000
            )
            self.assertEqual(result["status"], "success")
            self.assertTrue(result["host_metadata_unchanged"])

    def test_wrong_envelope_schema_source_architecture_and_user_are_refused(self) -> None:
        for key, value in (
            ("schema", True),
            ("spec_sha256", "0" * 64),
            ("architecture", "aarch64"),
            ("uid", 0),
            ("uid", True),
            ("gid", 1001),
            ("helper_sha256", {}),
            ("candidate_helper_sha256", {}),
            ("extra", "unknown"),
        ):
            candidate = deepcopy(self.valid)
            candidate[key] = value
            with self.subTest(key=key, value=value), self.assertRaises(subject.FixtureError):
                self.verify(candidate)
        with self.assertRaisesRegex(subject.FixtureError, "duplicate"):
            subject.verify_receipt(
                json.dumps(self.valid).replace('"schema": 1', '"schema": 2, "schema": 1', 1),
                self.fixtures,
                "x86_64",
                1000,
                1000,
            )

    def test_missing_extra_duplicate_or_generic_refusal_never_substitutes_for_cases(self) -> None:
        for name in CASES:
            candidate = deepcopy(self.valid)
            del candidate["cases"][name]
            with self.assertRaisesRegex(subject.FixtureError, "missing or extra"):
                self.verify(candidate)
        for field, value in (
            ("category", "source-refused"),
            ("exit", 0),
            ("exit", True),
            ("git_exit", 0),
            ("candidate_cli_exit", 128),
            ("inspection", {}),
        ):
            candidate = deepcopy(self.valid)
            candidate["cases"]["relative-leaf"][field] = value
            with self.subTest(field=field), self.assertRaises(subject.FixtureError):
                self.verify(candidate)
        candidate = deepcopy(self.valid)
        candidate["cases"]["unreviewed"] = candidate["cases"]["primary"]
        with self.assertRaises(subject.FixtureError):
            self.verify(candidate)

    def test_positive_cases_bind_source_index_format_known_bytes_and_modes(self) -> None:
        for name in ("primary", "relative-topology", "standalone"):
            for key, value in (
                ("source", "/wrong"),
                ("head", "0" * 40),
                ("index_sha256", "0" * 64),
                ("raw_index_sha256", "0" * 64),
                ("repository_format", 9),
                ("extensions", {} if name != "standalone" else {"relativeworktrees": "true"}),
                ("files", {}),
                ("git_version", 42),
                ("git_version", "git version 0.0.0"),
            ):
                candidate = deepcopy(self.valid)
                candidate["cases"][name]["inspection"][key] = value
                with self.subTest(case=name, key=key), self.assertRaises(subject.FixtureError):
                    self.verify(candidate)
        candidate = deepcopy(self.valid)
        candidate["cases"]["primary"]["inspection"]["files"]["executable.sh"]["mode"] = 0o644
        with self.assertRaises(subject.FixtureError):
            self.verify(candidate)
        candidate = deepcopy(self.valid)
        candidate["system_git"] = "git version 0.0.0"
        with self.assertRaises(subject.FixtureError):
            self.verify(candidate)

    def test_mutated_host_or_oversized_malformed_receipt_is_never_success(self) -> None:
        (self.fixtures.root / "changed-index").write_bytes(b"changed")
        with self.assertRaisesRegex(subject.FixtureError, "changed host"):
            self.verify(self.valid)
        with self.assertRaisesRegex(subject.FixtureError, "byte bound"):
            subject.verify_receipt(
                "x" * (subject.MAX_RECEIPT + 1), self.fixtures, "x86_64", 1000, 1000
            )
        with self.assertRaises(ValueError):
            subject.verify_receipt("{", self.fixtures, "x86_64", 1000, 1000)


class CandidateCallerTests(unittest.TestCase):
    def test_equal_semantics_from_another_source_index_or_format_are_refused(self) -> None:
        root = subject.TARGET / subject.CASES["primary"]
        expected = {"head": "1" * 40, "index_sha256": "2" * 64, "files": {}}
        observation = {
            **expected,
            "source": str(root),
            "git_dir": str(root / ".git"),
            "index": str(root / ".git/index"),
            "raw_index_sha256": "3" * 64,
            "git_version": "git version 99.0.1",
            "repository_format": 1,
            "files_sha256": "4" * 64,
        }
        for key, value in (
            ("source", "/another/checkout"),
            ("git_dir", "/another/.git"),
            ("index", "/another/index"),
            ("raw_index_sha256", "0" * 64),
            ("repository_format", 0),
            ("git_version", "git version 99.0.2"),
        ):
            candidate = {**observation, key: value}
            # Old semantic identity alone was equal despite wrong source/index.
            self.assertEqual(
                subject.source.identity(candidate), subject.source.identity(observation)
            )
            with (
                patch.object(subject.source, "inspect", return_value=observation),
                patch.object(
                    subject.subprocess,
                    "run",
                    side_effect=[
                        subprocess.CompletedProcess([], 0, expected["head"] + "\n", ""),
                        subprocess.CompletedProcess([], 0, json.dumps(candidate), ""),
                    ],
                ),
                self.subTest(key=key),
                self.assertRaisesRegex(subject.FixtureError, "exact source/index"),
            ):
                subject._case(root, expected)

    def test_category_without_actionable_recovery_is_refused(self) -> None:
        for name, category in subject.REFUSALS.items():
            root = subject.TARGET / subject.CASES[name]
            with (
                patch.object(
                    subject.source,
                    "inspect",
                    side_effect=subject.source.SourceError("controlled refusal", category=category),
                ),
                patch.object(
                    subject.subprocess,
                    "run",
                    side_effect=[
                        subprocess.CompletedProcess([], 128, "", "fatal"),
                        subprocess.CompletedProcess([], 1, "", f"[{category}]"),
                    ],
                ),
                self.subTest(name=name),
                self.assertRaisesRegex(subject.FixtureError, "actionable"),
            ):
                subject._case(root, {})


class LockedImagePythonTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.revision = "a" * 40
        self.manifest = self.root / ".github/automation.json"
        self.manifest.parent.mkdir()
        self.manifest.write_text(json.dumps({"revision": self.revision}))
        self.python = (
            self.root / "target/boxferry-automation" / self.revision / "automation/.venv/bin/python"
        )
        self.python.parent.mkdir(parents=True)
        self.python.write_text("# controlled retained interpreter")
        self.python.chmod(0o755)
        specification = importlib.util.spec_from_file_location(
            "image_git_caller", ROOT / "scripts/git-worktree.py"
        )
        assert specification is not None and specification.loader is not None
        self.caller = importlib.util.module_from_spec(specification)
        specification.loader.exec_module(self.caller)

    def mapped(self, value: str) -> Path:
        return self.root if value == "/opt/boxferry-tooling" else Path(value)

    def invoke(self, native: bool) -> tuple[object, list[object], str]:
        observed: list[object] = []
        errors = io.StringIO()

        class ExecObserved(BaseException):
            pass

        def execute(*args: object) -> None:
            observed.extend(args)
            raise ExecObserved

        with patch.object(os, "execv", side_effect=execute), redirect_stderr(errors):
            try:
                if native:
                    with (
                        patch("pathlib.Path", side_effect=self.mapped),
                        patch.object(sys, "argv", ["-c", self.revision, "print('probe')"]),
                    ):
                        exec(subject.IMAGE_PYTHON, {})
                else:
                    with (
                        patch.object(self.caller, "Path", side_effect=self.mapped),
                        patch.object(sys, "version_info", (3, 11)),
                        patch.object(
                            sys,
                            "argv",
                            [
                                str(ROOT / "scripts/git-worktree.py"),
                                "inspect",
                                "--source",
                                "/exact/source",
                            ],
                        ),
                    ):
                        result = self.caller.main()
                        return result, observed, errors.getvalue()
            except ExecObserved:
                return "exec", observed, errors.getvalue()
        return None, observed, errors.getvalue()

    def test_both_system311_launchers_select_only_retained_python_and_isolation(self) -> None:
        for native in (False, True):
            result, observed, _errors = self.invoke(native)
            self.assertEqual(result, "exec")
            self.assertEqual(observed[0], self.python)
            argv = observed[1]
            assert isinstance(argv, list)
            self.assertEqual(argv[:3], [str(self.python), "-I", "-c"])
            program = argv[3]
            assert isinstance(program, str)
            with (
                patch.object(sys, "version_info", (3, 11)),
                self.assertRaisesRegex(SystemExit, "3.13 or newer"),
            ):
                exec(program, {})
            if not native:
                self.assertEqual(
                    argv[4:],
                    [str(ROOT / "scripts/git-worktree.py"), "inspect", "--source", "/exact/source"],
                )
            else:
                self.assertTrue(program.endswith("print('probe')"))

    def test_missing_malformed_nonobject_and_invalid_revision_refuse_before_exec(self) -> None:
        for value in (None, "{", "[]", "{}", '{"revision":"../other"}', "x" * 4097):
            for native in (False, True):
                if value is None:
                    self.manifest.unlink(missing_ok=True)
                else:
                    self.manifest.write_text(value)
                with self.subTest(value=value, native=native):
                    if native:
                        with self.assertRaises((OSError, ValueError, KeyError)):
                            self.invoke(native)
                    else:
                        result, observed, errors = self.invoke(native)
                        self.assertEqual(result, 1)
                        self.assertEqual(observed, [])
                        self.assertIn("locked Python 3.13", errors)

    def test_missing_interpreter_and_native_source_mismatch_refuse(self) -> None:
        self.python.unlink()
        for native in (False, True):
            if native:
                with self.assertRaisesRegex(ValueError, "unavailable"):
                    self.invoke(native)
            else:
                result, observed, errors = self.invoke(native)
                self.assertEqual((result, observed), (1, []))
                self.assertIn("unavailable", errors)
        self.manifest.write_text(json.dumps({"revision": "b" * 40}))
        with self.assertRaisesRegex(ValueError, "differs from admitted"):
            self.invoke(True)


if __name__ == "__main__":
    unittest.main()
