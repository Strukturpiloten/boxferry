"""Extract the pinned native Renovate test runtime without starting its image."""

from __future__ import annotations

import argparse
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
from pathlib import Path

MAX_OUTPUT = 64 * 1024
IMAGE_PATTERN = re.compile(
    r"ghcr\.io/renovatebot/renovate:(?P<version>[0-9]+\.[0-9]+\.[0-9]+)"
    r"@(?P<digest>sha256:[a-f0-9]{64})"
)
Command = Callable[[Sequence[str], float], str]


class RenovateRuntimeError(ValueError):
    """The pinned runtime could not be acquired or verified."""


def run_command(command: Sequence[str], timeout: float) -> str:
    """Bound time and all output; do not print registry or engine credentials."""
    try:
        process = subprocess.Popen(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )
    except OSError as error:
        raise RenovateRuntimeError(
            f"runtime command unavailable: {type(error).__name__}"
        ) from error
    stdout = bytearray()
    observed = 0
    successful = False
    deadline = time.monotonic() + timeout
    try:
        with selectors.DefaultSelector() as streams:
            if process.stdout is None or process.stderr is None:
                raise RenovateRuntimeError("runtime command output streams are unavailable")
            streams.register(process.stdout, selectors.EVENT_READ, True)
            streams.register(process.stderr, selectors.EVENT_READ, False)
            while streams.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise RenovateRuntimeError("runtime command exceeded its time bound")
                for key, _ in streams.select(remaining):
                    chunk = os.read(key.fd, 8192)
                    if not chunk:
                        streams.unregister(key.fileobj)
                        continue
                    observed += len(chunk)
                    if observed > MAX_OUTPUT:
                        raise RenovateRuntimeError("runtime command output exceeds the byte bound")
                    if key.data:
                        stdout.extend(chunk)
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise RenovateRuntimeError("runtime command exceeded its time bound")
            if process.wait(timeout=remaining):
                raise RenovateRuntimeError(f"runtime command failed ({Path(command[0]).name})")
        result = stdout.decode("utf-8")
        successful = True
        return result
    except (OSError, subprocess.TimeoutExpired, UnicodeError) as error:
        raise RenovateRuntimeError(
            f"runtime command unavailable: {type(error).__name__}"
        ) from error
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
        raise RenovateRuntimeError("runtime pin is unavailable or malformed") from error


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
        raise RenovateRuntimeError("runtime package identity is malformed") from error
    code = (
        "import {pathToFileURL} from 'node:url';"
        "const module=await import(pathToFileURL(process.argv[1]+'/dist/util/regex.js'));"
        "if(module.regexEngineStatus.type!=='available')"
        "throw new Error('pinned runtime requires native RE2');"
        "process.stdout.write('native-re2-available');"
    )
    result = command(["node", "--input-type=module", "-e", code, str(path)], 30)
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
    try:
        command([engine, "pull", image], 300)
        identities = json.loads(
            command([engine, "image", "inspect", "--format", "{{json .RepoDigests}}", image], 30)
        )
        expected = "ghcr.io/renovatebot/renovate@" + image.rsplit("@", 1)[1]
        if not isinstance(identities, list) or expected not in identities:
            raise RenovateRuntimeError(
                "pulled OCI image identity differs from the requested digest"
            )
        container_id = "boxferry-renovate-" + uuid.uuid4().hex
        created = command(
            [engine, "create", "--name", container_id, "--network", "none", image], 30
        ).strip()
        if re.fullmatch(r"[a-f0-9]{12,64}", created) is None:
            raise RenovateRuntimeError("engine returned an invalid stopped-container identity")
        command([engine, "cp", f"{container_id}:/usr/local/renovate/.", str(scratch)], 180)
        _verify(scratch, version, command)
        (scratch / "source-image.txt").write_text(image + "\n")
        command([engine, "rm", "--force", container_id], 30)
        container_id = None
        if destination.exists():
            shutil.rmtree(destination)
        scratch.rename(destination)
        return destination
    except (OSError, ValueError) as error:
        if isinstance(error, RenovateRuntimeError):
            raise
        raise RenovateRuntimeError("runtime acquisition returned malformed evidence") from error
    finally:
        try:
            if container_id is not None:
                command([engine, "rm", "--force", container_id], 30)
        finally:
            if scratch.exists():
                shutil.rmtree(scratch)


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
