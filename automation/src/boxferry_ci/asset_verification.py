"""Verify every canonical candidate archive as inert bytes, on both architectures."""

from __future__ import annotations

import argparse
import signal
import sys
import tempfile
import threading
import time
from collections.abc import Callable, Generator
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path

from boxferry_ci import admission, bootstrap, tooling
from boxferry_ci.contracts import ContractError, exact_sha

SOURCES = {
    "automation/tool-artifacts.json": tooling.MAX_MANIFEST,
    "automation/uv-artifacts.json": 8192,
    "automation/pyproject.toml": 65536,
}
TOTAL_SECONDS = 600
MAX_TOTAL_BYTES = 12 * tooling.MAX_DOWNLOAD + 2 * bootstrap.MAX_UV_ARCHIVE_BYTES


class VerificationError(ValueError):
    """Candidate archive identity, integrity, acquisition or budget failed."""


@dataclass(frozen=True)
class CandidateAsset:
    tool: str
    architecture: str
    version: str
    filename: str
    digest: str
    url: str
    limit: int
    native: tooling.Asset | None = None

    @property
    def label(self) -> str:
        return f"{self.tool} {self.version} {self.architecture} ({self.filename})"


def snapshot(root: Path, destination: Path, candidate_sha: str | None) -> None:
    """Read only exact regular Git blobs, or an explicitly selected working tree."""
    root = root.resolve()
    if candidate_sha is not None:
        exact_sha(candidate_sha, "canonical asset candidate SHA")
    for relative, limit in SOURCES.items():
        try:
            if candidate_sha is not None:
                content = admission._tracked_blob(
                    root, candidate_sha, relative, admission.read_git, limit
                )
            else:
                source = root / relative
                if any(path.is_symlink() for path in (source, *source.parents) if path != root):
                    raise VerificationError("working-tree metadata must not follow links")
                if not source.is_file() or source.stat().st_size > limit:
                    raise VerificationError("working-tree metadata is missing or oversized")
                content = source.read_bytes()
                if len(content) > limit:
                    raise VerificationError("working-tree metadata exceeded its read budget")
        except (OSError, admission.AdmissionError) as error:
            raise VerificationError(
                f"cannot read canonical asset source {relative}: {error}"
            ) from error
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content)


def inventory(root: Path) -> list[CandidateAsset]:
    """Reuse canonical parsers and construct only their reviewed official URLs."""
    assets = [
        CandidateAsset(
            value.tool,
            value.architecture,
            value.tag,
            value.filename,
            value.digest,
            value.url,
            tooling.MAX_DOWNLOAD,
            value,
        )
        for value in tooling.load_manifest(root / "automation/tool-artifacts.json").values()
    ]
    version, checksums = bootstrap.uv_manifest(root / "automation")
    for architecture, filename in bootstrap.UV_ASSETS.items():
        assets.append(
            CandidateAsset(
                "uv",
                architecture,
                version,
                filename,
                checksums[filename],
                f"https://github.com/astral-sh/uv/releases/download/{version}/{filename}",
                bootstrap.MAX_UV_ARCHIVE_BYTES,
            )
        )
    return assets


@contextmanager
def batch_budget() -> Generator[float]:
    """Enforce one wall-clock deadline, including blocking reads and Git access."""
    if (
        not hasattr(signal, "SIGALRM")
        or not callable(getattr(signal, "setitimer", None))
        or threading.current_thread() is not threading.main_thread()
    ):
        raise VerificationError(
            "canonical asset verification requires a main-thread wall-clock guard"
        )
    start = time.monotonic()
    previous_handler = signal.getsignal(signal.SIGALRM)
    previous_timer = signal.getitimer(signal.ITIMER_REAL)
    if previous_timer[0]:
        raise VerificationError("canonical asset verifier requires its own process time budget")

    def exhausted(_number: int, _frame: object) -> None:
        raise VerificationError("canonical asset verification exceeded its total time budget")

    signal.signal(signal.SIGALRM, exhausted)
    try:
        signal.setitimer(signal.ITIMER_REAL, TOTAL_SECONDS)
        yield start + TOTAL_SECONDS
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous_handler)


def verify(
    root: Path,
    candidate_sha: str | None,
    *,
    tool_download: Callable[[tooling.Asset, Path], None] = tooling.download,
    uv_download: Callable[[str, str, Path], None] = bootstrap.download_uv_archive,
) -> tuple[int, int, int]:
    """Fetch each exact URL/digest identity once; never extract, chmod or execute it."""
    verified: set[tuple[str, str]] = set()
    origins: dict[str, str] = {}
    consumed = 0
    with (
        batch_budget() as deadline,
        tempfile.TemporaryDirectory(prefix="canonical-assets-") as temp,
    ):
        temporary = Path(temp)
        snapshot(root, temporary, candidate_sha)
        assets = inventory(temporary)
        for index, asset in enumerate(assets):
            identity = (asset.url, asset.digest)
            if asset.url in origins and origins[asset.url] != asset.digest:
                raise VerificationError(f"{asset.label}: conflicting integrity records for one URL")
            origins[asset.url] = asset.digest
            if identity in verified:
                continue
            path = temporary / f"asset-{index}.download"
            try:
                if time.monotonic() >= deadline:
                    raise VerificationError("total time budget exhausted")
                if asset.native is not None:
                    tool_download(asset.native, path)
                else:
                    uv_download(asset.version, asset.filename, path)
                if time.monotonic() >= deadline:
                    raise VerificationError("total time budget exhausted")
                actual = bootstrap.bounded_sha256(path, asset.limit)
                if actual != asset.digest:
                    raise VerificationError(
                        "official release asset SHA256 differs from candidate pin"
                    )
                consumed += path.stat().st_size
                if consumed > MAX_TOTAL_BYTES:
                    raise VerificationError("total download byte budget exhausted")
            except (
                OSError,
                tooling.ToolError,
                bootstrap.BootstrapError,
                VerificationError,
            ) as error:
                raise VerificationError(f"{asset.label}: {error}") from error
            finally:
                path.unlink(missing_ok=True)
            verified.add(identity)
    return len(assets), len(verified), consumed


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository-root", type=Path, required=True)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument(
        "--candidate-sha", help="read only regular Git blobs at this exact tested SHA"
    )
    mode.add_argument("--working-tree", action="store_true", help="explicit full local-gate mode")
    args = parser.parse_args(argv)
    try:
        count, unique, size = verify(args.repository_root, args.candidate_sha)
    except (VerificationError, tooling.ToolError, bootstrap.BootstrapError, ContractError) as error:
        print(f"Canonical candidate asset verification failed: {error}", file=sys.stderr)
        return 2
    source = args.candidate_sha or "explicit working tree"
    print(f"Verified {count} canonical assets at {source}: {unique} exact URLs, {size} bytes")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
