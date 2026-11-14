"""Offline actual package discovery/preflight integration; never provider acceptance."""

from __future__ import annotations

import base64
import contextlib
import copy
import hashlib
import io
import json
import sys
import tempfile
import unittest
import urllib.error
import urllib.request
from collections.abc import Sequence
from http.client import HTTPMessage
from pathlib import Path
from typing import Any
from unittest.mock import Mock, patch

from boxferry_ci import provider_probe as cli
from boxferry_ci import provider_probe_controller as control
from boxferry_ci import provider_probe_observer as observe
from boxferry_ci import provider_probe_source as source

FIXTURES = Path(__file__).parent / "fixtures/provider-probe-discovery"
REAL = json.loads((FIXTURES / "later-b-run.json").read_text())
S = REAL["head_sha"]
LOCAL_GIT = source.git
TOKEN = "CANARY_credential_signed=https://private.invalid/?token=never-emit"


def redirect_error(url: str, destination: str) -> urllib.error.HTTPError:
    headers = HTTPMessage()
    headers["Location"] = destination
    return urllib.error.HTTPError(url, 302, "redirect", headers, io.BytesIO())


class Response:
    def __init__(self, value: object, status: int = 200) -> None:
        self.status = status
        self.headers: dict[str, str] = {}
        self.raw = value if isinstance(value, bytes) else json.dumps(value).encode()

    def __enter__(self) -> Response:
        return self

    def __exit__(self, *args: object) -> None:
        pass

    def read(self, limit: int) -> bytes:
        return self.raw[:limit]


class OfflineCase(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="provider-probe-offline-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.clock = 100.0
        self.calls: list[str] = []
        for manager in (
            patch.object(source, "git", side_effect=self.git),
            patch.object(observe, "REGISTRATION_ROOT", self.root),
            patch.object(observe.time, "monotonic", side_effect=lambda: self.clock),
            patch.object(observe, "token_from_gh", return_value=TOKEN),
            patch(
                "urllib.request.OpenerDirector.open", side_effect=AssertionError("network disabled")
            ),
        ):
            manager.start()
            self.addCleanup(manager.stop)
        self.descriptor = json.loads(source.encoded(source.describe(S, 123)))
        self.raw = source.encoded(self.descriptor)
        self.manifest = self.root / "reviewed-source.json"
        self.manifest.write_bytes(self.raw)
        self.custody = self.root / "custody"

    def git(self, *args: str) -> bytes:
        """Authored already-held object model; no Git/network operation executes."""
        if args == ("rev-parse", "HEAD"):
            return S.encode()
        path = args[-1]
        if args[0] == "ls-tree":
            data = (source.ROOT / path).read_bytes()
            blob = hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()
            return f"100644 blob {blob}\t{path}\n".encode()
        if args[0] == "cat-file":
            data = (source.ROOT / path.split(":", 1)[1]).read_bytes()
            return str(len(data)).encode() if args[1] == "-s" else data
        raise AssertionError("unexpected local Git command")

    def init_arguments(self) -> list[str]:
        return [
            "init",
            "--custody",
            str(self.custody),
            "--series",
            REAL["display_title"].split()[3],
            "--source-sha",
            S,
            "--source-manifest",
            str(self.manifest),
            "--descriptor-sha256",
            hashlib.sha256(self.raw).hexdigest(),
            "--workflow-id",
            str(REAL["workflow_id"]),
            "--actor",
            REAL["actor"]["login"],
            "--actor-id",
            str(REAL["actor"]["id"]),
            "--triggering-actor",
            REAL["triggering_actor"]["login"],
            "--triggering-actor-id",
            str(REAL["triggering_actor"]["id"]),
        ]

    def initialize(self) -> control.Controller:
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(cli.main(self.init_arguments()), 0)
        result = control.Controller(observe, self.custody)
        self.assertEqual(result.e.state["requests"], 0)
        self.assertEqual(result.e.state["transport"], [])
        result.token = result.e.token = TOKEN
        return result

    def prepare(self, case: str = "B") -> control.Controller:
        result = self.initialize()
        result.c["preflight"] = True
        result.c["preflight_ids"] = []
        result.c["effects"]["dispatch:" + case] = {
            "state": "accepted",
            "transport_started_utc": "2026-11-08T17:39:02.344745+00:00",
            "after_utc": "2026-11-08T17:39:03.995247+00:00",
        }
        result.e.persist()
        return result

    def opener(self, controller: control.Controller, values: Sequence[object]) -> Mock:
        pending = list(values)

        def open_request(request: urllib.request.Request, timeout: float) -> Response:
            self.calls.append(request.full_url)
            self.assertEqual(controller.e.state["requests"], len(self.calls) + self.offset)
            self.assertEqual(
                sum(row["count"] for row in controller.e.state["transport"]),
                controller.e.state["requests"],
            )
            value = pending.pop(0)
            if isinstance(value, BaseException):
                raise value
            return value if isinstance(value, Response) else Response(value)

        self.offset = controller.e.state["requests"] - len(self.calls)
        fake = Mock()
        fake.open.side_effect = open_request
        controller.opener = controller.e.opener = fake
        return fake

    def inventory(self, *rows: dict[str, Any]) -> dict[str, Any]:
        return {"total_count": len(rows), "workflow_runs": list(rows)}

    def spent(self, controller: control.Controller, number: int) -> None:
        controller.e.state["requests"] = number
        controller.e.state["transport"] = [
            {"count": 1, "purpose": "authored prior reservation", "utc": observe.utc()}
            for _ in range(number)
        ]
        controller.e.persist()


class DiscoveryTests(OfflineCase):
    def test_later_real_b_metadata_success_is_not_the_missing_original_response(self) -> None:
        controller = self.prepare()
        self.opener(controller, [self.inventory(REAL), REAL])
        self.assertTrue(controller.discover("B"))
        report = controller.diagnostics()
        self.assertEqual(controller.e.state["runs"]["B"]["id"], 42506013197)
        self.assertEqual(report["refusal_code"], "NONE")
        self.assertEqual(report["discovery_accounting"]["global_delta"], 2)
        self.assertEqual(report["discovery_accounting"]["reads_after"], 1)
        self.assertTrue(report["own_series_discovery_facts"]["exact_target_authenticated"])
        self.assertEqual(
            observe.Evidence(self.custody).state["discovery_reports"][-1]["discovery_accounting"],
            report["discovery_accounting"],
        )

    def test_each_identity_and_window_guard_refuses_at_exact_count(self) -> None:
        mutations = (
            ("head_sha", "b" * 40),
            ("run_attempt", 2),
            ("workflow_id", 99),
            ("path", "other.yml"),
            ("repository", {"full_name": "other/repo"}),
            ("head_repository", {"full_name": "other/repo"}),
            ("event", "push"),
            ("head_branch", "other"),
            ("actor", {"id": 99, "login": "other"}),
            ("triggering_actor", {"id": 99, "login": "other"}),
            ("html_url", "https://github.com/other/repo/actions/runs/1"),
            ("created_at", "2026-11-08T17:40:03Z"),
            ("repository", None),
            ("head_repository", [TOKEN]),
            ("actor", None),
            ("triggering_actor", [TOKEN]),
        )
        for key, invalid in mutations:
            with self.subTest(field=key), tempfile.TemporaryDirectory() as scratch:
                # Each independent new-series fixture has separate owned registration/custody.
                old_root, old_custody = self.root, self.custody
                self.root, self.custody = Path(scratch), Path(scratch) / "custody"
                with patch.object(observe, "REGISTRATION_ROOT", self.root):
                    controller = self.prepare()
                    row = {**REAL, key: invalid}
                    self.opener(controller, [self.inventory(row)])
                    with self.assertRaises(observe.Inconclusive) as raised:
                        controller.discover("B")
                    report = controller.diagnostics(raised.exception)
                    self.assertNotIn(
                        report["refusal_code"], {"UNCLASSIFIED_REFUSAL", "UNCLASSIFIED_EXCEPTION"}
                    )
                    self.assertEqual(report["discovery_accounting"]["global_delta"], 1)
                    self.assertEqual(report["discovery_accounting"]["reads_remaining"], 2)
                    self.assertTrue(controller.c["aborted"])
                    self.assertNotIn(TOKEN, json.dumps(report))
                    self.assertFalse(controller.e.state["runs"])
                self.root, self.custody = old_root, old_custody

    def test_malformed_duplicate_ambiguous_and_failed_list_are_charged(self) -> None:
        values = (
            b'{"total_count": 0, "total_count": 1}',
            b"not JSON " + TOKEN.encode(),
            self.inventory(REAL, REAL),
            self.inventory(REAL, {**REAL, "id": REAL["id"] + 1}),
            self.inventory({**REAL, "workflow_id": None}),
            OSError(TOKEN),
        )
        for value in values:
            with self.subTest(value=type(value).__name__), tempfile.TemporaryDirectory() as scratch:
                self.custody = Path(scratch) / "custody"
                with patch.object(observe, "REGISTRATION_ROOT", Path(scratch)):
                    controller = self.prepare()
                    fake = self.opener(controller, [value])
                    with self.assertRaises(Exception) as raised:
                        controller.discover("B")
                    report = controller.diagnostics(raised.exception)
                    self.assertEqual(report["discovery_accounting"]["global_delta"], 1)
                    self.assertEqual(report["discovery_accounting"]["reads_after"], 1)
                    self.assertEqual(fake.open.call_count, 1)
                    self.assertNotIn(TOKEN, json.dumps(report))
                    self.assertNotIn(TOKEN.encode(), (self.custody / "state.json").read_bytes())

    def test_exact_read_failure_keeps_binding_and_reports_two_spent(self) -> None:
        controller = self.prepare()
        self.opener(controller, [self.inventory(REAL), OSError(TOKEN)])
        with self.assertRaises(observe.Inconclusive) as raised:
            controller.discover("B")
        report = controller.diagnostics(raised.exception)
        self.assertEqual(report["phase_code"], "DISCOVERY_EXACT_TARGET_READ")
        self.assertEqual(report["discovery_accounting"]["global_delta"], 2)
        self.assertTrue(report["own_series_discovery_facts"]["target_bound"])
        self.assertFalse(report["own_series_discovery_facts"]["exact_target_authenticated"])
        self.assertEqual(controller.e.state["runs"]["B"]["id"], REAL["id"])

    def assert_stopped_discovery(
        self, controller: control.Controller, fake: Mock, report: dict[str, Any], spent: int
    ) -> None:
        self.assertEqual(report["discovery_accounting"]["global_delta"], spent)
        self.assertEqual(report["discovery_accounting"]["reads_delta"], 1)
        self.assertTrue(controller.c["aborted"])
        self.assertLessEqual(len(json.dumps(report).encode()), 8192)
        self.assertNotIn(TOKEN, json.dumps(report))
        for path in self.custody.rglob("*"):
            if path.is_file():
                self.assertNotIn(TOKEN.encode(), path.read_bytes())
        with self.assertRaises(observe.Inconclusive):
            controller.discover("B")
        with self.assertRaises(observe.Inconclusive):
            controller.dispatch("C")
        self.assertEqual(fake.open.call_count, spent)
        self.assertEqual(set(controller.c["effects"]), {"dispatch:B"})

    def test_invalid_calendar_server_timestamps_have_fixed_refusal_before_binding(self) -> None:
        for stamp in (
            "2026-13-08T17:39:03Z",
            "2026-11-31T17:39:03Z",
            "0000-11-08T17:39:03Z",
            "2025-02-29T17:39:03Z",
        ):
            with self.subTest(timestamp=stamp), tempfile.TemporaryDirectory() as scratch:
                self.custody = Path(scratch) / "custody"
                with patch.object(observe, "REGISTRATION_ROOT", Path(scratch)):
                    controller = self.prepare()
                    fake = self.opener(controller, [self.inventory({**REAL, "created_at": stamp})])
                    with self.assertRaises(observe.Inconclusive) as raised:
                        controller.discover("B")
                    report = controller.diagnostics(raised.exception)
                    self.assertEqual(report["refusal_code"], "SERVER_CREATION_TIMESTAMP_MALFORMED")
                    self.assertEqual(report["phase_code"], "DISPATCH_WINDOW")
                    self.assertFalse(report["discovery_accounting"]["window_matches"])
                    self.assertEqual(report["discovery_accounting"]["observed_created_at"], stamp)
                    self.assertFalse(controller.e.state["runs"])
                    self.assert_stopped_discovery(controller, fake, report, 1)

    def test_invalid_local_dispatch_timestamps_have_fixed_private_refusal(self) -> None:
        for field, stamp in (
            ("transport_started_utc", "2026-13-08T17:39:03+00:00"),
            ("after_utc", "2026-11-31T17:39:03+00:00"),
            ("after_utc", "0000-11-08T17:39:03+00:00"),
        ):
            with (
                self.subTest(field=field, timestamp=stamp),
                tempfile.TemporaryDirectory() as scratch,
            ):
                self.custody = Path(scratch) / "custody"
                with patch.object(observe, "REGISTRATION_ROOT", Path(scratch)):
                    controller = self.prepare()
                    # Corrupt the owned local record to exercise the calendar boundary.
                    controller.c["effects"]["dispatch:B"][field] = stamp
                    fake = self.opener(controller, [self.inventory(REAL)])
                    with self.assertRaises(observe.Inconclusive) as raised:
                        controller.discover("B")
                    report = controller.diagnostics(raised.exception)
                    self.assertEqual(report["refusal_code"], "DISPATCH_TIMESTAMP_MALFORMED")
                    self.assertEqual(report["phase_code"], "DISPATCH_WINDOW")
                    self.assertFalse(report["discovery_accounting"]["window_matches"])
                    self.assertFalse(controller.e.state["runs"])
                    self.assert_stopped_discovery(controller, fake, report, 1)

    def assert_exact_refusal(
        self, value: object, code: str
    ) -> tuple[dict[str, Any], dict[str, Any]]:
        controller = self.prepare()
        raw = value if isinstance(value, bytes) else json.dumps(value).encode()
        fake = self.opener(controller, [self.inventory(REAL), raw])
        with self.assertRaises(observe.Inconclusive) as raised:
            controller.discover("B")
        report = controller.diagnostics(raised.exception)
        self.assertEqual(report["refusal_code"], code)
        self.assertEqual(report["phase_code"], "DISCOVERY_EXACT_TARGET_READ")
        self.assertEqual(controller.e.state["runs"]["B"]["id"], REAL["id"])
        self.assertTrue(report["own_series_discovery_facts"]["target_bound"])
        self.assertFalse(report["own_series_discovery_facts"]["exact_target_authenticated"])
        accounting = report["discovery_accounting"]
        self.assertEqual(accounting["observed_identity_provenance"], "exact")
        inventory = accounting["identity_observations"]["inventory"]
        exact = accounting["identity_observations"]["exact"]
        self.assertEqual(inventory["provenance"], "inventory")
        self.assertEqual(inventory["processing_stage"], "DISCOVERY_INVENTORY_RESPONSE")
        self.assertEqual(inventory["identity"]["observed_source_sha"], S)
        self.assertEqual(inventory["identity"]["observed_attempt"], 1)
        self.assertEqual(exact["provenance"], "exact")
        self.assertEqual(exact["processing_stage"], "DISCOVERY_EXACT_TARGET_READ")
        self.assertEqual(exact["response"]["request_run_id"], REAL["id"])
        self.assertEqual(exact["response"]["sha256"], hashlib.sha256(raw).hexdigest())
        self.assertEqual(exact["response"]["bytes"], len(raw))
        reference = exact["evidence_ref"]
        summary = (self.custody / reference["path"]).read_bytes()
        self.assertEqual(reference["bytes"], len(summary))
        self.assertEqual(reference["sha256"], hashlib.sha256(summary).hexdigest())
        self.assertEqual(json.loads(summary), exact["response"])
        self.assert_stopped_discovery(controller, fake, report, 2)
        return accounting, exact

    def test_exact_wrong_source_and_attempt_replace_flat_inventory_observations(self) -> None:
        for field, value, code, observed in (
            ("head_sha", "b" * 40, "O041", "observed_source_sha"),
            ("run_attempt", 2, "O115", "observed_attempt"),
        ):
            with self.subTest(field=field), tempfile.TemporaryDirectory() as scratch:
                self.custody = Path(scratch) / "custody"
                with patch.object(observe, "REGISTRATION_ROOT", Path(scratch)):
                    accounting, exact = self.assert_exact_refusal(
                        {**REAL, field: value, "private_unused": TOKEN}, code
                    )
                    self.assertEqual(exact["json_state"], "object")
                    self.assertEqual(exact["identity"][observed], value)
                    self.assertEqual(accounting[observed], value)

    def test_exact_numeric_identity_impostors_refuse_without_integer_coercion(self) -> None:
        mutations = (
            ("run_attempt", True, "O115", "observed_attempt", "boolean"),
            (
                "run_attempt",
                1.0,
                "O115",
                "observed_attempt",
                "non-integer-number",
            ),
            (
                "id",
                float(REAL["id"]),
                "O115",
                "observed_run_id",
                "non-integer-number",
            ),
            (
                "workflow_id",
                float(REAL["workflow_id"]),
                "O147",
                "observed_workflow_id",
                "non-integer-number",
            ),
            (
                "actor",
                {**REAL["actor"], "id": float(REAL["actor"]["id"])},
                "O006",
                "observed_actor_id",
                "non-integer-number",
            ),
            (
                "triggering_actor",
                {**REAL["triggering_actor"], "id": float(REAL["triggering_actor"]["id"])},
                "O006",
                "observed_triggering_actor_id",
                "non-integer-number",
            ),
        )
        for field, value, code, observed, kind in mutations:
            with self.subTest(field=field, value=value), tempfile.TemporaryDirectory() as scratch:
                self.custody = Path(scratch) / "custody"
                with patch.object(observe, "REGISTRATION_ROOT", Path(scratch)):
                    accounting, exact = self.assert_exact_refusal(
                        {**REAL, field: value, "private_unused": TOKEN}, code
                    )
                    self.assertIsNone(exact["identity"][observed])
                    self.assertIsNone(accounting[observed])
                    self.assertEqual(exact["identity"][observed + "_type"], kind)

    def test_exact_invalid_json_and_nonobjects_keep_only_predecode_hash_and_safe_facts(
        self,
    ) -> None:
        rows = [
            (b"not JSON " + TOKEN.encode(), "O055", "unparsed"),
            (
                b'{"id": 1, "id": 2, "private": ' + json.dumps(TOKEN).encode() + b"}",
                "O037",
                "unparsed",
            ),
            (json.dumps([TOKEN]).encode(), "O102", "nonobject"),
            (json.dumps(TOKEN).encode(), "O102", "nonobject"),
            (b"null", "O102", "nonobject"),
        ]
        for constant in ("NaN", "Infinity", "-Infinity", "1e309", "-1e309"):
            row = json.dumps({**REAL, "private_unused": TOKEN})[:-1]
            rows.append(
                (
                    (row + ', "unused_number": ' + constant + "}").encode(),
                    "O055",
                    "unparsed",
                )
            )
        # CPython's C JSON parser can exceed the Python frame recursion limit.
        depth = max(10_000, sys.getrecursionlimit() + 100)
        nested = (
            json.dumps({**REAL, "private_unused": TOKEN})[:-1]
            + ', "unused_nested": '
            + "[" * depth
            + "0"
            + "]" * depth
            + "}"
        ).encode()
        self.assertLess(len(nested), observe.MAX_JSON)
        rows.append((nested, "O055", "unparsed"))
        for raw, code, state in rows:
            with (
                self.subTest(code=code, state=state, sha=hashlib.sha256(raw).hexdigest()),
                tempfile.TemporaryDirectory() as scratch,
            ):
                self.custody = Path(scratch) / "custody"
                with patch.object(observe, "REGISTRATION_ROOT", Path(scratch)):
                    accounting, exact = self.assert_exact_refusal(raw, code)
                    self.assertEqual(exact["json_state"], state)
                    self.assertIsNone(exact["identity"]["observed_source_sha"])
                    self.assertIsNone(accounting["observed_attempt"])
        self.assertEqual(observe.decode(b'{"elapsed": 1.25}')["elapsed"], 1.25)
        self.assertEqual(observe.decode(b'{"elapsed": -1.25e2}')["elapsed"], -125.0)

    def test_two_normal_and_one_diagnostic_exhaust_shared_three_read_allowance(self) -> None:
        controller = self.prepare()
        fake = self.opener(controller, [self.inventory(), self.inventory(), self.inventory(REAL)])
        self.assertFalse(controller.discover("B"))
        self.clock += 5
        self.assertFalse(controller.discover("B"))
        self.clock += 5
        self.assertTrue(controller.discover("B", diagnostic=True))
        self.assertEqual(controller.e.state["requests"], 3)
        self.assertEqual(controller.e.state["discovery_reads"]["B"], 3)
        self.assertFalse(controller.e.state["runs"])
        self.assertTrue(controller.c["aborted"])
        with self.assertRaises(observe.Inconclusive) as raised:
            controller.discover("B", diagnostic=True)
        report = controller.diagnostics(raised.exception)
        self.assertEqual(report["discovery_accounting"]["global_delta"], 0)
        self.assertEqual(report["discovery_accounting"]["reads_delta"], 0)
        self.assertEqual(report["discovery_accounting"]["reads_remaining"], 0)
        self.assertEqual(fake.open.call_count, 3)
        with self.assertRaises(observe.Inconclusive):
            controller.discover("B")
        with self.assertRaises(observe.Inconclusive):
            controller.dispatch("C")
        self.assertEqual(fake.open.call_count, 3)
        self.assertEqual(set(controller.c["effects"]), {"dispatch:B"})

    def test_failed_transport_then_diagnostic_consumes_slot_without_normal_resume(self) -> None:
        controller = self.prepare()
        fake = self.opener(controller, [OSError(TOKEN), self.inventory(REAL)])
        with self.assertRaises(observe.Inconclusive):
            controller.discover("B")
        with self.assertRaises(observe.Inconclusive) as raised:
            controller.discover("B")
        self.assertEqual(
            controller.diagnostics(raised.exception)["refusal_code"], "DISCOVERY_ABORTED"
        )
        self.clock += 5
        self.assertTrue(controller.discover("B", diagnostic=True))
        self.assertEqual(controller.e.state["discovery_reads"]["B"], 2)
        self.assertEqual(fake.open.call_count, 2)
        self.assertFalse(controller.e.state["runs"])

    def test_offsets_399_to_400_then_zero_http_and_deadline_no_http(self) -> None:
        controller = self.prepare()
        self.spent(controller, 399)
        fake = self.opener(controller, [self.inventory()])
        self.assertFalse(controller.discover("B"))
        report = controller.diagnostics()["discovery_accounting"]
        self.assertEqual(
            (report["global_before"], report["global_after"], report["global_delta"]), (399, 400, 1)
        )
        self.clock += 5
        with self.assertRaises(observe.Inconclusive) as raised:
            controller.discover("B", diagnostic=True)
        self.assertEqual(
            controller.diagnostics(raised.exception)["discovery_accounting"]["global_delta"], 0
        )
        self.assertEqual(fake.open.call_count, 1)
        with self.assertRaises(observe.Inconclusive):
            controller.dispatch("C")
        self.clock = 1901
        with self.assertRaises(observe.Inconclusive):
            controller.e.transport(
                "https://api.github.com/repos/Strukturpiloten/boxferry/actions/runs/1",
                authorization=True,
                limit=64,
                purpose="offline deadline",
            )
        self.assertEqual(fake.open.call_count, 1)

    def test_cadence_refusal_spends_discovery_slot_but_no_transport(self) -> None:
        controller = self.prepare()
        fake = self.opener(controller, [self.inventory()])
        self.assertFalse(controller.discover("B"))
        with self.assertRaises(observe.Inconclusive) as raised:
            controller.discover("B")
        report = controller.diagnostics(raised.exception)
        self.assertEqual(report["discovery_accounting"]["global_delta"], 0)
        self.assertEqual(report["discovery_accounting"]["reads_after"], 2)
        self.assertEqual(fake.open.call_count, 1)

    def test_closed_structural_codes_and_unknown_exception_are_private(self) -> None:
        for key in control.STRUCTURAL_KEYS:
            with self.subTest(key=key), self.assertRaises(observe.Inconclusive) as raised:
                observe.object_field({key: TOKEN}, key)
            self.assertEqual(control.refusal_code(raised.exception), "STRUCTURE_" + key.upper())
        self.assertEqual(control.refusal_code(observe.Inconclusive(TOKEN)), "UNCLASSIFIED_REFUSAL")
        self.assertEqual(control.refusal_code(RuntimeError(TOKEN)), "UNCLASSIFIED_EXCEPTION")


class InitializationTests(OfflineCase):
    def preflight_values(self) -> list[object]:
        workflows = self.descriptor["workflows"]
        blobs = [
            {
                "sha": value["blob"],
                "encoding": "base64",
                "size": len((source.ROOT / path).read_bytes()),
                "content": base64.b64encode((source.ROOT / path).read_bytes()).decode(),
            }
            for path, value in workflows.items()
        ]
        return [
            REAL["actor"],
            {"full_name": observe.REPO, "default_branch": "main"},
            {"ref": "refs/heads/main", "object": {"type": "commit", "sha": S}},
            {
                "id": REAL["workflow_id"],
                "path": observe.WORKFLOW,
                "name": "629 recovery cancellation provider probe",
                "state": "active",
            },
            {"id": 999, "path": ".github/workflows/ci.yml"},
            {
                "id": 123,
                "workflow_id": 999,
                "head_sha": S,
                "head_branch": "main",
                "event": "push",
                "status": "completed",
                "conclusion": "success",
                "repository": {"full_name": observe.REPO},
            },
            {"sha": S, "tree": {"sha": "a" * 40}},
            {
                "sha": "a" * 40,
                "truncated": False,
                "tree": [{"path": ".github", "type": "tree", "mode": "040000", "sha": "b" * 40}],
            },
            {
                "sha": "b" * 40,
                "truncated": False,
                "tree": [{"path": "workflows", "type": "tree", "mode": "040000", "sha": "c" * 40}],
            },
            {
                "sha": "c" * 40,
                "truncated": False,
                "tree": [
                    {
                        "path": Path(path).name,
                        "type": "blob",
                        "mode": "100644",
                        "sha": value["blob"],
                    }
                    for path, value in workflows.items()
                ],
            },
            *blobs,
            self.inventory(),
        ]

    def test_actual_initialize_zero_http_and_preflight_fourteen_precharged_gets(self) -> None:
        controller = self.initialize()
        fake = self.opener(controller, self.preflight_values())
        self.assertEqual(fake.open.call_count, 0)
        controller.preflight(123)
        self.assertEqual(controller.e.state["requests"], 14)
        self.assertEqual(fake.open.call_count, 14)
        self.assertTrue(controller.c["preflight"])
        self.assertTrue(all(path.startswith(control.API) for path in self.calls))
        self.assertEqual(len(controller.e.state["transport"]), 14)

    def test_general_inventory_routes_cannot_bypass_shared_case_reservation(self) -> None:
        controller = self.initialize()
        fake = self.opener(controller, self.preflight_values())
        controller.preflight(123)
        count = fake.open.call_count
        path = control.PREFIX + "/actions/workflows/411281080/runs?per_page=100&page=1"
        with self.assertRaises(observe.Inconclusive):
            controller.e.api("actions/workflows/411281080/runs?per_page=100&page=1", "bypass")
        with self.assertRaises(observe.Inconclusive):
            controller.get(path)
        with self.assertRaises(observe.Inconclusive):
            controller.run_list()
        with self.assertRaises(observe.Inconclusive):
            controller.preflight(123)
        self.assertEqual(fake.open.call_count, count)
        self.assertEqual(controller.e.state["requests"], 14)
        self.assertFalse(controller.e.state["discovery_reads"])

    def test_copied_custody_is_read_only_and_cannot_fork_the_shared_ledger(self) -> None:
        controller = self.initialize()
        import shutil

        copied = self.root / "copy"
        shutil.copytree(self.custody, copied)
        with self.assertRaises(observe.Inconclusive):
            observe.Evidence(copied)
        proof = observe.Evidence(copied, read_only=True)
        with self.assertRaises(observe.Inconclusive):
            proof.transport(
                "https://api.github.com/repos/Strukturpiloten/boxferry/actions/runs/1",
                authorization=True,
                limit=64,
                purpose="offline copied proof",
            )
        self.assertEqual(proof.state["requests"], 0)
        self.assertEqual(controller.e.state["requests"], 0)

    def test_e_binding_uses_new_exact_package_schema_and_refuses_old_controller(self) -> None:
        controller = self.initialize()
        self.assertEqual(observe.e_controller(controller.e)["sha256"], controller.source_hash)
        self.assertEqual(
            observe.e_source(controller.e),
            hashlib.sha256(observe.e_bytes(self.descriptor)).hexdigest(),
        )
        controller.c["schema"] = "boxferry.provider632.controller.v5"
        with self.assertRaises(observe.Inconclusive):
            observe.e_controller(controller.e)

    def test_seed_charge_old_custody_and_reset_are_rejected(self) -> None:
        for option in ("--preflight-requests", "--preflight-started-utc"):
            with (
                self.subTest(option=option),
                contextlib.redirect_stderr(io.StringIO()),
                contextlib.redirect_stdout(io.StringIO()),
                self.assertRaises(SystemExit),
            ):
                cli.main([*self.init_arguments(), option, "1"])
        controller = self.initialize()
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            observe.parser().parse_args(
                ["charge", "--custody", str(self.custody), "--requests", "1"]
            )
        with self.assertRaises(observe.Inconclusive):
            observe.initialize(observe.parser().parse_args(self.init_arguments()))
        controller.e.state["schema"] = "boxferry.provider632.observer.v5"
        controller.e.persist()
        with self.assertRaises(observe.Inconclusive):
            observe.Evidence(self.custody)

    def test_manual_bulk_charge_and_exact_hard_deadline_cannot_attempt_transport(self) -> None:
        controller = self.initialize()
        with self.assertRaises(observe.Inconclusive):
            controller.e.charge(2, "manual seed refused")
        self.assertEqual(controller.e.state["requests"], 0)
        self.clock = 1900
        fake = self.opener(controller, [])
        with self.assertRaises(observe.Inconclusive):
            controller.e.transport(
                control.API + control.PREFIX + "/actions/runs/1",
                authorization=True,
                limit=64,
                purpose="deadline",
            )
        self.assertEqual(fake.open.call_count, 0)
        self.assertEqual(controller.e.state["requests"], 0)

    def test_preflight_failure_is_spent_and_abort_prevents_dispatch(self) -> None:
        controller = self.initialize()
        fake = self.opener(controller, [OSError(TOKEN)])
        with contextlib.redirect_stdout(io.StringIO()) as output:
            with patch.object(control, "Controller", return_value=controller):
                self.assertEqual(
                    control.main(
                        ["--custody", str(self.custody), "preflight", "--main-ci-run", "123"]
                    ),
                    2,
                )
        self.assertEqual(controller.e.state["requests"], 1)
        self.assertTrue(controller.c["aborted"])
        self.assertEqual(fake.open.call_count, 1)
        self.assertNotIn(TOKEN, output.getvalue())

    def test_descriptor_and_package_source_refuse_without_http_or_implicit_fetch(self) -> None:
        with self.assertRaises(ValueError):
            source.verify_descriptor(self.descriptor, self.raw, "0" * 64)
        value = copy.deepcopy(self.descriptor)
        value["implementation"][source.PACKAGE[0]]["mode"] = "100755"
        raw = source.encoded(value)
        with self.assertRaises(ValueError):
            source.verify_descriptor(value, raw, hashlib.sha256(raw).hexdigest())
        with patch.object(source, "git", side_effect=ValueError("held object absent")):
            with self.assertRaises(ValueError):
                source.describe(S, 123)
        process = Mock(returncode=1, stdout=b"", stderr=TOKEN.encode())
        with patch.object(source.subprocess, "run", return_value=process) as call:
            # Call original fixed local transport, not this test's held-object stub.
            with self.assertRaises(ValueError):
                LOCAL_GIT("cat-file", "blob", S + ":" + source.PACKAGE[0])
            environment = call.call_args.kwargs["env"]
            self.assertEqual(environment["GIT_NO_LAZY_FETCH"], "1")
            self.assertEqual(environment["GIT_ALLOW_PROTOCOL"], "")
            self.assertIn("protocol.allow=never", call.call_args.args[0])


class RefusalSafetyTests(OfflineCase):
    def test_actual_observe_malformed_job_name_and_interruption_abort_later_controls(self) -> None:
        for interruption in (False, True):
            with self.subTest(interruption=interruption), tempfile.TemporaryDirectory() as scratch:
                self.custody = Path(scratch) / "custody"
                with patch.object(observe, "REGISTRATION_ROOT", Path(scratch)):
                    controller = self.prepare()
                    controller.e.state["runs"]["B"] = {"id": REAL["id"]}
                    controller.c["discovered"]["B"] = REAL["id"]
                    controller.e.persist()
                    invalid_job = {
                        "id": 11,
                        "run_id": REAL["id"],
                        "run_attempt": 1,
                        "head_sha": S,
                        "name": [TOKEN],
                    }
                    values = (
                        [KeyboardInterrupt()]
                        if interruption
                        else [
                            REAL,
                            {"total_count": 1, "jobs": [invalid_job]},
                            {"total_count": 0, "artifacts": []},
                        ]
                    )
                    fake = self.opener(controller, values)
                    with patch.object(observe, "Evidence", return_value=controller.e):
                        with contextlib.redirect_stdout(io.StringIO()) as output:
                            self.assertEqual(
                                cli.main(
                                    [
                                        "observe",
                                        "snapshot",
                                        "--custody",
                                        str(self.custody),
                                        "--case",
                                        "B",
                                        "--run",
                                        str(REAL["id"]),
                                        "--label",
                                        "malformed",
                                    ]
                                ),
                                2,
                            )
                    self.assertNotIn(TOKEN, output.getvalue())
                    self.assertEqual(controller.e.state["requests"], 1 if interruption else 3)
                    self.assertTrue(controller.c["aborted"])
                    self.assertTrue(controller.e.state["normal_aborted"])
                    before = fake.open.call_count
                    fresh = control.Controller(observe, self.custody)
                    fresh.opener = fake
                    with self.assertRaises(observe.Inconclusive):
                        fresh.dispatch("C")
                    self.assertEqual(fake.open.call_count, before)
                    self.assertFalse(
                        any(row.get("method") == "POST" for row in fresh.c["effects"].values())
                    )

    def test_failed_abort_persistence_leaves_marker_that_next_load_cannot_ignore(self) -> None:
        controller = self.prepare()
        controller.e.state["runs"]["B"] = {"id": REAL["id"]}
        controller.e.persist()
        fake = self.opener(controller, [KeyboardInterrupt()])
        original = controller.e.persist

        def persist() -> None:
            if controller.e.state.get("normal_aborted"):
                raise OSError(TOKEN)
            original()

        with patch.object(observe, "Evidence", return_value=controller.e):
            with patch.object(controller.e, "persist", side_effect=persist):
                with contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(
                        cli.main(
                            [
                                "observe",
                                "snapshot",
                                "--custody",
                                str(self.custody),
                                "--case",
                                "B",
                                "--run",
                                str(REAL["id"]),
                                "--label",
                                "interrupt",
                            ]
                        ),
                        2,
                    )
        self.assertTrue((self.custody / "incomplete-observation.json").is_file())
        fresh = control.Controller(observe, self.custody)
        self.assertTrue(fresh.c["aborted"])
        self.assertEqual(fresh.e.state["requests"], 1)
        with self.assertRaises(observe.Inconclusive):
            fresh.dispatch("C")
        self.assertEqual(fake.open.call_count, 1)

    def test_custody_symlink_or_ancestor_is_refused_before_any_lock_registration_or_write(
        self,
    ) -> None:
        outside = self.root / "unrelated"
        outside.mkdir()
        (outside / "keep").write_text("preserve")
        linked = self.root / "linked"
        linked.symlink_to(outside, target_is_directory=True)
        for custody in (linked, linked / "new-custody"):
            arguments = self.init_arguments()
            arguments[arguments.index("--custody") + 1] = str(custody)
            with self.subTest(custody=str(custody)), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(cli.main(arguments), 2)
                self.assertEqual(cli.main(["observe", "status", "--custody", str(custody)]), 2)
            self.assertEqual(sorted(path.name for path in outside.iterdir()), ["keep"])
            self.assertEqual((outside / "keep").read_text(), "preserve")
            self.assertFalse(list(self.root.glob("provider632-series-*")))

    def test_executed_package_init_is_part_of_exact_held_closure(self) -> None:
        paths = (*source.PACKAGE, *source.WORKFLOWS)
        original = {path: (source.ROOT / path).read_bytes() for path in paths}
        copied = self.root / "held"
        for path, data in original.items():
            destination = copied / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(data)

        def held(*args: str) -> bytes:
            if args == ("rev-parse", "HEAD"):
                return S.encode()
            path = args[-1].split(":", 1)[-1]
            data = original[path]
            if args[0] == "ls-tree":
                blob = hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()
                return f"100644 blob {blob}\t{path}\n".encode()
            return str(len(data)).encode() if args[1] == "-s" else data

        with patch.object(source, "ROOT", copied), patch.object(source, "git", side_effect=held):
            descriptor = source.describe(S, 123)
            initializer = "automation/src/boxferry_ci/__init__.py"
            self.assertIn(initializer, descriptor["implementation"])
            raw = source.encoded(descriptor)
            self.manifest.write_bytes(raw)
            (copied / initializer).write_bytes(original[initializer] + b"\n# authored dirty init\n")
            with self.assertRaises(ValueError):
                source.describe(S, 123)
            arguments = self.init_arguments()
            arguments[arguments.index("--descriptor-sha256") + 1] = hashlib.sha256(raw).hexdigest()
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(cli.main(arguments), 2)
        self.assertFalse(self.custody.exists())


class TerminalCaptureTests(OfflineCase):
    def test_actual_cli_active_first_completed_full_then_one_stable_and_no_third_get(self) -> None:
        controller = self.prepare()
        controller.e.state["runs"]["B"] = {"id": REAL["id"]}
        controller.c["discovered"]["B"] = REAL["id"]
        controller.e.persist()
        active = {**REAL, "status": "in_progress", "conclusion": None}
        jobs = {"total_count": 0, "jobs": []}
        artifacts = {"total_count": 0, "artifacts": []}
        fake = self.opener(controller, [active, REAL, jobs, artifacts, REAL, jobs, artifacts])

        def capture(label: str) -> tuple[int, dict[str, Any]]:
            with patch.object(observe, "Evidence", return_value=controller.e):
                with contextlib.redirect_stdout(io.StringIO()) as output:
                    code = cli.main(
                        [
                            "observe",
                            "capture-terminal",
                            "--custody",
                            str(self.custody),
                            "--case",
                            "B",
                            "--run",
                            str(REAL["id"]),
                            "--label",
                            label,
                        ]
                    )
            return code, json.loads(output.getvalue())

        code, result = capture("B-sample-1")
        self.assertEqual(code, 0)
        self.assertEqual(result["outcome"], "NOT_YET_OBSERVED")
        self.assertEqual(controller.e.state["requests"], 1)
        self.assertFalse(controller.e.state["snapshots"])
        first_bytes = {
            path.name: path.read_bytes()
            for path in self.custody.iterdir()
            if path.is_file() and path.name not in {"state.json", "observer.lock"}
        }
        self.clock += 5
        code, result = capture("B-sample-2")
        self.assertEqual(code, 0)
        self.assertEqual(result["terminal_capture"], "B-terminal")
        self.assertEqual(controller.e.state["requests"], 4)
        terminal = copy.deepcopy(controller.e.state["snapshots"]["B-terminal"])
        self.clock += 5
        code, result = capture("B-sample-3")
        self.assertEqual(code, 0)
        self.assertEqual(result["terminal_capture"], "B-terminal-stable")
        self.assertEqual(controller.e.state["requests"], 7)
        self.assertEqual(controller.e.state["snapshots"]["B-terminal"], terminal)
        self.assertEqual(
            controller.e.state["terminal_captures"]["B-terminal"]["sample_label"], "B-sample-2"
        )
        self.assertEqual(
            sum(
                "/actions/runs/42506013197" == path.removeprefix(control.API + control.PREFIX)
                for path in self.calls
            ),
            3,
        )
        code, result = capture("B-sample-4")
        self.assertEqual(code, 2)
        self.assertEqual(result["refusal_code"], "TERMINAL_ALREADY_STABLE")
        self.assertEqual(fake.open.call_count, 7)
        self.assertEqual(controller.e.state["requests"], 7)
        for name, data in first_bytes.items():
            self.assertEqual((self.custody / name).read_bytes(), data)

    def test_actual_cli_old_or_wrong_helper_custody_is_unchanged_before_lock(self) -> None:
        for schema, lock in (
            ("boxferry.provider632.observer.v5", False),
            ("boxferry.provider632.observer.v5", True),
            (observe.OBSERVER_SCHEMA, False),
        ):
            with self.subTest(schema=schema, lock=lock), tempfile.TemporaryDirectory() as scratch:
                old = Path(scratch) / "old"
                old.mkdir()
                (old / "state.json").write_text(
                    json.dumps({"schema": schema, "observer_sha256": "0" * 64})
                )
                if lock:
                    (old / "observer.lock").write_bytes(b"immutable old lock")
                before = {
                    path.name: (path.read_bytes(), path.stat().st_mode) for path in old.iterdir()
                }
                mode = old.stat().st_mode
                for arguments in (
                    ["observe", "status", "--custody", str(old)],
                    ["control", "--custody", str(old), "preflight", "--main-ci-run", "123"],
                ):
                    with contextlib.redirect_stdout(io.StringIO()):
                        self.assertEqual(cli.main(arguments), 2)
                    after = {
                        path.name: (path.read_bytes(), path.stat().st_mode)
                        for path in old.iterdir()
                    }
                    self.assertEqual(after, before)
                    self.assertEqual(old.stat().st_mode, mode)


class TransportTests(OfflineCase):
    def test_all_three_inventory_pages_are_precharged(self) -> None:
        controller = self.prepare()
        controller.e.state["runs"]["B"] = {"id": REAL["id"]}
        pages = [
            {
                "total_count": 3,
                "jobs": [{"id": number, "run_id": REAL["id"], "run_attempt": 1, "head_sha": S}],
            }
            for number in (11, 12, 13)
        ]
        fake = self.opener(controller, pages)
        rows = controller.e.inventory("B", REAL["id"], "jobs", "pages")
        self.assertEqual([row["id"] for row in rows], [11, 12, 13])
        self.assertEqual(controller.e.state["requests"], 3)
        self.assertEqual(fake.open.call_count, 3)
        self.assertTrue(all(f"page={index}" in path for index, path in enumerate(self.calls, 1)))

    def test_pagination_changes_duplicates_empty_and_cap_refuse_at_charged_count(self) -> None:
        row = {"id": 11, "run_id": REAL["id"], "run_attempt": 1, "head_sha": S}
        cases = (
            ([{"total_count": 3, "jobs": [row]}, {"total_count": 2, "jobs": [row]}], 2),
            ([{"total_count": 3, "jobs": [row]}, {"total_count": 3, "jobs": [row]}], 2),
            ([{"total_count": 3, "jobs": []}], 1),
            ([{"total_count": 101, "jobs": []}], 1),
        )
        for pages, count in cases:
            with self.subTest(count=count), tempfile.TemporaryDirectory() as scratch:
                self.custody = Path(scratch) / "custody"
                with patch.object(observe, "REGISTRATION_ROOT", Path(scratch)):
                    controller = self.prepare()
                    controller.e.state["runs"]["B"] = {"id": REAL["id"]}
                    self.opener(controller, pages)
                    with self.assertRaises(observe.Inconclusive):
                        controller.e.inventory("B", REAL["id"], "jobs", "pages")
                    self.assertEqual(controller.e.state["requests"], count)

    def test_metadata_and_redirect_download_hops_total_four_without_forwarded_auth(self) -> None:
        controller = self.prepare()
        controller.e.state["runs"]["B"] = {"id": REAL["id"]}
        requests: list[Any] = []

        def opened(request: urllib.request.Request, timeout: float) -> Response:
            requests.append(request)
            self.assertEqual(controller.e.state["requests"], len(requests))
            if len(requests) == 1:
                return Response({"id": 70})
            if len(requests) < 4:
                raise redirect_error(
                    request.full_url, "https://artifact.example.test/data?signed=secret"
                )
            return Response(b"ZIP")

        controller.e.opener = Mock()
        controller.e.opener.open.side_effect = opened
        controller.e.api("actions/runs/42506013197/artifacts?per_page=100&page=1", "metadata")
        self.assertEqual(
            controller.e.download_bytes("actions/artifacts/70/zip", 256, "archive", REAL["id"]),
            b"ZIP",
        )
        self.assertEqual(controller.e.state["requests"], 4)
        self.assertIsNotNone(requests[0].get_header("Authorization"))
        self.assertIsNotNone(requests[1].get_header("Authorization"))
        self.assertTrue(
            all(request.get_header("Authorization") is None for request in requests[2:])
        )
        self.assertNotIn("signed=secret", (self.custody / "state.json").read_text())

    def test_unsafe_and_excess_redirects_never_open_an_extra_hop(self) -> None:
        for destination, expected in (
            ("http://private.invalid/a", 1),
            ("https://artifact.example.test/a", 5),
        ):
            with self.subTest(destination=destination), tempfile.TemporaryDirectory() as scratch:
                self.custody = Path(scratch) / "custody"
                with patch.object(observe, "REGISTRATION_ROOT", Path(scratch)):
                    controller = self.prepare()
                    opened = Mock(
                        side_effect=lambda request, timeout, destination=destination: (
                            _ for _ in ()
                        ).throw(redirect_error(request.full_url, destination))
                    )
                    controller.e.opener = Mock(open=opened)
                    with self.assertRaises(observe.Inconclusive):
                        controller.e.download_bytes(
                            "actions/artifacts/70/zip", 256, "archive", REAL["id"]
                        )
                    self.assertEqual(opened.call_count, expected)
                    self.assertEqual(controller.e.state["requests"], expected)

    def test_transport_ambiguity_spends_effect_and_never_retries(self) -> None:
        controller = self.prepare("A")
        controller.c["effects"].clear()
        fake = self.opener(controller, [OSError(TOKEN)])
        with self.assertRaises(observe.Inconclusive):
            controller.effect(
                "dispatch:A",
                "A",
                control.PREFIX + "/actions/workflows/411281080/dispatches",
                {"ref": "main"},
                204,
                "fixture-dispatch",
            )
        self.assertEqual(
            controller.c["effects"]["dispatch:A"]["state"], "uncertain-or-refused-no-retry"
        )
        self.assertTrue(controller.c["aborted"])
        with self.assertRaises(observe.Inconclusive):
            controller.effect(
                "dispatch:A",
                "A",
                control.PREFIX + "/actions/workflows/411281080/dispatches",
                {"ref": "main"},
                204,
                "fixture-dispatch",
            )
        self.assertEqual(controller.e.state["requests"], 1)
        self.assertEqual(fake.open.call_count, 1)


if __name__ == "__main__":
    unittest.main()


class CutoffTests(OfflineCase):
    def test_normal_dispatch_371_allowed_372_refused_and_1200_refused(self) -> None:
        controller = self.initialize()
        controller.c["preflight"] = True
        reached = RuntimeError("authored local marker; no HTTP")
        for spent, elapsed, allowed in ((371, 1199, True), (372, 1199, False), (0, 1200, False)):
            with self.subTest(spent=spent, elapsed=elapsed):
                self.spent(controller, spent)
                self.clock = 100 + elapsed
                with patch.object(controller, "head", side_effect=reached) as head:
                    expected = RuntimeError if allowed else observe.Inconclusive
                    with self.assertRaises(expected):
                        controller.dispatch("A")
                    self.assertEqual(head.call_count, int(allowed))
        self.assertFalse(controller.c["effects"])

    def test_normal_cancel_370_allowed_371_refused_and_hard_exhaustion_blocks_cleanup(self) -> None:
        controller = self.prepare("D")
        controller.e.state["runs"]["D"] = {"id": REAL["id"]}
        controller.c["discovered"]["D"] = REAL["id"]
        for spent, elapsed, allowed in ((370, 1199, True), (371, 1199, False), (0, 1200, False)):
            with self.subTest(spent=spent, elapsed=elapsed):
                self.spent(controller, spent)
                self.clock = 100 + elapsed
                with patch.object(
                    controller, "head", side_effect=RuntimeError("local marker")
                ) as head:
                    expected = RuntimeError if allowed else observe.Inconclusive
                    with self.assertRaises(expected):
                        controller.cancel("D", REAL["id"], False, None)
                    self.assertEqual(head.call_count, int(allowed))
        self.clock = 100
        self.spent(controller, 400)
        fake = self.opener(controller, [])
        with self.assertRaises(observe.Inconclusive):
            controller.cancel("D", REAL["id"], True, None, "budget")
        self.assertEqual(fake.open.call_count, 0)
        self.assertNotIn("cancel:D", controller.c["effects"])

    def test_d_requires_positive_failed_preparation_not_generic_cancellation(self) -> None:
        controller = self.prepare("D")
        controller.c["effects"]["cancel:D"] = {"state": "accepted"}
        controller.e.state["snapshots"]["D-terminal"] = {"jobs": [], "artifacts": []}
        failed = {
            "job": 1,
            "check": 2,
            "catalog_name_shape": ["prepare", "expanded-leaf"],
            "job_record": {"status": "completed", "conclusion": "cancelled"},
            "check_record": {"status": "completed", "conclusion": "cancelled"},
        }
        with patch.object(controller, "snapshot", return_value={}):
            with patch.object(controller, "observation", return_value=failed):
                controller.dispatch_guard("E")
            for field in ("job_record", "check_record"):
                invalid = {**failed, field: {"status": "completed", "conclusion": "success"}}
                with (
                    self.subTest(field=field),
                    patch.object(controller, "observation", return_value=invalid),
                ):
                    with self.assertRaises(observe.Inconclusive):
                        controller.dispatch_guard("E")
