"""Authored v5 observer regressions retained against the canonical package, offline only."""

from __future__ import annotations

import argparse
import copy
import hashlib
import http.client
import io
import json
import stat
import tempfile
import time
import urllib.error
import urllib.request
import zipfile
from pathlib import Path
from typing import Any, ClassVar, Self, cast
from unittest.mock import patch

from test_provider_probe_controller import OfflineCase, S, redirect_error

from boxferry_ci import provider_probe_observer as observe
from boxferry_ci.provider_probe_observer import (
    E_PRE,
    E_UPLOADS,
    HOLDS,
    MAX_ARCHIVE,
    MAX_REQUESTS,
    OBSERVER_SCHEMA,
    REPO,
    SCHEMA,
    WORKFLOW,
    Evidence,
    Inconclusive,
    compare_e,
    decode,
    e_provenance,
    e_steps,
    guard_burst,
    guard_e,
    hard_request_deadline,
    object_field,
    safe_redirect,
    verify_archive,
)


class ObserverTests(OfflineCase):
    def custody_state(self, path: Path, value: dict[str, Any]) -> None:
        # Adapt only new-package source custody; inherited transport/role assertions stay real.
        current = copy.deepcopy(value)
        current.update(
            source_descriptor=self.descriptor,
            discovery_reads={},
            custody_directory=str(path.parent.resolve()),
        )
        raw = self.raw
        name = "0000-source-manifest"
        (path.parent / name).write_bytes(raw)
        current["source_manifest_sha256"] = hashlib.sha256(raw).hexdigest()
        current.setdefault("files", []).insert(
            0,
            {
                "path": name,
                "bytes": len(raw),
                "sha256": hashlib.sha256(raw).hexdigest(),
                "utc": observe.utc(),
                "elapsed": 0,
            },
        )
        observe.write_json(path, current)

    def archive(
        self, witness: dict[str, Any], name: str = "witness.json", mode: int = stat.S_IFREG | 420
    ) -> bytes:
        data = io.BytesIO()
        with zipfile.ZipFile(data, "w") as archive:
            member = zipfile.ZipInfo(name)
            member.external_attr = mode << 16
            archive.writestr(member, json.dumps(witness))
        return data.getvalue()

    def test_archive_boundary(self) -> None:
        witness = {
            "schema": SCHEMA,
            "repository": REPO,
            "workflow_path": WORKFLOW,
            "source_sha": "b" * 40,
            "series": "a" * 32,
            "case": "E",
            "run_id": "12",
            "run_attempt": 1,
            "role": "prepare",
            "marker_phase": "completed",
        }
        raw = self.archive(witness)
        digest_value = hashlib.sha256(raw).hexdigest()
        artifact = {
            "name": "prefix-prepare",
            "digest": "sha256:" + digest_value,
            "size_in_bytes": len(raw),
        }
        state = {"source_sha": "b" * 40, "series": "a" * 32}
        self.assertEqual(verify_archive(raw, artifact, state, "E", 12, digest_value), witness)
        for invalid in (
            self.archive(witness, "../witness.json"),
            self.archive(witness, mode=stat.S_IFLNK | 511),
            self.archive({**witness, "run_attempt": 2}),
            b"x" * (MAX_ARCHIVE + 1),
        ):
            with self.assertRaises(Inconclusive):
                verify_archive(invalid, artifact, state, "E", 12, digest_value)
        with self.assertRaises(Inconclusive):
            verify_archive(raw, artifact, state, "E", 12, "0" * 64)

    def test_redirect_policy(self) -> None:
        self.assertEqual(
            safe_redirect("https://artifact.example.test/object?signed=secret"),
            "https://artifact.example.test/object?signed=secret",
        )
        for invalid in (
            "http://artifact.example.test/a",
            "https://user@artifact.example.test/a",
            "https://artifact.example.test:444/a",
            "https://127.0.0.1/a",
            "https://localhost/a",
            "https://artifact.example.test/a#fragment",
        ):
            with self.assertRaises(Inconclusive):
                safe_redirect(invalid)

    def test_redirect_gets_are_individually_charged_and_strip_token(self) -> None:
        observed = []

        class Response:
            status = 200
            headers: ClassVar[dict[str, str]] = {}

            def __enter__(self) -> Self:
                return self

            def __exit__(self, *args: object) -> None:
                pass

            def read(self, limit: int) -> bytes:
                return b"zip"

        class Fake:
            def open(self, request: urllib.request.Request, timeout: float) -> object:
                observed.append(request)
                if len(observed) == 1:
                    raise redirect_error(
                        request.full_url, "https://artifact.example.test/object?signed=secret"
                    )
                return Response()

        with tempfile.TemporaryDirectory() as directory:
            state = {
                "schema": OBSERVER_SCHEMA,
                "observer_sha256": hashlib.sha256(Path(observe.__file__).read_bytes()).hexdigest(),
                "started_monotonic": time.monotonic(),
                "boot": Path("/proc/sys/kernel/random/boot_id").read_text().strip(),
                "requests": 0,
                "transport": [],
                "files": [],
            }
            self.custody_state(Path(directory) / "state.json", state)
            evidence = Evidence(directory)
            evidence.opener = cast(urllib.request.OpenerDirector, Fake())
            evidence.token = "offline-fake-token"
            status, location, _ = evidence.transport(
                "https://api.github.com/repos/" + REPO + "/actions/artifacts/1/zip",
                authorization=True,
                limit=20,
                purpose="first",
            )
            self.assertEqual(status, 302)
            evidence.transport(
                safe_redirect(cast(str, location)), authorization=False, limit=20, purpose="second"
            )
            self.assertEqual(evidence.state["requests"], 2)
            evidence.save("offline-archive.zip", b"bounded zip fixture")
            evidence.save("offline-raw.log", b"bounded log fixture")
            self.assertEqual(len(evidence.state["files"]), 3)
            self.assertEqual(observed[0].get_header("Authorization"), "Bearer offline-fake-token")
            self.assertIsNone(observed[1].get_header("Authorization"))
            persisted = (Path(directory) / "state.json").read_text()
            self.assertNotIn("offline-fake-token", persisted)
            self.assertNotIn("signed=secret", persisted)
            evidence.state["requests"] = MAX_REQUESTS
            with self.assertRaises(Inconclusive):
                evidence.charge(1, "over-budget")
            self.assertEqual(len(observed), 2)

    def test_burst_requires_same_step_after_terminal_victim(self) -> None:

        def observation(elapsed: float, job: int = 10, step: int = 2) -> dict[str, Any]:
            return {
                "case": "A",
                "run": 1,
                "job": job,
                "check": 20,
                "elapsed": elapsed,
                "started_elapsed": elapsed - 0.25,
                "catalog_name_shape": ["prepare", "expanded-leaf"],
                "job_record": {
                    "status": "in_progress",
                    "runner_id": 100,
                    "steps": [{"name": HOLDS["prepare"], "number": step, "status": "in_progress"}],
                },
                "check_record": {"status": "in_progress"},
            }

        state = {
            "observations": {"before": observation(5), "after": observation(9)},
            "snapshots": {
                "pending": {"case": "B", "run_status": "pending", "elapsed": 4},
                "terminal": {
                    "case": "B",
                    "run_status": "completed",
                    "run_conclusion": "cancelled",
                    "elapsed": 8,
                },
            },
        }
        evidence = argparse.Namespace(state=state)
        args = argparse.Namespace(
            before="before", after="after", pending="pending", victim="terminal"
        )
        self.assertEqual(guard_burst(cast(Evidence, evidence), args)["outcome"], "OBSERVED")
        state["observations"]["after"] = observation(7)
        with self.assertRaises(Inconclusive):
            guard_burst(cast(Evidence, evidence), args)
        state["observations"]["after"] = observation(9, step=3)
        with self.assertRaises(Inconclusive):
            guard_burst(cast(Evidence, evidence), args)
        state["observations"]["after"] = observation(9, job=11)
        with self.assertRaises(Inconclusive):
            guard_burst(cast(Evidence, evidence), args)

    def test_e_v5_unphased_source_fixture_refuses(self) -> None:
        evidence = argparse.Namespace(
            state={"source_sha": S, "source_descriptor": self.descriptor, "files": []}
        )
        with self.assertRaises(Inconclusive):
            guard_e(cast(Evidence, evidence), argparse.Namespace(prepare="p", effect="e"))

    def test_hard_deadline_and_strict_json(self) -> None:
        with self.assertRaises(Inconclusive):
            with hard_request_deadline(0.01):
                time.sleep(0.1)
        with self.assertRaises(Inconclusive):
            decode(b'{"id":1,"id":2}')

    def test_terminal_reread_budget_does_not_block_required_E_pre_snapshots(self) -> None:
        base = time.monotonic()
        with tempfile.TemporaryDirectory() as directory:
            state = {
                "schema": OBSERVER_SCHEMA,
                "observer_sha256": hashlib.sha256(Path(observe.__file__).read_bytes()).hexdigest(),
                "started_monotonic": base,
                "boot": Path("/proc/sys/kernel/random/boot_id").read_text().strip(),
                "source_sha": "b" * 40,
                "requests": 0,
                "transport": [],
                "files": [],
                "reads": {},
                "runs": {"E": {"id": 12}},
                "terminal_runs": {},
            }
            self.custody_state(Path(directory) / "state.json", state)
            evidence = Evidence(directory)

            def fake_transport(
                url: str, *, authorization: bool, limit: int, purpose: str
            ) -> tuple[int, str | None, bytes]:
                evidence.charge(1, "offline GET")
                return (
                    200,
                    None,
                    json.dumps(
                        {
                            "id": 10,
                            "run_id": 12,
                            "run_attempt": 1,
                            "head_sha": "b" * 40,
                            "status": "completed",
                            "conclusion": "success",
                        }
                    ).encode(),
                )

            transport_patch = patch.object(evidence, "transport", side_effect=fake_transport)
            transport_patch.start()
            self.addCleanup(transport_patch.stop)
            with patch("time.monotonic", return_value=base):
                evidence.api("actions/jobs/10", "first-success")
            with patch("time.monotonic", return_value=base + 6):
                evidence.api("actions/jobs/10", "success-again-before-cancel")
            evidence.state["terminal_runs"]["12"] = True
            with patch("time.monotonic", return_value=base + 12):
                evidence.api("actions/jobs/10", "first-after-run-terminal")
            with patch("time.monotonic", return_value=base + 18):
                evidence.api("actions/jobs/10", "stable-terminal-reread")
            with patch("time.monotonic", return_value=base + 24), self.assertRaises(Inconclusive):
                evidence.api("actions/jobs/10", "forbidden-additional-reread")
            self.assertEqual(evidence.state["requests"], 4)

    def retention_fixture(self) -> object:
        import copy

        job = {
            "id": 50,
            "name": "629 preparation call / 629 preparation witness",
            "run_id": 5,
            "run_attempt": 1,
            "head_sha": "b" * 40,
            "check_run_url": f"https://api.github.com/repos/{REPO}/check-runs/60",
            "status": "completed",
            "conclusion": "success",
        }
        observation = {
            "case": "E",
            "run": 5,
            "job": 50,
            "check": 60,
            "elapsed": 5,
            "started_elapsed": 4.5,
            "catalog_name_shape": ["prepare", "expanded-leaf"],
            "job_record": copy.deepcopy(job),
            "check_record": {"status": "completed", "conclusion": "success"},
        }
        artifact = {
            "id": 70,
            "name": "probe-prepare",
            "digest": "sha256:" + "d" * 64,
            "size_in_bytes": 300,
            "workflow_run": {"id": 5},
        }
        preparation_row = {**copy.deepcopy(job), "catalog_name_shape": ["prepare", "expanded-leaf"]}
        effect_row = {
            "id": 51,
            "name": "629 inert effect call / 629 inert dependent witness",
            "catalog_name_shape": ["effect", "expanded-leaf"],
            "run_id": 5,
            "run_attempt": 1,
            "head_sha": "b" * 40,
            "check_run_url": f"https://api.github.com/repos/{REPO}/check-runs/61",
            "status": "in_progress",
            "conclusion": None,
        }
        before = {
            "case": "E",
            "run": 5,
            "elapsed": 6,
            "jobs": [preparation_row, effect_row],
            "artifacts": [copy.deepcopy(artifact)],
            "run_status": "in_progress",
            "run_conclusion": None,
        }
        after: dict[str, Any] = {
            **copy.deepcopy(before),
            "elapsed": 12,
            "run_status": "completed",
            "run_conclusion": "cancelled",
        }
        after["jobs"][1].update(status="completed", conclusion="cancelled")
        verified = {
            "run": 5,
            "case": "E",
            "archive_sha256": "d" * 64,
            "archive_bytes": 300,
            "witness": {"role": "prepare"},
        }
        evidence = argparse.Namespace(
            state={
                "observations": {
                    "before": observation,
                    "after": {**copy.deepcopy(observation), "started_elapsed": 12.5, "elapsed": 13},
                },
                "snapshots": {"before": before, "after": after},
                "runs": {"E": {"verified_artifacts": {"70": verified}}},
            }
        )
        args = argparse.Namespace(
            prepare_before="before",
            prepare_after="after",
            inventory_before="before",
            inventory_after="after",
            artifact=70,
        )
        return (evidence, args)

    def test_e_v5_qualification_without_cut_refuses(self) -> None:
        evidence = argparse.Namespace(state={}, persist=lambda: None)
        with self.assertRaises(Inconclusive):
            compare_e(cast(Evidence, evidence), argparse.Namespace())
        self.assertEqual(
            evidence.state["e_v5"]["qualification_facts"]["outcome"], "INCOMPLETE_E_EVIDENCE"
        )

    def test_e_v5_duplicate_source_upload_step_refuses(self) -> None:
        step = {
            "name": E_UPLOADS["prepare"],
            "number": 4,
            "status": "completed",
            "conclusion": "success",
        }
        with self.assertRaises(Inconclusive):
            e_steps({"job_record": {"steps": [step, dict(step)]}}, "prepare")

    def test_e_v5_bare_digest_has_no_phase_provenance(self) -> None:
        evidence = argparse.Namespace(state={})
        with self.assertRaises(Inconclusive):
            e_provenance(
                cast(Evidence, evidence), {"id": 70, "digest": "sha256:" + "0" * 64}, E_PRE
            )

    def test_null_nested_provider_metadata_is_inconclusive(self) -> None:

        state = {
            "source_sha": "b" * 40,
            "series": "a" * 32,
            "workflow_id": 9,
            "actor": {"login": "operator", "id": 10},
            "triggering_actor": {"login": "operator", "id": 10},
        }
        run = {
            "id": 1,
            "run_attempt": 1,
            "workflow_id": 9,
            "path": WORKFLOW,
            "repository": {"full_name": REPO},
            "head_repository": {"full_name": REPO},
            "event": "workflow_dispatch",
            "head_branch": "main",
            "head_sha": "b" * 40,
            "display_title": "629 provider probe " + "a" * 32 + " case A",
            "actor": state["actor"],
            "triggering_actor": state["triggering_actor"],
            "html_url": f"https://github.com/{REPO}/actions/runs/1",
        }
        evidence = argparse.Namespace(state=state)
        for field in ("repository", "head_repository", "actor", "triggering_actor"):
            with self.subTest(field=field), self.assertRaises(Inconclusive):
                Evidence.validate_run(cast(Evidence, evidence), {**run, field: None}, "A", 1)
        artifact = {
            "id": 70,
            "name": "boxferry-629-provider-probe-v1-" + "a" * 32 + "-1-attempt1-prepare",
            "workflow_run": None,
        }
        with self.assertRaises(Inconclusive):
            Evidence.validate_artifact(cast(Evidence, evidence), artifact, "A", 1)
        with self.assertRaises(Inconclusive):
            object_field({"app": None}, "app")

    def fixture_evidence(self, directory: str | Path, base: float) -> Evidence:
        state = {
            "schema": OBSERVER_SCHEMA,
            "observer_sha256": hashlib.sha256(Path(observe.__file__).read_bytes()).hexdigest(),
            "started_monotonic": base,
            "boot": Path("/proc/sys/kernel/random/boot_id").read_text().strip(),
            "requests": 0,
            "transport": [],
            "files": [],
            "reads": {},
            "runs": {"A": {"id": 1}},
            "terminal_runs": {"1": True},
            "observations": {"bound": {"run": 1, "job": 10, "check": 20}},
        }
        self.custody_state(Path(directory) / "state.json", state)
        return Evidence(directory)

    def test_binary_reads_share_persisted_cadence_and_terminal_allowance(self) -> None:
        for kind in ("log", "artifact"):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as directory:
                base = time.monotonic()
                evidence = self.fixture_evidence(directory, base)
                seen = []

                class Response:
                    status = 200
                    headers: ClassVar[dict[str, str]] = {}

                    def __enter__(self) -> Self:
                        return self

                    def __exit__(self, *args: object) -> None:
                        pass

                    def read(self, limit: int) -> bytes:
                        return b"public bounded log"

                class Fake:
                    def open(
                        self,
                        request: urllib.request.Request,
                        timeout: float,
                        seen: list[urllib.request.Request] = seen,
                    ) -> object:
                        seen.append(request)
                        return Response()

                def invoke(current: Evidence, label: str, kind: str = kind) -> object:
                    current.opener = cast(urllib.request.OpenerDirector, Fake())
                    current.token = "offline-fake-token"
                    if kind == "log":
                        return current.log("A", 1, 10, 20, label)
                    return current.download_bytes("actions/artifacts/70/zip", MAX_ARCHIVE, label, 1)

                with patch("time.monotonic", return_value=base):
                    invoke(evidence, "first")
                resumed = Evidence(directory)
                with patch("time.monotonic", return_value=base), self.assertRaises(Inconclusive):
                    invoke(resumed, "immediate-repeat")
                self.assertEqual(len(seen), 1)
                with patch("time.monotonic", return_value=base + 6):
                    invoke(resumed, "stable-reread")
                with (
                    patch("time.monotonic", return_value=base + 12),
                    self.assertRaises(Inconclusive),
                ):
                    invoke(resumed, "excess-terminal-reread")
                self.assertEqual(len(seen), 2)
                self.assertEqual(resumed.state["requests"], 2)

    def test_signed_query_log_urls_are_hash_only_inconclusive(self) -> None:
        for query in (
            "X-Amz-Signature=OFFLINE_SIGNATURE",
            "X-Goog-Signature=OFFLINE_SIGNATURE",
            "signed=secret",
            "opaque=unknown",
        ):
            with self.subTest(query=query), tempfile.TemporaryDirectory() as directory:
                base = time.monotonic()
                evidence = self.fixture_evidence(directory, base)
                raw = ("https://artifact.example.test/object?" + query).encode()

                class Response:
                    status = 200
                    headers: ClassVar[dict[str, str]] = {}

                    def __enter__(self) -> Self:
                        return self

                    def __exit__(self, *args: object) -> None:
                        pass

                    def read(self, limit: int, raw: bytes = raw) -> bytes:
                        return raw

                class Fake:
                    def open(self, request: urllib.request.Request, timeout: float) -> object:
                        return Response()

                evidence.opener = cast(urllib.request.OpenerDirector, Fake())
                evidence.token = "offline-fake-token"
                with self.assertRaises(Inconclusive):
                    evidence.log("A", 1, 10, 20, "privacy")
                self.assertEqual(len(evidence.state["files"]), 2)
                self.assertIn("privacy-refused-hash", evidence.state["files"][1]["path"])
                for path in Path(directory).iterdir():
                    self.assertNotIn(raw, path.read_bytes())

    def test_signed_url_control_chars_and_HTTP_exception_messages_are_sanitized(self) -> None:
        signed = "https://artifact.example.test/object?X-Amz-Signature=OFFLINE_SIGNATURE"
        for character in ("\x00", "\x01", "\x1f", "\x7f", "\n", "\r", "\t"):
            with self.subTest(control=ord(character)), self.assertRaises(Inconclusive) as raised:
                safe_redirect(signed + character)
            self.assertNotIn("OFFLINE_SIGNATURE", str(raised.exception))
        for during_read in (False, True):
            with self.subTest(during_read=during_read), tempfile.TemporaryDirectory() as directory:
                evidence = self.fixture_evidence(directory, time.monotonic())

                class Broken:
                    status = 200
                    headers: ClassVar[dict[str, str]] = {}

                    def __enter__(self) -> Self:
                        return self

                    def __exit__(self, *args: object) -> None:
                        pass

                    def read(self, limit: int) -> bytes:
                        raise http.client.IncompleteRead(b"OFFLINE_SIGNATURE", 10)

                class Fake:
                    def open(
                        self,
                        request: urllib.request.Request,
                        timeout: float,
                        during_read: bool = during_read,
                    ) -> object:
                        if during_read:
                            return Broken()
                        raise http.client.InvalidURL("OFFLINE_SIGNATURE")

                evidence.opener = cast(urllib.request.OpenerDirector, Fake())
                evidence.token = "offline-fake-token"
                with self.assertRaises(Inconclusive) as raised:
                    evidence.transport(
                        "https://api.github.com/repos/" + REPO + "/actions/artifacts/70/zip",
                        authorization=True,
                        limit=MAX_ARCHIVE,
                        purpose="offline-private-error",
                    )
                self.assertNotIn("OFFLINE_SIGNATURE", str(raised.exception))
                self.assertTrue(raised.exception.__suppress_context__)
                for path in Path(directory).iterdir():
                    self.assertNotIn(b"OFFLINE_SIGNATURE", path.read_bytes())
