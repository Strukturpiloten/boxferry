"""Controlled immutable producer fixtures; no registry or network access."""

from __future__ import annotations

import io
import json
import os
import shutil
import subprocess
import tarfile
import unittest
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import Any, cast
from unittest.mock import patch

import test_bootstrap as bootstrap_fixtures
from test_devcontainer_validation import write_markdownlint_metadata

from boxferry_ci import bootstrap
from boxferry_ci import devcontainer_validation as validation


class NodeAcquisitionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.fixture = bootstrap_fixtures.BootstrapTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root
        self.fixture.fixture_cache()
        producer = self.root / "fixture"
        (producer / "scripts").mkdir()
        (producer / "package.json").write_text('{"private": true}')
        (producer / "package-lock.json").write_text('{"lockfileVersion": 3,"packages":{"":{}}}')
        (producer / "scripts/run-node-tool.sh").write_text(
            'test -f "$1/node_modules/installed" || exit 2\n'
            'if [[ "$3" == --version ]]; then exit 0; fi\n'
            'printf "%s\\n" "$PWD" "$@" > invocation.txt\nexit 7\n'
        )
        self.admit()
        self.calls = 0

    def admit(self) -> None:
        producer = self.root / "fixture"
        self.fixture.git(producer, "add", ".")
        self.fixture.git(producer, "commit", "-m", "Node producer fixture")
        self.revision = self.fixture.git(producer, "rev-parse", "HEAD")
        objects = self.root / f"target/boxferry-automation/{self.revision}.git"
        self.fixture.git(self.root, "clone", "--bare", str(producer), str(objects))
        (objects / "config").write_text(bootstrap.BARE_CONFIG)
        self.fixture.config["revision"] = self.revision
        self.fixture.manifest()

    def install(self, argv: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        self.assertEqual(
            argv[:6], ["npm", "ci", "--ignore-scripts", "--include=dev", "--no-audit", "--no-fund"]
        )
        self.calls += 1
        stage = Path(argv[-1])
        (stage / "node_modules").mkdir()
        (stage / "node_modules/installed").write_text("complete")
        return subprocess.CompletedProcess(argv, 0)

    def invoke(self, *extra: str) -> int:
        return bootstrap.run_node_tool(
            self.root,
            [
                "--cwd",
                str(self.root),
                *extra,
                "prettier",
                "--config",
                "rules with spaces.json",
                "--check",
                "literal *.md",
            ],
        )

    def test_cold_warm_corrupt_revision_and_read_only(self) -> None:
        real_run = subprocess.run

        def dispatch(argv: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
            return (
                self.install(argv, **kwargs)
                if argv[0] == "npm"
                else real_run(argv, **cast(dict[str, Any], kwargs))
            )

        with patch.object(bootstrap.subprocess, "run", side_effect=dispatch):
            self.assertEqual(self.invoke(), 7)
            self.assertEqual(self.invoke(), 7)
            self.assertEqual(self.calls, 2)
            root = self.root / "target/boxferry-node-tools" / self.revision
            before = {p: p.stat().st_mtime_ns for p in root.rglob("*")}
            with self.assertRaisesRegex(bootstrap.BootstrapError, "root-owned"):
                self.invoke("--read-only")
            self.assertEqual(before, {p: p.stat().st_mtime_ns for p in root.rglob("*")})
            self.assertIn(
                "rules with spaces.json\n--check\nliteral *.md",
                (self.root / "invocation.txt").read_text(),
            )
            (root / "node_modules/installed").write_text("corrupted")
            with self.assertRaisesRegex(bootstrap.BootstrapError, "root-owned"):
                self.invoke("--read-only")
            self.assertEqual(self.invoke(), 7)
            (root / "scripts/run-node-tool.sh").write_text("exit 0")
            self.assertEqual(self.invoke(), 7)
            self.assertEqual(self.calls, 4)
            (self.root / "fixture/package.json").write_text('{"private":true,"description":"new"}')
            self.admit()
            self.assertEqual(self.invoke(), 7)
            self.assertEqual(self.calls, 5)

    def test_parallel_install_and_failed_install_never_publish_partial_graph(self) -> None:
        real_run = subprocess.run

        def dispatch(argv: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
            return (
                self.install(argv, **kwargs)
                if argv[0] == "npm"
                else real_run(argv, **cast(dict[str, Any], kwargs))
            )

        with patch.object(bootstrap.subprocess, "run", side_effect=dispatch):
            with ThreadPoolExecutor(max_workers=3) as workers:
                self.assertEqual(list(workers.map(lambda _: self.invoke(), range(3))), [7] * 3)
        self.assertEqual(self.calls, 3)
        root = self.root / "target/boxferry-node-tools" / self.revision
        (root / "node_modules/installed").write_text("stale")
        before = (root / ".boxferry-node.json").read_bytes()

        def fail_install(argv: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
            if argv[0] == "npm":
                raise subprocess.CalledProcessError(1, "npm")
            return real_run(argv, **cast(dict[str, Any], kwargs))

        with patch.object(bootstrap.subprocess, "run", side_effect=fail_install):
            with self.assertRaises(subprocess.CalledProcessError):
                self.invoke()
        self.assertEqual((root / ".boxferry-node.json").read_bytes(), before)

    def test_unsafe_missing_duplicate_and_oversized_exports_refuse(self) -> None:
        for name, kind, size in (
            ("../package.json", tarfile.REGTYPE, 0),
            ("package.json", tarfile.SYMTYPE, 0),
            ("package.json", tarfile.REGTYPE, bootstrap.NODE_BYTES + 1),
        ):
            data = io.BytesIO()
            with tarfile.open(fileobj=data, mode="w") as archive:
                member = tarfile.TarInfo(name)
                member.type, member.size = kind, size
                archive.addfile(member, io.BytesIO(b"x" * size))
            with patch.object(bootstrap, "run_git", return_value=data.getvalue()):
                with self.assertRaises(bootstrap.BootstrapError):
                    bootstrap.node_sources(self.root, self.revision)

    def test_handoff_authenticates_before_node_dispatch_without_uv(self) -> None:
        with (
            patch.object(
                bootstrap, "automation_identity", return_value={"revision": self.revision}
            ) as identity,
            patch.object(bootstrap, "ensure_uv", side_effect=AssertionError("unexpected uv")),
            patch.object(bootstrap, "run_node_tool", return_value=7) as node,
        ):
            self.assertEqual(
                bootstrap.run_project(self.root, self.root, ["node-tool", "--help"]), 7
            )
            identity.assert_called_once_with(self.root, self.revision)
            node.assert_called_once_with(self.root, ["--help"])
        with (
            patch.object(
                bootstrap, "automation_identity", side_effect=bootstrap.BootstrapError("unverified")
            ),
            patch.object(bootstrap, "run_node_tool") as node,
        ):
            self.assertEqual(bootstrap.run_project(self.root, self.root, ["node-tool"]), 2)
            node.assert_not_called()
        with patch.object(bootstrap, "run_git", return_value=b"\0" * 10240):
            with self.assertRaisesRegex(bootstrap.BootstrapError, "lacks"):
                bootstrap.node_sources(self.root, self.revision)

    def test_verified_archive_handoff_and_tampering(self) -> None:
        source = self.root / "fixture/automation/src/boxferry_ci/bootstrap.py"
        source.parent.mkdir(parents=True)
        source.write_bytes(Path(bootstrap.__file__).read_bytes())
        self.admit()
        project = bootstrap.prepare_implementation(self.root, self.revision)
        loaded = project / "src/boxferry_ci/bootstrap.py"
        with (
            patch.object(bootstrap, "__file__", str(loaded)),
            patch.object(bootstrap, "ensure_uv", side_effect=AssertionError("unexpected uv")),
            patch.object(bootstrap, "run_node_tool", return_value=7) as node,
        ):
            self.assertEqual(bootstrap.run_project(self.root, project, ["node-tool"]), 7)
            loaded.write_text("tampered = True")
            self.assertEqual(bootstrap.run_project(self.root, project, ["node-tool"]), 2)
            node.assert_called_once()

    def test_parallel_frontloaders_share_cold_node_install_without_uv(self) -> None:
        source = self.root / "fixture/automation/src/boxferry_ci/bootstrap.py"
        source.parent.mkdir(parents=True)
        source.write_bytes(Path(bootstrap.__file__).read_bytes())
        self.admit()
        scripts = self.root / "scripts"
        scripts.mkdir()
        launcher = scripts / "shared-ci.py"
        launcher.write_bytes(
            (Path(__file__).resolve().parents[2] / "templates/shared-ci.py").read_bytes()
        )
        binaries = self.root / "bin"
        binaries.mkdir()
        npm = binaries / "npm"
        npm.write_text(
            '#!/bin/bash\nroot="${@: -1}"\nmkdir "$root/node_modules"\n'
            'echo complete > "$root/node_modules/installed"\necho install >> "$NODE_FIXTURE_LOG"\n'
        )
        npm.chmod(0o755)
        log = self.root / "npm.log"
        environment = os.environ | {
            "PATH": str(binaries) + os.pathsep + os.environ["PATH"],
            "NODE_FIXTURE_LOG": str(log),
        }
        command = [
            "python3",
            str(launcher),
            "node-tool",
            "--cwd",
            str(self.root),
            "prettier",
            "--check",
            "literal *.md",
        ]
        processes = [
            subprocess.Popen(
                command, env=environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE
            )
            for _ in range(2)
        ]
        for process in processes:
            stdout, stderr = process.communicate(timeout=30)
            self.assertEqual(process.returncode, 7, (stdout, stderr))
        self.assertEqual(log.read_text(), "install\n" * 2)
        project = self.root / "target/boxferry-automation" / self.revision / "automation"
        self.assertFalse((project / ".venv").exists())
        self.assertFalse((project.parent / "package.json").exists())

    def test_unowned_explicit_installation_is_preserved(self) -> None:
        target = self.root / "unrelated"
        target.mkdir()
        retained = target / "retain.txt"
        retained.write_text("not a Node cache")
        with self.assertRaisesRegex(bootstrap.BootstrapError, "unowned"):
            bootstrap.run_node_tool(
                self.root,
                ["--cwd", str(self.root), "--installation-root", str(target), "--install-only"],
            )
        self.assertEqual(retained.read_text(), "not a Node cache")

    def test_ordinary_frontloader_releases_lock_before_nested_handoff(self) -> None:
        implementation = self.root / "fixture/automation/src/boxferry_ci/bootstrap.py"
        implementation.parent.mkdir(parents=True)
        implementation.write_text(
            "import fcntl, os\n"
            "def run_project(root, project, arguments):\n"
            '    fd = os.open(root / "target/boxferry-automation/dispatch.lock", os.O_RDWR)\n'
            "    try:\n"
            "        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)\n"
            "        return 7\n"
            "    finally:\n"
            "        os.close(fd)\n"
        )
        self.admit()
        scripts = self.root / "scripts"
        scripts.mkdir()
        launcher = scripts / "shared-ci.py"
        launcher.write_bytes(
            (Path(__file__).resolve().parents[2] / "templates/shared-ci.py").read_bytes()
        )
        result = subprocess.run(["python3", str(launcher), "plan"], capture_output=True)
        self.assertEqual(result.returncode, 7, result.stderr)

    def test_install_only_then_explicit_root_defaults_to_read_only(self) -> None:
        target = self.root / "image-tools"
        real_run = subprocess.run

        def dispatch(argv: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
            if argv[0] == "npm":
                return self.install(argv, **kwargs)
            return real_run(argv, **cast(dict[str, Any], kwargs))

        with patch.object(bootstrap.subprocess, "run", side_effect=dispatch):
            self.assertEqual(
                bootstrap.run_node_tool(
                    self.root,
                    ["--cwd", str(self.root), "--installation-root", str(target), "--install-only"],
                ),
                0,
            )
            with self.assertRaisesRegex(bootstrap.BootstrapError, "root-owned"):
                self.invoke("--installation-root", str(target))
            with (
                patch.object(bootstrap, "verify_node_custody"),
                patch.object(bootstrap.os, "open", side_effect=AssertionError("image root write")),
            ):
                self.assertEqual(self.invoke("--installation-root", str(target)), 7)
        self.assertEqual(self.calls, 1)

    def test_matching_writable_receipt_cannot_authorize_altered_fixture_data(self) -> None:
        real_run, real_call = subprocess.run, subprocess.call

        def dispatch(argv: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
            if argv[0] == "npm":
                self.assertEqual(kwargs["stdout"], bootstrap.sys.stderr)
                self.assertEqual(kwargs["stdin"], subprocess.DEVNULL)
                self.assertIn("--prefer-offline", argv)
                return self.install(argv, **kwargs)
            return real_run(argv, **cast(dict[str, Any], kwargs))

        with patch.object(bootstrap.subprocess, "run", side_effect=dispatch):
            self.assertEqual(self.invoke(), 7)
            installation = self.root / "target/boxferry-node-tools" / self.revision
            data = installation / "node_modules/installed"
            data.write_text("harmless altered fixture data")
            (installation / ".boxferry-node.json").write_text(
                json.dumps(
                    {"revision": self.revision, "inventory": bootstrap.node_inventory(installation)}
                )
            )

            def execute(argv: list[str], **kwargs: object) -> int:
                self.assertEqual(data.read_text(), "complete")
                return real_call(argv, **cast(dict[str, Any], kwargs))

            with patch.object(bootstrap.subprocess, "call", side_effect=execute):
                self.assertEqual(self.invoke(), 7)
        self.assertEqual(self.calls, 2)

    def test_failed_npm_status_is_preserved_without_tool_dispatch(self) -> None:
        with (
            patch.object(bootstrap, "automation_identity"),
            patch.object(
                bootstrap, "run_node_tool", side_effect=subprocess.CalledProcessError(17, "npm")
            ),
        ):
            self.assertEqual(bootstrap.run_project(self.root, self.root, ["node-tool"]), 17)

    def test_missing_registry_integrity_is_refused_before_install_or_tool(self) -> None:
        sources = {
            "package-lock.json": json.dumps(
                {
                    "lockfileVersion": 3,
                    "packages": {
                        "": {},
                        "node_modules/fixture": {
                            "version": "1.0.0",
                            "resolved": "https://registry.npmjs.org/fixture/-/fixture-1.0.0.tgz",
                        },
                    },
                }
            ).encode()
        }
        with (
            patch.object(bootstrap, "node_sources", return_value=sources),
            patch.object(bootstrap.subprocess, "call") as tool,
        ):
            with self.assertRaisesRegex(bootstrap.BootstrapError, "SHA512 integrity"):
                self.invoke()
            tool.assert_not_called()

    def test_image_entry_reads_manifest_without_checkout_cache_or_setup(self) -> None:
        target = self.root / "image-tools"
        real_run = subprocess.run

        def dispatch(argv: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
            if argv[0] == "npm":
                return self.install(argv, **kwargs)
            return real_run(argv, **cast(dict[str, Any], kwargs))

        with patch.object(bootstrap.subprocess, "run", side_effect=dispatch):
            self.assertEqual(
                bootstrap.run_node_tool(
                    self.root,
                    ["--cwd", str(self.root), "--installation-root", str(target), "--install-only"],
                ),
                0,
            )
        shutil.rmtree(self.root / "target")
        arguments = [
            "image-node-tool",
            "--repository-root",
            str(self.root),
            "--cwd",
            str(self.root),
            "prettier",
            "--version",
        ]
        with (
            patch.object(bootstrap, "__file__", str(target / bootstrap.NODE_IMAGE_ENTRY)),
            patch.object(bootstrap, "verify_node_custody"),
            patch.object(
                bootstrap, "prepare_object_cache", side_effect=AssertionError("image fetch")
            ),
            patch.object(bootstrap.os, "open", side_effect=AssertionError("image write")),
        ):
            self.assertEqual(bootstrap.main(arguments), 0)
            self.fixture.config["revision"] = "b" * 40
            self.fixture.manifest()
            with patch.object(bootstrap.subprocess, "call") as tool:
                self.assertEqual(bootstrap.main(arguments), 2)
                tool.assert_not_called()
        self.assertFalse((self.root / "target").exists())

    @unittest.skipUnless(
        os.environ.get("BOXFERRY_NODE_INTEGRATION") == "1",
        "opt-in real npm/Prettier regression requires cached producer assets",
    )
    def test_real_prettier_stdin_cold_warm_stdout_and_status(self) -> None:
        source = Path(
            os.environ.get(
                "BOXFERRY_NODE_INTEGRATION_SOURCE", str(Path(__file__).resolve().parents[2])
            )
        )
        for relative in bootstrap.NODE_FILES:
            (self.root / "fixture" / relative).write_bytes((source / relative).read_bytes())
        implementation = self.root / "fixture/automation/src/boxferry_ci/bootstrap.py"
        implementation.parent.mkdir(parents=True)
        implementation.write_bytes(Path(bootstrap.__file__).read_bytes())
        self.admit()
        scripts = self.root / "scripts"
        scripts.mkdir()
        launcher = scripts / "shared-ci.py"
        launcher.write_bytes(
            (Path(__file__).resolve().parents[2] / "templates/shared-ci.py").read_bytes()
        )
        command = [
            "python3",
            str(launcher),
            "node-tool",
            "--cwd",
            str(self.root),
            "prettier",
            "--parser",
            "json",
        ]
        environment = os.environ | {"npm_config_offline": "true"}
        results = [
            subprocess.run(command, input=b'{"answer":42}', capture_output=True, env=environment)
            for _ in range(2)
        ]
        self.assertEqual(
            [(r.returncode, r.stdout) for r in results], [(0, b'{ "answer": 42 }\n')] * 2
        )
        self.assertEqual(json.loads(results[0].stdout), {"answer": 42})
        self.assertTrue(all(b"added" in result.stderr for result in results))
        invalid = [
            subprocess.run(command, input=b"{", capture_output=True, env=environment)
            for _ in range(2)
        ]
        self.assertEqual([(r.returncode, r.stdout) for r in invalid], [(2, b"")] * 2)
        marker = self.root / "target/boxferry-node-tools" / self.revision / ".boxferry-node.json"
        retained = marker.read_bytes()
        unavailable = subprocess.run(
            command,
            input=b'{"answer":42}',
            capture_output=True,
            env=environment | {"npm_config_cache": str(self.root / "empty-npm-cache")},
        )
        self.assertEqual((unavailable.returncode, unavailable.stdout), (1, b""))
        self.assertIn(b"ENOTCACHED", unavailable.stderr)
        self.assertEqual(marker.read_bytes(), retained)

    @unittest.skipUnless(
        os.environ.get("BOXFERRY_NODE_INTEGRATION") == "1",
        "opt-in nested public regression requires cached producer assets",
    )
    def test_real_nested_tasks_keep_parent_and_independent_consumer_project(self) -> None:
        source = Path(__file__).resolve().parents[2]
        producer = self.root / "fixture"
        shutil.rmtree(producer / "automation")
        shutil.copytree(source / "automation/src", producer / "automation/src")
        for name in ("pyproject.toml", "uv.lock", "uv-artifacts.json"):
            shutil.copyfile(source / "automation" / name, producer / "automation" / name)
        for relative in bootstrap.NODE_FILES:
            shutil.copyfile(source / relative, producer / relative)
        self.admit()
        scripts = self.root / "scripts"
        scripts.mkdir()
        shutil.copyfile(source / "templates/shared-ci.py", scripts / "shared-ci.py")
        # A separate dependency-free consumer project must never sync the parent
        # shared environment, even though it inherits that environment's Python.
        (self.root / "pyproject.toml").write_text(
            '[project]\nname="nested-consumer"\nversion="0.1.0"\nrequires-python=">=3.13"\n'
        )
        uv = bootstrap.ensure_uv(producer / "automation", self.root / "uv-tools")
        environment = os.environ.copy()
        for name in ("UV_PROJECT_ENVIRONMENT", "VIRTUAL_ENV", "PYTHONPATH"):
            environment.pop(name, None)
        environment.update(npm_config_offline="true", UV_OFFLINE="1")
        subprocess.run([str(uv), "lock", "--offline"], cwd=self.root, env=environment, check=True)
        (self.root / ".prettierrc.json").write_text('{"tabWidth":4}')
        (self.root / ".markdownlint.json").write_text('{"default":true,"MD013":false}')
        (self.root / ".gitignore").write_text("target/\n.venv/\nfixture/\nuv-tools/\n")
        (self.root / "README.md").write_text("# Nested task\n")
        (self.root / "probe.sh").write_text(
            "set -euo pipefail\n"
            "parent_python=$(command -v python3)\n"
            "python3 -c 'import boxferry_ci.runner'\n"
            'test -z "${UV_PROJECT_ENVIRONMENT:-}"\n'
            'python3 scripts/shared-ci.py node-tool --cwd "$PWD" prettier '
            "--config .prettierrc.json --check README.md\n"
            'test -x "$parent_python"\n'
            'python3 scripts/shared-ci.py node-tool --cwd "$PWD" markdownlint-cli2 '
            "--config .markdownlint.json README.md\n"
            # Both nested public entries contend for export custody while their
            # enclosing task still uses the shared Python environment.
            'python3 scripts/shared-ci.py node-tool --cwd "$PWD" prettier '
            "--config .prettierrc.json --check README.md &\n"
            "formatter_pid=$!\n"
            'python3 scripts/shared-ci.py node-tool --cwd "$PWD" markdownlint-cli2 '
            "--config .markdownlint.json README.md\n"
            'wait "$formatter_pid"\n'
            "python3 -c 'import boxferry_ci.policy, boxferry_ci.runner'\n"
            "uv sync --locked --offline\n"
            'test -x "$PWD/.venv/bin/python3"\n'
            "uv run --frozen --offline python -c 'import importlib.util; "
            'assert importlib.util.find_spec("boxferry_ci") is None\'\n'
            '"$parent_python" -c \'import boxferry_ci.runner; print("parent-import-ok")\'\n'
            "exit ${PROBE_EXIT:-0}\n"
        )
        policy = {
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
            "local_commands": {"maintenance": [["bash", "probe.sh"]], "full": [["true"]]},
            "tasks": {"quality": {"commands": [["bash", "probe.sh"]], "timeout_seconds": 120}},
        }
        (scripts / "validation-policy.json").write_text(json.dumps(policy))
        command = [
            "python3",
            "scripts/shared-ci.py",
            "run",
            "--task",
            "quality",
            "--report-json",
            "report.json",
        ]
        for expected in (0, 0, 1):
            result = subprocess.run(
                command,
                cwd=self.root,
                env=environment | {"PROBE_EXIT": "37" if expected else "0"},
                capture_output=True,
                text=True,
                timeout=150,
            )
            self.assertEqual(result.returncode, expected, result.stdout + result.stderr)
            self.assertIn("parent-import-ok", result.stdout)
            self.assertIn("Linting: 1 file", result.stdout)
            report = json.loads((self.root / "report.json").read_text())
            self.assertEqual(report["results"][0]["returncode"], 37 if expected else 0)
        # Actual formatter failure propagates through the public task report.
        (self.root / "README.md").write_text("# Unformatted\n\ntext   \n")
        result = subprocess.run(
            command, cwd=self.root, env=environment, capture_output=True, text=True, timeout=150
        )
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        report = json.loads((self.root / "report.json").read_text())
        self.assertEqual(report["results"][0]["returncode"], 1)
        self.assertNotIn("parent-import-ok", result.stdout)
        # A Prettier-clean document still fails the independent Markdown rules.
        (self.root / "README.md").write_text("# First\n\n# Second\n")
        result = subprocess.run(
            command, cwd=self.root, env=environment, capture_output=True, text=True, timeout=150
        )
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("MD025", result.stdout + result.stderr)
        self.assertNotIn("parent-import-ok", result.stdout)

    def test_image_source_is_explicit_and_bound_without_duplicate_ownership(self) -> None:
        producer = self.root / "fixture"
        write_markdownlint_metadata(producer)
        write_markdownlint_metadata(self.root)
        expected = validation.markdownlint_versions(self.root)
        marker = self.root / ".github/node-tools.json"
        self.assertEqual(
            validation.selected_markdownlint_versions(self.root, producer, self.revision), expected
        )
        marker.write_text('{"schema":1,"source":"local"}')
        self.assertEqual(
            validation.selected_markdownlint_versions(self.root, producer, self.revision), expected
        )
        marker.write_text('{"schema":1,"source":"automation"}')
        with self.assertRaises(validation.ValidationError):
            validation.selected_markdownlint_versions(self.root, producer, self.revision)
        (self.root / "package.json").write_text('{"dependencies":{"unrelated":"1.0.0"}}')
        self.assertEqual(
            validation.selected_markdownlint_versions(self.root, producer, self.revision), expected
        )
        with self.assertRaises(validation.ValidationError):
            validation.selected_markdownlint_versions(self.root, producer, "b" * 40)
        for text in (
            "{}",
            '{"schema":true,"source":"local"}',
            '{"schema":1.0,"source":"local"}',
            '{"schema":1,"source":"local","source":"automation"}',
        ):
            marker.write_text(text)
            with self.assertRaises(validation.ValidationError):
                validation.selected_markdownlint_versions(self.root, producer, self.revision)
        marker.write_text('{"schema":1,"source":"automation"}')
        (producer / "package-lock.json").write_text("{}")
        with self.assertRaises(validation.ValidationError):
            validation.selected_markdownlint_versions(self.root, producer, self.revision)


if __name__ == "__main__":
    unittest.main()
