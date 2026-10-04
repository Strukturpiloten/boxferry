#!/usr/bin/env python3
"""Independent offline evidence for the shared typed native-presence boundary."""
from __future__ import annotations

import importlib.util
import os
import pathlib
import shlex
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

LIB = pathlib.Path(__file__).parent / "lib"
SPEC = importlib.util.spec_from_file_location("native_presence", LIB / "native-presence.py")
assert SPEC is not None and SPEC.loader is not None
presence = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(presence)


class NativePresenceTests(unittest.TestCase):
    def test_typed_commands_are_closed_local_or_explicit_unix_socket_only(self) -> None:
        for kind in ("container", "volume", "network", "image"):
            for socket in (None, "/tmp/private/podman.sock"):
                with self.subTest(kind=kind, socket=socket), \
                        mock.patch.object(presence.native_read, "readiness_read", return_value=("absent", b"")) as read:
                    self.assertEqual(presence.presence("/tmp/fake-podman", kind, "exact-name", socket), "absent")
                    expected = ["/tmp/fake-podman"]
                    if socket:
                        expected += ["--url", "unix://" + socket]
                    self.assertEqual(read.call_args.args[0], expected + [kind, "exists", "exact-name"])
                    self.assertEqual(read.call_args.kwargs, {"presence": True})
        for engine, kind, name, socket in (("podman", "exec", "name", None), ("podman", "container", "--all", None),
                                            ("podman", "network", "name\nother", None), ("podman", "volume", "name", "tcp://host"),
                                            ("podman;private", "image", "name", None)):
            with mock.patch.object(presence.native_read, "readiness_read") as read:
                self.assertEqual(presence.presence(engine, kind, name, socket), "unknown")
                read.assert_not_called()

    def test_actual_native_cli_marker_status_and_private_combined_output(self) -> None:
        cases = [("exit 0", 0, "present"), ("exit 1", 1, "absent"),
                 ("printf 'PRIVATE/config-error' >&2; exit 1", 2, "unknown"),
                 ("printf 'PRIVATE/warning' >&2; exit 0", 2, "unknown"),
                 ("printf ' '; exit 1", 2, "unknown"), ("exit 125", 2, "unknown")]
        with tempfile.TemporaryDirectory(prefix="boxferry-native-presence-") as name:
            native = pathlib.Path(name) / "fake-engine"
            for source, status, marker in cases:
                with self.subTest(source=source):
                    native.write_text("#!/bin/sh\n" + source + "\n", encoding="ascii")
                    native.chmod(0o700)
                    result = subprocess.run([sys.executable, str(LIB / "native-presence.py"), "--engine", str(native),
                                             "--kind", "volume", "--name", "owned-volume"],
                                            capture_output=True, text=True, timeout=6, check=False)
                    self.assertEqual((result.returncode, result.stdout, result.stderr), (status, marker + "\n", ""))

    def test_shell_protocol_rejects_launch_failures_and_extra_newlines(self) -> None:
        for status, marker, expected in ((0, "present\n", 0), (1, "absent\n", 1), (2, "unknown\n", 2),
                                          (1, "", 2), (127, "absent\n", 2), (0, "absent\n", 2),
                                          (1, "absent\n\n", 2), (1, "absent", 2), (124, "absent\n", 2)):
            with self.subTest(status=status, marker=marker):
                script = f"""set -Eeuo pipefail
source {shlex.quote(str(LIB / 'native-presence.sh'))}
timeout() {{ printf '%s' {shlex.quote(marker)}; return {status}; }}
state=0
native_presence 30s podman volume owned || state=$?
printf '%s:%s\\n' "$state" "$native_presence_unverified"
"""
                result = subprocess.run(["bash", "-c", script], capture_output=True, text=True, timeout=3, check=False)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, f"{expected}:{'true' if expected == 2 else 'false'}\n")

    def test_helper_and_timer_keep_the_native_clients_existing_privilege(self) -> None:
        with tempfile.TemporaryDirectory() as name:
            root = pathlib.Path(name)
            native_uid, timer_uid = root / "native-uid", root / "timer-uid"
            native = root / "engine"
            native.write_text(f"#!/bin/sh\nid -u > {shlex.quote(str(native_uid))}\nexit 1\n", encoding="ascii")
            native.chmod(0o700)
            body = f"""source {shlex.quote(str(LIB / 'native-presence.sh'))}
timeout() {{ id -u > {shlex.quote(str(timer_uid))}; shift 3; "$@"; }}
state=0
native_presence 30s {shlex.quote(str(native))} volume owned || state=$?
printf '%s\\n' "$state"
"""
            result = subprocess.run(["bash", "-c", body], capture_output=True, text=True, timeout=6, check=False)
            self.assertEqual((result.returncode, result.stdout, result.stderr), (0, "1\n", ""))
            self.assertEqual(native_uid.read_text().strip(), str(os.geteuid()))
            self.assertEqual(timer_uid.read_text().strip(), str(os.geteuid()))

    def test_shared_reader_cap_deadline_cancellation_and_group_uncertainty(self) -> None:
        read = presence.native_read.readiness_read
        for stream in ("stdout", "stderr"):
            source = f"import sys; sys.{stream}.write('x'*16385); sys.exit(1)"
            self.assertEqual(read([sys.executable, "-c", source], time.monotonic() + 2, presence=True), ("oversized", b""))
        self.assertEqual(read([sys.executable, "-c", "import time; time.sleep(60)"],
                             time.monotonic() + 0.1, presence=True), ("timed-out", b""))
        previous = {signum: presence.signal.getsignal(signum) for signum in
                    (presence.signal.SIGTERM, presence.signal.SIGINT, presence.signal.SIGHUP)}
        for failure in (KeyboardInterrupt, OSError):
            with mock.patch.object(presence.native_read, "readiness_read", side_effect=failure):
                self.assertEqual(presence.presence("podman", "container", "owned"), "unknown")
            self.assertEqual(previous, {signum: presence.signal.getsignal(signum) for signum in previous})
        with tempfile.TemporaryFile() as stream:
            child = mock.Mock(pid=12345, stdout=stream)
            child.wait.return_value = 1
            with mock.patch.object(presence.native_read.subprocess, "Popen", return_value=child), \
                    mock.patch.object(presence.native_read.os, "waitid", return_value=mock.Mock()), \
                    mock.patch.object(presence.native_read.os, "killpg") as kill:
                self.assertEqual(presence.presence("podman", "container", "owned"), "unknown")
            self.assertEqual(kill.call_args_list[0].args, (12345, presence.signal.SIGKILL))
            self.assertTrue(all(call.args == (12345, 0) for call in kill.call_args_list[1:]))
            self.assertTrue(stream.closed)


class LiveConsumerTests(unittest.TestCase):
    runner = (LIB.parent / "podman-live-conformance.sh").read_text(encoding="utf-8")

    def function(self, name: str) -> str:
        start = self.runner.index(name + "() {")
        return self.runner[start:self.runner.index("\n}\n", start) + 3]

    def bash(self, body: str):
        return subprocess.run(["bash", "-c", "set -Eeuo pipefail\nexec 3>&2\n" + body],
                              capture_output=True, text=True, timeout=8, check=False)

    def test_unknown_image_state_cannot_authorize_tag_pull_or_ownership(self) -> None:
        for status in (0, 1):
            for consumer in ("alias", "matrix", "workload"):
                with self.subTest(status=status, consumer=consumer), tempfile.TemporaryDirectory() as name:
                    root = pathlib.Path(name)
                    native = root / "engine"
                    native.write_text(f"#!/bin/sh\nprintf 'PRIVATE/image-query' >&2\nexit {status}\n", encoding="ascii")
                    native.chmod(0o700)
                    marker = root / "unsafe-mutation"
                    call = {"alias": "record_run_owned_archive_alias source localhost/alias:1 archive",
                            "matrix": "prepare_matrix_image test-cell \"$workload_image\"",
                            "workload": "prepare_workload_archive"}[consumer]
                    body = f"""source {shlex.quote(str(LIB / 'native-presence.sh'))}
engine={shlex.quote(str(native))}
artifact_root={shlex.quote(name)}
workload_archive={shlex.quote(str(root / 'archive'))}
workload_image=example.invalid/workload:1@sha256:{'a' * 64}
workload_local_tag=localhost/workload:1
timestamp() {{ printf 'time'; }}
format_duration() {{ printf '%s' "$1"; }}
engine_operation() {{ printf 'unsafe' > {shlex.quote(str(marker))}; return 1; }}
timed_operation() {{ printf 'unsafe' > {shlex.quote(str(marker))}; return 1; }}
record_run_owned_host_image() {{ printf 'unsafe' > {shlex.quote(str(marker))}; }}
""" + "".join(self.function(fn) for fn in ("engine_image_available", "record_run_owned_archive_alias",
                                                              "prepare_matrix_image", "prepare_workload_archive")) + call
                    result = self.bash(body)
                    self.assertEqual(result.returncode, 2, result.stderr)
                    self.assertFalse(marker.exists())
                    self.assertNotIn(": absent", result.stderr)
                    self.assertNotIn("PRIVATE", result.stdout + result.stderr)

    def test_applied_target_queries_exact_socket_and_rejects_diagnostic_absence(self) -> None:
        start = self.runner.index("  local kind name presence_status\n")
        loop = self.runner[start:self.runner.index('  release_outer "${apply_target_outer}"', start)]
        for kind in ("container", "network", "volume"):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as name:
                native = pathlib.Path(name) / "engine"
                log = pathlib.Path(name) / "calls"
                native.write_text(f"""#!/bin/sh
printf '%s\\n' "$*" >> {shlex.quote(str(log))}
if [ "$3" = {kind} ]; then printf 'PRIVATE/query' >&2; fi
exit 1
""", encoding="ascii")
                native.chmod(0o700)
                body = f"""source {shlex.quote(str(LIB / 'native-presence.sh'))}
engine={shlex.quote(str(native))}
apply_target_socket=/tmp/owned/podman.sock
expected_container=owned-container
expected_network=owned-network
expected_volume=owned-volume
verify_applied_absence() {{
""" + loop + "}\nverify_applied_absence\n"
                result = self.bash(body)
                self.assertEqual(result.returncode, 1, result.stderr)
                calls = log.read_text().splitlines()
                self.assertEqual(calls[-1], f"--url unix:///tmp/owned/podman.sock {kind} exists owned-{kind}")
                self.assertNotIn("PRIVATE", result.stdout + result.stderr)

    def test_three_limitation_readbacks_require_positive_absence(self) -> None:
        boundaries = (("outer", "\n  fi"), ("revalidation_candidate_outer", "\n  fi"), ("apply_target_outer", "\n    fi"))
        for variable, end in boundaries:
            start = self.runner.index('if ! outer_resource_absent container "${' + variable + '}"; then')
            block = self.runner[start:self.runner.index(end, start) + len(end)]
            for marker in ("absent", "unknown", "present"):
                body = f"""source {shlex.quote(str(LIB / 'podman-live-outer-storage.sh'))}
native_presence() {{ return {dict(present=0, absent=1, unknown=2)[marker]}; }}
engine=unused
outer=owned
revalidation_candidate_outer=owned
apply_target_outer=owned
id=owned-cell
verify_readback() {{
""" + block + "\nreturn 0\n}\nverify_readback\n"
                result = self.bash(body)
                self.assertEqual(result.returncode, 0 if marker == "absent" else 1, result.stderr)

    def test_failed_or_unknown_cleanup_retains_recovery_and_attempts_later_resources(self) -> None:
        cases = [("unknown", 0, 1, True), ("removal-failed", 0, 1, True),
                 ("clean", 42, 42, False), ("clean", 0, 0, False)]
        for scenario, original, expected, retained in cases:
            with self.subTest(scenario=scenario, original=original), tempfile.TemporaryDirectory() as name:
                root = pathlib.Path(name)
                runtime, discovery, artifacts = (root / part for part in ("runtime", "discovery", "artifacts"))
                for directory in (runtime, discovery, artifacts):
                    directory.mkdir()
                (runtime / "recovery").write_bytes(b"PRIVATE/runtime")
                recovery_names = ("podman.sock", "bootstrap.log", "runtime-evidence.tsv", "runtime-evidence.ready",
                                  "runtime-canaries.log", "selected-container-id", "smoke-baseline.json", "start-api",
                                  "resource-setup.status", "resource-setup.status.tmp")
                for recovery_name in recovery_names:
                    (discovery / recovery_name).write_bytes(b"PRIVATE/recovery")
                attempts = root / "attempts"
                body = f"""profile=smoke
runtime_root={shlex.quote(str(runtime))}
artifact_root={shlex.quote(str(artifacts))}
discovery_directories=({shlex.quote(str(discovery))})
outer_containers=(first later)
mounted_images=() fault_proxy_pids=() fault_proxy_sockets=()
discovery_parent_created=false
retain_artifacts=false
native_presence_unverified={'true' if scenario == 'unknown' else 'false'}
ignore_outer_cleanup_signals() {{ :; }}
release_remaining_run_owned_host_images() {{ :; }}
release_outer() {{
  printf '%s\\n' "$1" >> {shlex.quote(str(attempts))}
  [[ {scenario} != removal-failed || $1 != first ]]
}}
""" + self.function("cleanup") + f"set +e\n(exit {original})\ncleanup\n"
                result = self.bash(body)
                self.assertEqual(result.returncode, expected, result.stderr)
                self.assertEqual(attempts.read_text().splitlines(), ["first", "later"])
                self.assertEqual(runtime.exists(), retained)
                for recovery_name in recovery_names:
                    self.assertEqual((discovery / recovery_name).exists(), retained)
                    if retained:
                        self.assertEqual((discovery / recovery_name).read_bytes(), b"PRIVATE/recovery")
                if retained:
                    self.assertEqual((runtime / "recovery").read_bytes(), b"PRIVATE/runtime")
                    self.assertIn("recovery evidence retained", result.stderr)
                self.assertNotIn("PRIVATE", result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
