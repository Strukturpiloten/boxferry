#!/usr/bin/env python3
"""Bound work in the calling shell with identity-safe Linux process handles."""

from __future__ import annotations

import argparse
import ctypes
import datetime
import math
import os
import re
import select
import signal
import time
from dataclasses import dataclass
from pathlib import Path


DURATION = re.compile(r"(?P<value>[0-9]+(?:\.[0-9]+)?)(?P<unit>[smh]?)")
UNIT_SECONDS = {"": 1, "s": 1, "m": 60, "h": 60 * 60}


@dataclass(frozen=True)
class ProcessIdentity:
    """Stable-enough /proc identity used to validate a newly opened pidfd."""

    pid: int
    parent_pid: int
    process_group: int
    session: int
    start_time: int


@dataclass
class ProcessHandle:
    """A process identity paired with its non-reusable kernel handle."""

    identity: ProcessIdentity
    pidfd: int


def duration_seconds(value: str) -> float:
    """Parse the bounded duration syntax used by the live runner."""
    match = DURATION.fullmatch(value)
    if match is None:
        raise argparse.ArgumentTypeError(f"invalid duration: {value}")
    seconds = float(match.group("value")) * UNIT_SECONDS[match.group("unit")]
    if seconds <= 0:
        raise argparse.ArgumentTypeError("duration must be positive")
    return seconds


def process_identity(pid: int) -> ProcessIdentity | None:
    """Read one Linux process identity without trusting a reusable PID alone."""
    try:
        stat = Path(f"/proc/{pid}/stat").read_text(encoding="utf-8")
        fields = stat[stat.rfind(")") + 2 :].split()
        return ProcessIdentity(
            pid=pid,
            parent_pid=int(fields[1]),
            process_group=int(fields[2]),
            session=int(fields[3]),
            # starttime is field 22; fields starts at the field-3 state value.
            start_time=int(fields[19]),
        )
    except (OSError, ValueError, IndexError):
        return None


def process_snapshot() -> dict[int, ProcessIdentity]:
    """Return a best-effort identity snapshot of the current Linux process tree."""
    result: dict[int, ProcessIdentity] = {}
    for process_directory in Path("/proc").glob("[0-9]*"):
        try:
            pid = int(process_directory.name)
        except ValueError:
            continue
        identity = process_identity(pid)
        if identity is not None:
            result[pid] = identity
    return result


def descendant_identities(
    runner: ProcessHandle, excluded_pids: set[int]
) -> list[ProcessIdentity]:
    """Find current descendants deepest-first while the original runner exists."""
    snapshot = process_snapshot()
    observed_runner = snapshot.get(runner.identity.pid)
    if (
        observed_runner is None
        or observed_runner.start_time != runner.identity.start_time
    ):
        return []

    children: dict[int, list[int]] = {}
    for identity in snapshot.values():
        children.setdefault(identity.parent_pid, []).append(identity.pid)

    result: list[ProcessIdentity] = []
    visited: set[int] = set()

    def visit(parent_pid: int) -> None:
        for child_pid in children.get(parent_pid, []):
            if child_pid in excluded_pids or child_pid in visited:
                continue
            visited.add(child_pid)
            visit(child_pid)
            identity = snapshot.get(child_pid)
            if identity is not None:
                result.append(identity)

    visit(runner.identity.pid)
    return result


def open_process_handle(expected: ProcessIdentity) -> ProcessHandle | None:
    """Open and validate a pidfd before retaining authority to signal a process."""
    before = process_identity(expected.pid)
    if before is None or before.start_time != expected.start_time:
        return None
    try:
        pidfd = os.pidfd_open(expected.pid)
    except OSError:
        return None
    after = process_identity(expected.pid)
    if after is None or after.start_time != expected.start_time:
        os.close(pidfd)
        return None
    return ProcessHandle(identity=after, pidfd=pidfd)


def process_exited(handle: ProcessHandle) -> bool:
    """Report process exit through its pidfd, independent of PID reuse."""
    poller = select.poll()
    poller.register(handle.pidfd, select.POLLIN)
    return bool(poller.poll(0))


def wait_for_runner_or_cancel(runner: ProcessHandle, deadline: float) -> str:
    """Wait for runner exit, timely cancellation, or the hard deadline."""
    poller = select.poll()
    poller.register(runner.pidfd, select.POLLIN)
    poller.register(0, select.POLLIN | select.POLLHUP)
    control = b""
    control_open = True
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return "expired"
        events = poller.poll(math.ceil(remaining * 1000))
        if process_exited(runner):
            return "runner-exited"
        # Deadline wins when control and expiry become observable together.
        if time.monotonic() >= deadline:
            return "expired"
        for descriptor, _event in events:
            if descriptor != 0 or not control_open:
                continue
            chunk = os.read(0, 64)
            if not chunk:
                poller.unregister(0)
                control_open = False
                continue
            control += chunk
            if len(control) > len(b"cancel\n") or b"\n" in control:
                if control == b"cancel\n":
                    return "cancelled"
                raise SystemExit("invalid deadline helper control request")


def send_signal(handle: ProcessHandle, requested_signal: signal.Signals) -> None:
    """Signal only the process represented by the retained kernel handle."""
    try:
        signal.pidfd_send_signal(handle.pidfd, requested_signal)
    except ProcessLookupError:
        pass


def send_process_group(
    runner: ProcessHandle, requested_signal: signal.Signals
) -> None:
    """Signal the runner's pinned, isolated process group."""
    try:
        os.killpg(runner.identity.process_group, requested_signal)
    except ProcessLookupError:
        pass


def configure_parent_death_signal(parent_pid: int) -> None:
    """Ensure the private group anchor cannot outlive a failed watchdog."""
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(1, signal.SIGKILL, 0, 0, 0) != 0:
        os._exit(125)
    if os.getppid() != parent_pid:
        os._exit(125)


def start_group_anchor(runner: ProcessHandle) -> tuple[int, ProcessHandle]:
    """Pin the runner's process-group identity across runner and child exit."""
    watchdog_pid = os.getpid()
    ready_reader, ready_writer = os.pipe()
    anchor_pid = os.fork()
    if anchor_pid == 0:
        os.close(ready_reader)
        configure_parent_death_signal(watchdog_pid)
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        os.write(ready_writer, b"ready")
        os.close(ready_writer)
        for descriptor in (0, 1, 2, 3):
            try:
                os.close(descriptor)
            except OSError:
                pass
        while True:
            signal.pause()

    os.close(ready_writer)
    anchor_ready = os.read(ready_reader, len(b"ready"))
    os.close(ready_reader)
    anchor_identity = process_identity(anchor_pid)
    if anchor_ready != b"ready" or anchor_identity is None:
        os.waitpid(anchor_pid, 0)
        raise SystemExit("deadline process-group anchor failed to initialize")
    anchor = open_process_handle(anchor_identity)
    if (
        anchor is None
        or anchor.identity.process_group != runner.identity.process_group
        or anchor.identity.session != runner.identity.session
    ):
        if anchor is not None:
            send_signal(anchor, signal.SIGKILL)
            os.close(anchor.pidfd)
        else:
            # An unreaped direct child cannot have its numeric PID reused.
            try:
                os.kill(anchor_pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        os.waitpid(anchor_pid, 0)
        raise SystemExit("deadline process-group anchor has wrong identity")
    return anchor_pid, anchor


def stop_group_anchor(anchor_pid: int, anchor: ProcessHandle) -> None:
    """Kill and reap the private process-group identity anchor."""
    send_signal(anchor, signal.SIGKILL)
    os.waitpid(anchor_pid, 0)
    os.close(anchor.pidfd)


def capture_descendants(
    runner: ProcessHandle,
    watchdog_pid: int,
    retained: dict[tuple[int, int], ProcessHandle],
) -> list[ProcessHandle]:
    """Retain handles for new descendants and return those newly captured."""
    captured: list[ProcessHandle] = []
    for identity in descendant_identities(runner, {watchdog_pid}):
        key = (identity.pid, identity.start_time)
        if key in retained:
            continue
        handle = open_process_handle(identity)
        if handle is not None:
            retained[key] = handle
            captured.append(handle)
    return captured


def timestamp() -> str:
    """Return the evidence timestamp format used by the live runner."""
    return datetime.datetime.now(tz=datetime.timezone.utc).strftime(
        "%Y-%m-%dT%H:%M:%SZ"
    )


def write_timeout(name: str, kill_after: float) -> None:
    """Write the fail-closed deadline event to the runner's evidence descriptor."""
    os.write(
        3,
        (
            f"{timestamp()} STEP FAIL  {name} "
            f"(deadline expired; TERM, KILL after {kill_after:g}s)\n"
        ).encode(),
    )


def parser() -> argparse.ArgumentParser:
    """Build the intentionally small helper command contract."""
    result = argparse.ArgumentParser()
    result.add_argument("--runner-pid", required=True, type=int)
    result.add_argument("--deadline", required=True, type=duration_seconds)
    result.add_argument("--kill-after", required=True, type=duration_seconds)
    result.add_argument("--name", required=True)
    return result


def main() -> int:
    """Arm the watchdog, then terminate the bounded runner tree at its deadline."""
    args = parser().parse_args()
    watchdog_pid = os.getpid()
    if os.getppid() != args.runner_pid:
        raise SystemExit("deadline runner parent changed before initialization")

    runner_identity = process_identity(args.runner_pid)
    if runner_identity is None:
        raise SystemExit("deadline runner exited before initialization")
    if (
        runner_identity.process_group != args.runner_pid
        or runner_identity.session != args.runner_pid
    ):
        raise SystemExit("deadline runner must lead an isolated process group")
    runner = open_process_handle(runner_identity)
    if runner is None or os.getppid() != args.runner_pid:
        raise SystemExit("deadline runner identity changed before initialization")
    anchor_pid, anchor = start_group_anchor(runner)
    os.setsid()

    # The caller must not begin work until the runner group is pinned and the
    # watchdog has detached itself from that TERM/KILL containment boundary.
    os.write(1, b"ready\n")

    expires_at = time.monotonic() + args.deadline
    event = wait_for_runner_or_cancel(runner, expires_at)
    if event == "runner-exited":
        send_process_group(runner, signal.SIGKILL)
        os.waitpid(anchor_pid, 0)
        os.close(anchor.pidfd)
        os.close(runner.pidfd)
        return 0
    if event == "cancelled":
        stop_group_anchor(anchor_pid, anchor)
        os.write(1, b"cancelled\n")
        os.close(runner.pidfd)
        return 0

    retained: dict[tuple[int, int], ProcessHandle] = {}
    capture_descendants(runner, watchdog_pid, retained)
    write_timeout(args.name, args.kill_after)
    send_process_group(runner, signal.SIGTERM)

    # Keep every pidfd after parent exit. A TERM-resistant orphan still receives
    # KILL, and a surviving runner cannot evade the boundary by spawning again.
    kill_at = time.monotonic() + args.kill_after
    while time.monotonic() < kill_at:
        roots = [runner, *retained.values()]
        for root in roots:
            if process_exited(root):
                continue
            capture_descendants(root, watchdog_pid, retained)
        time.sleep(min(0.05, max(0.0, kill_at - time.monotonic())))

    for handle in retained.values():
        send_signal(handle, signal.SIGKILL)
    send_process_group(runner, signal.SIGKILL)
    os.waitpid(anchor_pid, 0)
    os.close(anchor.pidfd)
    for handle in retained.values():
        os.close(handle.pidfd)
    os.close(runner.pidfd)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
