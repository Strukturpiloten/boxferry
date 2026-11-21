"""Canonical full offline A/C and E paths derived from authored v5 ideal fixtures.

Every endpoint is an in-memory route. ZIPs, custody copies, fsync and guard calls
are real local operations. No helper/archive import, subprocess or HTTP is used.
Synthetic request totals are not a live provider or future-series budget promise.
"""

from __future__ import annotations

import argparse
import contextlib
import copy
import datetime as dt
import hashlib
import io
import json
import os
import stat
import urllib.request
import uuid
import zipfile
from collections.abc import Callable
from pathlib import Path
from typing import IO, Any, ClassVar, Self, cast
from unittest.mock import Mock, patch

from test_provider_probe_controller import OfflineCase, S

from boxferry_ci import provider_probe_controller as C
from boxferry_ci import provider_probe_observer as O


class FixtureResponse:
    headers: ClassVar[dict[str, str]] = {}

    def __init__(self, status: int, raw: bytes) -> None:
        self.status, self.raw = status, raw

    def __enter__(self) -> Self:
        return self

    def __exit__(self, *args: object) -> None:
        pass

    def read(self, limit: int) -> bytes:
        return self.raw[:limit]


class Fixture:
    def __init__(self, owner: OfflineCase) -> None:
        self.owner = owner
        self.root, self.custody = (owner.root, owner.custody)
        self.controller = owner.initialize()
        self.e = self.controller.e
        self.routes: dict[str, Any] = {}
        self.calls: list[tuple[float, str]] = []
        self.posts: list[tuple[float, str, bytes | None]] = []
        transport = patch.object(self.e, "transport", side_effect=self.transport)
        transport.start()
        owner.addCleanup(transport.stop)
        self.controller.opener = Mock(open=self.open)
        self.controller.c.update(preflight=True, preflight_ids=[], main_ci_run=123)

    @property
    def now(self) -> float:
        return self.owner.clock

    def advance(self, amount: float = 5.1) -> None:
        self.owner.clock += amount

    def transport(
        self, url: str, *, authorization: bool, limit: int, purpose: str
    ) -> tuple[int, str | None, bytes]:
        assert authorization is True
        suffix = url.removeprefix(C.API + C.PREFIX + "/")
        assert suffix in self.routes, suffix
        self.e.charge(1, "offline-" + purpose)
        self.calls.append((self.now, suffix))
        value = self.routes[suffix]
        raw = value if isinstance(value, bytes) else json.dumps(value).encode()
        assert len(raw) <= limit
        return (200, None, raw)

    def open(self, request: urllib.request.Request, timeout: float) -> FixtureResponse:
        assert timeout == 15
        path = request.full_url.removeprefix(C.API)
        method = request.get_method()
        self.calls.append((self.now, method + " " + path))
        if method == "GET":
            assert path == C.PREFIX + "/git/ref/heads/main"
            return FixtureResponse(
                200,
                json.dumps(dict(ref="refs/heads/main", object=dict(type="commit", sha=S))).encode(),
            )
        assert method == "POST"
        key = next(
            (key for key, row in self.controller.c["effects"].items() if row.get("path") == path)
        )
        assert self.controller.c["effects"][key]["state"] == "spent-before-transport"
        assert (
            json.loads(self.e.state_path.read_text())["controller_v5"]["effects"][key]["state"]
            == "spent-before-transport"
        )
        assert self.e.state["requests"] > 0
        self.posts.append((self.now, path, cast(bytes | None, request.data)))
        return FixtureResponse(202 if path.endswith("/cancel") else 204, b"")

    def run(
        self, case: str, identifier: int, status: str = "in_progress", conclusion: str | None = None
    ) -> dict[str, Any]:
        self.e.state["runs"][case] = dict(id=identifier)
        self.controller.c["discovered"][case] = identifier
        self.controller.c["effects"]["dispatch:" + case] = dict(state="accepted", case=case)
        row = dict(
            id=identifier,
            run_attempt=1,
            workflow_id=self.e.state["workflow_id"],
            path=O.WORKFLOW,
            repository=dict(full_name=O.REPO),
            head_repository=dict(full_name=O.REPO),
            event="workflow_dispatch",
            head_branch="main",
            head_sha=S,
            display_title=f"629 provider probe {self.e.state['series']} case {case}",
            actor=self.e.state["actor"],
            triggering_actor=self.e.state["triggering_actor"],
            html_url=f"https://github.com/{O.REPO}/actions/runs/{identifier}",
            status=status,
            conclusion=conclusion,
            created_at=O.utc(),
        )
        self.routes[f"actions/runs/{identifier}"] = row
        self.routes[f"actions/runs/{identifier}/attempts/1/jobs?per_page=100&page=1"] = dict(
            total_count=0, jobs=[]
        )
        self.routes[f"actions/runs/{identifier}/artifacts?per_page=100&page=1"] = dict(
            total_count=0, artifacts=[]
        )
        return row

    def job(
        self,
        case: str,
        role: str,
        identifier: int,
        check: int,
        status: str,
        conclusion: str | None = None,
    ) -> dict[str, Any]:
        run = self.e.state["runs"][case]["id"]
        name = (
            "629 preparation call / 629 preparation witness"
            if role == "prepare"
            else "629 inert effect call / 629 inert dependent witness"
        )
        steps = [dict(name=O.HOLDS[role], number=7, status=status, conclusion=conclusion)]
        if role == "effect":
            steps.insert(
                0,
                dict(
                    name="Upload inert dependent entry witness",
                    number=6,
                    status="completed",
                    conclusion="success",
                ),
            )
        job = dict(
            id=identifier,
            name=name,
            run_id=run,
            run_attempt=1,
            head_sha=S,
            check_run_url=f"{C.API}{C.PREFIX}/check-runs/{check}",
            status=status,
            conclusion=conclusion,
            runner_id=99,
            steps=steps,
        )
        checkrow = dict(
            id=check,
            name=name,
            head_sha=S,
            status=status,
            conclusion=conclusion,
            app=dict(id=15368, slug="github-actions"),
            details_url=f"https://github.com/{O.REPO}/actions/runs/{run}/job/{identifier}",
        )
        self.routes[f"actions/jobs/{identifier}"] = job
        self.routes[f"check-runs/{check}"] = checkrow
        return job

    def artifacts(
        self, case: str, roles: tuple[str, ...] = ("prepare", "effect"), first_id: int = 70
    ) -> list[dict[str, Any]]:
        run = self.e.state["runs"][case]["id"]
        artifacts: list[dict[str, Any]] = []
        for index, role in enumerate(roles, first_id):
            witness = dict(
                schema=O.SCHEMA,
                repository=O.REPO,
                workflow_path=O.WORKFLOW,
                source_sha=S,
                series=self.e.state["series"],
                case=case,
                run_id=str(run),
                run_attempt=1,
                role=role,
                marker_phase="completed" if role == "prepare" else "entry",
            )
            if role == "effect":
                witness.update(
                    preparation_artifact_id=str(first_id),
                    preparation_artifact_digest=artifacts[0]["digest"][7:],
                    preparation_marker=f"{O.SCHEMA}:{self.e.state['series']}:{run}:1:{S}",
                    caller_prerequisite_result="success",
                )
            buffer = io.BytesIO()
            with zipfile.ZipFile(buffer, "w") as archive:
                member = zipfile.ZipInfo("witness.json")
                member.create_system = 3
                member.external_attr = (stat.S_IFREG | 384) << 16
                archive.writestr(member, json.dumps(witness))
            raw = buffer.getvalue()
            digest = hashlib.sha256(raw).hexdigest()
            suffix = "prepare" if role == "prepare" else "effect-entry"
            artifact = dict(
                id=index,
                name=f"boxferry-629-provider-probe-v1-{self.e.state['series']}-{run}-attempt1-{suffix}",
                digest="sha256:" + digest,
                size_in_bytes=len(raw),
                expired=False,
                workflow_run=dict(id=run, head_sha=S),
                created_at=(dt.datetime.now(dt.UTC) - dt.timedelta(minutes=1)).isoformat(),
                expires_at=(dt.datetime.now(dt.UTC) + dt.timedelta(days=1)).isoformat(),
            )
            artifacts.append(artifact)
            self.routes[f"actions/artifacts/{index}"] = artifact
            self.routes[f"actions/artifacts/{index}/zip"] = raw
        self.routes[f"actions/runs/{run}/artifacts?per_page=100&page=1"] = dict(
            total_count=len(artifacts), artifacts=artifacts
        )
        return artifacts

    def inventory_jobs(self, case: str, jobs: list[dict[str, Any]]) -> None:
        run = self.e.state["runs"][case]["id"]
        self.routes[f"actions/runs/{run}/attempts/1/jobs?per_page=100&page=1"] = dict(
            total_count=len(jobs), jobs=jobs
        )


class EFixture(Fixture):
    def __init__(self, owner: OfflineCase) -> None:
        super().__init__(owner)
        self.active_log_attempts = 0
        original = self.e.download_bytes

        def guarded(suffix: str, limit: int, purpose: str, run_scope: int) -> bytes:
            if (
                suffix == "actions/jobs/51/logs"
                and self.routes["actions/jobs/51"]["status"] != "completed"
            ):
                self.active_log_attempts += 1
                raise AssertionError("active effect log endpoint must never be called")
            return original(suffix, limit, purpose, run_scope)

        download = patch.object(self.e, "download_bytes", side_effect=guarded)
        download.start()
        owner.addCleanup(download.stop)

    def make_wrapper(self, role: str, identifier: int, check: int) -> dict[str, Any]:
        leaf = self.job(
            "E",
            role,
            identifier,
            check,
            "completed" if role == "prepare" else "in_progress",
            "success" if role == "prepare" else None,
        )
        leaf["name"] = "629 preparation call" if role == "prepare" else "629 inert effect call"
        self.routes[f"check-runs/{check}"]["name"] = leaf["name"]
        leaf["steps"] = []
        return leaf

    def prepare(
        self,
        *,
        freeze: bool = True,
        wrapper: bool = False,
        effect_change: Callable[..., None] | None = None,
        prep_change: Callable[..., None] | None = None,
        artifact_change: Callable[..., None] | None = None,
    ) -> Self:
        self.run("E", 5)
        prep = self.job("E", "prepare", 50, 60, "completed", "success")
        prep["steps"] = [
            dict(name=O.E_UPLOADS["prepare"], number=4, status="completed", conclusion="success")
        ]
        effect = self.job("E", "effect", 51, 61, "in_progress")
        effect["steps"] = [
            dict(name=O.E_VALIDATION, number=5, status="completed", conclusion="success"),
            dict(name=O.E_UPLOADS["effect"], number=6, status="completed", conclusion="success"),
            dict(name=O.HOLDS["effect"], number=7, status="in_progress", conclusion=None),
        ]
        if effect_change:
            effect_change(effect)
        if prep_change:
            prep_change(prep)
        jobs = [prep, effect]
        self.wrapper = wrapper
        if wrapper:
            jobs.append(self.make_wrapper("prepare", 52, 62))
        self.inventory_jobs("E", jobs)
        self.arts = self.artifacts("E")
        if artifact_change:
            artifact_change(self)
        self.e.snapshot("E", 5, "E-inventory-before")
        self.advance(0.1)
        self.e.job("E", 5, 50, 60, "E-prepare-uploader-completed")
        self.advance(0.1)
        self.log_binding("prepare")
        self.advance(0.1)
        self.e.download("E", 5, 70, self.arts[0]["digest"][7:], "E-prepare-archive")
        if wrapper:
            self.advance(0.1)
            self.e.job("E", 5, 52, 62, "E-wrapper-before")
        self.advance(5.1)
        self.e.job("E", 5, 51, 61, "E-effect-hold")
        self.advance(0.1)
        self.e.job("E", 5, 50, 60, "E-prepare-pre-cancel")
        if freeze:
            O.guard_e(
                self.e, argparse.Namespace(prepare="E-prepare-pre-cancel", effect="E-effect-hold")
            )
        return self

    def log_binding(self, role: str) -> None:
        identifier = 70 if role == "prepare" else 71
        job, check = (50, 60) if role == "prepare" else (51, 61)
        label = "E-prepare-log" if role == "prepare" else "E-effect-terminal-log"
        digest = self.arts[0 if role == "prepare" else 1]["digest"][7:]
        raw = (
            f"##[group]Run {self.e.state['source_descriptor']['uploader']}\n"
            f"Artifact ID: {identifier}\n"
            f"SHA256 digest of uploaded artifact zip is {digest}\n"
        ).encode()
        self.routes[f"actions/jobs/{job}/logs"] = raw
        self.e.log("E", 5, job, check, label, O.E_PRE if role == "prepare" else O.E_POST)
        self.e.bind_e_uploader(label, identifier, digest, 4 if role == "prepare" else 6, 1, 3)

    def durable(self) -> Path:
        destination = self.root / "synthetic-external-copy"
        destination.mkdir(mode=0o700)
        self.controller.copy_e(destination)
        return destination

    def freeze_layout(self) -> None:
        # 212 indexed files plus the copied state and attestation. Use authored
        # bytes rather than reading either sealed series.
        files = self.e.state["files"]
        for number in range(len(files), 211):
            name = f"{number + 1:04d}-offline-layout-{number}"
            raw = (f"fixture-{number} " * 400).encode()
            (self.e.directory / name).write_bytes(raw)
            files.append(
                dict(
                    path=name,
                    bytes=len(raw),
                    sha256=hashlib.sha256(raw).hexdigest(),
                    utc=O.utc(),
                    elapsed=self.e.elapsed(),
                )
            )
        self.e.persist()
        O.guard_e(
            self.e, argparse.Namespace(prepare="E-prepare-pre-cancel", effect="E-effect-hold")
        )

    def cancel(self) -> None:
        self.destination = self.durable()
        self.advance(0.1)
        self.controller.cancel("E", 5, False, self.destination)

    def terminal(
        self,
        *,
        effect_conclusion: str = "cancelled",
        first_auth: bool = True,
        corroborate: bool = True,
    ) -> None:
        self.routes["actions/runs/5"].update(status="completed", conclusion="cancelled")
        self.routes["actions/jobs/51"].update(status="completed", conclusion=effect_conclusion)
        self.routes["check-runs/61"].update(status="completed", conclusion=effect_conclusion)
        self.routes["actions/jobs/51"]["steps"][-1].update(
            status="completed", conclusion=effect_conclusion
        )
        self.advance()
        self.e.snapshot("E", 5, "E-terminal")
        if first_auth:
            self.advance(0.1)
            self.e.job("E", 5, 50, 60, "E-prepare-terminal")
            self.advance(0.1)
            self.e.job("E", 5, 51, 61, "E-effect-terminal")
            if self.wrapper:
                self.advance(0.1)
                self.e.job("E", 5, 52, 62, "E-wrapper-terminal")
        self.advance()
        self.e.snapshot("E", 5, "E-terminal-stable")
        self.advance(0.1)
        self.e.job("E", 5, 50, 60, "E-prepare-stable")
        self.advance(0.1)
        self.e.job("E", 5, 51, 61, "E-effect-stable")
        if self.wrapper:
            self.advance(0.1)
            self.e.job("E", 5, 52, 62, "E-wrapper-stable")
        if corroborate:
            self.advance(0.1)
            self.log_binding("effect")
            self.advance(0.1)
            self.e.download("E", 5, 71, self.arts[1]["digest"][7:], "E-effect-terminal-archive")

    def compare(self) -> dict[str, Any]:
        return O.compare_e(
            self.e,
            argparse.Namespace(
                prepare_before="E-prepare-pre-cancel",
                prepare_after="E-prepare-stable",
                inventory_before="E-inventory-before",
                inventory_after="E-terminal-stable",
                artifact=70,
            ),
        )


class FreshProtocolCase(OfflineCase):
    def init_arguments(self) -> list[str]:
        arguments = super().init_arguments()
        arguments[arguments.index("--series") + 1] = uuid.uuid4().hex
        return arguments


class ProtocolTests(FreshProtocolCase):
    def test_full_e_leaf_only_cut_durable_copy_cancel_terminal_and_provenance(self) -> None:
        fixture = EFixture(self).prepare()
        cut = copy.deepcopy(fixture.e.state["e_v5"]["pre_cancel_cut"])
        fixture.cancel()
        self.assertEqual(fixture.e.state["requests"], 15)
        fixture.terminal()
        result = fixture.compare()
        self.assertEqual(fixture.e.state["requests"], 32)
        self.assertEqual(result["outcome"], "OBSERVED_LEAF_RETENTION_ONLY")
        self.assertEqual(result["qualification"], "CALLER_WRAPPER_UNOBSERVED")
        self.assertEqual(result["caller_wrapper_retention"], "UNOBSERVED")
        self.assertTrue(
            all(
                result[key]
                for key in (
                    "effect_cancellation_observed",
                    "preparation_leaf_retention_observed",
                    "effect_entry_provenance_verified",
                )
            )
        )
        self.assertFalse(result["production_ready"])
        self.assertEqual(len(fixture.posts), 1)
        self.assertEqual(fixture.active_log_attempts, 0)
        self.assertEqual(fixture.e.state["e_v5"]["pre_cancel_cut"], cut)
        self.assertEqual(fixture.e.state["e_v5_provenance"]["70"]["phase"], O.E_PRE)
        self.assertEqual(fixture.e.state["e_v5_provenance"]["71"]["phase"], O.E_POST)
        copied = fixture.controller.c["E_durable_readback"]
        self.assertTrue(copied["directory_fsynced"])
        self.assertGreater(copied["file_fsync_count"], 2)
        with self.assertRaises(O.Inconclusive):
            fixture.controller.cancel("E", 5, False, fixture.destination)
        self.assertEqual(len(fixture.posts), 1)

    def test_e_full_size_durable_copy_and_controller_recheck(self) -> None:
        fixture = EFixture(self).prepare(freeze=False)
        fixture.freeze_layout()
        self.assertEqual(len(fixture.e.state["files"]), 212)
        spent = fixture.e.state["requests"]
        expected = {
            item["path"]: (fixture.e.directory / item["path"]).read_bytes()
            for item in fixture.e.state["files"]
        }
        expected["state.json"] = fixture.e.state_path.read_bytes()
        cut = fixture.e.state["e_v5"]["pre_cancel_cut"]
        destination = fixture.durable()
        self.assertEqual(fixture.e.state["requests"], spent)
        self.assertEqual(
            {file.name for file in destination.iterdir()},
            expected.keys() | {"storage-attestation.json"},
        )
        for name, contents in expected.items():
            self.assertEqual((destination / name).read_bytes(), contents, name)
        self.assertEqual(
            O.decode((destination / "storage-attestation.json").read_bytes()),
            {
                "schema": "boxferry.provider632.storage-attestation.v5",
                "series": fixture.e.state["series"],
                "source_sha": S,
                "observer_sha256": fixture.e.state["observer_sha256"],
                "controller_sha256": fixture.controller.source_hash,
                "cut_sha256": cut["cut_sha256"],
                "root_reviewed": True,
                "storage_class": "operator-reviewed-durable-filesystem",
                "limits": "fsync-and-readback-only; no WORM or power-loss guarantee",
            },
        )
        fixture.controller.cancel("E", 5, False, destination)
        self.assertEqual(fixture.controller.c["E_durable_readback"]["file_fsync_count"], 214)
        self.assertEqual(len(fixture.posts), 1)

    def test_e_durable_changes_refuse_before_cancellation(self) -> None:
        for change in ("missing", "extra", "altered", "attestation", "state", "replaced"):
            with self.subTest(change=change):
                # Use exclusive new custody and mocked series registration for each case.
                case = FreshProtocolCase()
                case.setUp()
                try:
                    fixture = EFixture(case).prepare(freeze=False)
                    fixture.freeze_layout()
                    destination = fixture.durable()
                    data = destination / fixture.e.state["files"][-1]["path"]
                    if change == "missing":
                        data.unlink()
                    elif change == "extra":
                        (destination / "extra").write_bytes(b"unexpected")
                    elif change == "altered":
                        raw = data.read_bytes()
                        data.write_bytes(bytes((raw[0] ^ 1,)) + raw[1:])
                    elif change == "attestation":
                        path = destination / "storage-attestation.json"
                        path.write_bytes(
                            path.read_bytes().replace(
                                b'"root_reviewed":true', b'"root_reviewed":null'
                            )
                        )
                    elif change == "state":
                        path = destination / "state.json"
                        path.write_bytes(path.read_bytes() + b" ")
                    else:
                        moved = destination.with_name("moved-copy")
                        destination.rename(moved)
                        destination.mkdir(mode=0o700)
                        for file in moved.iterdir():
                            (destination / file.name).write_bytes(file.read_bytes())
                    refusal = {
                        "missing": "E durable file set differs",
                        "extra": "E durable file set differs",
                        "altered": "durable readback mismatch",
                        "attestation": "durable readback mismatch",
                        "state": "durable proof file shape mismatch",
                        "replaced": "E durable directory identity changed",
                    }
                    with self.assertRaisesRegex(O.Inconclusive, refusal[change]):
                        fixture.controller.cancel("E", 5, False, destination)
                    self.assertEqual(fixture.posts, [])
                    self.assertNotIn("cancel:E", fixture.controller.c["effects"])
                finally:
                    case.doCleanups()

    def test_e_failed_reopen_readback_records_abort_without_requests(self) -> None:
        fixture = EFixture(self).prepare(freeze=False)
        fixture.freeze_layout()
        destination = fixture.root / "durable"
        destination.mkdir(mode=0o700)
        listing = C.os.listdir
        calls = 0

        def fail_second_listing(fd: int) -> list[str]:
            nonlocal calls
            calls += 1
            if calls == 2:
                raise OSError("synthetic reopened directory read failure")
            return listing(fd)

        output = io.StringIO()
        spent = fixture.e.state["requests"]
        with (
            patch.object(C.os, "listdir", side_effect=fail_second_listing),
            contextlib.redirect_stdout(output),
        ):
            code = C.main(
                [
                    "--custody",
                    str(fixture.custody),
                    "copy-e",
                    "--durable-custody",
                    str(destination),
                    "--storage-reviewed",
                ]
            )
        self.assertEqual(calls, 2)
        self.assertEqual(code, 2)
        self.assertEqual(json.loads(output.getvalue())["refusal_code"], "E_COPY_IO")
        state = json.loads(fixture.e.state_path.read_bytes())
        self.assertEqual(state["requests"], spent)
        self.assertTrue(state["controller_v5"]["aborted"])
        self.assertNotIn("E_copy_verified", state["controller_v5"])
        self.assertNotIn("cancel:E", state["controller_v5"]["effects"])
        self.assertEqual(fixture.posts, [])
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(
                C.main(
                    [
                        "--custody",
                        str(fixture.custody),
                        "cancel",
                        "--case",
                        "E",
                        "--run",
                        "5",
                        "--durable-custody",
                        str(destination),
                    ]
                ),
                2,
            )
        self.assertEqual(json.loads(fixture.e.state_path.read_bytes())["requests"], spent)

    def test_e_copy_syncs_original_writers_and_refuses_writer_failure(self) -> None:
        for failure in (None, "sync", "wrap"):
            with self.subTest(failure=failure):
                case = FreshProtocolCase()
                case.setUp()
                try:
                    self.check_e_copy_writer_sync(case, failure)
                finally:
                    case.doCleanups()

    def check_e_copy_writer_sync(self, case: FreshProtocolCase, failure: str | None) -> None:
        fixture = EFixture(case).prepare(freeze=False)
        fixture.freeze_layout()
        destination = fixture.root / "durable"
        destination.mkdir(mode=0o700)
        target = destination.stat()
        identity = (target.st_dev, target.st_ino)
        expected = {item["path"] for item in fixture.e.state["files"]} | {
            "state.json",
            "storage-attestation.json",
        }
        real_open, real_fdopen, real_fsync = os.open, os.fdopen, os.fsync
        created: dict[int, str] = {}
        writers: dict[tuple[int, int], tuple[str, io.BufferedWriter]] = {}
        synced: list[str] = []
        failed_syncs: list[str] = []
        failed_wrappers: list[int] = []
        wrap_target = fixture.e.state["files"][1]["path"]
        peak_writers = 0
        directory_syncs = 0

        def track_open(
            name: str, flags: int, mode: int = 0o777, *, dir_fd: int | None = None
        ) -> int:
            descriptor = real_open(name, flags, mode, dir_fd=dir_fd)
            if dir_fd is not None and flags & os.O_EXCL:
                entry = os.fstat(dir_fd)
                if (entry.st_dev, entry.st_ino) == identity:
                    created[descriptor] = name
            return descriptor

        def track_writer(
            descriptor: int, mode: str, *, encoding: str | None = None, closefd: bool = True
        ) -> IO[bytes] | IO[str]:
            nonlocal peak_writers
            if failure == "wrap" and created.get(descriptor) == wrap_target:
                failed_wrappers.append(descriptor)
                raise OSError("synthetic failure before writer wrapper assumes ownership")
            stream = real_fdopen(descriptor, mode, encoding=encoding, closefd=closefd)
            if descriptor in created:
                self.assertEqual(mode, "wb")
                entry = os.fstat(descriptor)
                writers[(entry.st_dev, entry.st_ino)] = (
                    created.pop(descriptor),
                    cast(io.BufferedWriter, stream),
                )
                peak_writers = max(
                    peak_writers, sum(not writer.closed for _, writer in writers.values())
                )
            return stream

        def check_sync(descriptor: int) -> None:
            nonlocal directory_syncs
            entry = os.fstat(descriptor)
            key = (entry.st_dev, entry.st_ino)
            if key == identity:
                self.assertCountEqual(synced, expected)
                self.assertTrue(all(writer.closed for _, writer in writers.values()))
                directory_syncs += 1
            if key in writers:
                name, writer = writers[key]
                self.assertFalse(writer.closed, name)
                self.assertEqual(descriptor, writer.fileno(), name)
                self.assertEqual(entry.st_size, writer.tell(), name)
                if failure == "sync" and name == "storage-attestation.json":
                    failed_syncs.append(name)
                    raise OSError("synthetic writer writeback error")
                real_fsync(descriptor)
                synced.append(name)
                return
            real_fsync(descriptor)

        spent = fixture.e.state["requests"]
        output = io.StringIO()
        with (
            patch.object(C.os, "open", side_effect=track_open),
            patch.object(C.os, "fdopen", side_effect=track_writer),
            patch.object(C.os, "fsync", side_effect=check_sync),
            contextlib.redirect_stdout(output),
        ):
            code = C.main(
                [
                    "--custody",
                    str(fixture.custody),
                    "copy-e",
                    "--durable-custody",
                    str(destination),
                    "--storage-reviewed",
                ]
            )
        if failure == "wrap":
            self.assertEqual(len(failed_wrappers), 1)
            for descriptor in failed_wrappers:
                with self.assertRaises(OSError):
                    os.fstat(descriptor)
            self.assertEqual(len(writers), 1)
            self.assertEqual(synced, [])
            self.assertEqual(failed_syncs, [])
        else:
            self.assertEqual(failed_wrappers, [])
            self.assertCountEqual([name for name, _ in writers.values()], expected)
        self.assertTrue(all(writer.closed for _, writer in writers.values()))
        self.assertLessEqual(peak_writers, 8)
        state = json.loads(fixture.e.state_path.read_bytes())
        self.assertEqual(state["requests"], spent)
        self.assertNotIn("cancel:E", state["controller_v5"]["effects"])
        self.assertEqual(fixture.posts, [])
        if failure is None:
            self.assertEqual(code, 0)
            self.assertCountEqual(synced, expected)
            self.assertEqual(failed_syncs, [])
            self.assertEqual(directory_syncs, 1)
            self.assertIn("E_copy_verified", state["controller_v5"])
        else:
            self.assertEqual(code, 2)
            if failure == "sync":
                self.assertEqual(failed_syncs, ["storage-attestation.json"])
            self.assertEqual(directory_syncs, 0)
            self.assertEqual(json.loads(output.getvalue())["refusal_code"], "E_COPY_IO")
            self.assertTrue(state["controller_v5"]["aborted"])
            self.assertNotIn("E_copy_verified", state["controller_v5"])

    def test_e_copy_success_does_not_extend_hold_freshness(self) -> None:
        fixture = EFixture(self).prepare()
        destination = fixture.durable()
        fixture.advance(5.1)
        with self.assertRaisesRegex(O.Inconclusive, "fresh positive hold evidence required"):
            fixture.controller.cancel("E", 5, False, destination)
        self.assertEqual(fixture.posts, [])
        self.assertNotIn("cancel:E", fixture.controller.c["effects"])

    def test_full_e_observed_preparation_wrapper_has_separate_retention_scope(self) -> None:
        fixture = EFixture(self).prepare(wrapper=True)
        fixture.cancel()
        fixture.terminal()
        result = fixture.compare()
        self.assertEqual(result["caller_wrapper_retention"], "OBSERVED")
        self.assertEqual(result["outcome"], "OBSERVED_E_RETENTION_WITH_WRAPPER")
        self.assertFalse(result["production_ready"])
        self.assertEqual(fixture.e.state["requests"], 38)
        self.assertEqual(len(fixture.posts), 1)

    def test_full_e_natural_success_does_not_qualify_or_refund_cancel(self) -> None:
        fixture = EFixture(self).prepare()
        fixture.cancel()
        with self.assertRaises(O.Inconclusive):
            fixture.terminal(effect_conclusion="success")
        with self.assertRaises(O.Inconclusive):
            fixture.compare()
        self.assertEqual(fixture.controller.c["effects"]["cancel:E"]["state"], "accepted")
        self.assertEqual(len(fixture.posts), 1)

    def test_full_e_changed_preparation_wrapper_is_not_absent_or_retained(self) -> None:
        fixture = EFixture(self).prepare(wrapper=True)
        fixture.cancel()
        fixture.routes["actions/jobs/52"].update(conclusion="cancelled")
        fixture.routes["check-runs/62"].update(conclusion="cancelled")
        fixture.terminal(corroborate=False)
        with self.assertRaises(O.Inconclusive):
            fixture.compare()
        self.assertEqual(len(fixture.posts), 1)
        self.assertEqual(fixture.controller.c["effects"]["cancel:E"]["state"], "accepted")

    def test_full_e_missing_final_provenance_cannot_backfill_or_repeat(self) -> None:
        fixture = EFixture(self).prepare()
        fixture.cancel()
        fixture.terminal(corroborate=False)
        with self.assertRaises(O.Inconclusive):
            fixture.compare()
        facts = fixture.e.state["e_v5"]["qualification_facts"]
        self.assertEqual(facts["outcome"], "INCOMPLETE_E_EVIDENCE")
        self.assertFalse(facts["effect_entry_provenance_verified"])
        self.assertEqual(len(fixture.posts), 1)

    def test_successful_control_a_and_c_use_actual_zip_and_complete_stable_identities(self) -> None:
        fixture = Fixture(self)
        for case, run in (("A", 1), ("C", 3)):
            fixture.run(case, run, "in_progress", None)
            prepare_id, effect_id = (10, 11) if case == "A" else (30, 31)
            preparation = fixture.job(
                case,
                "prepare",
                prepare_id,
                prepare_id + 100,
                "in_progress" if case == "A" else "completed",
                None if case == "A" else "success",
            )
            if case == "A":
                fixture.e.job(case, run, prepare_id, prepare_id + 100, "A-after-B")
                fixture.advance()
                preparation.update(status="completed", conclusion="success")
                preparation["steps"][0].update(status="completed", conclusion="success")
                fixture.routes[f"check-runs/{prepare_id + 100}"].update(
                    status="completed", conclusion="success"
                )
            fixture.routes[f"actions/runs/{run}"].update(status="completed", conclusion="success")
            effect = fixture.job(case, "effect", effect_id, effect_id + 100, "completed", "success")
            fixture.inventory_jobs(case, [preparation, effect])
            artifacts = fixture.artifacts(case, first_id=70 if case == "A" else 80)
            fixture.e.snapshot(case, run, case + "-terminal")
            for artifact in artifacts:
                fixture.e.download(
                    case,
                    run,
                    artifact["id"],
                    artifact["digest"][7:],
                    case + "-archive-" + str(artifact["id"]),
                )
            fixture.advance()
            fixture.e.snapshot(case, run, case + "-terminal-stable")
            fixture.advance(0.1)
            fixture.e.job(case, run, prepare_id, prepare_id + 100, case + "-prepare-complete")
            fixture.advance(0.1)
            fixture.e.job(case, run, effect_id, effect_id + 100, case + "-effect-complete")
            result = fixture.controller.successful_control(case)
            self.assertEqual(result["case"], case)
            self.assertEqual(result["prepare_job"], prepare_id)
            self.assertEqual(len(result["artifact_ids"]), 2)
        self.assertEqual(fixture.posts, [])
