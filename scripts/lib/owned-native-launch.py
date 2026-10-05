"""Linux single-threaded read-only harness launch ownership, not an executor API."""

from __future__ import annotations

import os
import resource
import select
import signal
import sys
import threading
import time
from contextlib import contextmanager


CANCEL = {signal.SIGALRM, signal.SIGTERM, signal.SIGINT, signal.SIGHUP}
READY = b"R"
ACK = b"A"


def need(value):
    if not value:
        raise OSError


@contextmanager
def critical():
    previous = signal.pthread_sigmask(signal.SIG_BLOCK, CANCEL | {signal.SIGCHLD})
    try:
        yield previous
    finally:
        signal.pthread_sigmask(signal.SIG_SETMASK, previous)


def identity(pid):
    fd = None
    try:
        with critical():
            fd = os.open(f"/proc/{pid}/stat", os.O_RDONLY | os.O_NOFOLLOW)
        raw = os.read(fd, 4097)
    finally:
        with critical():
            if fd is not None:
                os.close(fd)
                fd = None
    need(len(raw) <= 4096 and raw.startswith(f"{pid} (".encode()))
    fields = raw[raw.rfind(b") ") + 2:].split()
    need(len(fields) >= 20)
    return int(fields[1]), int(fields[19]), int(fields[2]), int(fields[3])


@contextmanager
def shutdown_signals():
    """Publish shutdown state before propagating any catchable cancellation."""
    cancelled = [False]
    handlers = {}
    def defer(_signum, _frame):
        cancelled[0] = True
    with critical():
        for signum in CANCEL:
            handlers[signum] = signal.signal(signum, defer)
    try:
        yield
    finally:
        with critical():
            for signum, handler in handlers.items():
                signal.signal(signum, handler)
    if cancelled[0]:
        raise KeyboardInterrupt


class OwnedChild:
    def __init__(self):
        self.pid = self.pidfd = self.start = None
        self.state = "bootstrap-owned"
        self.returncode = None
        self.stdout = self.stderr = None
        self.fds = {}
        self.closed = False
        self.verified = False

    def close_fd(self, name):
        with critical():
            fd = self.fds.pop(name, None)
            if fd is not None:
                os.close(fd)


class OwnedLauncher:
    def __init__(self, work_deadline, teardown_deadline):
        need(sys.platform == "linux"
             and threading.current_thread() is threading.main_thread() and threading.active_count() == 1
             and hasattr(os, "fork") and hasattr(os, "WNOWAIT"))
        self.work_deadline, self.teardown_deadline = work_deadline, teardown_deadline
        self.teardown_left = 1.0
        self.teardown_late = False
        self.children = []
        self.parent = os.getpid()
        self.max_fd = resource.getrlimit(resource.RLIMIT_NOFILE)[0]
        need(0 < self.max_fd < 2**31 and teardown_deadline > work_deadline)

    def work_remaining(self, monotonic_deadline):
        return min(self.work_deadline - time.clock_gettime(time.CLOCK_BOOTTIME),
                   monotonic_deadline - time.monotonic())

    def acquire(self, argv, *, merge_output=False):
        need(not self.teardown_late and time.clock_gettime(time.CLOCK_BOOTTIME) < self.work_deadline and len(self.children) < 3
             and threading.current_thread() is threading.main_thread() and threading.active_count() == 1
             and isinstance(argv, list) and argv and all(type(arg) is str and "\0" not in arg for arg in argv))
        child = OwnedChild()
        # Only nonblocking allocation/fork/publication is masked. All bootstrap
        # and exec waiting occurs after restoration, under the caller's alarm.
        with critical() as previous:
            self.children.append(child)
            child.fds["null"] = os.open("/dev/null", os.O_RDWR | os.O_NONBLOCK | os.O_CLOEXEC)
            for prefix in ("output", "ready", "ack"):
                flags = os.O_CLOEXEC | (0 if prefix == "output" else os.O_NONBLOCK)
                read, write = os.pipe2(flags)
                child.fds[prefix + "-read"], child.fds[prefix + "-write"] = read, write
            pid = os.fork()
            if pid == 0:
                self.child_exec(child.fds, argv, merge_output, previous)
            child.pid = pid
            if hasattr(os, "pidfd_open"):
                child.pidfd = os.pidfd_open(pid)
            child.stdout = os.fdopen(child.fds["output-read"], "rb", buffering=0)
            del child.fds["output-read"]
            for name in ("null", "output-write", "ready-write", "ack-read"):
                child.close_fd(name)
        return child

    def child_exec(self, fds, argv, merge_output, previous):
        try:
            for signum in CANCEL | {signal.SIGCHLD}:
                signal.signal(signum, signal.SIG_DFL)
            signal.pthread_sigmask(signal.SIG_SETMASK, previous)
            os.dup2(fds["null"], 0)
            os.dup2(fds["output-write"], 1)
            os.dup2(fds["output-write"] if merge_output else fds["null"], 2)
            for name in ("null", "output-read", "output-write", "ready-read", "ack-write"):
                os.close(fds[name])
            os.setsid()
            need(os.write(fds["ready-write"], READY) == 1)
            # READY does not authorize exec. The parent first records verified
            # session ownership and sends ACK; without it no descendants exist.
            select.select([fds["ack-read"]], [], [])
            need(os.read(fds["ack-read"], 2) == ACK)
            os.closerange(3, self.max_fd)
            os.execvp(argv[0], argv)
        except BaseException:
            os._exit(127)

    def proof(self, child, snapshot, *, session=False):
        need(child.pid is not None and child.state not in {"reaped", "ownership-lost"})
        try:
            observed = os.waitid(os.P_PID, child.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
        except OSError:
            child.state = "ownership-lost"
            raise OSError from None
        if observed is not None and observed.si_pid != child.pid:
            child.state = "ownership-lost"
            raise OSError
        parent, start, group, session_id = snapshot
        valid = parent == self.parent and (child.start is None or child.start == start)
        if session:
            try:
                valid &= group == session_id == child.pid and os.getpgid(child.pid) == os.getsid(child.pid) == child.pid
            except OSError:
                child.state = "ownership-lost"
                raise OSError from None
        if not valid:
            child.state = "ownership-lost"
            raise OSError
        return observed

    def prepare(self, child, absolute_deadline):
        need(not self.teardown_late)
        remaining = self.work_remaining(absolute_deadline)
        need(remaining > 0 and select.select([child.fds["ready-read"]], [], [], remaining)[0])
        need(os.read(child.fds["ready-read"], 2) == READY)
        snapshot = identity(child.pid)
        with critical():
            self.proof(child, snapshot, session=True)
            child.start = snapshot[1]
            child.state = "session-admitted"
            need(self.work_remaining(absolute_deadline) > 0)
            need(os.write(child.fds["ack-write"], ACK) == 1)
            child.close_fd("ack-write")
            child.close_fd("ready-read")

    def observe(self, child):
        need(not self.teardown_late)
        snapshot = identity(child.pid)
        with critical():
            return self.proof(child, snapshot, session=child.state == "session-admitted")

    def teardown(self, child):
        with self.shutdown_scope() as end:
            return self._teardown(child, end)

    @contextmanager
    def shutdown_scope(self):
        # Charge the complete outer scope, including handler setup/restoration,
        # cache hits and FD closure. Nested registry iterations never debit again.
        started = time.clock_gettime(time.CLOCK_BOOTTIME)
        allowance = self.teardown_left
        end = min(self.teardown_deadline, started + allowance)
        try:
            with shutdown_signals():
                yield end
        finally:
            finished = time.clock_gettime(time.CLOCK_BOOTTIME)
            self.teardown_left = max(0, allowance - (finished - started))
            if finished >= end:
                self.teardown_late = True
        need(not self.teardown_late)

    def _teardown(self, child, end):
        if child is None:
            self._close_all(end)
            return None
        if child.closed and child.state == "reaped":
            need(child.state == "reaped" and child.verified)
            return child.returncode
        verified = False
        try:
            if child.pid is None:
                child.state = "reaped"  # Fork never created a child.
            elif child.state != "reaped":
                need(child.state != "ownership-lost" and time.clock_gettime(time.CLOCK_BOOTTIME) < end)
                try:
                    snapshot = identity(child.pid)
                except OSError:
                    child.state = "ownership-lost"
                    raise
                with critical():
                    self.proof(child, snapshot, session=child.state == "session-admitted")
                    need(time.clock_gettime(time.CLOCK_BOOTTIME) < end)
                    try:
                        if child.state == "session-admitted":
                            os.killpg(child.pid, signal.SIGKILL)
                        elif child.pidfd is not None and hasattr(signal, "pidfd_send_signal"):
                            signal.pidfd_send_signal(child.pidfd, signal.SIGKILL)
                        else:
                            os.kill(child.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                while True:
                    with critical():
                        try:
                            pid, status = os.waitpid(child.pid, os.WNOHANG)
                        except OSError:
                            child.state = "ownership-lost"
                            raise OSError from None
                        if pid:
                            if pid != child.pid:
                                child.state = "ownership-lost"
                                raise OSError
                            child.returncode = os.waitstatus_to_exitcode(status)
                            child.state = "reaped"
                    if pid:
                        break
                    need(time.clock_gettime(time.CLOCK_BOOTTIME) < end)
                    time.sleep(min(0.005, max(0, end - time.clock_gettime(time.CLOCK_BOOTTIME))))
            if child.state == "reaped" and child.start is not None:
                try:
                    os.killpg(child.pid, 0)  # Read-only, never signal after reap.
                except ProcessLookupError:
                    pass
                else:
                    raise OSError
            need(child.state == "reaped")
            verified = True
            return child.returncode
        finally:
            with critical():
                close_failed = False
                if child.stdout is not None:
                    try:
                        child.stdout.close()
                    except OSError:
                        close_failed = True
                for name in tuple(child.fds):
                    try:
                        child.close_fd(name)
                    except OSError:
                        close_failed = True
                if child.pidfd is not None:
                    try:
                        os.close(child.pidfd)
                    except OSError:
                        close_failed = True
                    child.pidfd = None
                child.closed = True
                child.verified = verified and not close_failed
            need(child.verified)

    def close_all(self):
        with self.shutdown_scope() as end:
            self._close_all(end)

    def _close_all(self, end):
        failed = False
        for child in self.children:
            try:
                self._teardown(child, end)
            except (OSError, KeyboardInterrupt):
                failed = True
        need(not failed)
