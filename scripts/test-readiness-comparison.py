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
import stat
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr
from types import SimpleNamespace
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parent.parent
SOURCE = ROOT / "scripts/lib/readiness-comparison.py"
SPEC = importlib.util.spec_from_file_location("readiness_comparison", SOURCE)
assert SPEC is not None and SPEC.loader is not None
comparison = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(comparison)
CID = "a" * 64


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
        script = b'curl -q --noproxy \'*\' -fs --max-time 5 --unix-socket "$socket" http://localhost/_ping >/dev/null'
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
            "harness_sha256", "contract_sha256", "observer_sha256", "collector_sha256")}
        comparison.write_report(self.directory, self.value, create=True)
        with mock.patch.object(comparison, "source_binding", return_value=dict(self.value["sources"], harness_sha256="b" * 64)):
            self.assertEqual(self.main("finish", "--cleanup", "verified", "--run-status", "0"), 2)
        self.assertEqual(comparison.read_report(self.directory)["classification"], "indeterminate")


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
