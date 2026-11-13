#!/usr/bin/env python3
"""Inspect an exact mounted source, or explicitly snapshot it into a standalone clone.

No repository repair or trust changes. The fallback must run where the original
metadata is accessible and requires a new task-owned destination.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import signal
import stat
import sys
import tempfile
import threading
from collections.abc import Generator, Mapping
from contextlib import contextmanager
from pathlib import Path, PurePosixPath
from typing import Any

from boxferry_ci import runner

MAX_FILES = 100_000
MAX_BYTES = 512 * 1024 * 1024
MAX_OUTPUT = 64 * 1024 * 1024
COMMAND_SECONDS = 120
ROUTING = (
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_EXEC_PATH",
    "GIT_SHALLOW_FILE",
    "GIT_GRAFT_FILE",
    "GIT_REPLACE_REF_BASE",
    "GIT_CONFIG_PARAMETERS",
    "GIT_NAMESPACE",
)


class SourceError(ValueError):
    """The intended source cannot be inspected without changing its meaning."""

    def __init__(self, message: str, *, category: str = "source-refused") -> None:
        super().__init__(message)
        self.category = category


def trusted_entries(root: Path) -> dict[str, str]:
    """Admit only the qualifier's finite exact-path ownership configuration."""
    count = os.environ.get("GIT_CONFIG_COUNT", "0")
    if not count.isdecimal() or not 0 <= int(count) <= 7:
        raise SourceError("Git injected configuration is not finite scoped trust")
    count_number = int(count)
    allowed = {
        str(root),
        *(
            str(root / ".boxferry-workspace" / name)
            for name in (
                "compose-lens",
                "podman-lens",
                "quadlet-lens",
                "docker-lens",
                "kubernetes-lens",
                "boxferry-website",
            )
        ),
    }
    expected = {f"GIT_CONFIG_{kind}_{i}" for i in range(count_number) for kind in ("KEY", "VALUE")}
    injected = {
        key for key in os.environ if key.startswith(("GIT_CONFIG_KEY_", "GIT_CONFIG_VALUE_"))
    }
    if injected != expected:
        raise SourceError("Git injected configuration has missing or extra entries")
    for index in range(count_number):
        if (
            os.environ[f"GIT_CONFIG_KEY_{index}"] != "safe.directory"
            or os.environ[f"GIT_CONFIG_VALUE_{index}"] not in allowed
        ):
            raise SourceError(
                "Git injected configuration must be exact intended safe.directory paths"
            )
    return {"GIT_CONFIG_COUNT": count, **{key: os.environ[key] for key in expected}}


@contextmanager
def cancellation() -> Generator[None]:
    """Let canonical runner cleanup finish before cancelling owned clone resources."""
    if threading.current_thread() is not threading.main_thread():
        raise SourceError("Git source operations require the main thread for scoped cancellation")
    previous = signal.getsignal(signal.SIGTERM)

    def cancel(_signal: int, _frame: object) -> None:
        raise runner.RunCancelled("Git source operation cancelled")

    if previous != signal.SIG_IGN:
        signal.signal(signal.SIGTERM, cancel)
    try:
        yield
    finally:
        signal.signal(signal.SIGTERM, previous)


def git(root: Path, *args: str) -> bytes:
    for key in ROUTING:
        if key in os.environ:
            raise SourceError(f"unset {key}; Git checks must inspect the explicit source and index")
    injected = trusted_entries(root)
    environment = dict(
        {
            key: value
            for key, value in os.environ.items()
            if key in {"PATH", "HOME", "TMPDIR", "LC_ALL", "LANG", "TZ"}
        },
        **injected,
        GIT_CONFIG_NOSYSTEM="1",
        GIT_CONFIG_GLOBAL=os.devnull,
        GIT_CONFIG_SYSTEM=os.devnull,
        GIT_OPTIONAL_LOCKS="0",
        GIT_TERMINAL_PROMPT="0",
        GIT_NO_REPLACE_OBJECTS="1",
        GIT_NO_LAZY_FETCH="1",
    )
    argv = (
        "git",
        "--no-replace-objects",
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "core.fsmonitor=false",
        "-c",
        "protocol.allow=never",
        "-c",
        "maintenance.auto=false",
        "-c",
        "gc.auto=0",
        "-C",
        str(root),
        *args,
    )
    with cancellation():
        result = runner.run(
            argv,
            cwd=root,
            stage="git-source-identity",
            timeout_seconds=COMMAND_SECONDS,
            environment=environment,
        )
    if result.status != "success" or result.returncode != 0:
        raise SourceError(
            f"Git refused the explicit source/index ({result.diagnostic.cause}); "
            "check its metadata mount and ownership (never add wildcard safe.directory trust)"
        )
    return result.stdout_bytes


def metadata(root: Path) -> Path:
    marker = root / ".git"
    if marker.is_dir() and not marker.is_symlink():
        directory = marker
    elif marker.is_file() and not marker.is_symlink():
        if marker.stat().st_size > 4096:
            raise SourceError("Git metadata pointer exceeds its bound")
        value = marker.read_text().strip()
        if not value.startswith("gitdir: ") or "\n" in value:
            raise SourceError("malformed Git metadata pointer")
        pointer = Path(value[8:])
        directory = (root / pointer).resolve()
        if not directory.is_dir():
            if pointer.is_absolute():
                raise SourceError(
                    "legacy absolute Git metadata is outside this mount; on the host run "
                    "python3 scripts/git-worktree.py standalone --source SOURCE --destination "
                    "NEW_TASK_CLONE, then explicitly open that clone in the container",
                    category="legacy-metadata-unmounted",
                )
            raise SourceError(
                "relative worktree metadata is not mounted; enable VS Code's application "
                "setting dev.containers.experimentalMountGitWorktreeCommonDir, or use "
                "devcontainer up --mount-git-worktree-common-dir true; rebuild/reopen",
                category="relative-metadata-unmounted",
            )
    else:
        raise SourceError("the explicit source must contain its own real .git directory or file")
    common = directory / "commondir"
    if common.exists():
        if common.is_symlink() or not common.is_file() or common.stat().st_size > 4096:
            raise SourceError("malformed Git common-directory metadata")
        target = (directory / common.read_text().strip()).resolve()
        if not target.is_dir():
            raise SourceError(
                "Git common metadata is missing; mount the common directory "
                "using the documented Dev Containers worktree option",
                category="relative-metadata-unmounted",
            )
    return directory.resolve()


@contextmanager
def parent_descriptor(root: Path, name: str) -> Generator[int]:
    directory = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        for part in PurePosixPath(name).parts[:-1]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=directory)
            os.close(directory)
            directory = child
        yield directory
    finally:
        os.close(directory)


def _member(root: Path, name: str) -> tuple[bytes, int, str]:
    with parent_descriptor(root, name) as parent:
        leaf = PurePosixPath(name).name
        try:
            observed = os.stat(leaf, dir_fd=parent, follow_symlinks=False)
        except FileNotFoundError:
            return b"", 0, "deleted"
        if stat.S_ISLNK(observed.st_mode):
            return (
                os.fsencode(os.readlink(leaf, dir_fd=parent)),
                stat.S_IMODE(observed.st_mode),
                "symlink",
            )
        if not stat.S_ISREG(observed.st_mode):
            raise SourceError(
                "standalone evidence requires regular files or symlinks; "
                "materialize submodules separately"
            )
        descriptor = os.open(leaf, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
        with os.fdopen(descriptor, "rb") as incoming:
            opened = os.fstat(incoming.fileno())
            if not stat.S_ISREG(opened.st_mode) or opened.st_size > MAX_OUTPUT:
                raise SourceError("source exceeds its byte bound or changed file type")
            content = incoming.read(MAX_OUTPUT + 1)
            if len(content) > MAX_OUTPUT:
                raise SourceError("source exceeds its byte bound")
            return content, stat.S_IMODE(opened.st_mode), "file"


def member(root: Path, name: str) -> tuple[bytes, int, str]:
    try:
        return _member(root, name)
    except FileNotFoundError:
        return b"", 0, "deleted"


def files(root: Path) -> tuple[dict[str, dict[str, Any]], int]:
    paths = git(root, "ls-files", "--cached", "--others", "--exclude-standard", "-z").split(b"\0")
    result: dict[str, dict[str, Any]] = {}
    total = 0
    for raw in sorted(set(filter(None, paths))):
        if len(result) >= MAX_FILES:
            raise SourceError("source exceeds its file bound")
        name = os.fsdecode(raw)
        path = PurePosixPath(name)
        if path.is_absolute() or any(p in {".", "..", ".git"} for p in path.parts):
            raise SourceError("unsafe source member")
        content, mode, kind = member(root, name)
        if kind == "deleted":
            result[name] = {"kind": kind}
            continue
        total += len(content)
        if total > MAX_BYTES or len(result) >= MAX_FILES:
            raise SourceError("source exceeds its file/byte bound")
        result[name] = {"kind": kind, "mode": mode, "sha256": hashlib.sha256(content).hexdigest()}
    return result, total


def inspect(source: Path) -> dict[str, Any]:
    root = source.resolve(strict=True)
    directory = metadata(root)
    top = Path(os.fsdecode(git(root, "rev-parse", "--show-toplevel")).strip()).resolve()
    actual = Path(os.fsdecode(git(root, "rev-parse", "--absolute-git-dir")).strip()).resolve()
    index = Path(
        os.fsdecode(git(root, "rev-parse", "--path-format=absolute", "--git-path", "index")).strip()
    )
    if top != root or actual != directory or index.resolve() != directory / "index":
        raise SourceError("Git selected another source or index")
    if index.is_symlink() or not index.is_file() or index.stat().st_size > MAX_OUTPUT:
        raise SourceError("the intended index must be a bounded regular file")
    descriptor = os.open(index, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as index_file:
        if not stat.S_ISREG(os.fstat(index_file.fileno()).st_mode):
            raise SourceError("the intended index changed file type")
        raw = index_file.read(MAX_OUTPUT + 1)
        if len(raw) > MAX_OUTPUT:
            raise SourceError("the intended index exceeded its byte bound")
    raw_index = hashlib.sha256(raw).hexdigest()
    configuration = {}
    for row in git(root, "config", "--local", "--null", "--list").split(b"\0"):
        key, separator, value = row.partition(b"\n")
        if separator and (key == b"core.repositoryformatversion" or key.startswith(b"extensions.")):
            name = key.decode("ascii")
            if name in configuration:
                raise SourceError("ambiguous repository format/extension declarations")
            configuration[name] = value.decode("utf-8")
    format_version = configuration.get("core.repositoryformatversion", "0")
    if format_version not in {"0", "1"}:
        raise SourceError("unsupported repository format version")
    entries = git(root, "ls-files", "--stage", "-z")
    if git(root, "ls-files", "--unmerged", "-z"):
        raise SourceError("resolve index conflicts before standalone validation")
    if any(line[:1] != b"H" for line in git(root, "ls-files", "-v", "-z").split(b"\0") if line):
        raise SourceError(
            "materialize sparse/assume-unchanged entries before standalone validation"
        )
    members, size = files(root)
    return {
        "schema": 1,
        "source": str(root),
        "git_dir": str(directory),
        "index": str(index),
        "git_version": os.fsdecode(git(root, "--version")).strip(),
        "head": os.fsdecode(git(root, "rev-parse", "--verify", "HEAD")).strip(),
        "index_sha256": hashlib.sha256(entries).hexdigest(),
        "raw_index_sha256": raw_index,
        "repository_format": int(format_version),
        "extensions": {
            key.removeprefix("extensions."): value
            for key, value in configuration.items()
            if key.startswith("extensions.")
        },
        "files_sha256": hashlib.sha256(json.dumps(members, sort_keys=True).encode()).hexdigest(),
        "bytes": size,
        "files": members,
    }


def identity(receipt: Mapping[str, object]) -> tuple[object, ...]:
    return tuple(receipt[key] for key in ("head", "index_sha256", "files_sha256"))


def baseline(root: Path) -> dict[str, str] | None:
    if b"origin" not in git(root, "remote").splitlines():
        return None
    origin = os.fsdecode(git(root, "remote", "get-url", "origin")).strip()
    if origin not in {
        "https://github.com/Strukturpiloten/boxferry",
        "https://github.com/Strukturpiloten/boxferry.git",
        "git@github.com:Strukturpiloten/boxferry.git",
        "ssh://git@github.com/Strukturpiloten/boxferry.git",
    }:
        raise SourceError(
            "standalone validation requires the canonical BoxFerry origin, without credentials"
        )
    return {
        "origin": origin,
        "origin_main": os.fsdecode(
            git(root, "rev-parse", "--verify", "refs/remotes/origin/main")
        ).strip(),
    }


def standalone(source: Path, destination: Path) -> dict[str, Any]:
    with cancellation():
        return _standalone(source, destination)


def _standalone(source: Path, destination: Path) -> dict[str, Any]:
    source = source.resolve(strict=True)
    destination = destination.absolute()
    before = inspect(source)
    authority = baseline(source)
    patch = git(
        source,
        "diff",
        "--cached",
        "--binary",
        "--full-index",
        "--no-renames",
        "--no-ext-diff",
        "--no-textconv",
        "HEAD",
        "--",
    )
    if destination.exists() or destination.is_symlink():
        raise SourceError("standalone destination already exists; choose a new task-owned path")
    if destination.parent.resolve() != destination.parent or destination.is_relative_to(source):
        raise SourceError(
            "standalone destination must be outside the source, without linked parents"
        )
    # Reserve this exact directory; cleanup never follows a pre-existing destination.
    destination.mkdir(mode=0o700)
    owned = destination.stat().st_ino
    try:
        git(
            source,
            "-c",
            "protocol.file.allow=always",
            "clone",
            "--no-local",
            "--no-hardlinks",
            "--no-recurse-submodules",
            "--no-checkout",
            "--",
            str(source),
            str(destination),
        )
        if authority is None:
            git(destination, "remote", "remove", "origin")
        else:
            # A fetched main baseline need not be reachable from any local head.
            # Transfer that exact local ref before restoring the canonical origin;
            # no remote network or credentials enter this explicit fallback.
            git(
                destination,
                "-c",
                "protocol.file.allow=always",
                "fetch",
                "--no-tags",
                "--no-write-fetch-head",
                "--no-recurse-submodules",
                "--",
                str(source),
                "refs/remotes/origin/main:refs/remotes/origin/main",
            )
            git(destination, "remote", "set-url", "origin", authority["origin"])
        git(destination, "reset", "--mixed", str(before["head"]))
        if patch:
            # The canonical runner has no stdin channel; keep this private input in
            # the owned clone metadata, never in the reviewed working tree.
            with tempfile.NamedTemporaryFile(dir=destination / ".git") as staged:
                staged.write(patch)
                staged.flush()
                git(
                    destination, "apply", "--cached", "--binary", "--whitespace=nowarn", staged.name
                )
        members = before["files"]
        assert isinstance(members, dict)
        for name, entry in members.items():
            if entry["kind"] == "deleted":
                continue
            target = destination / name
            target.parent.mkdir(parents=True, exist_ok=True)
            content, mode, kind = member(source, name)
            if (
                kind != entry["kind"]
                or mode != entry["mode"]
                or hashlib.sha256(content).hexdigest() != entry["sha256"]
            ):
                raise SourceError("source changed during standalone copy")
            if kind == "symlink":
                target.symlink_to(os.fsdecode(content))
            else:
                descriptor = os.open(
                    target, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600
                )
                with os.fdopen(descriptor, "wb") as output:
                    output.write(content)
                    os.fchmod(output.fileno(), mode)
        after = inspect(destination)
        if identity(before) != identity(after) or identity(inspect(source)) != identity(before):
            raise SourceError("source changed or standalone bytes/modes/HEAD/index differ")
        if baseline(source) != authority or baseline(destination) != authority:
            raise SourceError("standalone default-branch baseline identity changed")
        return {
            "source": before,
            "standalone": after,
            "baseline": authority,
            "identity_preserved": True,
        }
    except BaseException:
        if (
            destination.is_dir()
            and not destination.is_symlink()
            and destination.stat().st_ino == owned
        ):
            shutil.rmtree(destination)
        raise


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("inspect", "standalone"))
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--destination", type=Path)
    options = parser.parse_args()
    try:
        if options.mode == "standalone":
            if options.destination is None:
                parser.error("standalone requires --destination")
            result = standalone(options.source, options.destination)
        else:
            if options.destination is not None:
                parser.error("inspect accepts only --source")
            result = inspect(options.source)
        print(json.dumps(result, sort_keys=True))
    except (OSError, ValueError, runner.RunCancelled) as error:
        category = getattr(error, "category", "source-refused")
        print(f"BoxFerry Git source check failed [{category}]: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
