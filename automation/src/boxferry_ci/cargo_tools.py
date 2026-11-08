"""Exact candidate Cargo tools; declarations are inert data, never Dockerfile code."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import sys
import tempfile
import tomllib
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path

from boxferry_ci.bootstrap import safe_cache_directory
from boxferry_ci.contracts import ContractError
from boxferry_ci.runner import run

DOCKERFILE = ".devcontainer/Dockerfile"
# Existing crate-backed Renovate declarations retain their sole version ownership.
DECLARATIONS = {
    "cargo-deny": "CARGO_DENY_VERSION",
    "cargo-llvm-cov": "CARGO_LLVM_COV_VERSION",
    "cargo-semver-checks": "CARGO_SEMVER_CHECKS_VERSION",
    "zizmor": "ZIZMOR_VERSION",
}
VERSION = r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)"
MAX_DECLARATION_BYTES = 64 * 1024
COMPILER_KEYS = ("rustc", "rustc-wrapper", "rustc-workspace-wrapper", "rustdoc")
COMPILER_ENV = {key.upper().replace("-", "_") for key in COMPILER_KEYS}
COMPILER_ENV.update(f"CARGO_BUILD_{key}" for key in tuple(COMPILER_ENV))


def read_data(root: Path, relative: str) -> str:
    """Read one bounded regular data file without following directory/file links."""
    path = root / relative
    for component in (path, *path.parents):
        if component.is_symlink():
            raise ContractError(f"Cargo tool metadata must not follow symlinks: {relative}")
        if component == root:
            break
    if not path.is_file() or path.stat().st_size > MAX_DECLARATION_BYTES:
        raise ContractError(f"Cargo tool metadata is missing, nonregular or oversized: {relative}")
    with path.open("rb") as stream:
        data = stream.read(MAX_DECLARATION_BYTES + 1)
    if len(data) > MAX_DECLARATION_BYTES:
        raise ContractError(f"Cargo tool metadata exceeded its read bound: {relative}")
    return data.decode("utf-8")


def declarations(root: Path) -> dict[str, str]:
    """Parse only the four existing, uniquely annotated exact Cargo crate ARGs."""
    text = read_data(root, DOCKERFILE)
    selected: dict[str, str] = {}
    for package, argument in DECLARATIONS.items():
        rows = re.findall(rf"(?m)^\s*ARG\s+{argument}(?:[=\s].*)?$", text)
        marker = f"# renovate: datasource=crate depName={package}"
        if not rows and marker not in text:
            continue
        matches = re.findall(rf"(?m)^{re.escape(marker)}\r?\nARG {argument}=({VERSION})\r?$", text)
        if len(rows) != 1 or len(matches) != 1 or text.count(marker) != 1:
            raise ContractError(f"{argument} needs one adjacent crate marker and exact X.Y.Z value")
        selected[package] = matches[0]
    return selected


@dataclass(frozen=True)
class Selection:
    """Candidate-owned CLI version and exact compiler channel."""

    package: str
    version: str
    rust_channel: str

    @property
    def subcommand(self) -> str:
        return self.package.removeprefix("cargo-")


def select(
    root: Path, package: str, fallback: Path | None = None, *, require_candidate: bool = False
) -> Selection:
    if package not in DECLARATIONS or not package.startswith("cargo-"):
        raise ContractError("unsupported Cargo subcommand selection")
    metadata = root
    path = root / DOCKERFILE
    if path.parent.is_symlink():
        raise ContractError("Cargo tool Dockerfile directory must not be a symlink")
    if not path.exists() and not path.is_symlink():
        if require_candidate:
            raise ContractError("candidate Dockerfile must declare the selected Cargo tool")
        # Consumers without a Dockerfile already use this immutable archive's data.
        metadata = fallback if fallback is not None else Path(__file__).resolve().parents[3]
    versions = declarations(metadata)
    if package not in versions:
        raise ContractError(f"present Dockerfile does not declare {package}; no fallback permitted")
    toolchain = tomllib.loads(read_data(root, "rust-toolchain.toml")).get("toolchain", {})
    if not isinstance(toolchain, dict):
        raise ContractError("candidate Rust toolchain must be a TOML table")
    channel = toolchain.get("channel")
    if not isinstance(channel, str) or re.fullmatch(VERSION, channel) is None:
        raise ContractError("candidate Rust channel must be one exact X.Y.Z release")
    return Selection(package, versions[package], channel)


def command(
    argv: tuple[str, ...], cwd: Path, environment: Mapping[str, str], timeout: int = 30
) -> str:
    result = run(
        argv,
        cwd=cwd,
        environment=environment,
        stage="exact Cargo tool selection",
        timeout_seconds=timeout,
        max_output_bytes=2_000_000 if timeout > 30 else 64 * 1024,
    )
    if result.status != "success":
        raise ContractError(
            f"Cargo tool command failed: {result.diagnostic.cause}: {result.stderr}"
        )
    return result.stdout


def reject_overrides(selection: Selection, cwd: Path, environment: Mapping[str, str]) -> Path:
    """Reject applicable aliases/compiler replacements; ordinary ci-* aliases remain valid."""
    for key, value in environment.items():
        alias = key.removeprefix("CARGO_ALIAS_").lower().replace("_", "-")
        if value and (
            (key.startswith("CARGO_ALIAS_") and alias == selection.subcommand)
            or key in COMPILER_ENV
        ):
            raise ContractError(
                "Cargo alias or compiler environment override defeats exact selection"
            )
    home = Path(
        environment.get(
            "CARGO_HOME", str(Path(environment.get("HOME", str(Path.home()))) / ".cargo")
        )
    )
    home = (cwd / home).absolute()
    locations = [home / name for name in ("config", "config.toml")]
    locations.extend(
        directory / ".cargo" / name
        for directory in (cwd, *cwd.parents)
        for name in ("config", "config.toml")
    )
    for path in locations:
        if not path.exists() and not path.is_symlink():
            continue
        config = tomllib.loads(read_data(path.parent, path.name))
        aliases, build, injected = (
            config.get("alias", {}),
            config.get("build", {}),
            config.get("env", {}),
        )
        if not all(isinstance(table, dict) for table in (aliases, build, injected)):
            raise ContractError("Cargo alias/build/env configuration must be a table")
        affected = COMPILER_ENV | {
            "PATH",
            "CARGO_HOME",
            "RUSTUP_TOOLCHAIN",
            f"CARGO_ALIAS_{selection.subcommand.upper().replace('-', '_')}",
            f"CARGO_ALIAS_{selection.subcommand.upper()}",
        }
        if (
            "include" in config
            or selection.subcommand in aliases
            or any(key in build for key in COMPILER_KEYS)
            or any(key in affected for key in injected)
        ):
            raise ContractError("Cargo config overrides selected subcommand or compiler")
    return home


def verify(
    selection: Selection, cwd: Path, environment: Mapping[str, str], expected: Path | None = None
) -> Path:
    """Control Cargo's documented PATH/home dispatch and verify the invoked plugin version.

    Cargo's verbose command enumeration is not an execution-path API: a duplicate
    command can list a later PATH entry. Never accept that listing as path proof.
    """
    home = reject_overrides(selection, cwd, environment)
    # Resolve relative/empty PATH entries in the subprocess cwd, not the caller's cwd.
    search_path = os.pathsep.join(
        str(cwd / entry) for entry in environment.get("PATH", os.defpath).split(os.pathsep)
    )
    cargo = shutil.which("cargo", path=search_path)
    ambient = shutil.which(selection.package, path=search_path)
    if cargo is None or ambient is None or shutil.which("rustup", path=search_path) is None:
        # The Action otherwise bootstraps rustup and prepends a new Cargo bin directory.
        raise ContractError("Cargo, rustup or the selected PATH subcommand is unavailable")
    executable = Path(ambient).resolve(strict=True)
    if not executable.is_file() or (
        expected is not None and executable != expected.resolve(strict=True)
    ):
        raise ContractError("PATH selected a different executable from the owned installation")
    home_plugin = home / "bin" / selection.package
    if (home_plugin.exists() or home_plugin.is_symlink()) and home_plugin.resolve(
        strict=True
    ) != executable:
        raise ContractError("Cargo home shadows the selected PATH executable")
    selected_environment = dict(environment, RUSTUP_TOOLCHAIN=selection.rust_channel)
    for package in ("cargo", "rustc"):
        binary = shutil.which(package, path=search_path)
        if binary is None:
            raise ContractError(f"{package} executable unavailable for compiler verification")
        release = command((binary, "--version"), cwd, selected_environment)
        if (
            re.fullmatch(
                rf"{package} {re.escape(selection.rust_channel)} \([^\r\n]+\)\r?\n?", release
            )
            is None
        ):
            raise ContractError(f"{package} did not select candidate Rust {selection.rust_channel}")
    version = command((cargo, selection.subcommand, "--version"), cwd, selected_environment)
    if (
        re.fullmatch(
            rf"{re.escape(selection.package)} {re.escape(selection.version)}\r?\n?", version
        )
        is None
    ):
        raise ContractError(
            f"Cargo did not invoke the declared {selection.package} {selection.version}"
        )
    return executable


def install(selection: Selection, destination: Path, cwd: Path) -> dict[str, str]:
    """Use exact canonical registry resolution and its lockfile/checksum verification."""
    reject_overrides(selection, cwd, os.environ)
    home = safe_cache_directory(destination / selection.package / selection.version)
    # Never inherit registry replacement, tool aliases, or another worktree's target.
    environment = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith("CARGO_") and key not in {"RUSTFLAGS", "RUSTDOCFLAGS"}
    }
    environment.update(CARGO_HOME=str(home), RUSTUP_TOOLCHAIN=selection.rust_channel)
    cargo = shutil.which("cargo", path=environment.get("PATH"))
    if cargo is None:
        raise ContractError("Cargo executable is unavailable for exact installation")
    # A private cwd/home prevents candidate/global Cargo config redirecting crates.io.
    with tempfile.TemporaryDirectory(prefix="boxferry-cargo-install-") as temporary:
        for directory in (home, Path(temporary), *Path(temporary).parents):
            relatives = (
                ("config", "config.toml")
                if directory == home
                else (".cargo/config", ".cargo/config.toml")
            )
            for relative in relatives:
                configured = directory / relative
                if configured.exists() or configured.is_symlink():
                    raise ContractError(
                        "exact Cargo installation refuses ambient registry configuration"
                    )
        command(
            (
                cargo,
                "install",
                "--locked",
                "--force",
                "--version",
                f"={selection.version}",
                "--registry",
                "crates-io",
                "--root",
                str(home),
                selection.package,
            ),
            Path(temporary),
            environment,
            timeout=900,
        )
    binary = home / "bin" / selection.package
    if binary.is_symlink():
        raise ContractError("selected Cargo installation must not be an executable symlink")
    verification = dict(os.environ, CARGO_HOME=str(home), RUSTUP_TOOLCHAIN=selection.rust_channel)
    verification["PATH"] = str(binary.parent) + os.pathsep + verification.get("PATH", "")
    verify(selection, cwd, verification, binary)
    return {
        "CARGO_HOME": str(home),
        "RUSTUP_TOOLCHAIN": selection.rust_channel,
        "bin_directory": str(binary.parent),
        "rust_channel": selection.rust_channel,
        "executable": str(binary),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("install", "verify"))
    parser.add_argument("--repository-root", type=Path, required=True)
    parser.add_argument(
        "--package", choices=tuple(k for k in DECLARATIONS if k.startswith("cargo-")), required=True
    )
    parser.add_argument("--destination", type=Path)
    parser.add_argument("--require-candidate", action="store_true")
    parser.add_argument("--github-output", type=Path)
    parser.add_argument("--github-path", type=Path)
    parser.add_argument("--github-env", type=Path)
    options = parser.parse_args(argv)
    try:
        selection = select(
            options.repository_root.resolve(),
            options.package,
            require_candidate=options.require_candidate,
        )
        if options.operation == "install":
            if options.destination is None:
                raise ContractError("exact installation requires an owned destination")
            values = install(selection, options.destination, Path.cwd())
            executable = Path(values["executable"])
            for path, text in (
                (options.github_output, f"rust_channel={selection.rust_channel}\n"),
                (options.github_path, values["bin_directory"] + "\n"),
                (
                    options.github_env,
                    "".join(f"{key}={values[key]}\n" for key in ("CARGO_HOME", "RUSTUP_TOOLCHAIN")),
                ),
            ):
                if any("\n" in value or "\r" in value for value in values.values()):
                    raise ContractError("Cargo tool paths contain environment-file controls")
                if path is not None:
                    with path.open("a", encoding="utf-8") as output:
                        output.write(text)
        else:
            executable = verify(selection, Path.cwd(), os.environ)
        print(
            json.dumps(
                {
                    "package": selection.package,
                    "version": selection.version,
                    "rust_channel": selection.rust_channel,
                    "status": "verified",
                    "executable": str(executable),
                }
            )
        )
        return 0
    except (OSError, ValueError) as error:
        print(f"Exact Cargo tool selection failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
