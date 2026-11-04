"""Bounded subprocesses with run-owned cleanup and redacted diagnostic evidence."""

from __future__ import annotations

import os
import re
import selectors
import signal
import subprocess
import time
from collections.abc import Mapping
from dataclasses import asdict, dataclass, field
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

    def to_dict(self) -> dict[str, object]:
        return {
            "status": self.status,
            "returncode": self.returncode,
            "stdout": self.stdout,
            "stderr": self.stderr,
            "duration_seconds": self.duration_seconds,
            "diagnostic": asdict(self.diagnostic),
            "argv": list(self.argv),
        }


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
) -> RunResult:
    """Drain both pipes within one aggregate log bound and clean every owned child.

    Capture never limits ordinary files created by the command. Nonblocking pipe
    draining avoids deadlocks and caps memory without unbounded temporary logs.
    """
    if not argv or any(not arg or "\x00" in arg for arg in argv):
        raise ContractError("a command must contain nonempty, NUL-free argv elements")
    if not 0 < timeout_seconds <= MAX_TIMEOUT_SECONDS:
        raise ContractError(f"command timeout must be in (0, {MAX_TIMEOUT_SECONDS}] seconds")
    if not 0 < max_output_bytes <= MAX_OUTPUT_BYTES or not 0 <= grace_seconds <= 5:
        raise ContractError("command output or cleanup bounds are invalid")
    if os.name != "posix":
        raise ContractError("bounded shared CI execution requires POSIX process groups")
    env = dict(os.environ if environment is None else environment)
    hidden = (*protected_values(env), *secrets, *_argument_secrets(argv))
    started = time.monotonic()
    status, cause, returncode = "infrastructure", "command could not start", None
    captured = {"stdout": bytearray(), "stderr": bytearray()}
    total = 0
    process: subprocess.Popen[bytes] | None = None
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
                returncode = process.poll()
                if returncode is not None and not cleaned:
                    # An exited leader may leave descendants holding its pipes.
                    _cleanup(process, grace_seconds)
                    cleaned = True
                if returncode is not None and not selector.get_map():
                    status = "success" if returncode == 0 else "failed"
                    cause = (
                        "command completed" if returncode == 0 else f"command exited {returncode}"
                    )
                    break
                remaining = timeout_seconds - (time.monotonic() - started)
                if remaining <= 0:
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
                    captured[name].extend(data[:room])
                    total += min(len(data), room)
                    if len(data) > room:
                        exceeded = True
                        break
                if exceeded:
                    status, cause = "infrastructure", "command exceeded the bounded output limit"
                    break
        except (RunCancelled, KeyboardInterrupt):
            status, cause = "cancelled", "command was cancelled"
        except OSError as error:
            status, cause = "infrastructure", f"command could not start: {error}"
        finally:
            if process is not None:
                try:
                    _cleanup(process, grace_seconds)
                finally:
                    if process.stdout is not None:
                        process.stdout.close()
                    if process.stderr is not None:
                        process.stderr.close()
                returncode = process.returncode
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
    )
