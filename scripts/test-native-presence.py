#!/usr/bin/env python3
"""Independent offline evidence for the shared typed native-presence boundary."""
from __future__ import annotations

import importlib.util
import hashlib
import io
import json
import os
import pathlib
import shlex
import subprocess
import sys
import tempfile
import tarfile
import time
import unittest
from unittest import mock

LIB = pathlib.Path(__file__).parent / "lib"
SPEC = importlib.util.spec_from_file_location("native_presence", LIB / "native-presence.py")
assert SPEC is not None and SPEC.loader is not None
presence = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(presence)
OCI_SPEC = importlib.util.spec_from_file_location("oci_archive_identity", LIB / "oci-archive-identity.py")
assert OCI_SPEC is not None and OCI_SPEC.loader is not None
oci = importlib.util.module_from_spec(OCI_SPEC)
OCI_SPEC.loader.exec_module(oci)


class OciArchiveIdentityTests(unittest.TestCase):
    reference = "localhost/boxferry-archive/test/paperless:gotenberg"

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="boxferry-oci-metadata-")
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        self.path = self.root / "image.tar"
        self.config = json.dumps({"architecture": "amd64", "os": "linux",
                                  "rootfs": {"type": "layers", "diff_ids": []}}).encode()
        self.config_id = hashlib.sha256(self.config).hexdigest()
        self.manifest = {"schemaVersion": 2, "mediaType": oci.MANIFEST, "layers": [],
                         "config": {"mediaType": oci.CONFIG, "digest": "sha256:" + self.config_id,
                                    "size": len(self.config)}}

    def entries(self):
        manifest = json.dumps(self.manifest).encode()
        manifest_id = hashlib.sha256(manifest).hexdigest()
        self.index = {"schemaVersion": 2, "manifests": [{"mediaType": oci.MANIFEST,
                      "digest": "sha256:" + manifest_id, "size": len(manifest),
                      "annotations": {"org.opencontainers.image.ref.name": self.reference}}]}
        return [("oci-layout", b'{"imageLayoutVersion":"1.0.0"}'),
                ("index.json", json.dumps(self.index).encode()),
                ("blobs/sha256/" + manifest_id, manifest),
                ("blobs/sha256/" + self.config_id, self.config)]

    def write(self, entries=None, special=None):
        with tarfile.open(self.path, "w", format=tarfile.USTAR_FORMAT) as archive:
            for name, data in self.entries() if entries is None else entries:
                member = tarfile.TarInfo(name)
                member.size = len(data)
                archive.addfile(member, io.BytesIO(data))
            if special is not None:
                archive.addfile(special)

    def rejected(self):
        result = subprocess.run([sys.executable, str(LIB / "oci-archive-identity.py"),
                                 str(self.path), self.reference], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertEqual(result.stderr, "OCI archive identity verification failed.\n")

    def test_verified_config_identity_is_not_inferred_from_host_identity(self) -> None:
        self.write()
        self.assertEqual(oci.identity(str(self.path), self.reference), self.config_id)
        self.assertNotEqual(self.config_id, "b" * 64)

    def test_hash_size_schema_reference_and_nonunique_failures(self) -> None:
        for failure in ("config-hash", "manifest-hash", "config-size", "manifest-size", "schema",
                        "reference", "nonunique", "duplicate-json", "invalid-descriptor", "invalid-config"):
            with self.subTest(failure=failure):
                self.setUp()
                if failure == "config-size": self.manifest["config"]["size"] += 1
                if failure == "schema": self.manifest["schemaVersion"] = True
                if failure == "invalid-descriptor": self.manifest["config"]["mediaType"] = oci.MANIFEST
                if failure == "invalid-config":
                    self.config = b'{}'
                    self.config_id = hashlib.sha256(self.config).hexdigest()
                    self.manifest["config"].update(digest="sha256:" + self.config_id, size=len(self.config))
                entries = self.entries()
                if failure == "config-hash": entries[3] = (entries[3][0], b"x" * len(self.config))
                if failure == "manifest-hash": entries[2] = (entries[2][0], b"x" * len(entries[2][1]))
                if failure == "manifest-size": self.index["manifests"][0]["size"] += 1
                if failure == "reference": self.index["manifests"][0]["annotations"]["org.opencontainers.image.ref.name"] += "-other"
                if failure == "nonunique": self.index["manifests"] *= 2
                entries[1] = ("index.json", json.dumps(self.index).encode())
                if failure == "duplicate-json": entries[1] = ("index.json", b'{"schemaVersion":2,"schemaVersion":2}')
                self.write(entries)
                self.rejected()

    def test_duplicate_paths_symlink_special_truncation_and_ambiguity_refused(self) -> None:
        for failure in ("duplicate", "normalized-duplicate", "symlink", "fifo", "pax", "truncated", "trailing"):
            with self.subTest(failure=failure):
                entries, special = self.entries(), None
                if failure == "duplicate": entries.append(entries[0])
                if failure == "normalized-duplicate": entries.append(("./oci-layout", entries[0][1]))
                if failure in ("symlink", "fifo", "pax"):
                    special = tarfile.TarInfo("private-evidence")
                    special.type = {"symlink": tarfile.SYMTYPE, "fifo": tarfile.FIFOTYPE, "pax": tarfile.XHDTYPE}[failure]
                    if failure == "symlink": special.linkname = "index.json"
                self.write(entries, special)
                if failure == "truncated": self.path.write_bytes(self.path.read_bytes()[:100])
                if failure == "trailing": self.path.write_bytes(self.path.read_bytes() + b"unexpected")
                self.rejected()
        self.write()
        link = self.root / "link"
        link.symlink_to(self.path)
        with self.assertRaises(OSError): oci.identity(str(link), self.reference)

    def test_metadata_member_archive_and_time_budgets(self) -> None:
        self.write()
        for constant, limit in (("MAX_METADATA", 1), ("MAX_MEMBERS", 1), ("MAX_ARCHIVE", 512)):
            with self.subTest(constant=constant), mock.patch.object(oci, constant, limit):
                with self.assertRaises(oci.InvalidArchive): oci.identity(str(self.path), self.reference)
        with mock.patch.object(oci.time, "monotonic", side_effect=[0, 61]):
            with self.assertRaises(oci.InvalidArchive): oci.identity(str(self.path), self.reference)

    def test_layer_payloads_are_not_read_and_native_loader_retains_integrity_role(self) -> None:
        layer = b"layer payload is intentionally not metadata"
        layer_id = hashlib.sha256(layer).hexdigest()
        config = json.loads(self.config)
        config["rootfs"]["diff_ids"] = ["sha256:" + "a" * 64]
        self.config = json.dumps(config).encode()
        self.config_id = hashlib.sha256(self.config).hexdigest()
        self.manifest["config"].update(digest="sha256:" + self.config_id, size=len(self.config))
        self.manifest["layers"] = [{"mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                                   "digest": "sha256:" + layer_id, "size": len(layer)}]
        entries = self.entries() + [("blobs/sha256/" + layer_id, layer)]
        self.write(entries)
        with tarfile.open(self.path) as archive:
            layer_offset = archive.getmember("blobs/sha256/" + layer_id).offset_data
        original_fdopen = os.fdopen

        class MetadataOnlyFile:
            def __init__(self, descriptor, mode):
                self.file = original_fdopen(descriptor, mode)

            def __enter__(self): return self
            def __exit__(self, *args): self.file.close()
            def __getattr__(self, name): return getattr(self.file, name)
            def read(self, size):
                if layer_offset <= self.file.tell() < layer_offset + len(layer):
                    raise AssertionError("layer payload read")
                return self.file.read(size)

        with mock.patch.object(oci.os, "fdopen", MetadataOnlyFile):
            self.assertEqual(oci.identity(str(self.path), self.reference), self.config_id)


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
                    call = {"alias": 'record_run_owned_archive_alias "$workload_image" localhost/boxferry-archive/test/workload:alpine archive',
                            "matrix": "prepare_matrix_image test-cell \"$workload_image\"",
                            "workload": "prepare_workload_archive"}[consumer]
                    body = f"""source {shlex.quote(str(LIB / 'native-presence.sh'))}
engine={shlex.quote(str(native))}
artifact_root={shlex.quote(name)}
workload_archive={shlex.quote(str(root / 'archive'))}
workload_image=example.invalid/workload:1@sha256:{'a' * 64}
run_id=test
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


class ArchiveAliasTests(unittest.TestCase):
    runner = LiveConsumerTests.runner
    function = LiveConsumerTests.function
    bash = LiveConsumerTests.bash
    image_id = "b" * 64
    serialized_config = {"architecture": "amd64", "os": "linux", "rootfs": {"type": "layers", "diff_ids": []}}
    serialized_id = hashlib.sha256(json.dumps(serialized_config).encode()).hexdigest()
    outer_id = "d" * 64
    source = "example.invalid/source:1@sha256:" + "a" * 64
    alias = "localhost/boxferry-archive/test/forgejo:forgejo"
    helpers = ("host_image_id", "archive_host_alias", "record_run_owned_host_image",
               "verify_run_owned_archive_alias", "record_run_owned_archive_alias", "release_run_owned_host_image",
               "release_remaining_run_owned_host_images", "archive_target_id", "verify_archive_image_bindings",
               "restore_nested_archive_alias", "bind_saved_oci_archive_identity", "bind_saved_docker_archive_identity")

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="boxferry-archive-fake-")
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        self.state = self.root / "state.json"
        self.log = self.root / "calls.jsonl"
        self.engine = self.root / "engine"
        self.state.write_text(json.dumps({"host": {self.source: self.image_id}, "nested": {}}))
        self.engine.write_text(f"""#!{sys.executable}
import hashlib, io, json, os, pathlib, sys, tarfile
root = pathlib.Path({str(self.root)!r})
state_path = root / 'state.json'
state = json.loads(state_path.read_text())
args = sys.argv[1:]
with (root / 'calls.jsonl').open('a') as log:
    log.write(json.dumps(args) + '\\n')
scenario = os.environ.get('SCENARIO', '')
images = state['host']
def finish(status=0):
    state_path.write_text(json.dumps(state))
    sys.exit(status)
if args[:2] == ['container', 'inspect']:
    if scenario == 'target-inspect-failed': finish(42)
    observed_id = {'e' * 64!r} if scenario == 'target-replaced' else {self.outer_id!r}
    print({'e' * 64!r} if scenario == 'target-malformed' else observed_id + ' ' +
          ('another-run' if scenario == 'target-wrong-owner' else 'test'))
    finish()
if len(args) > 1 and args[0] == 'exec' and args[1] in ({self.outer_id!r}, 'owned-outer'):
    args = args[2:]
    if args[0] == 'podman': args = args[1:]
    elif 'podman load' in args[-1]: args = ['load', '--input', '/boxferry-workload.tar']
    else: finish()
    images = state['nested']
if images is state['nested'] and args[0] == 'run': finish()
if images is state['nested'] and args[:2] in (['image', 'exists'], ['image', 'inspect']) and '--format' not in args:
    if args[-1].startswith('registry.invalid/boxferry-test/') and args[-1].endswith(':fixture'):
        if scenario == 'assert-first-absent': finish(1)
        if scenario == 'assert-first-error': finish(42)
if args[:2] == ['image', 'exists']:
    if scenario == 'alias-unknown' and args[-1].startswith('localhost/boxferry-archive/'): finish(2)
    if scenario == 'cleanup-unknown': finish(2)
    if scenario == 'cleanup-still-present' and args[-1] not in images: finish(0)
    finish(0 if args[-1] in images else 1)
if args[:2] == ['image', 'inspect']:
    reference = args[-1]
    if reference not in images: finish(1)
    if scenario in ('inspect-failed', 'source-inspect-failed') and reference == {self.source!r}:
        print('PRIVATE/inspect-error', file=sys.stderr); finish(42)
    template = args[args.index('--format') + 1] if '--format' in args else ''
    if template == '{{{{.Id}}}} {{{{.Digest}}}}':
        if scenario == 'identity-malformed': print('PRIVATE/not-an-ID')
        elif scenario == 'identity-failed': finish(42)
        else: print('sha256:' + images[reference] + ' sha256:' + ('c' * 64 if scenario == 'digest-drift' else 'a' * 64))
    elif template == '{{{{.Digest}}}}':
        print('sha256:' + ('c' * 64 if scenario == 'digest-drift' else 'a' * 64))
    elif template == '{{{{.Digest}}}} {{{{.Architecture}}}} {{{{.Os}}}}':
        print('sha256:' + 'a' * 64 + ' amd64 linux')
    else:
        if scenario == 'identity-malformed': print('PRIVATE/not-an-ID')
        elif scenario == 'identity-failed': finish(42)
        elif scenario == 'alias-drift' and reference.startswith('localhost/boxferry-archive/'):
            print('sha256:' + 'c' * 64)
        else: print('sha256:' + images[reference])
    finish()
if args and args[0] == 'tag':
    if images is state['nested'] and scenario == 'retag-failed': finish(42)
    images[args[-1]] = args[-2]
    finish(42 if scenario == 'tag-partial-failed' else 0)
if args[:2] == ['image', 'rm']:
    if scenario == 'remove-failed': finish(42)
    images.pop(args[-1], None)
    finish()
if args and args[0] == 'save':
    if scenario == 'save-failed': finish(42)
    output_index = args.index('--output')
    output = pathlib.Path(args[output_index + 1])
    if args[args.index('--format') + 1] == 'oci-archive':
        config = json.dumps({self.serialized_config!r}).encode()
        config_id = hashlib.sha256(config).hexdigest()
        manifest = json.dumps({{'schemaVersion': 2, 'mediaType': 'application/vnd.oci.image.manifest.v1+json',
            'config': {{'mediaType': 'application/vnd.oci.image.config.v1+json', 'digest': 'sha256:' + config_id, 'size': len(config)}},
            'layers': []}}).encode()
        manifest_id = hashlib.sha256(manifest).hexdigest()
        index = json.dumps({{'schemaVersion': 2, 'manifests': [{{'mediaType': 'application/vnd.oci.image.manifest.v1+json',
            'digest': 'sha256:' + manifest_id, 'size': len(manifest),
            'annotations': {{'org.opencontainers.image.ref.name': args[-1]}}}}]}}).encode()
        with tarfile.open(output, 'w', format=tarfile.USTAR_FORMAT) as bundle:
            for name, data in [('oci-layout', b'{{"imageLayoutVersion":"1.0.0"}}'), ('index.json', index),
                               ('blobs/sha256/' + manifest_id, manifest), ('blobs/sha256/' + config_id,
                                config + b' ' if scenario == 'oci-save-tampered' else config)]:
                member = tarfile.TarInfo(name); member.size = len(data)
                bundle.addfile(member, io.BytesIO(data))
        if scenario == 'oci-save-alias-drift': images[args[-1]] = 'c' * 64
    else:
        output.write_text(json.dumps({{ref: images[ref] for ref in args[output_index + 2:]}}))
    finish()
if args and args[0] == 'load':
    if scenario == 'load-failed': finish(42)
    archive = pathlib.Path(os.environ['TEST_ARCHIVE'])
    if tarfile.is_tarfile(archive):
        with tarfile.open(archive) as bundle:
            for member in bundle:
                if member.isfile():
                    raw = bundle.extractfile(member).read()
                    with tarfile.open(fileobj=io.BytesIO(raw)) as image:
                        index = json.load(image.extractfile('index.json'))
                        descriptor = index['manifests'][0]
                        manifest = json.load(image.extractfile('blobs/sha256/' + descriptor['digest'][7:]))
                        state['nested'][descriptor['annotations']['org.opencontainers.image.ref.name']] = manifest['config']['digest'][7:]
    else: state['nested'].update(json.loads(archive.read_text()))
    finish()
if args and args[0] == 'pull': finish(42)
if args and args[0] == 'exec': finish()
finish(99)
""", encoding="utf-8")
        self.engine.chmod(0o700)

    def script(self, body: str, scenario: str = "") -> subprocess.CompletedProcess[str]:
        return self.bash(f"""engine={shlex.quote(str(self.engine))}
export SCENARIO={shlex.quote(scenario)}
export TEST_ARCHIVE={shlex.quote(str(self.root / 'archive'))}
run_id=test
runtime_root={shlex.quote(str(self.root))}
artifact_root={shlex.quote(str(self.root))}
script_directory={shlex.quote(str(LIB.parent))}
outer_containers=(owned-outer)
declare -a run_owned_host_images=()
declare -A run_owned_host_image_seen=() run_owned_host_image_id=() archive_image_id=() archive_loaded_image_id=()
declare -A archive_target_expected_id=([owned-outer]={self.outer_id})
native_presence_unverified=false
engine_operation() {{ shift; "$engine" "$@"; }}
engine_image_available() {{ shift; "$engine" image exists "$1"; }}
timed_operation() {{ shift 2; "$@"; }}
""" + "".join(self.function(name) for name in self.helpers) + body)

    def calls(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []

    def test_aliases_are_run_unique_and_have_bounded_grammar_and_tag_lengths(self) -> None:
        result = self.script("archive_host_alias forgejo forgejo\nrun_id=other\narchive_host_alias forgejo forgejo\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines(), [self.alias, "localhost/boxferry-archive/other/forgejo:forgejo"])
        for run, suite, identity in (("../other", "forgejo", "forgejo"), ("A", "forgejo", "forgejo"),
                                     ("a" * 97, "forgejo", "forgejo"), ("test", "supabase", "id"),
                                     ("test", "forgejo", "../id"), ("test", "forgejo", "a" * 65)):
            with self.subTest(run=run, suite=suite, identity=identity):
                result = self.script(f"run_id={shlex.quote(run)}\narchive_host_alias {shlex.quote(suite)} {shlex.quote(identity)}\n")
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(result.stdout, "")
        result = self.script(f"run_id={'a' * 96}\narchive_host_alias forgejo {'b' * 64}\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(result.stdout.strip().rsplit(':', 1)[1]), 64)

    def test_present_unknown_wrong_namespace_and_bad_source_never_authorize_tagging(self) -> None:
        for scenario in ("present", "alias-unknown", "digest-drift", "source-inspect-failed", "identity-malformed"):
            with self.subTest(scenario=scenario):
                self.state.write_text(json.dumps({"host": {self.source: self.image_id, **({self.alias: "c" * 64} if scenario == "present" else {})}, "nested": {}}))
                before = len(self.calls())
                result = self.script(f'record_run_owned_archive_alias {shlex.quote(self.source)} {self.alias} Forgejo\n', scenario)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(any(call[0] == "tag" for call in self.calls()[before:]))
                self.assertNotIn("PRIVATE", result.stdout + result.stderr)
        result = self.script(f'record_run_owned_archive_alias {shlex.quote(self.source)} localhost/boxferry-archive/other/forgejo:forgejo Forgejo\n')
        self.assertEqual(result.returncode, 1, result.stderr)

    def test_partial_tag_failure_is_registered_and_matching_cleanup_is_exact(self) -> None:
        result = self.script(f"""state=0
record_run_owned_archive_alias {shlex.quote(self.source)} {self.alias} Forgejo || state=$?
printf 'tag=%s expected=%s\\n' "$state" "${{run_owned_host_image_id[{self.alias}]}}"
export SCENARIO=
release_remaining_run_owned_host_images
""", "tag-partial-failed")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, f"tag=42 expected={self.image_id}\n")
        self.assertNotIn(self.alias, json.loads(self.state.read_text())["host"])
        self.assertEqual(json.loads(self.state.read_text())["host"][self.source], self.image_id)
        self.assertIn(["tag", self.image_id, self.alias], self.calls())
        self.assertIn(["image", "rm", "--ignore", "--no-prune", "--", self.alias], self.calls())

    def test_alias_and_source_cleanup_refuse_identity_drift_unknown_and_remove_errors(self) -> None:
        for reference in (self.alias, self.source):
            for scenario in ("actual-drift", "alias-drift", "cleanup-unknown", "identity-failed", "remove-failed", "cleanup-still-present"):
                with self.subTest(reference=reference, scenario=scenario):
                    host = {self.source: self.image_id, self.alias: self.image_id}
                    if scenario == "actual-drift": host[reference] = "c" * 64
                    self.state.write_text(json.dumps({"host": host, "nested": {}}))
                    before = len(self.calls())
                    result = self.script(f"""record_run_owned_host_image {shlex.quote(reference)} {self.image_id}
state=0
release_run_owned_host_image {shlex.quote(reference)} || state=$?
printf 'release=%s owned=%s uncertain=%s\\n' "$state" "${{run_owned_host_image_seen[{reference}]:-false}}" "$native_presence_unverified"
""", scenario)
                    if scenario == "alias-drift" and reference == self.source:
                        self.assertIn("release=0 owned=false", result.stdout)
                    else:
                        self.assertNotIn("release=0", result.stdout)
                        self.assertIn("owned=true", result.stdout)
                        self.assertIn("uncertain=true", result.stdout)
                    if scenario in ("actual-drift", "cleanup-unknown", "identity-failed") or scenario == "alias-drift" and reference == self.alias:
                        self.assertFalse(any(call[:2] == ["image", "rm"] for call in self.calls()[before:]))
                    self.assertNotIn("PRIVATE", result.stdout + result.stderr)

    def test_loaded_alias_retags_only_authenticated_target_and_removes_host_only_tag(self) -> None:
        stable = "registry.invalid/boxferry-test/forgejo-application:forgejo"
        self.state.write_text(json.dumps({"host": {self.source: self.image_id}, "nested": {self.alias: self.image_id}}))
        result = self.script(f"archive_loaded_image_id[{self.alias}]={self.image_id}\nrestore_nested_archive_alias owned-outer {self.alias} {stable}\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(self.state.read_text())["nested"], {stable: self.image_id})
        self.assertIn(["exec", self.outer_id, "podman", "tag", self.image_id, stable], self.calls())
        for scenario in ("target-inspect-failed", "target-malformed", "target-wrong-owner", "target-replaced", "alias-drift", "retag-failed"):
            with self.subTest(scenario=scenario):
                self.state.write_text(json.dumps({"host": {}, "nested": {self.alias: self.image_id}}))
                result = self.script(f"archive_loaded_image_id[{self.alias}]={self.image_id}\nstate=0\nrestore_nested_archive_alias owned-outer {self.alias} {stable} || state=$?\nprintf 'state=%s\\n' \"$state\"\n", scenario)
                self.assertNotIn("state=0", result.stdout)
                self.assertNotIn(stable, json.loads(self.state.read_text())["nested"])

    def test_archive_target_requires_create_bound_immutable_id(self) -> None:
        for binding in ("unset 'archive_target_expected_id[owned-outer]'", "archive_target_expected_id[owned-outer]=unknown"):
            with self.subTest(binding=binding):
                before = len(self.calls())
                result = self.script(binding + "\narchive_target_id owned-outer\n")
                self.assertNotEqual(result.returncode, 0, result.stderr)
                self.assertEqual(self.calls()[before:], [])
        creation = self.function("start_outer_runtime")
        stdout = '> "${artifact_root}/${id}.outer.log"'
        readback = 'created_id="$(< "${artifact_root}/${id}.outer.log")"'
        validation = '[[ "${created_id}" =~ ^[0-9a-f]{64}$ ]]'
        binding = 'archive_target_expected_id[${outer}]="${created_id}"'
        self.assertLess(creation.index(stdout), creation.index(readback))
        self.assertLess(creation.index(readback), creation.index(validation))
        self.assertLess(creation.index(validation), creation.index(binding))
        self.assertLess(creation.index(binding), creation.index('wait_for_file'))

    def module_function(self, suite: str, name: str) -> str:
        source = (LIB / f"{suite}-application.sh").read_text()
        start = source.index(name + "() {")
        return source[start:source.index("\n}\n", start) + 3]

    def application_script(self, suite: str, preparation_scenario: str = "", load_scenario: str = "", *, rows=("fixture",)):
        archive = self.root / "archive"
        if archive.exists(): archive.unlink()
        historical = f"registry.invalid/boxferry-test/{suite}-application:fixture"
        self.state.write_text(json.dumps({"host": {self.source: self.image_id, historical: "c" * 64}, "nested": {}}))
        (self.root / "images.tsv").write_text("".join(f"{identity}\t{self.source}\tlicense\tprovenance\turl\tpolicy" +
                                              ("\tlinux/amd64" if suite == "observability" else "") + "\n" for identity in rows))
        load_function = f"{suite}_load_image_archive" if suite in ("forgejo", "nextcloud") else f"{suite}_prepare_application_target"
        body = "".join(self.module_function(suite, name) for name in
                       (f"{suite}_prepare_image_archive", f"{suite}_image_reference", f"{suite}_assert_loaded_images", load_function))
        body += f"""
{suite.upper()}_ARCHIVE_MAX_BYTES=2684354560
repository_root={shlex.quote(str(self.root))}
{suite}_fixture_root() {{ printf '%s\\n' {shlex.quote(str(self.root))}; }}
activate_outer_runtime() {{ printf 'activated\\n'; }}
prepare_status=0
{suite}_prepare_image_archive {shlex.quote(str(archive))} || prepare_status=$?
printf 'prepare=%s\\n' "$prepare_status"
if ((prepare_status != 0)); then exit "$prepare_status"; fi
export SCENARIO={shlex.quote(load_scenario)}
load_status=0
{load_function} owned-outer prefix /unused || load_status=$?
printf 'load=%s\\n' "$load_status"
if ((load_status != 0)); then exit "$load_status"; fi
printf 'provisioning-admitted\\n'
"""
        return self.script(body, preparation_scenario)

    def test_all_application_consumers_save_unique_aliases_and_restore_stable_nested_references(self) -> None:
        for suite in ("forgejo", "nextcloud", "paperless", "immich", "observability"):
            with self.subTest(suite=suite):
                before = len(self.calls())
                result = self.application_script(suite)
                self.assertEqual(result.returncode, 0, result.stderr)
                alias = f"localhost/boxferry-archive/test/{suite}:fixture"
                stable = f"registry.invalid/boxferry-test/{suite}-application:fixture"
                expected_id = self.serialized_id if suite in ("paperless", "immich", "observability") else self.image_id
                self.assertEqual(json.loads(self.state.read_text())["nested"], {stable: expected_id})
                self.assertEqual(json.loads(self.state.read_text())["host"], {self.source: self.image_id, stable: "c" * 64})
                saves = [call for call in self.calls()[before:] if call[0] == "save"]
                self.assertEqual(len(saves), 1)
                self.assertEqual(saves[0][saves[0].index('--output') + 2:], [alias])
                self.assertNotIn(stable, saves[0])

    def test_all_application_consumer_errors_remain_failures_under_conditional_invocation(self) -> None:
        for suite in ("forgejo", "nextcloud", "paperless", "immich", "observability"):
            for preparation_scenario, load_scenario in (("tag-partial-failed", ""), ("save-failed", ""),
                                                       ("alias-drift", ""), ("", "load-failed"), ("", "retag-failed")):
                with self.subTest(suite=suite, preparation=preparation_scenario, load=load_scenario):
                    result = self.application_script(suite, preparation_scenario, load_scenario)
                    self.assertNotEqual(result.returncode, 0, result.stderr)
                    self.assertNotIn("activated", result.stdout)
                    if preparation_scenario: self.assertNotIn("load=", result.stdout)
                    else: self.assertIn("load=42", result.stdout)

    def test_oci_consumers_refuse_tampered_save_and_post_save_host_drift(self) -> None:
        for suite in ("paperless", "immich", "observability"):
            for failure in ("oci-save-tampered", "oci-save-alias-drift"):
                with self.subTest(suite=suite, failure=failure):
                    before = len(self.calls())
                    result = self.application_script(suite, failure)
                    self.assertNotEqual(result.returncode, 0, result.stderr)
                    self.assertNotIn("provisioning-admitted", result.stdout)
                    self.assertEqual(json.loads(self.state.read_text())["nested"], {})
                    self.assertFalse(any(call[0] == "exec" for call in self.calls()[before:]))

    def test_oci_host_identity_without_serialized_binding_never_authorizes_load(self) -> None:
        for suite in ("paperless", "immich", "observability"):
            with self.subTest(suite=suite):
                alias = f"localhost/boxferry-archive/test/{suite}:fixture"
                (self.root / "images.tsv").write_text(f"fixture\t{self.source}\n")
                before = len(self.calls())
                body = self.module_function(suite, f"{suite}_prepare_application_target") + f"""
{suite}_fixture_root() {{ printf '%s\\n' {shlex.quote(str(self.root))}; }}
archive_image_id[{alias}]={self.image_id}
{suite}_prepare_application_target owned-outer prefix /unused
"""
                result = self.script(body)
                self.assertNotEqual(result.returncode, 0, result.stderr)
                self.assertEqual(self.calls()[before:], [])

    def test_saved_oci_binding_keeps_pure_stdout_and_original_host_cleanup_identity(self) -> None:
        alias = "localhost/boxferry-archive/test/paperless:fixture"
        result = self.script(f"""
record_run_owned_archive_alias {shlex.quote(self.source)} {alias} fixture
"$engine" save --format oci-archive --output "$TEST_ARCHIVE" {alias}
source {shlex.quote(str(LIB / 'timed-operation.sh'))}
timestamp() {{ printf test; }}
format_duration() {{ printf '0s'; }}
bind_saved_oci_archive_identity {alias} "$TEST_ARCHIVE"
printf 'host=%s loaded=%s\\n' "${{archive_image_id[{alias}]}}" "${{archive_loaded_image_id[{alias}]}}"
release_run_owned_host_image {alias}
""")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, f"host={self.image_id} loaded={self.serialized_id}\n")
        self.assertIn("STEP PASS", result.stderr)
        self.assertEqual(json.loads(self.state.read_text())["host"], {self.source: self.image_id})

    def test_real_multi_row_loaded_assertions_stop_on_first_absence_or_query_error(self) -> None:
        for suite in ("forgejo", "nextcloud", "paperless", "immich", "observability"):
            for scenario, expected in (("", 0), ("assert-first-absent", 1), ("assert-first-error", 42)):
                with self.subTest(suite=suite, scenario=scenario):
                    before = len(self.calls())
                    result = self.application_script(suite, load_scenario=scenario, rows=("fixture", "later"))
                    self.assertEqual(result.returncode, expected, result.stderr)
                    stable_prefix = f"registry.invalid/boxferry-test/{suite}-application:"
                    expected_id = self.serialized_id if suite in ("paperless", "immich", "observability") else self.image_id
                    self.assertEqual(json.loads(self.state.read_text())["nested"],
                                     {stable_prefix + identity: expected_id for identity in ("fixture", "later")})
                    assertions = [call for call in self.calls()[before:] if len(call) >= 6 and call[:3] == ["exec", "owned-outer", "podman"]
                                  and call[3:5] in (["image", "exists"], ["image", "inspect"]) and '--format' not in call]
                    self.assertEqual([call[-1] for call in assertions],
                                     [stable_prefix + identity for identity in (("fixture",) if expected else ("fixture", "later"))])
                    if expected:
                        self.assertNotIn("activated", result.stdout)
                        self.assertNotIn("provisioning-admitted", result.stdout)
                        copies = [call for call in self.calls()[before:] if call and call[0] == "cp"
                                  and "/tmp/boxferry-fixture/" in call[-1]]
                        self.assertEqual(copies, [])

    def test_workload_archive_uses_only_unique_host_alias_and_identity_bound_save(self) -> None:
        body = self.function("prepare_workload_archive") + f"""
workload_archive={shlex.quote(str(self.root / 'archive'))}
workload_image={shlex.quote(self.source)}
workload_local_tag=localhost/boxferry-live/alpine:{'a' * 64}
state=0
prepare_workload_archive || state=$?
printf 'state=%s\\n' "$state"
"""
        stable = "localhost/boxferry-live/alpine:" + "a" * 64
        for scenario in ("", "tag-partial-failed", "save-failed"):
            with self.subTest(scenario=scenario):
                archive = self.root / "archive"
                if archive.exists(): archive.unlink()
                self.state.write_text(json.dumps({"host": {self.source: self.image_id, stable: "c" * 64}, "nested": {}}))
                before = len(self.calls())
                result = self.script(body, scenario)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.strip().splitlines()[-1], "state=42" if scenario else "state=0")
                saves = [call for call in self.calls()[before:] if call[0] == "save"]
                if scenario == "tag-partial-failed": self.assertEqual(saves, [])
                else: self.assertEqual(saves[0][-1], "localhost/boxferry-archive/test/workload:alpine")
                self.assertEqual(json.loads(self.state.read_text())["host"][stable], "c" * 64)

    def test_source_pull_ownership_retains_immutable_id_and_rejects_malformed_readback(self) -> None:
        for scenario in ("", "digest-drift", "identity-malformed", "identity-failed"):
            with self.subTest(scenario=scenario):
                result = self.script(f"""state=0
record_run_owned_host_image {shlex.quote(self.source)} || state=$?
printf 'state=%s id=%s uncertain=%s\\n' "$state" "${{run_owned_host_image_id[{self.source}]}}" "$native_presence_unverified"
""", scenario)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn(f"state=0 id={self.image_id}" if not scenario else "state=2 id=unknown uncertain=true", result.stdout)
                self.assertNotIn("PRIVATE", result.stdout + result.stderr)

    def test_common_workload_load_retag_and_apply_target_order_remain_before_api_activation(self) -> None:
        workloads = self.function("create_workloads")
        self.assertLess(workloads.index("podman load --input"), workloads.index('podman tag "${BF_ARCHIVE_IMAGE_ID}" "${image}"'))
        self.assertLess(workloads.index('podman tag "${BF_ARCHIVE_IMAGE_ID}" "${image}"'), workloads.index('podman tag "${image}" "${portable_image}"'))
        self.assertIn('podman image rm --no-prune -- "${BF_ARCHIVE_ALIAS}"', workloads)
        apply_target = self.function("start_apply_target")
        self.assertLess(apply_target.index("podman load --input"), apply_target.index("restore_nested_archive_alias"))
        self.assertLess(apply_target.index("restore_nested_archive_alias"), apply_target.index("tag apply-target workload"))
        self.assertNotIn("activate_outer_runtime", apply_target)

    def test_cleanup_attempts_later_registered_images_after_identity_mismatch(self) -> None:
        later = "localhost/boxferry-archive/test/forgejo:later"
        self.state.write_text(json.dumps({"host": {self.alias: self.image_id, later: "c" * 64}, "nested": {}}))
        result = self.script(f"""record_run_owned_host_image {self.alias} {self.image_id}
record_run_owned_host_image {later} {self.image_id}
state=0
release_remaining_run_owned_host_images || state=$?
printf 'state=%s\\n' "$state"
""")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "state=1\n")
        self.assertEqual(json.loads(self.state.read_text())["host"], {later: "c" * 64})

    def test_missing_or_unbound_catalogue_and_fixture_discovery_errors_never_reach_load(self) -> None:
        for suite in ("forgejo", "nextcloud", "paperless", "immich", "observability"):
            for failure in ("discovery", "missing-catalogue", "unbound"):
                with self.subTest(suite=suite, failure=failure):
                    (self.root / "images.tsv").write_text("fixture\tpin\n")
                    if failure == "missing-catalogue": (self.root / "images.tsv").unlink()
                    load_function = f"{suite}_load_image_archive" if suite in ("forgejo", "nextcloud") else f"{suite}_prepare_application_target"
                    before = len(self.calls())
                    body = self.module_function(suite, load_function) + f"""
{suite}_fixture_root() {{ printf '%s\\n' {shlex.quote(str(self.root))}; return {42 if failure == 'discovery' else 0}; }}
state=0
{load_function} owned-outer prefix /unused || state=$?
printf 'state=%s\\n' "$state"
"""
                    result = self.script(body)
                    self.assertEqual(result.stdout, f"state={42 if failure == 'discovery' else 1}\n")
                    self.assertEqual(self.calls()[before:], [])

    def test_image_identity_uncertainty_preserves_private_recovery_and_original_failure(self) -> None:
        runtime = self.root / "runtime"
        runtime.mkdir()
        recovery = runtime / "private-recovery"
        recovery.write_bytes(b"PRIVATE/immutable-ownership")
        self.state.write_text(json.dumps({"host": {self.alias: "c" * 64}, "nested": {}}))
        body = self.function("cleanup") + f"""
runtime_root={shlex.quote(str(runtime))}
profile=smoke
outer_containers=() mounted_images=() fault_proxy_pids=() fault_proxy_sockets=() discovery_directories=()
discovery_parent_created=false
retain_artifacts=false
ignore_outer_cleanup_signals() {{ :; }}
record_run_owned_host_image {self.alias} {self.image_id}
set +e
(exit 42)
cleanup
"""
        result = self.script(body)
        self.assertEqual(result.returncode, 42, result.stderr)
        self.assertEqual(recovery.read_bytes(), b"PRIVATE/immutable-ownership")
        self.assertIn("recovery evidence retained", result.stderr)
        self.assertEqual(json.loads(self.state.read_text())["host"], {self.alias: "c" * 64})
        self.assertNotIn("PRIVATE", result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
