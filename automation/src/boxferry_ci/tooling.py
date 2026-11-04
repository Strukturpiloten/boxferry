"""Install canonical integrity-pinned Linux development tools.

Local, CI and Dev Container wrappers call ``boxferry-ci tools --destination DIR
--profile core`` (Tombi, shfmt, ShellCheck, Hadolint), ``--profile website`` (all
six tools), or ``--tools lychee`` for an explicit subset. No shell evaluation,
PATH fallback, candidate configuration or version discovery occurs here.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import lzma
import os
import platform
import re
import shutil
import sys
import tarfile
import tempfile
import time
from collections.abc import Callable
from dataclasses import dataclass
from http.client import HTTPMessage, IncompleteRead
from pathlib import Path, PurePosixPath
from string import Formatter
from typing import IO, Any
from urllib.error import HTTPError
from urllib.parse import urlparse
from urllib.request import HTTPRedirectHandler, Request, build_opener

MANIFEST = Path(__file__).resolve().parents[2] / "tool-artifacts.json"
REPOSITORIES = {
    "tombi": "tombi-toml/tombi",
    "shfmt": "mvdan/sh",
    "shellcheck": "koalaman/shellcheck",
    "hadolint": "hadolint/hadolint",
    "actionlint": "rhysd/actionlint",
    "lychee": "lycheeverse/lychee",
}
CORE = ("tombi", "shfmt", "shellcheck", "hadolint")
WEBSITE = (*CORE, "actionlint", "lychee")
ARCHITECTURES = ("x86_64", "aarch64")
HOSTS = {"github.com", "release-assets.githubusercontent.com", "objects.githubusercontent.com"}
MAX_MANIFEST = 32 * 1024
MAX_DOWNLOAD = 64 * 1024 * 1024
MAX_UNPACKED = 128 * 1024 * 1024
MAX_BINARY = 64 * 1024 * 1024
MAX_MEMBERS = 4096
DOWNLOAD_SECONDS = 120
DOWNLOAD_ATTEMPTS = 4
RETRY_BACKOFF = (1, 2, 4)
INSTALL_SECONDS = 600
CHUNK = 1024 * 1024


class ToolError(ValueError):
    """Invalid canonical tool metadata or failed bounded installation."""


@dataclass(frozen=True)
class Asset:
    tool: str
    repository: str
    architecture: str
    tag: str
    digest: str
    filename: str
    member: str | None
    archive: str

    @property
    def url(self) -> str:
        return f"https://github.com/{self.repository}/releases/download/{self.tag}/{self.filename}"


def _unique(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise ToolError(f"tool metadata repeats key {key!r}")
        value[key] = item
    return value


def _path(value: str, *, directory: bool = False) -> str:
    if value.startswith("./"):
        value = value[2:]
    if directory and value in {"", "."}:
        return "."
    if (
        not value
        or len(value) > 512
        or "\\" in value
        or PurePosixPath(value).is_absolute()
        or any(part in {"", ".", ".."} for part in value.split("/"))
        or any(ord(char) < 32 or ord(char) == 127 for char in value)
    ):
        raise ToolError("tool archive path must be a bounded safe relative POSIX path")
    return value


def _template(value: object, version: str, *, filename: bool = False) -> str:
    if not isinstance(value, str) or len(value) > 512:
        raise ToolError("tool asset templates must be bounded strings")
    try:
        fields = list(Formatter().parse(value))
        if any(
            field not in {None, "version"} or spec or conversion
            for _, field, spec, conversion in fields
        ):
            raise ToolError("tool asset templates permit only the plain {version} field")
        rendered = _path(value.format(version=version))
    except ValueError as error:
        raise ToolError("invalid tool asset filename/member template") from error
    if filename and ("/" in rendered or not re.fullmatch(r"[A-Za-z0-9._+-]+", rendered)):
        raise ToolError("tool asset filename must be a single safe release-asset name")
    return rendered


def load_manifest(path: Path = MANIFEST) -> dict[tuple[str, str], Asset]:
    """Require one exact release per tool, with both supported architecture hashes."""
    try:
        if path.is_symlink() or not path.is_file() or path.stat().st_size > MAX_MANIFEST:
            raise ToolError("canonical tool metadata is linked, missing or oversized")
        raw = path.read_bytes()
        if len(raw) > MAX_MANIFEST:
            raise ToolError("canonical tool metadata exceeded its read budget")
        document = json.loads(raw, object_pairs_hook=_unique)
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ToolError("cannot read canonical tool metadata as bounded UTF-8 JSON") from error
    if (
        not isinstance(document, dict)
        or set(document) != {"schema", "provenance", "assets"}
        or type(document["schema"]) is not int
        or document["schema"] != 1
        or not isinstance(document["provenance"], dict)
        or not isinstance(document["assets"], list)
        or len(document["assets"]) != len(REPOSITORIES) * len(ARCHITECTURES)
    ):
        raise ToolError("canonical tool metadata must contain the complete schema-1 inventory")
    inventory: dict[tuple[str, str], Asset] = {}
    versions: dict[str, str] = {}
    for row in document["assets"]:
        if not isinstance(row, dict) or set(row) != {
            "tool",
            "repository",
            "architecture",
            "filenameTemplate",
            "memberTemplate",
            "archive",
            "currentVersion",
            "sha256",
        }:
            raise ToolError("tool asset row has missing or unknown fields")
        tool, architecture, tag = row["tool"], row["architecture"], row["currentVersion"]
        if (
            not isinstance(tool, str)
            or tool not in REPOSITORIES
            or row["repository"] != REPOSITORIES[tool]
            or not isinstance(architecture, str)
            or architecture not in ARCHITECTURES
            or not isinstance(tag, str)
        ):
            raise ToolError("tool row has an unsupported tool, origin, architecture or release")
        prefix = "lychee-v" if tool == "lychee" else "v"
        if not re.fullmatch(re.escape(prefix) + r"[0-9]+\.[0-9]+\.[0-9]+", tag):
            raise ToolError(f"{tool} currentVersion must retain its exact {prefix} release tag")
        if tool in versions and versions[tool] != tag:
            raise ToolError(f"{tool} versions differ across architectures; reject partial updates")
        versions[tool] = tag
        digest = row["sha256"]
        if not isinstance(digest, str) or not re.fullmatch(r"[a-f0-9]{64}", digest):
            raise ToolError(f"{tool} must have an exact lowercase SHA256")
        archive = row["archive"]
        if not isinstance(archive, str) or archive not in {"binary", "tar.gz", "tar.xz"}:
            raise ToolError(f"{tool} uses an unsupported archive type")
        version = tag.removeprefix(prefix)
        filename = _template(row["filenameTemplate"], version, filename=True)
        member = (
            None if row["memberTemplate"] is None else _template(row["memberTemplate"], version)
        )
        if (archive == "binary") != (member is None):
            raise ToolError(f"{tool} raw binaries cannot have archive member paths")
        if (tool, architecture) in inventory:
            raise ToolError(f"{tool} repeats architecture {architecture}")
        inventory[tool, architecture] = Asset(
            tool, REPOSITORIES[tool], architecture, tag, digest, filename, member, archive
        )
    if set(inventory) != {(tool, arch) for tool in REPOSITORIES for arch in ARCHITECTURES}:
        raise ToolError("tool metadata is missing an architecture or contains a duplicate")
    return inventory


def host_architecture() -> str:
    if platform.system() != "Linux":
        raise ToolError("canonical downloaded tools support Linux only")
    machine = platform.machine()
    if machine in {"x86_64", "amd64"}:
        return "x86_64"
    if machine in {"aarch64", "arm64"}:
        return "aarch64"
    raise ToolError("unsupported Linux tool architecture; expected x86_64 or aarch64")


def _https(url: str) -> None:
    try:
        parsed = urlparse(url)
        permitted = (
            parsed.scheme == "https"
            and parsed.hostname in HOSTS
            and parsed.port in {None, 443}
            and not parsed.username
            and not parsed.password
            and not parsed.fragment
        )
    except ValueError as error:
        raise ToolError("invalid official release download URL") from error
    if not permitted:
        raise ToolError("tool download redirected outside reviewed official HTTPS hosts")


class OfficialRedirect(HTTPRedirectHandler):
    """Follow at most four redirects among official HTTPS asset hosts, without auth."""

    max_redirections = 4
    max_repeats = 2

    def __init__(self, deadline: float | None = None) -> None:
        super().__init__()
        self.deadline = deadline

    def redirect_request(
        self,
        req: Request,
        fp: IO[bytes],
        code: int,
        msg: str,
        headers: HTTPMessage,
        newurl: str,
    ) -> Request | None:
        _https(newurl)
        if self.deadline is not None:
            remaining = self.deadline - time.monotonic()
            if remaining <= 0:
                raise ToolError("official asset redirect exceeded its download deadline")
            # urllib forwards the original request timeout to the next hop.
            req.timeout = min(20, remaining)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


class _TransientDownload(Exception):
    """A network operation failed; the detail contains only an exception class."""


def _remove_partial(destination: Path, identity: tuple[int, int] | None) -> None:
    """Remove only the file created by this attempt, never an existing destination."""
    if identity is None:
        return
    try:
        current = destination.lstat()
        if (current.st_dev, current.st_ino) == identity:
            destination.unlink()
    except FileNotFoundError:
        pass


def _download_attempt(asset: Asset, destination: Path, deadline: float) -> None:
    if destination.exists() or destination.is_symlink():
        raise ToolError("download destination appeared between attempts; refusing to replace it")
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise ToolError("official asset download deadline exhausted")
    request = Request(asset.url, headers={"User-Agent": "BoxFerry-canonical-tools/1"})
    try:
        response = build_opener(OfficialRedirect(deadline)).open(
            request, timeout=min(20, remaining)
        )
    except HTTPError:
        raise
    except (OSError, IncompleteRead) as error:
        raise _TransientDownload(type(error).__name__) from None
    identity: tuple[int, int] | None = None
    completed = False
    try:
        with response:
            _https(response.geturl())
            length = response.headers.get("Content-Length")
            if length is not None and (not length.isdecimal() or int(length) > MAX_DOWNLOAD):
                raise ToolError("release asset Content-Length exceeds its download budget")
            with destination.open("xb") as output:
                created = os.fstat(output.fileno())
                identity = (created.st_dev, created.st_ino)
                digest = hashlib.sha256()
                consumed = 0
                while True:
                    try:
                        chunk = response.read(CHUNK)
                    except HTTPError:
                        raise
                    except (OSError, IncompleteRead) as error:
                        raise _TransientDownload(type(error).__name__) from None
                    if time.monotonic() > deadline:
                        raise ToolError("release asset exceeded its download time budget")
                    if not chunk:
                        break
                    consumed += len(chunk)
                    if consumed > MAX_DOWNLOAD:
                        raise ToolError("release asset exceeded its download size budget")
                    digest.update(chunk)
                    output.write(chunk)
                if length is not None and consumed != int(length):
                    raise ToolError("release asset did not match its declared Content-Length")
            if consumed == 0 or digest.hexdigest() != asset.digest:
                raise ToolError("release asset SHA256 mismatch or empty download")
        if time.monotonic() > deadline:
            raise ToolError("release asset exceeded its download time budget")
        completed = True
    finally:
        if not completed:
            _remove_partial(destination, identity)


def download(asset: Asset, destination: Path) -> None:
    """Retry only transient acquisition failures within one finite download deadline."""
    _https(asset.url)
    if destination.exists() or destination.is_symlink():
        raise ToolError("download destination already exists; refusing to replace or remove it")
    deadline = time.monotonic() + DOWNLOAD_SECONDS
    for attempt in range(1, DOWNLOAD_ATTEMPTS + 1):
        try:
            _download_attempt(asset, destination, deadline)
            return
        except HTTPError as error:
            detail = f"HTTP {error.code}"
            transient = error.code in {408, 429} or 500 <= error.code <= 599
            error.close()
            if not transient:
                raise ToolError(f"official asset acquisition failed with {detail}") from None
        except _TransientDownload as error:
            detail = f"network {error}"
        except OSError as error:
            raise ToolError(
                f"official asset local file operation failed: {type(error).__name__}"
            ) from None
        if attempt == DOWNLOAD_ATTEMPTS:
            raise ToolError(f"official asset acquisition failed after {attempt} attempts: {detail}")
        delay = RETRY_BACKOFF[attempt - 1]
        if deadline - time.monotonic() <= delay:
            raise ToolError(f"official asset download deadline exhausted; last cause: {detail}")
        time.sleep(delay)


def _expand(source: Path, archive: str, destination: Path) -> None:
    """Bound decompression before tar metadata parsing (including extended headers)."""
    total = 0
    deadline = time.monotonic() + 60

    def write(chunk: bytes, output: IO[bytes]) -> None:
        nonlocal total
        total += len(chunk)
        if total > MAX_UNPACKED or time.monotonic() > deadline:
            raise ToolError("tool archive exceeded its unpacked size/time budget")
        output.write(chunk)

    try:
        with destination.open("xb") as output:
            if archive == "tar.gz":
                with gzip.open(source, "rb") as incoming:
                    while chunk := incoming.read(CHUNK):
                        write(chunk, output)
            else:
                # XZ dictionaries can be large independently of the archive's
                # expanded length; bound both allocation and emitted bytes.
                decoder = lzma.LZMADecompressor(memlimit=MAX_UNPACKED)
                with source.open("rb") as incoming:
                    while packed := incoming.read(CHUNK):
                        write(decoder.decompress(packed, max_length=CHUNK), output)
                        while not decoder.eof and not decoder.needs_input:
                            write(decoder.decompress(b"", max_length=CHUNK), output)
                        if decoder.eof:
                            if decoder.unused_data or incoming.read(1):
                                raise ToolError("tool XZ archive has unexpected trailing data")
                            break
                if not decoder.eof:
                    raise ToolError("tool XZ archive is truncated")
    except (OSError, EOFError, lzma.LZMAError) as error:
        raise ToolError(f"tool archive decompression failed: {type(error).__name__}") from error


def extract(asset: Asset, source: Path, destination: Path) -> None:
    """Validate all tar paths/types; copy only the explicitly selected executable."""
    if source.is_symlink() or not source.is_file() or not 0 < source.stat().st_size <= MAX_DOWNLOAD:
        raise ToolError("downloaded tool must be a bounded nonempty regular file")
    if asset.archive == "binary":
        shutil.copyfile(source, destination, follow_symlinks=False)
        return
    expanded = source.with_suffix(".expanded.tar")
    _expand(source, asset.archive, expanded)
    seen: set[str] = set()
    found = False
    total = 0
    try:
        with tarfile.open(expanded, mode="r:") as archive:
            for count, member in enumerate(archive, start=1):
                path = _path(member.name, directory=member.isdir())
                if count > MAX_MEMBERS or path in seen:
                    raise ToolError("tool archive exceeds its member budget or repeats a path")
                seen.add(path)
                if not (member.isfile() or member.isdir()) or member.size < 0:
                    raise ToolError("tool archive contains a link, device or unsupported member")
                if member.isdir() and member.size:
                    raise ToolError("tool archive directory contains unexpected bytes")
                total += member.size
                if total > MAX_UNPACKED:
                    raise ToolError("tool archive exceeds its declared file size budget")
                if path == asset.member:
                    if not member.isfile() or not 0 < member.size <= MAX_BINARY:
                        raise ToolError("tool executable must be a bounded nonempty regular file")
                    incoming = archive.extractfile(member)
                    if incoming is None:
                        raise ToolError("tool archive lacks its selected executable data")
                    with incoming, destination.open("xb") as output:
                        shutil.copyfileobj(incoming, output, length=CHUNK)
                    if destination.stat().st_size != member.size:
                        raise ToolError("tool executable size differs from archive metadata")
                    found = True
        if not found:
            raise ToolError("tool archive is missing its exact selected executable member")
    except (OSError, tarfile.TarError, EOFError) as error:
        raise ToolError(f"tool archive is invalid: {type(error).__name__}") from error
    finally:
        expanded.unlink(missing_ok=True)


Downloader = Callable[[Asset, Path], None]


def install(
    destination: Path,
    tools: tuple[str, ...],
    *,
    manifest: Path = MANIFEST,
    architecture: str | None = None,
    downloader: Downloader = download,
) -> list[Asset]:
    """Stage every selected verified binary, then atomically replace each tool path."""
    inventory = load_manifest(manifest)
    architecture = host_architecture() if architecture is None else architecture
    if architecture not in ARCHITECTURES:
        raise ToolError("unsupported canonical tool architecture")
    if (
        not tools
        or len(set(tools)) != len(tools)
        or any(tool not in REPOSITORIES for tool in tools)
    ):
        raise ToolError("select a nonempty unique subset of canonical tools")
    destination = destination.absolute()
    if any(parent.is_symlink() for parent in (destination, *destination.parents)):
        raise ToolError("tool destination and its parents must not be symbolic links")
    if destination.exists() and not destination.is_dir():
        raise ToolError("tool destination must be a directory")
    destination.mkdir(parents=True, exist_ok=True)
    selected = [inventory[tool, architecture] for tool in tools]
    deadline = time.monotonic() + INSTALL_SECONDS
    with tempfile.TemporaryDirectory(prefix=".boxferry-tools-", dir=destination) as temporary:
        staging = Path(temporary)
        for asset in selected:
            try:
                if time.monotonic() > deadline:
                    raise ToolError("selected tool installation exceeded its total time budget")
                source = staging / (asset.tool + ".download")
                downloader(asset, source)
                if time.monotonic() > deadline:
                    raise ToolError("selected tool installation exceeded its total time budget")
                # Keep integrity independent of the transport implementation.
                if (
                    source.is_symlink()
                    or not source.is_file()
                    or source.stat().st_size > MAX_DOWNLOAD
                ):
                    raise ToolError("download is not a bounded regular file")
                with source.open("rb") as incoming:
                    if hashlib.file_digest(incoming, "sha256").hexdigest() != asset.digest:
                        raise ToolError("download SHA256 differs from canonical metadata")
                extract(asset, source, staging / asset.tool)
                if time.monotonic() > deadline:
                    raise ToolError("selected tool installation exceeded its total time budget")
                (staging / asset.tool).chmod(0o755)
            except (ToolError, OSError) as error:
                raise ToolError(
                    f"{asset.tool} {asset.tag} {asset.architecture} ({asset.filename}): {error}"
                ) from error
        for asset in selected:
            target = destination / asset.tool
            if target.is_symlink() or (target.exists() and not target.is_file()):
                raise ToolError(
                    f"{asset.tool}: refusing a linked or non-regular installation target"
                )
        if time.monotonic() > deadline:
            raise ToolError(
                "selected tool installation exceeded its total time budget before replacement"
            )
        for asset in selected:
            os.replace(staging / asset.tool, destination / asset.tool)
    return selected


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--destination", required=True, type=Path)
    selection = parser.add_mutually_exclusive_group(required=True)
    selection.add_argument("--profile", choices=("core", "website"))
    selection.add_argument("--tools", nargs="+", choices=WEBSITE)
    options = parser.parse_args(argv)
    try:
        tools = (
            tuple(options.tools)
            if options.tools
            else CORE
            if options.profile == "core"
            else WEBSITE
        )
        assets = install(options.destination, tools)
        for asset in assets:
            print(f"Installed {asset.tool} {asset.tag} ({asset.architecture}; SHA256 verified).")
        return 0
    except (ToolError, OSError) as error:
        print(f"Canonical tool installation failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
