"""Canonical bounded, private read-only subprocess evidence for test harnesses."""

from __future__ import annotations

import os
import select
import signal
import subprocess
import time


def readiness_read(arguments: list[str], deadline: float, *, merge_output: bool = False,
                   presence: bool = False) -> tuple[str, bytes]:
    """Bound one read-only subprocess; neither stderr nor failed output escapes."""
    process = None
    outcome, raw = "read-failed", b""
    interrupted = False
    termination_failed = False
    completed = False
    status = None
    expires = min(deadline, time.monotonic() + 3)
    try:
        if expires <= time.monotonic():
            return "timed-out", b""
        process = subprocess.Popen(arguments, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                   stderr=subprocess.STDOUT if merge_output or presence else subprocess.DEVNULL,
                                   start_new_session=True)
        assert process.stdout is not None
        descriptor = process.stdout.fileno()
        os.set_blocking(descriptor, False)
        output = bytearray()
        while True:
            remaining = expires - time.monotonic()
            if remaining <= 0:
                outcome = "timed-out"
                break
            if not select.select([descriptor], [], [], remaining)[0]:
                outcome = "timed-out"
                break
            chunk = os.read(descriptor, min(4096, 16_385 - len(output)))
            if not chunk:
                # WNOWAIT keeps the leader PID reserved until its owned group
                # has been terminated. Never reap and then signal a numeric PGID.
                while True:
                    observed = os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
                    if observed is not None:
                        completed, raw = True, bytes(output)
                        break
                    remaining = expires - time.monotonic()
                    if remaining <= 0:
                        outcome = "timed-out"
                        break
                    time.sleep(min(0.01, remaining))
                break
            output.extend(chunk)
            if len(output) > 16_384:
                outcome = "oversized"
                break
    except subprocess.TimeoutExpired:
        outcome = "timed-out"
    except OSError:
        pass
    except KeyboardInterrupt:
        interrupted = True
    finally:
        if process is not None:
            # This session belongs only to this diagnostic read, not the daemon.
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            except (OSError, KeyboardInterrupt):
                termination_failed = True
                try:
                    process.kill()
                except (OSError, KeyboardInterrupt):
                    pass
            try:
                status = process.wait(timeout=max(0.001, min(1, deadline - time.monotonic())))
            except (OSError, subprocess.TimeoutExpired, KeyboardInterrupt):
                termination_failed = True
            finally:
                if process.stdout is not None:
                    try:
                        process.stdout.close()
                    except OSError:
                        termination_failed = True
            if presence and not termination_failed:
                # Reaping the leader alone does not prove that its descendants
                # disappeared. After reaping, only read the group identity:
                # never signal a numeric PGID that could have been reused.
                group_deadline = min(deadline, time.monotonic() + 0.25)
                try:
                    for _attempt in range(26):
                        try:
                            os.killpg(process.pid, 0)
                        except ProcessLookupError:
                            break
                        remaining = group_deadline - time.monotonic()
                        if remaining <= 0:
                            termination_failed = True
                            break
                        time.sleep(min(0.01, remaining))
                    else:
                        termination_failed = True
                except (OSError, KeyboardInterrupt):
                    termination_failed = True
    if termination_failed:
        return "termination-unverified", b""
    if interrupted:
        raise KeyboardInterrupt
    if completed:
        if presence:
            # Native exit 1 can also carry configuration errors. Only a fully
            # captured empty combined stream establishes presence or absence.
            return ({0: "present", 1: "absent"}.get(status, "unknown"), b"") if raw == b"" else ("unknown", b"")
        return ("read", raw) if status == 0 else ("read-failed", b"")
    return outcome, raw
