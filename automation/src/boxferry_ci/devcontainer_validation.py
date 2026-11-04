"""Build a held candidate and execute client probes in its exact native image.

This producer performs no publication or application/native-runtime qualification.
Two hosted lanes share one immutable seven-repository source resolution; local
execution requires real clean sibling checkouts and an explicit Podman backend.
Budgets are conservative qualification targets, not measured acceptance evidence.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shlex
import shutil
import signal
import stat
import sys
import tempfile
import time
import tomllib
import uuid
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .contracts import ContractError, exact_sha
from .dependencies import fetch_metadata
from .operational import _json
from .runner import RunCancelled, _argument_secrets, _redacted_argv, protected_values, redact, run

OWNERS = frozenset({"boxferry", "compose-lens", "podman-lens", "quadlet-lens"})
SIBLINGS = (
    "compose-lens",
    "docker-lens",
    "kubernetes-lens",
    "podman-lens",
    "quadlet-lens",
    "boxferry-website",
)
ALL_REPOSITORIES = ("boxferry", *SIBLINGS)
ARCHITECTURES = {"x86_64": "amd64", "aarch64": "arm64"}
CLI_PROJECT = "automation/devcontainer"
CONFIG = ".devcontainer/devcontainer.json"
LOCK = ".devcontainer/devcontainer-lock.json"
GIB = 1024**3
BUILD_SECONDS, PROBE_SECONDS, CLEANUP_SECONDS = 3600, 300, 300
MAX_TREE_FILES, MAX_TREE_BYTES = 25000, 256 * 1024**2
MAX_REPORT = 64 * 1024
PROBES: tuple[tuple[str, ...], ...] = (
    ("actionlint", "--version"),
    ("cargo", "--version"),
    ("cargo-clippy", "--version"),
    ("cargo-deny", "--version"),
    ("cargo-llvm-cov", "--version"),
    ("cargo-semver-checks", "--version"),
    ("curl", "--version"),
    ("gh", "--version"),
    ("git", "--version"),
    ("hadolint", "--version"),
    ("helm", "version", "--short"),
    ("jq", "--version"),
    ("kind", "version"),
    ("kubectl", "version", "--client=true"),
    ("kustomize", "version"),
    ("lychee", "--version"),
    ("markdownlint-cli2", "--help"),
    ("node", "--version"),
    ("npm", "--version"),
    ("prettier", "--version"),
    ("rustc", "--version"),
    ("rustfmt", "--version"),
    ("rustup", "--version"),
    ("shellcheck", "--version"),
    ("shfmt", "--version"),
    ("tombi", "--version"),
    ("uv", "--version"),
    ("ruff", "--version"),
    ("ty", "--version"),
    ("zizmor", "--version"),
)


class ValidationError(ContractError):
    """Candidate, native backend, resource or cleanup evidence failed."""


def manifest_digest(document: Mapping[str, object]) -> str:
    return hashlib.sha256(
        json.dumps(document, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()


def parse_sources(document: object, repository: str, candidate: str) -> dict[str, str]:
    if (
        not isinstance(document, dict)
        or set(document) != {"schema", "repositories"}
        or type(document["schema"]) is not int
        or document["schema"] != 1
    ):
        raise ValidationError("workspace source manifest has an unsupported schema")
    sources = document["repositories"]
    expected = set(ALL_REPOSITORIES) if repository == "boxferry" else {repository}
    if not isinstance(sources, dict) or set(sources) != expected:
        raise ValidationError(
            "workspace source manifest must identify every required real checkout"
        )
    result = {name: exact_sha(value, "workspace revision") for name, value in sources.items()}
    if result[repository] != candidate:
        raise ValidationError("workspace source manifest differs from the tested candidate")
    return result


def resolve_sources(
    repository: str, candidate: str, request: Callable[[str], bytes] = fetch_metadata
) -> dict[str, object]:
    """Read each permitted public default branch once; never resolve inside a worker."""
    if repository not in OWNERS:
        raise ValidationError("repository does not own a Dev Container")
    revisions = {repository: exact_sha(candidate, "candidate SHA")}
    if repository == "boxferry":
        for name in SIBLINGS:
            metadata = _json(request(f"https://api.github.com/repos/Strukturpiloten/{name}"))
            branch = metadata.get("default_branch")
            if not isinstance(branch, str) or re.fullmatch(r"[A-Za-z0-9_.-]+", branch) is None:
                raise ValidationError("public repository has an unsupported default branch")
            commit = _json(
                request(f"https://api.github.com/repos/Strukturpiloten/{name}/commits/{branch}")
            )
            revisions[name] = exact_sha(commit.get("sha"), "resolved sibling SHA")
    return {"schema": 1, "repositories": revisions}


def environment(private: Path) -> dict[str, str]:
    """Finite trusted host environment: no tokens, personal configs or runtime sockets."""
    return {
        "PATH": "/usr/local/bin:/usr/bin:/bin",
        "HOME": str(private / "home"),
        "DOCKER_CONFIG": str(private / "docker-config"),
        "XDG_CONFIG_HOME": str(private / "config"),
        "XDG_CACHE_HOME": str(private / "cache"),
        "XDG_RUNTIME_DIR": str(private / "runtime"),
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": "/dev/null",
        "GIT_TERMINAL_PROMPT": "0",
        "GIT_ASKPASS": "/bin/false",
        "LC_ALL": "C.UTF-8",
        "CI": "true",
    }


@dataclass
class Budget:
    path: Path
    initial_free: int
    deadline: float
    last_checked: float = 0
    check_disk: bool = True

    @classmethod
    def start(cls, path: Path, seconds: int) -> Budget:
        free = shutil.disk_usage(path).free
        if free < 12 * GIB:
            raise ValidationError("image qualification requires at least 12 GiB free initially")
        return cls(path, free, time.monotonic() + seconds)

    def check(self) -> None:
        now = time.monotonic()
        if now > self.deadline:
            raise ValidationError("image qualification exhausted its stage deadline")
        if self.check_disk and now - self.last_checked >= 1:
            self.last_checked = now
            free = shutil.disk_usage(self.path).free
            # Counts all filesystem growth conservatively, including external users;
            # never treats shared daemon layers as free run-owned disk capacity.
            if free < 2 * GIB or self.initial_free - free > 10 * GIB:
                raise ValidationError("image qualification exceeded disk growth or reserve budget")


class Commands:
    def __init__(self, private: Path) -> None:
        self.private = private
        self.env = environment(private)
        for key in (
            "HOME",
            "DOCKER_CONFIG",
            "XDG_CONFIG_HOME",
            "XDG_CACHE_HOME",
            "XDG_RUNTIME_DIR",
        ):
            Path(self.env[key]).mkdir(mode=0o700, parents=True, exist_ok=True)
        self.budget: Budget | None = None
        self.outcomes: list[dict[str, object]] = []

    def call(
        self, argv: tuple[str, ...], stage: str, timeout: int = 30, *, cwd: Path | None = None
    ) -> str:
        started = time.monotonic()
        hidden = (*protected_values(os.environ), *_argument_secrets(argv))
        safe = shlex.join(_redacted_argv(argv, hidden, cwd or self.private))
        outcome: dict[str, object] = {
            "stage": stage,
            "command": safe[:2048],
            "command_truncated": len(safe) > 2048,
            "status": "infrastructure",
            "cause": "command precheck not completed",
            "next_action": (
                "Correct the named command or resource cause; rerun the same candidate."
            ),
        }
        self.outcomes.append(outcome)
        try:
            if self.budget is not None:
                self.budget.check()
            result = run(
                argv,
                cwd=cwd or self.private,
                stage=stage,
                timeout_seconds=timeout,
                environment=self.env,
                secrets=hidden,
                monitor=self.budget.check if self.budget else None,
            )
            outcome.update(status=result.status, cause=result.diagnostic.cause)
            if result.status != "success":
                detail = redact((result.stderr or result.stdout)[-1024:], hidden)
                raise ValidationError(f"{result.diagnostic.cause}; {detail}")
            return result.stdout
        except (OSError, ValueError, RunCancelled, KeyboardInterrupt) as error:
            cause = redact(str(error), hidden)[:2048]
            if isinstance(error, (RunCancelled, KeyboardInterrupt)):
                outcome["status"] = "cancelled"
            outcome["cause"] = cause
            if isinstance(error, RunCancelled):
                raise RunCancelled(f"{stage}: {cause}") from None
            if isinstance(error, KeyboardInterrupt):
                raise KeyboardInterrupt(f"{stage}: {cause}") from None
            raise ValidationError(f"{stage}: {cause}") from None
        finally:
            outcome["duration_seconds"] = time.monotonic() - started

    def git(self, root: Path, *argv: str) -> str:
        return self.call(
            (
                "git",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "http.followRedirects=false",
                "-C",
                str(root),
                *argv,
            ),
            "source",
            120,
        )


def fingerprint(root: Path, commands: Commands) -> str:
    """Bind all nonignored build context, modes and bytes, including dirty local edits."""
    paths = commands.git(
        root, "ls-files", "--cached", "--others", "--exclude-standard", "-z"
    ).split("\0")
    paths = sorted(set(path for path in paths if path))
    if len(paths) > MAX_TREE_FILES:
        raise ValidationError("candidate input closure exceeds its file bound")
    digest, total = hashlib.sha256(), 0
    consumed = 0
    for path in paths:
        relative = Path(path)
        target = root / relative
        if relative.is_absolute() or ".." in relative.parts or ".git" in relative.parts:
            raise ValidationError("candidate input path escapes its build context")
        if not target.exists() and not target.is_symlink():
            digest.update(json.dumps([path, "deleted"]).encode())
            continue
        info = target.lstat()
        if not stat.S_ISREG(info.st_mode) or any(
            parent.is_symlink() for parent in target.parents if parent != root
        ):
            raise ValidationError(
                "candidate input closure requires regular files without symbolic links"
            )
        total += info.st_size
        if total > MAX_TREE_BYTES:
            raise ValidationError("candidate input closure exceeds its byte bound")
        digest.update(json.dumps([path, stat.S_IMODE(info.st_mode), info.st_size]).encode())
        with target.open("rb") as stream:
            while data := stream.read(65536):
                consumed += len(data)
                if consumed > MAX_TREE_BYTES:
                    raise ValidationError(
                        "candidate input changed beyond its byte bound while reading"
                    )
                if commands.budget:
                    commands.budget.check()
                digest.update(data)
    return digest.hexdigest()


def _held_revision(root: Path, expected: str, commands: Commands, *, clean: bool = True) -> None:
    if Path(commands.git(root, "rev-parse", "--show-toplevel").strip()).resolve() != root.resolve():
        raise ValidationError("workspace source must be an actual independent Git checkout")
    if commands.git(root, "rev-parse", "HEAD").strip() != expected:
        raise ValidationError("source checkout differs from its immutable revision")
    if clean and commands.git(root, "status", "--porcelain", "--untracked-files=all").strip():
        raise ValidationError("immutable source checkout has working-tree changes")


def _checkout(destination: Path, revision: str, commands: Commands, source: str) -> None:
    destination.mkdir()
    commands.git(destination, "init", "--quiet")
    commands.git(destination, "fetch", "--quiet", "--no-tags", "--depth=1", source, revision)
    commands.git(destination, "checkout", "--quiet", "--detach", "FETCH_HEAD")
    _held_revision(destination, revision, commands)


def _overlay(source: Path, target: Path, commands: Commands) -> None:
    paths = commands.git(
        source, "ls-files", "--cached", "--others", "--exclude-standard", "-z"
    ).split("\0")
    for path in filter(None, paths):
        original, destination = source / path, target / path
        if original.exists():
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(original, destination)
            destination.chmod(stat.S_IMODE(original.stat().st_mode))
        elif destination.exists():
            destination.unlink()


def validate_configuration(root: Path, repository: str) -> dict[str, Any]:
    path = root / CONFIG
    if path.stat().st_size > 32768:
        raise ValidationError("Dev Container configuration exceeds its byte bound")
    document = _json(path.read_bytes())
    allowed = {
        "name",
        "build",
        "workspaceMount",
        "workspaceFolder",
        "remoteUser",
        "updateRemoteUserUID",
        "features",
        "containerEnv",
        "mounts",
        "postCreateCommand",
        "waitFor",
        "hostRequirements",
        "customizations",
    }
    if set(document) - allowed:
        raise ValidationError(
            "Dev Container configuration includes unsupported hooks or runtime options"
        )
    workspace = f"/workspaces/{repository}"
    if document.get("build") != {"dockerfile": "Dockerfile", "context": ".."}:
        raise ValidationError(
            "only the reviewed Dockerfile and repository build context are supported"
        )
    if (
        document.get("workspaceFolder") != workspace
        or document.get("workspaceMount")
        != f"source=${{localWorkspaceFolder}},target={workspace},type=bind"
    ):
        raise ValidationError("Dev Container workspace differs from its reviewed repository target")
    if document.get("remoteUser") != "vscode" or document.get("updateRemoteUserUID") is not True:
        raise ValidationError("Dev Container must configure its reviewed nonroot user")
    lifecycle = "bash .devcontainer/verify-tools.sh"
    if repository != "boxferry":
        lifecycle += " && cargo fetch --locked"
    if (
        document.get("postCreateCommand") != lifecycle
        or document.get("waitFor") != "postCreateCommand"
    ):
        raise ValidationError("Dev Container lifecycle command differs from its reviewed verifier")
    env = document.get("containerEnv")
    paths = {"CARGO_HOME": f"/workspaces/.{repository}-cargo"}
    if repository != "boxferry":
        paths["CARGO_TARGET_DIR"] = f"/workspaces/.{repository}-target"
    if repository in {"boxferry", "podman-lens"}:
        paths["GH_CONFIG_DIR"] = f"/workspaces/.{repository}-gh"
    expected_env = dict(paths)
    if repository != "podman-lens":
        expected_env["PATH"] = (
            "/home/vscode/.local/bin:/usr/local/cargo/bin:/usr/local/sbin:"
            "/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
        )
    if env != expected_env:
        raise ValidationError("Dev Container environment differs from its finite reviewed contract")
    volume_names = {"CARGO_HOME": "cargo", "CARGO_TARGET_DIR": "target", "GH_CONFIG_DIR": "gh"}
    expected = {
        f"source={repository}-{volume_names[key]}-${{devcontainerId}},target={target},type=volume"
        for key, target in paths.items()
    }
    if repository == "boxferry":
        expected.update(
            f"source=${{localWorkspaceFolder}}/../{name},target={workspace}/.boxferry-workspace/{name},type=bind"
            for name in SIBLINGS
        )
    mounts = document.get("mounts")
    if (
        not isinstance(mounts, list)
        or any(not isinstance(item, str) for item in mounts)
        or len(mounts) != len(expected)
        or set(mounts) != expected
    ):
        raise ValidationError(
            "Dev Container mounts differ from the finite reviewed workspace contract"
        )
    # Configuration substitutions can read host environment even during build.
    for key in ("features", "build"):
        if "${" in json.dumps(document.get(key)):
            raise ValidationError("Dev Container build inputs cannot substitute host environment")
    if not (root / LOCK).is_file() or not (root / ".devcontainer/verify-tools.sh").is_file():
        raise ValidationError(
            "candidate requires a committed feature lock and executable verifier source"
        )
    return document


def canonical_versions(automation: Path) -> dict[str, str]:
    project = tomllib.loads((automation / "automation/pyproject.toml").read_text())
    versions = {"uv": project["tool"]["uv"]["required-version"].removeprefix("==")}
    for name in ("ruff", "ty"):
        matches = [
            value.split("==", 1)[1]
            for value in project["dependency-groups"]["dev"]
            if value.startswith(name + "==")
        ]
        if len(matches) != 1:
            raise ValidationError("shared Python authority has no unique exact tool version")
        versions[name] = matches[0]
    source = (automation / ".github/workflows/ci.yml").read_text()
    nodes = set(re.findall(r"^\s+node-version: ([0-9]+\.[0-9]+\.[0-9]+)$", source, re.MULTILINE))
    if len(nodes) != 1:
        raise ValidationError("shared Node authority has no unique exact version")
    versions["node"] = nodes.pop()
    if any(re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", value) is None for value in versions.values()):
        raise ValidationError("canonical tool versions must be exact numeric releases")
    return versions


def _cli(automation: Path) -> tuple[Path, str]:
    project = automation / CLI_PROJECT
    package = _json((project / "package.json").read_bytes())
    lock = _json((project / "package-lock.json").read_bytes())
    dependencies = package.get("devDependencies")
    if not isinstance(dependencies, dict) or set(dependencies) != {"@devcontainers/cli"}:
        raise ValidationError(
            "official CLI project must declare only its exact canonical dependency"
        )
    version = dependencies["@devcontainers/cli"]
    packages = lock.get("packages")
    if lock.get("lockfileVersion") != 3 or not isinstance(packages, dict):
        raise ValidationError("official CLI requires a bounded npm v3 integrity lock")
    root, record = packages.get(""), packages.get("node_modules/@devcontainers/cli")
    if (
        not isinstance(root, dict)
        or root.get("devDependencies") != dependencies
        or not isinstance(record, dict)
    ):
        raise ValidationError("official CLI root declaration and locked package disagree")
    if (
        not isinstance(version, str)
        or re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version) is None
        or record.get("version") != version
        or not isinstance(record.get("integrity"), str)
        or re.fullmatch(r"sha512-[A-Za-z0-9+/]{86}==", record["integrity"]) is None
        or record.get("resolved")
        != f"https://registry.npmjs.org/@devcontainers/cli/-/cli-{version}.tgz"
    ):
        raise ValidationError("official CLI declaration and integrity-locked package disagree")
    executable = project / "node_modules/@devcontainers/cli/devcontainer.js"
    if not executable.is_file():
        raise ValidationError(
            "install the canonical CLI with npm ci --ignore-scripts before qualification"
        )
    return executable, version


def backend(repository: str, mode: str, selected: str) -> str:
    expected = (
        "podman" if mode == "local" or repository in {"podman-lens", "quadlet-lens"} else "docker"
    )
    if selected != expected:
        raise ValidationError(
            "backend differs from reviewed hosted/local runtime policy; no fallback"
        )
    return expected


def validate_hosted_podman_host(environment_values: Mapping[str, str], release: str) -> None:
    fields = dict(line.split("=", 1) for line in release.splitlines() if "=" in line)
    if (
        environment_values.get("GITHUB_ACTIONS") != "true"
        or environment_values.get("RUNNER_OS") != "Linux"
        or environment_values.get("GITHUB_REPOSITORY")
        not in {"Strukturpiloten/podman-lens", "Strukturpiloten/quadlet-lens"}
        or fields.get("ID", "").strip('"') != "ubuntu"
        or re.fullmatch(r"[0-9]{2}\.[0-9]{2}", fields.get("VERSION_ID", "").strip('"')) is None
    ):
        raise ValidationError(
            "rootful Podman prerequisite requires its reviewed Ubuntu hosted caller"
        )


def prepare_hosted_podman(commands: Commands) -> dict[str, object]:
    """Reuse the existing Ubuntu signed-APT OUTER prerequisite exception.

    Quadlet's hosted-generator-podman.py and Podman's native workflow own this
    host-package prerequisite, distinct from immutable INNER native runtime pins.
    There is no new floating download or invented exact outer version pin.
    """
    validate_hosted_podman_host(os.environ, Path("/etc/os-release").read_text())
    if not Path("/usr/bin/podman").is_file():
        commands.call(
            (
                "/usr/bin/sudo",
                "-n",
                "/usr/bin/timeout",
                "--signal=TERM",
                "--kill-after=2s",
                "80s",
                "/bin/sh",
                "-ceu",
                "apt-get update && apt-get install --yes --no-install-recommends podman",
            ),
            "podman-signed-apt-prerequisite",
            85,
        )
    owner = commands.call(
        ("/usr/bin/dpkg-query", "--search", "/usr/bin/podman"), "podman-package-owner"
    ).strip()
    if owner != "podman: /usr/bin/podman":
        raise ValidationError("hosted Podman binary is outside the reviewed signed-APT package")
    package = commands.call(
        ("/usr/bin/dpkg-query", "--show", "--showformat=${Version}", "podman"),
        "podman-package-version",
    ).strip()
    if re.fullmatch(r"[A-Za-z0-9.+:~_-]{1,128}", package) is None:
        raise ValidationError("hosted Podman package version is unavailable")
    uid = commands.call(
        (
            "/usr/bin/sudo",
            "-n",
            "/usr/bin/timeout",
            "--signal=TERM",
            "--kill-after=2s",
            "5s",
            "/usr/bin/id",
            "-u",
        ),
        "podman-rootful-prerequisite",
        8,
    ).strip()
    if uid != "0":
        raise ValidationError("reviewed hosted Podman requires actual root privileges")
    return {
        "installation": "ubuntu-signed-apt-prerequisite",
        "package": package,
        "rootless": False,
        "remote": False,
        "root_uid": 0,
        "version_policy": "reviewed-host-package-prerequisite-not-inner-runtime-pin",
    }


def _engine(private: Path, engine: str, mode: str = "local") -> str:
    if engine == "docker":
        return "/usr/bin/docker"
    launcher = private / "podman"
    override = private / "containers.conf"
    override.write_text('[engine]\nlock_type = "file"\n')
    base = (
        "/usr/bin/podman",
        "--remote=false",
        "--root",
        str(private / "store"),
        "--runroot",
        str(private / "runroot"),
        "--tmpdir",
        str(private / "podman-tmp"),
        "--storage-driver",
        "overlay",
    )
    if mode == "hosted":
        # The timeout itself is root-owned, matching the reviewed hosted launcher;
        # killing an unprivileged sudo client is not a privileged-child deadline.
        clean = (
            "/usr/bin/env",
            "-i",
            *(f"{key}={value}" for key, value in environment(private).items()),
            "CONTAINERS_CONF_OVERRIDE=" + str(override),
        )
        prefix = (
            '#!/bin/sh\ncase "$1" in\n'
            "  build) seconds=3600 ;;\n"
            '  buildx) if [ "${2:-}" = build ]; then seconds=3600; else seconds=30; fi ;;\n'
            "  start) seconds=300 ;;\n"
            "  *) seconds=30 ;;\nesac\n"
            'exec /usr/bin/sudo -n /usr/bin/timeout --signal=TERM --kill-after=2s "${seconds}s" '
        )
        launcher.write_text(prefix + shlex.join((*clean, *base)) + ' "$@"\n')
    else:
        launcher.write_text(
            "#!/bin/sh\nexec "
            + shlex.join(("/usr/bin/env", "CONTAINERS_CONF_OVERRIDE=" + str(override), *base))
            + ' "$@"\n'
        )
    launcher.chmod(0o700)
    return str(launcher)


def exclude_git_build_metadata(snapshot: Path) -> dict[str, dict[str, str | int]]:
    """Bind effective ignores while retaining real Git metadata for runtime smoke."""
    result = {}
    paths = [snapshot / ".dockerignore"]
    # Podman gives this root-level file precedence over .dockerignore.
    container_ignore = snapshot / ".containerignore"
    if container_ignore.exists() or container_ignore.is_symlink():
        paths.append(container_ignore)
    specific = snapshot / ".devcontainer/Dockerfile.dockerignore"
    if specific.exists():
        paths.append(specific)
    for path in paths:
        if path.is_symlink() or (path.exists() and not path.is_file()):
            raise ValidationError("build-context ignore must be a regular candidate file")
        content = path.read_bytes() if path.exists() else b""
        if len(content) > 128 * 1024:
            raise ValidationError("build-context ignore exceeds its byte bound")
        content = (
            content.rstrip(b"\r\n")
            + b"\n# BoxFerry qualification: Git metadata is runtime-only.\n.git\n**/.git\n"
        )
        path.write_bytes(content)
        result[path.relative_to(snapshot).as_posix()] = {
            "sha256": hashlib.sha256(content).hexdigest(),
            "mode": stat.S_IMODE(path.stat().st_mode),
        }
    return result


def build_context_fingerprint(
    snapshot: Path, commands: Commands, ignores: Mapping[str, Mapping[str, str | int]]
) -> str:
    # Candidate .gitignore can hide these files; bind generated bytes and modes
    # explicitly in addition to the ordinary complete tracked input fingerprint.
    actual = {}
    for name in ignores:
        path = snapshot / name
        if path.is_symlink() or not path.is_file():
            raise ValidationError("held build-context ignore was substituted")
        actual[name] = {
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "mode": stat.S_IMODE(path.stat().st_mode),
        }
    if actual != ignores:
        raise ValidationError("held build-context ignore changed during qualification")
    return manifest_digest({"files": fingerprint(snapshot, commands), "ignores": actual})


def _image_identity(value: object) -> str:
    if not isinstance(value, str) or re.fullmatch(r"(?:sha256:)?[a-f0-9]{64}", value) is None:
        raise ValidationError("built image has no immutable SHA-256 image ID")
    return value if value.startswith("sha256:") else "sha256:" + value


def validate_engine(
    document: object,
    selected: str,
    architecture: str,
    private: Path,
    *,
    rootless: bool | None = None,
) -> None:
    if not isinstance(document, dict):
        raise ValidationError("outer engine returned malformed native evidence")
    if selected == "docker":
        if (
            document.get("OSType") != "linux"
            or document.get("Architecture")
            not in {architecture, {"amd64": "x86_64", "arm64": "aarch64"}[architecture]}
            or not isinstance(document.get("DockerRootDir"), str)
            or not document["DockerRootDir"].startswith("/")
        ):
            raise ValidationError("reviewed Docker outer backend or architecture was not observed")
    else:
        host, store = document.get("host"), document.get("store")
        if not isinstance(host, dict) or not isinstance(store, dict):
            raise ValidationError("outer Podman store and host facts are missing")
        if (
            host.get("serviceIsRemote") is not False
            or host.get("arch") != architecture
            or store.get("graphRoot") != str(private / "store")
            or store.get("runRoot") != str(private / "runroot")
            or not isinstance(host.get("security"), dict)
            or host["security"].get("rootless") is not rootless
        ):
            raise ValidationError(
                "Podman must use the exact private local store and native architecture"
            )


def passwd_identity(path: Path, user: str) -> tuple[str, str]:
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 65536:
        raise ValidationError("built image passwd content exceeds its regular-file bound")
    entries = [
        line.split(":") for line in path.read_text().splitlines() if line.split(":", 1)[0] == user
    ]
    if len(entries) != 1 or len(entries[0]) != 7:
        raise ValidationError("configured nonroot user has no unique built-image passwd entry")
    uid, gid = entries[0][2:4]
    if re.fullmatch(r"[1-9][0-9]{0,8}", uid) is None or re.fullmatch(r"[0-9]{1,9}", gid) is None:
        raise ValidationError(
            "configured development user needs an actual positive UID and numeric GID"
        )
    return uid, gid


def _inspection(output: str) -> dict[str, Any]:
    document = json.loads(output)
    if not isinstance(document, list) or len(document) != 1 or not isinstance(document[0], dict):
        raise ValidationError("engine inspection must identify one held resource")
    return document[0]


def validate_engine_version(document: dict[str, Any], selected: str) -> None:
    if selected == "docker":
        server = document.get("Server")
        if not isinstance(server, dict):
            raise ValidationError("outer Docker daemon version was not observed")
        platform = server.get("Platform")
        components = server.get("Components")
        if (
            not isinstance(platform, dict)
            or not isinstance(platform.get("Name"), str)
            or not platform["Name"].startswith("Docker Engine")
            or not isinstance(components, list)
            or not any(
                isinstance(item, dict)
                and item.get("Name") == "Engine"
                and item.get("Version") == server.get("Version")
                for item in components
            )
        ):
            raise ValidationError("outer daemon is not the reviewed actual Docker Engine")
    else:
        client = document.get("Client")
        if (
            not isinstance(client, dict)
            or not isinstance(client.get("Version"), str)
            or not isinstance(client.get("APIVersion"), str)
            or re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", client["Version"]) is None
            or re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", client["APIVersion"]) is None
        ):
            raise ValidationError("outer Podman client version was not observed")


def remove_owned_container(
    commands: Commands, engine: tuple[str, ...], identity: str, token: str
) -> None:
    inspected = _inspection(
        commands.call((*engine, "container", "inspect", identity), "cleanup-identity")
    )
    held_id = inspected.get("Id")
    config = inspected.get("Config")
    labels = config.get("Labels") if isinstance(config, dict) else None
    if (
        not isinstance(labels, dict)
        or labels.get("io.boxferry.devcontainer.run") != token
        or not isinstance(held_id, str)
        or re.fullmatch(r"[a-f0-9]{64}", held_id) is None
        or (re.fullmatch(r"[a-f0-9]{64}", identity) and held_id != identity)
    ):
        raise ValidationError("cleanup refuses a container without exact run ownership")
    commands.call((*engine, "rm", "--force", held_id), "cleanup-container")
    remaining = commands.call(
        (
            *engine,
            "container",
            "ls",
            "--all",
            "--no-trunc",
            "--filter",
            "id=" + held_id,
            "--format",
            "{{.ID}}",
        ),
        "cleanup-container-absence",
    )
    if remaining.strip():
        raise ValidationError("exact owned container remains after cleanup")


def inspect_image(document: object, architecture: str) -> str:
    if not isinstance(document, list) or len(document) != 1 or not isinstance(document[0], dict):
        raise ValidationError("engine returned malformed built-image inspection")
    image = document[0]
    identity = _image_identity(image.get("Id"))
    if image.get("Architecture") != architecture or image.get("Os") != "linux":
        raise ValidationError("built image differs from the actual native host architecture")
    config = image.get("Config")
    if not isinstance(config, dict) or config.get("Volumes") or config.get("Env") is None:
        raise ValidationError("built image declares unsupported anonymous volumes or environment")
    return identity


def _runtime_environment(
    config: dict[str, Any], sources: Mapping[str, str], repository: str
) -> list[str]:
    values = {
        **config["containerEnv"],
        "HOME": "/tmp/qualification-home",
        "CI": "true",
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": "/dev/null",
        "GIT_TERMINAL_PROMPT": "0",
        "LC_ALL": "C.UTF-8",
        "RUSTUP_HOME": "/usr/local/rustup",
        "PATH": "/home/vscode/.local/bin:/usr/local/cargo/bin:/usr/local/sbin:"
        "/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        "GIT_CONFIG_COUNT": str(len(sources)),
    }
    for index, name in enumerate(sources):
        target = (
            f"/workspaces/{repository}"
            if name == repository
            else f"/workspaces/boxferry/.boxferry-workspace/{name}"
        )
        values[f"GIT_CONFIG_KEY_{index}"] = "safe.directory"
        values[f"GIT_CONFIG_VALUE_{index}"] = target
    return [f"{name}={value}" for name, value in values.items()]


def probe_script(versions: Mapping[str, str], sources: Mapping[str, str], repository: str) -> str:
    lines = [
        "set -euo pipefail",
        'test "$(id -u)" -gt 0',
        "mkdir -p /tmp/qualification-home",
        'test "$(uname -m)" = "$1"',
    ]
    for name, revision in sources.items():
        destination = (
            f"/workspaces/{repository}"
            if name == repository
            else f"/workspaces/boxferry/.boxferry-workspace/{name}"
        )
        lines.append(
            f'test "$(git -C {shlex.quote(destination)} rev-parse HEAD)" = {shlex.quote(revision)}'
        )
    for argv in PROBES:
        if repository != "boxferry" and argv[0] in {"helm", "kind", "kubectl", "kustomize"}:
            continue
        command = shlex.join(argv)
        name = argv[0]
        lines.append("printf '%s\\n' " + shlex.quote("probe: " + name))
        if name in versions:
            lines.append(f"observed=$({command})")
            if name == "node":
                expected = "v" + versions[name]
                lines.append('actual="$observed"')
            else:
                expected = name + " " + versions[name]
                lines.append('read -r observed_name observed_version _ <<< "$observed"')
                lines.append('actual="$observed_name $observed_version"')
            lines.extend(
                (
                    "expected=" + shlex.quote(expected),
                    'if [[ "$actual" != "$expected" ]]; then',
                    "  printf 'Version mismatch for %s: expected %.160s; observed %.160s\\n' "
                    + shlex.quote(name)
                    + ' "$expected" "$actual" >&2',
                    "  exit 1",
                    "fi",
                )
            )
        else:
            lines.append(command + " >/dev/null")
    lines.append("bash .devcontainer/verify-tools.sh")
    return "\n".join(lines)


def validate_binding(candidate: str, policy: str, event: str, payload: Mapping[str, Any]) -> None:
    if event == "pull_request":
        expected = payload.get("pull_request", {}).get("base", {}).get("sha")
    elif event == "merge_group":
        expected = payload.get("merge_group", {}).get("base_sha")
    elif event in {"push", "workflow_dispatch"}:
        expected = candidate
    else:
        raise ValidationError("unsupported hosted qualification event")
    if exact_sha(expected, "trusted event policy SHA") != policy:
        raise ValidationError(
            "qualification policy differs from the independently trusted event base"
        )


def _verify_origin(root: Path, repository: str, commands: Commands) -> None:
    origin = commands.git(root, "remote", "get-url", "origin").strip()
    if origin not in {
        f"https://github.com/Strukturpiloten/{repository}.git",
        f"git@github.com:Strukturpiloten/{repository}.git",
    }:
        raise ValidationError("source origin differs from its scoped workspace repository")


def _local_sibling_source(
    name: str, source: Path, paths: Mapping[str, Path] | None, revision: str, commands: Commands
) -> Path:
    if name not in SIBLINGS:
        raise ValidationError("local sibling is outside the scoped workspace repositories")
    local = paths[name] if paths is not None else source.parent / name
    _held_revision(local, revision, commands)
    _verify_origin(local, name, commands)
    return local.resolve()


def qualify(
    *,
    repository: str,
    candidate: str,
    policy_sha: str,
    automation_sha: str,
    source: Path,
    policy: Path,
    automation: Path,
    manifest: Mapping[str, object],
    engine: str,
    mode: str,
    work: Path,
    expected_architecture: str | None = None,
    local_sources: Mapping[str, Path] | None = None,
) -> dict[str, object]:
    """Execute a fresh build and verifier; every error including cleanup blocks success."""
    if repository not in OWNERS:
        raise ValidationError("repository does not own a Dev Container")
    candidate = exact_sha(candidate, "candidate SHA")
    policy_sha = exact_sha(policy_sha, "policy SHA")
    automation_sha = exact_sha(automation_sha, "automation SHA")
    sources = parse_sources(manifest, repository, candidate)
    if local_sources is not None and (
        mode != "local"
        or repository != "boxferry"
        or set(local_sources) != set(SIBLINGS)
        or any(not isinstance(path, Path) for path in local_sources.values())
    ):
        raise ValidationError(
            "explicit local Box sources require exactly the six scoped sibling paths"
        )
    selected = backend(repository, mode, engine)
    machine = platform.machine()
    if platform.system() != "Linux" or machine not in ARCHITECTURES:
        raise ValidationError("qualification requires an actual supported native Linux host")
    if mode == "hosted" and os.environ.get("GITHUB_SHA") != candidate:
        raise ValidationError("hosted tested candidate differs from the current workflow SHA")
    if expected_architecture is not None and expected_architecture != ARCHITECTURES[machine]:
        raise ValidationError("native host differs from the required architecture lane")
    if mode == "hosted":
        if os.environ.get("GITHUB_REPOSITORY") != f"Strukturpiloten/{repository}":
            raise ValidationError("hosted repository differs from the scoped candidate owner")
        event_path = Path(os.environ.get("GITHUB_EVENT_PATH", "/nonexistent"))
        if event_path.stat().st_size > 2 * 1024**2:
            raise ValidationError("hosted event evidence exceeds its byte bound")
        validate_binding(
            candidate,
            policy_sha,
            os.environ.get("GITHUB_EVENT_NAME", ""),
            _json(event_path.read_bytes()),
        )
    private = Path(
        tempfile.mkdtemp(
            prefix="bfv-" if selected == "podman" else "devcontainer-",
            dir="/tmp" if selected == "podman" else work,
        )
    )
    private.chmod(0o700)
    commands = Commands(private)
    report: dict[str, object] = {
        "schema": 1,
        "status": "failure",
        "candidate_sha": candidate,
        "policy_sha": policy_sha,
        "automation_sha": automation_sha,
        "architecture": ARCHITECTURES[machine],
        "engine": selected,
        "workspace_sources": dict(sources),
        "source_manifest_sha256": manifest_digest(manifest),
        "scope": "image-tooling-and-workspace-smoke",
        "cleanup": "pending",
    }
    token = uuid.uuid4().hex
    name, tag = f"boxferry-devcontainer-{token}", f"boxferry-devcontainer:{token}"
    executable = _engine(private, selected, mode)
    engine_argv = (executable,)
    container_id: str | None = None
    image_id: str | None = None
    created = False
    build_started = False
    try:
        commands.budget = Budget.start(private, BUILD_SECONDS)
        _held_revision(source, candidate, commands, clean=mode != "local")
        _verify_origin(source, repository, commands)
        _held_revision(policy, policy_sha, commands)
        _verify_origin(policy, repository, commands)
        _held_revision(automation, automation_sha, commands)
        _verify_origin(automation, "boxferry", commands)
        held = fingerprint(source, commands)
        report["input_sha256"] = held
        prerequisite = None
        if selected == "podman" and mode == "hosted":
            prerequisite = prepare_hosted_podman(commands)
            report["outer_prerequisite"] = prerequisite
        version = _json(
            commands.call((*engine_argv, "version", "--format", "{{json .}}"), "engine-version")
        )
        validate_engine_version(version, selected)
        report["engine_version"] = version
        info = _json(commands.call((*engine_argv, "info", "--format", "{{json .}}"), "engine-info"))
        expected_rootless = False if mode == "hosted" else os.geteuid() != 0
        validate_engine(info, selected, ARCHITECTURES[machine], private, rootless=expected_rootless)
        report["runner"] = {"os": "linux", "architecture": ARCHITECTURES[machine], "mode": mode}
        if selected == "podman":
            observed = info.get("version")
            if (
                not isinstance(observed, dict)
                or observed.get("Version") != version["Client"]["Version"]
            ):
                raise ValidationError("Podman info and client versions disagree")
            report["outer_mode"] = "rootless" if expected_rootless else "rootful"
            if prerequisite:
                prerequisite["version"] = observed["Version"]
        snapshots: dict[str, Path] = {}
        for name_in_manifest, revision in sources.items():
            destination = private / name_in_manifest
            if name_in_manifest == repository:
                remote = str(source.resolve())
            elif mode == "local":
                remote = str(
                    _local_sibling_source(
                        name_in_manifest, source, local_sources, revision, commands
                    )
                )
            else:
                remote = f"https://github.com/Strukturpiloten/{name_in_manifest}.git"
            _checkout(destination, revision, commands, remote)
            snapshots[name_in_manifest] = destination
        snapshot = snapshots[repository]
        if mode == "local":
            _overlay(source, snapshot, commands)
        if fingerprint(snapshot, commands) != held:
            raise ValidationError("private candidate snapshot differs from held input closure")
        config = validate_configuration(snapshot, repository)
        if repository == "boxferry":
            for sibling in SIBLINGS:
                (snapshot / ".boxferry-workspace" / sibling).mkdir(parents=True, exist_ok=True)
        ignores = exclude_git_build_metadata(snapshot)
        context_held = build_context_fingerprint(snapshot, commands, ignores)
        report["build_context_sha256"] = context_held
        report["build_context_ignore_sha256"] = {
            name: binding["sha256"] for name, binding in ignores.items()
        }
        report["build_context_ignore_modes"] = {
            name: binding["mode"] for name, binding in ignores.items()
        }
        report["git_metadata_excluded_from_build"] = True
        cli, cli_version = _cli(automation)
        node = shutil.which("node")
        if node is None or not Path(node).is_file():
            raise ValidationError("trusted Node executable is unavailable")
        node_version = commands.call((node, "--version"), "node-version").strip()
        if node_version != "v" + canonical_versions(automation)["node"]:
            raise ValidationError("host Node differs from selected canonical automation authority")
        actual_cli = commands.call((node, str(cli), "--version"), "cli-version").strip()
        if actual_cli != cli_version:
            raise ValidationError("installed official CLI differs from its canonical lock")
        report["cli_version"] = cli_version
        # CLI build does not run lifecycle hooks. Runtime comes from our finite argv,
        # never candidate runArgs, mounts, host hooks, devcontainer up or a socket.
        if commands.call(
            (*engine_argv, "image", "ls", tag, "--format", "{{.ID}}"), "owned-tag-absence"
        ).strip():
            raise ValidationError("unique qualification image tag already exists")
        if commands.call(
            (
                *engine_argv,
                "container",
                "ls",
                "--all",
                "--filter",
                "name=" + name,
                "--format",
                "{{.ID}}",
            ),
            "owned-container-absence",
        ).strip():
            raise ValidationError("unique qualification container name already exists")
        build_started = True
        output = commands.call(
            (
                node,
                str(cli),
                "build",
                "--workspace-folder",
                str(snapshot),
                "--config",
                str(snapshot / CONFIG),
                "--docker-path",
                executable,
                "--platform",
                "linux/" + ARCHITECTURES[machine],
                "--image-name",
                tag,
                "--experimental-frozen-lockfile",
            ),
            "build",
            BUILD_SECONDS,
        )
        lines = output.strip().splitlines()
        if not lines:
            raise ValidationError("official CLI returned no build outcome")
        result = _json(lines[-1])
        if result.get("outcome") != "success" or result.get("imageName") != [tag]:
            raise ValidationError(
                "official CLI did not return the exact requested built-image identity"
            )
        image_id = inspect_image(
            json.loads(commands.call((*engine_argv, "image", "inspect", tag), "image-inspect")),
            ARCHITECTURES[machine],
        )
        report["image_id"] = image_id
        commands.budget = Budget(
            private, commands.budget.initial_free, time.monotonic() + PROBE_SECONDS
        )
        # Copy inert passwd bytes from a held stopped container. No untracked
        # `run --rm` discovery process can outlive cancellation or escape cleanup.
        created = True
        container_id = commands.call(
            (
                *engine_argv,
                "create",
                "--name",
                name,
                "--label",
                f"io.boxferry.devcontainer.run={token}",
                "--network",
                "none",
                "--read-only",
                "--user",
                config["remoteUser"],
                "--entrypoint",
                "/bin/true",
                image_id,
            ),
            "create-user-inspection",
        ).strip()
        if re.fullmatch(r"[a-f0-9]{64}", container_id) is None:
            raise ValidationError("engine returned an invalid stopped-container identity")
        passwd = private / "image-passwd"
        commands.call(
            (*engine_argv, "cp", f"{container_id}:/etc/passwd", str(passwd)), "read-image-user"
        )
        identity, group = passwd_identity(passwd, config["remoteUser"])
        remove_owned_container(commands, engine_argv, container_id, token)
        created, container_id = False, None
        report["uid"], report["gid"] = int(identity), int(group)
        mounts: list[str] = []
        for item, path in snapshots.items():
            target = (
                config["workspaceFolder"]
                if item == repository
                else f"/workspaces/boxferry/.boxferry-workspace/{item}"
            )
            mounts.extend(("--mount", f"type=bind,source={path},target={target},readonly"))
        for key in ("CARGO_HOME", "GH_CONFIG_DIR", "CARGO_TARGET_DIR"):
            if key not in config["containerEnv"]:
                continue
            mounts.extend(
                (
                    "--tmpfs",
                    f"{config['containerEnv'][key]}:rw,nosuid,nodev,uid={identity},gid={group},mode=0700,size=256m",
                )
            )
        argv = (
            *engine_argv,
            "create",
            "--name",
            name,
            "--label",
            f"io.boxferry.devcontainer.run={token}",
            "--network",
            "none",
            "--read-only",
            "--cap-drop",
            "ALL",
            "--security-opt",
            "no-new-privileges",
            "--pids-limit",
            "512",
            "--memory",
            "4g",
            "--cpus",
            "2",
            "--tmpfs",
            "/tmp:rw,nosuid,nodev,size=256m",
            "--user",
            f"{identity}:{group}",
            "--workdir",
            config["workspaceFolder"],
            *mounts,
            "--entrypoint",
            "/usr/bin/env",
            image_id,
            "-i",
            *_runtime_environment(config, sources, repository),
            "/bin/bash",
            "-c",
            probe_script(canonical_versions(automation), sources, repository),
            "qualification",
            machine,
        )
        created = True  # Track interrupted creates by unique name plus held owner label.
        container_id = commands.call(argv, "create-probe").strip()
        if re.fullmatch(r"[a-f0-9]{64}", container_id) is None:
            raise ValidationError("engine did not return one exact owned container ID")
        commands.call(
            (*engine_argv, "start", "--attach", container_id),
            "tooling-workspace-probe",
            PROBE_SECONDS,
        )
        state = _inspection(
            commands.call((*engine_argv, "container", "inspect", container_id), "probe-state")
        )
        if (
            not isinstance(state.get("State"), dict)
            or state["State"].get("ExitCode") != 0
            or _image_identity(state.get("Image")) != image_id
        ):
            raise ValidationError(
                "exact-image verifier exited unsuccessfully or changed image identity"
            )
        _held_revision(source, candidate, commands, clean=mode != "local")
        _held_revision(policy, policy_sha, commands)
        _held_revision(automation, automation_sha, commands)
        if (
            fingerprint(source, commands) != held
            or build_context_fingerprint(snapshot, commands, ignores) != context_held
        ):
            raise ValidationError("candidate input closure changed during image qualification")
        report["status"] = "success"
    except (OSError, ValueError, RunCancelled, KeyboardInterrupt) as error:
        report["cause"] = redact(str(error), protected_values(os.environ))[:4096]
    finally:
        commands.budget = Budget(
            private,
            shutil.disk_usage(private).free,
            time.monotonic() + CLEANUP_SECONDS,
            check_disk=False,
        )
        try:
            if created:
                remove_owned_container(commands, engine_argv, container_id or name, token)
            if build_started:
                inspected_id = _inspection(
                    commands.call((*engine_argv, "image", "inspect", tag), "cleanup-image-identity")
                ).get("Id")
                cleanup_image_id = _image_identity(inspected_id)
                report["cleanup_image_id"] = cleanup_image_id
                if image_id is not None and cleanup_image_id != image_id:
                    raise ValidationError("cleanup refuses a substituted image tag")
                # Remove only our tag. Never force-remove a shared image ID or prune.
                commands.call((*engine_argv, "image", "rm", tag), "cleanup-image-tag")
                remaining = commands.call(
                    (*engine_argv, "image", "ls", tag, "--format", "{{.Repository}}:{{.Tag}}"),
                    "cleanup-image-absence",
                )
                if remaining.strip():
                    raise ValidationError("owned image tag remains after cleanup")
            report["cleanup"] = "success"
        except (OSError, ValueError, RunCancelled, KeyboardInterrupt) as error:
            report["status"], report["cleanup"] = "failure", "failure"
            report["cleanup_cause"] = redact(str(error), protected_values(os.environ))[:4096]
        report["outcomes"] = commands.outcomes
        # Private Podman stores must be empty/unmounted before directory removal;
        # normal shutdown never deletes another store or global Docker resources.
        if selected == "podman" and (private / "store").exists() and report["cleanup"] == "success":
            try:
                if mode == "hosted":
                    owned = tuple(
                        str(private / name)
                        for name in (
                            "store",
                            "runroot",
                            "podman-tmp",
                            "home",
                            "config",
                            "cache",
                            "runtime",
                            "docker-config",
                        )
                    )
                    commands.call(
                        (
                            "/usr/bin/sudo",
                            "-n",
                            "/usr/bin/timeout",
                            "--signal=TERM",
                            "--kill-after=2s",
                            "60s",
                            "/bin/rm",
                            "-rf",
                            "--",
                            *owned,
                        ),
                        "cleanup-private-rootful-store",
                        65,
                    )
                else:
                    commands.call(
                        (*engine_argv, "unshare", "rm", "-rf", str(private / "store")),
                        "cleanup-private-store",
                        60,
                    )
            except (OSError, ValueError, RunCancelled, KeyboardInterrupt) as error:
                report["status"], report["cleanup"] = "failure", "failure"
                report["cleanup_cause"] = str(error)[:4096]
        try:
            if report["cleanup"] == "success":
                shutil.rmtree(private)
            else:
                report["retained_private_directory"] = str(private)
        except OSError as error:
            report["status"], report["cleanup"] = "failure", "failure"
            report["cleanup_cause"] = str(error)[:4096]
    return report


def local_sources(
    repository: str, candidate: str, source: Path, commands: Commands
) -> dict[str, object]:
    revisions = {repository: candidate}
    if repository == "boxferry":
        for name in SIBLINGS:
            path = source.parent / name
            revision = exact_sha(
                commands.git(path, "rev-parse", "HEAD").strip(), "local sibling SHA"
            )
            _held_revision(path, revision, commands)
            _verify_origin(path, name, commands)
            revisions[name] = revision
    return {"schema": 1, "repositories": revisions}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("resolve", "qualify", "prepare-podman"))
    parser.add_argument("--repository", required=True, choices=sorted(OWNERS))
    parser.add_argument("--candidate-sha", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--github-output", type=Path)
    parser.add_argument("--source", type=Path)
    parser.add_argument("--policy", type=Path)
    parser.add_argument("--automation", type=Path)
    parser.add_argument("--policy-sha")
    parser.add_argument("--automation-sha")
    parser.add_argument("--sources-json")
    parser.add_argument("--engine", choices=("podman", "docker"))
    parser.add_argument("--architecture", choices=("amd64", "arm64"))
    parser.add_argument("--mode", choices=("local", "hosted"), default="hosted")
    args = parser.parse_args(argv)
    result: dict[str, object]
    previous = signal.signal(
        signal.SIGTERM,
        lambda _signum, _frame: (_ for _ in ()).throw(RunCancelled("workflow cancellation")),
    )
    try:
        if args.command == "prepare-podman":
            with tempfile.TemporaryDirectory(prefix="bfv-prerequisite-") as temporary:
                commands = Commands(Path(temporary))
                result = {
                    "schema": 1,
                    "status": "success",
                    "prerequisite": prepare_hosted_podman(commands),
                    "outcomes": commands.outcomes,
                }
        elif args.command == "resolve":
            result = resolve_sources(args.repository, args.candidate_sha)
        else:
            if not all(
                (
                    args.source,
                    args.policy,
                    args.automation,
                    args.policy_sha,
                    args.automation_sha,
                    args.engine,
                )
            ):
                raise ValidationError(
                    "qualification requires explicit candidate, policy, automation "
                    "and backend identities"
                )
            args.output.parent.mkdir(parents=True, exist_ok=True)
            if args.sources_json:
                manifest = _json(args.sources_json)
            elif args.mode == "local":
                with tempfile.TemporaryDirectory(prefix="devcontainer-local-sources-") as temporary:
                    manifest = local_sources(
                        args.repository, args.candidate_sha, args.source, Commands(Path(temporary))
                    )
            else:
                raise ValidationError(
                    "hosted qualification needs the once-resolved shared source manifest"
                )
            result = qualify(
                repository=args.repository,
                candidate=args.candidate_sha,
                policy_sha=args.policy_sha,
                automation_sha=args.automation_sha,
                source=args.source.resolve(),
                policy=args.policy.resolve(),
                automation=args.automation.resolve(),
                manifest=manifest,
                engine=args.engine,
                mode=args.mode,
                work=args.output.parent.resolve(),
                expected_architecture=args.architecture,
            )
    except (OSError, ValueError, RunCancelled, KeyboardInterrupt) as error:
        result = {
            "schema": 1,
            "status": "failure",
            "cause": redact(str(error), protected_values(os.environ))[:4096],
        }
    finally:
        signal.signal(signal.SIGTERM, previous)
    encoded = json.dumps(result, sort_keys=True)
    if len(encoded.encode()) > MAX_REPORT:
        result = {
            "schema": 1,
            "status": "failure",
            "cause": "qualification report exceeds its byte bound",
        }
        encoded = json.dumps(result)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(encoded + "\n")
    if args.github_output and args.command == "resolve" and "repositories" in result:
        with args.github_output.open("a") as stream:
            stream.write(f"sources={encoded}\nsource_digest={manifest_digest(result)}\n")
            if args.automation:
                stream.write(f"node_version={canonical_versions(args.automation)['node']}\n")
    if args.github_output and args.command == "qualify" and result.get("status") == "success":
        with args.github_output.open("a") as stream:
            for name in (
                "candidate_sha",
                "source_manifest_sha256",
                "input_sha256",
                "build_context_sha256",
                "architecture",
            ):
                stream.write(f"{name}={result[name]}\n")
    return (
        0
        if (args.command == "resolve" and "repositories" in result)
        or result.get("status") == "success"
        else 1
    )


if __name__ == "__main__":
    sys.exit(main())
