"""Inert provider-observation topology, source closure, and executable input boundaries.

This is an authored fixture contract, not provider evidence or a recovery classifier.
"""

from __future__ import annotations

import ast
import contextlib
import copy
import io
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from typing import Any
from unittest.mock import patch

import yaml

ROOT = Path(__file__).resolve().parents[2]
DIRECTORY = ROOT / ".github/workflows"
CALLER = "recovery-cancellation-provider-probe.yml"
PREPARE = "recovery-cancellation-probe-prepare.yml"
EFFECT = "recovery-cancellation-probe-effect.yml"
CANONICAL_WORKFLOW = "shared-rust-ci.yml"
FENCE = (
    "${{ github.repository == 'Strukturpiloten/boxferry' && "
    "github.event_name == 'workflow_dispatch' && "
    "github.event.repository.default_branch == 'main' && "
    "github.ref == 'refs/heads/main' && github.run_attempt == 1 }}"
)
COMMON_INPUTS = {"case", "series", "expected_source_sha"}
EXTRA_INPUTS = {
    "preparation_artifact_id",
    "preparation_artifact_digest",
    "preparation_marker",
    "caller_prerequisite_result",
}


def workflow_sources() -> dict[str, str]:
    return {name: (DIRECTORY / name).read_text() for name in (CALLER, PREPARE, EFFECT)}


def parse_workflows(sources: dict[str, str]) -> dict[str, Any]:
    # BaseLoader preserves GitHub's YAML 1.2 `on` instead of YAML 1.1 boolean coercion.
    return {name: yaml.load(source, Loader=yaml.BaseLoader) for name, source in sources.items()}


def load_workflows() -> dict[str, Any]:
    return parse_workflows(workflow_sources())


def reviewed_metadata(source: str, job_key: str) -> tuple[str, str, str]:
    """Read one full SHA/exact-tag pair and runner from reviewed workflow source.

    Comments are source evidence, not supplied by PyYAML. Both views must agree;
    missing, duplicate, conflicting and malformed metadata refuse this contract.
    No canonical workflow code is executed or imported.
    """
    document = yaml.load(source, Loader=yaml.BaseLoader)
    job = document["jobs"][job_key]
    declarations = re.findall(
        r"(?m)^ +uses: (actions/upload-artifact@[a-f0-9]{40}) # (v[0-9]+\.[0-9]+\.[0-9]+) *$",
        source,
    )
    runners = re.findall(r"(?m)^ +runs-on: (ubuntu-[0-9]{2}\.[0-9]{2}) *$", source)
    uploaders = [
        step["uses"]
        for candidate in document["jobs"].values()
        for step in candidate.get("steps", [])
        if str(step.get("uses", "")).startswith("actions/upload-artifact")
    ]
    assert len(declarations) == len(uploaders) == len(runners) == 1, "ambiguous metadata"
    uploader, tag = declarations[0]
    assert uploaders == [uploader], "malformed uploader declaration"
    assert job["runs-on"] == runners[0], "malformed canonical runner"
    return uploader, tag, runners[0]


def python_body(step: dict[str, Any]) -> str:
    script = step["run"]
    prefix = "set -euo pipefail\npython3 - <<'PY'\n"
    assert script.startswith(prefix) and script.endswith("PY\n"), "foreign shell command"
    return script[len(prefix) : -3]


def inert_python(step: dict[str, Any]) -> None:
    """Close executable operations independently of YAML spelling or script hash."""
    body = python_body(step)
    assert "${{" not in body, "expression interpolated into executable source"
    tree = ast.parse(body)
    calls = {
        "print",
        "range",
        "SystemExit",
        "Path",
        "open",
        "len",
        "json.dumps",
        "json.dump",
        "json.loads",
        "re.fullmatch",
        "witness.update",
        "stream.write",
        "payload.encode",
        "':'.join",
        "time.sleep",
        "Path('witness.json').open",
        "Path('probe-context.json').open",
        "Path('probe-context.json').read_text",
    }
    environment = {
        "PROBE_CASE",
        "PROBE_SERIES",
        "PROBE_EXPECTED_SHA",
        "PROBE_SHA",
        "PROBE_REPOSITORY",
        "PROBE_EVENT",
        "PROBE_DEFAULT_BRANCH",
        "PROBE_REF",
        "PROBE_WORKFLOW_REF",
        "PROBE_RUN",
        "PROBE_ATTEMPT",
        "GITHUB_OUTPUT",
        "PREPARATION_ARTIFACT_ID",
        "PREPARATION_ARTIFACT_DIGEST",
        "PREPARATION_MARKER",
        "CALLER_PREREQUISITE_RESULT",
    }
    for node in ast.walk(tree):
        assert not isinstance(
            node,
            (
                ast.ImportFrom,
                ast.While,
                ast.Try,
                ast.FunctionDef,
                ast.AsyncFunctionDef,
                ast.ClassDef,
            ),
        ), "foreign control flow"
        if isinstance(node, ast.Import):
            assert all(item.name in {"json", "os", "re", "time"} for item in node.names)
        if isinstance(node, ast.ImportFrom):
            raise AssertionError("foreign import")
        if isinstance(node, ast.Call):
            target = ast.unparse(node.func)
            assert target in calls, "foreign operation: " + target
            if target == "Path":
                assert len(node.args) == 1 and isinstance(node.args[0], ast.Constant)
                assert node.args[0].value in {"witness.json", "probe-context.json"}
            if target == "open":
                assert ast.unparse(node.args[0]) == "os.environ['GITHUB_OUTPUT']"
                assert ast.literal_eval(node.args[1]) == "a"
            if target == "time.sleep":
                assert [ast.literal_eval(arg) for arg in node.args] == [5]
            if target == "range":
                assert [ast.literal_eval(arg) for arg in node.args] == [48]
        if isinstance(node, ast.Subscript) and ast.unparse(node.value) == "os.environ":
            if isinstance(node.slice, ast.Constant):
                assert node.slice.value in environment, "foreign environment read"
            else:
                assert isinstance(node.slice, ast.Name) and node.slice.id == "key"


def contract(
    workflows: dict[str, Any],
    sources: dict[str, str] | None = None,
    canonical_source: str | None = None,
) -> None:
    sources = workflow_sources() if sources is None else sources
    canonical_source = (
        (DIRECTORY / CANONICAL_WORKFLOW).read_text()
        if canonical_source is None
        else canonical_source
    )
    uploader, tag, runner = reviewed_metadata(canonical_source, "check")
    caller = workflows[CALLER]
    assert set(caller) == {"name", "run-name", "on", "permissions", "concurrency", "jobs"}
    assert caller["name"] == "629 recovery cancellation provider probe"
    assert set(caller["on"]) == {"workflow_dispatch"}
    inputs = caller["on"]["workflow_dispatch"]["inputs"]
    assert set(inputs) == COMMON_INPUTS
    assert inputs["case"]["type"] == "choice"
    assert inputs["case"]["options"] == list("ABCDE")
    assert all(item["required"] == "true" for item in inputs.values())
    assert caller["concurrency"] == {
        "group": "boxferry-629-provider-probe-v1-${{ github.repository }}",
        "cancel-in-progress": "false",
    }
    assert set(caller["jobs"]) == {"prepare", "effect"}
    for role, filename, name in (
        ("prepare", PREPARE, "629 preparation call"),
        ("effect", EFFECT, "629 inert effect call"),
    ):
        job = caller["jobs"][role]
        expected = {"name", "if", "permissions", "uses", "with"}
        assert set(job) == expected | ({"needs"} if role == "effect" else set())
        assert job["name"] == name and job["if"] == FENCE
        assert job["uses"] == "./.github/workflows/" + filename
        assert job["permissions"] == {"contents": "read"}
        for key in COMMON_INPUTS:
            assert job["with"][key] == "${{ inputs." + key + " }}"
        assert set(job["with"]) == COMMON_INPUTS | (EXTRA_INPUTS if role == "effect" else set())
    effect = caller["jobs"]["effect"]
    assert effect["needs"] == "prepare"
    for key, value in {
        "caller_prerequisite_result": "${{ needs.prepare.result }}",
        "preparation_artifact_id": "${{ needs.prepare.outputs.artifact_id }}",
        "preparation_artifact_digest": "${{ needs.prepare.outputs.artifact_digest }}",
        "preparation_marker": "${{ needs.prepare.outputs.marker }}",
    }.items():
        assert effect["with"][key] == value
    for filename, workflow in workflows.items():
        assert workflow["permissions"] == {"contents": "read"}
        if filename == CALLER:
            continue
        assert set(workflow) == {"name", "on", "permissions", "jobs"}
        assert set(workflow["on"]) == {"workflow_call"}
        declaration = workflow["on"]["workflow_call"]
        role = "prepare" if filename == PREPARE else "effect"
        assert set(declaration) == {"inputs"} | ({"outputs"} if role == "prepare" else set())
        assert set(declaration["inputs"]) == COMMON_INPUTS | (
            EXTRA_INPUTS if role == "effect" else set()
        )
        assert all(
            item["type"] == "string" and item["required"] == "true"
            for item in declaration["inputs"].values()
        )
        assert set(workflow["jobs"]) == {role}
        job = workflow["jobs"][role]
        assert set(job) == {"name", "runs-on", "timeout-minutes", "permissions", "env", "steps"} | (
            {"outputs"} if role == "prepare" else set()
        )
        assert job["name"] == (
            "629 preparation witness" if role == "prepare" else "629 inert dependent witness"
        )
        assert reviewed_metadata(sources[filename], role) == (uploader, tag, runner)
        assert job["runs-on"] == runner and job["timeout-minutes"] == "6"
        assert job["permissions"] == {"contents": "read"}
        expected_env = {
            "PROBE_CASE": "${{ inputs.case }}",
            "PROBE_SERIES": "${{ inputs.series }}",
            "PROBE_EXPECTED_SHA": "${{ inputs.expected_source_sha }}",
            "PROBE_SHA": "${{ github.sha }}",
            "PROBE_REPOSITORY": "${{ github.repository }}",
            "PROBE_EVENT": "${{ github.event_name }}",
            "PROBE_DEFAULT_BRANCH": "${{ github.event.repository.default_branch }}",
            "PROBE_REF": "${{ github.ref }}",
            "PROBE_WORKFLOW_REF": "${{ github.workflow_ref }}",
            "PROBE_RUN": "${{ github.run_id }}",
            "PROBE_ATTEMPT": "${{ github.run_attempt }}",
        }
        if role == "effect":
            expected_env.update(
                {
                    "PREPARATION_ARTIFACT_ID": "${{ inputs.preparation_artifact_id }}",
                    "PREPARATION_ARTIFACT_DIGEST": "${{ inputs.preparation_artifact_digest }}",
                    "PREPARATION_MARKER": "${{ inputs.preparation_marker }}",
                    "CALLER_PREREQUISITE_RESULT": "${{ inputs.caller_prerequisite_result }}",
                }
            )
        assert job["env"] == expected_env, "foreign or rebound environment source"
        if role == "prepare":
            assert job["outputs"] == {
                "artifact_id": "${{ steps.upload.outputs.artifact-id }}",
                "artifact_digest": "${{ steps.upload.outputs.artifact-digest }}",
                "marker": "${{ steps.validate.outputs.marker }}",
            }
            assert set(declaration["outputs"]) == {"artifact_id", "artifact_digest", "marker"}
            for key, output in declaration["outputs"].items():
                assert set(output) == {"description", "value"}
                assert output["value"] == "${{ jobs.prepare.outputs." + key + " }}"
        steps = job["steps"]
        assert len(steps) == 4
        assert steps[0]["id"] == "validate"
        assert [step["name"] for step in steps] == (
            [
                "Validate closed preparation identity",
                "Fixed preparation hold",
                "Write completed preparation witness",
                "Upload inert preparation witness",
            ]
            if role == "prepare"
            else [
                "Validate closed dependent identity and write entry witness",
                "Upload inert dependent entry witness",
                "Fixed inert dependent hold",
                "Print inert completion marker",
            ]
        )
        assert [index for index, step in enumerate(steps) if "uses" in step] == (
            [3] if role == "prepare" else [1]
        )
        for step in steps:
            if "uses" in step:
                assert set(step) == {"name", "id", "uses", "with"}
                assert step["id"] == "upload" and step["uses"] == uploader
                assert step["with"] == {
                    "name": "${{ steps.validate.outputs.artifact_name }}",
                    "path": "witness.json",
                    "archive": "true",
                    "if-no-files-found": "error",
                    "retention-days": "7",
                }
            else:
                assert set(step) in ({"name", "shell", "run"}, {"name", "id", "shell", "run"})
                assert step["shell"] == "bash"
                # Check pathlib imports separately so no additional module is admitted.
                body = python_body(step)
                imports = [
                    node for node in ast.walk(ast.parse(body)) if isinstance(node, ast.ImportFrom)
                ]
                assert all(
                    node.module == "pathlib"
                    and len(node.names) == 1
                    and node.names[0].name == "Path"
                    for node in imports
                )
                inert_python({**step, "run": step["run"].replace("from pathlib import Path\n", "")})


class ProviderProbeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.workflows = load_workflows()
        self.base = {
            "PROBE_CASE": "C",
            "PROBE_SERIES": "a" * 32,
            "PROBE_EXPECTED_SHA": "b" * 40,
            "PROBE_SHA": "b" * 40,
            "PROBE_REPOSITORY": "Strukturpiloten/boxferry",
            "PROBE_EVENT": "workflow_dispatch",
            "PROBE_DEFAULT_BRANCH": "main",
            "PROBE_REF": "refs/heads/main",
            "PROBE_WORKFLOW_REF": (
                "Strukturpiloten/boxferry/.github/workflows/" + CALLER + "@refs/heads/main"
            ),
            "PROBE_RUN": "12345",
            "PROBE_ATTEMPT": "1",
            "CALLER_PREREQUISITE_RESULT": "success",
            "PREPARATION_ARTIFACT_ID": "67890",
            "PREPARATION_ARTIFACT_DIGEST": "c" * 64,
            "PREPARATION_MARKER": "boxferry.provider-probe.v1:" + "a" * 32 + ":12345:1:" + "b" * 40,
        }

    def execute(
        self, filename: str, changes: dict[str, str] | None = None
    ) -> tuple[subprocess.CompletedProcess[str], dict[str, Any] | None, str]:
        role = "prepare" if filename == PREPARE else "effect"
        steps = self.workflows[filename]["jobs"][role]["steps"]
        code = python_body(steps[0])
        if role == "prepare":
            code += "\n" + python_body(steps[2])
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "output"
            output.touch()
            result = subprocess.run(
                [sys.executable, "-c", code],
                cwd=temporary,
                capture_output=True,
                text=True,
                env={**self.base, **(changes or {}), "GITHUB_OUTPUT": str(output)},
                timeout=5,
            )
            witness = Path(temporary) / "witness.json"
            return (
                result,
                json.loads(witness.read_text()) if witness.exists() else None,
                output.read_text(),
            )

    def test_closed_inert_topology(self) -> None:
        contract(self.workflows)
        for name in self.workflows:
            source = (DIRECTORY / name).read_text()
            self.assertTrue(source.startswith("---\n"))

    def test_coherent_reviewed_pin_and_runner_updates_remain_supported(self) -> None:
        canonical = (DIRECTORY / CANONICAL_WORKFLOW).read_text()
        current_uploader, current_tag, current_runner = reviewed_metadata(canonical, "check")
        # Synthetic authored metadata, not a claimed upstream release or integrity record.
        expected = ("actions/upload-artifact@" + "e" * 40, "v98.7.6", "ubuntu-28.04")
        for upgrade_uploader, upgrade_runner in ((True, False), (False, True), (True, True)):
            with self.subTest(uploader=upgrade_uploader, runner=upgrade_runner):
                wanted = (
                    expected[0] if upgrade_uploader else current_uploader,
                    expected[1] if upgrade_uploader else current_tag,
                    expected[2] if upgrade_runner else current_runner,
                )

                def updated(source: str, expected_metadata: tuple[str, str, str]) -> str:
                    return source.replace(
                        current_uploader + " # " + current_tag,
                        expected_metadata[0] + " # " + expected_metadata[1],
                    ).replace("runs-on: " + current_runner, "runs-on: " + expected_metadata[2])

                canonical_updated = updated(canonical, wanted)
                sources = {
                    name: updated(source, wanted) for name, source in workflow_sources().items()
                }
                self.assertEqual(reviewed_metadata(canonical_updated, "check"), wanted)
                for name, role in ((PREPARE, "prepare"), (EFFECT, "effect")):
                    self.assertEqual(reviewed_metadata(sources[name], role), wanted)
                contract(parse_workflows(sources), sources, canonical_updated)
                # Coherent probes alone do not silently choose their own authority.
                with self.assertRaises(AssertionError):
                    contract(parse_workflows(sources), sources, canonical)

    def test_metadata_requires_exact_tag_sha_runner_and_consumer_coherence(self) -> None:
        canonical = (DIRECTORY / CANONICAL_WORKFLOW).read_text()
        uploader, tag, runner = reviewed_metadata(canonical, "check")
        for malformed in (
            canonical.replace(uploader, "actions/upload-artifact@main"),
            canonical.replace(uploader, "actions/upload-artifact@" + "A" * 40),
            canonical.replace(" # " + tag, ""),
            canonical.replace(" # " + tag, " # v7"),
            canonical.replace("runs-on: " + runner, "runs-on: ubuntu-latest"),
            canonical.replace(
                "runs-on: " + runner, "runs-on: " + runner + "\n    runs-on: ubuntu-28.04"
            ),
            canonical
            + "\n# duplicate/conflicting evidence\n    uses: "
            + uploader
            + " # v98.7.6\n",
        ):
            with (
                self.subTest(source=malformed),
                self.assertRaises((AssertionError, yaml.YAMLError)),
            ):
                reviewed_metadata(malformed, "check")
        for filename in (PREPARE, EFFECT):
            for old, new in (
                (uploader, "actions/upload-artifact@" + "e" * 40),
                (" # " + tag, " # v98.7.6"),
                ("runs-on: " + runner, "runs-on: ubuntu-28.04"),
            ):
                sources = workflow_sources()
                sources[filename] = sources[filename].replace(old, new)
                with self.subTest(filename=filename, old=old), self.assertRaises(AssertionError):
                    contract(parse_workflows(sources), sources, canonical)

    def test_rejects_effect_permission_identity_source_and_dependency_drift(self) -> None:
        mutations = [
            (CALLER, lambda w: w["permissions"].update(actions="write")),
            (CALLER, lambda w: w["on"].update(push={})),
            (CALLER, lambda w: w["concurrency"].update(group="${{ inputs.series }}")),
            (CALLER, lambda w: w["jobs"]["effect"].update(needs=[])),
            (CALLER, lambda w: w["jobs"]["effect"].update(**{"if": "${{ always() }}"})),
            (CALLER, lambda w: w["jobs"]["prepare"].update(uses="owner/repo/workflow@main")),
            (CALLER, lambda w: w["jobs"]["prepare"].update(secrets="inherit")),
            (EFFECT, lambda w: w["jobs"]["effect"]["steps"].append({"run": "curl example.org"})),
            (EFFECT, lambda w: w["jobs"]["effect"].update(environment="production")),
            (PREPARE, lambda w: w["jobs"]["prepare"]["steps"][3]["with"].update(archive="false")),
            (PREPARE, lambda w: w["jobs"]["prepare"].update(**{"timeout-minutes": "60"})),
            (EFFECT, lambda w: w["jobs"]["effect"]["steps"][0].update(**{"if": "always()"})),
        ]
        mutations.extend(
            [
                (
                    PREPARE,
                    lambda w: w["jobs"]["prepare"]["env"].update(
                        PROBE_SHA="${{ inputs.expected_source_sha }}"
                    ),
                ),
                (
                    PREPARE,
                    lambda w: w["jobs"]["prepare"]["outputs"].update(
                        artifact_id="${{ steps.validate.outputs.marker }}"
                    ),
                ),
                (EFFECT, lambda w: w["jobs"]["effect"]["steps"][2].update(name="Ambiguous hold")),
                (
                    CALLER,
                    lambda w: w["jobs"]["effect"]["with"].update(
                        caller_prerequisite_result="success"
                    ),
                ),
            ]
        )
        for name, mutate in mutations:
            with self.subTest(name=name, mutate=mutate):
                workflows = copy.deepcopy(self.workflows)
                mutate(workflows[name])
                with self.assertRaises(AssertionError):
                    contract(workflows)
        for added in (
            "os.system('curl example.org')",
            "print(os.environ['SECRET'])",
            "Path('/tmp/effect').open('w')",
            "time.sleep(600)",
        ):
            with self.subTest(added=added), self.assertRaises(AssertionError):
                inert_python({"run": "set -euo pipefail\npython3 - <<'PY'\n" + added + "\nPY\n"})

    def test_positive_exact_witnesses_and_namespace(self) -> None:
        for case in "ABCDE":
            for filename, role, phase in (
                (PREPARE, "prepare", "completed"),
                (EFFECT, "effect", "entry"),
            ):
                with self.subTest(case=case, filename=filename):
                    result, witness, output = self.execute(filename, {"PROBE_CASE": case})
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIsNotNone(witness)
                    assert witness is not None
                    expected: dict[str, Any] = {
                        "schema": "boxferry.provider-probe.v1",
                        "repository": "Strukturpiloten/boxferry",
                        "workflow_path": ".github/workflows/" + CALLER,
                        "source_sha": "b" * 40,
                        "series": "a" * 32,
                        "case": case,
                        "run_id": "12345",
                        "run_attempt": 1,
                        "role": role,
                        "marker_phase": phase,
                    }
                    if role == "effect":
                        expected.update(
                            preparation_artifact_id="67890",
                            preparation_artifact_digest="c" * 64,
                            preparation_marker=self.base["PREPARATION_MARKER"],
                            caller_prerequisite_result="success",
                        )
                    self.assertEqual(witness, expected)
                    self.assertLessEqual(len(json.dumps(witness).encode()), 4096)
                    suffix = "prepare" if role == "prepare" else "effect-entry"
                    self.assertEqual(
                        output,
                        "marker="
                        + self.base["PREPARATION_MARKER"]
                        + "\n"
                        + "artifact_name=boxferry-629-provider-probe-v1-"
                        + "a" * 32
                        + "-12345-attempt1-"
                        + suffix
                        + "\n",
                    )
                    self.assertTrue(
                        all(not line.startswith("::") for line in result.stdout.splitlines())
                    )

    def test_invalid_inputs_and_provider_identities_fail_before_witness(self) -> None:
        invalid = {
            "PROBE_CASE": ["", "F", "$(touch injected)", "A\n::warning::bad"],
            "PROBE_SERIES": ["", "A" * 32, "a" * 31, "../series"],
            "PROBE_EXPECTED_SHA": ["main", "B" * 40, "b" * 39, "c" * 40],
            "PROBE_SHA": ["c" * 40],
            "PROBE_RUN": ["0", "-1", "1/2", "1" * 21],
            "PROBE_ATTEMPT": ["2", "01"],
            "PROBE_REPOSITORY": ["attacker/boxferry"],
            "PROBE_EVENT": ["workflow_call", "pull_request", "push"],
            "PROBE_DEFAULT_BRANCH": ["other"],
            "PROBE_REF": ["refs/heads/other"],
            "PROBE_WORKFLOW_REF": [self.base["PROBE_WORKFLOW_REF"].replace(CALLER, PREPARE)],
        }
        for key, values in invalid.items():
            for value in values:
                for filename in (PREPARE, EFFECT):
                    with self.subTest(key=key, value=value, filename=filename):
                        result, witness, output = self.execute(filename, {key: value})
                        self.assertNotEqual(result.returncode, 0)
                        self.assertIsNone(witness)
                        self.assertEqual(output, "")
                        self.assertEqual(result.stdout, "")
        for key, values in {
            "CALLER_PREREQUISITE_RESULT": ["", "cancelled", "skipped", "failure"],
            "PREPARATION_ARTIFACT_ID": ["", "0", "../1"],
            "PREPARATION_ARTIFACT_DIGEST": ["", "c" * 63, "C" * 64],
            "PREPARATION_MARKER": [
                "",
                self.base["PREPARATION_MARKER"].replace(":12345:", ":12346:"),
            ],
        }.items():
            for value in values:
                with self.subTest(key=key, value=value):
                    result, witness, output = self.execute(EFFECT, {key: value})
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIsNone(witness)
                    self.assertEqual(output, "")
                    self.assertEqual(result.stdout, "")

    def test_fixed_hold_cases_are_bounded_without_real_wait(self) -> None:
        for filename, role, index, held in (
            (PREPARE, "prepare", 1, {"A", "D"}),
            (EFFECT, "effect", 2, {"E"}),
        ):
            step = self.workflows[filename]["jobs"][role]["steps"][index]
            self.assertEqual(
                step["name"],
                "Fixed preparation hold" if role == "prepare" else "Fixed inert dependent hold",
            )
            for case in "ABCDE":
                with (
                    self.subTest(role=role, case=case),
                    patch.dict(os.environ, PROBE_CASE=case),
                    patch.object(time, "sleep") as sleep,
                    contextlib.redirect_stdout(io.StringIO()) as output,
                ):
                    exec(compile(python_body(step), "<inert hold>", "exec"), {})
                    self.assertEqual(sleep.call_count, 48 if case in held else 0)
                    self.assertTrue(all(call.args == (5,) for call in sleep.call_args_list))
                    self.assertEqual(len(output.getvalue().splitlines()), sleep.call_count)


if __name__ == "__main__":
    unittest.main()
