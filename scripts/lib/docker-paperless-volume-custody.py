#!/usr/bin/env python3
"""Fixed source-stage Paperless custody in the existing root owner guard.

No CLI, providers, guest code, content reads, namespace entry or runtime writes.
The importing guard authenticates these bytes from original sealed owner custody.
"""
from __future__ import annotations

import hashlib
import json
import os
import pathlib
import re
import socket
import stat
import struct
import time

ROLES = ("data", "media", "consume", "export", "pgdata", "redisdata")
GRAPH = "home/docker/.local/share/docker"
OPATH = os.O_PATH | os.O_DIRECTORY | os.O_CLOEXEC
LIMIT = 1_048_576
READ_LIMIT = 16 * LIMIT
CALL_LIMIT = 256
CHECK_READ_BOUND = 5 * LIMIT + 34 * 4096 + 2 * 65536 + 16384 + 256 * 4096 + 42
HOLD_READ_BOUND = 2 * CHECK_READ_BOUND
# 17 retained descriptors; at most another 17 during fresh acquisition, plus
# two traversal/proc-read temporaries. No RLIMIT is raised by this module.
RETAINED_FDS = 17
PEAK_FDS = 36


class Refused(Exception):
    """No private values are included in errors."""


def need(condition):
    if not condition:
        raise Refused("Paperless volume custody refused")


def identity(fd):
    value = os.fstat(fd)
    return value.st_dev, value.st_ino, stat.S_IFMT(value.st_mode)


def directory(root, path):
    """Fixed ordinary components never follow symlinks; bind crossings allowed."""
    need(type(path) is str and not path.startswith("/")
         and all(part not in {"", ".", ".."} for part in path.split("/")))
    current = os.dup(root)
    try:
        for part in path.split("/"):
            following = os.open(part, OPATH | os.O_NOFOLLOW, dir_fd=current)
            os.close(current)
            current = following
        return current
    except BaseException:
        os.close(current)
        raise


def root_authority():
    # Refuse plain users before any new custody I/O. Namespace-local UID zero
    # alone is insufficient: require the initial user mapping and namespace.
    need(os.getuid() == os.geteuid() == os.getgid() == os.getegid() == 0)
    need(os.stat("/proc/self/ns/user") == os.stat("/proc/1/ns/user"))
    need(pathlib.Path("/proc/self/uid_map").read_text().split() == ["0", "0", "4294967295"])


class Lease:
    """One guard-owned lease. Scalars identify it; only this object owns FDs."""

    def __init__(self, contract, owner, binding_sha, custody_sha, lifecycle):
        root_authority()
        self.contract, self.owner, self.lifecycle = contract, owner, lifecycle
        self.fds = {}
        self.closed = False
        self.failed = False
        self.read_bytes = 0
        self.calls = 0
        self.checks = 0
        self.binding_sha = binding_sha
        self.custody_sha = custody_sha
        self.path = pathlib.Path(owner["directory"]) / "paperless-transport/binding.json"
        self.core = None
        try:
            contract.outer_run_directory(pathlib.Path(owner["directory"]), owner["directory_identity"])
            self.binding_raw = contract.outer_runtime_bytes(self.path, private=True)
            actual_binding = hashlib.sha256(self.binding_raw).hexdigest()
            need(binding_sha is None or actual_binding == binding_sha)
            self.binding_sha = actual_binding
            self.binding = binding = json.loads(self.binding_raw, object_pairs_hook=contract.no_duplicate_keys)
            need(set(binding) == {"arguments", "owner", "protocol_identity", "directory_identity", "lock_identity",
                                  "cutoff", "cleanup_cutoff", "ledger_key"} and binding["owner"] == owner)
            args = binding["arguments"]
            need(args["custody_path"] == str(pathlib.Path(owner["directory"]) / "owner-custody.json")
                 and args["custody_sha256"] == custody_sha and args["source_hashes"] == owner["paperless_sources"])
            self.peer = args["inner_peer"]
            need(type(self.peer) is dict and set(self.peer) == {"pid", "start", "uid", "gid"}
                 and all(type(x) is int and x > 0 for x in self.peer.values()))
            need(type(binding["cutoff"]) in (int, float) and type(binding["cleanup_cutoff"]) in (int, float)
                 and binding["cutoff"] < binding["cleanup_cutoff"] <= owner["cleanup_deadline"]
                 and binding["cutoff"] <= owner["execution_deadline"])
            self.cutoff = binding["cutoff"]
            manifest = pathlib.Path(args["manifest_path"])
            need(manifest.parent.parent == pathlib.Path(owner["directory"]) and manifest.name == "manifest.json"
                 and re.fullmatch(r"boxferry-paperless\.[A-Za-z0-9]{8}", manifest.parent.name) is not None)
            self.manifest_path = manifest
            self.manifest_raw = contract.outer_runtime_bytes(manifest, private=True)
            self.read_bytes += len(self.binding_raw) + len(self.manifest_raw)
            need(self.read_bytes <= READ_LIMIT)
            need(hashlib.sha256(self.manifest_raw).hexdigest() == args["manifest_sha256"])
            config = json.loads(self.manifest_raw, object_pairs_hook=contract.no_duplicate_keys)["config"]
            need(config["stage"] == "source-fixture" and config["target"] is None
                 and config["prefix"] == owner["prefix"] and config["run_id"] == owner["run"]
                 and config["endpoint"] == {"id": owner["context"], "uri": "unix://" + owner["socket_directory"] + "/docker.sock",
                     "scope": "owned-inner", "mode": "rootless", "ownership_sha256": custody_sha})
            self.wall_cutoff = config["work_deadline_ms"]
            need(type(self.wall_cutoff) is int)
            self.fds = self.snapshot()
            need(len(self.fds) == RETAINED_FDS)
            self.check()
        except BaseException:
            self.close()
            raise

    def boundary(self):
        need(not self.closed and not self.failed)
        self.contract.owner_lifecycle(self.lifecycle, self.cutoff)
        need(self.contract.owner_clock() < self.cutoff
             and time.time_ns() // 1_000_000 < self.wall_cutoff)

    def read(self, proc, name, limit):
        self.boundary()
        fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=proc)
        try:
            raw = bytearray()
            while True:
                self.boundary()
                block = os.read(fd, limit + 1 - len(raw))
                self.read_bytes += len(block)
                need(self.read_bytes <= READ_LIMIT)
                if not block:
                    break
                raw.extend(block)
                need(len(raw) <= limit)
            need(raw)
            self.boundary()
            return bytes(raw)
        finally:
            os.close(fd)

    def private_read(self, path):
        self.boundary()
        raw = self.contract.outer_runtime_bytes(path, private=True)
        self.read_bytes += len(raw)
        need(self.read_bytes <= READ_LIMIT)
        return raw

    def observe(self, arguments):
        self.boundary()
        self.calls += 1
        need(self.calls <= CALL_LIMIT)
        status, raw = self.contract.owner_call(self.owner, arguments, self.cutoff, lifecycle=self.lifecycle)
        self.read_bytes += len(raw)
        need(status == 0 and self.read_bytes <= READ_LIMIT)
        return raw

    def process(self, pid):
        need(type(pid) is int and pid > 0)
        return os.open(f"/proc/{pid}", OPATH | os.O_NOFOLLOW)

    def start(self, proc):
        fields = self.read(proc, "stat", 4096).rsplit(b")", 1)[1].split()
        need(len(fields) >= 20 and fields[0] != b"Z")
        return int(fields[19]), int(fields[1])

    def snapshot(self):
        """Every snapshot opens CURRENT proc-root; never reopen through held root."""
        self.boundary()
        result = {}
        try:
            owner, contract = self.owner, self.contract
            cid = contract.owner_cid(owner)
            need(cid is not None)
            self.observe(["daemon-binding", cid, owner["outer_name"], owner["outer_owner"], owner["outer_run"], owner["outer_image"]])
            self.observe(["check-mounts", cid, owner["volume_name"], "rootless", owner["socket_directory"], "/boxferry-core", owner["graph_root"]])
            value = json.loads(self.observe(["observe", "container", cid]), object_pairs_hook=contract.no_duplicate_keys)
            need(value["Id"] == cid and value["Name"].removeprefix("/") == owner["outer_name"]
                 and value["State"]["Running"] is True)
            init = value["State"]["Pid"]
            need(type(init) is int and init > 1)
            result["socket_directory"] = os.open(owner["socket_directory"], OPATH | os.O_NOFOLLOW)
            info = os.fstat(result["socket_directory"])
            need(f"{info.st_dev}:{info.st_ino}" == owner["socket_identity"] and info.st_uid == 0
                 and stat.S_IMODE(info.st_mode) == 0o777)
            socket_path = pathlib.Path(owner["socket_directory"]) / "docker.sock"
            before = socket_path.lstat()
            need(stat.S_ISSOCK(before.st_mode))
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as stream:
                stream.settimeout(min(.2, max(.001, self.cutoff - contract.owner_clock())))
                stream.connect(str(socket_path))
                peer = struct.unpack("3i", stream.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
                need(peer == (self.peer["pid"], self.peer["uid"], self.peer["gid"]))
                need(before == socket_path.lstat())
                result["socket"] = os.open("docker.sock", os.O_PATH | os.O_NOFOLLOW | os.O_CLOEXEC,
                                           dir_fd=result["socket_directory"])
                need(identity(result["socket"]) == (before.st_dev, before.st_ino, stat.S_IFSOCK))
            actual_socket = {"device": before.st_dev, "inode": before.st_ino, "uid": before.st_uid,
                             "gid": before.st_gid, "mode": stat.S_IMODE(before.st_mode)}
            need(actual_socket == self.binding["arguments"]["socket_identity"])
            result["proc"] = self.process(peer[0])
            need(self.start(result["proc"])[0] == self.peer["start"])
            status = self.read(result["proc"], "status", 16384).splitlines()
            for key, expected in ((b"Uid:", self.peer["uid"]), (b"Gid:", self.peer["gid"])):
                matching = [line.split()[1:] for line in status if line.startswith(key)]
                need(matching == [[str(expected).encode()] * 4])
            result["pidfd"] = contract.verified_parent_fd(peer[0], self.peer["start"])
            need(contract.parent_alive(result["pidfd"]))
            # Prove original core membership through host-visible ancestry. No
            # credentials from an inherited listener/proxy stand in for dockerd.
            cursor = peer[0]
            for _ in range(32):
                proc = self.process(cursor)
                try:
                    start, parent = self.start(proc)
                    if cursor == init:
                        original = (cid, init, start)
                        need(self.core is None or self.core == original)
                        self.core = original
                        result["core_pidfd"] = contract.verified_parent_fd(init, start)
                        break
                finally:
                    os.close(proc)
                need(parent > 1 and parent != cursor)
                cursor = parent
            else:
                need(False)
            proc = result["proc"]
            result["userns"] = os.open("ns/user", os.O_RDONLY | os.O_CLOEXEC, dir_fd=proc)
            result["mountns"] = os.open("ns/mnt", os.O_RDONLY | os.O_CLOEXEC, dir_fd=proc)
            # Intentional kernel magic links; ordinary traversal uses NOFOLLOW.
            result["root"] = os.open("root", OPATH, dir_fd=proc)
            result["exe"] = os.open("exe", os.O_PATH | os.O_CLOEXEC, dir_fd=proc)
            binary = directory(result["root"], "usr/local/bin")
            try:
                executable = os.open("dockerd", os.O_PATH | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=binary)
                try:
                    need(stat.S_ISREG(os.fstat(executable).st_mode) and identity(executable) == identity(result["exe"]))
                finally:
                    os.close(executable)
            finally:
                os.close(binary)
            command = self.read(proc, "cmdline", 4096).split(b"\0")
            need(command[0] in {b"dockerd", b"/usr/local/bin/dockerd"}
                 and b"--host=unix:///boxferry-core/docker.sock" in command)
            # Listener credentials alone can name a proxy or a creator that no
            # longer owns the endpoint. Require the fixed dockerd to retain its
            # listening socket in the current proc/network view as well.
            listeners = [line.split() for line in self.read(proc, "net/unix", 65536).splitlines()[1:]
                         if line.split()[-1:] == [b"/boxferry-core/docker.sock"]]
            need(len(listeners) == 1 and len(listeners[0]) == 8
                 and listeners[0][3:6] == [b"00010000", b"0001", b"01"]
                 and listeners[0][6].isdigit())
            wanted = "socket:[" + listeners[0][6].decode("ascii") + "]"
            descriptors = os.open("fd", os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=proc)
            try:
                matches = 0
                with os.scandir(descriptors) as entries:
                    for index, entry in enumerate(entries):
                        need(index < 256 and entry.name.isdigit())
                        target = os.readlink(entry.name, dir_fd=descriptors)
                        need(len(target.encode()) <= 4096)
                        self.read_bytes += len(target.encode())
                        need(self.read_bytes <= READ_LIMIT)
                        matches += target == wanted
                need(matches == 1)
            finally:
                os.close(descriptors)
            # The current core's image filesystem must expose the same binary.
            core_root = os.open(f"/proc/{init}/root", OPATH)
            try:
                binary = directory(core_root, "usr/local/bin")
                try:
                    executable = os.open("dockerd", os.O_PATH | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=binary)
                    try:
                        need(identity(executable) == identity(result["exe"]))
                    finally:
                        os.close(executable)
                finally:
                    os.close(binary)
            finally:
                os.close(core_root)
            result["graph"] = directory(result["root"], GRAPH)
            host_root = os.open("/", OPATH)
            try:
                result["outer_graph"] = directory(host_root, owner["volume_path"].removeprefix("/"))
            finally:
                os.close(host_root)
            need(identity(result["graph"]) == identity(result["outer_graph"]))
            mounts = self.read(proc, "mountinfo", 65536)
            lines = [line.split() for line in mounts.splitlines()]
            graph_path = b"/" + GRAPH.encode()
            need(all(len(line) >= 10 for line in lines))
            graph_mounts = [line for line in lines if line[4] == graph_path]
            need(len(graph_mounts) == 1 and b"rw" in graph_mounts[0][5].split(b","))
            # Running canonical containers legitimately mount overlay storage
            # elsewhere below graph. Only the fixed volume traversal subtree
            # must remain free of additional mount transitions.
            volume_path = graph_path + b"/volumes"
            need(not any(line[4] == volume_path or line[4].startswith(volume_path + b"/") for line in lines))
            # Mount IDs are compared only inside the SAME retained namespace.
            signature = hashlib.sha256(mounts).digest()
            need(not hasattr(self, "mount_signature") or self.mount_signature == signature)
            self.mount_signature = signature
            for role in ROLES:
                result[role] = directory(result["graph"], "volumes/" + owner["prefix"] + "-paper-" + role + "/_data")
            need(len({identity(result[role]) for role in ROLES}) == len(ROLES))
            self.boundary()
            return result
        except BaseException:
            for fd in result.values():
                os.close(fd)
            raise

    def check(self):
        current = {}
        try:
            self.boundary()
            self.contract.outer_run_directory(pathlib.Path(self.owner["directory"]), self.owner["directory_identity"])
            need(self.private_read(self.path) == self.binding_raw
                 and self.private_read(self.manifest_path) == self.manifest_raw)
            for key in ("pidfd", "core_pidfd"):
                need(self.contract.parent_alive(self.fds[key]))
            self.checks += 1
            need(self.checks <= CALL_LIMIT)
            current = self.snapshot()
            need(set(current) == set(self.fds)
                 and all(identity(current[key]) == identity(fd) for key, fd in self.fds.items()
                         if key not in {"pidfd", "core_pidfd"}))
            self.boundary()
            return {"checks": self.checks, "calls": self.calls, "read_bytes": self.read_bytes}
        except BaseException:
            self.failed = True
            self.close()
            raise
        finally:
            for fd in current.values():
                os.close(fd)

    def close(self):
        # Idempotent; never validate first or acquire teardown authority here.
        self.closed = True
        known, self.fds = self.fds, {}
        for fd in known.values():
            try:
                os.close(fd)
            except OSError:
                # Do not abandon later known handles, or retry a number the
                # kernel may already have released for reuse.
                self.failed = True
