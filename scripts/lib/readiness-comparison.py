#!/usr/bin/env python3
"""Private startup observations, never Docker capability or application evidence."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import pathlib
import re
import signal
import stat
import sys
import time
from contextlib import ExitStack, contextmanager
from fractions import Fraction


HERE = pathlib.Path(__file__).parent
SPEC = importlib.util.spec_from_file_location("comparison_contract", HERE / "docker-application-contract.py")
assert SPEC is not None and SPEC.loader is not None
contract = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(contract)
LAUNCH_SPEC = importlib.util.spec_from_file_location("owned_native_launch", HERE / "owned-native-launch.py")
assert LAUNCH_SPEC is not None and LAUNCH_SPEC.loader is not None
owned_native_launch = importlib.util.module_from_spec(LAUNCH_SPEC)
LAUNCH_SPEC.loader.exec_module(owned_native_launch)
REPORT = "readiness-comparison.json"
INSPECT = ('{"id":{{json .ID}},"name":{{json .Name}},"owner":'
           '{{json (index .Config.Labels "io.boxferry.docker-core-run")}},'
           '"running":{{json .State.Running}},"pid":{{json .State.Pid}},'
           '"privileged":{{json .HostConfig.Privileged}}}')
NATIVE_COMMAND = ('exec curl -q --noproxy \'*\' -fs --max-time 5 --unix-socket "$1" '
                  'http://localhost/_ping >/dev/null 2>/dev/null')
PHASES = {"host-prerequisite", "native-binding", "bridge-prerequisite", "outer-admission",
          "endpoint-binding", "permission-transition", "baseline", "harmonized", "native-before",
          "native-after", "native-later", "clock", "cleanup", "record"}
LOG_CATEGORIES = {"empty", "content-present", "permission-error-observed", "storage-error-observed",
                  "network-error-observed", "socket-error-observed", "startup-error-observed",
                  "multiple-errors-observed"}
LOG_WORK_SECONDS = 7


class Refused(Exception):
    """A value-free refusal; exception/native text is never recorded."""


def need(value: bool) -> None:
    if not value:
        raise Refused


def origin_ms(text: str) -> int:
    need(isinstance(text, str) and re.fullmatch(r"[0-9]{1,12}\.[0-9]{2}", text) is not None)
    seconds, centiseconds = text.split(".")
    return int(seconds) * 1000 + int(centiseconds) * 10


def number(error: BaseException) -> int | None:
    value = getattr(error, "errno", None)
    return value if type(value) is int and 1 <= value <= 4095 else None


def private_directory(path: pathlib.Path) -> list[int]:
    need(path.is_absolute() and path.resolve(strict=True) == path)
    s = path.lstat()
    need(stat.S_ISDIR(s.st_mode) and s.st_uid == os.geteuid() and stat.S_IMODE(s.st_mode) == 0o700)
    return [s.st_dev, s.st_ino, s.st_uid]


def read_report(directory: pathlib.Path) -> dict:
    identity = private_directory(directory)
    fd = os.open(directory / REPORT, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(fd, "rb") as source:
        s = os.fstat(source.fileno())
        need(stat.S_ISREG(s.st_mode) and s.st_nlink == 1 and s.st_uid == os.geteuid()
             and stat.S_IMODE(s.st_mode) == 0o600 and s.st_size <= 65536)
        value = json.loads(source.read(65537), object_pairs_hook=contract.no_duplicate_keys)
    need(value["directory"] == identity and value["qualification"] == "none")
    validate_report(value)
    return value


def write_report(directory: pathlib.Path, value: dict, *, create: bool = False) -> None:
    validate_report(value)
    need(private_directory(directory) == value["directory"])
    raw = (json.dumps(value, sort_keys=True) + "\n").encode()
    need(len(raw) <= 65536)
    flags = os.O_WRONLY | os.O_NOFOLLOW | (os.O_CREAT | os.O_EXCL if create else 0)
    fd = os.open(directory / REPORT, flags, 0o600)
    with os.fdopen(fd, "wb") as output:
        s = os.fstat(output.fileno())
        need(stat.S_ISREG(s.st_mode) and s.st_nlink == 1 and s.st_uid == os.geteuid()
             and stat.S_IMODE(s.st_mode) == 0o600)
        os.ftruncate(output.fileno(), 0)
        output.write(raw)
        output.flush()
        os.fsync(output.fileno())


def digest(path: pathlib.Path) -> str:
    return hashlib.sha256(contract.bounded_regular_bytes(path, 512 * 1024)).hexdigest()


def source_binding(args) -> dict:
    pins = contract.canonical_images(str(args.native_root), args.native_revision, args.native_script_sha256)
    source = contract.bounded_regular_bytes(args.native_root / "scripts/native-conformance.sh", 512 * 1024)
    literal = b'curl -q --noproxy \'*\' -fs --max-time 5 --unix-socket "$socket" http://localhost/_ping >/dev/null'
    # The bound native runner has a loop poll AND a final confirmation. Admit
    # those two literal call sites, not an arbitrary count or a comment match.
    lines = source.splitlines()
    loop_call = b"    if " + literal + b"; then break; fi"
    final_call = b"[[ -S $socket ]] && " + literal + b" || {"
    deadline = b"deadline=$((SECONDS + 360))"
    loop = b"while (( SECONDS < deadline )); do"
    need(source.count(literal) == 2 and all(lines.count(line) == 1 for line in (loop_call, final_call, deadline, loop)))
    begin, first, final = lines.index(loop), lines.index(loop_call), lines.index(final_call)
    need(begin == lines.index(deadline) + 1 and begin < first < final
         and lines[first + 1:final].count(b"done") == 1 and lines[final - 1] == b"done")
    return {"native_revision": args.native_revision,
            "boxferry_revision": contract.git(HERE.parent.parent, "rev-parse", "HEAD"),
            "native_script_sha256": args.native_script_sha256,
            "image_sha256": pins["debian11-rootful"].split("@sha256:")[1],
            "bridge_sha256": digest(args.native_root / "scripts/native-bridge-prerequisite.py"),
            "harness_sha256": digest(HERE.parent / "docker-application-conformance.sh"),
            "contract_sha256": digest(HERE / "docker-application-contract.py"),
            "observer_sha256": digest(pathlib.Path(__file__)),
            "collector_sha256": digest(HERE / "bounded-native-read.py"),
            "launcher_sha256": digest(HERE / "owned-native-launch.py")}


def host_prerequisite(expected_namespace: str) -> list[int]:
    need(isinstance(expected_namespace, str))
    need(re.fullmatch(r"[1-9][0-9]{0,19}:[1-9][0-9]{0,19}", expected_namespace) is not None)
    expected = [int(value) for value in expected_namespace.split(":")]
    s = os.stat("/proc/self/ns/pid")
    need(os.geteuid() == os.getuid() == 0 and [s.st_dev, s.st_ino] == expected)
    mapping = pathlib.Path("/proc/self/uid_map").read_text().split()
    need(mapping == ["0", "0", "4294967295"])
    status = pathlib.Path("/proc/self/status").read_text()
    caps = next(line.split()[1] for line in status.splitlines() if line.startswith("CapEff:"))
    need(re.fullmatch(r"[0-9a-fA-F]{1,16}", caps) is not None)
    need(int(caps, 16) & (1 << 12) and int(caps, 16) & (1 << 21))
    outcome, raw = contract.readiness_read(["podman", "info", "--format", "{{.Host.Security.Rootless}}"],
                                         time.monotonic() + 4)
    need(outcome == "read" and raw.strip() == b"false")
    return expected


def bridge_prerequisite(root: pathlib.Path) -> None:
    # The literal native implementation receives NO hosted environment: its
    # local policy cannot load modules even when the caller is a hosted job.
    spec = importlib.util.spec_from_file_location("comparison_bridge", root / "scripts/native-bridge-prerequisite.py")
    need(spec is not None and spec.loader is not None)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    need(module.ensure_bridge_prerequisite({}) ==
         "DOCKERLENS_NATIVE_HOST_NETWORK: bridge_filter=ready module_load=not-needed")


def initialize(args) -> dict:
    identity = private_directory(args.directory)
    need(not any(args.directory.iterdir()))
    need(args.native_root is not None and args.directory != pathlib.Path("/tmp"))
    for source in (args.native_root.resolve(strict=True), HERE.parent.parent.resolve(strict=True)):
        need(args.directory != source and source not in args.directory.parents)
    return {"schema_version": 2, "qualification": "none", "status": "pending",
            "directory": identity, "phase": "host-prerequisite", "errno": None,
            "prerequisite": "unknown", "namespace": None, "sources": {}, "outer": None, "socket": None,
            "origin": None, "consumer_budget_ms": 180000, "native_budget_ms": 360000,
            "consumer": {"baseline": None, "first_failure": None, "first_harmonized_failure": None,
                         "last": None, "first_ready_ms": None, "polls": 0, "budget_closed": False},
            "native": {"before": None, "after": None, "last": None, "first_ready_ms": None, "polls": 0},
            "differences": {"network": "owned-default-not-native-custom-network",
                            "sidecar": "not-started-native-egress-sidecar-omitted",
                            "socket": "baseline-original-then-native-0666-transition",
                            "supervision": "native-literal-curl-under-shared-bounded-observer",
                            "scheduling": "serial-observers-consume-startup-budgets"},
            "uncertain": False, "cleanup": "not-created", "classification": "indeterminate",
            "startup_logs": {"status": "not-run", "category": None}}


def validate_report(value: dict) -> None:
    """A closed private record cannot become a channel for arbitrary native text."""
    need(type(value) is dict and set(value) == {
        "schema_version", "qualification", "status", "directory", "phase", "errno", "prerequisite",
        "namespace", "sources", "outer", "socket", "origin", "consumer_budget_ms", "native_budget_ms",
        "consumer", "native", "differences", "uncertain", "cleanup", "classification", "startup_logs"})
    need(type(value["schema_version"]) is int and value["schema_version"] == 2 and value["qualification"] == "none"
         and value["status"] in {"pending", "completed", "withheld"} and value["phase"] in PHASES
         and value["prerequisite"] in {"unknown", "ready"} and type(value["uncertain"]) is bool
         and value["cleanup"] in {"not-created", "verified", "unknown"}
         and value["classification"] in {"indeterminate", "prerequisite-not-met", "both-routes-ready",
             "consumer-only-ready-observed", "early-native-only-ready-observed",
             "native-ready-after-consumer-budget", "shared-no-ready-observed"}
         and value["consumer_budget_ms"] == 180000 and value["native_budget_ms"] == 360000)
    logs = value["startup_logs"]
    need(type(logs) is dict and set(logs) == {"status", "category"}
         and logs["status"] in {"not-run", "observed", "withheld"})
    need(logs["category"] in LOG_CATEGORIES if logs["status"] == "observed" else logs["category"] is None)
    need(value["errno"] is None or type(value["errno"]) is int and 1 <= value["errno"] <= 4095)
    need(value["origin"] is None or type(value["origin"]) is int and value["origin"] >= 0)
    need(type(value["directory"]) is list and len(value["directory"]) == 3
         and all(type(x) is int and x >= 0 for x in value["directory"]))
    need(value["namespace"] is None or type(value["namespace"]) is list and len(value["namespace"]) == 2
         and all(type(x) is int and x > 0 for x in value["namespace"]))
    sources = value["sources"]
    need(sources == {} or set(sources) == {"native_revision", "boxferry_revision", "native_script_sha256", "image_sha256",
         "bridge_sha256", "harness_sha256", "contract_sha256", "observer_sha256", "collector_sha256", "launcher_sha256"})
    for key, item in sources.items():
        need(type(item) is str and re.fullmatch(r"[0-9a-f]{40}" if key.endswith("_revision") else r"[0-9a-f]{64}", item) is not None)
    for route, keys in (("consumer", {"baseline", "first_failure", "first_harmonized_failure", "last", "first_ready_ms", "polls", "budget_closed"}),
                        ("native", {"before", "after", "last", "first_ready_ms", "polls"})):
        need(set(value[route]) == keys)
        for key, item in value[route].items():
            if key == "budget_closed":
                need(type(item) is bool)
            elif key in {"first_ready_ms", "polls"}:
                need(item is None and key == "first_ready_ms" or type(item) is int and 0 <= item <= 3600000)
            elif item is not None:
                expected = {"exit", "http", "collector", "teardown", "phase", "elapsed_ms", "errno"}
                if route == "consumer":
                    expected |= {"error", "wrapper"}
                    need(item["error"] in contract.PING_ERRORS and type(item["wrapper"]) is int and 0 <= item["wrapper"] <= 255)
                need(set(item) == expected and item["collector"] in contract.PING_COLLECTORS
                     and item["teardown"] in contract.PING_TEARDOWN_SIGNALS
                     and item["phase"] in ({"baseline", "harmonized"} if route == "consumer" else {"native-before", "native-after", "native-later"})
                     and type(item["elapsed_ms"]) is int and 0 <= item["elapsed_ms"] <= 3600000
                     and item["errno"] is None)
                need(item["exit"] is None or type(item["exit"]) is int and 0 <= item["exit"] <= 255)
                need(item["http"] is None or route == "consumer" and type(item["http"]) is int and 0 <= item["http"] <= 599)
    need(value["differences"] == {"network": "owned-default-not-native-custom-network",
        "sidecar": "not-started-native-egress-sidecar-omitted", "socket": "baseline-original-then-native-0666-transition",
        "supervision": "native-literal-curl-under-shared-bounded-observer", "scheduling": "serial-observers-consume-startup-budgets"})
    outer = value["outer"]
    if outer is not None:
        need(set(outer) in ({"id", "pid", "start", "namespace", "layout", "container_leaf", "directory", "path_sha256"},
             {"id", "pid", "start", "namespace", "layout", "container_leaf", "directory", "path_sha256", "limits"}))
        need(re.fullmatch(r"[0-9a-f]{64}", outer["id"]) is not None
             and re.fullmatch(r"[0-9a-f]{64}", outer["path_sha256"]) is not None
             and outer["layout"] in {None, "systemd-scope", "cgroupfs-parent"} and type(outer["container_leaf"]) is bool)
        for key in ("pid", "start"):
            need(outer[key] is None or type(outer[key]) is int and outer[key] > 0)
        need(type(outer["directory"]) is list and len(outer["directory"]) == 3
             and all(type(x) is int and x >= 0 for x in outer["directory"]))
        need(outer["namespace"] is None or type(outer["namespace"]) is list and len(outer["namespace"]) == 2
             and all(type(x) is int and x > 0 for x in outer["namespace"]))
        if "limits" in outer:
            need(set(outer["limits"]) == {"memory_bytes", "cpu_quota", "cpu_period", "tasks"}
                 and all(type(x) is int and x > 0 for x in outer["limits"].values()))
    node = value["socket"]
    if node is not None:
        need(set(node) == {"device", "inode", "uid", "gid", "path_sha256", "original_mode", "native_mode"}
             and re.fullmatch(r"[0-9a-f]{64}", node["path_sha256"]) is not None)
        need(all(type(node[key]) is int and node[key] >= 0 for key in ("device", "inode", "uid", "gid", "original_mode"))
             and node["native_mode"] in {None, 0o666})


def inspect_outer(outer: str, run: str, *, cid: str | None = None, deadline: float | None = None, launcher=None) -> dict:
    need(re.fullmatch(r"[A-Za-z0-9_]{1,64}", run) is not None and outer == "bf-docker-core-" + run)
    need(cid is None or re.fullmatch(r"[0-9a-f]{64}", cid) is not None)
    if deadline is None:
        deadline = time.monotonic() + 4
    need(time.monotonic() < deadline)
    options = {} if launcher is None else {"launcher": launcher}
    outcome, raw = contract.readiness_read(["podman", "inspect", "--format", INSPECT, cid or outer], deadline, **options)
    need(outcome == "read")
    record = json.loads(raw, object_pairs_hook=contract.no_duplicate_keys)
    need(set(record) == {"id", "name", "owner", "running", "pid", "privileged"})
    need(re.fullmatch(r"[0-9a-f]{64}", record["id"]) is not None
         and record["name"] == outer and record["owner"] == run)
    return record


def node_identity(metadata) -> tuple:
    return (metadata.st_dev, metadata.st_ino, metadata.st_uid, metadata.st_gid, metadata.st_mode)


class LogBudget:
    def __init__(self):
        self.boot_deadline = time.clock_gettime(time.CLOCK_BOOTTIME) + LOG_WORK_SECONDS
        self.monotonic_deadline = time.monotonic() + LOG_WORK_SECONDS
        self.expired = False
        self.launcher = None

    def check(self):
        need(not self.expired and time.clock_gettime(time.CLOCK_BOOTTIME) < self.boot_deadline)

    def reader_deadline(self):
        self.check()
        # BOOTTIME advances through suspend; translate its remaining budget to
        # the canonical reader's MONOTONIC domain, never resetting either end.
        remaining = self.boot_deadline - time.clock_gettime(time.CLOCK_BOOTTIME)
        need(remaining > 0)
        return min(self.monotonic_deadline, time.monotonic() + remaining)


@contextmanager
def log_budget():
    """Interrupt blocking brackets as well as subprocess collection.

    An occupied/blocked alarm facility is refused without stealing caller state.
    KeyboardInterrupt enters the canonical reader's owned-process teardown; its
    existing <=1s handoff reserve remains separate from the collection alarm.
    """
    budget = LogBudget()
    previous_mask = signal.pthread_sigmask(signal.SIG_BLOCK, set())
    installed = False
    try:
        signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGALRM})
        previous_handler = signal.getsignal(signal.SIGALRM)
        previous_timer = signal.getitimer(signal.ITIMER_REAL)
        need(signal.SIGALRM not in previous_mask and previous_timer == (0.0, 0.0)
             and signal.SIGALRM not in signal.sigpending())
        def expired(_signum, _frame):
            budget.expired = True
            raise KeyboardInterrupt
        signal.signal(signal.SIGALRM, expired)
        installed = True
        budget.check()
        signal.setitimer(signal.ITIMER_REAL, budget.boot_deadline - time.clock_gettime(time.CLOCK_BOOTTIME))
        signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
        budget.launcher = owned_native_launch.OwnedLauncher(budget.boot_deadline, budget.boot_deadline + 1)
        yield budget
        budget.check()
    finally:
        # Defer cancellation only during finite teardown, without blocking
        # waits under a signal mask or losing a cancellation observation.
        signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGALRM, signal.SIGTERM, signal.SIGINT, signal.SIGHUP})
        cleanup_failed = False
        cancel_handlers = {}
        try:
            if installed:
                signal.setitimer(signal.ITIMER_REAL, 0)
                def cancelled(_signum, _frame):
                    budget.expired = True
                for signum in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
                    cancel_handlers[signum] = signal.signal(signum, cancelled)
                signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
                try:
                    if budget.launcher is not None:
                        budget.launcher.close_all()
                except (OSError, KeyboardInterrupt):
                    cleanup_failed = True
                finally:
                    signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGALRM, signal.SIGTERM, signal.SIGINT, signal.SIGHUP})
                if signal.SIGALRM in signal.sigpending():
                    signal.sigtimedwait({signal.SIGALRM}, 0)
                    budget.expired = True
                signal.signal(signal.SIGALRM, previous_handler)
        finally:
            for signum, handler in cancel_handlers.items():
                signal.signal(signum, handler)
            signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
        need(not cleanup_failed)
    budget.check()


def log_binding(value: dict, outer: str, run: str, socket: pathlib.Path, budget: LogBudget,
                held: dict) -> tuple:
    """Recheck only the admitted daemon and held task directories; never connect."""
    budget.check()
    binding = value["outer"]
    record = inspect_outer(outer, run, cid=binding["id"], deadline=budget.reader_deadline(), launcher=budget.launcher)
    need(record == {"id": binding["id"], "name": outer, "owner": run, "running": True,
                    "pid": binding["pid"], "privileged": True})
    identity = process(binding["pid"])
    need(identity == {key: binding[key] for key in ("pid", "start", "namespace")})
    host = os.stat("/proc/self/ns/pid")
    need([host.st_dev, host.st_ino] == value["namespace"])
    root = cgroup_path(binding)
    expected_cgroup = "0::/" + str(root.relative_to("/sys/fs/cgroup")) + "\n"
    need(pathlib.Path(f'/proc/{binding["pid"]}/cgroup').read_text() == expected_cgroup)
    need(binding["pid"] in {int(x) for x in (root / "cgroup.procs").read_text().split()})
    need(effective_limits(root) == binding["limits"])
    need(private_directory(socket.parent.parent) == binding["directory"])
    directories = []
    for path, descriptor in held.items():
        need(path.resolve(strict=True) == path)
        metadata = path.lstat()
        need(stat.S_ISDIR(metadata.st_mode)
             and node_identity(os.fstat(descriptor)) == node_identity(metadata))
        directories.append(node_identity(metadata))
    socket_directory = socket.parent.lstat()
    need(socket_directory.st_uid == 0)
    try:
        node = socket.lstat()
    except FileNotFoundError:
        need(value["socket"] is None)
        endpoint_identity = None
    else:
        need(stat.S_ISSOCK(node.st_mode) and node.st_uid == 0)
        endpoint_identity = node_identity(node)
        if value["socket"] is not None:
            previous = value["socket"]
            need([node.st_dev, node.st_ino, node.st_uid, node.st_gid] ==
                 [previous[key] for key in ("device", "inode", "uid", "gid")]
                 and stat.S_IMODE(node.st_mode) ==
                 (previous["original_mode"] if previous["native_mode"] is None else previous["native_mode"]))
    budget.check()
    return record, identity, tuple(directories), endpoint_identity


def startup_logs(value: dict, outer: str, run: str, socket: pathlib.Path) -> None:
    """Optional observation: seven seconds shared work plus one second handoff.

    Failure only withholds this category. It cannot change readiness samples,
    classification, qualification, uncertainty or cleanup authority.
    """
    with log_budget() as budget, ExitStack() as stack:
        need(value["startup_logs"]["status"] == "withheld")
        binding = value["outer"]
        need(binding is not None and "limits" in binding and binding["pid"] is not None
             and binding["layout"] is not None and value["prerequisite"] == "ready")
        need(isinstance(socket, pathlib.Path) and
             re.fullmatch(r"/tmp/boxferry-docker-core\.[A-Za-z0-9_]{1,64}/socket/docker\.sock", str(socket)) is not None
             and socket == pathlib.Path(f"/tmp/boxferry-docker-core.{run}/socket/docker.sock")
             and hashlib.sha256(str(socket).encode()).hexdigest() == binding["path_sha256"])
        held = {}
        for path in (socket.parent.parent, socket.parent, cgroup_path(binding)):
            with owned_native_launch.critical():
                descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
                try:
                    stack.callback(os.close, descriptor)
                except BaseException:
                    os.close(descriptor)
                    raise
                held[path] = descriptor
            budget.check()
        before = log_binding(value, outer, run, socket, budget, held)
        outcome, raw = contract.readiness_read(["podman", "logs", "--tail", "80", binding["id"]],
                                             budget.reader_deadline(), merge_output=True, launcher=budget.launcher)
        budget.check()
        need(outcome == "read" and type(raw) is bytes and len(raw) <= 16384)
        after = log_binding(value, outer, run, socket, budget, held)
        budget.check()
        need(after == before)
        category = contract.readiness_log_category(raw)
        need(category in LOG_CATEGORIES)
    value["startup_logs"] = {"status": "observed", "category": category}


def process(pid: int) -> dict:
    need(type(pid) is int and 0 < pid < 2**31)
    raw = pathlib.Path(f"/proc/{pid}/stat").read_text()
    need(len(raw) < 4096 and raw.startswith(f"{pid} ("))
    fields = raw[raw.rfind(")") + 2:].split()
    status = pathlib.Path(f"/proc/{pid}/status").read_text()
    rows = {line.split(":", 1)[0]: line.split(":", 1)[1].split() for line in status.splitlines() if ":" in line}
    need(rows["Uid"] == ["0"] * 4 and rows["Gid"] == ["0"] * 4)
    caps = int(rows["CapEff"][0], 16)
    need(caps & (1 << 12) and caps & (1 << 21) and int(rows["NSpid"][-1]) == 1)
    s = os.stat(f"/proc/{pid}/ns/pid")
    return {"pid": pid, "start": int(fields[19]), "namespace": [s.st_dev, s.st_ino]}


def cgroup_path(outer: dict) -> pathlib.Path:
    prefix = {"systemd-scope": "machine.slice/libpod-", "cgroupfs-parent": "libpod_parent/libpod-"}[outer["layout"]]
    suffix = ".scope" if outer["layout"] == "systemd-scope" else ""
    return pathlib.Path("/sys/fs/cgroup") / (prefix + outer["id"] + suffix + ("/container" if outer["container_leaf"] else ""))


def effective_limits(root: pathlib.Path) -> dict:
    """Read physical v2 limits, including a delegated leaf's capped ancestors."""
    memory_limits, cpu_limits, task_limits = [], [], []
    current = root
    while current != pathlib.Path("/sys/fs/cgroup"):
        need(current.resolve(strict=True) == current)
        memory = (current / "memory.max").read_text().strip()
        cpu = (current / "cpu.max").read_text().split()
        tasks = (current / "pids.max").read_text().strip()
        need(memory == "max" or memory.isdecimal() and int(memory) > 0)
        need(tasks == "max" or tasks.isdecimal() and int(tasks) > 0)
        need(len(cpu) == 2 and cpu[1].isdecimal() and int(cpu[1]) > 0
             and (cpu[0] == "max" or cpu[0].isdecimal() and int(cpu[0]) > 0))
        if memory != "max":
            memory_limits.append(int(memory))
        if tasks != "max":
            task_limits.append(int(tasks))
        if cpu[0] != "max":
            cpu_limits.append(Fraction(int(cpu[0]), int(cpu[1])))
        current = current.parent
    need(bool(memory_limits) and bool(cpu_limits) and bool(task_limits))
    cpu = min(cpu_limits)
    memory, tasks = min(memory_limits), min(task_limits)
    need(memory <= 4 * 1024**3 and cpu <= 2 and tasks <= 512)
    return {"memory_bytes": memory, "cpu_quota": cpu.numerator,
            "cpu_period": cpu.denominator, "tasks": tasks}


def admit_outer(value: dict, outer: str, run: str, socket: pathlib.Path, created: pathlib.Path, origin: str) -> None:
    need(re.fullmatch(r"[0-9]{1,12}\.[0-9]{2}", origin) is not None)
    need(re.fullmatch(r"/tmp/boxferry-docker-core\.[A-Za-z0-9_]{1,64}/socket/docker\.sock", str(socket)) is not None)
    private_directory(socket.parent.parent)
    raw = contract.bounded_regular_bytes(created, 65)
    need(re.fullmatch(rb"[0-9a-f]{64}\n", raw) is not None)
    # Keep exact creation identity even if subsequent effective-cap admission fails.
    value["origin"] = origin_ms(origin)
    value["outer"] = {"id": raw[:64].decode(), "pid": None, "start": None,
                      "namespace": None, "layout": None, "container_leaf": False,
                      "directory": private_directory(socket.parent.parent),
                      "path_sha256": hashlib.sha256(str(socket).encode()).hexdigest()}
    record = inspect_outer(outer, run)
    need(record["id"] == value["outer"]["id"] and record["running"] is True and record["privileged"] is True)
    identity = process(record["pid"])
    value["outer"].update(identity)
    need(identity["namespace"] != value["namespace"])
    cg = pathlib.Path(f'/proc/{identity["pid"]}/cgroup').read_text()
    for layout, prefix in (("systemd-scope", "machine.slice/libpod-"), ("cgroupfs-parent", "libpod_parent/libpod-")):
        base = "/" + prefix + record["id"] + (".scope" if layout == "systemd-scope" else "")
        if cg in {"0::" + base + "\n", "0::" + base + "/container\n"}:
            value["outer"]["layout"] = layout
            value["outer"]["container_leaf"] = cg.endswith("/container\n")
            break
    need(value["outer"]["layout"] is not None)
    root = cgroup_path(value["outer"])
    need(root.resolve(strict=True) == root and identity["pid"] in {int(x) for x in (root / "cgroup.procs").read_text().split()})
    value["outer"]["limits"] = effective_limits(root)
    need(inspect_outer(outer, run) == record and process(record["pid"]) == identity)


def endpoint(value: dict, socket: pathlib.Path, *, transition: bool = False) -> None:
    need(value["outer"] is not None and "limits" in value["outer"]
         and private_directory(socket.parent.parent) == value["outer"]["directory"]
         and hashlib.sha256(str(socket).encode()).hexdigest() == value["outer"]["path_sha256"])
    outer = value["outer"]
    need(process(outer["pid"]) == {key: outer[key] for key in ("pid", "start", "namespace")})
    host = os.stat("/proc/self/ns/pid")
    need([host.st_dev, host.st_ino] == value["namespace"])
    need(socket.parent.resolve(strict=True) == socket.parent and socket.name == "docker.sock")
    s = socket.lstat()
    need(stat.S_ISSOCK(s.st_mode) and s.st_uid == 0)
    identity = {"device": s.st_dev, "inode": s.st_ino, "uid": s.st_uid, "gid": s.st_gid,
                "path_sha256": hashlib.sha256(str(socket).encode()).hexdigest()}
    if value["socket"] is None:
        value["socket"] = dict(identity, original_mode=stat.S_IMODE(s.st_mode), native_mode=None)
    need(all(value["socket"][k] == v for k, v in identity.items()))
    if transition:
        need((value["consumer"]["baseline"] is not None or value["consumer"]["budget_closed"])
             and value["socket"]["native_mode"] is None)
        os.chmod(socket, 0o666, follow_symlinks=False)
        after = socket.lstat()
        need((after.st_dev, after.st_ino, after.st_uid, after.st_gid) == (s.st_dev, s.st_ino, s.st_uid, s.st_gid)
             and stat.S_IMODE(after.st_mode) == 0o666)
        value["socket"]["native_mode"] = 0o666


def elapsed(value: dict) -> int:
    need(type(value["origin"]) is int)
    result = time.clock_gettime_ns(time.CLOCK_BOOTTIME) // 1000000 - value["origin"]
    need(0 <= result <= 3600000)
    return result


def parse_box(record: str, wrapper_status: int) -> dict:
    fields = record.split()
    need(len(fields) == 5 and fields[2] in contract.PING_ERRORS and fields[3] in contract.PING_COLLECTORS
         and fields[4] in contract.PING_TEARDOWN_SIGNALS and 0 <= wrapper_status <= 255)
    need(fields[0] == "unknown" or re.fullmatch(r"[0-9]|[1-9][0-9]", fields[0]) is not None)
    need(fields[1] == "unknown" or re.fullmatch(r"[0-5][0-9]{2}", fields[1]) is not None)
    return {"exit": None if fields[0] == "unknown" else int(fields[0]),
            "http": None if fields[1] == "unknown" else int(fields[1]), "error": fields[2],
            "collector": fields[3], "teardown": fields[4], "wrapper": wrapper_status, "errno": None}


def observed(observation: dict) -> bool:
    return (observation["collector"] in {"completed", "oversized", "timed-out"} and observation["exit"] is not None
            and observation["teardown"] not in {"not-run", "unknown", "cancelled", "failed"}
            and observation.get("wrapper", 0) == 0)


def box_observation(value: dict, phase: str, record: str, wrapper: int, handoff: str) -> None:
    need(phase in {"baseline", "harmonized"})
    consumer = value["consumer"]
    # The canonical shell has already sampled this literal clock at parser
    # handoff. Import/recording delay, including suspend, is not poll duration.
    handoff_elapsed = origin_ms(handoff) - value["origin"]
    need(0 <= handoff_elapsed <= elapsed(value))
    if consumer["last"] is not None:
        need(handoff_elapsed >= consumer["last"]["elapsed_ms"])
    if value["native"]["last"] is not None:
        # /proc/uptime has centisecond precision; the native observer samples
        # nanoseconds. Their same-tick order is intentionally not invented.
        need(handoff_elapsed + 9 >= value["native"]["last"]["elapsed_ms"])
    observation = dict(parse_box(record, wrapper), phase=phase, elapsed_ms=handoff_elapsed)
    if phase == "baseline":
        need(consumer["baseline"] is None and (value["socket"] is None or value["socket"]["native_mode"] is None))
        consumer["baseline"] = observation
    else:
        need(value["socket"] is not None and value["socket"]["native_mode"] == 0o666)
    consumer["last"] = observation
    consumer["polls"] += 1
    if not observed(observation):
        value["uncertain"] = True
        raise Refused
    ready = (observation["collector"] in {"completed", "oversized"} and observation["exit"] == 0
             and observation["http"] is not None and observation["elapsed_ms"] < 180000)
    if ready and consumer["first_ready_ms"] is None:
        consumer["first_ready_ms"] = observation["elapsed_ms"]
    if not ready and consumer["first_failure"] is None:
        consumer["first_failure"] = observation
    if not ready and phase == "harmonized" and consumer["first_harmonized_failure"] is None:
        consumer["first_harmonized_failure"] = observation


def native_poll(value: dict, phase: str, socket: pathlib.Path) -> bool:
    need(phase in {"native-before", "native-after", "native-later"})
    endpoint(value, socket)
    need(value["socket"]["native_mode"] == 0o666)
    remaining = 360 - elapsed(value) / 1000
    need(remaining > 0)
    teardown = {}
    outcome, code, _stdout, _stderr = contract.native_read.native_poll_read(
        ["/bin/sh", "-c", NATIVE_COMMAND, "comparison-native", str(socket)],
        time.monotonic() + min(6, remaining), teardown_observations=teardown)
    observation = {"exit": code if type(code) is int and 0 <= code <= 255 else None, "http": None,
                   "collector": outcome if outcome in contract.PING_COLLECTORS else "unknown",
                   "teardown": teardown.get("signal") if teardown.get("signal") in contract.PING_TEARDOWN_SIGNALS else "unknown",
                   "phase": phase, "elapsed_ms": elapsed(value), "errno": None}
    native = value["native"]
    native["last"] = observation
    native["polls"] += 1
    key = {"native-before": "before", "native-after": "after"}.get(phase)
    if key is not None and native[key] is None:
        native[key] = observation
    if not observed(observation):
        value["uncertain"] = True
        raise Refused
    ready = (observation["collector"] == "completed" and observation["exit"] == 0
             and observation["elapsed_ms"] < 360000)
    if ready and native["first_ready_ms"] is None:
        native["first_ready_ms"] = observation["elapsed_ms"]
    return ready


def classify(value: dict) -> str:
    if value["prerequisite"] != "ready":
        return "prerequisite-not-met"
    if value["uncertain"]:
        return "indeterminate"
    consumer = value["consumer"]["first_ready_ms"]
    native = value["native"]["first_ready_ms"]
    if consumer is not None and consumer < 180000:
        return "both-routes-ready" if native is not None and native < 360000 else "consumer-only-ready-observed"
    if native is not None and native < 180000:
        return "early-native-only-ready-observed"
    if native is not None and native < 360000:
        return "native-ready-after-consumer-budget"
    return "shared-no-ready-observed"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("init", "start", "bind", "endpoint", "transition", "box", "consumer-expired", "native", "native-ready", "logs", "cleanup-check", "finish"))
    parser.add_argument("--directory", type=pathlib.Path, required=True)
    parser.add_argument("--native-root", type=pathlib.Path)
    parser.add_argument("--native-revision")
    parser.add_argument("--native-script-sha256")
    parser.add_argument("--expected-pid-namespace")
    parser.add_argument("--outer")
    parser.add_argument("--run")
    parser.add_argument("--socket", type=pathlib.Path)
    parser.add_argument("--created", type=pathlib.Path)
    parser.add_argument("--origin")
    parser.add_argument("--phase", choices=sorted(PHASES))
    parser.add_argument("--record")
    parser.add_argument("--handoff-boottime")
    parser.add_argument("--wrapper-status", type=int, default=0)
    parser.add_argument("--cleanup", choices=("verified", "unknown"))
    parser.add_argument("--run-status", type=int, default=1)
    args = parser.parse_args()
    value = None
    writable = False
    result = 0
    previous = {}
    try:
        for signum in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
            previous[signum] = signal.signal(signum, lambda *_: (_ for _ in ()).throw(KeyboardInterrupt()))
        if args.operation == "init":
            value = initialize(args)
            write_report(args.directory, value, create=True)
            writable = True
            value["namespace"] = host_prerequisite(args.expected_pid_namespace)
            value["phase"] = "native-binding"
            value["sources"] = source_binding(args)
            value["phase"] = "bridge-prerequisite"
            bridge_prerequisite(args.native_root)
            value["prerequisite"] = "ready"
        else:
            value = read_report(args.directory)
            if args.operation == "logs":
                need(value["startup_logs"]["status"] == "not-run")
                writable = True
                # Durable withholding precedes any observation. Timeout/KILL
                # cannot leave a partially classified category looking verified.
                value["startup_logs"] = {"status": "withheld", "category": None}
                write_report(args.directory, value)
                startup_logs(value, args.outer, args.run, args.socket)
                write_report(args.directory, value)
                return 0
            writable = True
            value["phase"] = args.phase or {"bind": "outer-admission", "endpoint": "endpoint-binding",
                                           "transition": "permission-transition"}.get(args.operation, "cleanup")
            if args.operation == "start":
                need(value["origin"] is None and isinstance(args.origin, str)
                     and re.fullmatch(r"[0-9]{1,12}\.[0-9]{2}", args.origin) is not None)
                value["origin"] = origin_ms(args.origin)
            elif args.operation == "bind":
                need(value["origin"] == origin_ms(args.origin))
                admit_outer(value, args.outer, args.run, args.socket, args.created, args.origin)
            elif args.operation == "endpoint":
                endpoint(value, args.socket)
            elif args.operation == "transition":
                endpoint(value, args.socket, transition=True)
            elif args.operation == "box":
                box_observation(value, args.phase, args.record, args.wrapper_status, args.handoff_boottime)
            elif args.operation == "consumer-expired":
                # No fabricated curl observation when the endpoint never appeared.
                need(elapsed(value) >= 174000 or value["consumer"]["first_ready_ms"] is not None)
                value["consumer"]["budget_closed"] = True
            elif args.operation == "native":
                result = 0 if native_poll(value, args.phase, args.socket) else 1
            elif args.operation == "native-ready":
                result = 0 if value["native"]["first_ready_ms"] is not None else 1
            elif args.operation == "cleanup-check":
                record = inspect_outer(args.outer, args.run)
                need(value["outer"] is not None and record["id"] == value["outer"]["id"])
            elif args.operation == "finish":
                if value["sources"]:
                    need(source_binding(args) == value["sources"])
                closed = args.cleanup == "verified"
                if value["outer"] is not None:
                    outer = value["outer"]
                    closed &= outer["pid"] is not None and not pathlib.Path(f'/proc/{outer["pid"]}').exists()
                    closed &= outer["layout"] is not None and not cgroup_path(outer).exists()
                value["cleanup"] = "verified" if closed else "unknown"
                value["classification"] = classify(value)
                value["status"] = "completed" if closed and not value["uncertain"] and args.run_status == 0 else "withheld"
                result = 0 if value["status"] == "completed" else 2
        write_report(args.directory, value)
        return result
    except (Exception, KeyboardInterrupt) as error:
        if value is not None and writable:
            if args.operation == "logs":
                value["startup_logs"] = {"status": "withheld", "category": None}
            else:
                value["errno"] = number(error)
                value["uncertain"] = True
                value["status"] = "withheld"
                value["classification"] = classify(value)
            try:
                write_report(args.directory, value)
            except Exception:
                pass
        print("readiness-comparison=unknown", file=sys.stderr)
        return 2
    finally:
        for signum, handler in previous.items():
            signal.signal(signum, handler)


if __name__ == "__main__":
    raise SystemExit(main())
