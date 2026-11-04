"""Reconstruct one current queue tree from inert immutable public Git objects.

No checkout, candidate/user configuration, hooks or credentials participate.
Only fixed system Git and its HTTPS transport acquire the allowlisted objects.
The private bare repository is discarded after each proof; this is not a cache
or a claim about an independent synthetic PR merge ref.
"""

from __future__ import annotations

import os
import re
import resource
import selectors
import signal
import subprocess
import tempfile
import threading
import time
from collections.abc import Generator
from contextlib import contextmanager
from pathlib import Path
from types import FrameType

SHA = re.compile(r"[0-9a-f]{40}\Z")
MAX_BYTES = 128 * 1024 * 1024
MAX_FILES = 25000
MAX_OUTPUT = 64 * 1024
MAX_SECONDS = 120
MAX_SOURCES = 20


class QueueTreeError(ValueError):
    """Immutable objects do not establish one bounded, clean queue tree."""


class _MergeConflict(QueueTreeError):
    """Git positively reported unmerged entries, rather than a verifier failure."""


def _conflict_output(output: str) -> bool:
    fields = output.split("\0")
    if len(fields) < 3 or fields[-1] or not SHA.fullmatch(fields[0]):
        return False
    seen: set[tuple[str, str]] = set()
    for entry in fields[1:-1]:
        metadata, separator, path = entry.partition("\t")
        if (
            not separator
            or not path
            or not re.fullmatch(r"(?:100644|100755|120000|160000) [0-9a-f]{40} [123]", metadata)
        ):
            return False
        identity = (metadata[-1], path)
        if identity in seen:
            return False
        seen.add(identity)
    return bool(seen)


def _environment(root: Path) -> dict[str, str]:
    # An allowlist, rather than removing known credential variable names. Even
    # HTTPS proxy/netrc, Git executable overrides and tracing can expose tokens.
    return {
        "PATH": "/usr/bin:/bin",
        "HOME": str(root),
        "XDG_CONFIG_HOME": str(root),
        "LC_ALL": "C",
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_SYSTEM": os.devnull,
        "GIT_CONFIG_GLOBAL": os.devnull,
        "GIT_ATTR_NOSYSTEM": "1",
        "GIT_NO_REPLACE_OBJECTS": "1",
        "GIT_NO_LAZY_FETCH": "1",
        "GIT_TERMINAL_PROMPT": "0",
        "GIT_ALLOW_PROTOCOL": "https",
        "GIT_CONFIG_COUNT": "0",
    }


def _limits() -> None:
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    resource.setrlimit(resource.RLIMIT_FSIZE, (MAX_BYTES, MAX_BYTES))
    resource.setrlimit(resource.RLIMIT_AS, (1024**3, 1024**3))
    resource.setrlimit(resource.RLIMIT_CPU, (MAX_SECONDS, MAX_SECONDS))
    resource.setrlimit(resource.RLIMIT_NOFILE, (64, 64))


def _storage(root: Path) -> None:
    size = count = 0
    for directory, _directories, files in os.walk(root, followlinks=False):
        for name in files:
            item = Path(directory) / name
            if item.is_symlink():
                raise QueueTreeError("queue object storage contains a symlink")
            try:
                size += item.stat().st_size
            except FileNotFoundError:
                continue  # Git may rename its private temporary pack atomically.
            count += 1
            if size > MAX_BYTES or count > MAX_FILES:
                raise QueueTreeError("queue object storage exceeds its byte/file budget")


def _git(root: Path, deadline: float, *arguments: str) -> str:
    command = [
        "/usr/bin/git",
        "--no-replace-objects",
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "core.attributesFile=/dev/null",
        "-c",
        "credential.helper=",
        "-c",
        "credential.interactive=false",
        "-c",
        "http.followRedirects=false",
        "-c",
        "http.maxRequests=1",
        "-c",
        "http.lowSpeedLimit=1024",
        "-c",
        "http.lowSpeedTime=20",
        "-c",
        "protocol.allow=never",
        "-c",
        "protocol.https.allow=always",
        "-c",
        "fetch.fsckObjects=true",
        "-c",
        "transfer.fsckObjects=true",
        "-c",
        "fetch.unpackLimit=1",
        "-c",
        "pack.threads=1",
        "-c",
        "index.threads=1",
        "-c",
        "gc.auto=0",
        "-c",
        "maintenance.auto=false",
        "-C",
        str(root),
        *arguments,
    ]
    if time.monotonic() >= deadline:
        raise QueueTreeError("queue tree proof exceeded its time budget")
    stage = {
        "init": "initialize",
        "fetch": "fetch",
        "fsck": "verify-objects",
        "rev-parse": "verify-identity",
        "rev-list": "verify-parents",
        "merge-base": "merge-base",
        "merge-tree": "clean-merge",
    }.get(arguments[0], "verify")
    try:
        process = subprocess.Popen(
            command,
            env=_environment(root),
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
            preexec_fn=_limits,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise QueueTreeError(f"queue Git {stage} verifier unavailable") from error
    output = bytearray()
    observed = 0
    had_stderr = False
    successful = False
    try:
        with selectors.DefaultSelector() as streams:
            if process.stdout is None or process.stderr is None:
                raise QueueTreeError("queue Git verifier output unavailable")
            streams.register(process.stdout, selectors.EVENT_READ, True)
            streams.register(process.stderr, selectors.EVENT_READ, False)
            while streams.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise QueueTreeError("queue tree proof exceeded its time budget")
                _storage(root)
                for key, _ in streams.select(min(remaining, 0.1)):
                    chunk = os.read(key.fd, 8192)
                    if not chunk:
                        streams.unregister(key.fileobj)
                        continue
                    observed += len(chunk)
                    if observed > MAX_OUTPUT:
                        raise QueueTreeError("queue Git output exceeds its byte budget")
                    if key.data:
                        output.extend(chunk)
                    else:
                        had_stderr = True
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise QueueTreeError("queue tree proof exceeded its time budget")
        status = process.wait(timeout=remaining)
        _storage(root)
        decoded = output.decode("ascii")
        if status:
            # Only documented merge-tree exit=1 with valid NUL-delimited
            # unmerged entries proves a conflict. Unknown/fatal output cannot
            # eliminate a possible source from the selection.
            if (
                status == 1
                and arguments[:4] == ("merge-tree", "--write-tree", "-z", "--no-messages")
                and not had_stderr
                and _conflict_output(decoded)
            ):
                raise _MergeConflict("queue Git clean-merge proved a conflict")
            # Native stderr may contain untrusted filenames/commit messages.
            raise QueueTreeError(f"queue Git {stage} failed; no queue tree proof")
        successful = True
        return decoded
    except (OSError, UnicodeError, subprocess.TimeoutExpired) as error:
        raise QueueTreeError("queue Git object verifier failed within its budget") from error
    finally:
        if not successful or process.poll() is None:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
        if process.stdout is not None:
            process.stdout.close()
        if process.stderr is not None:
            process.stderr.close()


def _fetch(root: Path, deadline: float, repository: str, revisions: tuple[str, ...]) -> None:
    _git(
        root,
        deadline,
        "fetch",
        "--quiet",
        "--no-tags",
        "--no-write-fetch-head",
        "--no-recurse-submodules",
        "--no-filter",
        "--no-auto-maintenance",
        f"https://github.com/{repository}.git",
        *revisions,
    )


@contextmanager
def _scratch() -> Generator[Path]:
    """Reap children and remove private objects on ordinary CI cancellation."""

    def terminate(_signal: int, _frame: FrameType | None) -> None:
        raise QueueTreeError("queue tree proof cancelled")

    main_thread = threading.current_thread() is threading.main_thread()
    previous = signal.getsignal(signal.SIGTERM)
    if main_thread:
        signal.signal(signal.SIGTERM, terminate)
    try:
        with tempfile.TemporaryDirectory(prefix="boxferry-queue-objects-") as temporary:
            yield Path(temporary)
    finally:
        if main_thread:
            signal.signal(signal.SIGTERM, previous)


def matching_queue_sources(
    repository: str, base: str, sources: tuple[str, ...], candidate: str, *, deadline: float
) -> set[str]:
    """Match bounded waiting H markers in one scratch/acquisition/time budget."""
    from .eligibility import REPOSITORIES

    if (
        repository not in REPOSITORIES
        or not 1 <= len(sources) <= MAX_SOURCES
        or any(
            not isinstance(value, str) or not SHA.fullmatch(value)
            for value in (base, *sources, candidate)
        )
    ):
        raise QueueTreeError("queue tree proof requires bounded allowlisted immutable identities")
    deadline = min(deadline, time.monotonic() + MAX_SECONDS)
    revisions = tuple(dict.fromkeys((base, *sources, candidate)))
    with _scratch() as root:
        _git(root, deadline, "init", "--bare", "--quiet", "--template=", "--object-format=sha1")
        _fetch(root, deadline, repository, revisions)
        # A complete graph is required even for waiting entries: unknown ancestry
        # must not turn an ambiguous source into a uniquely accepted source.
        if (root / "shallow").exists() or (root / "objects/info/alternates").exists():
            raise QueueTreeError("queue object graph is shallow or uses alternates")
        if list((root / "objects/pack").glob("*.promisor")):
            raise QueueTreeError("queue object graph is partial")
        for revision in revisions:
            if (
                _git(root, deadline, "rev-parse", "--verify", f"{revision}^{{commit}}").strip()
                != revision
            ):
                raise QueueTreeError("queue commit identity differs from requested object")
        _git(root, deadline, "fsck", "--strict", "--no-reflogs", "--no-dangling", *revisions)
        parents = _git(root, deadline, "rev-list", "--parents", "--max-count=1", candidate).split()
        if candidate == base or candidate in sources or parents != [candidate, base]:
            raise QueueTreeError(
                "current queue candidate is not the reviewed single-parent squash shape"
            )
        _git(root, deadline, "merge-base", "--is-ancestor", base, candidate)
        actual = _git(root, deadline, "rev-parse", "--verify", f"{candidate}^{{tree}}").strip()
        if not SHA.fullmatch(actual):
            raise QueueTreeError("queue candidate has no valid immutable tree")
        matches: set[str] = set()
        for source in dict.fromkeys(sources):
            bases = _git(root, deadline, "merge-base", "--all", base, source).splitlines()
            if len(bases) != 1 or not SHA.fullmatch(bases[0]):
                raise QueueTreeError("queue inputs lack one unambiguous merge base")
            try:
                merged = _git(
                    root,
                    deadline,
                    "merge-tree",
                    "--write-tree",
                    "-z",
                    "--no-messages",
                    base,
                    source,
                )
            except _MergeConflict:
                continue  # Positively proved conflicts are unrelated nonmatches.
            if not merged.endswith("\0") or not SHA.fullmatch(merged[:-1]):
                raise QueueTreeError("queue clean merge did not produce one immutable tree")
            if merged[:-1] == actual:
                matches.add(source)
        return matches


def verify_queue_tree(
    repository: str, base: str, source: str, candidate: str, *, deadline: float
) -> None:
    """Require Q's full tree to equal a unique clean merge of immutable B and H."""
    if matching_queue_sources(repository, base, (source,), candidate, deadline=deadline) != {
        source
    }:
        raise QueueTreeError("queue candidate tree differs from independently merged source")
