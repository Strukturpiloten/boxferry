from __future__ import annotations

import argparse
import contextlib
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from boxferry_ci.__main__ import _snapshot, _task, main
from boxferry_ci.contracts import ContractError
from boxferry_ci.policy import load_policy


class Cli(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.config = self.root / "scripts/validation-policy.json"
        self.config.parent.mkdir()
        self.config.write_text(
            json.dumps(
                {
                    "schema": 2,
                    "repository": "Strukturpiloten/example",
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
            )
        )

    def command(self, *args: str) -> str:
        return subprocess.run(
            ("git", *args),
            cwd=self.root,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()

    def arguments(self, command: str, *extra: str) -> list[str]:
        return [command, "--repository-root", str(self.root), "--config", str(self.config), *extra]

    def test_task_writes_actionable_json_and_summary(self) -> None:
        report, summary = self.root / "report.json", self.root / "summary.md"
        self.assertEqual(
            main(
                self.arguments(
                    "run",
                    "--task",
                    "quality",
                    "--report-json",
                    str(report),
                    "--summary",
                    str(summary),
                )
            ),
            0,
        )
        evidence = json.loads(report.read_text())
        self.assertEqual(evidence["status"], "success")
        diagnostic = evidence["results"][0]["diagnostic"]
        self.assertEqual(
            set(diagnostic),
            {
                "stage",
                "cause",
                "affected_input",
                "evidence_link",
                "next_action",
            },
        )
        self.assertEqual(diagnostic["evidence_link"], str(report))
        self.assertIn("stage:", summary.read_text())

    def test_unknown_task_fails_with_report_and_never_runs_shell(self) -> None:
        report = self.root / "report.json"
        with contextlib.redirect_stderr(io.StringIO()):
            result = main(self.arguments("run", "--task", "unknown", "--report-json", str(report)))
        self.assertEqual(result, 1)
        evidence = json.loads(report.read_text())
        self.assertEqual(evidence["status"], "infrastructure")
        self.assertIn("not configured", evidence["cause"])
        self.assertIn("next_action", evidence["results"][0]["diagnostic"])

    def test_silent_failure_identifies_redacted_command_in_json_and_summary(self) -> None:
        report, summary = self.root / "report.json", self.root / "summary.md"
        raw = json.loads(self.config.read_text())
        raw["tasks"]["quality"]["commands"] = [
            [
                sys.executable,
                "-c",
                "import sys; sys.exit(9)",
                "--token",
                "private-task-token",
            ]
        ]
        self.config.write_text(json.dumps(raw))
        self.assertEqual(
            main(
                self.arguments(
                    "run",
                    "--task",
                    "quality",
                    "--report-json",
                    str(report),
                    "--summary",
                    str(summary),
                )
            ),
            1,
        )
        evidence = json.loads(report.read_text())
        result = evidence["results"][0]
        self.assertEqual(result["stdout"], "")
        self.assertEqual(result["stderr"], "")
        self.assertEqual(result["returncode"], 9)
        self.assertEqual(
            result["argv"],
            [
                Path(sys.executable).name,
                "-c",
                "import sys; sys.exit(9)",
                "--token",
                "[REDACTED]",
            ],
        )
        self.assertIn("argv:", summary.read_text())
        self.assertIn("import sys; sys.exit(9)", summary.read_text())
        self.assertNotIn("private-task-token", report.read_text() + summary.read_text())

    def test_gate_check_failure_is_distinct_from_infrastructure(self) -> None:
        revision = "a" * 40
        plan = {
            "schema": 3,
            "repository": "Strukturpiloten/example",
            "event": "push",
            "base_sha": revision,
            "head_sha": revision,
            "tested_sha": revision,
            "profile": "full",
            "profiles": ["full"],
            "jobs": {"quality": True},
            "reasons": ["full push validation"],
            "local_fingerprint": None,
        }
        needs = {"validation-plan": {"result": "success"}, "quality": {"result": "failure"}}
        report = self.root / "report.json"
        with contextlib.redirect_stderr(io.StringIO()):
            result = main(
                self.arguments(
                    "gate",
                    "--event",
                    "push",
                    "--tested-sha",
                    revision,
                    "--plan-json",
                    json.dumps(plan),
                    "--needs-json",
                    json.dumps(needs),
                    "--report-json",
                    str(report),
                )
            )
        self.assertEqual(result, 1)
        self.assertEqual(json.loads(report.read_text())["status"], "failed")

    def test_policy_rejects_shell_expressions_and_boolean_timeout(self) -> None:
        original = json.loads(self.config.read_text())
        for task in (
            {"commands": [["bash", "-c", "touch unauthorized"]]},
            {"commands": [["true"]], "timeout_seconds": True},
        ):
            with self.subTest(task=task), self.assertRaises(ContractError):
                self.config.write_text(json.dumps(dict(original, tasks={"quality": task})))
                load_policy(self.config)
        self.assertFalse((self.root / "unauthorized").exists())

    def test_job_names_cannot_inject_github_output(self) -> None:
        raw = json.loads(self.config.read_text())
        raw["jobs"] = ["quality\ninjected=true"]
        self.config.write_text(json.dumps(raw))
        with self.assertRaises(ContractError):
            load_policy(self.config)

    def test_shared_cargo_target_is_rejected_before_execution(self) -> None:
        with patch.dict(os.environ, {"CARGO_TARGET_DIR": str(self.root.parent)}):
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(main(self.arguments("run", "--task", "quality")), 1)

    def test_rust_defaults_select_optional_aliases_from_trusted_base(self) -> None:
        self.command("init", "--initial-branch=main")
        self.command("config", "user.email", "fixture@example.invalid")
        self.command("config", "user.name", "Fixture")
        cargo = self.root / ".cargo/config.toml"
        cargo.parent.mkdir()
        aliases = (
            "ci-check",
            "ci-policy",
            "ci-clippy",
            "ci-test",
            "ci-doc",
            "ci-doctest",
            "ci-package",
        )
        cargo.write_text("[alias]\n" + "\n".join(f'{name} = "check"' for name in aliases))
        self.command("add", "--all")
        self.command("commit", "--quiet", "-m", "trusted base")
        base = self.command("rev-parse", "HEAD")
        # Candidate deletion cannot remove the previously required package step.
        cargo.write_text('[alias]\nci-check = "check"\n')
        task = _task(
            argparse.Namespace(task="rust-quality", policy_sha=base), load_policy(self.config)
        )
        self.assertIn(("cargo", "ci-package"), task.commands)

    def test_dependency_snapshot_is_exact_and_copies_only_metadata(self) -> None:
        self.command("init", "--initial-branch=main")
        self.command("config", "user.email", "fixture@example.invalid")
        self.command("config", "user.name", "Fixture")
        (self.root / "Cargo.lock").write_text("locked baseline\n")
        (self.root / "private.txt").write_text("not dependency input\n")
        (self.root / ".devcontainer").mkdir()
        dockerfile = self.root / ".devcontainer/Dockerfile"
        declared = (
            "# renovate: datasource=crate depName=cargo-semver-checks\n"
            "ARG CARGO_SEMVER_CHECKS_VERSION=0.51.0\n"
        )
        dockerfile.write_text(declared)
        self.command("add", "--all")
        self.command("commit", "--quiet", "-m", "baseline")
        revision = self.command("rev-parse", "HEAD")
        (self.root / "Cargo.lock").write_text("untrusted working copy\n")
        dockerfile.write_text("untrusted Dockerfile working copy\n")
        target = self.root / "snapshot"
        target.mkdir()
        _snapshot(self.root, revision, target)
        self.assertEqual((target / "Cargo.lock").read_text(), "locked baseline\n")
        self.assertFalse((target / "private.txt").exists())
        self.assertEqual((target / ".devcontainer/Dockerfile").read_text(), declared)


class ArchiveIdentityCli(unittest.TestCase):
    """Actual locked uv/CLI/positive-task chain; only Cargo and registry are fixture stubs."""

    def setUp(self) -> None:
        import shutil
        import textwrap

        from boxferry_ci import bootstrap

        self.bootstrap = bootstrap
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.producer = self.root / "producer"
        self.consumer = self.root / "consumer"
        self.producer.mkdir()
        self.consumer.mkdir()
        canonical = Path(__file__).resolve().parents[2]
        (self.producer / "automation").mkdir()
        for file in (canonical / "automation").iterdir():
            if file.is_file():
                shutil.copy2(file, self.producer / "automation" / file.name)
        shutil.copytree(
            canonical / "automation/src",
            self.producer / "automation/src",
            ignore=shutil.ignore_patterns("__pycache__", "*.pyc"),
        )
        shutil.copytree(canonical / "renovate-presets", self.producer / "renovate-presets")
        (self.producer / ".devcontainer").mkdir()
        shutil.copy2(
            canonical / ".devcontainer/Dockerfile", self.producer / ".devcontainer/Dockerfile"
        )
        self.initialize(self.producer)
        self.automation = self.commit(self.producer, "immutable automation fixture")
        (self.consumer / ".github").mkdir()
        (self.consumer / "scripts").mkdir()
        (self.consumer / "src").mkdir()
        (self.consumer / "target/fixture-bin").mkdir(parents=True)
        (self.consumer / ".gitignore").write_text("target/\n")
        (self.consumer / "Cargo.toml").write_text(
            '[package]\nname="kubernetes-lens"\nversion="0.1.0"\npublish=false\n'
        )
        (self.consumer / "src/lib.rs").write_text("pub fn supported_api() {}\n")
        shutil.copy2(canonical / "rust-toolchain.toml", self.consumer / "rust-toolchain.toml")
        (self.consumer / ".github/automation.json").write_text(
            json.dumps(
                {
                    "schema": 1,
                    "repository": bootstrap.REPOSITORY,
                    "revision": self.automation,
                    "policy": "scripts/validation-policy.json",
                    "interface": 1,
                }
            )
        )
        policy = {
            "schema": 2,
            "repository": "Strukturpiloten/kubernetes-lens",
            "jobs": ["api"],
            "profile_jobs": {"maintenance": ["api"]},
            "public_documentation_root": "docs/",
            "full_documentation_roots": [],
            "executable_documentation_roots": [],
            "executable_documentation_paths": [],
            "documentation_examples_manifest": None,
            "path_profiles": {"maintenance": ["README.md"]},
            "local_commands": {"maintenance": [["true"]], "full": [["true"]]},
            "tasks": {
                "api-published": {
                    "commands": [["python", "scripts/invoke-api.py", "published"]],
                    "timeout_seconds": 30,
                },
                "api-development": {
                    "commands": [["python", "scripts/invoke-api.py", "development"]],
                    "timeout_seconds": 30,
                },
                "api-help": {
                    "commands": [["python", "-m", "boxferry_ci.api_baselines", "run", "--help"]],
                    "timeout_seconds": 10,
                },
                "source-probe": {
                    "commands": [["python", "-c", "print('verified source task')"]],
                    "timeout_seconds": 10,
                },
                "initial-api-contract": {
                    "commands": [["python", "scripts/positive-api.py"]],
                    "timeout_seconds": 10,
                },
            },
        }
        (self.consumer / "scripts/validation-policy.json").write_text(json.dumps(policy))
        (self.consumer / "scripts/positive-api.py").write_text(
            textwrap.dedent("""\
            from pathlib import Path
            import json
            import tomllib
            from boxferry_ci import bootstrap, __main__ as cli
            Path('target/positive-import-origin.json').write_text(json.dumps({
                'bootstrap': str(Path(bootstrap.__file__).resolve()),
                'cli': str(Path(cli.__file__).resolve()),
                'identity': bootstrap.automation_identity(Path.cwd()),
            }))
            package = tomllib.loads(Path('Cargo.toml').read_text())['package']
            assert package['publish'] is False
            assert 'pub fn supported_api()' in Path('src/lib.rs').read_text()
            Path('target/positive-task-ran').write_text('executed')
            raise SystemExit(int(Path('target/task-exit').read_text()))
        """)
        )
        (self.consumer / "scripts/invoke-api.py").write_text(
            textwrap.dedent("""\
            import sys
            import json
            from email.message import Message
            from contextlib import ExitStack
            from pathlib import Path
            from unittest.mock import patch
            from urllib.error import HTTPError
            from boxferry_ci import api_baselines as api, bootstrap, __main__ as cli
            Path('target/api-import-origin.json').write_text(json.dumps({
                'api': str(Path(api.__file__).resolve()),
                'bootstrap': str(Path(bootstrap.__file__).resolve()),
                'cli': str(Path(cli.__file__).resolve()),
                'identity': bootstrap.automation_identity(Path.cwd()),
            }))
            mode = sys.argv[1]
            args = ['run', '--repository-root', '.',
                    '--repository', 'Strukturpiloten/kubernetes-lens',
                    '--api-purpose', mode, '--event', 'local', '--package', 'kubernetes-lens',
                    '--allow-initial', '--config', 'scripts/validation-policy.json',
                    '--report-json', 'target/api-evidence.json']
            baseline = Path('target/baseline').read_text()
            absent = HTTPError(
                'https://crates.io/api/v1/crates/kubernetes-lens',
                404, 'fixture absence', Message(), None,
            )
            with ExitStack() as stack:
                stack.enter_context(patch.object(
                    api.cargo_tools, 'verify', return_value=Path('/fixture/not-a-real-tool'),
                ))
                if mode == 'development':
                    stack.enter_context(patch.object(
                        api, 'default_authority', return_value=('main', baseline),
                    ))
                    stack.enter_context(patch.object(
                        api, 'registry_version', side_effect=AssertionError('private baseline'),
                    ))
                else:
                    stack.enter_context(patch.object(api, 'urlopen', side_effect=absent))
                raise SystemExit(api.main(args))
                with patch.object(api, 'urlopen', side_effect=absent):
                    raise SystemExit(api.main(args))
        """)
        )
        cargo = self.consumer / "target/fixture-bin/cargo"
        cargo.write_text(
            textwrap.dedent("""\
            #!/usr/bin/env python3
            import json, os, sys, tomllib
            from pathlib import Path
            if 'metadata' in sys.argv:
                package = tomllib.loads(Path('Cargo.toml').read_text())['package']
                print(json.dumps({'workspace_members': ['fixture'], 'packages': [{
                    'id':'fixture', 'name':package['name'], 'version':package['version'],
                    'publish':[] if package.get('publish') is False else None,
                    'targets':[{'kind':['lib']}]}]}))
            elif sys.argv[1:3] == ['semver-checks', 'check-release']:
                assert '--baseline-rev' in sys.argv and '--package' in sys.argv
                assert sys.argv[sys.argv.index('--release-type')+1] == 'minor'
                Path(os.environ['FIXTURE_COMPATIBILITY_LOG']).write_text(json.dumps(sys.argv[1:]))
                if os.environ.get('FIXTURE_SOURCE_TAMPER'):
                    source = Path(os.environ['FIXTURE_SOURCE_TAMPER'])
                    source.write_bytes(source.read_bytes() + b'\\n')
                print('fixture compatibility command; no real Cargo backend')
            else:
                raise SystemExit(91)
        """)
        )
        cargo.chmod(0o755)
        self.initialize(self.consumer)
        baseline = self.commit(self.consumer, "existing private library and canonical leaf policy")
        (self.consumer / "target/baseline").write_text(baseline)
        (self.consumer / "workflow.txt").write_text("workflow-only candidate\n")
        self.candidate = self.commit(self.consumer, "candidate differs from automation source")
        self.assertNotEqual(self.candidate, self.automation)
        objects = self.consumer / f"target/boxferry-automation/{self.automation}.git"
        objects.parent.mkdir(parents=True)
        self.git(self.root, "clone", "--bare", str(self.producer), str(objects))
        (objects / "config").write_text(bootstrap.BARE_CONFIG)
        self.project = bootstrap.prepare_implementation(self.consumer, self.automation)
        (self.consumer / "target/task-exit").write_text("0")
        self.uv = Path(os.environ.get("UV") or shutil.which("uv") or "missing-uv")
        version, _assets = bootstrap.uv_manifest(canonical / "automation")
        self.assertTrue(
            bootstrap.uv_version_matches(self.uv, version), "run with canonical locked uv"
        )
        self.environment = dict(
            os.environ,
            UV_OFFLINE="1",
            PYTHONDONTWRITEBYTECODE="1",
            PATH=str(self.consumer / "target/fixture-bin")
            + os.pathsep
            + os.environ.get("PATH", ""),
            FIXTURE_COMPATIBILITY_LOG=str(self.consumer / "target/compatibility.json"),
        )

    def git(self, root: Path, *args: str) -> str:
        return subprocess.run(
            ["git", "-C", str(root), *args],
            check=True,
            capture_output=True,
            text=True,
            env=self.bootstrap.git_environment(),
        ).stdout.strip()

    def initialize(self, root: Path) -> None:
        self.git(root, "init", "-q", "--initial-branch=main", "--template=")
        self.git(root, "config", "user.name", "Archive fixture")
        self.git(root, "config", "user.email", "archive@example.invalid")

    def commit(self, root: Path, message: str) -> str:
        self.git(root, "add", ".")
        self.git(root, "commit", "-qm", message)
        return self.git(root, "rev-parse", "HEAD")

    def task(self, name: str, *, bind: bool = False) -> tuple[int, str]:
        captured = []

        def execute(argv: list[str], env: dict[str, str]) -> int:
            result = subprocess.run(argv, env=env, capture_output=True, text=True, timeout=45)
            captured.append(result.stdout + result.stderr)
            return result.returncode

        args = [
            "run",
            "--task",
            name,
            "--report-json",
            str(self.consumer / "target/task-report.json"),
        ]
        if bind:
            args += ["--automation-sha", self.automation]
        with (
            patch.dict(os.environ, self.environment, clear=True),
            patch.object(self.bootstrap, "ensure_uv", return_value=self.uv),
            patch.object(self.bootstrap.subprocess, "call", side_effect=execute),
        ):
            result = self.bootstrap.run_project(self.consumer, self.project, args)
        return result, "".join(captured)

    def test_locked_archive_resolves_A_and_executes_real_private_positive_success_and_failure(
        self,
    ) -> None:
        result, output = self.task("api-help")
        self.assertEqual(result, 0, output)
        result, output = self.task("api-development")
        self.assertEqual(result, 0, output)
        evidence = json.loads((self.consumer / "target/api-evidence.json").read_text())
        self.assertEqual(evidence["context"]["automation"], self.automation)
        self.assertEqual(evidence["automation_source"]["kind"], "interface-1-archive")
        self.assertEqual(evidence["checks"][0]["kind"], "git")
        self.assertTrue((self.consumer / "target/compatibility.json").exists())
        result, output = self.task("api-published")
        self.assertEqual(result, 0, output)
        initial = json.loads(
            (self.consumer / "target/automation-initial-api-contract.json").read_text()
        )
        self.assertEqual(initial["status"], "success")
        self.assertEqual((self.consumer / "target/positive-task-ran").read_text(), "executed")
        evidence = json.loads((self.consumer / "target/api-evidence.json").read_text())
        self.assertEqual(evidence["context"]["automation"], self.automation)
        self.assertEqual(evidence["checks"][0]["kind"], "initial-contract")
        (self.consumer / "target/task-exit").write_text("7")
        result, output = self.task("api-published")
        self.assertEqual(result, 1, output)
        self.assertNotEqual(
            json.loads((self.consumer / "target/automation-initial-api-contract.json").read_text())[
                "status"
            ],
            "success",
        )
        self.assertEqual(
            json.loads((self.consumer / "target/api-evidence.json").read_text())["status"],
            "failure",
        )

    def test_inherited_foreign_pythonpath_cannot_replace_selected_archive_or_children(self) -> None:
        foreign = self.root / "foreign-producer"
        self.git(self.root, "clone", "--no-hardlinks", str(self.producer), str(foreign))
        self.git(foreign, "config", "user.email", "fixture@example.invalid")
        self.git(foreign, "config", "user.name", "Foreign fixture")
        (foreign / "other-checkout.txt").write_text("different clean producer commit\n")
        other_revision = self.commit(foreign, "different same-version producer")
        self.assertNotEqual(other_revision, self.automation)
        self.assertEqual(
            (foreign / "automation/pyproject.toml").read_bytes(),
            (self.project / "pyproject.toml").read_bytes(),
        )
        self.assertEqual(self.git(foreign, "status", "--porcelain"), "")
        inherited = str(foreign / "automation/src")
        self.environment["PYTHONPATH"] = inherited
        for task, bind in (
            ("api-development", False),
            ("api-published", False),
            ("api-development", True),
            ("api-published", True),
        ):
            with self.subTest(task=task, bind=bind):
                for artifact in (
                    "api-import-origin.json",
                    "api-evidence.json",
                    "positive-import-origin.json",
                    "positive-task-ran",
                    "automation-initial-api-contract.json",
                ):
                    (self.consumer / "target" / artifact).unlink(missing_ok=True)
                # Cover the normal default and the explicit trusted outer CLI binding.
                # The nested initial CLI always verifies explicit A in a real subprocess.
                result, output = self.task(task, bind=bind)
                self.assertEqual(result, 0, output)
                self.assertEqual(self.environment["PYTHONPATH"], inherited)
                evidence = json.loads((self.consumer / "target/api-evidence.json").read_text())
                self.assertEqual(evidence["context"]["automation"], self.automation)
                self.assertEqual(evidence["automation_source"]["kind"], "interface-1-archive")
                origins = ["api-import-origin.json"]
                if task == "api-published":
                    origins.append("positive-import-origin.json")
                    initial = json.loads(
                        (self.consumer / "target/automation-initial-api-contract.json").read_text()
                    )
                    self.assertEqual(initial["status"], "success")
                    self.assertEqual(
                        (self.consumer / "target/positive-task-ran").read_text(), "executed"
                    )
                for name in origins:
                    origin = json.loads((self.consumer / "target" / name).read_text())
                    self.assertEqual(
                        origin.pop("identity"),
                        {"revision": self.automation, "kind": "interface-1-archive"},
                    )
                    for module in origin.values():
                        self.assertTrue(Path(module).is_relative_to(self.project / "src"), origin)
                        self.assertFalse(Path(module).is_relative_to(foreign), origin)
        self.assertEqual(self.git(foreign, "status", "--porcelain"), "")

    def test_locked_archive_A_equals_C_still_rejects_tampered_bytes_before_task(self) -> None:
        self.git(self.consumer, "fetch", str(self.producer), self.automation)
        self.git(self.consumer, "reset", "--soft", self.automation)
        self.assertEqual(self.git(self.consumer, "rev-parse", "HEAD"), self.automation)
        result, output = self.task("source-probe", bind=True)
        self.assertEqual(result, 0, output)
        for relative in ("tool-artifacts.json", "uv.lock", "src/boxferry_ci/__init__.py"):
            file = self.project / relative
            original = file.read_bytes()
            file.write_bytes(original + b"\n")
            result, output = self.task("source-probe", bind=True)
            self.assertEqual(result, 1, output)
            self.assertIn("differs from its commit", output)
            file.write_bytes(original)

    def test_loaded_archive_mutation_during_check_blocks_completion_without_repair(self) -> None:
        source = self.project / "tool-artifacts.json"
        previous = source.read_bytes()
        self.environment["FIXTURE_SOURCE_TAMPER"] = str(source)
        result, output = self.task("api-development")
        self.assertEqual(result, 1, output)
        self.assertIn("differs from its commit", output)
        self.assertEqual(source.read_bytes(), previous + b"\n")
        self.assertEqual(self.git(self.consumer, "status", "--porcelain"), "")
        self.assertEqual(
            json.loads((self.consumer / "target/api-evidence.json").read_text())["status"],
            "failure",
        )

    def test_locked_archive_rejects_changed_manifest_missing_cache_and_unexpected_source(
        self,
    ) -> None:
        manifest = self.consumer / ".github/automation.json"
        previous = manifest.read_bytes()
        config = json.loads(previous)
        config["revision"] = "f" * 40
        manifest.write_text(json.dumps(config))
        result, output = self.task("source-probe", bind=True)
        self.assertEqual(result, 1, output)
        self.assertIn("differs from the consumer manifest", output)
        manifest.write_bytes(previous)
        extra = self.project / "src/boxferry_ci/unexpected.py"
        extra.write_text("not owned by the immutable source\n")
        result, output = self.task("source-probe", bind=True)
        self.assertEqual(result, 1, output)
        self.assertIn("unexpected source", output)
        extra.unlink()
        cache = self.project.parent.parent / f"{self.automation}.git"
        moved = cache.with_suffix(".saved")
        cache.rename(moved)
        result, output = self.task("source-probe", bind=True)
        self.assertEqual(result, 1, output)
        self.assertFalse(cache.exists(), "identity verifier must not fetch, repair or create cache")
