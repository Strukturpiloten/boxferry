#!/usr/bin/env python3
"""Canonical shared-CI bootstrap and locked project implementation.

This launcher is for local and read-only candidate validation. Privileged services
must execute their trusted workflow's immutable checkout, never this candidate file.
Only the fixed BoxFerry remote is accepted. The object cache is verified before a
fresh archive is extracted; candidate Git configuration and hooks are never loaded.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import tomllib
from http.client import HTTPMessage
from pathlib import Path, PurePosixPath
from typing import IO, Any
from urllib.parse import urlparse
from urllib.request import HTTPRedirectHandler, Request, build_opener

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
MAX_UV_ARCHIVE_BYTES = 32 * 1024 * 1024
MAX_UV_EXTRACTED_BYTES = 64 * 1024 * 1024
UV_ASSETS = {
    "x86_64": "uv-x86_64-unknown-linux-gnu.tar.gz",
    "aarch64": "uv-aarch64-unknown-linux-gnu.tar.gz",
}
UV_DOWNLOAD_HOSTS = {
    "github.com",
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
}


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


def uv_manifest(project: Path) -> tuple[str, dict[str, str]]:
    """Require one canonical uv version and complete, matching integrity records."""
    try:
        manifest = project / "pyproject.toml"
        metadata = project / "uv-artifacts.json"
        if manifest.stat().st_size > 65536 or metadata.stat().st_size > 8192:
            raise BootstrapError("shared uv configuration exceeds its size budget")
        configuration = tomllib.loads(manifest.read_text(encoding="utf-8"))
        tool = configuration.get("tool")
        uv = tool.get("uv") if isinstance(tool, dict) else None
        required = uv.get("required-version") if isinstance(uv, dict) else None
        if not isinstance(required, str) or not re.fullmatch(r"==[0-9]+\.[0-9]+\.[0-9]+", required):
            raise BootstrapError(
                "shared pyproject tool.uv.required-version must pin one exact release"
            )
        version = required[2:]
        value = json.loads(metadata.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise BootstrapError(f"cannot read shared uv configuration: {error}") from error
    if (
        not isinstance(value, dict)
        or set(value) != {"schema", "assets"}
        or type(value["schema"]) is not int
        or value["schema"] != 1
    ):
        raise BootstrapError("unsupported shared uv artifact metadata schema")
    assets = value["assets"]
    if not isinstance(assets, list) or len(assets) != len(UV_ASSETS):
        raise BootstrapError("shared uv metadata must cover both supported Linux architectures")
    checksums: dict[str, str] = {}
    for asset in assets:
        if not isinstance(asset, dict) or set(asset) != {"filename", "currentVersion", "sha256"}:
            raise BootstrapError("shared uv artifact record has unexpected fields")
        filename, digest = asset["filename"], asset["sha256"]
        if (
            not isinstance(filename, str)
            or filename not in UV_ASSETS.values()
            or filename in checksums
        ):
            raise BootstrapError("shared uv artifact filename is unsupported or duplicated")
        if asset["currentVersion"] != version:
            raise BootstrapError(
                "shared uv artifact version disagrees with canonical pyproject pin"
            )
        if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
            raise BootstrapError("shared uv artifact SHA256 must be a lowercase integrity digest")
        checksums[filename] = digest
    return version, checksums


def uv_host_asset() -> str:
    """Select only the explicitly supported Linux host and architecture."""
    system, machine = platform.system(), platform.machine()
    if system != "Linux" or machine not in UV_ASSETS:
        raise BootstrapError(
            f"shared uv bootstrap supports Linux x86_64 and aarch64; observed {system}/{machine}. "
            "Use the supported development container or a supported Linux host."
        )
    return UV_ASSETS[machine]


def uv_version_matches(executable: Path, version: str) -> bool:
    """An ambient executable is usable only after an exact, bounded version check."""
    try:
        result = subprocess.run(
            [str(executable), "--version"],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            check=False,
            timeout=10,
        )
    except (OSError, subprocess.TimeoutExpired):
        return False
    return (
        result.returncode == 0
        and len(result.stdout) <= 256
        and re.fullmatch(
            rb"uv " + re.escape(version.encode()) + rb"(?: \([^\r\n]*\))?\r?\n?", result.stdout
        )
        is not None
    )


def safe_cache_directory(path: Path) -> Path:
    """Reject links in every cache component before creating local directories."""
    absolute = path.absolute()
    if ".." in absolute.parts:
        raise BootstrapError("shared uv cache path must not contain parent traversal")
    for directory in (*reversed(absolute.parents), absolute):
        if directory.is_symlink():
            raise BootstrapError("shared uv cache must not follow directory symlinks")
        if directory.exists() and not directory.is_dir():
            raise BootstrapError("shared uv cache component is not a directory")
        directory.mkdir(exist_ok=True)
    return absolute


def bounded_sha256(path: Path, limit: int) -> str:
    """Hash a regular file under a finite byte bound; never trust sidecar hashes."""
    if path.is_symlink() or not path.is_file() or path.stat().st_size > limit:
        raise BootstrapError("shared uv cache contains a linked, non-regular or oversized file")
    digest = hashlib.sha256()
    consumed = 0
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            consumed += len(chunk)
            if consumed > limit:
                raise BootstrapError("shared uv cache file exceeded its size budget while reading")
            digest.update(chunk)
    return digest.hexdigest()


class UvArchiveRedirect(HTTPRedirectHandler):
    """Follow only HTTPS redirects to official release-asset hosts, without auth."""

    def redirect_request(
        self,
        req: Request,
        fp: IO[bytes],
        code: int,
        msg: str,
        headers: HTTPMessage,
        newurl: str,
    ) -> Request | None:
        location = urlparse(newurl)
        if (
            location.scheme != "https"
            or location.hostname not in UV_DOWNLOAD_HOSTS
            or location.username
            or location.password
            or location.port not in {None, 443}
        ):
            raise BootstrapError("official uv archive redirected outside reviewed HTTPS hosts")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def download_uv_archive(version: str, filename: str, destination: Path) -> None:
    """Download bounded official bytes; the caller checks their pinned SHA256."""
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version) or filename not in UV_ASSETS.values():
        raise BootstrapError("invalid canonical uv release or asset identity")
    request = Request(
        f"https://github.com/astral-sh/uv/releases/download/{version}/{filename}",
        headers={"User-Agent": "BoxFerry-shared-uv-bootstrap/1"},
    )
    deadline = time.monotonic() + 120
    try:
        with (
            build_opener(UvArchiveRedirect).open(request, timeout=30) as response,
            destination.open("xb") as output,
        ):
            length = response.headers.get("Content-Length")
            if length is not None and (
                not length.isdecimal() or int(length) > MAX_UV_ARCHIVE_BYTES
            ):
                raise BootstrapError("official uv archive exceeds its compressed size budget")
            consumed = 0
            while chunk := response.read(1024 * 1024):
                consumed += len(chunk)
                if consumed > MAX_UV_ARCHIVE_BYTES or time.monotonic() > deadline:
                    raise BootstrapError(
                        "official uv archive exceeded its download size/time budget"
                    )
                output.write(chunk)
    except OSError as error:
        raise BootstrapError(
            f"official uv archive acquisition failed: {type(error).__name__}"
        ) from error


def extract_uv_binary(archive: Path, filename: str, destination: Path) -> str:
    """Validate every official tar member and extract only its regular uv binary."""
    root = filename.removesuffix(".tar.gz")
    expected = {root, f"{root}/uv", f"{root}/uvx"}
    seen: set[str] = set()
    total = 0
    try:
        with tarfile.open(archive, mode="r|gz") as source:
            for member in source:
                if member.name not in expected or member.name in seen:
                    raise BootstrapError(
                        "official uv archive contains an unexpected or duplicate path"
                    )
                seen.add(member.name)
                if member.name == root:
                    if not member.isdir() or member.size != 0:
                        raise BootstrapError("official uv archive root must be an empty directory")
                    continue
                if not member.isfile() or member.size <= 0:
                    raise BootstrapError(
                        "official uv archive binaries must be regular, nonempty files"
                    )
                total += member.size
                if total > MAX_UV_EXTRACTED_BYTES:
                    raise BootstrapError("official uv archive exceeds its extracted size budget")
                if member.name.endswith("/uv"):
                    stream = source.extractfile(member)
                    if stream is None:
                        raise BootstrapError("official uv archive has no uv binary data")
                    with stream, destination.open("xb") as output:
                        shutil.copyfileobj(stream, output, length=1024 * 1024)
        if seen != expected:
            raise BootstrapError("official uv archive is missing required members")
    except (OSError, tarfile.TarError, EOFError) as error:
        raise BootstrapError(f"official uv archive is invalid: {type(error).__name__}") from error
    destination.chmod(0o755)
    return bounded_sha256(destination, MAX_UV_EXTRACTED_BYTES)


def ensure_uv(project: Path, cache_root: Path, *, prefer_path: bool = True) -> Path:
    """Use verified matching uv, or reconstruct it from its official pinned archive."""
    version, checksums = uv_manifest(project)
    filename = uv_host_asset()
    if prefer_path:
        ambient = shutil.which("uv")
        if ambient is not None and uv_version_matches(Path(ambient), version):
            return Path(ambient).absolute()
    cache = safe_cache_directory(cache_root) / version / filename.removesuffix(".tar.gz")
    safe_cache_directory(cache)
    archive = cache / filename
    expected = checksums[filename]
    if archive.exists() or archive.is_symlink():
        if bounded_sha256(archive, MAX_UV_ARCHIVE_BYTES) != expected:
            raise BootstrapError(
                "cached uv archive SHA256 disagrees with pinned metadata; "
                "remove the archive to reacquire it"
            )
    else:
        with tempfile.TemporaryDirectory(prefix="download-", dir=cache) as temporary:
            downloaded = Path(temporary) / filename
            download_uv_archive(version, filename, downloaded)
            if bounded_sha256(downloaded, MAX_UV_ARCHIVE_BYTES) != expected:
                raise BootstrapError("downloaded uv archive SHA256 disagrees with pinned metadata")
            downloaded.replace(archive)
    executable = cache / "uv"
    if executable.is_symlink():
        raise BootstrapError("cached uv executable must not be a symlink")
    # Extract again from authenticated archive bytes on every cache reuse. A
    # mutable executable or matching attacker-written hash file cannot authorize it.
    with tempfile.TemporaryDirectory(prefix="binary-", dir=cache) as temporary:
        extracted = Path(temporary) / "uv"
        digest = extract_uv_binary(archive, filename, extracted)
        if not executable.exists() or bounded_sha256(executable, MAX_UV_EXTRACTED_BYTES) != digest:
            extracted.replace(executable)
        else:
            executable.chmod(0o755)
    if not uv_version_matches(executable, version):
        raise BootstrapError("verified official uv binary does not report the canonical version")
    return executable


def install_uv_cli(arguments: list[str]) -> int:
    """Install the same local tool for shared GitHub actions without another pin."""
    parser = argparse.ArgumentParser(description="Acquire canonical integrity-verified Linux uv")
    parser.add_argument("--project", type=Path, required=True)
    parser.add_argument("--cache-root", type=Path, required=True)
    parser.add_argument("--github-path", type=Path)
    parser.add_argument("--no-path-uv", action="store_true")
    options = parser.parse_args(arguments)
    try:
        executable = ensure_uv(
            options.project, options.cache_root, prefer_path=not options.no_path_uv
        )
        if options.github_path is not None:
            directory = str(executable.parent)
            if "\n" in directory or "\r" in directory:
                raise BootstrapError(
                    "verified uv path contains an environment-file control character"
                )
            with options.github_path.open("a", encoding="utf-8") as output:
                output.write(directory + "\n")
        print(executable)
        return 0
    except (BootstrapError, OSError) as error:
        print(f"Shared uv bootstrap failed: {error}", file=sys.stderr)
        return 2


def forwarded_arguments(arguments: list[str], repository_root: Path, policy: str) -> list[str]:
    """Supply consumer defaults without replacing explicit trusted-policy arguments."""
    result = list(arguments)
    if not result or result[0] not in {"plan", "gate", "run", "run-local"}:
        return result
    if not any(
        item == "--repository-root" or item.startswith("--repository-root=") for item in result
    ):
        result.extend(["--repository-root", str(repository_root)])
    if not any(item == "--config" or item.startswith("--config=") for item in result):
        result.extend(["--config", str(repository_root / policy)])
    return result


def run_project(repository_root: Path, project: Path, arguments: list[str]) -> int:
    """Run the selected project with its own verified uv and locked CLI."""
    try:
        config = load_config(repository_root)
        arguments = forwarded_arguments(arguments, repository_root, config["policy"])
        executable = ensure_uv(project, repository_root / "target/boxferry-automation/tools")
        command = [
            str(executable),
            "run",
            "--locked",
            "--project",
            str(project),
            "--directory",
            str(repository_root),
            "python",
            "-m",
            "boxferry_ci",
            *arguments,
        ]
        environment = os.environ.copy()
        environment["PATH"] = str(executable.parent) + os.pathsep + environment.get("PATH", "")
        return subprocess.call(command, env=environment)
    except (BootstrapError, OSError) as error:
        print(f"Shared CI bootstrap failed: {error}", file=sys.stderr)
        return 2


def main(argv: list[str] | None = None) -> int:
    """Resolve the pinned shared implementation and invoke its locked CLI."""
    arguments = sys.argv[1:] if argv is None else argv
    if arguments and arguments[0] == "install-uv":
        return install_uv_cli(arguments[1:])
    parser = argparse.ArgumentParser(description=__doc__, add_help=False)
    parser.add_argument("--implementation-checkout", type=Path)
    options, arguments = parser.parse_known_args(arguments)
    # Both canonical locations (templates/ and scripts/) are one level below root.
    repository_root = Path(__file__).resolve().parent.parent
    try:
        config = load_config(repository_root)
        if options.implementation_checkout is not None:
            project = options.implementation_checkout.resolve() / "automation"
            if not (project / "pyproject.toml").is_file() or not (project / "uv.lock").is_file():
                raise BootstrapError("development override has no locked automation project")
            print(
                "Shared CI evidence: mutable development implementation override", file=sys.stderr
            )
        else:
            project = prepare_implementation(repository_root, config["revision"])
            print(f"Shared CI implementation: {REPOSITORY}@{config['revision']}", file=sys.stderr)
        return run_project(repository_root, project, arguments)
    except (BootstrapError, OSError) as error:
        print(f"Shared CI bootstrap failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
