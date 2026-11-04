#!/usr/bin/env python3
"""Stable interface-1 frontloader for the immutable shared-CI implementation.

This launcher is for local and read-only candidate validation. Privileged services
must execute their trusted workflow's immutable checkout, never this candidate file.
Only the fixed BoxFerry remote is accepted. The object cache is verified before a
fresh archive is extracted; candidate Git configuration and hooks are never loaded.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path, PurePosixPath
from typing import Any

REPOSITORY = "Strukturpiloten/boxferry"
REMOTE = "https://github.com/Strukturpiloten/boxferry.git"
INTERFACE = 1
CONFIG_KEYS = {"schema", "repository", "revision", "policy", "interface"}
BARE_CONFIG = "[core]\n\trepositoryformatversion = 0\n\tbare = true\n"
MAX_ARCHIVE_BYTES = 32 * 1024 * 1024
MAX_CACHE_BYTES = 128 * 1024 * 1024
# Interface 1 retains Cargo tool ARG metadata at this exact immutable DATA path.
# Never execute Dockerfile instructions or export the rest of .devcontainer.
CARGO_METADATA_PATH = ".devcontainer/Dockerfile"


class BootstrapError(ValueError):
    """Invalid bootstrap configuration or unavailable immutable implementation."""


def load_config(repository_root: Path) -> dict[str, Any]:
    """Read the narrow consumer manifest without permitting another remote."""
    path = repository_root / ".github/automation.json"
    try:
        if path.stat().st_size > 8192:
            raise BootstrapError("automation manifest exceeds 8192 bytes")
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise BootstrapError(f"cannot read automation manifest: {error}") from error
    if not isinstance(value, dict) or set(value) != CONFIG_KEYS:
        raise BootstrapError("automation manifest must contain exactly the interface-1 keys")
    if type(value["schema"]) is not int or value["schema"] != 1:
        raise BootstrapError("unsupported automation manifest schema")
    if type(value["interface"]) is not int or value["interface"] != INTERFACE:
        raise BootstrapError("unsupported shared automation interface")
    if value["repository"] != REPOSITORY:
        raise BootstrapError("shared automation repository must be Strukturpiloten/boxferry")
    if not isinstance(value["revision"], str) or not re.fullmatch(
        r"[0-9a-f]{40}", value["revision"]
    ):
        raise BootstrapError("shared automation revision must be an exact lowercase commit SHA")
    policy = value["policy"]
    if not isinstance(policy, str) or not policy or "\\" in policy:
        raise BootstrapError("policy must be a repository-relative POSIX path")
    parts = PurePosixPath(policy)
    if parts.is_absolute() or any(part in {".", ".."} for part in policy.split("/")):
        raise BootstrapError("policy must stay inside the consumer repository")
    if not (repository_root / policy).resolve().is_relative_to(repository_root.resolve()):
        raise BootstrapError("policy symlink escapes the consumer repository")
    return value


def git_environment() -> dict[str, str]:
    """Remove inherited Git overrides and disable all global/system Git config."""
    environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    environment.update(
        GIT_CONFIG_NOSYSTEM="1",
        GIT_CONFIG_GLOBAL=os.devnull,
        GIT_CONFIG_SYSTEM=os.devnull,
        GIT_TERMINAL_PROMPT="0",
        GIT_NO_REPLACE_OBJECTS="1",
    )
    return environment


def run_git(directory: Path, *arguments: str, binary: bool = False) -> bytes | str:
    """Run bounded Git operations only inside the dedicated, checked bare cache."""
    command = [
        "git",
        "--no-replace-objects",
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "maintenance.auto=false",
        "-c",
        "gc.auto=0",
        "-c",
        "protocol.file.allow=never",
        "-c",
        "protocol.ext.allow=never",
        "-C",
        str(directory),
        *arguments,
    ]
    try:
        result = subprocess.run(
            command,
            env=git_environment(),
            capture_output=True,
            check=False,
            timeout=120,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise BootstrapError(f"shared automation Git operation failed: {error}") from error
    if result.returncode:
        detail = result.stderr.decode("utf-8", errors="replace")[-4000:]
        raise BootstrapError(f"shared automation Git operation failed: {detail}")
    if binary:
        if len(result.stdout) > MAX_ARCHIVE_BYTES:
            raise BootstrapError("shared automation archive exceeds its size budget")
        return result.stdout
    return result.stdout.decode("utf-8", errors="strict").strip()


def validate_cache(cache: Path) -> None:
    """Reject mutable Git configuration, replacement refs, links and excess data."""
    if cache.is_symlink() or not cache.is_dir():
        raise BootstrapError("shared automation object cache must be a real directory")
    total = 0
    for files, entry in enumerate(cache.rglob("*"), start=1):
        if entry.is_symlink():
            raise BootstrapError("shared automation object cache contains a symlink")
        if files > 20000:
            raise BootstrapError("shared automation object cache exceeds its file budget")
        if entry.is_file():
            total += entry.stat().st_size
            if total > MAX_CACHE_BYTES:
                raise BootstrapError("shared automation object cache exceeds its size budget")
    if (cache / "config").read_text(encoding="utf-8") != BARE_CONFIG:
        raise BootstrapError("shared automation object cache has untrusted Git configuration")
    if (cache / "objects/info/alternates").exists() or (cache / "refs/replace").exists():
        raise BootstrapError("shared automation object cache contains object overrides")


def extract_archive(archive: Path, destination: Path) -> None:
    """Extract regular files/directories only, with finite path and size bounds."""
    with tarfile.open(archive, mode="r:") as source:
        members = source.getmembers()
        if len(members) > 5000 or sum(member.size for member in members) > MAX_ARCHIVE_BYTES:
            raise BootstrapError("shared automation archive exceeds extraction budgets")
        for member in members:
            parts = PurePosixPath(member.name)
            if (
                parts.is_absolute()
                or ".." in parts.parts
                or not parts.parts
                or (
                    parts.parts[0] not in {"automation", "renovate-presets"}
                    and not (
                        (member.name in {".devcontainer", ".devcontainer/"} and member.isdir())
                        or (member.name == CARGO_METADATA_PATH and member.isfile())
                    )
                )
                or not (member.isfile() or member.isdir())
            ):
                raise BootstrapError("shared automation archive contains an unsafe member")
            target = destination.joinpath(*parts.parts)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                stream = source.extractfile(member)
                if stream is None:
                    raise BootstrapError("shared automation archive member has no data")
                with stream, target.open("xb") as output:
                    shutil.copyfileobj(stream, output)
                target.chmod(
                    0o644
                    if member.name == CARGO_METADATA_PATH
                    else 0o755
                    if member.mode & 0o111
                    else 0o644
                )


def prepare_implementation(repository_root: Path, revision: str) -> Path:
    """Verify cached commit objects and reconstruct the exact pinned project."""
    root = repository_root / "target/boxferry-automation"
    for entry in (repository_root / "target", root):
        if entry.is_symlink():
            raise BootstrapError("shared automation cache must not follow symlinks")
        entry.mkdir(exist_ok=True)
    objects = root / f"{revision}.git"
    if not objects.exists():
        with tempfile.TemporaryDirectory(prefix="objects-", dir=root) as temporary:
            temporary_root = Path(temporary)
            run_git(temporary_root, "init", "--bare", "--template=", ".")
            (temporary_root / "config").write_text(BARE_CONFIG, encoding="utf-8")
            run_git(temporary_root, "fetch", "--depth=1", "--no-tags", REMOTE, revision)
            if run_git(temporary_root, "rev-parse", "FETCH_HEAD^{commit}") != revision:
                raise BootstrapError("shared automation fetch did not return the pinned commit")
            temporary_root.rename(objects)
    validate_cache(objects)
    run_git(objects, "fsck", "--full", "--strict", "--no-reflogs", "--no-dangling")
    if run_git(objects, "rev-parse", f"{revision}^{{commit}}") != revision:
        raise BootstrapError("shared automation cache does not contain the pinned commit")
    data = run_git(
        objects,
        "archive",
        "--format=tar",
        revision,
        "automation",
        "renovate-presets",
        CARGO_METADATA_PATH,
        binary=True,
    )
    if not isinstance(data, bytes):
        raise BootstrapError("shared automation archive was not binary")
    destination = root / revision
    if destination.is_symlink():
        raise BootstrapError("shared automation checkout must not be a symlink")
    with tempfile.TemporaryDirectory(prefix="archive-", dir=root) as temporary:
        temporary_root = Path(temporary)
        archive = temporary_root / "shared.tar"
        archive.write_bytes(data)
        extract_archive(archive, temporary_root)
        project = temporary_root / "automation"
        if not (project / "pyproject.toml").is_file() or not (project / "uv.lock").is_file():
            raise BootstrapError("pinned shared automation has no locked Python project")
        cargo_metadata = temporary_root / CARGO_METADATA_PATH
        if not cargo_metadata.is_file() or cargo_metadata.is_symlink():
            raise BootstrapError("pinned shared automation has no regular Cargo tool metadata")
        if destination.exists():
            shutil.rmtree(destination)
        destination.mkdir()
        project.rename(destination / "automation")
        presets = temporary_root / "renovate-presets"
        if not presets.is_dir():
            raise BootstrapError("pinned shared automation has no Renovate presets")
        presets.rename(destination / "renovate-presets")
        (destination / ".devcontainer").mkdir()
        cargo_metadata.rename(destination / CARGO_METADATA_PATH)
    return destination / "automation"


def main(argv: list[str] | None = None) -> int:
    """Load interface 1 from the verified immutable project and delegate."""
    arguments = sys.argv[1:] if argv is None else argv
    parser = argparse.ArgumentParser(description=__doc__, add_help=False)
    parser.add_argument("--implementation-checkout", type=Path)
    options, arguments = parser.parse_known_args(arguments)
    repository_root = Path(__file__).resolve().parent.parent
    try:
        config = load_config(repository_root)
        if options.implementation_checkout is not None:
            project = options.implementation_checkout.resolve() / "automation"
            if not (project / "pyproject.toml").is_file() or not (project / "uv.lock").is_file():
                raise BootstrapError("development override has no locked automation project")
            print(
                "Shared CI evidence: mutable development implementation override",
                file=sys.stderr,
            )
        else:
            project = prepare_implementation(repository_root, config["revision"])
            print(
                f"Shared CI implementation: {REPOSITORY}@{config['revision']}",
                file=sys.stderr,
            )
        implementation = project / "src/boxferry_ci/bootstrap.py"
        spec = importlib.util.spec_from_file_location(
            "_boxferry_verified_bootstrap", implementation
        )
        if spec is None or spec.loader is None:
            raise BootstrapError("pinned implementation has no bootstrap loader")
        module = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = module
        spec.loader.exec_module(module)
        run_project = getattr(module, "run_project", None)
        if not callable(run_project):
            raise BootstrapError("pinned implementation lacks interface-1 run_project")
        return run_project(repository_root, project, arguments)
    except (BootstrapError, OSError, ImportError) as error:
        print(f"Shared CI bootstrap failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
