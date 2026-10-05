#!/usr/bin/env python3
"""Offline controls: never launch a daemon or claim compatibility evidence."""

from __future__ import annotations

import copy
import hashlib
import importlib.util
import io
import json
import os
import pathlib
import shlex
import signal
import stat
import subprocess
import sys
import tempfile
import time
import unittest
from contextlib import ExitStack, contextmanager, redirect_stderr
from types import SimpleNamespace
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parent.parent
SOURCE = ROOT / "scripts/lib/readiness-comparison.py"
SPEC = importlib.util.spec_from_file_location("readiness_comparison", SOURCE)
assert SPEC is not None and SPEC.loader is not None
comparison = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(comparison)
CID = "a" * 64


class FiniteDelayedClose:
    """Delay an actual owned FileIO close once, retaining its physical proof."""
    def __init__(self, stream, delay):
        self.stream, self.delay = stream, delay

    @property
    def closed(self):
        return self.stream.closed

    def fileno(self):
        return self.stream.fileno()

    def close(self):
        if not self.stream.closed:
            time.sleep(self.delay)
        self.stream.close()


def native_readiness_layout() -> bytes:
    """Independently authored source-shape data, not an oracle implementation.

    Only the reviewed command literal and its two syntactic call-site envelopes
    are represented. This is deliberately not executable native runner code;
    no launcher, permissions, daemon checks or other oracle logic is copied.
    """
    command = b'curl -q --noproxy \'*\' -fs --max-time 5 --unix-socket "$socket" http://localhost/_ping >/dev/null'
    return b"\n".join((b"deadline=$((SECONDS + 360))", b"while (( SECONDS < deadline )); do",
                      b"    if " + command + b"; then break; fi", b"done",
                      b"[[ -S $socket ]] && " + command + b" || {", b""))


class ComparisonTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="boxferry-readiness-offline-")
        self.addCleanup(self.temporary.cleanup)
        self.directory = pathlib.Path(self.temporary.name)
        self.directory.chmod(0o700)
        self.value = comparison.initialize(SimpleNamespace(directory=self.directory, native_root=ROOT))
        self.value.update(prerequisite="ready", namespace=[5, 12345], origin=100000)

    def box(self, record="7 000 connect-error-observed completed absent", phase="baseline", elapsed=1000, wrapper=0):
        with mock.patch.object(comparison, "elapsed", return_value=elapsed):
            absolute = self.value["origin"] + elapsed
            handoff = f"{absolute // 1000}.{absolute % 1000 // 10:02d}"
            comparison.box_observation(self.value, phase, record, wrapper, handoff)

    def native(self, code=0, collector="completed", signal="absent", elapsed=1000):
        def collect(argv, deadline, **kwargs):
            self.assertEqual(argv, ["/bin/sh", "-c", comparison.NATIVE_COMMAND, "comparison-native", "/private/socket"])
            self.assertGreater(deadline, 10)
            kwargs["teardown_observations"]["signal"] = signal
            return collector, code, b"DO-NOT-PRINT/response", b"DO-NOT-PRINT/error"
        with mock.patch.object(comparison, "endpoint"), \
                mock.patch.object(comparison, "elapsed", side_effect=[1, elapsed]), \
                mock.patch.object(comparison.time, "monotonic", return_value=10), \
                mock.patch.object(comparison.contract.native_read, "native_poll_read", side_effect=collect):
            self.value["socket"] = {"native_mode": 0o666}
            return comparison.native_poll(self.value, "native-before", pathlib.Path("/private/socket"))

    def test_actual_box_failure_retained_after_later_ready(self):
        self.box()
        failure = copy.deepcopy(self.value["consumer"]["baseline"])
        self.value["socket"] = {"native_mode": 0o666}
        self.box(phase="harmonized", elapsed=1200)
        harmonized_failure = copy.deepcopy(self.value["consumer"]["first_harmonized_failure"])
        self.box("0 200 unknown completed denied", phase="harmonized", elapsed=1500)
        self.assertEqual(self.value["consumer"]["baseline"], failure)
        self.assertEqual(self.value["consumer"]["first_failure"], failure)
        self.assertEqual(self.value["consumer"]["first_harmonized_failure"], harmonized_failure)
        self.assertEqual(self.value["consumer"]["first_ready_ms"], 1500)

    def test_unknown_collectors_and_wrapper_failures_are_not_route_failures(self):
        for record, status in (("7 000 unknown invalid-output absent", 0),
                               ("7 000 unknown completed unknown", 0),
                               ("7 000 unknown completed absent", 124),
                               ("7 000 unknown termination-unverified denied", 0)):
            with self.subTest(record=record):
                self.setUp()
                with self.assertRaises(comparison.Refused):
                    self.box(record, wrapper=status)
                self.assertTrue(self.value["uncertain"])
                self.assertIsNone(self.value["consumer"]["first_failure"])
                self.assertEqual(comparison.classify(self.value), "indeterminate")

    def test_closed_parser_refuses_malformed_duplicate_or_injected_fields(self):
        for record in ("0 200 unknown completed absent extra", "0 private unknown completed absent",
                       "0 200 secret completed absent", "0 200 unknown completed not-a-signal", "256 200 unknown completed absent"):
            with self.subTest(record=record), self.assertRaises(comparison.Refused):
                comparison.parse_box(record, 0)

    def test_native_exposes_exit_only_and_does_not_invent_errno_or_http(self):
        self.assertFalse(self.native(code=7))
        self.assertEqual(self.value["native"]["before"]["exit"], 7)
        self.assertIsNone(self.value["native"]["before"]["http"])
        self.assertIsNone(self.value["native"]["before"]["errno"])
        self.assertNotIn("DO-NOT-PRINT", json.dumps(self.value))
        self.assertEqual(comparison.NATIVE_COMMAND,
                         'exec curl -q --noproxy \'*\' -fs --max-time 5 --unix-socket "$1" http://localhost/_ping >/dev/null 2>/dev/null')

    def test_verified_absence_allows_denied_signal_but_uncertain_native_withholds(self):
        self.assertTrue(self.native(signal="denied"))
        with self.assertRaises(comparison.Refused):
            self.native(code=None, collector="termination-unverified", signal="denied")
        self.assertTrue(self.value["uncertain"])
        self.assertEqual(self.value["native"]["first_ready_ms"], 1000)

    def test_exact_boundaries_and_suspended_boottime_never_promote(self):
        self.box("0 200 unknown completed absent", elapsed=180000)
        self.assertIsNone(self.value["consumer"]["first_ready_ms"])
        self.assertFalse(self.native(elapsed=360000))
        self.assertIsNone(self.value["native"]["first_ready_ms"])
        with mock.patch.object(comparison.time, "clock_gettime_ns", return_value=460000000000), \
                mock.patch.object(comparison.time, "monotonic", return_value=101):
            self.assertEqual(comparison.elapsed(self.value), 360000)
        self.assertEqual(comparison.origin_ms("100.29"), 100290)
        self.assertEqual(comparison.origin_ms("100.99"), 100990)
        for text in ("100.999", "-1.00", "nan", "100", None):
            with self.assertRaises(comparison.Refused):
                comparison.origin_ms(text)

    def test_timed_out_known_exit_zero_does_not_promote_native_readiness(self):
        self.assertFalse(self.native(code=0, collector="timed-out", elapsed=1000))
        self.assertIsNone(self.value["native"]["first_ready_ms"])
        self.assertEqual(self.value["native"]["before"]["exit"], 0)

    def test_independent_classification_expected_results(self):
        cases = ((None, 1000, "early-native-only-ready-observed"),
                 (None, 180000, "native-ready-after-consumer-budget"),
                 (None, 359999, "native-ready-after-consumer-budget"),
                 (None, 360000, "shared-no-ready-observed"),
                 (1000, 190000, "both-routes-ready"),
                 (179999, 1000, "both-routes-ready"),
                 (1000, None, "consumer-only-ready-observed"),
                 (180000, None, "shared-no-ready-observed"),
                 (None, None, "shared-no-ready-observed"))
        for consumer, native, expected in cases:
            with self.subTest(consumer=consumer, native=native):
                self.value["consumer"]["first_ready_ms"] = consumer
                self.value["native"]["first_ready_ms"] = native
                self.assertEqual(comparison.classify(self.value), expected)
        self.value["prerequisite"] = "unknown"
        self.assertEqual(comparison.classify(self.value), "prerequisite-not-met")

    def test_actual_numeric_errno_only(self):
        self.assertEqual(comparison.number(OSError(13, "DO-NOT-PRINT")), 13)
        for value in (None, 0, -1, 99999, True, "111"):
            self.assertIsNone(comparison.number(SimpleNamespace(errno=value)))
        self.assertIsNone(comparison.number(RuntimeError("errno=111 DO-NOT-PRINT")))

    def main(self, operation, *arguments):
        argv = [str(SOURCE), operation, "--directory", str(self.directory), *arguments]
        with mock.patch.object(sys, "argv", argv), redirect_stderr(io.StringIO()) as output:
            status = comparison.main()
        self.assertNotIn("DO-NOT-PRINT", output.getvalue())
        return status

    def test_refused_exclusive_init_never_overwrites_historical_report(self):
        historical = self.directory / comparison.REPORT
        historical.write_bytes(b"DO-NOT-PRINT/historical")
        historical.chmod(0o600)
        self.assertEqual(self.main("init", "--native-root", str(ROOT)), 2)
        self.assertEqual(historical.read_bytes(), b"DO-NOT-PRINT/historical")

    def test_private_report_refuses_symlinks_modes_duplicates_and_unknown_fields(self):
        comparison.write_report(self.directory, self.value, create=True)
        self.assertEqual((self.directory / comparison.REPORT).stat().st_mode & 0o777, 0o600)
        self.assertEqual(comparison.read_report(self.directory), self.value)
        report = self.directory / comparison.REPORT
        for raw in (b'{"directory":[],"directory":[]}', json.dumps(dict(self.value, injected="DO-NOT-PRINT")).encode(), b"{"):
            report.write_bytes(raw)
            with self.assertRaises(Exception):
                comparison.read_report(self.directory)
        report.unlink()
        report.symlink_to(SOURCE)
        with self.assertRaises(OSError):
            comparison.read_report(self.directory)
        self.directory.chmod(0o755)
        with self.assertRaises(comparison.Refused):
            comparison.private_directory(self.directory)

    def test_prerequisite_refusal_persists_closed_withheld_record(self):
        with mock.patch.object(comparison, "host_prerequisite", side_effect=OSError(1, "DO-NOT-PRINT/private")):
            self.assertEqual(self.main("init", "--native-root", str(ROOT), "--expected-pid-namespace", "5:12345"), 2)
        value = comparison.read_report(self.directory)
        self.assertEqual(value["status"], "withheld")
        self.assertEqual(value["phase"], "host-prerequisite")
        self.assertEqual(value["errno"], 1)
        self.assertNotIn("DO-NOT-PRINT", json.dumps(value))

    def test_initial_source_binding_uses_exact_catalogue_and_native_literal(self):
        script = native_readiness_layout()
        with mock.patch.object(comparison, "host_prerequisite", return_value=[5, 12345]), \
                mock.patch.object(comparison.contract, "canonical_images", return_value={"debian11-rootful": "image@sha256:" + CID}) as catalogue, \
                mock.patch.object(comparison.contract, "bounded_regular_bytes", return_value=script), \
                mock.patch.object(comparison.contract, "git", return_value="d" * 40), \
                mock.patch.object(comparison, "digest", return_value=CID), \
                mock.patch.object(comparison, "bridge_prerequisite"):
            self.assertEqual(self.main("init", "--native-root", str(ROOT), "--native-revision", "b" * 40,
                                       "--native-script-sha256", "c" * 64, "--expected-pid-namespace", "5:12345"), 0)
        catalogue.assert_called_once_with(str(ROOT), "b" * 40, "c" * 64)
        self.assertEqual(comparison.read_report(self.directory)["sources"]["image_sha256"], CID)

    def test_source_binding_refuses_missing_extra_changed_or_misplaced_native_calls(self):
        genuine = native_readiness_layout()
        literal = b'curl -q --noproxy \'*\' -fs --max-time 5 --unix-socket "$socket" http://localhost/_ping >/dev/null'
        first = b"    if " + literal + b"; then break; fi"
        final = b"[[ -S $socket ]] && " + literal + b" || {"
        variants = {
            "missing-loop": genuine.replace(first, b"", 1),
            "missing-final": genuine.replace(final, b"", 1),
            "extra-call": genuine + literal + b"\n",
            "extra-comment": genuine + b"# " + literal + b"\n",
            "changed-loop": genuine.replace(literal, literal.replace(b"--max-time 5", b"--max-time 6"), 1),
            "changed-final": genuine.replace(final, final.replace(b"http://localhost/_ping", b"http://localhost/info")),
            "loop-as-comment": genuine.replace(first, b"# " + first),
            "final-as-comment": genuine.replace(final, b"# " + final),
            "wrong-budget": genuine.replace(b"SECONDS + 360", b"SECONDS + 180"),
            "final-inside-loop": genuine.replace(b"done\n" + final, final + b"\ndone\n"),
            "reversed-sites": genuine.replace(first, b"SWAP").replace(final, first).replace(b"SWAP", final),
        }
        args = SimpleNamespace(native_root=ROOT, native_revision="b" * 40, native_script_sha256="c" * 64)
        for label, source in variants.items():
            with self.subTest(label=label), \
                    mock.patch.object(comparison.contract, "canonical_images", return_value={"debian11-rootful": "image@sha256:" + CID}) as catalogue, \
                    mock.patch.object(comparison.contract, "bounded_regular_bytes", return_value=source), \
                    mock.patch.object(comparison.contract, "git") as git, \
                    self.assertRaises(comparison.Refused):
                comparison.source_binding(args)
            catalogue.assert_called_once_with(str(ROOT), "b" * 40, "c" * 64)
            git.assert_not_called()

    def test_host_admission_requires_namespace_mapping_caps_and_rootful_info(self):
        caps = hex((1 << 12) | (1 << 21))[2:]
        for uid, namespace, mapping, effective, outcome, raw, admitted in (
                (0, [5, 12345], "0 0 4294967295", caps, "read", b"false\n", True),
                (1, [5, 12345], "0 0 4294967295", caps, "read", b"false", False),
                (0, [5, 99999], "0 0 4294967295", caps, "read", b"false", False),
                (0, [5, 12345], "0 100000 65536", caps, "read", b"false", False),
                (0, [5, 12345], "0 0 4294967295", "0", "read", b"false", False),
                (0, [5, 12345], "0 0 4294967295", caps, "read", b"true", False),
                (0, [5, 12345], "0 0 4294967295", caps, "timed-out", b"false", False)):
            with self.subTest(uid=uid, namespace=namespace, mapping=mapping, effective=effective, raw=raw), \
                    mock.patch.object(comparison.os, "geteuid", return_value=uid), \
                    mock.patch.object(comparison.os, "getuid", return_value=uid), \
                    mock.patch.object(comparison.os, "stat", return_value=SimpleNamespace(st_dev=namespace[0], st_ino=namespace[1])), \
                    mock.patch.object(pathlib.Path, "read_text", side_effect=[mapping, "CapEff:\t" + effective]), \
                    mock.patch.object(comparison.contract, "readiness_read", return_value=(outcome, raw)) as read:
                if admitted:
                    self.assertEqual(comparison.host_prerequisite("5:12345"), [5, 12345])
                    self.assertEqual(read.call_args.args[0], ["podman", "info", "--format", "{{.Host.Security.Rootless}}"])
                else:
                    with self.assertRaises(comparison.Refused):
                        comparison.host_prerequisite("5:12345")

    def test_host_and_inspect_use_real_canonical_reader_success_contract(self):
        # Only the read-only command producer is substituted. Session ownership,
        # collection, teardown and the 'read' result are the real shared reader.
        original_popen = subprocess.Popen
        outer = {"id": CID, "name": "bf-docker-core-AbCd", "owner": "AbCd", "running": True, "pid": 321, "privileged": True}
        calls = []
        def producer(argv, **kwargs):
            calls.append(argv)
            if argv == ["podman", "info", "--format", "{{.Host.Security.Rootless}}"]:
                raw = "false\n"
            else:
                self.assertEqual(argv, ["podman", "inspect", "--format", comparison.INSPECT, "bf-docker-core-AbCd"])
                raw = json.dumps(outer) + "\n"
            return original_popen([sys.executable, "-c", "import sys; sys.stdout.write(" + repr(raw) + ")"], **kwargs)
        caps = hex((1 << 12) | (1 << 21))[2:]
        with mock.patch.object(comparison.os, "geteuid", return_value=0), \
                mock.patch.object(comparison.os, "getuid", return_value=0), \
                mock.patch.object(comparison.os, "stat", return_value=SimpleNamespace(st_dev=5, st_ino=12345)), \
                mock.patch.object(pathlib.Path, "read_text", side_effect=["0 0 4294967295", "CapEff:\t" + caps]), \
                mock.patch.object(comparison.contract.native_read.subprocess, "Popen", side_effect=producer), \
                mock.patch.object(comparison.contract, "readiness_read", wraps=comparison.contract.readiness_read) as read:
            self.assertEqual(comparison.host_prerequisite("5:12345"), [5, 12345])
            self.assertEqual(comparison.inspect_outer("bf-docker-core-AbCd", "AbCd"), outer)
            self.assertEqual(read.call_count, 2)
        self.assertEqual(len(calls), 2)

    def test_real_canonical_reader_failure_is_not_host_or_inspect_success(self):
        original_popen = subprocess.Popen
        def failed_producer(argv, **kwargs):
            return original_popen([sys.executable, "-c", "import sys; print('false'); sys.exit(7)"], **kwargs)
        caps = hex((1 << 12) | (1 << 21))[2:]
        with mock.patch.object(comparison.os, "geteuid", return_value=0), \
                mock.patch.object(comparison.os, "getuid", return_value=0), \
                mock.patch.object(comparison.os, "stat", return_value=SimpleNamespace(st_dev=5, st_ino=12345)), \
                mock.patch.object(pathlib.Path, "read_text", side_effect=["0 0 4294967295", "CapEff:\t" + caps]), \
                mock.patch.object(comparison.contract.native_read.subprocess, "Popen", side_effect=failed_producer):
            with self.assertRaises(comparison.Refused):
                comparison.host_prerequisite("5:12345")
            with self.assertRaises(comparison.Refused):
                comparison.inspect_outer("bf-docker-core-AbCd", "AbCd")

    def test_actual_handoff_before_budget_survives_delayed_or_suspended_recorder(self):
        for recorder_elapsed in (180010, 360000, 3599999):
            with self.subTest(recorder_elapsed=recorder_elapsed):
                value = copy.deepcopy(self.value)
                with mock.patch.object(comparison.time, "clock_gettime_ns", return_value=(100000 + recorder_elapsed) * 1000000):
                    comparison.box_observation(value, "baseline", "0 200 unknown completed absent", 0, "279.99")
                self.assertEqual(value["consumer"]["first_ready_ms"], 179990)
                self.assertEqual(value["consumer"]["baseline"]["elapsed_ms"], 179990)
        for handoff in ("280.00", "280.01", "460.00"):
            value = copy.deepcopy(self.value)
            with mock.patch.object(comparison.time, "clock_gettime_ns", return_value=460000000000):
                comparison.box_observation(value, "baseline", "0 200 unknown completed absent", 0, handoff)
            self.assertIsNone(value["consumer"]["first_ready_ms"])

    def test_handoff_future_pre_origin_and_out_of_order_refused_without_observation(self):
        for handoff in (None, "100.999", "99.99", "102.00"):
            with self.subTest(handoff=handoff), mock.patch.object(comparison, "elapsed", return_value=1000), \
                    self.assertRaises(comparison.Refused):
                comparison.box_observation(self.value, "baseline", "0 200 unknown completed absent", 0, handoff)
            self.assertEqual(self.value["consumer"]["polls"], 0)
        self.box(elapsed=1200)
        self.value["socket"] = {"native_mode": 0o666}
        with mock.patch.object(comparison, "elapsed", return_value=2000), self.assertRaises(comparison.Refused):
            comparison.box_observation(self.value, "harmonized", "0 200 unknown completed absent", 0, "101.19")
        self.value["native"]["last"] = {"elapsed_ms": 1510}
        with mock.patch.object(comparison, "elapsed", return_value=2000), self.assertRaises(comparison.Refused):
            comparison.box_observation(self.value, "harmonized", "0 200 unknown completed absent", 0, "101.50")
        self.value["native"]["last"] = {"elapsed_ms": 1509}
        with mock.patch.object(comparison, "elapsed", return_value=2000):
            comparison.box_observation(self.value, "harmonized", "0 200 unknown completed absent", 0, "101.50")
        self.assertEqual(self.value["consumer"]["first_ready_ms"], 1500)

    def test_bridge_uses_empty_environment_never_native_cli_or_module_load(self):
        module = SimpleNamespace(ensure_bridge_prerequisite=mock.Mock(
            return_value="DOCKERLENS_NATIVE_HOST_NETWORK: bridge_filter=ready module_load=not-needed"))
        spec = SimpleNamespace(loader=SimpleNamespace(exec_module=mock.Mock()))
        with mock.patch.object(comparison.importlib.util, "spec_from_file_location", return_value=spec), \
                mock.patch.object(comparison.importlib.util, "module_from_spec", return_value=module), \
                mock.patch.dict(os.environ, {"GITHUB_ACTIONS": "true", "DOCKERLENS_ALLOW_MODULE_LOAD": "true"}):
            comparison.bridge_prerequisite(ROOT)
        module.ensure_bridge_prerequisite.assert_called_once_with({})
        module.ensure_bridge_prerequisite.return_value = "DO-NOT-PRINT/untrusted"
        with mock.patch.object(comparison.importlib.util, "spec_from_file_location", return_value=spec), \
                mock.patch.object(comparison.importlib.util, "module_from_spec", return_value=module), \
                self.assertRaises(comparison.Refused):
            comparison.bridge_prerequisite(ROOT)

    def test_effective_physical_caps_accept_capped_parent_not_unlimited_leaf(self):
        root = pathlib.Path("/sys/fs/cgroup/machine.slice/libpod-" + CID + ".scope/container")
        def read(path):
            if path.parent == root:
                return "max 100000" if path.name == "cpu.max" else "max"
            return {"memory.max": "4294967296", "pids.max": "512", "cpu.max": "200000 100000"}[path.name]
        with mock.patch.object(pathlib.Path, "resolve", autospec=True, side_effect=lambda path, **_: path), \
                mock.patch.object(pathlib.Path, "read_text", autospec=True, side_effect=read):
            self.assertEqual(comparison.effective_limits(root),
                             {"memory_bytes": 4294967296, "cpu_quota": 2, "cpu_period": 1, "tasks": 512})
        for changed in ({"memory.max": "max", "pids.max": "max", "cpu.max": "max 100000"},
                        {"memory.max": "4294967297", "pids.max": "512", "cpu.max": "200000 100000"},
                        {"memory.max": "4294967296", "pids.max": "513", "cpu.max": "200000 100000"},
                        {"memory.max": "4294967296", "pids.max": "512", "cpu.max": "200001 100000"}):
            with self.subTest(changed=changed), \
                    mock.patch.object(pathlib.Path, "resolve", autospec=True, side_effect=lambda path, **_: path), \
                    mock.patch.object(pathlib.Path, "read_text", autospec=True, side_effect=lambda path: changed[path.name]), \
                    self.assertRaises(comparison.Refused):
                comparison.effective_limits(root)

    def test_creation_cid_identity_and_unknown_cgroup_layout_refused(self):
        outer = {"id": CID, "name": "bf-docker-core-AbCd", "owner": "AbCd", "running": True, "pid": 321, "privileged": True}
        with mock.patch.object(comparison, "private_directory", return_value=[1, 2, 0]), \
                mock.patch.object(comparison.contract, "bounded_regular_bytes", return_value=(CID + "\n").encode()), \
                mock.patch.object(comparison, "inspect_outer", return_value=outer), \
                mock.patch.object(comparison, "process", return_value={"pid": 321, "start": 123, "namespace": [5, 99999]}), \
                mock.patch.object(pathlib.Path, "read_text", return_value="0::/unreviewed/private-host-path\n"), \
                self.assertRaises(comparison.Refused):
            comparison.admit_outer(self.value, outer["name"], "AbCd", pathlib.Path("/tmp/boxferry-docker-core.AbCd/socket/docker.sock"), pathlib.Path("/unused"), "100.00")
        self.assertEqual(self.value["outer"]["id"], CID)
        self.assertIsNone(self.value["outer"]["layout"])
        self.assertNotIn("unreviewed", json.dumps(self.value))
        self.value["outer"]["id"] = "b" * 64
        comparison.write_report(self.directory, self.value, create=True)
        with mock.patch.object(comparison, "inspect_outer", return_value=outer):
            self.assertEqual(self.main("cleanup-check", "--outer", outer["name"], "--run", "AbCd"), 2)

    def test_cleanup_uncertainty_withholds_even_successful_route_observations(self):
        self.value["consumer"]["first_ready_ms"] = 1000
        self.value["native"]["first_ready_ms"] = 1100
        comparison.write_report(self.directory, self.value, create=True)
        self.assertEqual(self.main("finish", "--cleanup", "unknown", "--run-status", "0"), 2)
        self.assertEqual(comparison.read_report(self.directory)["status"], "withheld")
        self.assertEqual(self.main("finish", "--cleanup", "verified", "--run-status", "1"), 2)
        self.assertEqual(self.main("finish", "--cleanup", "verified", "--run-status", "0"), 0)
        value = comparison.read_report(self.directory)
        self.assertEqual(value["classification"], "both-routes-ready")
        self.assertEqual(value["qualification"], "none")

    def test_known_cgroup_layouts_exact_pid_and_creation_cid_admitted(self):
        outer = {"id": CID, "name": "bf-docker-core-AbCd", "owner": "AbCd", "running": True, "pid": 321, "privileged": True}
        for layout, raw in (("systemd-scope", "0::/machine.slice/libpod-" + CID + ".scope/container\n"),
                            ("cgroupfs-parent", "0::/libpod_parent/libpod-" + CID + "\n")):
            def read(path):
                return raw if str(path).startswith("/proc/") else "321\n"
            with self.subTest(layout=layout), \
                    mock.patch.object(comparison, "private_directory", return_value=[1, 2, 0]), \
                    mock.patch.object(comparison.contract, "bounded_regular_bytes", return_value=(CID + "\n").encode()), \
                    mock.patch.object(comparison, "inspect_outer", return_value=outer), \
                    mock.patch.object(comparison, "process", return_value={"pid": 321, "start": 123, "namespace": [5, 99999]}), \
                    mock.patch.object(pathlib.Path, "read_text", autospec=True, side_effect=read), \
                    mock.patch.object(pathlib.Path, "resolve", autospec=True, side_effect=lambda path, **_: path), \
                    mock.patch.object(comparison, "effective_limits", return_value={"memory_bytes": 4294967296, "cpu_quota": 2, "cpu_period": 1, "tasks": 512}):
                comparison.admit_outer(self.value, outer["name"], "AbCd", pathlib.Path("/tmp/boxferry-docker-core.AbCd/socket/docker.sock"), pathlib.Path("/unused"), "100.00")
                self.assertEqual(self.value["outer"]["layout"], layout)
                self.assertEqual(self.value["origin"], 100000)
        for changes in ({"id": "b" * 64}, {"privileged": False}, {"running": False}):
            with self.subTest(changes=changes), \
                    mock.patch.object(comparison, "private_directory", return_value=[1, 2, 0]), \
                    mock.patch.object(comparison.contract, "bounded_regular_bytes", return_value=(CID + "\n").encode()), \
                    mock.patch.object(comparison, "inspect_outer", return_value=dict(outer, **changes)), \
                    self.assertRaises(comparison.Refused):
                comparison.admit_outer(self.value, outer["name"], "AbCd", pathlib.Path("/tmp/boxferry-docker-core.AbCd/socket/docker.sock"), pathlib.Path("/unused"), "100.00")

    def test_permission_transition_requires_baseline_and_same_authenticated_inode(self):
        socket = self.directory / "socket/docker.sock"
        socket.parent.mkdir()
        identity = {"pid": 321, "start": 123, "namespace": [5, 99999]}
        self.value["outer"] = dict(identity, id=CID, layout="systemd-scope", container_leaf=False,
                                  directory=[1, 2, 0], path_sha256=hashlib.sha256(str(socket).encode()).hexdigest(),
                                  limits={"memory_bytes": 4294967296, "cpu_quota": 2, "cpu_period": 1, "tasks": 512})
        def metadata(inode=22, mode=0o600):
            return SimpleNamespace(st_mode=stat.S_IFSOCK | mode, st_uid=0, st_gid=0, st_dev=11, st_ino=inode)
        with mock.patch.object(comparison, "private_directory", return_value=[1, 2, 0]), \
                mock.patch.object(comparison, "process", return_value=identity), \
                mock.patch.object(comparison.os, "stat", return_value=SimpleNamespace(st_dev=5, st_ino=12345)), \
                mock.patch.object(pathlib.Path, "lstat", return_value=metadata()), \
                mock.patch.object(comparison.os, "chmod") as chmod:
            comparison.endpoint(self.value, socket)
            with self.assertRaises(comparison.Refused):
                comparison.endpoint(self.value, socket, transition=True)
            chmod.assert_not_called()
        self.box()
        with mock.patch.object(comparison, "private_directory", return_value=[1, 2, 0]), \
                mock.patch.object(comparison, "process", return_value=identity), \
                mock.patch.object(comparison.os, "stat", return_value=SimpleNamespace(st_dev=5, st_ino=12345)), \
                mock.patch.object(pathlib.Path, "lstat", side_effect=[metadata(), metadata(mode=0o666)]), \
                mock.patch.object(comparison.os, "chmod") as chmod:
            comparison.endpoint(self.value, socket, transition=True)
            chmod.assert_called_once_with(socket, 0o666, follow_symlinks=False)
        self.assertEqual(self.value["socket"]["original_mode"], 0o600)
        self.assertEqual(self.value["socket"]["native_mode"], 0o666)
        with mock.patch.object(comparison, "private_directory", return_value=[1, 2, 0]), \
                mock.patch.object(comparison, "process", return_value=identity), \
                mock.patch.object(comparison.os, "stat", return_value=SimpleNamespace(st_dev=5, st_ino=12345)), \
                mock.patch.object(pathlib.Path, "lstat", return_value=metadata(inode=23)), \
                mock.patch.object(comparison.os, "chmod") as chmod, self.assertRaises(comparison.Refused):
            comparison.endpoint(self.value, socket, transition=True)
        chmod.assert_not_called()

    def test_pid_and_physical_cgroup_absence_required_for_final_closure(self):
        self.value["outer"] = {"id": CID, "pid": 321, "start": 123, "namespace": [5, 99999],
            "layout": "systemd-scope", "container_leaf": False, "directory": [1, 2, 0], "path_sha256": "b" * 64}
        comparison.write_report(self.directory, self.value, create=True)
        for residual in ("/proc/321", str(comparison.cgroup_path(self.value["outer"])), None):
            with self.subTest(residual=residual), \
                    mock.patch.object(pathlib.Path, "exists", autospec=True, side_effect=lambda path: str(path) == residual):
                self.assertEqual(self.main("finish", "--cleanup", "verified", "--run-status", "0"), 0 if residual is None else 2)
                value = comparison.read_report(self.directory)
                self.assertEqual(value["cleanup"], "verified" if residual is None else "unknown")

    def test_changed_source_binding_withholds_after_verified_cleanup(self):
        self.value["sources"] = {key: ("a" * 40 if key.endswith("_revision") else "a" * 64) for key in (
            "native_revision", "boxferry_revision", "native_script_sha256", "image_sha256", "bridge_sha256",
            "harness_sha256", "contract_sha256", "observer_sha256", "collector_sha256", "launcher_sha256")}
        comparison.write_report(self.directory, self.value, create=True)
        with mock.patch.object(comparison, "source_binding", return_value=dict(self.value["sources"], harness_sha256="b" * 64)):
            self.assertEqual(self.main("finish", "--cleanup", "verified", "--run-status", "0"), 2)
        self.assertEqual(comparison.read_report(self.directory)["classification"], "indeterminate")

    def log_value(self):
        self.log_socket = pathlib.Path("/tmp/boxferry-docker-core.AbCd/socket/docker.sock")
        self.log_process = {"pid": 321, "start": 123, "namespace": [5, 99999]}
        self.log_outer = {"id": CID, "name": "bf-docker-core-AbCd", "owner": "AbCd",
                          "running": True, "pid": 321, "privileged": True}
        self.log_limits = {"memory_bytes": 4294967296, "cpu_quota": 2, "cpu_period": 1, "tasks": 512}
        self.value["outer"] = dict(self.log_process, id=CID, layout="systemd-scope", container_leaf=False,
                                  directory=[1, 2, 0], path_sha256=hashlib.sha256(str(self.log_socket).encode()).hexdigest(),
                                  limits=self.log_limits)
        self.value["consumer"]["first_ready_ms"] = 1000
        self.value["native"]["first_ready_ms"] = 1100
        self.value["classification"] = comparison.classify(self.value)

    @contextmanager
    def log_guards(self, raw=b"failed to start daemon", outcome="read", *, real_reader=False):
        """Authored process/cgroup metadata; no daemon or host lookup is run."""
        root = comparison.cgroup_path(self.value["outer"])
        directories = {self.log_socket.parent.parent: (2, 0o700),
                       self.log_socket.parent: (3, 0o777), root: (4, 0o755)}
        originals = {name: getattr(os, name) for name in ("open", "close", "fstat", "stat")}
        methods = {name: getattr(pathlib.Path, name) for name in ("resolve", "lstat", "read_text")}
        original_private = comparison.private_directory
        original_acquire = comparison.owned_native_launch.OwnedLauncher.acquire
        descriptors = {}
        def metadata(path):
            inode, mode = directories[path]
            return SimpleNamespace(st_dev=1, st_ino=inode, st_uid=0, st_gid=0, st_mode=stat.S_IFDIR | mode)
        def opening(path, flags, *args, **kwargs):
            if pathlib.Path(path) in directories:
                fd = originals["open"](self.directory, flags, *args, **kwargs)
                descriptors[fd] = pathlib.Path(path)
                return fd
            return originals["open"](path, flags, *args, **kwargs)
        def closing(fd):
            descriptors.pop(fd, None)
            return originals["close"](fd)
        def lstat(path):
            if path in directories:
                return metadata(path)
            if path == self.log_socket:
                raise FileNotFoundError
            return methods["lstat"](path)
        def reading(path):
            if path == pathlib.Path("/proc/321/cgroup"):
                return "0::/machine.slice/libpod-" + CID + ".scope\n"
            if path == root / "cgroup.procs":
                return "321\n"
            return methods["read_text"](path)
        calls = []
        def read(argv, deadline, **kwargs):
            calls.append((argv, deadline, kwargs))
            if argv == ["podman", "logs", "--tail", "80", CID]:
                return outcome, raw
            self.assertEqual(argv, ["podman", "inspect", "--format", comparison.INSPECT, CID])
            self.assertEqual(set(kwargs), {"launcher"})
            return "read", json.dumps(self.log_outer).encode()
        def producer(owner, argv, **kwargs):
            calls.append((argv, None, kwargs))
            if argv == ["podman", "logs", "--tail", "80", CID]:
                source = "import sys;sys.stdout.buffer.write(" + repr(raw) + ");sys.stderr.write('PRIVACY-SENTINEL')"
                if outcome == "cancelled":
                    source += ";import os,signal;os.kill(os.getppid(),signal.SIGTERM)"
                elif outcome != "read":
                    source += ";sys.exit(7)"
            else:
                self.assertEqual(argv, ["podman", "inspect", "--format", comparison.INSPECT, CID])
                source = "import sys;sys.stdout.write(" + repr(json.dumps(self.log_outer)) + ")"
            return original_acquire(owner, [sys.executable, "-I", "-B", "-c", source], **kwargs)
        with ExitStack() as stack:
            for name, replacement in (("open", opening), ("close", closing),
                    ("fstat", lambda fd: metadata(descriptors[fd]) if fd in descriptors else originals["fstat"](fd)),
                    ("stat", lambda path, *args, **kwargs: SimpleNamespace(st_dev=5, st_ino=12345)
                     if path == "/proc/self/ns/pid" else originals["stat"](path, *args, **kwargs))):
                stack.enter_context(mock.patch.object(comparison.os, name, side_effect=replacement))
            for name, replacement in (("resolve", lambda path, **kwargs: path if path in directories else methods["resolve"](path, **kwargs)),
                                      ("lstat", lstat), ("read_text", reading)):
                stack.enter_context(mock.patch.object(pathlib.Path, name, autospec=True, side_effect=replacement))
            stack.enter_context(mock.patch.object(comparison, "private_directory", side_effect=lambda path:
                [1, 2, 0] if path == self.log_socket.parent.parent else original_private(path)))
            process = stack.enter_context(mock.patch.object(comparison, "process", return_value=self.log_process))
            limits = stack.enter_context(mock.patch.object(comparison, "effective_limits", return_value=self.log_limits))
            if real_reader:
                stack.enter_context(mock.patch.object(comparison.owned_native_launch.OwnedLauncher, "acquire", new=producer))
                reader = None
            else:
                reader = stack.enter_context(mock.patch.object(comparison.contract, "readiness_read", side_effect=read))
            yield SimpleNamespace(calls=calls, process=process, limits=limits, metadata=metadata,
                                  reading=reading, lstat=lstat, descriptors=descriptors, reader=reader)

    def run_logs(self):
        return self.main("logs", "--outer", "bf-docker-core-AbCd", "--run", "AbCd", "--socket", str(self.log_socket))

    def test_startup_log_categories_independently_expected_and_private(self):
        cases = ((b"", "empty"), (b"unrecognized PRIVACY-SENTINEL", "content-present"),
                 (b"Permission denied PRIVACY-SENTINEL", "permission-error-observed"),
                 (b"failed to mount overlay", "storage-error-observed"),
                 (b"iptables failed", "network-error-observed"), (b"address already in use", "socket-error-observed"),
                 (b"failed to start daemon", "startup-error-observed"),
                 (b"permission denied; iptables failed", "multiple-errors-observed"))
        self.assertEqual(comparison.LOG_CATEGORIES, {expected for _raw, expected in cases})
        for raw, expected in cases:
            with self.subTest(expected=expected):
                self.log_value()
                self.value["startup_logs"] = {"status": "not-run", "category": None}
                comparison.write_report(self.directory, self.value, create=not (self.directory / comparison.REPORT).exists())
                before = copy.deepcopy(self.value)
                with self.log_guards(raw), mock.patch.object(comparison.time, "monotonic", return_value=10):
                    self.assertEqual(self.run_logs(), 0)
                value = comparison.read_report(self.directory)
                self.assertEqual(value.pop("startup_logs"), {"status": "observed", "category": expected})
                before.pop("startup_logs")
                self.assertEqual(value, before)
                self.assertNotIn("PRIVACY-SENTINEL", (self.directory / comparison.REPORT).read_text())

    def test_optional_log_failure_never_changes_readiness_or_qualification(self):
        for outcome, raw in (("read-failed", b"failed to start daemon"), ("unknown", b"permission denied"),
                             ("oversized", b"iptables failed"), ("termination-unverified", b"permission denied"),
                             ("timed-out", b""), ("read", b"x" * 16385)):
            with self.subTest(outcome=outcome):
                self.log_value()
                self.value["startup_logs"] = {"status": "not-run", "category": None}
                comparison.write_report(self.directory, self.value, create=not (self.directory / comparison.REPORT).exists())
                before = copy.deepcopy(self.value)
                with self.log_guards(raw, outcome):
                    self.assertEqual(self.run_logs(), 2)
                value = comparison.read_report(self.directory)
                self.assertEqual(value.pop("startup_logs"), {"status": "withheld", "category": None})
                before.pop("startup_logs")
                self.assertEqual(value, before)

    def test_logs_shared_deadline_exact_cid_merged_streams_and_one_invocation(self):
        self.log_value()
        comparison.write_report(self.directory, self.value, create=True)
        with self.log_guards() as guards, mock.patch.object(comparison.time, "monotonic", return_value=10), \
                mock.patch.object(comparison.time, "clock_gettime", return_value=100):
            self.assertEqual(self.run_logs(), 0)
            self.assertEqual([call[1] for call in guards.calls], [17, 17, 17])
            self.assertEqual(set(guards.calls[1][2]), {"merge_output", "launcher"})
            self.assertTrue(guards.calls[1][2]["merge_output"])
            self.assertEqual(self.run_logs(), 2)
            self.assertEqual(len(guards.calls), 3)

    def test_duplicate_observed_and_withheld_reports_are_byte_preserving(self):
        for logs in ({"status": "observed", "category": "startup-error-observed"},
                     {"status": "withheld", "category": None}):
            with self.subTest(logs=logs):
                self.log_value()
                self.value["startup_logs"] = logs
                report = self.directory / comparison.REPORT
                comparison.write_report(self.directory, self.value, create=not report.exists())
                before = report.read_bytes()
                with mock.patch.object(comparison.contract, "readiness_read") as read, \
                        mock.patch.object(comparison, "startup_logs") as collect:
                    self.assertEqual(self.run_logs(), 2)
                self.assertEqual(report.read_bytes(), before)
                read.assert_not_called()
                collect.assert_not_called()

    def test_real_blocking_metadata_read_interrupted_by_whole_seven_second_alarm(self):
        self.log_value()
        comparison.write_report(self.directory, self.value, create=True)
        read_fd, write_fd = os.pipe()
        handler = signal.getsignal(signal.SIGALRM)
        previous_mask = signal.pthread_sigmask(signal.SIG_BLOCK, set())
        started = time.clock_gettime(time.CLOCK_BOOTTIME)
        try:
            with self.log_guards() as guards:
                def blocking(path):
                    if path == pathlib.Path("/proc/321/cgroup"):
                        # Actual interruptible descriptor read: the writer is
                        # held open with no bytes, not a fake-clock timeout.
                        with os.fdopen(os.dup(read_fd), "r") as source:
                            return source.read()
                    return guards.reading(path)
                with mock.patch.object(pathlib.Path, "read_text", new=blocking):
                    self.assertEqual(self.run_logs(), 2)
                self.assertEqual(len(guards.calls), 1)  # No log read after bracket expiry.
        finally:
            os.close(read_fd)
            os.close(write_fd)
        duration = time.clock_gettime(time.CLOCK_BOOTTIME) - started
        self.assertGreaterEqual(duration, 6.5)
        self.assertLess(duration, 8)
        self.assertEqual(comparison.read_report(self.directory)["startup_logs"], {"status": "withheld", "category": None})
        self.assertEqual(signal.getsignal(signal.SIGALRM), handler)
        self.assertEqual(signal.getitimer(signal.ITIMER_REAL), (0.0, 0.0))
        self.assertEqual(signal.pthread_sigmask(signal.SIG_BLOCK, set()), previous_mask)

    def test_real_aggregate_alarm_enters_canonical_reader_teardown_without_orphan(self):
        self.log_value()
        comparison.write_report(self.directory, self.value, create=True)
        original_read = comparison.contract.readiness_read
        original_acquire = comparison.owned_native_launch.OwnedLauncher.acquire
        children = []
        with self.log_guards() as guards:
            fast_read = guards.reader.side_effect
            def read(argv, deadline, **kwargs):
                if argv[1] == "logs":
                    # Deliberately keep the underlying real reader alive past
                    # this control's short aggregate alarm to exercise cleanup.
                    return original_read(argv, time.monotonic() + 3, **kwargs)
                return fast_read(argv, deadline, **kwargs)
            def producer(owner, argv, **kwargs):
                self.assertEqual(argv, ["podman", "logs", "--tail", "80", CID])
                child = original_acquire(owner, [sys.executable, "-I", "-B", "-c", "import time;time.sleep(60)"], **kwargs)
                children.append(child)
                return child
            guards.reader.side_effect = read
            with mock.patch.object(comparison, "LOG_WORK_SECONDS", 0.2), \
                    mock.patch.object(comparison.owned_native_launch.OwnedLauncher, "acquire", new=producer):
                self.assertEqual(self.run_logs(), 2)
        self.assertEqual(len(children), 1)
        child = children[0]
        self.assertIsNotNone(child.returncode)
        self.assertTrue(child.stdout.closed)
        with self.assertRaises(ProcessLookupError):
            os.kill(child.pid, 0)
        with self.assertRaises(ProcessLookupError):
            os.killpg(child.pid, 0)
        self.assertIsNone(comparison.read_report(self.directory)["startup_logs"]["category"])

    def test_boottime_boundary_and_suspend_refuse_even_without_monotonic_advance(self):
        for after in (107, 108, 200):
            with self.subTest(boottime=after):
                self.log_value()
                self.value["startup_logs"] = {"status": "not-run", "category": None}
                comparison.write_report(self.directory, self.value, create=not (self.directory / comparison.REPORT).exists())
                before = copy.deepcopy(self.value)
                clock = [100]
                with self.log_guards() as guards, \
                        mock.patch.object(comparison.time, "monotonic", return_value=10), \
                        mock.patch.object(comparison.time, "clock_gettime", side_effect=lambda _clock: clock[0]):
                    original = guards.reader.side_effect
                    def suspend(argv, deadline, **kwargs):
                        result = original(argv, deadline, **kwargs)
                        if argv[1] == "logs":
                            clock[0] = after
                        return result
                    guards.reader.side_effect = suspend
                    self.assertEqual(self.run_logs(), 2)
                value = comparison.read_report(self.directory)
                self.assertEqual(value.pop("startup_logs"), {"status": "withheld", "category": None})
                before.pop("startup_logs")
                self.assertEqual(value, before)

    def test_alarm_during_reader_acquisition_reaps_registered_child(self):
        self.log_value()
        comparison.write_report(self.directory, self.value, create=True)
        original_read = comparison.contract.readiness_read
        original_acquire = comparison.owned_native_launch.OwnedLauncher.acquire
        children = []
        handler = signal.getsignal(signal.SIGALRM)
        with self.log_guards() as guards:
            fast_read = guards.reader.side_effect
            def read(argv, deadline, **kwargs):
                return original_read(argv, deadline, **kwargs) if argv[1] == "logs" else fast_read(argv, deadline, **kwargs)
            def producer(owner, argv, **kwargs):
                child = original_acquire(owner, [sys.executable, "-I", "-B", "-c", "import time;time.sleep(60)"], **kwargs)
                children.append(child)
                signal.raise_signal(signal.SIGALRM)  # Before Popen returns its handle.
                return child
            guards.reader.side_effect = read
            with mock.patch.object(comparison.owned_native_launch.OwnedLauncher, "acquire", new=producer):
                self.assertEqual(self.run_logs(), 2)
        self.assertEqual(len(children), 1)
        self.assertIsNotNone(children[0].returncode)
        self.assertTrue(children[0].stdout.closed)
        with self.assertRaises(ProcessLookupError):
            os.kill(children[0].pid, 0)
        with self.assertRaises(ProcessLookupError):
            os.killpg(children[0].pid, 0)
        self.assertEqual(signal.getsignal(signal.SIGALRM), handler)
        self.assertEqual(signal.getitimer(signal.ITIMER_REAL), (0.0, 0.0))

    def test_alarm_handler_timer_and_mask_restored_and_occupied_facility_not_stolen(self):
        previous = signal.getsignal(signal.SIGALRM)
        mask = signal.pthread_sigmask(signal.SIG_BLOCK, set())
        handler = lambda *_: None
        try:
            signal.signal(signal.SIGALRM, handler)
            with comparison.log_budget() as budget:
                budget.check()
            self.assertEqual(signal.getsignal(signal.SIGALRM), handler)
            self.assertEqual(signal.getitimer(signal.ITIMER_REAL), (0.0, 0.0))
            signal.setitimer(signal.ITIMER_REAL, 30, 30)
            with self.assertRaises(comparison.Refused), comparison.log_budget():
                self.fail("occupied timer entered")
            remaining, interval = signal.getitimer(signal.ITIMER_REAL)
            self.assertGreater(remaining, 29)
            self.assertEqual(interval, 30)
            self.assertEqual(signal.getsignal(signal.SIGALRM), handler)
            signal.setitimer(signal.ITIMER_REAL, 0)
            signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGALRM})
            with self.assertRaises(comparison.Refused), comparison.log_budget():
                self.fail("blocked alarm entered")
            self.assertIn(signal.SIGALRM, signal.pthread_sigmask(signal.SIG_BLOCK, set()))
        finally:
            signal.setitimer(signal.ITIMER_REAL, 0)
            signal.signal(signal.SIGALRM, previous)
            signal.pthread_sigmask(signal.SIG_SETMASK, mask)

    def test_schema_one_historical_bytes_and_unknown_log_shapes_are_refused(self):
        historical = copy.deepcopy(self.value)
        historical["schema_version"] = 1
        historical.pop("startup_logs")
        raw = json.dumps(historical).encode()
        report = self.directory / comparison.REPORT
        report.write_bytes(raw)
        report.chmod(0o600)
        self.log_socket = pathlib.Path("/unused")
        self.assertEqual(self.run_logs(), 2)
        self.assertEqual(report.read_bytes(), raw)
        for changes in ({"status": "observed", "category": "unknown"},
                        {"status": "withheld", "category": "permission-error-observed"},
                        {"status": "not-run", "category": None, "raw": "PRIVACY-SENTINEL"}):
            with self.subTest(changes=changes), self.assertRaises(comparison.Refused):
                comparison.validate_report(dict(self.value, startup_logs=changes))
        with self.assertRaises(comparison.Refused):
            comparison.validate_report(dict(self.value, schema_version=2.0))

    def test_real_bounded_reader_logs_success_nonzero_and_overflow(self):
        for raw, outcome, expected in ((b"failed to start daemon", "read", "startup-error-observed"),
                                        (b"permission denied", "read-failed", None),
                                        (b"failed to start daemon", "cancelled", None),
                                        (b"iptables failed" + b"x" * 17000, "read", None)):
            with self.subTest(outcome=outcome, expected=expected):
                self.log_value()
                self.value["startup_logs"] = {"status": "not-run", "category": None}
                comparison.write_report(self.directory, self.value, create=not (self.directory / comparison.REPORT).exists())
                with self.log_guards(raw, outcome, real_reader=True):
                    self.assertEqual(self.run_logs(), 0 if expected is not None else 2)
                value = comparison.read_report(self.directory)
                self.assertEqual(value["startup_logs"]["category"], expected)
                self.assertNotIn("PRIVACY-SENTINEL", (self.directory / comparison.REPORT).read_text())

    def test_exact_cid_name_run_pid_running_privilege_drift_before_and_after(self):
        for changes in ({"id": "b" * 64}, {"name": "bf-docker-core-Other"}, {"owner": "Other"},
                        {"pid": 322}, {"running": False}, {"privileged": False}):
            for when in (1, 2):
                with self.subTest(changes=changes, inspection=when):
                    self.log_value()
                    self.value["startup_logs"] = {"status": "not-run", "category": None}
                    comparison.write_report(self.directory, self.value, create=not (self.directory / comparison.REPORT).exists())
                    with self.log_guards() as guards:
                        original = guards.reader.side_effect
                        inspected = 0
                        def drift(argv, deadline, **kwargs):
                            nonlocal inspected
                            result = original(argv, deadline, **kwargs)
                            if argv[1] == "inspect":
                                inspected += 1
                                if inspected == when:
                                    return "read", json.dumps(dict(self.log_outer, **changes)).encode()
                            return result
                        guards.reader.side_effect = drift
                        self.assertEqual(self.run_logs(), 2)
                        self.assertEqual(sum(call[0][1] == "logs" for call in guards.calls), when - 1)
                    self.assertIsNone(comparison.read_report(self.directory)["startup_logs"]["category"])

    def test_process_start_namespace_and_effective_limit_drift_before_and_after(self):
        for key, changed in (("start", 124), ("namespace", [5, 88888]), ("pid", 322)):
            for when in (1, 2):
                with self.subTest(key=key, observation=when):
                    self.log_value()
                    self.value["startup_logs"] = {"status": "not-run", "category": None}
                    comparison.write_report(self.directory, self.value, create=not (self.directory / comparison.REPORT).exists())
                    with self.log_guards() as guards:
                        records = [dict(self.log_process), dict(self.log_process)]
                        records[when - 1][key] = changed
                        guards.process.side_effect = records
                        self.assertEqual(self.run_logs(), 2)
                    self.assertFalse(comparison.read_report(self.directory)["uncertain"])
        for changes in ({"memory_bytes": 2147483648}, {"tasks": 511}, {"cpu_quota": 1}, {"cpu_period": 2}):
            for when in (1, 2):
                with self.subTest(changes=changes, observation=when):
                    self.log_value()
                    self.value["startup_logs"] = {"status": "not-run", "category": None}
                    comparison.write_report(self.directory, self.value, create=not (self.directory / comparison.REPORT).exists())
                    with self.log_guards() as guards:
                        limits = [dict(self.log_limits), dict(self.log_limits)]
                        limits[when - 1].update(changes)
                        guards.limits.side_effect = limits
                        self.assertEqual(self.run_logs(), 2)
                    self.assertIsNone(comparison.read_report(self.directory)["startup_logs"]["category"])

    def test_cgroup_membership_and_layout_drift_before_and_after(self):
        for target, changed in (("/proc/321/cgroup", "0::/unrelated\n"),
                                (str(comparison.cgroup_path({"layout": "systemd-scope", "id": CID,
                                     "container_leaf": False}) / "cgroup.procs"), "322\n")):
            for when in (1, 2):
                with self.subTest(target=target, observation=when):
                    self.log_value()
                    self.value["startup_logs"] = {"status": "not-run", "category": None}
                    comparison.write_report(self.directory, self.value, create=not (self.directory / comparison.REPORT).exists())
                    with self.log_guards() as guards:
                        seen = 0
                        def read(path):
                            nonlocal seen
                            if str(path) == target:
                                seen += 1
                                if seen == when:
                                    return changed
                            return guards.reading(path)
                        with mock.patch.object(pathlib.Path, "read_text", new=read):
                            self.assertEqual(self.run_logs(), 2)
                    self.assertIsNone(comparison.read_report(self.directory)["startup_logs"]["category"])

    def test_held_socket_directory_and_host_namespace_drift_withholds(self):
        for when in ("before", "after"):
            for kind in ("directory", "host-namespace"):
                with self.subTest(when=when, kind=kind):
                    self.log_value()
                    self.value["startup_logs"] = {"status": "not-run", "category": None}
                    comparison.write_report(self.directory, self.value, create=not (self.directory / comparison.REPORT).exists())
                    with self.log_guards() as guards:
                        def altered():
                            return when == "before" or any(call[0][1] == "logs" for call in guards.calls)
                        def lstat(path):
                            if path == self.log_socket.parent and altered():
                                original = guards.metadata(path)
                                return SimpleNamespace(**dict(vars(original), st_ino=99))
                            return guards.lstat(path)
                        original_stat = comparison.os.stat.side_effect
                        def stat_call(path, *args, **kwargs):
                            if path == "/proc/self/ns/pid" and altered():
                                return SimpleNamespace(st_dev=5, st_ino=88888)
                            return original_stat(path, *args, **kwargs)
                        with mock.patch.object(pathlib.Path, "lstat", new=lstat), \
                                mock.patch.object(comparison.os, "stat", side_effect=stat_call):
                            self.assertEqual(self.run_logs(), 2)
                    self.assertIsNone(comparison.read_report(self.directory)["startup_logs"]["category"])

    def test_deadline_exact_boundary_cancel_and_classifier_drift_withhold_only_logs(self):
        for failure in ("deadline", "cancel", "classifier"):
            with self.subTest(failure=failure):
                self.log_value()
                self.value["startup_logs"] = {"status": "not-run", "category": None}
                comparison.write_report(self.directory, self.value, create=not (self.directory / comparison.REPORT).exists())
                before = copy.deepcopy(self.value)
                with self.log_guards() as guards, ExitStack() as stack:
                    original = guards.reader.side_effect
                    clock = [10]
                    def read(argv, deadline, **kwargs):
                        result = original(argv, deadline, **kwargs)
                        if argv[1] == "logs":
                            if failure == "cancel":
                                raise KeyboardInterrupt
                            if failure == "deadline":
                                clock[0] = deadline
                        return result
                    guards.reader.side_effect = read
                    stack.enter_context(mock.patch.object(comparison.time, "monotonic", side_effect=lambda: clock[0]))
                    stack.enter_context(mock.patch.object(comparison.time, "clock_gettime", side_effect=lambda _clock: clock[0]))
                    if failure == "classifier":
                        stack.enter_context(mock.patch.object(comparison.contract, "readiness_log_category", return_value="PRIVACY-SENTINEL"))
                    self.assertEqual(self.run_logs(), 2)
                value = comparison.read_report(self.directory)
                self.assertEqual(value.pop("startup_logs"), {"status": "withheld", "category": None})
                before.pop("startup_logs")
                self.assertEqual(value, before)

    def test_unknown_log_diagnostic_does_not_block_otherwise_verified_finish(self):
        self.log_value()
        comparison.write_report(self.directory, self.value, create=True)
        with self.log_guards(outcome="read-failed"):
            self.assertEqual(self.run_logs(), 2)
        with mock.patch.object(pathlib.Path, "exists", return_value=False):
            self.assertEqual(self.main("finish", "--cleanup", "verified", "--run-status", "0"), 0)
        value = comparison.read_report(self.directory)
        self.assertEqual(value["startup_logs"], {"status": "withheld", "category": None})
        self.assertEqual(value["classification"], "both-routes-ready")
        self.assertEqual(value["qualification"], "none")

    def test_unadmitted_logs_and_prior_comparison_failure_are_never_promoted(self):
        for binding in (None, {"pid": None, "limits": {}}, {"pid": 321, "limits": {}, "layout": None}):
            with self.subTest(binding=binding):
                self.log_value()
                self.value["startup_logs"] = {"status": "withheld", "category": None}
                self.value["outer"] = binding
                with mock.patch.object(comparison.contract, "readiness_read") as read, self.assertRaises(comparison.Refused):
                    comparison.startup_logs(self.value, "bf-docker-core-AbCd", "AbCd", self.log_socket)
                read.assert_not_called()
        self.log_value()
        self.value["startup_logs"] = {"status": "not-run", "category": None}
        self.value.update(uncertain=True, status="withheld", classification="indeterminate", errno=13)
        comparison.write_report(self.directory, self.value, create=True)
        before = copy.deepcopy(self.value)
        with self.log_guards():
            self.assertEqual(self.run_logs(), 0)
        value = comparison.read_report(self.directory)
        value.pop("startup_logs")
        before.pop("startup_logs")
        self.assertEqual(value, before)

    def test_late_finite_log_reader_closure_withholds_category_despite_physical_reap(self):
        self.log_value()
        comparison.write_report(self.directory, self.value, create=True)
        before = copy.deepcopy(self.value)
        children = []
        with self.log_guards(real_reader=True):
            original = comparison.owned_native_launch.OwnedLauncher.acquire
            def acquire(owner, argv, **kwargs):
                child = original(owner, argv, **kwargs)
                child.stdout = FiniteDelayedClose(child.stdout, 0.6)
                children.append(child)
                return child
            with mock.patch.object(comparison.owned_native_launch.OwnedLauncher, "acquire", new=acquire):
                self.assertEqual(self.run_logs(), 2)
        self.assertEqual(len(children), 2)
        for child in children:
            self.assertEqual(child.state, "reaped")
            self.assertTrue(child.verified)
            self.assertTrue(child.stdout.closed)
            self.assertEqual(child.fds, {})
            self.assertIsNone(child.pidfd)
        value = comparison.read_report(self.directory)
        self.assertEqual(value.pop("startup_logs"), {"status": "withheld", "category": None})
        before.pop("startup_logs")
        self.assertEqual(value, before)
        self.assertNotIn("PRIVACY-SENTINEL", (self.directory / comparison.REPORT).read_text())

    def test_directory_open_publication_closes_real_fd_for_all_four_signals(self):
        for signum in comparison.owned_native_launch.CANCEL:
            with self.subTest(signum=signum):
                self.log_value()
                self.value["startup_logs"] = {"status": "not-run", "category": None}
                comparison.write_report(self.directory, self.value, create=not (self.directory / comparison.REPORT).exists())
                previous_mask = signal.pthread_sigmask(signal.SIG_BLOCK, set())
                opened = []
                with self.log_guards() as guards:
                    original = comparison.os.open.side_effect
                    def open_fd(path, *args, **kwargs):
                        fd = original(path, *args, **kwargs)
                        if pathlib.Path(path) == self.log_socket.parent.parent:
                            opened.append(fd)
                            os.kill(os.getpid(), signum)
                        return fd
                    with mock.patch.object(comparison.os, "open", side_effect=open_fd):
                        self.assertEqual(self.run_logs(), 2)
                    self.assertEqual(guards.calls, [])
                self.assertEqual(len(opened), 1)
                with self.assertRaises(OSError):
                    os.fstat(opened[0])
                self.assertEqual(signal.pthread_sigmask(signal.SIG_BLOCK, set()), previous_mask)
                self.assertIsNone(comparison.read_report(self.directory)["startup_logs"]["category"])

    def test_failed_directory_callback_registration_closes_exact_fd(self):
        self.log_value()
        comparison.write_report(self.directory, self.value, create=True)
        opened = []
        with self.log_guards() as guards:
            original = comparison.os.open.side_effect
            def open_fd(path, *args, **kwargs):
                fd = original(path, *args, **kwargs)
                if pathlib.Path(path) == self.log_socket.parent.parent:
                    opened.append(fd)
                return fd
            with mock.patch.object(comparison.os, "open", side_effect=open_fd), \
                    mock.patch.object(comparison.ExitStack, "callback", side_effect=RuntimeError("PRIVACY-SENTINEL")):
                self.assertEqual(self.run_logs(), 2)
            self.assertEqual(guards.calls, [])
        self.assertEqual(len(opened), 1)
        with self.assertRaises(OSError):
            os.fstat(opened[0])


class OwnedLaunchTests(unittest.TestCase):
    def owner(self, seconds=2):
        now = time.clock_gettime(time.CLOCK_BOOTTIME)
        return comparison.owned_native_launch.OwnedLauncher(now + seconds, now + seconds + 1)

    @contextmanager
    def cancellations(self):
        previous = {}
        def cancelled(*_):
            raise KeyboardInterrupt
        try:
            for signum in comparison.owned_native_launch.CANCEL:
                previous[signum] = signal.signal(signum, cancelled)
            yield
        finally:
            for signum, handler in previous.items():
                signal.signal(signum, handler)

    def child(self, owner, source="import time;time.sleep(60)"):
        return owner.acquire([sys.executable, "-I", "-B", "-c", source], merge_output=True)

    def assert_closed(self, child):
        self.assertEqual(child.state, "reaped")
        self.assertTrue(child.closed)
        self.assertTrue(child.verified)
        self.assertTrue(child.stdout.closed)
        self.assertEqual(child.fds, {})
        self.assertIsNone(child.pidfd)
        with self.assertRaises(ProcessLookupError):
            os.kill(child.pid, 0)

    def test_four_real_signals_at_fork_return_publish_child_before_unmask(self):
        original_fork = os.fork
        for signum in comparison.owned_native_launch.CANCEL:
            with self.subTest(signum=signum), self.cancellations():
                owner = self.owner()
                def fork():
                    pid = original_fork()
                    if pid:
                        os.kill(os.getpid(), signum)
                    return pid
                with mock.patch.object(os, "fork", side_effect=fork), self.assertRaises(KeyboardInterrupt):
                    self.child(owner)
                self.assertEqual(len(owner.children), 1)
                child = owner.children[0]
                self.assertEqual(child.state, "bootstrap-owned")
                with mock.patch.object(os, "killpg", wraps=os.killpg) as groups:
                    owner.close_all()
                self.assertFalse(any(call.args[1] != 0 for call in groups.call_args_list))
                self.assert_closed(child)

    def test_blocked_bootstrap_is_interruptible_and_never_executes_without_ack(self):
        owner = self.owner(0.15)
        # Genuine child-side blocking bootstrap, not a blocked Popen factory.
        def blocked(_owner, _fds, _argv, _merge, previous):
            signal.pthread_sigmask(signal.SIG_SETMASK, previous)
            time.sleep(60)
            os._exit(127)
        started = time.clock_gettime(time.CLOCK_BOOTTIME)
        with mock.patch.object(type(owner), "child_exec", new=blocked):
            child = self.child(owner)
        with self.assertRaises(OSError):
            owner.prepare(child, time.monotonic() + 2)
        with mock.patch.object(os, "killpg", wraps=os.killpg) as groups:
            owner.close_all()
        self.assertFalse(any(call.args[1] != 0 for call in groups.call_args_list))
        self.assertLess(time.clock_gettime(time.CLOCK_BOOTTIME) - started, 1.15)
        self.assert_closed(child)

    def test_ready_ack_prevents_exec_before_session_publication(self):
        with tempfile.TemporaryDirectory(prefix="boxferry-owned-exec-") as name:
            marker = pathlib.Path(name) / "exec"
            owner = self.owner()
            child = self.child(owner, "from pathlib import Path;Path(" + repr(str(marker)) + ").touch()")
            # READY can be available, but without prepare/ACK the executable
            # cannot run and no descendants can be spawned.
            self.assertTrue(comparison.owned_native_launch.select.select([child.fds["ready-read"]], [], [], 1)[0])
            self.assertFalse(marker.exists())
            owner.close_all()
            self.assertFalse(marker.exists())
            self.assert_closed(child)

    def test_four_real_signals_interrupt_bootstrap_wait_before_ack(self):
        for signum in comparison.owned_native_launch.CANCEL:
            with self.subTest(signum=signum), self.cancellations():
                owner = self.owner()
                parent = os.getpid()
                def blocked(_owner, _fds, _argv, _merge, previous):
                    signal.pthread_sigmask(signal.SIG_SETMASK, previous)
                    time.sleep(0.05)
                    os.kill(parent, signum)
                    time.sleep(60)
                    os._exit(127)
                with mock.patch.object(type(owner), "child_exec", new=blocked):
                    child = self.child(owner)
                with self.assertRaises(KeyboardInterrupt):
                    owner.prepare(child, time.monotonic() + 1)
                with mock.patch.object(os, "killpg", wraps=os.killpg) as groups:
                    owner.close_all()
                self.assertFalse(any(call.args[1] != 0 for call in groups.call_args_list))
                self.assert_closed(child)

    def test_old_schema_two_source_shape_is_refused_without_rewriting(self):
        with tempfile.TemporaryDirectory(prefix="boxferry-old-comparison-") as name:
            directory = pathlib.Path(name)
            directory.chmod(0o700)
            value = comparison.initialize(SimpleNamespace(directory=directory, native_root=ROOT))
            value["sources"] = {key: "a" * (40 if key.endswith("_revision") else 64) for key in (
                "native_revision", "boxferry_revision", "native_script_sha256", "image_sha256", "bridge_sha256",
                "harness_sha256", "contract_sha256", "observer_sha256", "collector_sha256")}
            report = directory / comparison.REPORT
            raw = json.dumps(value).encode()
            report.write_bytes(raw)
            report.chmod(0o600)
            with self.assertRaises(comparison.Refused):
                comparison.read_report(directory)
            self.assertEqual(report.read_bytes(), raw)

    def test_external_reap_echild_never_authorizes_numeric_rescue(self):
        owner = self.owner()
        child = self.child(owner, "pass")
        owner.prepare(child, time.monotonic() + 1)
        # This deliberately violates the exclusive-wait-owner contract.
        os.waitpid(child.pid, 0)
        with mock.patch.object(os, "killpg") as groups, mock.patch.object(os, "kill") as direct:
            with self.assertRaises(OSError):
                owner.teardown(child)
            with self.assertRaises(OSError):
                owner.close_all()
        groups.assert_not_called()
        direct.assert_not_called()
        self.assertEqual(child.state, "ownership-lost")
        self.assertTrue(child.stdout.closed)
        self.assertEqual(child.fds, {})

    def test_echild_with_live_proc_node_still_withholds_group_signal(self):
        owner = self.owner()
        child = self.child(owner, "pass")
        owner.prepare(child, time.monotonic() + 1)
        while owner.observe(child) is None:
            time.sleep(0.005)
        with mock.patch.object(os, "waitid", side_effect=ChildProcessError), \
                mock.patch.object(os, "killpg") as groups:
            with self.assertRaises(OSError):
                owner.teardown(child)
        groups.assert_not_called()
        self.assertEqual(child.state, "ownership-lost")
        os.waitpid(child.pid, 0)  # Independent test owner reaps its exited child.

    def test_shutdown_signals_before_signal_during_wait_and_at_reap_commit(self):
        module = comparison.owned_native_launch
        original_identity, original_waitpid, original_sleep, original_killpg = module.identity, os.waitpid, time.sleep, os.killpg
        for signum in module.CANCEL:
            for seam in ("identity", "sleep", "reap"):
                with self.subTest(signum=signum, seam=seam), self.cancellations():
                    owner = self.owner()
                    child = self.child(owner)
                    owner.prepare(child, time.monotonic() + 1)
                    injected = [False]
                    waited = [False]
                    signals = []
                    def inject():
                        if not injected[0]:
                            injected[0] = True
                            os.kill(os.getpid(), signum)
                    def identity(pid):
                        if seam == "identity":
                            inject()
                        return original_identity(pid)
                    def waitpid(pid, flags):
                        if seam == "sleep" and not waited[0]:
                            waited[0] = True
                            return 0, 0
                        result = original_waitpid(pid, flags)
                        if result[0] and seam == "reap":
                            inject()  # Kernel reaped; Python state not committed yet.
                        return result
                    def sleep(seconds):
                        if seam == "sleep":
                            inject()
                        return original_sleep(seconds)
                    def killpg(pid, number):
                        if number:
                            self.assertNotEqual(child.state, "reaped")
                            os.waitid(os.P_PID, pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
                        signals.append(number)
                        return original_killpg(pid, number)
                    with mock.patch.object(module, "identity", side_effect=identity), \
                            mock.patch.object(os, "waitpid", side_effect=waitpid), \
                            mock.patch.object(time, "sleep", side_effect=sleep), \
                            mock.patch.object(os, "killpg", side_effect=killpg):
                        with self.assertRaises(KeyboardInterrupt):
                            owner.teardown(child)
                        owner.close_all()
                    self.assertTrue(injected[0])
                    self.assertEqual(signals.count(signal.SIGKILL), 1)
                    self.assert_closed(child)

    def test_fork_failure_closes_registered_pipes_without_any_pid_signal(self):
        owner = self.owner()
        with mock.patch.object(os, "fork", side_effect=OSError), self.assertRaises(OSError):
            self.child(owner)
        with mock.patch.object(os, "killpg") as groups, mock.patch.object(os, "kill") as direct:
            owner.close_all()
        groups.assert_not_called()
        direct.assert_not_called()
        self.assertEqual(owner.children[0].fds, {})
        self.assertTrue(owner.children[0].verified)

    def test_non_main_thread_or_multithreaded_use_refused_before_fork(self):
        with mock.patch.object(comparison.owned_native_launch.threading, "active_count", return_value=2), \
                mock.patch.object(os, "fork") as fork, self.assertRaises(OSError):
            self.owner()
        fork.assert_not_called()

    def test_partial_pipe_failure_closes_only_registered_descriptors(self):
        owner = self.owner()
        original_pipe = os.pipe2
        count = [0]
        def pipe(flags):
            count[0] += 1
            if count[0] == 2:
                raise OSError
            return original_pipe(flags)
        with mock.patch.object(os, "pipe2", side_effect=pipe), mock.patch.object(os, "fork") as fork, self.assertRaises(OSError):
            self.child(owner)
        fork.assert_not_called()
        owner.close_all()
        self.assertEqual(owner.children[0].fds, {})
        self.assertTrue(owner.children[0].verified)

    def test_shared_teardown_reserve_exhaustion_does_not_signal_again_or_reset(self):
        owner = self.owner()
        child = self.child(owner)
        owner.prepare(child, time.monotonic() + 1)
        owner.teardown_left = 0.02
        with mock.patch.object(os, "waitpid", return_value=(0, 0)), \
                mock.patch.object(os, "killpg", wraps=os.killpg) as groups:
            with self.assertRaises(OSError):
                owner.teardown(child)
            self.assertTrue(child.closed)
            self.assertNotEqual(child.state, "reaped")
            with self.assertRaises(OSError):
                owner.close_all()
        self.assertEqual(sum(call.args[1] == signal.SIGKILL for call in groups.call_args_list), 1)
        self.assertEqual(owner.teardown_left, 0)
        os.waitpid(child.pid, 0)  # Independent test reaps its already-killed child.

    def test_identity_open_publication_closes_actual_fd_for_all_four_signals(self):
        original_open = os.open
        for signum in comparison.owned_native_launch.CANCEL:
            with self.subTest(signum=signum), self.cancellations():
                opened = []
                mask = signal.pthread_sigmask(signal.SIG_BLOCK, set())
                def open_fd(path, *args, **kwargs):
                    fd = original_open(path, *args, **kwargs)
                    opened.append(fd)
                    os.kill(os.getpid(), signum)
                    return fd
                with mock.patch.object(os, "open", side_effect=open_fd), self.assertRaises(KeyboardInterrupt):
                    comparison.owned_native_launch.identity(os.getpid())
                self.assertEqual(len(opened), 1)
                with self.assertRaises(OSError):
                    os.fstat(opened[0])
                self.assertEqual(signal.pthread_sigmask(signal.SIG_BLOCK, set()), mask)

    def test_two_early_finite_closes_and_registry_retry_charge_total_shared_time(self):
        owner = self.owner(5)
        children = [self.child(owner, "pass") for _ in range(2)]
        for child in children:
            owner.prepare(child, time.monotonic() + 1)
            child.stdout = FiniteDelayedClose(child.stdout, 0.6)
            while owner.observe(child) is None:
                self.assertLess(time.clock_gettime(time.CLOCK_BOOTTIME), owner.work_deadline)
                time.sleep(0.005)
        self.assertEqual(owner.teardown(children[0]), 0)
        with self.assertRaises(OSError):
            owner.teardown(children[1])
        self.assertTrue(owner.teardown_late)
        self.assertEqual(owner.teardown_left, 0)
        for child in children:
            self.assert_closed(child)
        with mock.patch.object(os, "killpg") as groups, self.assertRaises(OSError):
            owner.close_all()
        groups.assert_not_called()
        with mock.patch.object(os, "fork") as fork, self.assertRaises(OSError):
            self.child(owner, "pass")
        fork.assert_not_called()

    def test_legitimate_work_gap_is_not_charged_as_teardown(self):
        owner = self.owner(5)
        children = [self.child(owner, "pass") for _ in range(2)]
        for child in children:
            owner.prepare(child, time.monotonic() + 1)
            while owner.observe(child) is None:
                self.assertLess(time.clock_gettime(time.CLOCK_BOOTTIME), owner.work_deadline)
                time.sleep(0.005)
        self.assertEqual(owner.teardown(children[0]), 0)
        time.sleep(1.05)
        self.assertEqual(owner.teardown(children[1]), 0)
        self.assertFalse(owner.teardown_late)
        self.assertGreater(owner.teardown_left, 0.9)
        owner.close_all()
        for child in children:
            self.assert_closed(child)

    def test_handler_overhead_cached_returns_and_registry_scope_are_charged_once(self):
        owner = self.owner()
        child = self.child(owner, "pass")
        owner.prepare(child, time.monotonic() + 1)
        owner.teardown(child)
        original = comparison.owned_native_launch.shutdown_signals
        @contextmanager
        def delayed_handlers():
            with original():
                time.sleep(0.05)
                yield
                time.sleep(0.05)
        previous = owner.teardown_left
        with mock.patch.object(comparison.owned_native_launch, "shutdown_signals", new=delayed_handlers):
            owner.close_all()
        charged = previous - owner.teardown_left
        self.assertGreaterEqual(charged, 0.1)
        self.assertLess(charged, 0.2)
        self.assert_closed(child)


class HarnessControls(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.harness = (ROOT / "scripts/docker-application-conformance.sh").read_text()

    def test_profile_argument_isolation_refuses_before_catalogue_or_runtime(self):
        base = ["bash", str(ROOT / "scripts/docker-application-conformance.sh"), "--docker-lens-root", "/absent",
                "--docker-lens-revision", "b" * 40, "--native-script-sha256", "c" * 64]
        cases = (["--profile", "volume-fixtures", "--evidence-directory", "/unused", "--diagnostic-directory", "/unused"],
                 ["--profile", "readiness-comparison", "--lane", "upstream-rootful", "--diagnostic-directory", "/unused", "--expected-pid-namespace", "5:12345"],
                 ["--profile", "readiness-comparison", "--lane", "debian11-rootful", "--diagnostic-directory", "/unused"],
                 ["--profile", "readiness-comparison", "--lane", "debian11-rootful", "--diagnostic-directory", "/unused", "--expected-pid-namespace", "5:12345", "--api-version", "1.41"],
                 ["--profile", "readiness-comparison", "--lane", "debian11-rootful", "--diagnostic-directory", "/unused", "--expected-pid-namespace", "5:12345", "--boxferry-binary", "/unused"])
        for args in cases:
            with self.subTest(args=args):
                result = subprocess.run(base + args, capture_output=True, text=True, timeout=3, check=False)
                self.assertEqual(result.returncode, 2)
                self.assertTrue(result.stderr.startswith("usage:"))
                self.assertNotIn("Traceback", result.stderr)

    def test_canonical_launcher_watchdog_guard_and_startup_only_exit(self):
        text = self.harness
        self.assertEqual(text.count("podman run --pull=never --detach"), 1)
        self.assertIn("--privileged --pids-limit=512 --memory=4g --cpus=2 --image-volume=ignore", text)
        self.assertIn("--execution-seconds 780 --cleanup-seconds 120", text)
        self.assertIn("runtime_deadline=$((runtime_deadline + 750))", text)
        self.assertIn("if [[ $profile != volume-fixtures && $profile != readiness-comparison ]]; then", text)
        self.assertLess(text.index('"$comparison" start'), text.index("podman run --pull=never --detach"))
        self.assertLess(text.index('"$comparison" bind'), text.index("read -r ping_readiness_uptime"))
        self.assertIn("ping_readiness_uptime=$comparison_origin", text)
        diagnostic = text[text.index("if [[ $profile == readiness-comparison ]]; then\n  bounded 6s python3 \"$comparison\" consumer-expired"):
                          text.index('\nchmod 0666 "$socket_path"')]
        self.assertIn("exit 0", diagnostic)
        for excluded in ("http://localhost/version", "http://localhost/info", "replay-test-only", "verify-candidate", "volume-fixtures --allow", "docker load", "podman exec"):
            self.assertNotIn(excluded, diagnostic)
        self.assertIn("comparison_origin%.*} + 360", diagnostic)
        cleanup = text[text.index("cleanup_owned() {"):text.index("on_exit() {")]
        self.assertIn('elif [[ $profile != readiness-comparison ]]; then', cleanup)
        self.assertLess(cleanup.index('"$comparison" cleanup-check'), cleanup.index("podman rm --force --volumes"))
        self.assertNotIn("prune", cleanup)

    def test_optional_logs_exactly_once_before_cleanup_with_signal_handler_armed(self):
        text = self.harness
        on_exit = text[text.index("on_exit() {"):text.index("\ntrap on_exit EXIT")]
        self.assertEqual(on_exit.count('"$comparison" logs'), 1)
        self.assertLess(on_exit.index("trap 'cleanup_cancelled=true' HUP INT TERM"), on_exit.index('"$comparison" logs'))
        self.assertLess(on_exit.index('"$comparison" logs'), on_exit.index("cleanup_owned ||"))
        self.assertNotIn('"$comparison" logs', text.replace(on_exit, ""))
        for initialized in ("true", "false"):
            for diagnostic_status in (0, 2, 124, 125, 137, 143):
                for prior in (0, 1):
                    for cancel in (False, True):
                        with self.subTest(initialized=initialized, diagnostic_status=diagnostic_status,
                                          prior=prior, cancel=cancel):
                            body = f'''set -u
comparison_initialized={initialized}
comparison=observer
diagnostic_directory=/unused
outer=bf-docker-core-AbCd
run_id=AbCd
socket_path=/tmp/boxferry-docker-core.AbCd/socket/docker.sock
run_dir=/nonexistent
lens_root=/source
lens_revision=revision
script_sha=digest
bounded() {{
  [[ $1 == 12s && $2 == python3 && $3 == observer && $4 == logs ]] || exit 99
  printf 'logs\n'
  {"kill -TERM $$" if cancel else ":"}
  return {diagnostic_status}
}}
cleanup_owned() {{ printf 'cleanup\n'; return 0; }}
cleanup_bounded() {{ printf 'finish %s\n' "$*"; return 0; }}
report_host_cache() {{ :; }}
{on_exit}
(exit {prior})
on_exit
'''
                            result = subprocess.run(["bash", "-c", body], capture_output=True, text=True, timeout=3, check=False)
                            expected = 1 if prior or initialized == "true" and cancel else 0
                            self.assertEqual(result.returncode, expected, result.stderr)
                            self.assertEqual(result.stdout.splitlines()[:2] if initialized == "true" else result.stdout.splitlines(),
                                             ["logs", "cleanup"] if initialized == "true" else ["cleanup"])
                            if initialized == "true":
                                self.assertIn("--cleanup " + ("unknown" if cancel else "verified"), result.stdout)
                                self.assertIn("--run-status " + str(expected), result.stdout)
                            self.assertNotIn("/version", result.stdout)
                            self.assertNotIn("/info", result.stdout)

    def test_actual_loop_records_baseline_transition_and_harmonized_bracket(self):
        start = self.harness.index("read -r ping_readiness_uptime")
        end = self.harness.index('\nchmod 0666 "$socket_path"', start)
        loop = self.harness[start:end].replace("-S $socket_path", "-f $socket_path")
        with tempfile.TemporaryDirectory(prefix="boxferry-comparison-loop-") as name:
            directory = pathlib.Path(name)
            socket_path = directory / "socket"
            socket_path.touch()
            log = directory / "calls"
            count = directory / "polls"
            body = f'''set -Eeuo pipefail
profile=readiness-comparison
comparison=observer
contract=contract
diagnostic_directory=unused
comparison_origin=100.00
comparison_baseline=false
comparison_bracketed=false
socket_path={shlex.quote(str(socket_path))}
run_id=AbCd
outer=bf-docker-core-AbCd
read() {{
  if [[ $2 == ping_readiness_uptime || $2 == ping_poll_uptime || $2 == ping_handoff_uptime || $2 == comparison_now || $2 == now ]]; then
    printf -v "$2" '%s' 101.00
  else builtin read "$@"; fi
}}
sleep() {{ :; }}
bounded() {{
  if [[ $2 == podman ]]; then printf 'true\\n'; return; fi
  printf '%s\\n' "$3 $4 $*" >> {shlex.quote(str(log))}
  if [[ $3 == contract && $4 == readiness-ping ]]; then
    if [[ -f {shlex.quote(str(count))} ]]; then printf '0 200 unknown completed absent\\n';
    else : > {shlex.quote(str(count))}; printf '7 000 connect-error-observed completed absent\\n'; fi
  fi
  return 0
}}
{loop}
'''
            result = subprocess.run(["bash", "-c", body], capture_output=True, text=True, timeout=3, check=False)
            self.assertEqual(result.returncode, 0, result.stderr)
            calls = log.read_text().splitlines()
            phases = [(line.split()[1], "baseline" if "--phase baseline" in line else
                       "harmonized" if "--phase harmonized" in line else
                       "native-before" if "--phase native-before" in line else
                       "native-after" if "--phase native-after" in line else "") for line in calls]
            self.assertEqual(phases[:9], [("endpoint", ""), ("readiness-ping", ""), ("box", "baseline"),
                             ("transition", ""), ("native", "native-before"), ("endpoint", ""),
                             ("readiness-ping", ""), ("box", "harmonized"), ("native", "native-after")])
            self.assertIn("--record 7 000 connect-error-observed completed absent", "\n".join(calls))
            self.assertIn("--handoff-boottime 101.00", "\n".join(calls))
            self.assertNotIn("/version", "\n".join(calls))


if __name__ == "__main__":
    unittest.main()
