"""Exact-base receipts and immutable reader capability negotiation."""

from __future__ import annotations

import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
import zipfile
from datetime import UTC, datetime
from pathlib import Path
from unittest.mock import Mock, patch

from boxferry_ci import eligibility as subject

ROOT = Path(__file__).resolve().parents[2]
BASE, HEAD, AUTHORITY = (character * 40 for character in "abc")


class ReaderContractTests(unittest.TestCase):
    def test_legacy_source_and_supported_static_markers(self) -> None:
        self.assertEqual(subject.reader_contract(b"def wait(): pass\n"), 1)
        for declaration in (
            b"ADMISSION_READER_CONTRACT = 2\n",
            b"ADMISSION_READER_CONTRACT: int = 2\n",
        ):
            with self.subTest(declaration=declaration):
                self.assertEqual(subject.reader_contract(declaration), 2)

    def test_unknown_dynamic_nested_and_duplicate_markers_fail_closed(self) -> None:
        for declaration in (
            b"ADMISSION_READER_CONTRACT = 1\n",
            b"ADMISSION_READER_CONTRACT = 3\n",
            b"ADMISSION_READER_CONTRACT = True\n",
            b"ADMISSION_READER_CONTRACT = int('2')\n",
            b"ADMISSION_READER_CONTRACT = 2\nADMISSION_READER_CONTRACT = 2\n",
            b"if True:\n    ADMISSION_READER_CONTRACT = 2\n",
            b"ADMISSION_READER_CONTRACT = (\n",
        ):
            with self.subTest(declaration=declaration), self.assertRaises(subject.EligibilityError):
                subject.reader_contract(declaration)
        with self.assertRaises(subject.EligibilityError):
            subject.reader_contract(b"#" * (subject.MAX_FILE_BYTES + 1))

    def test_receipt_requires_exact_tested_base(self) -> None:
        api = Mock(spec=subject.GitHub)
        api.repository = "Strukturpiloten/boxferry"
        api.items.return_value = [
            {
                "context": subject.STATUS_CONTEXT,
                "state": "success",
                "creator": {"login": "github-actions[bot]"},
                "target_url": "https://github.com/Strukturpiloten/boxferry/actions/runs/10",
            }
        ]
        report = {"status": "pass", "candidate_sha": HEAD, "base_sha": BASE}
        payload = {"reports": {HEAD: report}}
        with (
            patch.object(subject, "trusted_run", return_value=True),
            patch.object(subject, "run_artifact", return_value={}),
            patch.object(subject, "validate_state", return_value=payload),
        ):
            self.assertEqual(
                subject.candidate_receipt(
                    api, HEAD, AUTHORITY, "main", datetime.now(UTC), expected_base_sha=BASE
                ),
                report,
            )
            for base in (None, "d" * 40):
                with self.subTest(base=base):
                    if base is None:
                        report.pop("base_sha", None)
                    else:
                        report["base_sha"] = base
                    self.assertIsNone(
                        subject.candidate_receipt(
                            api, HEAD, AUTHORITY, "main", datetime.now(UTC), expected_base_sha=BASE
                        )
                    )

    def test_preinstall_workflow_probe_reads_held_reader_in_clean_foreign_process(self) -> None:
        workflow = (ROOT / ".github/workflows/shared-dependency-wait.yml").read_text()
        start = workflow.index(
            "          set -euo pipefail",
            workflow.index("- name: Detect the immutable admission reader capability"),
        )
        end = workflow.index("      - name:", start)
        script = "\n".join(line[10:] for line in workflow[start:end].splitlines())
        current = ROOT / "automation/src/boxferry_ci"
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            isolated = root / "python"
            subprocess.run(
                ["/usr/bin/python3", "-m", "venv", "--without-pip", str(isolated)],
                check=True,
                capture_output=True,
                timeout=20,
            )
            version = subprocess.run(
                [str(isolated / "bin/python3"), "--version"],
                check=True,
                capture_output=True,
                text=True,
                timeout=20,
            )
            print(
                f"reader-contract pre-install interpreter: {version.stdout.strip()}",
                file=sys.stderr,
            )
            workspace = root / "workspace"
            workspace.mkdir()
            foreign = root / "foreign-cwd"
            foreign.mkdir()
            proposed = workspace / "_proposed_automation/automation/src/boxferry_ci"
            shutil.copytree(current, proposed, ignore=shutil.ignore_patterns("__pycache__"))
            for path in (foreign, root / "inherited"):
                package = path / "boxferry_ci"
                package.mkdir(parents=True)
                (package / "__init__.py").write_text(
                    "raise AssertionError('shadow package loaded')\n"
                )
            (root / "inherited/sitecustomize.py").write_text(
                "raise SystemExit('inherited Python path reached startup')\n"
            )
            held = foreign / "_automation/automation/src/boxferry_ci/eligibility.py"
            held.parent.mkdir(parents=True)
            output = root / "github-output"
            environment = {
                **os.environ,
                "PATH": str(isolated / "bin") + os.pathsep + os.environ["PATH"],
                "PYTHONPATH": str(root / "inherited"),
                "PYTHONDONTWRITEBYTECODE": "1",
                "GITHUB_WORKSPACE": str(workspace),
                "GITHUB_OUTPUT": str(output),
            }
            for name, content, contract in (
                ("legacy", b"raise AssertionError('held reader must not execute')\n", "1"),
                ("current", (current / "eligibility.py").read_bytes(), "2"),
                ("malformed", b"ADMISSION_READER_CONTRACT = (\n", None),
                ("unknown", b"ADMISSION_READER_CONTRACT = 3\n", None),
                (
                    "duplicate",
                    b"ADMISSION_READER_CONTRACT = 2\nADMISSION_READER_CONTRACT = 2\n",
                    None,
                ),
            ):
                with self.subTest(reader=name):
                    held.write_bytes(content)
                    output.unlink(missing_ok=True)
                    result = subprocess.run(
                        ["bash", "-c", script],
                        cwd=foreign,
                        env=environment,
                        capture_output=True,
                        text=True,
                        check=False,
                        timeout=20,
                    )
                    self.assertEqual(result.returncode, 0 if contract else 2, result.stderr)
                    self.assertEqual(result.stdout, "")
                    if contract:
                        self.assertEqual(output.read_text(), f"contract={contract}\n")
                        self.assertEqual(result.stderr, "")
                    else:
                        self.assertFalse(output.exists())

    def test_waiter_preserves_legacy_argv_and_passes_trusted_base_only_to_contract_two(
        self,
    ) -> None:
        workflow = (ROOT / ".github/workflows/shared-dependency-wait.yml").read_text()
        start = workflow.index(
            "          set -euo pipefail",
            workflow.index("- name: Read the exact candidate receipt"),
        )
        end = workflow.index("      - name:", start)
        script = "\n".join(line[10:] for line in workflow[start:end].splitlines())
        with tempfile.TemporaryDirectory() as temporary:
            uv = Path(temporary) / "uv"
            uv.write_text('#!/bin/bash\nprintf "%s\\n" "$@" > "$CAPTURE"\n')
            uv.chmod(0o755)
            capture = Path(temporary) / "argv"
            environment = {
                **os.environ,
                "PATH": f"{temporary}:{os.environ['PATH']}",
                "CAPTURE": str(capture),
                "CANDIDATE_SHA": HEAD,
                "ADMISSION_SHA": AUTHORITY,
                "EXPECTED_BASE_SHA": BASE,
                "GITHUB_REPOSITORY": "Strukturpiloten/boxferry",
                "GITHUB_STEP_SUMMARY": "summary",
            }
            expected = [
                "run",
                "--locked",
                "--project",
                "_automation/automation",
                "python",
                "-m",
                "boxferry_ci.eligibility",
                "wait",
                "--repository",
                "Strukturpiloten/boxferry",
                "--candidate-sha",
                HEAD,
                "--implementation-revision",
                AUTHORITY,
            ]
            for contract in ("1", "2"):
                with self.subTest(contract=contract):
                    subprocess.run(
                        ["bash", "-c", script],
                        check=True,
                        env={**environment, "READER_CONTRACT": contract},
                    )
                    extra = [] if contract == "1" else ["--expected-base-sha", BASE]
                    self.assertEqual(
                        capture.read_text().splitlines(),
                        expected + extra + ["--summary", "summary"],
                    )
            capture.unlink()
            result = subprocess.run(
                ["bash", "-c", script],
                check=False,
                env={**environment, "READER_CONTRACT": "3"},
                capture_output=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(capture.exists())

    def test_fresh_module_cli_dispatch_reaches_actionable_missing_label_path(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "sitecustomize.py").write_text(
                "from datetime import UTC, datetime\n"
                "from boxferry_ci import eligibility as core, recovery\n"
                "class API(core.GitHub):\n"
                "    def get(self, path):\n"
                "        raise core.MissingResource('missing label')\n"
                "def exercise(api, **options):\n"
                "    try:\n"
                "        recovery.label_unused(api, 17, datetime.now(UTC), source_head='a'*40, "
                "since='2026-11-01T00:00:00Z')\n"
                "    except recovery.RecoveryRefused as error:\n"
                "        raise core.EligibilityError(str(error)) from error\n"
                "core.GitHub = API\n"
                "core.service = exercise\n"
            )
            event = root / "event.json"
            event.write_text("{}")
            result = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "boxferry_ci.eligibility",
                    "service",
                    "--repository",
                    "Strukturpiloten/compose-lens",
                    "--implementation-revision",
                    AUTHORITY,
                    "--event-name",
                    "schedule",
                    "--event-path",
                    str(event),
                    "--run-id",
                    "100",
                    "--run-attempt",
                    "1",
                    "--output",
                    str(root / "state"),
                ],
                env={
                    **os.environ,
                    "GH_TOKEN": "test-token",
                    "PYTHONDONTWRITEBYTECODE": "1",
                    "PYTHONPATH": os.pathsep.join((temporary, str(ROOT / "automation/src"))),
                },
                capture_output=True,
                text=True,
                check=False,
                timeout=20,
            )
            self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
            self.assertIn("provision the fixed repository rebase label", result.stdout)
            self.assertNotIn("Traceback", result.stderr)

    def test_real_reader_receipts_need_no_service_only_queue_module(self) -> None:
        # Exercise capability detection, artifact decoding/provenance and exact
        # base receipts through the real CLI while the service helper is absent.
        repository = "Strukturpiloten/boxferry"
        fixture = {
            "get": {
                "": {"default_branch": "main"},
                "actions/runs/10": {
                    "id": 10,
                    "run_attempt": 1,
                    "repository": {"full_name": repository},
                    "event": "schedule",
                    "path": subject.WORKFLOW_PATH,
                    "head_branch": "main",
                    "head_sha": BASE,
                    "status": "completed",
                    "conclusion": "success",
                    "referenced_workflows": [
                        {
                            "path": f"{subject.SHARED_WORKFLOW_PATH}@{AUTHORITY}",
                            "sha": AUTHORITY,
                        }
                    ],
                },
                f"compare/{BASE}...main": {"status": "ahead"},
                "actions/runs/10/artifacts?per_page=100": {
                    "artifacts": [{"id": 1, "name": subject.ARTIFACT_NAME, "size_in_bytes": 2048}],
                },
            },
            "statuses": [
                {
                    "context": subject.STATUS_CONTEXT,
                    "state": "success",
                    "creator": {"login": "github-actions[bot]"},
                    "target_url": f"https://github.com/{repository}/actions/runs/10",
                }
            ],
        }
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fixture_path = root / "fixture.json"
            fixture_path.write_text(json.dumps(fixture))
            archive_path = root / "state.zip"
            (root / "sitecustomize.py").write_text(
                "import json, os, sys\n"
                "from pathlib import Path\n"
                "sys.modules['boxferry_ci.queue_tree'] = None\n"
                "from boxferry_ci import eligibility as core\n"
                "fixture = json.loads(Path(os.environ['READER_FIXTURE']).read_text())\n"
                "class API(core.GitHub):\n"
                " def get(self, path):\n"
                "  return fixture['get'][path]\n"
                " def items(self, path):\n"
                f"  assert path == 'commits/{HEAD}/statuses?per_page=100'\n"
                "  return fixture['statuses']\n"
                " def artifact(self, identifier):\n"
                "  assert identifier == 1\n"
                "  return Path(os.environ['READER_ARCHIVE']).read_bytes()\n"
                " def request(self, *args, **kwargs):\n"
                "  raise AssertionError('unexpected network or effect')\n"
                "core.GitHub = API\n"
            )
            environment = {
                **os.environ,
                "GH_TOKEN": "test-token",
                "PYTHONDONTWRITEBYTECODE": "1",
                "PYTHONPATH": os.pathsep.join((temporary, str(ROOT / "automation/src"))),
                "READER_FIXTURE": str(fixture_path),
                "READER_ARCHIVE": str(archive_path),
            }
            capability = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "boxferry_ci.eligibility",
                    "reader-contract",
                    "--module-path",
                    str(ROOT / "automation/src/boxferry_ci/eligibility.py"),
                ],
                env=environment,
                capture_output=True,
                text=True,
                check=False,
                timeout=20,
            )
            self.assertEqual(capability.returncode, 0, capability.stdout + capability.stderr)
            self.assertEqual(capability.stdout.strip(), "2")
            for receipt_base, exit_code, status in ((BASE, 0, "pass"), ("d" * 40, 1, "waiting")):
                payload = {
                    "schema": 1,
                    "repository": repository,
                    "run_id": 10,
                    "run_attempt": 1,
                    "workflow_path": subject.WORKFLOW_PATH,
                    "implementation_revision": AUTHORITY,
                    "observed_at": "2026-11-01T00:00:00Z",
                    "dependency_state": {"schema_version": 1, "first_seen": {}},
                    "reruns": {},
                    "reports": {
                        HEAD: {
                            "status": "pass",
                            "candidate_sha": HEAD,
                            "base_sha": receipt_base,
                        }
                    },
                }
                raw = io.BytesIO()
                with zipfile.ZipFile(raw, "w") as archive:
                    archive.writestr(subject.STATE_FILE, json.dumps(payload))
                archive_path.write_bytes(raw.getvalue())
                result = subprocess.run(
                    [
                        sys.executable,
                        "-m",
                        "boxferry_ci.eligibility",
                        "wait",
                        "--repository",
                        repository,
                        "--implementation-revision",
                        AUTHORITY,
                        "--candidate-sha",
                        HEAD,
                        "--expected-base-sha",
                        BASE,
                        "--timeout",
                        "0",
                    ],
                    env=environment,
                    capture_output=True,
                    text=True,
                    check=False,
                    timeout=20,
                )
                with self.subTest(receipt_base=receipt_base):
                    self.assertEqual(result.returncode, exit_code, result.stdout + result.stderr)
                    self.assertIn(status, result.stdout)
                    self.assertNotIn("Traceback", result.stderr)

    def test_leftover_artifact_from_earlier_service_attempt_is_not_a_current_receipt(self) -> None:
        api = Mock(spec=subject.GitHub)
        api.repository = "Strukturpiloten/boxferry"
        api.items.return_value = [
            {
                "context": subject.STATUS_CONTEXT,
                "state": "success",
                "creator": {"login": "github-actions[bot]"},
                "target_url": "https://github.com/Strukturpiloten/boxferry/actions/runs/10",
            }
        ]
        api.get.return_value = {"run_attempt": 2}
        report = {"status": "pass", "candidate_sha": HEAD, "base_sha": BASE}
        with (
            patch.object(subject, "trusted_run", return_value=True),
            patch.object(subject, "run_artifact", return_value={}),
            patch.object(
                subject,
                "validate_state",
                return_value={
                    "run_attempt": 1,
                    "reports": {HEAD: report},
                },
            ),
        ):
            self.assertIsNone(
                subject.candidate_receipt(
                    api,
                    HEAD,
                    AUTHORITY,
                    "main",
                    datetime.now(UTC),
                    expected_base_sha=BASE,
                )
            )
