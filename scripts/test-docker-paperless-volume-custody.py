#!/usr/bin/env python3
"""Offline real-FD custody controls. Root/proc fixtures are NOT native evidence."""
from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import os
import pathlib
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import threading
import time
import types
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


custody = load("custody_under_test", ROOT / "scripts/lib/docker-paperless-volume-custody.py")
contract = load("custody_owner_contract", ROOT / "scripts/lib/docker-application-contract.py")
transport = load("custody_transport", ROOT / "scripts/lib/docker-paperless-transport.py")


def raw(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def fd_count():
    return len(os.listdir("/proc/self/fd"))


class DirectoryControls(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="paperless-real-fd-")
        self.addCleanup(self.temporary.cleanup)
        self.base = pathlib.Path(self.temporary.name)
        self.root = self.base / "root"
        self.graph = self.root / custody.GRAPH
        self.graph.mkdir(parents=True)
        self.prefix = "source"
        for role in custody.ROLES:
            path = self.role(role)
            path.mkdir(parents=True)
            (path / "unchanged").write_bytes(b"contents-not-read")
        binary = self.root / "usr/local/bin/dockerd"
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b"test-only-executable-object")
        binary.chmod(0o700)
        self.proc = self.base / "proc-fixture"
        self.proc.mkdir()
        (self.proc / "root").symlink_to(self.root)
        (self.proc / "exe").symlink_to(binary)
        (self.proc / "ns").mkdir()
        for name in ("user", "mnt"):
            (self.proc / "ns" / name).symlink_to("/proc/self/ns/" + name)
        fields = pathlib.Path("/proc/self/stat").read_bytes().rsplit(b")", 1)[1]
        (self.proc / "stat").write_bytes(b"1 (dockerd)" + fields)
        (self.proc / "status").write_bytes((f"Uid:\t{os.getuid()} {os.getuid()} {os.getuid()} {os.getuid()}\n"
            f"Gid:\t{os.getgid()} {os.getgid()} {os.getgid()} {os.getgid()}\n").encode())
        (self.proc / "cmdline").write_bytes(b"dockerd\0--rootless\0--host=unix:///boxferry-core/docker.sock\0")
        (self.proc / "net").mkdir()
        (self.proc / "net/unix").write_bytes(b"Num RefCount Protocol Flags Type St Inode Path\n"
            b"0: 2 0 00010000 0001 01 1234 /boxferry-core/docker.sock\n")
        (self.proc / "fd").mkdir()
        (self.proc / "fd/7").symlink_to("socket:[1234]")
        (self.proc / "mountinfo").write_bytes(
            b"1 0 0:1 / / rw - ext4 fixture rw\n2 1 0:2 / /" + custody.GRAPH.encode() + b" rw - ext4 fixture rw\n"
            b"3 2 0:3 / /" + custody.GRAPH.encode() + b"/overlay2/canonical/merged rw - overlay fixture rw\n")
        self.socket_directory = self.base / "socket"
        self.socket_directory.mkdir(mode=0o777)
        self.socket_directory.chmod(0o777)  # fixture setup, never acquisition
        self.listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.listener.bind(str(self.socket_directory / "docker.sock"))
        self.listener.listen(8)
        self.listener.settimeout(.02)
        self.stopped = threading.Event()
        def accept():
            while not self.stopped.is_set():
                try:
                    stream, _ = self.listener.accept()
                except socket.timeout:
                    continue
                stream.close()
        self.thread = threading.Thread(target=accept)
        self.thread.start()
        self.addCleanup(self.stop_socket)
        directory_info = self.socket_directory.stat()
        socket_info = transport.socket_stamp(self.socket_directory / "docker.sock")
        peer = {"pid": os.getpid(), "start": contract.process_start(os.getpid()),
                "uid": os.getuid(), "gid": os.getgid()}
        self.owner = {"directory": str(self.base), "socket_directory": str(self.socket_directory),
            "directory_identity": f"{self.base.stat().st_dev}:{self.base.stat().st_ino}",
            "socket_identity": f"{directory_info.st_dev}:{directory_info.st_ino}", "prefix": self.prefix,
            "run": "source-run", "context": "owned", "volume_path": str(self.graph),
            "graph_root": "/outer/unrelated", "outer_name": "owned-core", "outer_owner": "owned-label",
            "outer_run": "owned-run", "outer_image": "fixed-image", "volume_name": "owned-volume",
            "execution_deadline": contract.owner_clock() + 30, "cleanup_deadline": contract.owner_clock() + 40,
            "paperless_sources": {"authored": "original"}}
        manifest = self.base / "boxferry-paperless.ABCDEFGH/manifest.json"
        manifest.parent.mkdir(mode=0o700)
        manifest.write_bytes(raw({"config": {"stage": "source-fixture", "target": None, "prefix": self.prefix,
            "run_id": self.owner["run"], "work_deadline_ms": time.time_ns() // 1_000_000 + 20_000,
            "endpoint": {"id": "owned", "uri": "unix://" + str(self.socket_directory / "docker.sock"),
                         "scope": "owned-inner", "mode": "rootless", "ownership_sha256": "a" * 64}}}))
        manifest.chmod(0o600)
        self.binding = {"arguments": {"custody_path": str(self.base / "owner-custody.json"),
            "custody_sha256": "a" * 64, "source_hashes": self.owner["paperless_sources"], "inner_peer": peer,
            "manifest_path": str(manifest), "manifest_sha256": hashlib.sha256(manifest.read_bytes()).hexdigest(),
            "socket_identity": socket_info}, "owner": self.owner, "protocol_identity": [], "directory_identity": [],
            "lock_identity": [], "cutoff": contract.owner_clock() + 20, "cleanup_cutoff": contract.owner_clock() + 35,
            "ledger_key": "b" * 64}
        binding = self.base / "paperless-transport/binding.json"
        binding.parent.mkdir(mode=0o700)
        binding.write_bytes(raw(self.binding))
        binding.chmod(0o600)
        self.binding_sha = hashlib.sha256(binding.read_bytes()).hexdigest()
        self.real_open, self.real_fstat = os.open, os.fstat
        def fixed_open(path, flags, *args, **kwargs):
            if path == f"/proc/{os.getpid()}/root":
                path = self.proc / "root"
            return self.real_open(path, flags, *args, **kwargs)
        def root_socket_directory(fd):
            value = self.real_fstat(fd)
            if (value.st_dev, value.st_ino) == (directory_info.st_dev, directory_info.st_ino):
                fields = list(value)
                fields[4] = 0
                return os.stat_result(fields)
            return value
        # Only kernel/root admission and the proc fixture location are mocked.
        # The production snapshot, traversal, six FDs, current-root and closure
        # comparisons, counters, exceptions and close paths execute unchanged.
        for patch in (mock.patch.object(custody, "root_authority"),
                      mock.patch.object(custody.Lease, "process", lambda _, _pid: self.real_open(self.proc, custody.OPATH)),
                      mock.patch.object(custody.os, "open", side_effect=fixed_open),
                      mock.patch.object(custody.os, "fstat", side_effect=root_socket_directory)):
            patch.start()
            self.addCleanup(patch.stop)
        def observe(_owner, arguments, _deadline, **_kwargs):
            if arguments[0] == "observe":
                return 0, raw({"Id": "c" * 64, "Name": "owned-core", "State": {"Running": True, "Pid": os.getpid()}})
            return 0, b""
        def parent(directory, expected):
            info = directory.lstat()
            custody.need(directory == self.base and directory.resolve() == directory
                and f"{info.st_dev}:{info.st_ino}" == expected and stat.S_IMODE(info.st_mode) == 0o700
                and info.st_uid == os.geteuid())
        self.api = types.SimpleNamespace(owner_clock=contract.owner_clock, owner_lifecycle=lambda *_args: None,
            outer_run_directory=parent,
            no_duplicate_keys=contract.no_duplicate_keys, outer_runtime_bytes=lambda path, **_kwargs: pathlib.Path(path).read_bytes(),
            owner_call=observe, owner_cid=lambda _: "c" * 64, verified_parent_fd=contract.verified_parent_fd,
            parent_alive=contract.parent_alive)
        self.leases = []
        self.addCleanup(lambda: [lease.close() for lease in self.leases])

    def stop_socket(self):
        self.stopped.set()
        self.thread.join(timeout=2)
        self.listener.close()

    def role(self, role):
        return self.graph / ("volumes/" + self.prefix + "-paper-" + role + "/_data")

    def lease(self):
        value = custody.Lease(self.api, self.owner, self.binding_sha, "a" * 64, {})
        self.leases.append(value)
        return value

    def test_all_six_retained_objects_and_no_storage_mutations(self):
        before = {str(path): (path.stat().st_mode, path.stat().st_uid, path.stat().st_gid, path.read_bytes())
                  for path in self.graph.rglob("unchanged")}
        count = fd_count()
        lease = self.lease()
        self.assertEqual(len(lease.fds), 17)
        for _ in range(3):
            lease.check()
        self.assertGreaterEqual(lease.calls, 3 * 5)
        self.assertGreater(lease.read_bytes, 0)
        lease.close()
        self.assertEqual(lease.fds, {})
        self.assertEqual(fd_count(), count)
        self.assertEqual(before, {str(path): (path.stat().st_mode, path.stat().st_uid, path.stat().st_gid, path.read_bytes())
                                 for path in self.graph.rglob("unchanged")})
        with self.assertRaises(custody.Refused):
            lease.check()

    def test_replace_every_role_with_recreated_metadata_refuses_and_closes(self):
        for role in custody.ROLES:
            with self.subTest(role=role):
                count = fd_count()
                lease = self.lease()
                path = self.role(role)
                retained = path.with_name("retained")
                path.rename(retained)
                path.mkdir(mode=stat.S_IMODE(retained.stat().st_mode))
                with self.assertRaises(custody.Refused):
                    lease.check()
                self.assertEqual(lease.fds, {})
                self.assertEqual(fd_count(), count)
                path.rmdir()
                retained.rename(path)

    def test_symlink_alias_and_ordinary_component_refuse_without_fd_leaks(self):
        for role in custody.ROLES:
            with self.subTest(role=role):
                path = self.role(role)
                retained = path.with_name("retained")
                path.rename(retained)
                path.symlink_to(self.role("media" if role != "media" else "data"))
                count = fd_count()
                with self.assertRaises(OSError):
                    self.lease()
                self.assertEqual(fd_count(), count)
                path.unlink()
                retained.rename(path)
        graph = self.graph
        moved = graph.with_name("original")
        graph.rename(moved)
        graph.symlink_to(moved)
        count = fd_count()
        with self.assertRaises(OSError):
            self.lease()
        self.assertEqual(fd_count(), count)

    def test_short_proc_reads_reach_eof_and_charge_every_observed_byte(self):
        lease = self.lease()
        expected = (self.proc / "mountinfo").read_bytes()
        original = os.read
        before = lease.read_bytes
        with mock.patch.object(custody.os, "read", side_effect=lambda fd, size: original(fd, min(size, 7))):
            self.assertEqual(lease.read(lease.fds["proc"], "mountinfo", 65536), expected)
        self.assertEqual(lease.read_bytes - before, len(expected))

    def test_oversize_short_proc_reads_charge_the_probe_and_close_temporary_fd(self):
        lease = self.lease()
        (self.proc / "bounded-inventory").write_bytes(b"x" * 17)
        count, before = fd_count(), lease.read_bytes
        original = os.read
        with mock.patch.object(custody.os, "read", side_effect=lambda fd, size: original(fd, min(size, 7))):
            with self.assertRaises(custody.Refused):
                lease.read(lease.fds["proc"], "bounded-inventory", 16)
        self.assertEqual(lease.read_bytes - before, 17)
        self.assertEqual(fd_count(), count)

    def test_short_proc_read_cannot_hide_later_volume_submount(self):
        lease = self.lease()
        mountinfo = self.proc / "mountinfo"
        original_length = len(mountinfo.read_bytes())
        with mountinfo.open("ab") as output:
            output.write(b"4 2 0:4 / /" + custody.GRAPH.encode()
                         + b"/volumes/source-paper-pgdata/_data rw - ext4 fixture rw\n")
        original = os.read
        with mock.patch.object(custody.os, "read",
                               side_effect=lambda fd, size: original(fd, min(size, original_length))):
            with self.assertRaises(custody.Refused):
                lease.check()
        self.assertTrue(lease.closed)
        self.assertEqual(lease.fds, {})

    def test_actual_old_atime_socket_acquires_all_handles_without_warming(self):
        path = self.socket_directory / "docker.sock"
        original = path.lstat()
        os.utime(path, ns=(1_000_000_000, original.st_mtime_ns))
        before = path.lstat()
        lease = self.lease()
        self.assertEqual(len(lease.fds), 17)
        after = path.lstat()
        self.assertEqual((before.st_dev, before.st_ino, before.st_mode, before.st_uid, before.st_gid,
                          before.st_nlink, before.st_size, before.st_mtime_ns, before.st_ctime_ns),
                         (after.st_dev, after.st_ino, after.st_mode, after.st_uid, after.st_gid,
                          after.st_nlink, after.st_size, after.st_mtime_ns, after.st_ctime_ns))
        lease.close()
        self.assertTrue(lease.closed)
        self.assertEqual(lease.fds, {})

    def test_socket_security_field_change_after_connect_refuses(self):
        fields = ("st_dev", "st_ino", "st_mode", "st_uid", "st_gid", "st_nlink", "st_size",
                  "st_mtime_ns", "st_ctime_ns")
        path = self.socket_directory / "docker.sock"
        original_lstat = pathlib.Path.lstat
        for field in fields:
            calls = 0
            def changed(pathname, *arguments, **keywords):
                nonlocal calls
                info = original_lstat(pathname, *arguments, **keywords)
                if pathname == path:
                    calls += 1
                    if calls == 2:
                        values = {name: getattr(info, name) for name in fields}
                        values[field] ^= 0o020 if field == "st_mode" else 1
                        return types.SimpleNamespace(**values)
                return info
            with self.subTest(field=field), mock.patch.object(pathlib.Path, "lstat", new=changed):
                with self.assertRaises(custody.Refused):
                    self.lease()
            self.assertEqual(calls, 2)

    def test_replaced_socket_path_refuses_even_with_same_kernel_peer(self):
        path = self.socket_directory / "docker.sock"
        original_connect = socket.socket.connect
        replaced = False
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as replacement:
            def connect(stream, address):
                nonlocal replaced
                result = original_connect(stream, address)
                if address == str(path) and not replaced:
                    path.rename(self.socket_directory / "original.sock")
                    replacement.bind(str(path))
                    replacement.listen(1)
                    replaced = True
                return result
            with mock.patch.object(socket.socket, "connect", new=connect):
                with self.assertRaises(custody.Refused):
                    self.lease()
        self.assertTrue(replaced)

    def test_partial_acquisition_closes_all_known_handles(self):
        self.role("redisdata").rename(self.role("redisdata").with_name("retained"))
        count = fd_count()
        with self.assertRaises(FileNotFoundError):
            self.lease()
        self.assertEqual(fd_count(), count)

    def test_canonical_socket_mode_is_fixed_and_parent_drift_refuses_before_io(self):
        count = fd_count()
        self.socket_directory.chmod(0o700)
        with self.assertRaises(custody.Refused):
            self.lease()
        self.assertEqual(fd_count(), count)
        self.socket_directory.chmod(0o777)
        self.base.chmod(0o777)
        with mock.patch.object(custody.Lease, "observe", side_effect=AssertionError("new native I/O")), \
                mock.patch.object(custody.Lease, "process", side_effect=AssertionError("new proc I/O")):
            with self.assertRaises(custody.Refused):
                self.lease()
        self.assertEqual(fd_count(), count)

    def test_current_root_and_mount_namespace_drift_cannot_hide_behind_held_root(self):
        lease = self.lease()
        # The old root still resolves all original paths through its retained FD.
        original = custody.directory(lease.fds["root"], custody.GRAPH)
        os.close(original)
        other = self.base / "replacement-root"
        other.mkdir()
        (self.proc / "root").unlink()
        (self.proc / "root").symlink_to(other)
        with self.assertRaises(OSError):
            lease.check()
        self.assertFalse(lease.fds)

    def test_distinct_role_alias_namespace_and_listener_incarnation_are_required(self):
        original = custody.directory
        def alias(root, path):
            if path.endswith('source-paper-redisdata/_data'):
                path = path.replace('source-paper-redisdata', 'source-paper-pgdata')
            return original(root, path)
        count = fd_count()
        with mock.patch.object(custody, 'directory', side_effect=alias):
            with self.assertRaises(custody.Refused):
                self.lease()
        self.assertEqual(fd_count(), count)
        lease = self.lease()
        (self.proc/'ns/mnt').unlink()
        (self.proc/'ns/mnt').symlink_to('/proc/self/ns/user')
        with self.assertRaises(custody.Refused):
            lease.check()
        (self.proc/'ns/mnt').unlink()
        (self.proc/'ns/mnt').symlink_to('/proc/self/ns/mnt')
        lease = self.lease()
        (self.proc/'fd/7').unlink()
        with self.assertRaises(custody.Refused):
            lease.check()
        self.assertFalse(lease.fds)

    def test_mount_drift_and_submount_are_uncertainty(self):
        lease = self.lease()
        with (self.proc / "mountinfo").open("ab") as output:
            output.write(b"3 2 0:3 / /" + custody.GRAPH.encode() + b"/volumes rw - tmpfs other rw\n")
        with self.assertRaises(custody.Refused):
            lease.check()
        self.assertFalse(lease.fds)

    def test_daemon_proxy_wrong_core_source_and_deadline_are_fail_closed(self):
        for control in ("proxy", "core", "source", "deadline", "peer", "read-limit", "call-limit"):
            with self.subTest(control=control):
                lease = self.lease()
                old_command = (self.proc / "cmdline").read_bytes()
                old_binding = lease.path.read_bytes()
                if control == "proxy":
                    (self.proc / "cmdline").write_bytes(b"socket-proxy\0")
                elif control == "core":
                    lease.core = ("d" * 64, os.getpid(), self.binding["arguments"]["inner_peer"]["start"])
                elif control == "source":
                    lease.path.write_bytes(old_binding + b" ")
                elif control == "deadline":
                    lease.cutoff = contract.owner_clock() - 1
                elif control == "peer":
                    lease.peer = {**lease.peer, "uid": lease.peer["uid"] + 1}
                elif control == "read-limit":
                    lease.read_bytes = custody.READ_LIMIT
                else:
                    lease.calls = custody.CALL_LIMIT
                with self.assertRaises(custody.Refused):
                    lease.check()
                self.assertFalse(lease.fds)
                (self.proc / "cmdline").write_bytes(old_command)
                lease.path.write_bytes(old_binding)


class SocketCustodyControls(unittest.TestCase):
    """Canonical launcher permissions remain behind original private custody."""

    def setUp(self):
        owners = load("socket_owner_tests", ROOT / "scripts/test-docker-application-owner-recovery.py")
        self.fixture = owners.OwnerRecovery("runTest")
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.value = copy.deepcopy(self.fixture.value)
        self.sealed = copy.deepcopy(self.value)
        self.sealed.update(prefix="unit", fixture_root="/tmp/boxferry-fixture/unit",
                           paperless_sources={name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest()
                                              for name in contract.PAPERLESS_SOURCES})
        # Root/proc admission is separate from these filesystem/schema controls.
        for name in ("owner_paperless_root", "owner_paperless_parent_root"):
            patch = mock.patch.object(contract, name)
            patch.start()
            self.addCleanup(patch.stop)

    def refuse_before_native(self, value):
        with mock.patch.object(contract, "owner_call", side_effect=AssertionError("new native I/O")) as call:
            with self.assertRaises(contract.ContractError):
                contract.prepare_owner_custody(value)
            call.assert_not_called()
        self.assertFalse((self.fixture.directory / "owner-custody.json").exists())
        self.assertEqual(self.fixture.events(), [])

    def test_sealed_canonical_777_socket_under_original_700_parent_is_accepted(self):
        self.fixture.socket.chmod(0o777)
        self.assertTrue(contract.owner_validate(self.sealed))
        self.assertEqual(stat.S_IMODE(self.fixture.directory.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE(self.fixture.socket.stat().st_mode), 0o777)

    def test_sealed_700_socket_refuses_before_native(self):
        self.refuse_before_native(self.sealed)

    def test_legacy_700_socket_remains_accepted(self):
        self.assertTrue(contract.owner_validate(self.value))

    def test_legacy_777_socket_remains_refused_before_native(self):
        self.fixture.socket.chmod(0o777)
        self.refuse_before_native(self.value)

    def wrong_owner(self, path):
        original = pathlib.Path.lstat

        def changed(candidate, *args, **kwargs):
            info = original(candidate, *args, **kwargs)
            if candidate == path:
                fields = list(info)
                fields[4] = os.geteuid() + 1
                return os.stat_result(fields)
            return info

        return mock.patch.object(pathlib.Path, "lstat", changed)

    def test_sealed_wrong_socket_owner_refuses_before_native(self):
        self.fixture.socket.chmod(0o777)
        with self.wrong_owner(self.fixture.socket):
            self.refuse_before_native(self.sealed)

    def test_sealed_socket_symlink_refuses_before_native(self):
        target = self.fixture.directory / "socket-original"
        self.fixture.socket.rename(target)
        target.chmod(0o777)
        self.fixture.socket.symlink_to(target, target_is_directory=True)
        self.refuse_before_native(self.sealed)

    def test_sealed_public_parent_refuses_before_native(self):
        self.fixture.socket.chmod(0o777)
        self.fixture.directory.chmod(0o777)
        self.refuse_before_native(self.sealed)

    def test_sealed_wrong_parent_owner_refuses_before_native(self):
        self.fixture.socket.chmod(0o777)
        with self.wrong_owner(self.fixture.directory):
            self.refuse_before_native(self.sealed)

    def test_sealed_parent_identity_change_refuses_before_native(self):
        self.fixture.socket.chmod(0o777)
        self.sealed["directory_identity"] = "0:0"
        self.refuse_before_native(self.sealed)

    def test_sealed_replaced_parent_refuses_before_native(self):
        target = self.fixture.base / "original-parent"
        self.fixture.directory.rename(target)
        self.fixture.directory.mkdir(mode=0o700)
        self.fixture.socket.mkdir(mode=0o777)
        self.fixture.socket.chmod(0o777)
        self.refuse_before_native(self.sealed)

    def test_sealed_parent_symlink_refuses_before_native(self):
        target = self.fixture.base / "original-parent"
        self.fixture.directory.rename(target)
        self.fixture.directory.symlink_to(target, target_is_directory=True)
        self.addCleanup(self.fixture.directory.unlink)
        self.refuse_before_native(self.sealed)


class AuthorityControls(unittest.TestCase):
    def test_unmocked_nonroot_refuses_before_new_custody_io(self):
        self.assertNotEqual(os.geteuid(), 0, "run offline controls as the plain source-test user")
        with mock.patch.object(custody.os, "stat", side_effect=AssertionError("new custody I/O")), \
             mock.patch.object(custody.pathlib.Path, "read_text", side_effect=AssertionError("new custody I/O")):
            with self.assertRaises(custody.Refused):
                custody.root_authority()
        with mock.patch.object(contract.pathlib.Path, "read_text", side_effect=AssertionError("new custody I/O")):
            with self.assertRaises(contract.ContractError):
                contract.owner_paperless_sources({})

    def test_custody_work_reservation_refuses_before_any_outer_or_proc_work(self):
        caller = transport.Caller.__new__(transport.Caller)
        caller.check = lambda: None
        caller.hashes = {}
        caller.owner = {'paperless_sources': {}}
        caller.binding_sha = 'a'*64
        caller.custody_sequence = 1
        caller.custody_calls = caller.custody_bytes = 0
        caller.contract = types.SimpleNamespace(owner_capability=mock.Mock())
        for requests, response_bytes in ((254, 0), (0, 16*transport.LIMIT-custody.CHECK_READ_BOUND+1)):
            caller.requests, caller.response_bytes = requests, response_bytes
            with self.assertRaises(transport.Refused):
                caller.volume_check()
        caller.contract.owner_capability.assert_not_called()
        self.assertLess(custody.HOLD_READ_BOUND, custody.READ_LIMIT)
        self.assertLess(custody.PEAK_FDS, 128)

    def test_original_source_authority_precedes_module_execution(self):
        sources = {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in contract.PAPERLESS_SOURCES}
        owner = {"lane": "upstream-rootless", "prefix": "owned", "fixture_root": "/tmp/boxferry-fixture/owned",
                 "paperless_sources": sources}
        with mock.patch.object(contract, "owner_paperless_root"):
            self.assertEqual(contract.owner_paperless_module(owner).ROLES, custody.ROLES)
            owner["paperless_sources"] = {**sources, "scripts/lib/docker-paperless-volume-custody.py": "f" * 64}
            with mock.patch("builtins.exec", side_effect=AssertionError("unauthenticated source execution")):
                with self.assertRaises(contract.ContractError):
                    contract.owner_paperless_module(owner)
        self.assertEqual(contract.PAPERLESS_SOURCES, transport.SOURCE_NAMES)


# The guard fixture changes ONLY root/proc admission. The imported module is
# authenticated by production owner_paperless_sources before instrumentation;
# production Lease initialization/check/close and real guard frames still run.
GUARD_FIXTURE = r'''import importlib.util,json,os,pathlib,struct,sys
source,path,digest,base=sys.argv[1:];base=pathlib.Path(base)
spec=importlib.util.spec_from_file_location('protocol_owner',source)
module=importlib.util.module_from_spec(spec);sys.modules[spec.name]=module;spec.loader.exec_module(module)
module.owner_paperless_root=lambda value:None
def admit_parent(value):
 assert pathlib.Path('/proc/'+str(value['parent_pid'])+'/status').exists()
 with open(base/'events','a') as output:output.write(json.dumps([None,None,None,None,'root-parent-admitted'])+'\n')
module.owner_paperless_parent_root=admit_parent
original=module.owner_paperless_module
original_credentials=module.socket.socket.getsockopt
def credentials(stream,level,option,*args):
 result=original_credentials(stream,level,option,*args)
 if level==module.socket.SOL_SOCKET and option==module.socket.SO_PEERCRED:
  pid,uid,gid=struct.unpack('3i',result);return struct.pack('3i',pid,0,0)
 return result
module.socket.socket.getsockopt=credentials
def fixture_module(value):
 held=original(value)
 held.root_authority=lambda:None
 def snapshot(self):
  self.boundary();result={}
  try:
   for name in ('socket_directory','socket','proc','userns','mountns','root','exe','graph','outer_graph',*held.ROLES):
    result[name]=os.open(base/'fds'/name,held.OPATH|os.O_NOFOLLOW)
   result['pidfd']=os.pidfd_open(os.getpid());result['core_pidfd']=os.pidfd_open(os.getpid())
   self.calls+=3;self.read_bytes+=123
   return result
  except BaseException:
   for fd in result.values():os.close(fd)
   raise
 held.Lease.snapshot=snapshot
 close=held.Lease.close
 def logged_close(self):
  count=len(self.fds);close(self)
  if count:
   with open(base/'events','a') as output:output.write(json.dumps([None,None,None,None,'custody-closed',count,len(self.fds)])+'\n')
 held.Lease.close=logged_close
 return held
module.owner_paperless_module=fixture_module
original_observation=module.owner_save_expiry_observation
original_fdopen=module.os.fdopen
fault_path=base/'expiry-observation-fault'
pending_observation=str(module.owner_expiry_observation_path(json.loads(pathlib.Path(path).read_bytes())))+'.pending'
def fault_mode():return fault_path.read_text() if fault_path.exists() else ''
original_decision=module.owner_expiry_decision
def decision(*args,**kwargs):
    fault=fault_mode()
    if fault.startswith('decision-'):
        with open(base/'events','a') as output:output.write(json.dumps([None,None,None,None,'decision-fault',fault])+'\n')
        if fault=='decision-build':raise RuntimeError('controlled decision construction failure')
        original_start=module.process_start
        def unavailable(_pid):raise OSError(5,'controlled decision proc read failure')
        try:
            module.process_start=unavailable
            module.process_start(os.getpid())
        finally:module.process_start=original_start
    return original_decision(*args,**kwargs)
module.owner_expiry_decision=decision
def observation(value,lease,decision):
    fault=fault_mode()
    if fault:
        with open(base/'events','a') as output:output.write(json.dumps([None,None,None,None,'observation-fault',fault])+'\n')
    if fault=='budget':lease.read_bytes=16*1024*1024
    return original_observation(value,lease,decision)
module.owner_save_expiry_observation=observation
def fdopen(descriptor,*args,**kwargs):
    target=os.readlink('/proc/self/fd/'+str(descriptor))
    output=original_fdopen(descriptor,*args,**kwargs)
    if target!=pending_observation or not fault_mode():return output
    class FaultyOutput:
        def __enter__(self):return self
        def write(self,data):return output.write(data[:len(data)//2]) if fault_mode()=='short-write' else output.write(data)
        def flush(self):
            if fault_mode()=='enospc':raise OSError(28,'controlled observation ENOSPC')
            return output.flush()
        def fileno(self):return output.fileno()
        def __exit__(self,*exc):
            output.close()
            if fault_mode()=='close':raise OSError(5,'controlled observation close error')
            return False
    return FaultyOutput()
module.os.fdopen=fdopen
module.guard_owner_parent(module.owner_read(pathlib.Path(path),digest)['parent_pid'],
 module.owner_read(pathlib.Path(path),digest)['parent_start'],pathlib.Path(path),digest)
'''

CLIENT = r'''import json,os,pathlib,secrets,socket,sys
path,phase,binding,sequence,mode=sys.argv[1:]
challenge=secrets.token_hex(32)
if mode=='reused-challenge':challenge='f'*64
frame=(phase+' '+challenge+' '+sequence+'\n').encode()
if mode=='malformed':frame=b'paperless-check bad\n'
if mode=='oversized':frame=b'x'*4097
if mode=='partial-frame':frame=frame[:40]
with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as stream:
 stream.settimeout(3);stream.connect(path)
 stream.sendall(frame)
 if mode in ('partial','partial-frame'):sys.exit(0)
 response=b''
 while b'\n' not in response:
  block=stream.recv(4097-len(response))
  if not block:sys.exit(2)
  response+=block
  if len(response)>4096:sys.exit(3)
 value=json.loads(response)
 assert value['challenge']==challenge and value['binding_sha256']==binding and value['sequence']==int(sequence)+1
 if mode=='lost-ack':sys.exit(0)
 stream.sendall(('ack '+challenge+'\n').encode())
 assert stream.recv(130)==('ok '+challenge+'\n').encode()
 print(json.dumps({'pid':os.getpid(),'reply':value}))
'''


SUPERVISOR = "\nimport json,pathlib,sys,time,traceback\ncontrol=pathlib.Path(sys.argv[1]);scope={};sequence=0\nwhile True:\n    command=control/(str(sequence)+'.py')\n    if not command.exists():time.sleep(.002);continue\n    try:\n        exec(compile(command.read_text(),str(command),'exec'),scope)\n        outcome={'ok':True,'value':scope.get('result')}\n    except BaseException as error:\n        outcome={'ok':False,'kind':type(error).__name__,'message':str(error)}\n    pending=control/(str(sequence)+'.json.pending')\n    pending.write_text(json.dumps(outcome))\n    pending.replace(control/(str(sequence)+'.json'))\n    sequence+=1\n"


class SupervisorPublication(unittest.TestCase):
    def launch_paused_publication(self, control):
        # Instrument the actual fixture at the real file-write boundary. Both
        # old direct publication and new pending publication use this barrier.
        barrier = r"""
import pathlib,sys,time
control=pathlib.Path(sys.argv[1]);original_write=pathlib.Path.write_text
def pause_response(path,data,*args,**kwargs):
    if path.name not in ('0.json','0.json.pending'):
        return original_write(path,data,*args,**kwargs)
    midpoint=len(data)//2
    with path.open('w') as output:
        output.write(data[:midpoint]);output.flush()
        (control/'response-write-started').write_bytes(b'ready')
        cutoff=time.monotonic()+5
        while not (control/'allow-response-publication').exists():
            if time.monotonic()>=cutoff:raise TimeoutError('response barrier expired')
            time.sleep(.001)
        output.write(data[midpoint:])
    return len(data)
pathlib.Path.write_text=pause_response
"""
        expected = {"payload": "w" * 100}
        (control / "0.py").write_text("result=" + repr(expected) + "\n")
        process = subprocess.Popen([sys.executable, "-c", barrier + SUPERVISOR, str(control)],
            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, start_new_session=True)
        self.addCleanup(process.stderr.close)
        def stop():
            if process.poll() is None:
                process.kill()
            process.wait(timeout=3)
        self.addCleanup(stop)
        self.wait_for(control / "response-write-started", process)
        return process, expected

    def wait_for(self, path, process):
        cutoff = time.monotonic() + 5
        while not path.exists():
            self.assertIsNone(process.poll(), "supervisor exited before publication barrier")
            self.assertLess(time.monotonic(), cutoff)
            time.sleep(.001)

    def test_response_is_published_only_after_complete_write_and_close(self):
        with tempfile.TemporaryDirectory(prefix="custody-response-publication-") as temporary:
            control = pathlib.Path(temporary)
            process, expected = self.launch_paused_publication(control)
            response, pending = control / "0.json", control / "0.json.pending"
            self.assertFalse(response.exists(), "partial response must remain unpublished")
            with self.assertRaises(json.JSONDecodeError):
                json.loads(pending.read_text())
            (control / "allow-response-publication").touch()
            self.wait_for(response, process)
            # A published response must parse immediately, with no JSON retry.
            self.assertEqual(json.loads(response.read_text()), {"ok": True, "value": expected})
            self.assertFalse(pending.exists())

    def test_command_is_published_only_after_complete_write_and_close(self):
        with tempfile.TemporaryDirectory(prefix="custody-command-publication-") as temporary:
            control = pathlib.Path(temporary)
            process = subprocess.Popen([sys.executable, "-c", SUPERVISOR, str(control)],
                stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, start_new_session=True)
            self.addCleanup(process.stderr.close)
            def stop():
                if process.poll() is None:
                    process.kill()
                process.wait(timeout=3)
            self.addCleanup(stop)
            caller = GuardProtocol("runTest")
            caller.control, caller.counter = control, 0
            started, release = threading.Event(), threading.Event()
            failures = []
            original_write = pathlib.Path.write_text
            def paused_write(path, data, *args, **kwargs):
                if path.name not in ("0.py", "0.py.pending"):
                    return original_write(path, data, *args, **kwargs)
                midpoint = len(data) // 2
                with path.open("w") as output:
                    output.write(data[:midpoint]); output.flush()
                    started.set()
                    if not release.wait(5):
                        raise TimeoutError("command publication barrier expired")
                    output.write(data[midpoint:])
                return len(data)
            expected = {"payload": "v" * 100}
            def publish():
                try:
                    caller.publish_command("result=" + repr(expected) + "\n")
                except BaseException as error:
                    failures.append(error)
            writer = threading.Thread(target=publish)
            with mock.patch.object(pathlib.Path, "write_text", paused_write):
                writer.start()
                try:
                    self.assertTrue(started.wait(5))
                    self.assertFalse((control / "0.py").exists(), "partial command must remain unpublished")
                    self.assertFalse((control / "0.json").exists())
                finally:
                    release.set(); writer.join(timeout=5)
            self.assertFalse(writer.is_alive())
            self.assertEqual(failures, [])
            self.wait_for(control / "0.json", process)
            self.assertEqual(json.loads((control / "0.json").read_text()), {"ok": True, "value": expected})
            self.assertFalse((control / "0.py.pending").exists())

    def test_supervisor_death_before_publication_leaves_no_consumable_response(self):
        with tempfile.TemporaryDirectory(prefix="custody-response-owner-loss-") as temporary:
            control = pathlib.Path(temporary)
            process, _expected = self.launch_paused_publication(control)
            self.assertFalse((control / "0.json").exists())
            process.kill(); process.wait(timeout=3)
            self.assertFalse((control / "0.json").exists())
            with self.assertRaises(json.JSONDecodeError):
                json.loads((control / "0.json.pending").read_text())
            caller = GuardProtocol("runTest")
            caller.control, caller.counter, caller.binding_sha = control, 0, "a" * 64
            caller.fixture = types.SimpleNamespace(parent=process, directory=control)
            self.assertIsNone(caller.callback("paperless-check", 1, binding="e" * 64))


class GuardProtocol(unittest.TestCase):
    def setUp(self):
        owners = load('protocol_owner_tests', ROOT / 'scripts/test-docker-application-owner-recovery.py')
        self.fixture = owners.OwnerRecovery('runTest')
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.base = self.fixture.base
        self.control = self.base / 'control'
        self.control.mkdir()
        self.fixture.parent.kill(); self.fixture.parent.wait(timeout=3)
        self.fixture.parent = subprocess.Popen([sys.executable, '-c', SUPERVISOR, str(self.control)], start_new_session=True)
        value = self.fixture.value
        value.update(parent_pid=self.fixture.parent.pid, parent_start=contract.process_start(self.fixture.parent.pid),
                     prefix='unit', fixture_root='/tmp/boxferry-fixture/unit')
        self.fixture.state['fixture_root'] = value['fixture_root']
        self.fixture.socket.chmod(0o777)  # canonical bind mode in this root-admission fixture
        self.fixture.save()
        value['paperless_sources'] = {name: hashlib.sha256((ROOT/name).read_bytes()).hexdigest() for name in contract.PAPERLESS_SOURCES}
        with mock.patch.object(owners.contract, 'owner_paperless_root'), \
             mock.patch.object(owners.contract, 'owner_paperless_parent_root'):
            self.fixture.prepare()
        self.fixture.guard = subprocess.Popen([sys.executable, '-c', GUARD_FIXTURE,
            str(ROOT/'scripts/lib/docker-application-contract.py'), str(self.fixture.path), self.fixture.digest, str(self.base)],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        self.addCleanup(self.fixture.guard.stdout.close)
        self.addCleanup(self.fixture.guard.stderr.close)
        cutoff = time.monotonic()+3
        while not (self.fixture.directory/'owner-ready.json').exists():
            self.assertIsNone(self.fixture.guard.poll(), self.fixture.guard.stderr.read().decode() if self.fixture.guard.poll() is not None else '')
            self.assertLess(time.monotonic(), cutoff)
            time.sleep(.005)
        self.fixture.create()
        # Legacy readiness has separate legacy peer semantics, left unchanged.
        with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as stream:
            stream.settimeout(3);stream.connect(str(self.fixture.directory/'owner-guard.sock'))
            stream.sendall(('ready '+'f'*64+'\n').encode());self.assertIn(b'"phase":"ready"',stream.recv(4096))
        manifest = self.fixture.directory/'boxferry-paperless.ABCDEFGH/manifest.json'
        manifest.parent.mkdir(mode=0o700)
        manifest.write_bytes(raw({'config':{'stage':'source-fixture','target':None,'prefix':'unit','run_id':value['run'],
            'endpoint':{'id':value['context'],'uri':'unix://'+value['socket_directory']+'/docker.sock','scope':'owned-inner',
                        'mode':'rootless','ownership_sha256':self.fixture.digest},
            'work_deadline_ms':time.time_ns()//1_000_000+10_000}}));manifest.chmod(0o600)
        binding = {'arguments':{'custody_path':str(self.fixture.path),'custody_sha256':self.fixture.digest,
            'source_hashes':value['paperless_sources'],'inner_peer':{'pid':os.getpid(),'start':contract.process_start(os.getpid()),'uid':os.getuid(),'gid':os.getgid()},
            'manifest_path':str(manifest),'manifest_sha256':hashlib.sha256(manifest.read_bytes()).hexdigest()},
            'owner':value,'protocol_identity':[],'directory_identity':[],'lock_identity':[],
            'cutoff':min(value['execution_deadline'],contract.owner_clock()+10),'cleanup_cutoff':value['cleanup_deadline'],
            'ledger_key':'a'*64}
        path = self.fixture.directory/'paperless-transport/binding.json';path.parent.mkdir(mode=0o700)
        path.write_bytes(raw(binding));path.chmod(0o600)
        self.binding_sha = hashlib.sha256(path.read_bytes()).hexdigest()
        for name in ('socket_directory','socket','proc','userns','mountns','root','exe','graph','outer_graph',*custody.ROLES):
            (self.base/'fds'/name).mkdir(parents=True)
        self.counter=0

    def publish_command(self, code):
        path=self.control/(str(self.counter)+'.py')
        pending=path.with_suffix('.py.pending')
        pending.write_text(code)
        pending.replace(path)
        return path

    def callback(self, phase, sequence, mode='normal', binding=None):
        args=[str(self.fixture.directory/'owner-guard.sock'),phase,binding or self.binding_sha,str(sequence),mode]
        command=[sys.executable,'-c',CLIENT,*args]
        code='import subprocess\ncompleted=subprocess.run('+repr(command)+',capture_output=True,text=True,process_group=0,timeout=4)\nresult={"status":completed.returncode,"stdout":completed.stdout,"stderr":completed.stderr}\n'
        path=self.publish_command(code)
        response=path.with_suffix('.json');self.counter+=1
        cutoff=time.monotonic()+5
        while not response.exists():
            if self.fixture.parent.poll() is not None:return None
            self.assertLess(time.monotonic(),cutoff);time.sleep(.005)
        value=json.loads(response.read_text());self.assertTrue(value['ok'],value)
        return value['value']

    def recover(self):
        self.fixture.guard.wait(timeout=5)
        self.assertFalse(self.fixture.directory.exists(),self.fixture.guard.stderr.read().decode())
        events=self.fixture.events();operations=[row[4] for row in events]
        self.assertLess(operations.index('custody-closed'),operations.index('remove'))
        self.assertLess(operations.index('remove'),operations.index('volume-remove'))
        closed=next(row for row in events if row[4]=='custody-closed')
        self.assertEqual(closed[5:],[17,0])
        self.assertEqual(operations.count('root-parent-admitted'),1)

    def test_real_separate_callback_pids_share_one_live_lease_then_close_before_removal(self):
        first=self.callback('paperless-hold',0);second=self.callback('paperless-check',1)
        self.assertEqual(first['status'],0,first);self.assertEqual(second['status'],0,second)
        self.assertNotEqual(json.loads(first['stdout'])['pid'],json.loads(second['stdout'])['pid'])
        self.assertEqual(self.callback('paperless-close',2)['status'],0)
        self.fixture.parent.kill();self.fixture.parent.wait(timeout=3)
        self.recover()

    def test_replayed_saved_sequence_and_second_hold_poison_the_live_lease(self):
        self.assertEqual(self.callback('paperless-hold',0)['status'],0)
        self.callback('paperless-check',0)
        self.recover()

    def test_original_owner_loss_closes_active_lease_before_removal(self):
        self.assertEqual(self.callback('paperless-hold',0)['status'],0)
        self.fixture.parent.kill();self.fixture.parent.wait(timeout=3)
        self.recover()
        self.assertFalse(contract.owner_expiry_observation_path(self.fixture.value).exists())
    def test_guard_cancellation_does_not_attribute_lease_expiry(self):
        self.assertEqual(self.callback('paperless-hold',0)['status'],0)
        self.assertEqual(self.callback('paperless-check',1)['status'],0)
        self.fixture.guard.send_signal(signal.SIGTERM)
        self.recover()
        self.assertFalse(contract.owner_expiry_observation_path(self.fixture.value).exists())

    def test_idle_work_cutoff_cannot_consume_owner_execution_allowance(self):
        path=self.fixture.directory/'paperless-transport/binding.json'
        binding=json.loads(path.read_bytes())
        binding['cutoff']=contract.owner_clock()+1
        path.write_bytes(raw(binding));self.binding_sha=hashlib.sha256(path.read_bytes()).hexdigest()
        observation=contract.owner_expiry_observation_path(self.fixture.value)
        self.assertFalse(observation.exists())
        self.addCleanup(observation.unlink, missing_ok=True)
        guard_start=contract.process_start(self.fixture.guard.pid)
        self.assertEqual(self.callback('paperless-hold',0)['status'],0)
        self.assertEqual(self.callback('paperless-check',1)['status'],0)
        self.recover()
        self.assertLess(contract.owner_clock(),self.fixture.value['execution_deadline'])
        decision=json.loads(observation.read_bytes())
        self.assertLessEqual(observation.stat().st_size,4096)
        self.assertEqual(decision['decision'],'guard-selected-lease-expiry')
        self.assertEqual((decision['guard_pid'],decision['guard_start']),(self.fixture.guard.pid,guard_start))
        self.assertEqual((decision['owner_pid'],decision['owner_start']),
            (self.fixture.parent.pid,self.fixture.value['parent_start']))
        self.assertEqual(decision['custody_sha256'],self.fixture.digest)
        self.assertEqual(decision['guard_source_sha256'],self.fixture.value['guard_source_sha256'])
        self.assertEqual(decision['lease_binding_sha256'],self.binding_sha)
        self.assertEqual(decision['lease_sequence'],2)
        self.assertEqual(decision['lease_cutoff'],binding['cutoff'])
        self.assertTrue(decision['owner_alive'])
        self.assertFalse(decision['cancelled'])
        self.assertTrue(decision['decided_boottime']>=binding['cutoff']
                        or decision['decided_wall_ms']>=decision['lease_wall_cutoff'])
        self.assertLess(decision['decided_boottime'],self.fixture.value['execution_deadline'])
    def expiry_writer_fault(self, fault):
        (self.base/'expiry-observation-fault').write_text(fault)
        path=self.fixture.directory/'paperless-transport/binding.json'
        binding=json.loads(path.read_bytes());binding['cutoff']=contract.owner_clock()+1
        path.write_bytes(raw(binding));self.binding_sha=hashlib.sha256(path.read_bytes()).hexdigest()
        observation=contract.owner_expiry_observation_path(self.fixture.value)
        pending=observation.with_name(observation.name+'.pending')
        self.addCleanup(observation.unlink,missing_ok=True)
        self.addCleanup(pending.unlink,missing_ok=True)
        self.assertEqual(self.callback('paperless-hold',0)['status'],0)
        self.assertEqual(self.callback('paperless-check',1)['status'],0)
        self.recover()
        self.assertIsNotNone(self.fixture.parent.poll())
        self.assertFalse(observation.exists(),'failed instrumentation must never publish qualifying proof')
        events=self.fixture.events();operations=[row[4] for row in events]
        event='decision-fault' if fault.startswith('decision-') else 'observation-fault'
        if fault.startswith('decision-'):
            self.assertLess(operations.index(event),operations.index('custody-closed'))
        else:self.assertLess(operations.index('custody-closed'),operations.index(event))
        self.assertLess(operations.index(event),operations.index('remove'))
        self.assertEqual(next(row for row in events if row[4]==event)[5],fault)
        if pending.exists():self.assertLessEqual(pending.stat().st_size,4096)
    def test_decision_construction_failure_does_not_prevent_original_expiry_recovery(self):
        self.expiry_writer_fault('decision-build')
    def test_decision_proc_read_failure_does_not_prevent_original_expiry_recovery(self):
        self.expiry_writer_fault('decision-proc-read')
    def test_observation_enospc_does_not_prevent_original_expiry_recovery(self):
        self.expiry_writer_fault('enospc')
    def test_observation_short_write_does_not_prevent_original_expiry_recovery(self):
        self.expiry_writer_fault('short-write')
    def test_observation_close_failure_does_not_prevent_original_expiry_recovery(self):
        self.expiry_writer_fault('close')
    def test_observation_budget_refusal_does_not_prevent_original_expiry_recovery(self):
        self.expiry_writer_fault('budget')
    def test_guard_death_cannot_recreate_lease_from_binding_and_sequence(self):
        self.assertEqual(self.callback('paperless-hold',0)['status'],0)
        self.fixture.guard.kill();self.fixture.guard.wait(timeout=3)
        result=self.callback('paperless-check',1)
        self.assertNotEqual(result['status'],0)
        state=json.loads(self.fixture.state_path.read_text())
        self.assertTrue(state['container'] and state['volume'])
        self.assertFalse(any(row[4] in ('remove','volume-remove') for row in self.fixture.events()))

    def test_lost_hold_ack_closes_known_descriptors_and_recovers(self):
        self.callback('paperless-hold',0,'lost-ack')
        self.recover()

    def test_changed_custody_quiesces_known_work_and_closes_without_native_teardown(self):
        self.assertEqual(self.callback('paperless-hold',0)['status'],0)
        self.fixture.path.write_bytes(self.fixture.path.read_bytes()+b' ')
        self.callback('paperless-check',1)
        self.fixture.guard.wait(timeout=5)
        self.fixture.parent.wait(timeout=3)
        self.assertNotEqual(self.fixture.guard.returncode,0)
        operations=[row[4] for row in self.fixture.events()]
        self.assertIn('custody-closed',operations)
        self.assertNotIn('remove',operations)
        self.assertNotIn('volume-remove',operations)
        state=json.loads(self.fixture.state_path.read_text())
        self.assertTrue(state['container'] and state['volume'])
        self.assertTrue(self.fixture.directory.exists())

    def test_partial_request_after_hold_poisoned(self):
        self.assertEqual(self.callback('paperless-hold',0)['status'],0)
        self.callback('paperless-check',1,'partial-frame')
        self.recover()

    def test_second_hold_cannot_renew_source_lease(self):
        self.assertEqual(self.callback('paperless-hold',0)['status'],0)
        self.callback('paperless-hold',1)
        self.recover()

    def test_reused_challenge_with_new_sequence_is_refused(self):
        self.assertEqual(self.callback('paperless-hold',0,'reused-challenge')['status'],0)
        self.callback('paperless-check',1,'reused-challenge')
        self.recover()

    def test_concurrent_same_sequence_cannot_both_receive_live_acceptance(self):
        self.assertEqual(self.callback('paperless-hold',0)['status'],0)
        command=[sys.executable,'-c',CLIENT,str(self.fixture.directory/'owner-guard.sock'),
                 'paperless-check',self.binding_sha,'1','normal']
        outputs=[self.base/'concurrent-1',self.base/'concurrent-2']
        code='import pathlib,subprocess\nprocesses=[]\n'
        for output in outputs:
            code+='processes.append(subprocess.Popen('+repr(command)+',stdout=open('+repr(str(output))+',"w"),stderr=subprocess.DEVNULL,process_group=0))\n'
        code+='[process.wait(timeout=4) for process in processes]\nresult=True\n'
        self.publish_command(code)
        self.counter+=1
        self.recover()
        successes=[path.read_text() for path in outputs if path.exists() and path.stat().st_size]
        self.assertLessEqual(len(successes),1)

    def test_wrong_binding_after_hold_poisoned(self):
        self.assertEqual(self.callback('paperless-hold',0)['status'],0)
        self.callback('paperless-check',1,binding='e'*64)
        self.recover()

    def test_malformed_after_hold_poisoned(self):
        self.assertEqual(self.callback('paperless-hold',0)['status'],0)
        self.callback('paperless-check',1,'malformed')
        self.recover()

    def test_oversized_after_hold_poisoned(self):
        self.assertEqual(self.callback('paperless-hold',0)['status'],0)
        self.callback('paperless-check',1,'oversized')
        self.recover()

    def test_foreign_client_session_cannot_use_echoed_binding(self):
        self.assertEqual(self.callback('paperless-hold',0)['status'],0)
        completed=subprocess.run([sys.executable,'-c',CLIENT,str(self.fixture.directory/'owner-guard.sock'),
            'paperless-check',self.binding_sha,'1','normal'],capture_output=True,timeout=4)
        self.assertNotEqual(completed.returncode,0)
        self.recover()

    def test_replay_after_descriptor_close_cannot_reacquire(self):
        self.assertEqual(self.callback('paperless-hold',0)['status'],0)
        self.assertEqual(self.callback('paperless-close',1)['status'],0)
        self.callback('paperless-hold',2)
        self.recover()


if __name__ == "__main__":
    if os.geteuid() == 0:
        # The source suite is intentionally plain-user, including its genuine
        # nonroot refusal. This changes only the test process, never the host.
        os.setgroups([])
        os.setgid(65534)
        os.setuid(65534)
    unittest.main()
