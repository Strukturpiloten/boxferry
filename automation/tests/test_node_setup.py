"""Immutable Node authority and action wiring, without installation or Git writes."""

from __future__ import annotations

import ast
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest
import venv
from contextlib import ExitStack
from pathlib import Path
from unittest.mock import patch

from boxferry_ci import bootstrap, devcontainer_validation, node_setup

ROOT = Path(__file__).resolve().parents[2]
SHA = "a" * 40


class NodeSetupTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.selected = self.root / "selected"
        shutil.copytree(ROOT / "automation/src", self.selected / "automation/src")
        shutil.copyfile(
            ROOT / "automation/pyproject.toml", self.selected / "automation/pyproject.toml"
        )
        (self.selected / ".github/workflows").mkdir(parents=True)
        shutil.copyfile(
            ROOT / ".github/workflows/ci.yml", self.selected / ".github/workflows/ci.yml"
        )
        (self.selected / ".github/actions/shared-node").mkdir(parents=True)
        self.interpreter = self.selected / "automation/.venv/bin/python"
        venv.EnvBuilder(with_pip=False, symlinks=True).create(self.interpreter.parent.parent)
        site = next((self.interpreter.parent.parent / "lib").glob("python*/site-packages"))
        (site / "selected.pth").write_text(str(self.selected / "automation/src") + "\n")
        self.state = self.root / "git-state.json"
        self.metadata: dict[str, str] = {"root": str(self.selected), "head": SHA, "status": ""}
        self.state.write_text(json.dumps(self.metadata))
        binary = self.root / "bin"
        binary.mkdir()
        git = binary / "git"
        git.write_text(
            "#!/usr/bin/python3 -I\nimport json,sys\n"
            f"state=json.load(open({str(self.state)!r}))\n"
            "if '--show-toplevel' in sys.argv:print(state['root'])\n"
            "elif 'rev-parse' in sys.argv:print(state['head'])\n"
            "elif 'status' in sys.argv:print(state['status'])\n"
            "else:raise SystemExit(2)\n"
        )
        git.chmod(0o700)
        self.environment = {**os.environ, "PATH": str(binary) + os.pathsep + os.environ["PATH"]}
        self.output = self.root / "github-output"
        self.canary = self.root / "import-canary"
        malicious = self.root / "malicious/boxferry_ci"
        malicious.mkdir(parents=True)
        (malicious / "__init__.py").write_text(
            f"open({str(self.canary)!r},'w').write('untrusted import')\n"
            "raise RuntimeError('canary')\n"
        )
        self.malicious = malicious.parent

    def invoke(self, *, sha: str = SHA, optimize: bool = False) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                str(self.interpreter),
                "-I",
                *(["-O"] if optimize else []),
                "-m",
                "boxferry_ci.node_setup",
                "--automation-root",
                str(self.selected),
                "--automation-sha",
                sha,
                "--github-output",
                str(self.output),
            ],
            cwd=self.malicious,
            env={
                **self.environment,
                "PYTHONPATH": str(self.malicious),
                "PYTHONHOME": str(self.malicious),
                "PYTHONOPTIMIZE": "2",
            },
            capture_output=True,
            text=True,
            check=False,
            timeout=15,
        )

    def test_isolated_project_reads_canonical_version_ignoring_ambient_python(self) -> None:
        expected = devcontainer_validation.canonical_versions(self.selected)["node"]
        self.output.write_text("existing=value\n")
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertEqual(self.output.read_text(), f"existing=value\nnode_version={expected}\n")
        self.assertFalse(self.canary.exists())

    def test_canonical_change_alone_changes_query_without_another_inventory(self) -> None:
        workflow = self.selected / ".github/workflows/ci.yml"
        original = devcontainer_validation.canonical_versions(self.selected)["node"]
        workflow.write_text(
            workflow.read_text().replace(f"node-version: {original}", "node-version: 99.7.3")
        )
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.output.read_text(), "node_version=99.7.3\n")

    def test_sha_dirty_and_ancestor_refusals_survive_explicit_optimization(self) -> None:
        for fault in ("sha", "malformed-sha", "dirty", "ancestor"):
            with self.subTest(fault=fault):
                state = dict(self.metadata)
                if fault == "dirty":
                    state["status"] = " M changed.py"
                if fault == "ancestor":
                    state["root"] = str(self.root)
                self.state.write_text(json.dumps(state))
                self.output.unlink(missing_ok=True)
                selected_sha = (
                    "b" * 40 if fault == "sha" else "not-a-sha" if fault == "malformed-sha" else SHA
                )
                result = self.invoke(sha=selected_sha, optimize=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, "")
                self.assertFalse(self.output.exists())
                self.assertFalse(self.canary.exists())

    def test_missing_malformed_and_ambiguous_authority_have_no_output(self) -> None:
        workflow = self.selected / ".github/workflows/ci.yml"
        original = workflow.read_text()
        version = devcontainer_validation.canonical_versions(self.selected)["node"]
        for text in (
            original.replace(f"node-version: {version}", "node-version: latest"),
            original + "\n    node-version: 99.8.2\n",
            "---\njobs: {}\n",
        ):
            with self.subTest(text=text[-45:]):
                workflow.write_text(text)
                self.output.unlink(missing_ok=True)
                result = self.invoke()
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(self.output.exists())
                self.assertEqual(result.stdout, "")
        workflow.unlink()
        self.assertNotEqual(self.invoke().returncode, 0)
        self.assertFalse(self.output.exists())
        workflow.write_text(original)
        (self.selected / "automation/pyproject.toml").write_text("[[malformed")
        self.assertNotEqual(self.invoke().returncode, 0)
        self.assertFalse(self.output.exists())

    def test_own_bootstrap_and_canonical_reader_roots_must_all_match(self) -> None:
        filenames = (node_setup, bootstrap, devcontainer_validation)
        for foreign in (*filenames, None):
            with self.subTest(foreign=foreign), ExitStack() as stack:
                for module in filenames:
                    stack.enter_context(
                        patch.object(
                            module,
                            "__file__",
                            str(
                                (self.root if module is foreign else self.selected)
                                / "automation/src/boxferry_ci"
                                / Path(module.__file__).name
                            ),
                        )
                    )
                observe = stack.enter_context(
                    patch.object(
                        bootstrap,
                        "automation_identity",
                        return_value={"revision": SHA, "kind": "git-checkout"},
                    )
                )
                if foreign is None:
                    self.assertEqual(
                        node_setup.node_version(self.selected, SHA),
                        devcontainer_validation.canonical_versions(self.selected)["node"],
                    )
                    observe.assert_called_once_with(self.selected, SHA, allow_dirty_self=False)
                else:
                    with self.assertRaises(node_setup.NodeSetupError):
                        node_setup.node_version(self.selected, SHA)
                    observe.assert_not_called()
        with patch.object(
            bootstrap,
            "automation_identity",
            return_value={"revision": SHA, "kind": "interface-1-archive"},
        ):
            with self.assertRaises(node_setup.NodeSetupError):
                node_setup.node_version(ROOT, SHA)

        with self.assertRaises(node_setup.NodeSetupError):
            node_setup._module_root("/node_setup.py")

    def test_nonisolated_or_foreign_project_interpreter_has_no_output(self) -> None:
        for isolated, executable in ((False, self.interpreter), (True, Path(sys.executable))):
            with (
                self.subTest(isolated=isolated),
                patch.object(node_setup.sys, "flags", type("Flags", (), {"isolated": isolated})()),
                patch.object(node_setup.sys, "executable", str(executable)),
            ):
                self.assertEqual(
                    node_setup.main(
                        [
                            "--automation-root",
                            str(self.selected),
                            "--automation-sha",
                            SHA,
                            "--github-output",
                            str(self.output),
                        ]
                    ),
                    1,
                )
                self.assertFalse(self.output.exists())

    def action_script(self) -> str:
        action = (ROOT / ".github/actions/shared-node/action.yml").read_text()
        return textwrap.dedent(action.split("      run: |\n", 1)[1].split("    - name:", 1)[0])

    def test_action_runs_only_selected_project_and_old_source_fails_without_fallback(self) -> None:
        environment = {
            **self.environment,
            "ACTION_PATH": str(self.selected / ".github/actions/shared-node"),
            "AUTOMATION_SHA": SHA,
            "GITHUB_OUTPUT": str(self.output),
        }
        result = subprocess.run(
            ["bash", "-c", self.action_script()],
            env=environment,
            cwd=self.malicious,
            capture_output=True,
            text=True,
            timeout=15,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.output.read_text().startswith("node_version="))
        self.assertFalse((self.selected / "package-lock.json").exists())
        self.output.unlink()
        (self.selected / "automation/src/boxferry_ci/node_setup.py").unlink()
        result = subprocess.run(
            ["bash", "-c", self.action_script()],
            env=environment,
            capture_output=True,
            text=True,
            timeout=15,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.output.exists())
        self.interpreter.unlink()
        result = subprocess.run(
            ["bash", "-c", self.action_script()],
            env=environment,
            capture_output=True,
            text=True,
            timeout=15,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("requires the selected shared Python project", result.stderr)
        self.assertFalse(self.output.exists())

    def test_fake_setup_node_receives_dynamic_version_and_explicit_cache_inputs(self) -> None:
        action = (ROOT / ".github/actions/shared-node/action.yml").read_text()
        setup = action.split("    - name: Install the canonical Node toolchain", 1)[1]
        self.assertIn("actions/setup-node@949feb2413d6458794dcd2491c4babbbce0c15c1 # v7.1.0", setup)
        bindings = dict(re.findall(r"^        ([a-z-]+): (.+)$", setup, re.MULTILINE))
        self.assertEqual(set(bindings), {"node-version", "cache", "cache-dependency-path"})
        for cache, lock in (("", ""), ("npm", "package-lock.json")):
            expressions = {
                "steps.version.outputs.node_version": "99.7.3",
                "inputs.cache": cache,
                "inputs.cache-dependency-path": lock,
            }
            received = {}
            for key, value in bindings.items():
                match = re.fullmatch(r"\$\{\{ (.+) \}\}", value)
                if match is None:
                    self.fail("setup-node input must use the selected output or caller cache input")
                received[key] = expressions[match[1]]
            self.assertEqual(
                received, {"node-version": "99.7.3", "cache": cache, "cache-dependency-path": lock}
            )
        inputs = action.split("inputs:\n", 1)[1].split("outputs:\n", 1)[0]
        self.assertNotIn("node-version:", inputs)
        self.assertEqual(inputs.count('default: ""'), 2)

    def test_updater_order_keeps_no_cache_and_only_automation_checkout(self) -> None:
        workflow = (ROOT / ".github/workflows/shared-devcontainer-updates.yml").read_text()
        ordering = [
            workflow.index(value)
            for value in (
                "path: _automation",
                "actions/shared-python",
                "actions/shared-node",
                "npm ci --ignore-scripts --prefix _automation/automation/devcontainer",
            )
        ]
        self.assertEqual(ordering, sorted(ordering))
        node = workflow.split("uses: ./_automation/.github/actions/shared-node", 1)[1].split(
            "      - name:", 1
        )[0]
        self.assertIn('cache: ""', node)
        self.assertNotIn("cache-dependency-path", node)
        self.assertIn("automation-sha: ${{ inputs.automation-sha }}", node)
        self.assertNotIn("node-version:", workflow)
        self.assertEqual(workflow.count("uses: actions/checkout@"), 1)
        self.assertNotIn("schedule:", workflow)

    def test_native_probe_requires_exact_successful_selected_node_version(self) -> None:
        tools = self.root / "bin"
        for name, value in (("id", "1000"), ("uname", "x86_64")):
            executable = tools / name
            executable.write_text("#!/bin/sh\nprintf '%s\\n' " + value + "\n")
            executable.chmod(0o700)
        (self.root / ".devcontainer").mkdir()
        (self.root / ".devcontainer/verify-tools.sh").write_text("exit 0\n")
        with patch.object(devcontainer_validation, "PROBES", (("node", "--version"),)):
            script = devcontainer_validation.probe_script({"node": "99.7.3"}, {}, "boxferry")
        script = script.replace("/tmp/qualification-home", str(self.root / "owned-home"))
        for observed, status, expected in (
            ("v99.7.3", 0, 0),
            ("v99.7.2", 0, 1),
            ("99.7.3", 0, 1),
            ("v99.7.3 extra", 0, 1),
            ("v99.7.3", 23, 23),
            ("v99.7.2" + "x" * 2000, 0, 1),
        ):
            executable = tools / "node"
            executable.write_text(
                "#!/bin/sh\nprintf '%s\\n' " + shlex.quote(observed) + f"\nexit {status}\n"
            )
            executable.chmod(0o700)
            with self.subTest(observed=observed[:35], status=status):
                result = subprocess.run(
                    ["bash", "-c", script, "qualification", "x86_64"],
                    cwd=self.root,
                    env={"PATH": str(tools) + ":/usr/bin:/bin"},
                    capture_output=True,
                    text=True,
                    timeout=5,
                )
                self.assertEqual(result.returncode, expected, result.stderr)
                self.assertIn("probe: node", result.stdout)
                if status == 0 and expected != 0:
                    self.assertIn("Version mismatch for node: expected v99.7.3", result.stderr)
                    self.assertLess(len(result.stderr), 360)

    def test_producer_smoke_binds_same_candidate_after_literal_node(self) -> None:
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        smoke = workflow.split("      - name: Validate candidate shared Node composite", 1)[
            1
        ].split("      - name:", 1)[0]
        self.assertIn("uses: ./_automation/.github/actions/shared-node", smoke)
        self.assertIn("automation-sha: ${{ github.sha }}", smoke)
        self.assertIn('cache: ""', smoke)
        self.assertNotIn("cache-dependency-path", smoke)
        self.assertIn("not an admitted-source receipt", workflow)
        order = [
            workflow.index(value)
            for value in (
                "path: _automation",
                "actions/shared-python",
                "Install pinned Node.js toolchain",
                "Validate candidate shared Node composite",
                "Check Python formatting",
            )
        ]
        self.assertEqual(order, sorted(order))
        self.assertEqual(
            len(re.findall(r"^\s+node-version: [0-9]+\.[0-9]+\.[0-9]+$", workflow, re.MULTILINE)), 1
        )

    def test_production_refusal_checks_do_not_use_assert(self) -> None:
        tree = ast.parse(Path(node_setup.__file__).read_text())
        self.assertFalse(any(isinstance(node, ast.Assert) for node in ast.walk(tree)))


if __name__ == "__main__":
    unittest.main()
