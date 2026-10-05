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


def native_poll_read(arguments: list[str], deadline: float, *,
                     teardown_observations: dict[str, str] | None = None) -> tuple[str, int | None, bytes, bytes]:
    """Collect separate private streams without replacing a native failure status.

    Overflow discards only that stream but continues draining both. Only WNOWAIT evidence
    before group teardown supplies a native exit; our own signals supply none.
    The caller's absolute deadline includes startup and all teardown; collection
    leaves a 250 ms reserve, and teardown waits use only the remaining budget.
    """
    process = None
    outcome, native_status = "launch-failed", None
    payloads = [bytearray(), bytearray()]
    overflow = [False, False]
    terminated = True
    if teardown_observations is not None:
        teardown_observations["signal"] = "not-run"
    try:
        if deadline - time.monotonic() <= 0.25:
            return "timed-out", None, b"", b""
        process = subprocess.Popen(arguments, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, start_new_session=True)
        assert process.stdout is not None and process.stderr is not None
        streams = {process.stdout.fileno(): 0, process.stderr.fileno(): 1}
        for descriptor in streams:
            os.set_blocking(descriptor, False)
        # Start the request window after launch, but never borrow from the
        # caller's absolute startup/readiness/teardown deadline.
        expires = min(time.monotonic() + 5, deadline - 0.25)
        outcome = "timed-out"
        while time.monotonic() < expires:
            observed = os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
            if observed is not None:
                native_status = observed.si_status if observed.si_code == os.CLD_EXITED else -observed.si_status
            if not streams and observed is not None:
                outcome = ("both-oversized" if all(overflow) else "stdout-oversized" if overflow[0]
                           else "stderr-oversized" if overflow[1] else "completed")
                break
            remaining = expires - time.monotonic()
            if remaining <= 0:
                break
            if not streams:
                time.sleep(min(0.01, remaining))
                continue
            ready = select.select(list(streams), [], [], min(0.05, remaining))[0]
            for descriptor in ready:
                chunk = os.read(descriptor, 4096)
                if not chunk:
                    del streams[descriptor]
                elif not overflow[streams[descriptor]]:
                    stream_index = streams[descriptor]
                    output = payloads[stream_index]
                    if len(output) + len(chunk) > 16_384:
                        overflow[stream_index] = True
                        payloads[stream_index] = bytearray()
                    else:
                        output.extend(chunk)
    except KeyboardInterrupt:
        outcome = "cancelled"
    except (OSError, subprocess.SubprocessError):
        outcome = "read-failed"
    finally:
        if process is not None:
            signal_observation = "unknown"
            teardown_cancelled = False
            reaped = False
            closed = process.stdout is not None and process.stderr is not None
            # Never poll()/reap before signaling: the unreaped leader owns PGID.
            try:
                os.killpg(process.pid, signal.SIGKILL)
                signal_observation = "sent"
            except ProcessLookupError:
                signal_observation = "absent"
            except (OSError, KeyboardInterrupt) as failure:
                signal_observation = ("cancelled" if isinstance(failure, KeyboardInterrupt) else
                                      "denied" if isinstance(failure, PermissionError) else "failed")
                teardown_cancelled = isinstance(failure, KeyboardInterrupt)
                try:
                    process.kill()
                except KeyboardInterrupt:
                    teardown_cancelled = True
                except OSError:
                    pass
            if teardown_observations is not None:
                teardown_observations["signal"] = signal_observation
            try:
                process.wait(timeout=max(0, min(0.25, deadline - time.monotonic())))
                reaped = True
            except KeyboardInterrupt:
                teardown_cancelled = True
            except (OSError, subprocess.TimeoutExpired):
                pass
            for stream in (process.stdout, process.stderr):
                if stream is not None:
                    try:
                        stream.close()
                    except (OSError, KeyboardInterrupt) as failure:
                        closed = False
                        teardown_cancelled |= isinstance(failure, KeyboardInterrupt)
            # Successful reap AND both closes authorize only read-only lookup,
            # independently of earlier signal denial. Never mutate/reap again.
            absent = False
            try:
                group_deadline = min(deadline, time.monotonic() + 0.25)
                while reaped and closed and time.monotonic() < group_deadline:
                    try:
                        os.killpg(process.pid, 0)
                    except ProcessLookupError:
                        absent = time.monotonic() < group_deadline
                        break
                    except KeyboardInterrupt:
                        teardown_cancelled = True
                        break
                    except OSError:
                        break
                    remaining = group_deadline - time.monotonic()
                    if remaining <= 0:
                        break
                    time.sleep(min(0.01, remaining))
            except (OSError, KeyboardInterrupt) as failure:
                absent = False
                teardown_cancelled |= isinstance(failure, KeyboardInterrupt)
            terminated = reaped and closed and absent and not teardown_cancelled
    if not terminated:
        outcome = "termination-unverified"
    elif outcome in {"completed", "stdout-oversized", "stderr-oversized", "both-oversized"}:
        try:
            if time.monotonic() >= deadline:
                outcome = "timed-out"
        except (OSError, KeyboardInterrupt):
            outcome = "termination-unverified"
    if outcome not in {"completed", "stdout-oversized", "stderr-oversized", "both-oversized"}:
        return outcome, native_status, b"", b""
    return outcome, native_status, bytes(payloads[0]), bytes(payloads[1])
