"""Bounded subprocesses with run-owned cleanup and redacted diagnostic evidence."""

from __future__ import annotations

import hashlib
import json
import os
import re
import selectors
import signal
import subprocess
import time
from collections.abc import Callable, Mapping
from dataclasses import asdict, dataclass, field, replace
from enum import StrEnum
from pathlib import Path

from boxferry_ci.contracts import ContractError

MAX_OUTPUT_BYTES = 2_000_000
MAX_TIMEOUT_SECONDS = 7_200
_SENSITIVE_OPTION = re.compile(
    r"token|password|secret|credential|private[-_]key|authorization|api[-_]key|access[-_]key",
    re.IGNORECASE,
)


class RunCancelled(Exception):
    """The caller cancelled a run; its process group must be cleaned up."""


@dataclass(frozen=True)
class Diagnostic:
    stage: str
    cause: str
    affected_input: str
    evidence_link: str
    next_action: str


@dataclass(frozen=True)
class AbortContext:
    """Bounded original failure metadata, available before owned cleanup waits."""

    stage: str
    status: str
    cause: str


class AbortHookError(ContractError):
    """An original raised failure plus a separately redacted abort-hook failure."""

    def __init__(self, primary: str, secondary: str) -> None:
        super().__init__(primary + "; abort hook failed: " + secondary)
        self.primary_cause = primary
        self.abort_hook_failure = secondary


class OpaqueObservation(StrEnum):
    """Closed, opt-in purposes for bounded private inspection calls."""

    INSPECT_BEFORE = "inspect-before"
    INSPECT_AFTER = "inspect-after"

    @property
    def stage(self) -> str:
        return "probe-live-" + self.value

    @property
    def timeout(self) -> int:
        return 2

    @property
    def output_cap(self) -> int:
        return 65536


class OpaqueReason(StrEnum):
    COMPLETED = "completed"
    EXIT_NONZERO = "exit-nonzero"
    TIMEOUT = "timeout"
    OUTPUT_LIMIT = "output-limit"
    CANCELLED = "cancelled"
    SPAWN_UNAVAILABLE = "spawn-unavailable"
    MONITOR_FAILED = "monitor-failed"
    INVALID_JSON = "invalid-json"
    INVALID_NATIVE_SCHEMA = "invalid-native-schema"
    PRIVATE_PARSER_FAILED = "private-parser-failed"
    ABORT_HOOK_FAILED = "abort-hook-failed"
    CLEANUP_FAILED = "cleanup-failed"
    INVOCATION_UNAVAILABLE = "invocation-unavailable"


@dataclass(frozen=True)
class OpaqueMetrics:
    mode: OpaqueObservation
    reason: OpaqueReason
    status: str
    argv_sha256: str
    program_sha256: str | None
    stdout_prefix_bytes: int
    stderr_prefix_bytes: int
    stdout_prefix_sha256: str
    stderr_prefix_sha256: str
    prefix_scope: str = "observed-prefix-including-overflow-detection"
    cleanup_failed: bool = False
    errno: int | None = None

    def __post_init__(self) -> None:
        if (
            type(self.mode) is not OpaqueObservation
            or type(self.reason) is not OpaqueReason
            or type(self.status) is not str
            or self.status not in ("success", "failed", "cancelled", "infrastructure")
            or type(self.cleanup_failed) is not bool
            or self.prefix_scope != "observed-prefix-including-overflow-detection"
        ):
            raise ContractError("invalid opaque metrics")
        for value in (self.stdout_prefix_bytes, self.stderr_prefix_bytes):
            if type(value) is not int or not 0 <= value <= MAX_OUTPUT_BYTES + 1:
                raise ContractError("invalid opaque metrics")
        for value in (self.stdout_prefix_sha256, self.stderr_prefix_sha256):
            if type(value) is not str or re.fullmatch(r"[0-9a-f]{64}", value) is None:
                raise ContractError("invalid opaque metrics")
        for value in (self.argv_sha256, self.program_sha256):
            if value not in (None, "") and (
                type(value) is not str or re.fullmatch(r"[0-9a-f]{64}", value) is None
            ):
                raise ContractError("invalid opaque metrics")
        if self.errno is not None and (type(self.errno) is not int or not 0 < self.errno < 4096):
            raise ContractError("invalid opaque metrics")


@dataclass(frozen=True)
class OpaqueAbortContext(AbortContext):
    """Generated opaque metadata only; never an arbitrary diagnostic formatter."""

    metrics: OpaqueMetrics

    def __post_init__(self) -> None:
        if (
            type(self.metrics) is not OpaqueMetrics
            or type(self.metrics.mode) is not OpaqueObservation
            or type(self.metrics.reason) is not OpaqueReason
            or self.stage != self.metrics.mode.stage
            or self.status != self.metrics.status
            or self.cause != self.metrics.reason.value
        ):
            raise ContractError("invalid opaque abort context")

    def safe_fields(self) -> dict[str, object]:
        return {
            "opaque_failure_mode": self.metrics.mode.value,
            "opaque_failure_reason": self.metrics.reason.value,
            "opaque_failure_status": self.status,
            "opaque_failure_argv_sha256": self.metrics.argv_sha256,
            "opaque_failure_program_sha256": self.metrics.program_sha256,
            "opaque_failure_stdout_prefix_bytes": self.metrics.stdout_prefix_bytes,
            "opaque_failure_stderr_prefix_bytes": self.metrics.stderr_prefix_bytes,
            "opaque_failure_stdout_prefix_sha256": self.metrics.stdout_prefix_sha256,
            "opaque_failure_stderr_prefix_sha256": self.metrics.stderr_prefix_sha256,
        }


class OpaqueRunError(ContractError):
    """Fresh fixed-message error; no original exception, stream or argv attributes."""

    def __init__(self, reason: OpaqueReason, metrics: OpaqueMetrics | None = None) -> None:
        if type(reason) is not OpaqueReason:
            raise ContractError("invalid opaque reason")
        super().__init__(reason.value)
        self.reason = reason
        self.metrics = metrics


class OpaqueParseError(OpaqueRunError):
    """A source-owned private parser may classify a fixed schema failure."""


def opaque_errno(error: BaseException) -> int | None:
    """Only built-in OS exception slots may contribute a bounded numeric errno."""
    if type(error) not in (
        OSError,
        FileNotFoundError,
        PermissionError,
        ProcessLookupError,
        BlockingIOError,
        InterruptedError,
        IsADirectoryError,
        NotADirectoryError,
        BrokenPipeError,
        TimeoutError,
    ):
        return None
    assert isinstance(error, OSError)
    value = error.errno
    return value if type(value) is int and 0 < value < 4096 else None


def _cancellation(error: BaseException) -> int:
    # Retain only the cancellation kind, never an exception or its attributes.
    return (
        2 if isinstance(error, KeyboardInterrupt) else 1 if isinstance(error, RunCancelled) else 0
    )


def _raise_cancellation(kind: int) -> None:
    if kind == 2:
        raise KeyboardInterrupt(OpaqueReason.CANCELLED.value) from None
    if kind == 1:
        raise RunCancelled(OpaqueReason.CANCELLED.value) from None


def _opaque_setup(
    argv: tuple[str, ...],
    mode: OpaqueObservation,
    stage: str,
    timeout: float,
    output_cap: int,
    consumer: Callable[[bytes], None] | None,
) -> tuple[str, str | None]:
    failed = False
    cancellation = 0
    argv_hash, program_hash = "", None
    try:
        if (
            type(mode) is not OpaqueObservation
            or stage != mode.stage
            or timeout != mode.timeout
            or output_cap != mode.output_cap
            or not callable(consumer)
            or not argv
            or any(type(arg) is not str or not arg or "\x00" in arg for arg in argv)
        ):
            raise ValueError
        encoded = json.dumps(argv, ensure_ascii=True, separators=(",", ":")).encode("ascii")
        if len(encoded) > 32768:
            raise ValueError
        argv_hash = hashlib.sha256(encoded).hexdigest()
    except BaseException as error:
        cancellation = _cancellation(error)
        failed = True
    _raise_cancellation(cancellation)
    if failed:
        raise OpaqueRunError(OpaqueReason.INVOCATION_UNAVAILABLE) from None
    return argv_hash, program_hash


@dataclass(frozen=True)
class RunResult:
    status: str
    returncode: int | None
    stdout: str
    stderr: str
    duration_seconds: float
    diagnostic: Diagnostic
    argv: tuple[str, ...]
    # Exact bytes are for internal Git parsing only, never a public report/log.
    stdout_bytes: bytes = field(repr=False, default=b"")
    abort_hook_failure: str = ""

    def to_dict(self) -> dict[str, object]:
        result: dict[str, object] = {
            "status": self.status,
            "returncode": self.returncode,
            "stdout": self.stdout,
            "stderr": self.stderr,
            "duration_seconds": self.duration_seconds,
            "diagnostic": asdict(self.diagnostic),
            "argv": list(self.argv),
        }
        if self.abort_hook_failure:
            result["abort_hook_failure"] = self.abort_hook_failure
        return result


@dataclass(frozen=True)
class OpaqueRunResult(RunResult):
    metrics: OpaqueMetrics = field(kw_only=True)

    def to_dict(self) -> dict[str, object]:
        value = super().to_dict()
        value["opaque_observation"] = asdict(self.metrics)
        return value


def protected_values(environment: Mapping[str, str]) -> tuple[str, ...]:
    markers = ("TOKEN", "SECRET", "PASSWORD", "CREDENTIAL", "PRIVATE_KEY", "AUTHORIZATION")
    return tuple(
        value
        for key, value in environment.items()
        if len(value) >= 4 and any(marker in key.upper() for marker in markers)
    )


def redact(value: str, secrets: tuple[str, ...]) -> str:
    for secret in sorted(set(secrets), key=len, reverse=True):
        if secret:
            value = value.replace(secret, "[REDACTED]")
    return value


def _argument_secrets(argv: tuple[str, ...]) -> tuple[str, ...]:
    values: list[str] = []
    for index, argument in enumerate(argv):
        name, separator, value = argument.partition("=")
        if name.startswith("-") and _SENSITIVE_OPTION.search(name):
            if separator:
                values.append(value)
            elif index + 1 < len(argv):
                values.append(argv[index + 1])
    return tuple(values)


def _redacted_argv(argv: tuple[str, ...], secrets: tuple[str, ...], cwd: Path) -> tuple[str, ...]:
    safe: list[str] = []
    redact_next = False
    root = str(cwd.resolve())
    for index, argument in enumerate(argv):
        name, separator, _value = argument.partition("=")
        if redact_next:
            safe.append("[REDACTED]")
            redact_next = False
        elif name.startswith("-") and _SENSITIVE_OPTION.search(name):
            safe.append(redact(name, secrets) + ("=[REDACTED]" if separator else ""))
            redact_next = not separator
        else:
            value = redact(argument, secrets).replace(root + os.sep, "<repository>/")
            if value == root:
                value = "<repository>"
            if index == 0 and Path(value).is_absolute():
                value = Path(value).name
            safe.append(value)
    return tuple(safe)


def _cleanup(process: subprocess.Popen[bytes], grace_seconds: float) -> None:
    # Every run has its own session. Clean descendants even if the leader exited.
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        try:
            process.wait(timeout=grace_seconds)
        except subprocess.TimeoutExpired:
            pass
    finally:
        # Also runs if cancellation interrupts the graceful wait.
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()


def run(
    argv: tuple[str, ...],
    *,
    cwd: Path,
    stage: str,
    timeout_seconds: float = 900,
    environment: Mapping[str, str] | None = None,
    secrets: tuple[str, ...] = (),
    affected_input: str = "candidate checkout",
    evidence_link: str = "local command output",
    max_output_bytes: int = MAX_OUTPUT_BYTES,
    grace_seconds: float = 0.25,
    monitor: Callable[[], None] | None = None,
    on_abort: Callable[[AbortContext], None] | None = None,
    opaque_observation: OpaqueObservation | None = None,
    on_private_stdout: Callable[[bytes], None] | None = None,
) -> RunResult:
    """Drain both pipes within one aggregate log bound and clean every owned child.

    Capture never limits ordinary files created by the command. Nonblocking pipe
    draining avoids deadlocks and caps memory without unbounded temporary logs.
    """
    opaque = opaque_observation is not None
    argv_hash, program_hash = "", None
    if opaque:
        argv_hash, program_hash = _opaque_setup(
            argv, opaque_observation, stage, timeout_seconds, max_output_bytes, on_private_stdout
        )
    elif on_private_stdout is not None:
        raise ContractError("private stdout requires a closed opaque observation")
    if not argv or any(not arg or "\x00" in arg for arg in argv):
        raise ContractError("a command must contain nonempty, NUL-free argv elements")
    if not 0 < timeout_seconds <= MAX_TIMEOUT_SECONDS:
        raise ContractError(f"command timeout must be in (0, {MAX_TIMEOUT_SECONDS}] seconds")
    if not 0 < max_output_bytes <= MAX_OUTPUT_BYTES or not 0 <= grace_seconds <= 5:
        raise ContractError("command output or cleanup bounds are invalid")
    if os.name != "posix":
        raise ContractError("bounded shared CI execution requires POSIX process groups")
    setup_failed = False
    setup_cancellation = 0
    env: dict[str, str] = {}
    try:
        env = dict(os.environ if environment is None else environment)
        if opaque:
            started = time.monotonic()
    except BaseException as error:
        if not opaque:
            raise
        setup_cancellation = _cancellation(error)
        setup_failed = True
    _raise_cancellation(setup_cancellation)
    if setup_failed:
        raise OpaqueRunError(OpaqueReason.INVOCATION_UNAVAILABLE) from None
    hidden = () if opaque else (*protected_values(env), *secrets, *_argument_secrets(argv))
    if not opaque:
        started = time.monotonic()
    status, cause, returncode = "infrastructure", "command could not start", None
    captured = {"stdout": bytearray(), "stderr": bytearray()}
    total = 0
    process: subprocess.Popen[bytes] | None = None
    notified = False
    hook_failure = ""
    reason = OpaqueReason.INVOCATION_UNAVAILABLE
    prefix_counts = {"stdout": 0, "stderr": 0}
    prefix_setup_failed = False
    prefix_cancellation = 0
    try:
        prefix_hashes = {"stdout": hashlib.sha256(), "stderr": hashlib.sha256()}
    except BaseException as error:
        if not opaque:
            raise
        prefix_cancellation = _cancellation(error)
        prefix_setup_failed = True
    _raise_cancellation(prefix_cancellation)
    if prefix_setup_failed:
        raise OpaqueRunError(OpaqueReason.INVOCATION_UNAVAILABLE) from None
    cleanup_failed = False
    safe_errno = None
    pending_opaque_raise = False
    phase = "spawn"
    first_failure: tuple[OpaqueReason, str] | None = None

    def fail(why: OpaqueReason, state: str) -> None:
        nonlocal first_failure, reason, status
        if first_failure is None:
            first_failure = (why, state)
        reason, status = first_failure

    def metrics(why: OpaqueReason, state: str) -> OpaqueMetrics:
        assert opaque_observation is not None
        return OpaqueMetrics(
            opaque_observation,
            why,
            state,
            argv_hash,
            program_hash,
            prefix_counts["stdout"],
            prefix_counts["stderr"],
            prefix_hashes["stdout"].hexdigest(),
            prefix_hashes["stderr"].hexdigest(),
            cleanup_failed=cleanup_failed,
            errno=safe_errno,
        )

    def notify(primary: str, abort_status: str | None = None) -> None:
        nonlocal notified, hook_failure
        if opaque:
            fail(reason, abort_status or status)
        if notified or on_abort is None:
            return
        notified = True
        try:
            if opaque:
                context = OpaqueAbortContext(
                    stage,
                    abort_status or status,
                    reason.value,
                    metrics(reason, abort_status or status),
                )
            else:
                context = AbortContext(
                    redact(stage, hidden), abort_status or status, redact(primary, hidden)[:2048]
                )
            on_abort(context)
        except BaseException as error:
            if opaque:
                hook_failure = OpaqueReason.ABORT_HOOK_FAILED.value
            else:
                try:
                    summary = type(error).__name__ + ": " + str(error)
                except BaseException:
                    summary = "unprintable abort hook exception"
                hook_failure = redact(summary, hidden)[:512]

    selector_failed = False
    try:
        with selectors.DefaultSelector() as selector:
            try:
                process = subprocess.Popen(
                    argv,
                    cwd=cwd,
                    env=env,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    start_new_session=True,
                )
                if process.stdout is None or process.stderr is None:
                    raise ContractError("command output pipes were not created")
                for stream, name in ((process.stdout, "stdout"), (process.stderr, "stderr")):
                    os.set_blocking(stream.fileno(), False)
                    selector.register(stream, selectors.EVENT_READ, name)
                cleaned = False
                while True:
                    if monitor is not None:
                        phase = "monitor"
                        monitor()
                    phase = "capture"
                    returncode = process.poll()
                    if returncode is not None and not cleaned:
                        # Default callers retain the existing early cleanup. Hook callers
                        # must first know whether an abort is needed, before cleanup can wait.
                        if returncode != 0:
                            if opaque:
                                reason = OpaqueReason.EXIT_NONZERO
                            notify("" if opaque else f"command exited {returncode}", "failed")
                        if on_abort is None or returncode != 0:
                            phase = "cleanup"
                            _cleanup(process, grace_seconds)
                            phase = "capture"
                            cleaned = True
                    if returncode is not None and not selector.get_map():
                        status = "success" if returncode == 0 else "failed"
                        cause = (
                            "command completed"
                            if returncode == 0
                            else f"command exited {returncode}"
                        )
                        if opaque:
                            reason = (
                                OpaqueReason.COMPLETED
                                if returncode == 0
                                else OpaqueReason.EXIT_NONZERO
                            )
                            if returncode == 0:
                                phase = "parse"
                                try:
                                    if time.monotonic() - started >= timeout_seconds:
                                        raise OpaqueParseError(OpaqueReason.TIMEOUT)
                                    private = bytes(captured["stdout"])
                                    captured["stdout"].clear()
                                    assert on_private_stdout is not None
                                    on_private_stdout(private)
                                    if time.monotonic() - started >= timeout_seconds:
                                        raise OpaqueParseError(OpaqueReason.TIMEOUT)
                                except BaseException as error:
                                    if _cancellation(error):
                                        fail(OpaqueReason.CANCELLED, "cancelled")
                                    else:
                                        fail(
                                            error.reason
                                            if type(error) is OpaqueParseError
                                            else OpaqueReason.PRIVATE_PARSER_FAILED,
                                            "infrastructure",
                                        )
                                finally:
                                    private = b""
                                    captured["stdout"].clear()
                        break
                    remaining = timeout_seconds - (time.monotonic() - started)
                    if remaining <= 0:
                        reason = OpaqueReason.TIMEOUT
                        cause = f"command timed out after {timeout_seconds:g}s"
                        break
                    exceeded = False
                    for key, _events in selector.select(timeout=min(0.025, remaining)):
                        try:
                            data = os.read(key.fd, min(65536, max_output_bytes - total + 1))
                        except BlockingIOError:
                            continue
                        if not data:
                            selector.unregister(key.fileobj)
                            continue
                        room = max_output_bytes - total
                        name = str(key.data)
                        if opaque:
                            prefix_counts[name] += len(data)
                            prefix_hashes[name].update(data)
                            if name == "stdout":
                                captured[name].extend(data[:room])
                        else:
                            captured[name].extend(data[:room])
                        total += min(len(data), room)
                        if len(data) > room:
                            exceeded = True
                            break
                    if exceeded:
                        reason = OpaqueReason.OUTPUT_LIMIT
                        status, cause = (
                            "infrastructure",
                            "command exceeded the bounded output limit",
                        )
                        break
            except (RunCancelled, KeyboardInterrupt):
                if opaque:
                    if phase == "cleanup":
                        cleanup_failed = True
                    fail(OpaqueReason.CANCELLED, "cancelled")
                else:
                    status = "cancelled"
                    reason = OpaqueReason.CANCELLED
                cause = "command was cancelled"
            except OSError as error:
                if opaque:
                    reason = (
                        OpaqueReason.CLEANUP_FAILED
                        if phase == "cleanup"
                        else OpaqueReason.SPAWN_UNAVAILABLE
                        if phase == "spawn"
                        else OpaqueReason.INVOCATION_UNAVAILABLE
                    )
                    safe_errno = opaque_errno(error)
                    status = "infrastructure"
                    pending_opaque_raise = phase == "cleanup"
                    cleanup_failed = phase == "cleanup"
                    fail(reason, status)
                else:
                    status, cause = "infrastructure", f"command could not start: {error}"
            except BaseException as error:
                if opaque:
                    reason = (
                        OpaqueReason.CLEANUP_FAILED
                        if phase == "cleanup"
                        else OpaqueReason.MONITOR_FAILED
                    )
                    status = "infrastructure"
                    pending_opaque_raise = True
                    cleanup_failed = phase == "cleanup"
                    fail(reason, status)
                    if process is not None:
                        notify("")
                else:
                    if process is not None:
                        notify(str(error))
                        if hook_failure:
                            raise AbortHookError(redact(str(error), hidden), hook_failure) from None
                    raise
            finally:
                if process is not None:
                    if status != "success":
                        notify(cause)
                    if opaque:
                        try:
                            _cleanup(process, grace_seconds)
                        except BaseException as error:
                            safe_errno = opaque_errno(error)
                            cleanup_failed = True
                            pending_opaque_raise = True
                            fail(
                                OpaqueReason.CANCELLED
                                if _cancellation(error)
                                else OpaqueReason.CLEANUP_FAILED,
                                "cancelled" if _cancellation(error) else "infrastructure",
                            )
                            notify("")
                        finally:
                            for stream in (process.stdout, process.stderr):
                                if stream is not None:
                                    try:
                                        stream.close()
                                    except BaseException as error:
                                        cleanup_failed = True
                                        pending_opaque_raise = True
                                        fail(
                                            OpaqueReason.CANCELLED
                                            if _cancellation(error)
                                            else OpaqueReason.CLEANUP_FAILED,
                                            "cancelled"
                                            if _cancellation(error)
                                            else "infrastructure",
                                        )
                                        notify("")
                            returncode = process.returncode
                    else:
                        try:
                            _cleanup(process, grace_seconds)
                        finally:
                            if process.stdout is not None:
                                process.stdout.close()
                            if process.stderr is not None:
                                process.stderr.close()
                            returncode = process.returncode
    except BaseException as error:
        if not opaque:
            raise
        selector_failed = True
        fail(
            OpaqueReason.CANCELLED
            if _cancellation(error)
            else OpaqueReason.CLEANUP_FAILED
            if process is not None
            else OpaqueReason.INVOCATION_UNAVAILABLE,
            "cancelled" if _cancellation(error) else "infrastructure",
        )
    if selector_failed:
        pending_opaque_raise = True
        cleanup_failed = process is not None
        if process is not None:
            notify("")
    if opaque:
        captured["stdout"].clear()
        captured["stderr"].clear()
        # Clock, metrics and result construction all belong to the private
        # boundary. Never retry a failed clock or fabricate elapsed time.
        final_failed = False
        safe_metrics: OpaqueMetrics | None = None
        result: OpaqueRunResult | None = None
        try:
            safe_metrics = metrics(reason, status)
            if not pending_opaque_raise:
                duration = time.monotonic() - started
                assert opaque_observation is not None
                display = ("opaque-observation", opaque_observation.value, argv_hash)
                if program_hash is not None:
                    display += (program_hash,)
                result = OpaqueRunResult(
                    status,
                    returncode,
                    "",
                    "",
                    duration,
                    Diagnostic(
                        stage,
                        reason.value,
                        "held local process observation",
                        "opaque prefix metrics",
                        "Review the fixed reason and scoped observation evidence.",
                    ),
                    display,
                    b"",
                    hook_failure,
                    metrics=safe_metrics,
                )
        except BaseException as error:
            fail(
                OpaqueReason.CANCELLED
                if _cancellation(error)
                else OpaqueReason.INVOCATION_UNAVAILABLE,
                "cancelled" if _cancellation(error) else "infrastructure",
            )
            final_failed = True
        if final_failed:
            notify("")
            if safe_metrics is not None:
                # Reuse the observed evidence; do not repeat any failing producer.
                try:
                    safe_metrics = replace(safe_metrics, reason=reason, status=status)
                except BaseException:
                    safe_metrics = None
        if pending_opaque_raise or final_failed:
            raise OpaqueRunError(reason, safe_metrics) from None
        assert result is not None
        return result
    raw_out = bytes(captured["stdout"])
    out = redact(raw_out.decode("utf-8", "replace"), hidden)
    err = redact(bytes(captured["stderr"]).decode("utf-8", "replace"), hidden)
    diagnostic = Diagnostic(
        stage=redact(stage, hidden),
        cause=redact(cause, hidden),
        affected_input=redact(affected_input, hidden),
        evidence_link=redact(evidence_link, hidden),
        next_action=(
            "No action required."
            if status == "success"
            else "Inspect the captured stdout/stderr; correct the check or runner cause and retry."
        ),
    )
    return RunResult(
        status,
        returncode,
        out,
        err,
        time.monotonic() - started,
        diagnostic,
        _redacted_argv(argv, hidden, cwd),
        raw_out,
        hook_failure,
    )
