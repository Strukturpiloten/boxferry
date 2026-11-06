"""One explicit local-rootful PID/mount lifetime; no reusable privileged service.

Root code is a frozen isolated stdlib payload. All package imports and qualification
run only after dropping to the independently observed original caller identity.
Root READY namespace strings enter only the post-G worker argv; the dropped worker
checks its own namespace links before importing any package.
"""

from __future__ import annotations

import hashlib
import inspect
import json
import os
import select
import signal
import stat
import subprocess
import tempfile
import time
import uuid
from collections.abc import Callable, Mapping
from contextlib import ExitStack
from pathlib import Path
from typing import Any

from .contracts import ContractError
from .runner import AbortContext, RunCancelled, protected_values, redact, run

LIFETIME_SECONDS = 5400
STARTUP_SECONDS = 8
ESCALATION_SECONDS = 2
CLOSURE_SECONDS = 6
MAX_WITNESS = 16384
OWNED_CHILDREN = (
    "store",
    "runroot",
    "podman-tmp",
    "home",
    "config",
    "cache",
    "runtime",
    "docker-config",
    "boxferry",
    "compose-lens",
    "docker-lens",
    "kubernetes-lens",
    "podman-lens",
    "quadlet-lens",
    "boxferry-website",
    "podman",
    "containers.conf",
    "image-passwd",
    ".lifetime-nonce",
)


class LifetimeAbort(ContractError):
    """A terminal runner failure must not start another inner cleanup budget."""


# This code executes only after PID1 has dropped every saved/supplementary identity.
WORKER_PROGRAM = r"""
import json, os, pathlib, sys
plan = json.loads(sys.argv[1])
assert os.getuid() == os.geteuid() == plan["uid"] != 0
assert os.getgid() == os.getegid() == plan["gid"]
assert sorted(os.getgroups()) == sorted(plan["groups"])
os.set_inheritable(plan["abort_fd"], False)
# Root control/host-proc descriptors never reach package imports or exec children.
for value in os.listdir("/proc/self/fd"):
    fd = int(value)
    if fd <= 2 or fd == plan["abort_fd"]:
        continue
    try:
        os.fstat(fd)
    except OSError:
        continue  # listdir's own directory descriptor has already closed.
    raise RuntimeError("unexpected inherited worker descriptor")
# These strings come only from root PID1's READY snapshot after authenticated G.
for key, leaf in (("pid_namespace", "pid"), ("mount_namespace", "mnt")):
    expected = plan.get("pid1_" + key)
    prefix = leaf + ":["
    if (not isinstance(expected, str) or len(expected) > 64
        or not expected.startswith(prefix) or not expected.endswith("]")
        or not expected[len(prefix):-1].isdecimal()
        or expected == plan["host_" + key]
        or os.readlink("/proc/self/ns/" + leaf) != expected):
        raise RuntimeError("root-attested worker namespace mismatch")
if sys.version_info < (3, 13):
    raise RuntimeError("selected automation worker requires Python 3.13 or newer")
sys.path.insert(0, plan["module_root"])
from boxferry_ci import devcontainer_validation as validation
kwargs = plan["qualification"]
for key in ("source", "policy", "automation", "work"):
    kwargs[key] = pathlib.Path(kwargs[key])
if kwargs["local_sources"] is not None:
    kwargs["local_sources"] = {
        key: pathlib.Path(value) for key, value in kwargs["local_sources"].items()
    }
kwargs["lifetime_context"] = {
    "private": plan["scope"], "nonce": plan["nonce"],
    "pid_namespace": os.readlink("/proc/self/ns/pid"),
    "mount_namespace": os.readlink("/proc/self/ns/mnt"),
    "host_pid_namespace": plan["host_pid_namespace"],
    "host_mount_namespace": plan["host_mount_namespace"],
    "pid1_pid_namespace": plan["pid1_pid_namespace"],
    "pid1_mount_namespace": plan["pid1_mount_namespace"],
    "abort_fd": plan["abort_fd"], "result": plan["result"],
    "result_identity": plan["result_identity"],
}
report = validation.qualify(**kwargs)
fd = os.open(plan["result"], os.O_WRONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
facts = os.fstat(fd)
assert [facts.st_dev, facts.st_ino] == plan["result_identity"]
assert facts.st_uid == plan["uid"] and facts.st_mode & 0o777 == 0o600
os.ftruncate(fd, 0)
os.write(fd, validation.serialize_report(report).encode() + b"\n")
os.close(fd)
raise SystemExit(0 if report.get("status") == "success" else 1)
"""


WORKER = WORKER_PROGRAM
INIT: str


def _r_identity(fd: int) -> list[int]:
    value = os.fstat(fd)
    return [value.st_dev, value.st_ino, value.st_uid, stat.S_IMODE(value.st_mode)]


def _r_directory(path: str) -> int:
    """Open every absolute directory component without following a link."""
    if not path.startswith("/") or any(part in {".", ".."} for part in path.split("/")):
        raise ValueError("invalid absolute custody path")
    fd = os.open("/", os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    try:
        for part in filter(None, path.split("/")):
            child = os.open(
                part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=fd
            )
            os.close(fd)
            fd = child
        return fd
    except BaseException:
        os.close(fd)
        raise


def _r_start(pid: int) -> str:
    with open(f"/proc/{pid}/stat", encoding="ascii") as stream:
        return stream.read(8192).rsplit(")", 1)[1].split()[19]


def _r_status(pid: int) -> dict[str, str]:
    with open(f"/proc/{pid}/status", encoding="ascii") as stream:
        return dict(line.rstrip().split(":", 1) for line in stream if ":" in line)


def _r_only_child(supervisor: int) -> int:
    # This is the exact held Popen child's own list, never host-wide discovery.
    with open(f"/proc/{supervisor}/task/{supervisor}/children", "rb") as stream:
        content = stream.read(257)
    values = content.split()
    if len(content) > 256 or len(values) != 1 or not values[0].isdigit() or int(values[0]) <= 0:
        raise ValueError("owned supervisor must expose exactly one namespace child")
    return int(values[0])


def _r_payload_hash() -> str:
    with open("/proc/self/cmdline", "rb") as stream:
        command = stream.read(131073)
    parts = command.split(b"\0")
    if (
        len(command) > 131072
        or len(parts) < 5
        or parts[:4] != [b"/usr/bin/python3", b"-I", b"-S", b"-c"]
    ):
        raise ValueError("unexpected fixed root interpreter argv")
    return hashlib.sha256(parts[4]).hexdigest()


def _r_auth(plan: dict[str, Any]) -> tuple[int, int, list[int]]:
    if os.getuid() != 0 or os.geteuid() != 0:
        raise ValueError("fixed bootstrap requires real root through sudo")
    uid, gid = int(os.environ["SUDO_UID"]), int(os.environ["SUDO_GID"])
    caller = plan["caller_pid"]
    if (
        uid <= 0
        or uid != plan["uid"]
        or gid != plan["gid"]
        or _r_start(caller) != plan["caller_start"]
    ):
        raise ValueError("original operator identity mismatch")
    facts = _r_status(caller)
    if [int(value) for value in facts["Uid"].split()] != [uid] * 4 or [
        int(value) for value in facts["Gid"].split()
    ] != [gid] * 4:
        raise ValueError("operator changed saved/effective identity")
    ancestor = os.getppid()
    for _ in range(8):
        if ancestor == caller:
            break
        ancestor = int(_r_status(ancestor)["PPid"])
    else:
        raise ValueError("sudo ancestry does not bind the original caller")
    groups = [int(value) for value in facts["Groups"].split()]
    if len(groups) > 128:
        raise ValueError("operator group evidence exceeds bound")
    return uid, gid, groups


def _r_environment(scope: str) -> dict[str, str]:
    return {
        "PATH": "/usr/local/bin:/usr/bin:/bin",
        "HOME": scope + "/home",
        "DOCKER_CONFIG": scope + "/docker-config",
        "XDG_CONFIG_HOME": scope + "/config",
        "XDG_CACHE_HOME": scope + "/cache",
        "XDG_RUNTIME_DIR": scope + "/runtime",
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": "/dev/null",
        "GIT_TERMINAL_PROMPT": "0",
        "GIT_ASKPASS": "/bin/false",
        "LC_ALL": "C.UTF-8",
        "CI": "true",
    }


def _r_control(plan: dict[str, Any], name: str, flags: int, *, claim: bool) -> int:
    parent = _r_directory(plan["preparation"])
    try:
        if _r_identity(parent) != plan["preparation_identity"] or plan["preparation_identity"][
            2:
        ] != [plan["uid"], 0o700]:
            raise ValueError("control directory custody changed")
        fd = os.open(name, flags | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK, dir_fd=parent)
        facts = os.fstat(fd)
        expected = plan[name + "_identity"]
        if (
            [facts.st_dev, facts.st_ino] != expected
            or facts.st_uid != (plan["uid"] if claim else 0)
            or stat.S_IMODE(facts.st_mode) != (0o600 if claim else 0o400)
        ):
            os.close(fd)
            raise ValueError("control inode custody mismatch")
        if (
            facts.st_nlink != 1
            or (name == "lease" and not stat.S_ISFIFO(facts.st_mode))
            or (name == "witness" and not stat.S_ISREG(facts.st_mode))
        ):
            os.close(fd)
            raise ValueError("invalid control inode")
        if claim and name == "witness" and facts.st_size != 0:
            os.close(fd)
            raise ValueError("root witness must start empty, never stale user data")
        if claim:
            # Claim this already-open inode. Never unlink a possibly replaced name.
            os.fchown(fd, 0, 0)
            os.fchmod(fd, 0o400)
            locked = os.fstat(fd)
            if (
                [locked.st_dev, locked.st_ino] != expected
                or locked.st_uid != 0
                or stat.S_IMODE(locked.st_mode) != 0o400
                or locked.st_nlink != 1
            ):
                os.close(fd)
                raise ValueError("control inode claim was not established")
        return fd
    finally:
        os.close(parent)


def _r_lease(fd: int) -> bool:
    try:
        value = os.read(fd, 1)
    except BlockingIOError:
        return True
    if value:
        raise ValueError("unexpected lease bytes")
    return False


def _r_event(fd: int, nonce: str, event: str, **values: object) -> None:
    encoded = (
        json.dumps({"nonce": nonce, "event": event, **values}, sort_keys=True).encode() + b"\n"
    )
    if len(encoded) > 2048 or os.fstat(fd).st_size + len(encoded) > 16384:
        raise ValueError("root witness bound exceeded")
    os.write(fd, encoded)


def _r_drop(uid: int, gid: int, groups: list[int]) -> None:
    os.setgroups(groups)
    os.setresgid(gid, gid, gid)
    os.setresuid(uid, uid, uid)
    if (
        os.getresuid() != (uid, uid, uid)
        or os.getresgid() != (gid, gid, gid)
        or sorted(os.getgroups()) != sorted(groups)
    ):
        raise ValueError("worker identity drop failed")


def _r_init(plan: dict[str, Any], lease: int, witness: int, ready: int, go: int) -> None:
    if os.getuid() != 0 or os.getpid() != 1:
        raise ValueError("fixed init must remain root PID1")
    aborted = False

    def cancel(_number: int, _frame: object) -> None:
        nonlocal aborted
        aborted = True

    signal.signal(signal.SIGTERM, cancel)
    signal.signal(signal.SIGINT, cancel)
    signal.signal(signal.SIGHUP, cancel)
    identity = {
        "nonce": plan["nonce"],
        "pid": os.getpid(),
        "uid": os.geteuid(),
        "pid_namespace": os.readlink("/proc/self/ns/pid"),
        "mount_namespace": os.readlink("/proc/self/ns/mnt"),
    }
    os.write(ready, json.dumps(identity).encode() + b"\n")
    os.close(ready)
    deadline = time.monotonic() + 8
    while not aborted and _r_lease(lease) and time.monotonic() < deadline:
        if select.select([go], [], [], 0.02)[0]:
            if os.read(go, 2) == b"G":
                break
            aborted = True
    else:
        aborted = True
    os.close(go)
    if aborted or not _r_lease(lease):
        os._exit(125)
    # G attests this exact READY snapshot via the bootstrap's bound live pidfd.
    # Never read these fields from the outer plan or the unchanged PID1 cmdline.
    worker_plan = {
        **plan,
        "pid1_pid_namespace": identity["pid_namespace"],
        "pid1_mount_namespace": identity["mount_namespace"],
    }
    notify_read, notify_write = os.pipe2(os.O_CLOEXEC | os.O_NONBLOCK)
    child = os.fork()
    if child == 0:
        controls = (lease, witness, notify_read)
        for fd in controls:
            os.close(fd)
        _r_drop(plan["uid"], plan["gid"], plan["groups"])
        os.set_inheritable(notify_write, True)
        worker_plan["abort_fd"] = notify_write
        worker_plan["control_fds"] = [fd for fd in (*controls, ready, go) if fd != notify_write]
        os.execve(
            "/usr/bin/python3",
            ("/usr/bin/python3", "-I", "-S", "-c", WORKER, json.dumps(worker_plan)),
            plan["environment"],
        )
    os.close(notify_write)
    termination = None
    worker_status = None
    while True:
        try:
            notice = os.read(notify_read, 2)
        except BlockingIOError:
            notice = b""
        if notice:
            aborted = True  # Any notification bytes abort, never an arbitrary command.
        if aborted or not _r_lease(lease):
            if termination is None:
                termination = time.monotonic()
                try:
                    os.kill(child, signal.SIGTERM)
                except ProcessLookupError:
                    pass
            if time.monotonic() - termination >= 2:
                os._exit(125)  # PID1 exit makes the kernel kill ALL namespace descendants.
        try:
            pid, status = os.waitpid(-1, os.WNOHANG)
        except ChildProcessError:
            pid = 0
        if pid == child:
            worker_status = os.waitstatus_to_exitcode(status)
            break
        time.sleep(0.02)
    _r_event(witness, plan["nonce"], "worker-exit", status=worker_status)
    os._exit(0 if worker_status == 0 and termination is None else 125)


def _r_bootstrap(plan: dict[str, Any]) -> int:
    uid, gid, groups = _r_auth(plan)
    plan["uid"], plan["gid"], plan["groups"] = uid, gid, groups
    if set(plan) != set(PLAN_KEYS) or plan["schema"] != 1 or len(plan["nonce"]) != 32:
        raise ValueError("invalid closed lifetime plan")
    if plan["environment"] != _r_environment(plan["scope"]):
        raise ValueError("noncanonical privileged environment")
    if plan["host_pid_namespace"] != os.readlink("/proc/self/ns/pid") or plan[
        "host_mount_namespace"
    ] != os.readlink("/proc/self/ns/mnt"):
        raise ValueError("original outer namespace facts differ from root bootstrap")
    scope = _r_directory(plan["scope"])
    if _r_identity(scope) != plan["scope_identity"] or plan["scope_identity"][2:] != [uid, 0o700]:
        raise ValueError("private lifetime custody mismatch")
    if os.listdir(scope) != [".lifetime-nonce"]:
        raise ValueError("one lifetime requires a fresh store, never cross-namespace reuse")
    marker = os.open(".lifetime-nonce", os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=scope)
    try:
        facts = os.fstat(marker)
        if (
            not stat.S_ISREG(facts.st_mode)
            or facts.st_uid != uid
            or stat.S_IMODE(facts.st_mode) != 0o600
            or facts.st_nlink != 1
            or os.read(marker, 64).decode() != plan["nonce"]
        ):
            raise ValueError("fresh private nonce custody mismatch")
    finally:
        os.close(marker)
        os.close(scope)
    lease = _r_control(plan, "lease", os.O_RDONLY, claim=True)
    witness = _r_control(plan, "witness", os.O_WRONLY | os.O_APPEND, claim=True)
    if not _r_lease(lease):
        _r_event(witness, plan["nonce"], "abort-before-start")
        os.close(lease)
        os.close(witness)
        return 125
    ready_read, ready_write = os.pipe2(os.O_CLOEXEC)
    go_read, go_write = os.pipe2(os.O_CLOEXEC)
    child_plan = json.dumps(plan)
    command = (
        "/usr/bin/unshare",
        "--mount",
        "--propagation=private",
        "--pid",
        "--fork",
        "--kill-child=SIGKILL",
        "--forward-signals",
        "--mount-proc",
        "/usr/bin/python3",
        "-I",
        "-S",
        "-c",
        INIT,
        child_plan,
        str(lease),
        str(witness),
        str(ready_write),
        str(go_read),
    )
    process = subprocess.Popen(
        command,
        env=plan["environment"],
        close_fds=True,
        pass_fds=(lease, witness, ready_write, go_read),
    )
    os.close(ready_write)
    os.close(go_read)
    os.set_blocking(ready_read, False)
    init_fd = None
    record = b""
    start = time.monotonic()
    deadline = start + 5400
    escalation = None
    released = False
    aborted = False
    init_record = None

    def cancel(_number: int, _frame: object) -> None:
        nonlocal aborted
        aborted = True

    signal.signal(signal.SIGTERM, cancel)
    signal.signal(signal.SIGINT, cancel)
    signal.signal(signal.SIGHUP, cancel)
    try:
        while True:
            try:
                alive = _r_lease(lease)
            except (OSError, ValueError):
                alive = False
            if not alive or time.monotonic() >= deadline:
                aborted = True
            if init_fd is None and time.monotonic() - start >= 8:
                aborted = True
            if init_fd is None:
                try:
                    record += os.read(ready_read, 2049 - len(record))
                except BlockingIOError:
                    pass
                if len(record) > 2048:
                    raise ValueError("PID1 identity handoff overflow")
                if b"\n" in record:
                    init_record = json.loads(record)
                    if (
                        set(init_record)
                        != {"nonce", "pid", "uid", "pid_namespace", "mount_namespace"}
                        or init_record["nonce"] != plan["nonce"]
                        or init_record["pid"] != 1
                        or init_record["uid"] != 0
                    ):
                        raise ValueError("fixed PID1 READY identity mismatch")
                    pid = _r_only_child(process.pid)
                    start_identity = _r_start(pid)
                    candidate_fd = os.pidfd_open(pid, 0)
                    try:
                        facts = _r_status(pid)
                        if (
                            _r_start(pid) != start_identity
                            or int(facts["PPid"]) != process.pid
                            or facts["Uid"].split() != ["0"] * 4
                            or facts["NSpid"].split()[-1] != "1"
                            or os.readlink(f"/proc/{pid}/ns/pid") != init_record["pid_namespace"]
                            or os.readlink(f"/proc/{pid}/ns/mnt") != init_record["mount_namespace"]
                            or init_record["pid_namespace"] == plan["host_pid_namespace"]
                            or init_record["mount_namespace"] == plan["host_mount_namespace"]
                            or select.select([candidate_fd], [], [], 0)[0]
                        ):
                            raise ValueError(
                                "PID1 custody is not the exact live root unshare child"
                            )
                    except BaseException:
                        os.close(candidate_fd)
                        raise
                    init_fd = candidate_fd
                    _r_event(
                        witness,
                        plan["nonce"],
                        "pid1-bound",
                        pid=pid,
                        start=start_identity,
                        scope_identity=plan["scope_identity"],
                        preparation_identity=plan["preparation_identity"],
                        caller_uid=uid,
                        bootstrap_sha256=_r_payload_hash(),
                    )
                    if (
                        not aborted
                        and _r_lease(lease)
                        and not select.select([init_fd], [], [], 0)[0]
                    ):
                        os.write(go_write, b"G")
                        released = True
                    os.close(go_write)
                    go_write = -1
            if aborted:
                if escalation is None:
                    escalation = time.monotonic()
                    if init_fd is not None:
                        if not select.select([init_fd], [], [], 0)[0]:
                            try:
                                signal.pidfd_send_signal(init_fd, signal.SIGTERM)
                            except ProcessLookupError:
                                pass  # A readable pidfd below still proves exact death.
                    else:
                        process.terminate()  # Known exact child only; no worker GO exists.
                if time.monotonic() - escalation >= 2:
                    if init_fd is not None and not select.select([init_fd], [], [], 0)[0]:
                        try:
                            signal.pidfd_send_signal(init_fd, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                    if process.poll() is None:
                        process.kill()
                if time.monotonic() - escalation >= 4:
                    break
            code = process.poll()
            if code is not None:
                closed = init_fd is not None and bool(select.select([init_fd], [], [], 0)[0])
                if closed:
                    _r_event(
                        witness,
                        plan["nonce"],
                        "closed",
                        namespace_closed=True,
                        supervisor_closed=True,
                        released=released,
                        aborted=aborted,
                        returncode=code,
                    )
                else:
                    _r_event(witness, plan["nonce"], "uncertain", namespace_closed=False)
                return 0 if closed and code == 0 and not aborted else 125
            time.sleep(0.02)
        _r_event(witness, plan["nonce"], "uncertain", namespace_closed=False)
        return 125
    except BaseException:
        if init_fd is not None:
            if not select.select([init_fd], [], [], 0)[0]:
                signal.pidfd_send_signal(init_fd, signal.SIGKILL)
        if process.poll() is None:
            process.kill()
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            pass
        raise
    finally:
        if go_write >= 0:
            os.close(go_write)
        os.close(ready_read)
        if init_fd is not None:
            os.close(init_fd)
        os.close(lease)
        os.close(witness)


def _r_mount(fd: int) -> str:
    with open(f"/proc/self/fdinfo/{fd}", encoding="ascii") as stream:
        return dict(line.split(":", 1) for line in stream if ":" in line)["mnt_id"].strip()


def _r_remove(
    parent: int,
    name: str,
    device: int,
    mount: str,
    budget: list[int],
    relative: tuple[str, ...] = (),
) -> None:
    budget[0] -= 1
    if budget[0] < 0:
        raise ValueError("finite cleanup entry bound exhausted")
    path = (*relative, name)
    facts = os.stat(name, dir_fd=parent, follow_symlinks=False)
    if stat.S_ISDIR(facts.st_mode):
        child = os.open(
            name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=parent
        )
        try:
            opened = os.fstat(child)
            if (
                (opened.st_dev, opened.st_ino) != (facts.st_dev, facts.st_ino)
                or opened.st_dev != device
                or _r_mount(child) != mount
            ):
                raise ValueError("cleanup refuses substituted directory or mount")
            os.fchown(child, 0, 0)
            os.fchmod(child, 0o700)
            with os.scandir(child) as entries:
                for entry in entries:
                    _r_remove(child, entry.name, device, mount, budget, path)
            now = os.stat(name, dir_fd=parent, follow_symlinks=False)
            if (now.st_dev, now.st_ino) != (opened.st_dev, opened.st_ino):
                raise ValueError("cleanup directory entry was replaced")
            os.rmdir(name, dir_fd=parent)
        finally:
            os.close(child)
    elif stat.S_ISCHR(facts.st_mode):
        # Only the observed private overlay layer whiteout shape is allowed.
        # The scope-relative context originates at _r_finalize's held root fd.
        if not (
            len(path) >= 5
            and path[:2] == ("store", "overlay")
            and len(path[2]) == 64
            and all(character in "0123456789abcdef" for character in path[2])
            and path[3] == "diff"
            and facts.st_dev == device
            and facts.st_rdev == os.makedev(0, 0)
            and stat.S_IMODE(facts.st_mode) == 0
            and facts.st_uid == facts.st_gid == 0
            and facts.st_nlink == 1
        ):
            raise ValueError("cleanup refuses unexpected special inode")
        # O_PATH holds metadata only: never open a device for I/O, even after
        # a replacement race. O_NOFOLLOW also holds a substituted link itself.
        leaf = os.open(name, os.O_PATH | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=parent)
        try:
            fields = ("st_dev", "st_ino", "st_mode", "st_rdev", "st_uid", "st_gid", "st_nlink")
            held = tuple(getattr(facts, field) for field in fields)
            opened = os.fstat(leaf)
            if tuple(getattr(opened, field) for field in fields) != held or _r_mount(leaf) != mount:
                raise ValueError("cleanup refuses substituted whiteout or mount")
            current = os.stat(name, dir_fd=parent, follow_symlinks=False)
            if tuple(getattr(current, field) for field in fields) != held:
                raise ValueError("cleanup whiteout entry was replaced")
            os.unlink(name, dir_fd=parent)
        finally:
            os.close(leaf)
    else:
        if not (
            stat.S_ISREG(facts.st_mode)
            or stat.S_ISLNK(facts.st_mode)
            or stat.S_ISFIFO(facts.st_mode)
            or stat.S_ISSOCK(facts.st_mode)
        ):
            raise ValueError("cleanup refuses unexpected special inode")
        os.unlink(name, dir_fd=parent)


def _r_finalize(plan: dict[str, Any]) -> None:
    uid, gid, _groups = _r_auth(plan)
    if set(plan) != set(PLAN_KEYS):
        raise ValueError("unknown finalization plan field")
    witness = _r_control(plan, "witness", os.O_RDONLY, claim=False)
    records = [json.loads(line) for line in os.read(witness, 16385).splitlines()]
    os.close(witness)
    closed = [record for record in records if record.get("event") == "closed"]
    bound = [record for record in records if record.get("event") == "pid1-bound"]
    if (
        len(closed) != 1
        or len(bound) != 1
        or not records
        or records[-1] != closed[0]
        or closed[0].get("namespace_closed") is not True
        or closed[0].get("supervisor_closed") is not True
        or bound[0].get("scope_identity") != plan["scope_identity"]
        or bound[0].get("preparation_identity") != plan["preparation_identity"]
        or bound[0].get("caller_uid") != uid
        or bound[0].get("bootstrap_sha256") != _r_payload_hash()
        or any(
            record.get("event") not in {"pid1-bound", "worker-exit", "closed"} for record in records
        )
        or any(record.get("nonce") != plan["nonce"] for record in records)
    ):
        raise ValueError("directory cleanup requires proved exact namespace closure")
    parent = _r_directory(str(Path(plan["scope"]).parent))
    name = Path(plan["scope"]).name
    scope = os.open(
        name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=parent
    )
    try:
        if _r_identity(scope) != plan["scope_identity"]:
            raise ValueError("outer finalizer custody mismatch")
        entries = []
        with os.scandir(scope) as stream:
            for entry in stream:
                if entry.name not in OWNED_CHILDREN or len(entries) >= len(OWNED_CHILDREN):
                    raise ValueError("unrecorded private child prevents directory cleanup")
                entries.append(entry.name)
        nonce = os.open(".lifetime-nonce", os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=scope)
        try:
            marker = os.fstat(nonce)
            if (
                not stat.S_ISREG(marker.st_mode)
                or marker.st_uid != uid
                or stat.S_IMODE(marker.st_mode) != 0o600
                or marker.st_nlink != 1
                or os.read(nonce, 64).decode() != plan["nonce"]
            ):
                raise ValueError("private scope nonce mismatch")
        finally:
            os.close(nonce)
        # Namespace is gone. Claim this held directory before traversing entries;
        # user workers cannot replace descendants during privileged finalization.
        os.fchown(scope, 0, 0)
        os.fchmod(scope, 0o700)
        mount = _r_mount(scope)
        budget = [1000000]
        for entry in entries:
            _r_remove(scope, entry, os.fstat(scope).st_dev, mount, budget)
        current = os.stat(name, dir_fd=parent, follow_symlinks=False)
        if (current.st_dev, current.st_ino) != tuple(plan["scope_identity"][:2]):
            raise ValueError("outer parent entry changed; never unlink replacement")
        control = _r_directory(plan["preparation"])
        try:
            if _r_identity(control) != plan["preparation_identity"] or set(os.listdir(control)) != {
                "lease",
                "witness",
                "result",
            }:
                raise ValueError("outer control directory custody changed")
            os.fchown(control, 0, 0)
            os.fchmod(control, 0o700)
            for leaf in ("lease", "witness", "result"):
                value = os.stat(leaf, dir_fd=control, follow_symlinks=False)
                if [value.st_dev, value.st_ino] != plan[leaf + "_identity"]:
                    raise ValueError("control inode substitution refuses unlink")
                os.unlink(leaf, dir_fd=control)
            os.fchown(control, uid, gid)
            os.fchmod(control, 0o700)
        finally:
            os.close(control)
        os.fchown(scope, uid, gid)
        os.fchmod(scope, 0o700)
    finally:
        os.close(scope)
        os.close(parent)


_ROOT_IMPORTS = (
    "from __future__ import annotations\n"
    "import hashlib, json, os, select, signal, stat, subprocess, sys, time\n"
    "from pathlib import Path\nfrom typing import Any\n"
)
_COMMON = (
    _r_identity,
    _r_directory,
    _r_start,
    _r_status,
    _r_auth,
    _r_payload_hash,
    _r_only_child,
    _r_environment,
    _r_control,
    _r_lease,
    _r_event,
    _r_drop,
)
_INIT_BODY = "\n".join(inspect.getsource(function) for function in (*_COMMON, _r_init))
INIT_PROGRAM = (
    _ROOT_IMPORTS
    + "WORKER="
    + repr(WORKER_PROGRAM)
    + "\n"
    + _INIT_BODY
    + "\n_r_init(json.loads(sys.argv[1]), *(int(value) for value in sys.argv[2:]))\n"
)
PLAN_KEYS = {
    "schema",
    "nonce",
    "caller_pid",
    "caller_start",
    "uid",
    "gid",
    "groups",
    "scope",
    "scope_identity",
    "preparation",
    "preparation_identity",
    "lease_identity",
    "witness_identity",
    "result",
    "result_identity",
    "module_root",
    "qualification",
    "environment",
    "host_pid_namespace",
    "host_mount_namespace",
}
ROOT_PROGRAM = (
    _ROOT_IMPORTS
    + "INIT="
    + repr(INIT_PROGRAM)
    + "\nPLAN_KEYS="
    + repr(sorted(PLAN_KEYS))
    + "\nOWNED_CHILDREN="
    + repr(OWNED_CHILDREN)
    + "\n"
    + "\n".join(
        inspect.getsource(function)
        for function in (*_COMMON, _r_bootstrap, _r_mount, _r_remove, _r_finalize)
    )
    + "\nassert len(sys.argv[2]) <= 16384\n"
    + "plan=json.loads(sys.argv[2])\n"
    + 'assert sys.argv[1] in {"qualify", "finalize"}\n'
    + 'sys.exit((_r_bootstrap if sys.argv[1] == "qualify" else _r_finalize)(plan))\n'
)
INIT = INIT_PROGRAM
BOOTSTRAP_SHA256 = hashlib.sha256(ROOT_PROGRAM.encode()).hexdigest()
INIT_SHA256 = hashlib.sha256(INIT_PROGRAM.encode()).hexdigest()
MODULE_SHA256 = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()


def validate_worker_context(context: Mapping[str, object]) -> Path:
    """Bind the root parent and handed namespace strings without privileged proc links."""
    if set(context) != {
        "private",
        "nonce",
        "pid_namespace",
        "mount_namespace",
        "host_pid_namespace",
        "host_mount_namespace",
        "pid1_pid_namespace",
        "pid1_mount_namespace",
        "abort_fd",
        "result",
        "result_identity",
    }:
        raise ContractError("unknown worker lifetime context")
    if os.getppid() != 1 or _r_status(1)["Uid"].split() != ["0"] * 4:
        raise ContractError("local rootful qualifier must be the dropped worker of root PID1")
    with Path("/proc/1/cmdline").open("rb") as stream:
        command = stream.read(65537)
    parts = command.split(b"\0")
    if (
        len(command) > 65536
        or len(parts) < 6
        or parts[:4] != [b"/usr/bin/python3", b"-I", b"-S", b"-c"]
        or hashlib.sha256(parts[4]).hexdigest() != INIT_SHA256
    ):
        raise ContractError("root PID1 is not the exact fixed init payload")
    parent = json.loads(parts[5])
    if (
        parent.get("uid") != os.getuid()
        or parent.get("scope") != context["private"]
        or parent.get("nonce") != context["nonce"]
        or any(
            parent.get(key) != context.get(key)
            for key in ("host_pid_namespace", "host_mount_namespace")
        )
    ):
        raise ContractError("worker context differs from protected PID1 plan")
    for key, leaf in (("pid_namespace", "pid"), ("mount_namespace", "mnt")):
        observed = os.readlink("/proc/self/ns/" + leaf)
        if (
            context.get(key) != observed
            or context.get("host_" + key) == observed
            or context.get("pid1_" + key) != observed
        ):
            raise ContractError("qualifier is outside the exact private PID/mount lifetime")
    private = Path(str(context["private"]))
    if (
        private.is_symlink()
        or private.stat().st_uid != os.getuid()
        or stat.S_IMODE(private.stat().st_mode) != 0o700
        or (private / ".lifetime-nonce").read_text() != context["nonce"]
    ):
        raise ContractError("worker private scope custody mismatch")
    return private


def notify_abort(
    context: dict[str, Any], failure: AbortContext, report: Mapping[str, object]
) -> None:
    """Persist the first bounded failure before A; opaque errors never render native bytes."""
    from .runner import OpaqueAbortContext, OpaqueReason, OpaqueRunError, opaque_errno

    if context.get("abort_notified"):
        return
    context["abort_notified"] = True
    opaque = isinstance(failure, OpaqueAbortContext)
    preview = {
        **report,
        "status": "failure",
        "cleanup": "pending-outer-closure",
        "cause": failure.stage + ": " + failure.cause,
        "command_failure_stage": failure.stage,
        "command_failure_status": failure.status,
        "command_failure_cause": failure.cause,
        "command_failure": {
            "stage": failure.stage,
            "status": failure.status,
            "cause": failure.cause,
        },
    }
    if opaque:
        preview.update(failure.safe_fields())
    from .devcontainer_validation import serialize_report

    pending_failure = False
    try:
        fd = os.open(str(context["result"]), os.O_WRONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
        try:
            facts = os.fstat(fd)
            if (
                [facts.st_dev, facts.st_ino] != context["result_identity"]
                or facts.st_uid != os.getuid()
                or stat.S_IMODE(facts.st_mode) != 0o600
            ):
                raise ContractError("abort handoff custody changed")
            encoded = serialize_report(preview).encode() + b"\n"
            os.ftruncate(fd, 0)
            if os.write(fd, encoded) != len(encoded):
                raise ContractError("incomplete primary failure handoff")
        finally:
            os.close(fd)
    except BaseException as error:
        if not opaque and not isinstance(error, (OSError, ValueError)):
            raise
        secondary = (
            OpaqueReason.ABORT_HANDOFF_FAILED.value
            if opaque
            else redact(str(error), protected_values(os.environ))[:512]
        )
        if opaque:
            pending_failure = True
            code = opaque_errno(error)
            if code is not None:
                secondary += " errno=" + str(code)
        fallback = {
            "stage": failure.stage,
            "status": failure.status,
            "cause": failure.cause,
            "abort_hook_failure": secondary,
        }
        if opaque:
            try:
                line = b"BOXFERRY_LIFETIME_FAILURE " + json.dumps(fallback).encode() + b"\n"
                if len(line) <= 4096:
                    os.write(2, line)
            except BaseException:
                pending_failure = True
        else:
            line = b"BOXFERRY_LIFETIME_FAILURE " + json.dumps(fallback).encode() + b"\n"
            if len(line) <= 4096:
                os.write(2, line)
            raise
    finally:
        # The existing one-purpose byte/order is unchanged, including handoff failures.
        if opaque:
            try:
                if os.write(int(context["abort_fd"]), b"A") != 1:
                    pending_failure = True
            except BaseException:
                pending_failure = True
        elif os.write(int(context["abort_fd"]), b"A") != 1:
            raise ContractError("inner abort notification failed")
    if pending_failure:
        # Outside the original handlers: no original exception/attrs/context is copied.
        raise OpaqueRunError(OpaqueReason.ABORT_HANDOFF_FAILED) from None


def _witness(fd: int, nonce: str) -> list[dict[str, Any]]:
    content = os.pread(fd, MAX_WITNESS + 1, 0)
    if len(content) > MAX_WITNESS:
        raise ContractError("root witness exceeds bound")
    if content and not content.endswith(b"\n"):
        content = content.rsplit(b"\n", 1)[0] if b"\n" in content else b""
    if content:
        facts = os.fstat(fd)
        if facts.st_uid != 0 or stat.S_IMODE(facts.st_mode) != 0o400 or facts.st_nlink != 1:
            raise ContractError("root lifetime witness was not claimed read-only")
    records = [json.loads(line) for line in content.splitlines()]
    if any(not isinstance(record, dict) or record.get("nonce") != nonce for record in records):
        raise ContractError("root witness nonce mismatch")
    return records


def _persist_outer_abort(path: Path, record: Mapping[str, object]) -> str:
    """Write once outside root-claimed directories; never share the inner result inode."""
    encoded = json.dumps(record, sort_keys=True).encode() + b"\n"
    if len(encoded) > 4096:
        raise ContractError("outer abort receipt exceeds bound")
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    try:
        facts = os.fstat(fd)
        if (
            not stat.S_ISREG(facts.st_mode)
            or facts.st_uid != os.getuid()
            or stat.S_IMODE(facts.st_mode) != 0o600
            or facts.st_nlink != 1
        ):
            raise ContractError("outer abort receipt custody changed")
        if os.write(fd, encoded) != len(encoded):
            raise ContractError("incomplete outer abort receipt")
        os.fsync(fd)
        current = path.lstat()
        if (
            (current.st_dev, current.st_ino) != (facts.st_dev, facts.st_ino)
            or current.st_size != len(encoded)
            or current.st_uid != os.getuid()
            or stat.S_IMODE(current.st_mode) != 0o600
            or current.st_nlink != 1
        ):
            raise ContractError("outer abort receipt changed during persistence")
    finally:
        os.close(fd)
    return hashlib.sha256(encoded).hexdigest()


class _OuterAbortEvidence:
    """Capture the first outer cause before EOF; inner and cleanup evidence stay separate."""

    def __init__(self, path: Path, nonce: str, close: Callable[[], None]) -> None:
        self.path, self.nonce, self.close = path, nonce, close
        self.first: dict[str, object] | None = None
        self.persisted = False
        self.persistence_failure = ""
        self.close_failure = ""
        self.sha256 = ""
        self.hidden = protected_values(os.environ)

    def abort(self, failure: AbortContext, signal_number: int | None = None) -> None:
        try:
            if self.first is None:

                def bounded(value: str, maximum: int) -> str:
                    return redact(value, self.hidden).encode()[:maximum].decode("utf-8", "ignore")

                self.first = {
                    "schema": 1,
                    "lifetime_nonce": self.nonce,
                    "caller_pid": os.getpid(),
                    "uid": os.getuid(),
                    "stage": bounded(failure.stage, 256),
                    "status": bounded(failure.status, 32),
                    "cause": bounded(failure.cause, 2048),
                }
                if signal_number is not None:
                    self.first.update(
                        signal_number=signal_number, signal_name=signal.Signals(signal_number).name
                    )
                # Any interrupted or partial attempt remains a distinct failure, never a retry.
                self.persistence_failure = "outer abort receipt persistence did not complete"
                self.sha256 = _persist_outer_abort(self.path, self.first)
                self.persisted = True
                self.persistence_failure = ""
        except (OSError, ValueError) as error:
            self.persistence_failure = redact(str(error), self.hidden)[:512]
        finally:
            try:
                self.close()
            except OSError as error:
                self.close_failure = redact(str(error), self.hidden)[:512]
        if self.persistence_failure or self.close_failure:
            raise ContractError(self.persistence_failure or self.close_failure)

    def fields(self) -> dict[str, object]:
        if self.first is None:
            return {}
        fields = {
            "outer_abort_stage": self.first["stage"],
            "outer_abort_status": self.first["status"],
            "outer_abort_cause": self.first["cause"],
            "outer_abort_persisted": self.persisted,
            "outer_abort_receipt": str(self.path),
        }
        for name in ("signal_number", "signal_name"):
            if name in self.first:
                fields["outer_abort_" + name] = self.first[name]
        fields["outer_abort_receipt_disposition"] = (
            "retained-user-owned-outside-root-custody"
            if self.persisted
            else "retain-if-created; persistence/custody unproved"
        )
        if self.persisted:
            fields["outer_abort_receipt_owner_uid"] = self.first["uid"]
            fields["outer_abort_receipt_mode"] = 0o600
        if self.sha256:
            fields["outer_abort_receipt_sha256"] = self.sha256
        if self.persistence_failure:
            fields["outer_abort_persistence_failure"] = self.persistence_failure
        if self.close_failure:
            fields["outer_abort_close_failure"] = self.close_failure
        return fields


def _lease_writer(directory: int) -> int:
    """Establish one user writer without retaining a dummy reader across launch."""
    expected = os.stat("lease", dir_fd=directory, follow_symlinks=False)
    if (
        not stat.S_ISFIFO(expected.st_mode)
        or expected.st_uid != os.getuid()
        or stat.S_IMODE(expected.st_mode) != 0o600
        or expected.st_nlink != 1
    ):
        raise ContractError("lease setup custody changed")
    writer = -1
    try:
        with ExitStack() as temporary:
            reader = os.open(
                "lease",
                os.O_RDONLY | os.O_NONBLOCK | os.O_CLOEXEC | os.O_NOFOLLOW,
                dir_fd=directory,
            )
            temporary.callback(os.close, reader)
            writer = os.open(
                "lease",
                os.O_WRONLY | os.O_NONBLOCK | os.O_CLOEXEC | os.O_NOFOLLOW,
                dir_fd=directory,
            )
            for descriptor in (reader, writer):
                facts = os.fstat(descriptor)
                if (
                    (facts.st_dev, facts.st_ino) != (expected.st_dev, expected.st_ino)
                    or not stat.S_ISFIFO(facts.st_mode)
                    or facts.st_uid != os.getuid()
                    or stat.S_IMODE(facts.st_mode) != 0o600
                    or facts.st_nlink != 1
                ):
                    raise ContractError("lease setup inode custody changed")
    except BaseException:
        if writer >= 0:
            os.close(writer)
        raise
    return writer


def execute(kwargs: dict[str, Any]) -> dict[str, object]:
    """User preparation precedes one fixed root envelope; no parent path deletion."""
    from . import devcontainer_validation as validation

    validation.effective_outer_mode("local", "podman", "rootful")
    module_root = Path(kwargs["automation"]) / "automation/src"
    selected = module_root / "boxferry_ci/devcontainer_lifetime.py"
    if (
        hashlib.sha256(selected.read_bytes()).hexdigest() != MODULE_SHA256
        or hashlib.sha256(Path(__file__).read_bytes()).hexdigest() != MODULE_SHA256
    ):
        raise ContractError("lifetime payload differs from selected immutable automation")
    serial: dict[str, Any] = {
        key: str(value) if isinstance(value, Path) else value for key, value in kwargs.items()
    }
    if kwargs.get("local_sources") is not None:
        serial["local_sources"] = {
            key: str(value) for key, value in kwargs["local_sources"].items()
        }
    work = Path(kwargs["work"])
    preparation = Path(tempfile.mkdtemp(prefix="bf-lifetime-", dir=work))
    preparation.chmod(0o700)
    scope = Path(tempfile.mkdtemp(prefix="bfv-life-", dir="/tmp"))
    scope.chmod(0o700)
    nonce = uuid.uuid4().hex
    report: dict[str, object] = {
        "schema": 1,
        "status": "failure",
        "cleanup": "pending",
        "run_nonce": nonce,
        "lifetime_nonce": nonce,
        "retained_private_directory": str(scope),
        "retained_lifetime_preparation": str(preparation),
        "lifetime_bootstrap_sha256": BOOTSTRAP_SHA256,
        "lifetime_init_sha256": INIT_SHA256,
        "lifetime_module_sha256": MODULE_SHA256,
    }
    with ExitStack() as held:
        scope_fd = control_fd = None
        try:
            scope_fd = _r_directory(str(scope))
            held.callback(os.close, scope_fd)
            control_fd = _r_directory(str(preparation))
            held.callback(os.close, control_fd)
            marker = scope / ".lifetime-nonce"
            marker.touch(mode=0o600)
            marker.write_text(nonce)
            os.mkfifo(preparation / "lease", 0o600)
            writer = _lease_writer(control_fd)

            def close_lease() -> None:
                nonlocal writer
                if writer >= 0:
                    descriptor = writer
                    writer = -1  # Never retry an uncertain close against a reused descriptor.
                    os.close(descriptor)

            held.callback(close_lease)
            witness_path, result_path = preparation / "witness", preparation / "result"
            for path in (witness_path, result_path):
                path.touch(mode=0o600)
            witness = os.open(witness_path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
            held.callback(os.close, witness)
            result_fd = os.open(result_path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
            held.callback(os.close, result_fd)
            plan = {
                "schema": 1,
                "nonce": nonce,
                "caller_pid": os.getpid(),
                "caller_start": _r_start(os.getpid()),
                "uid": os.getuid(),
                "gid": os.getgid(),
                "groups": [],
                "scope": str(scope),
                "scope_identity": _r_identity(scope_fd),
                "preparation": str(preparation),
                "preparation_identity": _r_identity(control_fd),
                "lease_identity": [os.fstat(writer).st_dev, os.fstat(writer).st_ino],
                "witness_identity": [os.fstat(witness).st_dev, os.fstat(witness).st_ino],
                "result": str(result_path),
                "result_identity": [os.fstat(result_fd).st_dev, os.fstat(result_fd).st_ino],
                "module_root": str(module_root),
                "qualification": serial,
                "environment": validation.environment(scope),
                "host_pid_namespace": os.readlink("/proc/self/ns/pid"),
                "host_mount_namespace": os.readlink("/proc/self/ns/mnt"),
            }
            encoded = json.dumps(plan, sort_keys=True)
        except (OSError, ValueError) as error:
            report.update(
                status="failure",
                cleanup="failure",
                namespace_closed=False,
                cause=redact(str(error), protected_values(os.environ))[:2048],
                cleanup_cause="lifetime setup failed before root launch; retain custody",
            )
            for name, descriptor in (("scope", scope_fd), ("control", control_fd)):
                if descriptor is not None:
                    facts = os.fstat(descriptor)
                    report[f"retained_{name}_owner_uid"] = facts.st_uid
                    report[f"retained_{name}_mode"] = stat.S_IMODE(facts.st_mode)
            return report
        previous = {number: signal.getsignal(number) for number in (signal.SIGTERM, signal.SIGINT)}
        # This sibling is user-only and survives any root claim/finalization of preparation.
        outer_abort = _OuterAbortEvidence(
            preparation.with_name(preparation.name + "-outer-abort.json"), nonce, close_lease
        )

        def cancel(number: int, _frame: object) -> None:
            cause = f"rootful lifetime signal {signal.Signals(number).name} ({number})"
            try:
                outer_abort.abort(
                    AbortContext("local-rootful-lifetime", "cancelled", cause), number
                )
            except ContractError:
                pass  # Distinct persistence/close evidence is retained; preserve cancellation.
            raise RunCancelled(cause)

        try:
            if len(encoded.encode()) > 16384:
                raise ContractError("lifetime plan exceeds bound")
            for number in previous:
                signal.signal(number, cancel)
            outcome = run(
                (
                    "/usr/bin/sudo",
                    "-n",
                    "/usr/bin/timeout",
                    "--signal=TERM",
                    "--kill-after=2s",
                    str(LIFETIME_SECONDS + 4) + "s",
                    "/usr/bin/python3",
                    "-I",
                    "-S",
                    "-c",
                    ROOT_PROGRAM,
                    "qualify",
                    encoded,
                ),
                cwd=preparation,
                stage="local-rootful-lifetime",
                timeout_seconds=LIFETIME_SECONDS + 8,
                environment={"PATH": "/usr/bin:/bin", "LC_ALL": "C.UTF-8"},
                secrets=protected_values(os.environ),
                on_abort=outer_abort.abort,
            )
            content = os.pread(result_fd, validation.MAX_REPORT + 1, 0)
            if len(content) > validation.MAX_REPORT:
                raise ContractError("worker report exceeds bound")
            if content:
                report.update(json.loads(content))
            fallback = [
                line.removeprefix("BOXFERRY_LIFETIME_FAILURE ")
                for line in outcome.stderr.splitlines()
                if line.startswith("BOXFERRY_LIFETIME_FAILURE ") and len(line) <= 4096
            ]
            if len(fallback) == 1:
                original = json.loads(fallback[0])
                if set(original) == {"stage", "status", "cause", "abort_hook_failure"}:
                    report.update(
                        status="failure",
                        cause=original["stage"] + ": " + original["cause"],
                        command_failure_stage=original["stage"],
                        command_failure_status=original["status"],
                        command_failure_cause=original["cause"],
                        abort_hook_failure=original["abort_hook_failure"],
                    )
            if outcome.status != "success":
                report["lifetime_failure_evidence"] = validation.failure_evidence(outcome.stderr)
                report.update(
                    status="failure",
                    cause=report.get("cause")
                    or outer_abort.fields().get("outer_abort_cause")
                    or outcome.diagnostic.cause,
                )
            if outcome.abort_hook_failure:
                report["abort_hook_failure"] = outcome.abort_hook_failure
        except (OSError, ValueError, RunCancelled, KeyboardInterrupt) as error:
            report.update(
                status="failure", cause=redact(str(error), protected_values(os.environ))[:2048]
            )
            if secondary := getattr(error, "abort_hook_failure", ""):
                report["abort_hook_failure"] = secondary
        finally:
            try:
                close_lease()
            except OSError as error:
                report.update(
                    status="failure",
                    abort_hook_failure=redact(str(error), protected_values(os.environ))[:512],
                )
            for number, handler in previous.items():
                signal.signal(number, handler)
            try:
                deadline = time.monotonic() + CLOSURE_SECONDS
                records: list[dict[str, Any]] = []
                while time.monotonic() < deadline:
                    records = _witness(witness, nonce)
                    if any(
                        record.get("event") in {"closed", "uncertain", "abort-before-start"}
                        for record in records
                    ):
                        break
                    time.sleep(0.02)
                closure = [record for record in records if record.get("event") == "closed"]
                report["namespace_closed"] = (
                    len(closure) == 1
                    and closure[0].get("namespace_closed") is True
                    and closure[0].get("supervisor_closed") is True
                )
                if not report["namespace_closed"]:
                    raise ContractError("exact namespace closure is unproved; retain custody")
                report["engine_resource_cleanup"] = report.get("cleanup")
                cleanup = run(
                    (
                        "/usr/bin/sudo",
                        "-n",
                        "/usr/bin/timeout",
                        "--signal=TERM",
                        "--kill-after=2s",
                        "60s",
                        "/usr/bin/python3",
                        "-I",
                        "-S",
                        "-c",
                        ROOT_PROGRAM,
                        "finalize",
                        encoded,
                    ),
                    cwd=preparation,
                    stage="finalize-local-rootful-directories",
                    timeout_seconds=65,
                    environment={"PATH": "/usr/bin:/bin", "LC_ALL": "C.UTF-8"},
                    secrets=protected_values(os.environ),
                )
                if cleanup.status != "success":
                    report["cleanup_failure_evidence"] = validation.failure_evidence(cleanup.stderr)
                if (
                    cleanup.status != "success"
                    or _r_identity(scope_fd) != plan["scope_identity"]
                    or os.listdir(scope_fd)
                    or _r_identity(control_fd) != plan["preparation_identity"]
                    or os.listdir(control_fd)
                ):
                    raise ContractError(
                        cleanup.diagnostic.cause + "; outer custody/finalization incomplete"
                    )
                report.update(
                    cleanup="success",
                    directory_cleanup="success",
                    empty_private_directory=str(scope),
                    empty_lifetime_control_directory=str(preparation),
                )
                report.pop("retained_private_directory", None)
                report.pop("retained_lifetime_preparation", None)
            except (OSError, ValueError, RunCancelled, KeyboardInterrupt) as secondary:
                report.update(
                    status="failure",
                    cleanup="failure",
                    cleanup_cause=redact(str(secondary), protected_values(os.environ))[:2048],
                )
            report["retained_scope_owner_uid"] = os.fstat(scope_fd).st_uid
            report["retained_scope_mode"] = stat.S_IMODE(os.fstat(scope_fd).st_mode)
            report["retained_control_owner_uid"] = os.fstat(control_fd).st_uid
            report["retained_control_mode"] = stat.S_IMODE(os.fstat(control_fd).st_mode)
            # Worker data cannot mint or replace the original caller's outer evidence.
            for name in tuple(report):
                if isinstance(name, str) and name.startswith("outer_abort_"):
                    report.pop(name)
            report.update(outer_abort.fields())
            if outer_abort.first is not None and (
                not outer_abort.persisted or outer_abort.close_failure
            ):
                report["status"] = "failure"
    return report
