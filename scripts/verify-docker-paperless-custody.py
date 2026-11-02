#!/usr/bin/env python3
"""Fixed main-only native mechanism; never an application or product executor."""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import select
import selectors
import signal
import socket
import stat
import struct
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
ROLES = ("data", "media", "consume", "export", "pgdata", "redisdata")
CASES = ("positive", *("replace-" + role for role in ROLES), "owner-loss", "guard-loss", "expiry", "lost-ack")
LIMIT = 1_048_576
CHECK_BYTES = 5 * LIMIT + 34 * 4096 + 2 * 65536 + 16384 + 256 * 4096 + 42


class Refused(ValueError):
    pass


def need(value):
    if not value:
        raise Refused("fixed custody mechanism refused")


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def read_private(path, maximum=128 * 1024):
    need(path.is_absolute() and path.resolve(strict=True) == path)
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        before = os.fstat(fd)
        need(stat.S_ISREG(before.st_mode) and before.st_uid == os.geteuid()
             and stat.S_IMODE(before.st_mode) == 0o600 and before.st_nlink == 1
             and 0 < before.st_size <= maximum)
        raw = bytearray()
        while block := os.read(fd, maximum + 1 - len(raw)):
            raw.extend(block)
            need(len(raw) <= maximum)
        need(len(raw) == before.st_size and before == os.fstat(fd) and before == path.lstat())
        return bytes(raw)
    finally:
        os.close(fd)


def write_new(path, value):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(encoded(value))
        stream.flush()
        os.fsync(stream.fileno())


def contract():
    spec = importlib.util.spec_from_file_location("mechanism_owner", ROOT / "scripts/lib/docker-application-contract.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def callback(path, digest, phase, binding, sequence, lost_ack=False, control_binding=None):
    c = contract()
    owner = c.owner_read(path, digest)
    need(os.geteuid() == os.getuid() == os.getegid() == os.getgid() == 0
         and os.getsid(0) == owner["parent_pid"])
    need(control_binding is not None
         and Path(f"/proc/{owner['parent_pid']}/cmdline").read_bytes().split(b"\0")[:-1]
             == [os.fsencode(arg) for arg in (sys.executable, str(Path(__file__).resolve()), "run", "--binding", str(control_binding))])
    control = json.loads(read_private(control_binding))
    need(control["scope"] == "paperless-custody-mechanism"
         and owner["paperless_sources"] == {name: control["box"]["files"][name] for name in c.PAPERLESS_SOURCES})
    original_send = socket.socket.sendall

    def record_send(stream, data, *args, **kwargs):
        match = re.fullmatch(rb"(paperless-hold|paperless-check|paperless-close) ([0-9a-f]{64}) ([0-9]{1,3})\n", data)
        if match is not None:
            need(c.owner_clock() + 1 < owner["execution_deadline"])
        result = original_send(stream, data, *args, **kwargs)
        if match is not None:
            write_new(control_binding.parent / ("request-" + str(sequence) + ".json"), {
                "phase": match[1].decode(), "sequence": int(match[3]), "owner_pid": owner["parent_pid"],
                "owner_start": owner["parent_start"], "pid": os.getpid(), "start": c.process_start(os.getpid()),
                "sid": os.getsid(0), "sent_boottime": c.owner_clock(), "binding_sha256": binding,
                "challenge_sha256": hashlib.sha256(match[2]).hexdigest()})
        return result

    if not lost_ack:
        socket.socket.sendall = record_send
        try:
            reply = c.owner_capability(path, digest, phase=phase, context=owner["context"],
                execution_deadline=owner["execution_deadline"], cleanup_deadline=owner["cleanup_deadline"],
                paperless_binding=binding, paperless_sequence=sequence)
        finally:
            socket.socket.sendall = original_send
        print(json.dumps({"pid": os.getpid(), "start": c.process_start(os.getpid()), "sid": os.getsid(0), "reply": reply}))
        return
    # One fixed negative control: authenticate the actual guard, consume a live
    # check reply, and close without confirmation. No new guard operation exists.
    need(phase == "paperless-check")
    need(control["case"] == "lost-ack")
    marker = json.loads(read_private(path.parent / "owner-ready.json"))
    pidfd = c.verified_parent_fd(marker["pid"], marker["start"])
    try:
        need(marker["custody_sha256"] == digest and c.parent_alive(pidfd))
        need(Path(f"/proc/{marker['pid']}/cmdline").read_bytes().split(b"\0")[:-1]
             == [os.fsencode(arg) for arg in c.owner_guard_command(owner, path, digest)])
        challenge = secrets.token_hex(32)
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as stream:
            stream.settimeout(max(.001, owner["execution_deadline"] - c.owner_clock()))
            stream.connect(str(path.parent / "owner-guard.sock"))
            peer, uid, gid = struct.unpack("3i", stream.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
            need((peer, uid, gid) == (marker["pid"], 0, 0))
            record_send(stream, f"paperless-check {challenge} {sequence}\n".encode())
            raw = bytearray()
            while b"\n" not in raw:
                block = stream.recv(4097 - len(raw))
                need(block)
                raw.extend(block)
                need(len(raw) <= 4096)
            reply = json.loads(raw, object_pairs_hook=c.no_duplicate_keys)
            need(reply["challenge"] == challenge and reply["sequence"] == sequence + 1
                 and reply["binding_sha256"] == binding and reply["custody_sha256"] == digest)
            write_new(control_binding.parent / "lost-ack.json", {"phase": "paperless-check",
                "owner_pid": owner["parent_pid"], "owner_start": owner["parent_start"], "pid": os.getpid(),
                "start": c.process_start(os.getpid()), "sid": os.getsid(0), "sequence": reply["sequence"],
                "binding_sha256": binding, "challenge_sha256": hashlib.sha256(challenge.encode()).hexdigest()})
        print(json.dumps({"pid": os.getpid(), "start": c.process_start(os.getpid()), "sid": os.getsid(0), "lost_ack": True}))
    finally:
        os.close(pidfd)


class Mechanism:
    def __init__(self, binding_path):
        need(os.getuid() == os.geteuid() == os.getgid() == os.getegid() == 0)
        self.binding_path = binding_path
        self.binding = b = json.loads(read_private(binding_path))
        need(set(b) == {"schema", "scope", "case", "box", "sdk", "sdk_root", "context"}
             and b["schema"] == 1 and b["scope"] == "paperless-custody-mechanism"
             and b["case"] in CASES)
        for name, expected in b["box"]["files"].items():
            path = ROOT / name
            need(path.resolve(strict=True) == path and hashlib.sha256(path.read_bytes()).hexdigest() == expected)
        need(b["context"]["run_id"] == os.environ.get("GITHUB_RUN_ID")
             and b["context"]["attempt"] == os.environ.get("GITHUB_RUN_ATTEMPT"))
        self.c = c = contract()
        c.owner_paperless_root({})
        need(os.getsid(0) == os.getpid() and os.getpgrp() == os.getpid())
        self.sdk = Path(b["sdk_root"])
        need(c.git(self.sdk, "rev-parse", "HEAD") == b["sdk"]["revision"]
             and c.git(self.sdk, "status", "--porcelain", "--untracked-files=all") == "")
        for name, expected in b["sdk"]["files"].items():
            need(hashlib.sha256(c.bounded_regular_bytes(self.sdk / name, LIMIT)).hexdigest() == expected)
        self.deadline = int(c.owner_clock()) + 780
        self.cleanup_deadline = self.deadline + 120
        self.calls = self.bytes = self.sequence = self.custody_calls = self.custody_bytes = 0
        self.callback_pids = []
        self.callbacks = []
        self.guard = None
        self.owner = None
        self.path = None
        self.directory = None
        self.closed = False
        self.prepared = False
        self.phase_number = 0
        self.uncertain = False

    def charge(self, calls, observed):
        need(type(calls) is int and calls >= 0 and type(observed) is int and observed >= 0)
        self.calls += calls
        self.bytes += observed
        need(self.calls <= 256 and self.bytes <= 16 * LIMIT)

    def boundary(self, cleanup=False):
        need(self.c.owner_clock() < (self.cleanup_deadline if cleanup else self.deadline))

    def outer(self, arguments, *, unprepared=False, cleanup=False):
        self.boundary(cleanup)
        need(self.calls + 1 <= 256 and self.bytes + LIMIT + 1 <= 16 * LIMIT)
        if self.owner is not None:
            status, raw = self.c.owner_call(self.owner, arguments,
                self.cleanup_deadline if cleanup else self.deadline, unprepared=unprepared,
                paperless_caller=not unprepared)
        else:
            command = self.c.outer_runtime_command(self.directory / "outer-runtime-helper", self.directory_identity,
                self.closure, "docker")
            # Before custody, only read-only preflight or the existing exact
            # immutable host-side core pull is selected by authored code below.
            process = subprocess.Popen([*command, *arguments], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                       process_group=0)
            try:
                raw = bytearray()
                os.set_blocking(process.stdout.fileno(), False)
                with selectors.DefaultSelector() as monitor:
                    monitor.register(process.stdout, selectors.EVENT_READ)
                    ended = False
                    while not ended:
                        self.boundary()
                        for key, _event in monitor.select(min(.1, max(.001, self.deadline - self.c.owner_clock()))):
                            block = os.read(key.fd, min(65536, LIMIT + 1 - len(raw)))
                            if not block:
                                ended = True
                                break
                            raw.extend(block)
                            need(len(raw) <= LIMIT)
                raw = bytes(raw)
                status = process.wait(timeout=max(.001, self.deadline - self.c.owner_clock()))
            finally:
                self.c.owner_command_quiescence(process, min(self.cleanup_deadline, self.c.owner_clock() + 2))
                process.wait(timeout=2)
                process.stdout.close()
        self.charge(1, len(raw))
        self.boundary(cleanup)
        need(status == 0)
        return raw

    def prepare(self):
        c, b = self.c, self.binding
        images = c.canonical_images(str(self.sdk), b["sdk"]["revision"], b["sdk"]["files"]["scripts/native-conformance.sh"])
        image = images["upstream-rootless"]
        for _ in range(8):
            token = secrets.token_hex(4)
            directory = Path("/tmp") / ("boxferry-docker-core." + token)
            try:
                directory.mkdir(mode=0o700)
                break
            except FileExistsError:
                continue
        else:
            raise Refused()
        self.directory = directory
        info = directory.stat()
        self.directory_identity = f"{info.st_dev}:{info.st_ino}"
        helper_closure = {name: b["sdk"]["files"][name] for name in c.OUTER_RUNTIME_FILES}
        self.closure = encoded(helper_closure).decode()
        c.capture_outer_runtime(self.sdk, b["sdk"]["revision"], self.closure,
                                directory / "outer-runtime-helper", self.directory_identity)
        graph = self.outer(["preflight"]).decode().strip()
        need(graph.startswith("/") and graph != "/")
        socket_dir = directory / "socket"
        socket_dir.mkdir(mode=0o777)
        socket_dir.chmod(0o777)  # Canonical fixture creation; acquisition never repairs modes.
        socket_info = socket_dir.stat()
        prefix = "bf-custody-" + token
        runner = {"nonce": secrets.token_hex(16), "revision": b["sdk"]["revision"],
                  "workflow_run": b["context"]["run_id"], "attempt": b["context"]["attempt"]}
        self.owner = owner = {"parent_pid": os.getpid(), "parent_start": c.process_start(os.getpid()),
            "execution_deadline": self.deadline, "cleanup_deadline": self.cleanup_deadline,
            "guard_source_sha256": b["box"]["files"]["scripts/lib/docker-application-contract.py"],
            "directory": str(directory), "directory_identity": self.directory_identity, "helper_closure": helper_closure,
            "engine": "docker", "lane": "upstream-rootless", "context": "custody-" + token,
            "run": "bf-" + token, "prefix": prefix, "outer_name": "bf-docker-core-" + token,
            "outer_run": token, "outer_owner": "io.boxferry.docker-core-run", "outer_image": image,
            "volume_name": "bf-docker-core-data-" + token, "graph_root": graph,
            "volume_path": graph + "/volumes/bf-docker-core-data-" + token + "/_data",
            "socket_directory": str(socket_dir), "socket_identity": f"{socket_info.st_dev}:{socket_info.st_ino}",
            "guest_directory": "/boxferry-core", "fixture_root": "/tmp/boxferry-fixture/" + prefix,
            "runner_profile": runner, "cid_file": str(directory / "outer-create.cid"),
            "paperless_sources": {name: b["box"]["files"][name] for name in c.PAPERLESS_SOURCES}}
        self.path = directory / "owner-custody.json"
        self.path, self.digest = c.prepare_owner_custody(owner)
        self.prepared = True
        self.guard = subprocess.Popen(c.owner_guard_command(owner, self.path, self.digest),
                                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        self.wait_file(directory / "owner-ready.json")
        self.legacy("pending")
        self.phase("prepared")
        self.outer(["pull", image])
        self.outer(["volume-create", owner["volume_name"], owner["outer_owner"], token])
        need(self.outer(["volume-path", owner["volume_name"], owner["outer_owner"], token]).decode().strip() == owner["volume_path"])
        self.outer(["prepare-data", image, "rootless", owner["volume_name"], owner["volume_path"], graph])
        self.outer(["apparmor", "prepare", "upstream-rootless", runner["nonce"], owner["outer_name"], token, image])
        self.outer(["apparmor", "load", "upstream-rootless", runner["nonce"]])
        cid = self.outer(["create-daemon", owner["outer_name"], token, owner["outer_owner"], "bridge",
            owner["volume_name"], str(socket_dir), "/boxferry-core", "upstream-rootless", image,
            str(directory / "outer.log"), runner["nonce"]]).decode().strip()
        need(cid == c.owner_cid(owner))
        self.cid = cid
        self.wait_file(socket_dir / "docker.sock")
        self.legacy("ready")
        self.socket = socket_dir / "docker.sock"
        self.http = c.bind_unix_http(self.sdk / "scripts/native-unix-http.py", b["sdk"]["revision"],
                                   b["sdk"]["files"]["scripts/native-unix-http.py"])
        self.bind()
        # Explicit readiness, version and mapping are native prerequisites.
        version = self.request("GET", "/version", expected=200)
        need(version["Version"] == "29.8.1" and version["ApiVersion"] == "1.56")
        info = self.request("GET", "/v1.56/info", expected=200)
        need(info["DockerRootDir"] == "/home/docker/.local/share/docker"
             and any("rootless" in option for option in info["SecurityOptions"]))
        for role in ROLES:
            self.create_role(role)

    def wait_file(self, path, *, guard_required=True):
        while not path.exists():
            self.boundary()
            need(not guard_required or self.guard is None or self.guard.poll() is None)
            time.sleep(.01)

    def legacy(self, phase):
        return self.c.owner_capability(self.path, self.digest, phase=phase, context=self.owner["context"],
            execution_deadline=self.deadline, cleanup_deadline=self.cleanup_deadline)

    def request(self, method, path, body=None, *, expected):
        self.boundary()
        need(self.calls + 1 <= 256 and self.bytes + LIMIT + 1 <= 16 * LIMIT)
        status, raw = self.http.request(self.socket, method, path, body, timeout=min(15, self.deadline - self.c.owner_clock()),
            max_response_bytes=LIMIT, max_request_bytes=LIMIT,
            content_type="application/json" if body is not None else None, peer_lease=self.peer)
        self.charge(1, len(raw))
        need(status == expected)
        return json.loads(raw) if raw else None

    def create_role(self, role):
        need(role in ROLES)
        name = self.owner["prefix"] + "-paper-" + role
        labels = {"io.boxferry.live-run": self.owner["run"],
                  "io.boxferry.application": self.owner["prefix"] + "-paperless"}
        value = self.request("POST", "/v1.56/volumes/create", encoded({"Name": name, "Driver": "local", "Labels": labels}), expected=201)
        need(value["Name"] == name and value["Driver"] == "local" and value["Labels"] == labels
             and value.get("Options") in (None, {}) and value["Mountpoint"] ==
             "/home/docker/.local/share/docker/volumes/" + name + "/_data")

    def peer(self, stream):
        expected = self.inner_peer
        need(struct.unpack("3i", stream.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
             == (expected["pid"], expected["uid"], expected["gid"]))
        fd = self.c.verified_parent_fd(expected["pid"], expected["start"])
        caller = self
        class Peer:
            def check(self):
                caller.boundary()
                caller.c.owner_validate(caller.owner)
                need(caller.c.parent_alive(fd) and caller.socket.lstat() == caller.socket_stamp)

            def reserve_read(self, size):
                self.check()
                need(type(size) is int and 0 < size <= LIMIT and caller.bytes + size + 1 <= 16 * LIMIT)

            def close(self):
                os.close(fd)
        return Peer()

    def bind(self):
        c, owner = self.c, self.owner
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as stream:
            stream.settimeout(min(15, self.deadline - c.owner_clock()))
            stream.connect(str(self.socket))
            pid, uid, gid = struct.unpack("3i", stream.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
        need(pid > 0 and uid > 0 and gid > 0)
        self.inner_peer = {"pid": pid, "start": c.process_start(pid), "uid": uid, "gid": gid}
        info = self.socket.lstat()
        self.socket_stamp = info
        socket_identity = {"device": info.st_dev, "inode": info.st_ino, "uid": info.st_uid,
                           "gid": info.st_gid, "mode": stat.S_IMODE(info.st_mode)}
        manifest_directory = self.directory / ("boxferry-paperless." + secrets.token_hex(4))
        manifest_directory.mkdir(mode=0o700)
        self.cutoff = min(self.deadline, c.owner_clock() + 60) if self.binding["case"] == "expiry" else self.deadline
        endpoint = {"id": owner["context"], "uri": "unix://" + str(self.socket), "scope": "owned-inner",
                    "mode": "rootless", "ownership_sha256": self.digest}
        manifest = manifest_directory / "manifest.json"
        write_new(manifest, {"config": {"stage": "source-fixture", "target": None, "prefix": owner["prefix"],
            "run_id": owner["run"], "endpoint": endpoint, "work_deadline_ms": time.time_ns() // 1_000_000
            + int((self.cutoff - c.owner_clock()) * 1000)}})
        private = self.directory / "paperless-transport"
        private.mkdir(mode=0o700)
        binding = {"arguments": {"custody_path": str(self.path), "custody_sha256": self.digest,
            "source_hashes": owner["paperless_sources"], "manifest_path": str(manifest),
            "manifest_sha256": hashlib.sha256(manifest.read_bytes()).hexdigest(), "inner_peer": self.inner_peer,
            "socket_identity": socket_identity}, "owner": owner, "protocol_identity": [], "directory_identity": [],
            "lock_identity": [], "cutoff": self.cutoff, "cleanup_cutoff": self.cleanup_deadline, "ledger_key": secrets.token_hex(32)}
        write_new(private / "binding.json", binding)
        self.binding_sha = hashlib.sha256(encoded(binding)).hexdigest()

    def phase(self, stage):
        marker = json.loads(read_private(self.directory / "owner-ready.json"))
        write_new(self.binding_path.parent / f"phase-{self.phase_number}.json", {"stage": stage,
            "owner": self.owner, "custody_sha256": self.digest, "guard": marker, "cid": getattr(self, "cid", None),
            "case": self.binding["case"], "calls": self.calls, "read_bytes": self.bytes,
            "callback_pids": self.callback_pids, "callbacks": self.callbacks,
            "inner_peer": getattr(self, "inner_peer", None)})
        self.wait_file(self.binding_path.parent / f"ack-{self.phase_number}.json", guard_required=stage != "closed")
        ack = json.loads(read_private(self.binding_path.parent / f"ack-{self.phase_number}.json"))
        need(set(ack) == {"stage", "calls", "read_bytes"} and ack["stage"] == stage
             and type(ack["calls"]) is int and ack["calls"] >= 0
             and type(ack["read_bytes"]) is int and ack["read_bytes"] >= 0)
        self.charge(ack["calls"], ack["read_bytes"])
        self.phase_number += 1

    def volume_phase(self, phase, *, accepted=True, lost_ack=False):
        holding = phase == "paperless-hold"
        call_bound, read_bound = (6, 2 * CHECK_BYTES) if holding else ((0, 0) if phase == "paperless-close" else (3, CHECK_BYTES))
        need(self.calls + call_bound <= 256 and self.bytes + read_bound <= 16 * LIMIT)
        command = [sys.executable, str(Path(__file__).resolve()), "callback", "--custody", str(self.path),
            "--digest", self.digest, "--phase", phase, "--binding", self.binding_sha, "--sequence", str(self.sequence),
            "--control-binding", str(self.binding_path)]
        if lost_ack:
            command.append("--lost-ack")
        process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        pidfd = None
        try:
            start = self.c.process_start(process.pid)
            pidfd = self.c.verified_parent_fd(process.pid, start)
            sid = os.getsid(process.pid)
            raw = process.communicate(timeout=max(.001, self.deadline - self.c.owner_clock()))[0]
            need(len(raw) <= 4096)
            need((process.returncode == 0) == accepted)
            if accepted:
                value = json.loads(raw)
                need((value["pid"], value["start"], value["sid"]) == (process.pid, start, sid)
                     and sid == self.owner["parent_pid"])
                self.callback_pids.append(value["pid"])
                self.callbacks.append({key: value[key] for key in ("pid", "start", "sid")})
                if not lost_ack:
                    reply = value["reply"]
                    need(reply["sequence"] == self.sequence + 1
                         and reply["calls"] - self.custody_calls == call_bound
                         and 0 <= reply["read_bytes"] - self.custody_bytes <= read_bound)
                    self.charge(reply["calls"] - self.custody_calls, reply["read_bytes"] - self.custody_bytes)
                    self.sequence, self.custody_calls, self.custody_bytes = reply["sequence"], reply["calls"], reply["read_bytes"]
            self.uncertain |= not accepted or lost_ack
        finally:
            if process.poll() is None:
                process.kill()
            process.wait(timeout=2)
            process.stdout.close()
            if pidfd is not None:
                os.close(pidfd)

    def run(self):
        print("MECHANISM: fixed owned fixture started; no application acceptance", flush=True)
        self.prepare()
        self.volume_phase("paperless-hold")
        self.volume_phase("paperless-check")
        need(len(set(self.callback_pids)) == 2)
        self.phase("held")  # Worker retains authentic incarnations before any loss.
        case = self.binding["case"]
        if case == "owner-loss":
            print("MECHANISM: authentic owner loss armed", flush=True)
            os.kill(os.getpid(), signal.SIGKILL)
        elif case == "guard-loss":
            self.guard.kill()
            self.guard.wait(timeout=max(.001, self.cleanup_deadline - self.c.owner_clock()))
            self.phase("closed")
            self.c.cleanup_owner_custody(self.owner, bound_cid=self.cid)
        elif case.startswith("replace-"):
            role = case.removeprefix("replace-")
            name = self.owner["prefix"] + "-paper-" + role
            self.request("DELETE", "/v1.56/volumes/" + name, expected=204)
            self.create_role(role)
            self.phase("replaced")
            self.volume_phase("paperless-check", accepted=False)
            self.guard.wait(timeout=max(.001, self.cleanup_deadline - self.c.owner_clock()))
        elif case == "expiry":
            self.guard.wait(timeout=max(.001, self.cleanup_deadline - self.c.owner_clock()))
            self.uncertain = True
        elif case == "lost-ack":
            self.volume_phase("paperless-check", lost_ack=True)
            self.guard.wait(timeout=max(.001, self.cleanup_deadline - self.c.owner_clock()))
        else:
            self.volume_phase("paperless-close")
            self.phase("closed")
            self.legacy("close")
            self.guard.wait(timeout=max(.001, self.cleanup_deadline - self.c.owner_clock()))
        need(not self.directory.exists())
        write_new(self.binding_path.parent / "result.json", {"case": case, "callback_pids": self.callback_pids,
            "callbacks": self.callbacks, "calls": self.calls, "read_bytes": self.bytes, "uncertain": self.uncertain})
        self.closed = True

    def recover(self):
        if self.directory is not None and not self.prepared:
            # Before durable custody, no runtime create has been selected.
            # Discard only this original owner's proven private temporary tree.
            self.c.owner_remove_private({"directory": str(self.directory),
                "directory_identity": self.directory_identity, "cleanup_deadline": self.cleanup_deadline})
            return
        if self.owner is not None:
            if self.guard is not None and self.guard.poll() is None:
                try:
                    self.legacy("close")
                finally:
                    self.guard.wait(timeout=max(.001, self.cleanup_deadline - self.c.owner_clock()))
            elif self.path.exists():
                # The original live owner, never the observer, retains its
                # original exact-owned recovery after proven guard death.
                self.c.cleanup_owner_custody(self.owner, bound_cid=getattr(self, "cid", None))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_subparsers(dest="mode", required=True)
    run = modes.add_parser("run")
    run.add_argument("--binding", type=Path, required=True)
    child = modes.add_parser("callback")
    child.add_argument("--custody", type=Path, required=True)
    child.add_argument("--digest", required=True)
    child.add_argument("--phase", choices=("paperless-hold", "paperless-check", "paperless-close"), required=True)
    child.add_argument("--binding", required=True)
    child.add_argument("--sequence", type=int, required=True)
    child.add_argument("--lost-ack", action="store_true")
    child.add_argument("--control-binding", type=Path)
    args = parser.parse_args()
    mechanism = None
    try:
        if args.mode == "callback":
            need(args.control_binding is not None)
            callback(args.custody, args.digest, args.phase, args.binding, args.sequence, args.lost_ack, args.control_binding)
        else:
            mechanism = Mechanism(args.binding)
            mechanism.run()
        return 0
    except Exception:
        if mechanism is not None:
            try:
                mechanism.recover()
            except Exception:
                pass  # Failure remains failure; the trusted worker reports residuals.
        print("fixed custody mechanism refused; no native acceptance", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
