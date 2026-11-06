"""Extract the pinned native Renovate test runtime without starting its image."""

from __future__ import annotations

import argparse
import errno
import json
import os
import re
import selectors
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import uuid
from collections.abc import Callable, Sequence
from enum import Enum
from pathlib import Path

MAX_OUTPUT = 64 * 1024
IMAGE_PATTERN = re.compile(
    r"ghcr\.io/renovatebot/renovate:(?P<version>[0-9]+\.[0-9]+\.[0-9]+)"
    r"@(?P<digest>sha256:[a-f0-9]{64})"
)
Command = Callable[[Sequence[str], float], str]


class RenovateRuntimeError(ValueError):
    """The pinned runtime could not be acquired or verified."""


class _Operation(Enum):
    UNKNOWN = "unknown"
    PULL = "image-pull"
    INSPECT = "digest-inspect"
    CREATE = "stopped-create"
    COPY = "copy"
    CLEANUP = "cleanup"
    RE2 = "native-RE2"


class _Reason(Enum):
    UNKNOWN = "unknown"
    STORAGE = "storage"
    AUTHENTICATION = "authentication"
    PERMISSION = "permission"
    NETWORK = "network"
    MISSING_RUNTIME = "missing-runtime"
    TIME_BOUND = "time bound"
    BYTE_BOUND = "output byte bound"
    INVALID_OUTPUT = "invalid-output"


class _CommandFailure(RenovateRuntimeError):
    def __init__(
        self, reason: _Reason, status: int | None, operation: _Operation = _Operation.UNKNOWN
    ) -> None:
        self.reason, self.status, self.operation = reason, status, operation
        super().__init__(
            f"runtime operation {operation.value} failed "
            f"(exit status {status if status is not None else 'unavailable'}; "
            f"reason {reason.value})"
        )


class _AcquisitionFailure(RenovateRuntimeError):
    def __init__(
        self, primary: RenovateRuntimeError, cleanup: Sequence[RenovateRuntimeError]
    ) -> None:
        self.primary, self.cleanup = primary, tuple(cleanup)
        suffix = "; cleanup failure: " + "; ".join(map(str, cleanup)) if cleanup else ""
        super().__init__(str(primary) + suffix)


def _failure_reason(content: bytes | bytearray) -> _Reason:
    # These are bounded diagnostic hints, never proof of the underlying host cause.
    # Only the closed category escapes; streams, matched text and credentials stay private.
    lowered = content.lower()
    for reason, phrases in (
        (_Reason.STORAGE, (b"no space left on device", b"disk quota exceeded")),
        (
            _Reason.AUTHENTICATION,
            (
                b"authentication required",
                b"unauthorized",
                b"requested access to the resource is denied",
            ),
        ),
        (_Reason.PERMISSION, (b"permission denied", b"operation not permitted")),
        (
            _Reason.NETWORK,
            (
                b"network is unreachable",
                b"connection refused",
                b"could not resolve host",
                b"no such host",
                b"connection reset by peer",
                b"connection timed out",
                b"temporary failure in name resolution",
                b"i/o timeout",
                b"tls handshake timeout",
            ),
        ),
        (_Reason.MISSING_RUNTIME, (b"executable file not found in $path", b"command not found")),
    ):
        if any(phrase in lowered for phrase in phrases):
            return reason
    return _Reason.UNKNOWN


def _os_reason(error: OSError, *, spawning: bool = False) -> _Reason:
    if spawning and error.errno == errno.ENOENT:
        return _Reason.MISSING_RUNTIME
    if error.errno in (errno.EACCES, errno.EPERM):
        return _Reason.PERMISSION
    if error.errno in (errno.ENOSPC, errno.EDQUOT):
        return _Reason.STORAGE
    if error.errno in (errno.ENETUNREACH, errno.ECONNREFUSED, errno.ETIMEDOUT):
        return _Reason.NETWORK
    return _Reason.UNKNOWN


def _invoke(operation: _Operation, argv: Sequence[str], timeout: float, command: Command) -> str:
    """Bind a fixed call site to safe runner facts, without rendering injected exceptions."""
    try:
        return command(argv, timeout)
    except _CommandFailure as error:
        raise _CommandFailure(error.reason, error.status, operation) from None
    except OSError as error:
        raise _CommandFailure(_os_reason(error), None, operation) from None
    except Exception:
        raise _CommandFailure(_Reason.UNKNOWN, None, operation) from None


def run_command(command: Sequence[str], timeout: float) -> str:
    """Bound time and aggregate output; publish only closed reasons and numeric status."""
    try:
        process = subprocess.Popen(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )
    except OSError as error:
        raise _CommandFailure(_os_reason(error, spawning=True), None) from None
    stdout, stderr = bytearray(), bytearray()
    observed = 0
    successful = False
    failure: _CommandFailure | None = None
    deadline = time.monotonic() + timeout
    result = ""
    try:
        with selectors.DefaultSelector() as streams:
            if process.stdout is None or process.stderr is None:
                raise _CommandFailure(_Reason.UNKNOWN, None)
            streams.register(process.stdout, selectors.EVENT_READ, True)
            streams.register(process.stderr, selectors.EVENT_READ, False)
            while streams.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise _CommandFailure(_Reason.TIME_BOUND, None)
                for key, _ in streams.select(remaining):
                    chunk = os.read(key.fd, 8192)
                    if not chunk:
                        streams.unregister(key.fileobj)
                        continue
                    observed += len(chunk)
                    if observed > MAX_OUTPUT:
                        raise _CommandFailure(_Reason.BYTE_BOUND, None)
                    (stdout if key.data else stderr).extend(chunk)
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise _CommandFailure(_Reason.TIME_BOUND, None)
            if process.wait(timeout=remaining):
                raise _CommandFailure(_failure_reason(stdout + b"\n" + stderr), process.returncode)
        result = stdout.decode("utf-8")
        successful = True
    except _CommandFailure as error:
        failure = error
    except subprocess.TimeoutExpired:
        failure = _CommandFailure(_Reason.TIME_BOUND, None)
    except OSError as error:
        failure = _CommandFailure(_os_reason(error), None)
    except UnicodeError:
        failure = _CommandFailure(_Reason.INVALID_OUTPUT, None)
    finally:
        if process.poll() is None or not successful:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
        if process.stdout is not None:
            process.stdout.close()
        if process.stderr is not None:
            process.stderr.close()
    if failure is not None:
        raise _CommandFailure(failure.reason, process.returncode) from None
    return result


def _pin(path: Path) -> tuple[str, str]:
    if path.is_symlink():
        raise RenovateRuntimeError("runtime pin must be a regular repository file")
    try:
        with path.open("rb") as stream:
            content = stream.read(4097)
        if len(content) > 4096:
            raise RenovateRuntimeError("runtime pin exceeds the byte bound")
        document = json.loads(content)
        if not isinstance(document, dict) or set(document) != {"image"}:
            raise RenovateRuntimeError("runtime pin must contain only the exact image identity")
        image = document["image"]
        match = IMAGE_PATTERN.fullmatch(image) if isinstance(image, str) else None
        if match is None:
            raise RenovateRuntimeError("runtime needs an exact official version and OCI digest")
        return image, match["version"]
    except (OSError, ValueError) as error:
        if isinstance(error, RenovateRuntimeError):
            raise
        raise RenovateRuntimeError("runtime pin is unavailable or malformed") from None


def _verify(path: Path, version: str, command: Command) -> None:
    package = path / "package.json"
    if package.is_symlink() or not package.is_file():
        raise RenovateRuntimeError("extracted runtime package identity is unavailable")
    try:
        with package.open("rb") as stream:
            content = stream.read(MAX_OUTPUT + 1)
        if len(content) > MAX_OUTPUT or json.loads(content).get("version") != version:
            raise RenovateRuntimeError("runtime package version differs from the pinned image tag")
    except (OSError, ValueError, AttributeError) as error:
        if isinstance(error, RenovateRuntimeError):
            raise
        raise RenovateRuntimeError("runtime package identity is malformed") from None
    code = (
        "import {pathToFileURL} from 'node:url';"
        "const module=await import(pathToFileURL(process.argv[1]+'/dist/util/regex.js'));"
        "if(module.regexEngineStatus.type!=='available')"
        "throw new Error('pinned runtime requires native RE2');"
        "process.stdout.write('native-re2-available');"
    )
    result = _invoke(
        _Operation.RE2, ["node", "--input-type=module", "-e", code, str(path)], 30, command
    )
    if result.strip() != "native-re2-available":
        raise RenovateRuntimeError("native RE2 verification did not return its expected evidence")


def prepare_runtime(
    pin: Path,
    destination: Path,
    *,
    engine: str,
    command: Command = run_command,
) -> Path:
    """Return a verified cache path; use explicit Podman locally and Docker in CI.

    The image is pulled by immutable digest, inspected, created with networking
    disabled, and copied while stopped. No entrypoint, build, Renovate platform
    command, or dependency update runs. Failed container cleanup fails installation.
    An existing unrelated destination is never removed.
    """
    if engine not in {"podman", "docker"}:
        raise RenovateRuntimeError("select the reviewed podman or docker engine explicitly")
    image, version = _pin(pin)
    if destination.is_symlink():
        raise RenovateRuntimeError("runtime destination must not be a symbolic link")
    destination = destination.absolute()
    marker = destination / "source-image.txt"
    managed = False
    if destination.exists():
        if not destination.is_dir():
            raise RenovateRuntimeError("runtime destination is not a directory")
        if marker.is_file() and not marker.is_symlink():
            with marker.open("rb") as stream:
                observed = stream.read(4097).decode("utf-8").strip()
            if len(observed) > 4096:
                raise RenovateRuntimeError("cache identity exceeds the byte bound")
            managed = IMAGE_PATTERN.fullmatch(observed) is not None
            if observed == image:
                try:
                    _verify(destination, version, command)
                    return destination
                except RenovateRuntimeError:
                    pass  # A damaged managed cache can be reacquired from its immutable source.
        if not managed and any(destination.iterdir()):
            raise RenovateRuntimeError("existing destination is not a managed Renovate cache")
    destination.parent.mkdir(parents=True, exist_ok=True)
    scratch = Path(tempfile.mkdtemp(prefix=".renovate-extract-", dir=destination.parent))
    container_id: str | None = None
    failure: RenovateRuntimeError | None = None
    cleanup_failures: list[RenovateRuntimeError] = []
    try:
        _invoke(_Operation.PULL, [engine, "pull", image], 300, command)
        identities = json.loads(
            _invoke(
                _Operation.INSPECT,
                [engine, "image", "inspect", "--format", "{{json .RepoDigests}}", image],
                30,
                command,
            )
        )
        expected = "ghcr.io/renovatebot/renovate@" + image.rsplit("@", 1)[1]
        if not isinstance(identities, list) or expected not in identities:
            raise RenovateRuntimeError(
                "pulled OCI image identity differs from the requested digest"
            )
        container_id = "boxferry-renovate-" + uuid.uuid4().hex
        created = _invoke(
            _Operation.CREATE,
            [engine, "create", "--name", container_id, "--network", "none", image],
            30,
            command,
        ).strip()
        if re.fullmatch(r"[a-f0-9]{12,64}", created) is None:
            raise RenovateRuntimeError("engine returned an invalid stopped-container identity")
        _invoke(
            _Operation.COPY,
            [engine, "cp", f"{container_id}:/usr/local/renovate/.", str(scratch)],
            180,
            command,
        )
        _verify(scratch, version, command)
        (scratch / "source-image.txt").write_text(image + "\n")
        _invoke(_Operation.CLEANUP, [engine, "rm", "--force", container_id], 30, command)
        container_id = None
        if destination.exists():
            shutil.rmtree(destination)
        scratch.rename(destination)
    except (OSError, ValueError) as error:
        failure = (
            error
            if isinstance(error, RenovateRuntimeError)
            else RenovateRuntimeError("runtime acquisition returned malformed evidence")
        )
    finally:
        try:
            if container_id is not None:
                try:
                    _invoke(
                        _Operation.CLEANUP, [engine, "rm", "--force", container_id], 30, command
                    )
                except RenovateRuntimeError as error:
                    cleanup_failures.append(error)
        finally:
            try:
                if scratch.exists():
                    shutil.rmtree(scratch)
            except OSError as error:
                cleanup_failures.append(
                    _CommandFailure(_os_reason(error), None, _Operation.CLEANUP)
                )
    if failure is not None:
        if cleanup_failures:
            raise _AcquisitionFailure(failure, cleanup_failures) from None
        raise failure from None
    if cleanup_failures:
        raise _AcquisitionFailure(cleanup_failures[0], cleanup_failures[1:]) from None
    return destination


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", required=True, choices=("podman", "docker"))
    parser.add_argument("--pin", required=True, type=Path)
    parser.add_argument("--destination", required=True, type=Path)
    options = parser.parse_args()
    try:
        print(prepare_runtime(options.pin, options.destination, engine=options.engine))
    except RenovateRuntimeError as error:
        print(f"Renovate test runtime: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
