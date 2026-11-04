from __future__ import annotations

import contextlib
import hashlib
import io
import json
import os
import subprocess
import tempfile
import textwrap
import unittest
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

from boxferry_ci.admission import (
    MANIFEST_PATH,
    AdmissionContext,
    AdmissionError,
    main,
    parse_manifest,
    read_git,
    resolve_admission,
    validate_context,
)

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/shared-dependency-wait.yml"
OLD_AUTOMATION = "a" * 40
PROPOSED_AUTOMATION = "b" * 40
POLICY = "c" * 40
CANDIDATE = "d" * 40
POLICY_PATH = "scripts/validation-policy.json"


def manifest(**changes: object) -> bytes:
    value = {
        "schema": 1,
        "repository": "Strukturpiloten/boxferry",
        "revision": OLD_AUTOMATION,
        "policy": POLICY_PATH,
        "interface": 1,
    }
    value.update(changes)
    return json.dumps(value).encode()


class GitFixture:
    """Immutable-object fixture; deliberately does not read working-tree files."""

    def __init__(self, implementation: Path, policy: Path) -> None:
        self.heads = {implementation: PROPOSED_AUTOMATION, policy: POLICY}
        self.files: dict[Path, dict[str, tuple[bytes, bytes]]] = {
            policy: {
                MANIFEST_PATH: (b"100644", manifest()),
                POLICY_PATH: (b"100644", b'{"schema":2}'),
            }
        }
        self.calls: list[tuple[Path, tuple[str, ...]]] = []

    @staticmethod
    def object_sha(content: bytes) -> str:
        return hashlib.sha1(
            b"blob " + str(len(content)).encode() + b"\0" + content,
            usedforsecurity=False,
        ).hexdigest()

    def __call__(self, root: Path, args: tuple[str, ...], limit: int) -> bytes:
        self.calls.append((root, args))
        if args == ("rev-parse", "HEAD"):
            output = self.heads[root].encode() + b"\n"
        elif args[:2] == ("ls-tree", "-z"):
            if args[2] != self.heads[root] or args[3] != "--":
                raise AssertionError("fixture was not read at its exact held revision")
            item = self.files.get(root, {}).get(args[4])
            if item is None:
                output = b""
            else:
                mode, content = item
                output = (
                    mode
                    + b" blob "
                    + self.object_sha(content).encode()
                    + b"\t"
                    + args[4].encode()
                    + b"\0"
                )
        elif args[0] == "cat-file":
            matches = [
                data
                for _mode, data in self.files[root].values()
                if self.object_sha(data) == args[2]
            ]
            if len(matches) != 1:
                raise AssertionError("fixture object missing or ambiguous")
            output = str(len(matches[0])).encode() + b"\n" if args[1] == "-s" else matches[0]
        else:
            raise AssertionError(f"unexpected Git operation: {args}")
        if len(output) > limit:
            raise AdmissionError("fixture output exceeds size budget")
        return output


class Admission(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.implementation = self.root / "proposed"
        self.policy = self.root / "trusted"
        self.implementation.mkdir()
        self.policy.mkdir()
        self.git = GitFixture(self.implementation, self.policy)
        self.context = AdmissionContext(
            "pull_request",
            "Strukturpiloten/compose-lens",
            CANDIDATE,
            PROPOSED_AUTOMATION,
            POLICY,
            CANDIDATE,
            pr_base_sha=POLICY,
        )

    def resolve(self, context: AdmissionContext | None = None) -> dict[str, str]:
        return resolve_admission(
            context or self.context, self.implementation, self.policy, reader=self.git
        ).outputs()

    def test_upgrade_b_uses_policy_a_from_exact_base_object(self) -> None:
        # Even a modified worktree manifest proposing B cannot replace the held A object.
        (self.policy / ".github").mkdir()
        (self.policy / MANIFEST_PATH).write_bytes(manifest(revision=PROPOSED_AUTOMATION))
        outputs = self.resolve()
        self.assertEqual(outputs["policy_automation_sha"], OLD_AUTOMATION)
        self.assertEqual(outputs["proposed_automation_sha"], PROPOSED_AUTOMATION)
        self.assertEqual(outputs["policy_sha"], POLICY)
        self.assertEqual(outputs["candidate_sha"], CANDIDATE)
        self.assertEqual(outputs["required"], "true")
        self.assertIn((self.policy, ("ls-tree", "-z", POLICY, "--", MANIFEST_PATH)), self.git.calls)

    def test_merge_queue_uses_queue_base_not_pr_fields(self) -> None:
        context = replace(
            self.context, event="merge_group", queue_base_sha=POLICY, pr_base_sha="e" * 40
        )
        self.assertEqual(self.resolve(context)["policy_automation_sha"], OLD_AUTOMATION)
        with self.assertRaisesRegex(AdmissionError, "event revision"):
            self.resolve(replace(context, policy_sha="e" * 40))

    def test_full_events_are_not_applicable_without_reading_manifest(self) -> None:
        for event in ("push", "workflow_dispatch", "workflow_call"):
            with self.subTest(event=event):
                self.git.calls.clear()
                outputs = self.resolve(replace(self.context, event=event, policy_sha=CANDIDATE))
                self.assertEqual(outputs["required"], "false")
                self.assertEqual(outputs["policy_automation_sha"], "")
                self.assertEqual(self.git.calls, [(self.implementation, ("rev-parse", "HEAD"))])

    def test_event_candidate_and_policy_mismatches_fail_closed(self) -> None:
        cases = (
            replace(self.context, event="pull_request_target"),
            replace(self.context, event="workflow_run"),
            replace(self.context, event="schedule"),
            replace(self.context, repository="someone/other"),
            replace(self.context, candidate_sha="e" * 40),
            replace(self.context, policy_sha="e" * 40),
            replace(self.context, pr_base_sha=""),
            replace(self.context, event="merge_group", queue_base_sha=""),
            replace(self.context, event="push"),
        )
        for context in cases:
            with self.subTest(context=context), self.assertRaises(AdmissionError):
                validate_context(context)

    def test_nonimmutable_or_wrong_checkout_sources_are_rejected(self) -> None:
        for revision in ("main", "automation-v1", "a" * 39, "A" * 40, "g" * 40):
            with self.subTest(revision=revision), self.assertRaises(AdmissionError):
                self.resolve(replace(self.context, automation_sha=revision))
        self.git.heads[self.implementation] = "e" * 40
        with self.assertRaisesRegex(AdmissionError, "proposed automation checkout"):
            self.resolve()
        self.git.heads[self.implementation] = PROPOSED_AUTOMATION
        self.git.heads[self.policy] = "e" * 40
        with self.assertRaisesRegex(AdmissionError, "trusted policy checkout"):
            self.resolve()

    def test_missing_seed_links_or_missing_referenced_policy_fail(self) -> None:
        for path in (MANIFEST_PATH, POLICY_PATH):
            original = self.git.files[self.policy].pop(path)
            with (
                self.subTest(path=path),
                self.assertRaisesRegex(AdmissionError, "prerequisite before the waiter"),
            ):
                self.resolve()
            self.git.files[self.policy][path] = original
        for path in (MANIFEST_PATH, POLICY_PATH):
            original = self.git.files[self.policy][path]
            self.git.files[self.policy][path] = (b"120000", original[1])
            with self.subTest(path=path), self.assertRaisesRegex(AdmissionError, "not a link"):
                self.resolve()
            self.git.files[self.policy][path] = original

    def test_manifest_schema_source_and_pin_are_strict(self) -> None:
        cases = (
            {"schema": True},
            {"schema": 2},
            {"interface": True},
            {"interface": 2},
            {"repository": "Strukturpiloten/other"},
            {"revision": "main"},
            {"revision": "A" * 40},
            {"revision": None},
            {"unexpected": "ignored"},
        )
        for changes in cases:
            with self.subTest(changes=changes), self.assertRaises(AdmissionError):
                parse_manifest(manifest(**changes))
        for raw in (b"[]", b"null", b"\xff", b"{", b" " * 8193):
            with self.subTest(raw=raw[:20]), self.assertRaises(AdmissionError):
                parse_manifest(raw)
        repeated = manifest().replace(b'"schema": 1', b'"schema": 1, "schema": 1')
        with self.assertRaisesRegex(AdmissionError, "repeats key"):
            parse_manifest(repeated)

    def test_policy_paths_reject_traversal_controls_and_nonjson_inputs(self) -> None:
        for path in (
            "",
            "/tmp/policy.json",
            "../policy.json",
            "scripts/../policy.json",
            "scripts/./policy.json",
            "scripts//policy.json",
            "scripts\\policy.json",
            "scripts/policy.json\noutput=evil",
            "scripts/policy\x7f.json",
            ".git/config.json",
            "scripts/policy.sh",
            "x" * 513 + ".json",
            None,
        ):
            with self.subTest(path=path), self.assertRaises(AdmissionError):
                parse_manifest(manifest(policy=path))

    def test_blob_size_is_checked_before_reading_contents(self) -> None:
        self.git.files[self.policy][MANIFEST_PATH] = (b"100644", b" " * 8193)
        with self.assertRaisesRegex(AdmissionError, "size budget"):
            self.resolve()
        self.assertFalse(any(args[:2] == ("cat-file", "blob") for _, args in self.git.calls))

    def test_cli_outputs_name_the_base_policy_and_keep_failure_actionable(self) -> None:
        output, summary = self.root / "output", self.root / "summary"
        argv = [
            "--repository",
            self.context.repository,
            "--event-name",
            self.context.event,
            "--candidate-sha",
            CANDIDATE,
            "--automation-sha",
            PROPOSED_AUTOMATION,
            "--policy-sha",
            POLICY,
            "--tested-sha",
            CANDIDATE,
            "--pr-base-sha",
            POLICY,
            "--implementation-root",
            str(self.implementation),
            "--policy-root",
            str(self.policy),
            "--github-output",
            str(output),
            "--summary",
            str(summary),
        ]
        with patch("boxferry_ci.admission.resolve_admission") as resolver:
            resolver.return_value = resolve_admission(
                self.context, self.implementation, self.policy, reader=self.git
            )
            with contextlib.redirect_stdout(io.StringIO()) as stdout:
                self.assertEqual(main(argv), 0)
            self.assertEqual(json.loads(stdout.getvalue())["policy_automation_sha"], OLD_AUTOMATION)
        self.assertIn("policy_automation_sha=" + OLD_AUTOMATION, output.read_text())
        self.assertIn(
            "Reader policy: `Strukturpiloten/boxferry@" + OLD_AUTOMATION, summary.read_text()
        )
        self.assertIn("does not replace admission policy before merge", summary.read_text())
        with patch(
            "boxferry_ci.admission.resolve_admission", side_effect=AdmissionError("missing seed")
        ):
            with contextlib.redirect_stderr(io.StringIO()) as stderr:
                self.assertEqual(main(argv), 2)
            self.assertIn("Dependency admission setup failed: missing seed", stderr.getvalue())

    def test_git_reader_is_read_only_and_disables_inherited_git_overrides(self) -> None:
        completed = subprocess.CompletedProcess(["git"], 0, stdout=b"ok", stderr=b"")
        with patch.dict(os.environ, {"GIT_CONFIG_COUNT": "1", "GIT_DIR": "/untrusted"}):
            with patch("boxferry_ci.admission.subprocess.run", return_value=completed) as run:
                self.assertEqual(read_git(self.policy, ("rev-parse", "HEAD"), 128), b"ok")
        arguments = run.call_args.args[0]
        self.assertIn("--no-replace-objects", arguments)
        self.assertIn("core.hooksPath=/dev/null", arguments)
        environment = run.call_args.kwargs["env"]
        self.assertNotIn("GIT_DIR", environment)
        self.assertNotIn("GIT_CONFIG_COUNT", environment)
        self.assertEqual(environment["GIT_CONFIG_GLOBAL"], os.devnull)
        self.assertEqual(run.call_args.kwargs["timeout"], 15)
        with patch("boxferry_ci.admission.subprocess.run", return_value=completed):
            with self.assertRaisesRegex(AdmissionError, "size budget"):
                read_git(self.policy, ("rev-parse", "HEAD"), 1)

    def test_workflow_reader_uses_selected_a_with_only_read_permissions(self) -> None:
        workflow = WORKFLOW.read_text()
        self.assertTrue(workflow.startswith("---\n"))
        self.assertIn("  contents: read\n  actions: read\n  statuses: read\n", workflow)
        self.assertNotIn(": write", workflow)
        self.assertNotIn("secrets:", workflow)
        selected = workflow.split("name: Check out the trusted base admission implementation A", 1)[
            1
        ]
        self.assertIn("ref: ${{ steps.policy.outputs.policy_automation_sha }}", selected)
        self.assertIn("path: _automation", selected)
        self.assertIn('--implementation-revision "${ADMISSION_SHA}"', selected)
        self.assertNotIn('--implementation-revision "${AUTOMATION_SHA}"', selected)
        self.assertEqual(workflow.count("persist-credentials: false"), 3)
        self.assertIn("admission_status=not-applicable", workflow)

    def test_workflow_rejects_floating_inputs_before_checkout(self) -> None:
        workflow = WORKFLOW.read_text()
        first_run = workflow.split("        run: |\n", 1)[1].split("      - name:", 1)[0]
        script = textwrap.dedent(first_run)
        environment = dict(
            os.environ,
            CANDIDATE_SHA=CANDIDATE,
            AUTOMATION_SHA=PROPOSED_AUTOMATION,
            POLICY_SHA=POLICY,
        )
        for key in ("CANDIDATE_SHA", "AUTOMATION_SHA", "POLICY_SHA"):
            for revision in ("main", "automation-v1", "a" * 39, "A" * 40, "g" * 40):
                with self.subTest(key=key, revision=revision):
                    result = subprocess.run(
                        ["bash", "-c", script],
                        env=dict(environment, **{key: revision}),
                        capture_output=True,
                        text=True,
                        check=False,
                        timeout=5,
                    )
                    self.assertNotEqual(result.returncode, 0)
        result = subprocess.run(
            ["bash", "-c", script],
            env=environment,
            capture_output=True,
            check=False,
            timeout=5,
        )
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
