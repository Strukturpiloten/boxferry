"""Build a held candidate and execute client probes in its exact native image.

This producer performs no publication or application/native-runtime qualification.
Two hosted lanes share one immutable seven-repository source resolution. Local
development uses VS Code Dev Containers with the developer's configured Podman store.
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

from . import git_worktree_validation
from .contracts import ContractError, exact_sha
from .dependencies import fetch_metadata
from .operational import _json
from .runner import (
    RunCancelled,
    _argument_secrets,
    _redacted_argv,
    protected_values,
    redact,
    run,
)

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
MAX_FAILURE_STREAM = 2048
_CAUSAL_LINE = re.compile(r"\b(error|failed|failure|fatal|denied|cannot|unable|not found)\b", re.I)


def _bounded_text(value: str, limit: int) -> str:
    return value.encode("utf-8")[:limit].decode("utf-8", "ignore")


def failure_evidence(value: str) -> dict[str, object]:
    """Keep initial context and recent causal lines, including long terminal suffixes."""
    causes = []
    for line in value.splitlines():
        if match := _CAUSAL_LINE.search(line):
            context = line[max(0, match.start() - 64) :]
            if len(context.encode("utf-8")) > 176:
                context = (
                    _bounded_text(context, 48)
                    + " [... line omitted ...] "
                    + context.encode("utf-8")[-104:].decode("utf-8", "ignore")
                )
            causes.append(context)
    if len(causes) > 4:
        causes = [causes[0], "[... earlier causal lines omitted ...]", *causes[-3:]]
    causal = "\n".join(causes)
    if len(value.encode("utf-8")) <= MAX_FAILURE_STREAM:
        excerpt = value
    else:
        excerpt = (
            _bounded_text(value, 512)
            + "\n[... causal lines ...]\n"
            + _bounded_text(causal, 896)
            + "\n[... final context ...]\n"
            + value.encode("utf-8")[-512:].decode("utf-8", "ignore")
        )
    return {
        "excerpt": excerpt,
        "truncated": excerpt != value,
        "causal": _bounded_text(causal or value, 768),
    }


# Observation is disabled until a fresh exact-owned opaque capture and source review.
PROBES: tuple[tuple[str, ...], ...] = (
    ("actionlint", "--version"),
    ("cargo", "--version"),
    ("cargo-clippy", "--version"),
    ("cargo-deny", "--version"),
    ("cargo-llvm-cov", "llvm-cov", "--version"),
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
    evidence_paths: tuple[Path, ...] = ()

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
        self,
        argv: tuple[str, ...],
        stage: str,
        timeout: int = 30,
        *,
        cwd: Path | None = None,
        max_output_bytes: int = 2_000_000,
    ) -> str:
        started = time.monotonic()
        hidden = (*protected_values(os.environ), *_argument_secrets(argv))
        home = os.environ.get("HOME", "")
        evidence_hidden = (
            *hidden,
            str(self.private.resolve()),
            str((cwd or self.private).resolve()),
            *(str(path.resolve()) for path in self.evidence_paths),
            *((home,) if home.startswith("/") and home != "/" else ()),
        )
        safe = shlex.join(_redacted_argv(argv, evidence_hidden, cwd or self.private))
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
                max_output_bytes=max_output_bytes,
            )
            outcome.update(status=result.status, cause=result.diagnostic.cause)
            if result.abort_hook_failure:
                outcome["abort_hook_failure"] = result.abort_hook_failure
            if result.status != "success":
                evidence = {
                    name: failure_evidence(redact(value, evidence_hidden))
                    for name, value in (("stderr", result.stderr), ("stdout", result.stdout))
                }
                outcome["failure_evidence"] = evidence
                detail = "; ".join(
                    f"{name}: {item['causal']}" for name, item in evidence.items() if item["causal"]
                )
                raise ValidationError(f"{result.diagnostic.cause}; {detail}")
            return result.stdout
        except (OSError, ValueError, RunCancelled, KeyboardInterrupt) as error:
            cause = redact(str(error), evidence_hidden)[:2048]
            if isinstance(error, (RunCancelled, KeyboardInterrupt)):
                outcome["status"] = "cancelled"
            outcome["cause"] = cause
            if secondary := getattr(error, "abort_hook_failure", ""):
                outcome["abort_hook_failure"] = secondary
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
    default_workspace = repository == "boxferry" and not (
        "workspaceFolder" in document or "workspaceMount" in document
    )
    if not default_workspace and (
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
    permitted_mount_sets = [expected]
    if repository == "boxferry":
        # Admit the historical explicit workspace during the source transition;
        # fresh single-clone VS Code setup needs only the ordinary named volumes.
        permitted_mount_sets.append(
            expected
            | {
                f"source=${{localWorkspaceFolder}}/../{name},target={workspace}/.boxferry-workspace/{name},type=bind"
                for name in SIBLINGS
            }
        )
    mounts = document.get("mounts")
    if (
        not isinstance(mounts, list)
        or any(not isinstance(item, str) for item in mounts)
        or len(mounts) != len(set(mounts))
        or set(mounts) not in permitted_mount_sets
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
    # Runtime probes use a CI-owned explicit target even when VS Code's local
    # configuration relies on its normal single-clone workspace defaults.
    if default_workspace:
        document["workspaceFolder"] = workspace
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


def markdownlint_versions(candidate: Path) -> dict[str, str]:
    """Bind the help banner to the image-owned candidate's exact npm graph."""

    def unique(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            if key in result:
                raise ValidationError("markdownlint metadata contains ambiguous duplicate keys")
            result[key] = value
        return result

    documents = []
    for relative in ("package.json", "package-lock.json"):
        path = candidate / relative
        try:
            if not stat.S_ISREG(path.lstat().st_mode) or path.stat().st_size > 2 * 1024**2:
                raise ValidationError("markdownlint metadata must be bounded regular JSON files")
            with path.open("rb") as stream:
                data = stream.read(2 * 1024**2 + 1)
            if len(data) > 2 * 1024**2:
                raise ValidationError("markdownlint metadata exceeded its byte bound while reading")
            try:
                document = json.loads(data, object_pairs_hook=unique)
            except RecursionError as error:
                raise ValidationError(
                    "candidate markdownlint package/lock JSON exceeds supported nesting depth"
                ) from error
        except (OSError, UnicodeError, json.JSONDecodeError) as error:
            raise ValidationError(
                "candidate markdownlint package/lock metadata is unavailable"
            ) from error
        if not isinstance(document, dict):
            raise ValidationError("candidate markdownlint package/lock metadata must be objects")
        documents.append(document)
    package, lock = documents
    sections = ("dependencies", "devDependencies", "optionalDependencies", "peerDependencies")

    def declaration(document: dict[str, Any]) -> tuple[str, str]:
        declarations = []
        for section in sections:
            values = document.get(section, {})
            if not isinstance(values, dict):
                raise ValidationError("candidate markdownlint declarations are malformed")
            if "markdownlint-cli2" in values:
                declarations.append((section, values["markdownlint-cli2"]))
        if len(declarations) != 1 or declarations[0][0] not in sections[:2]:
            raise ValidationError(
                "candidate needs one supported exact markdownlint-cli2 declaration"
            )
        section, version = declarations[0]
        if not isinstance(version, str) or re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version) is None:
            raise ValidationError("candidate markdownlint-cli2 version must be exact numeric")
        return section, version

    declared = declaration(package)
    packages = lock.get("packages")
    if lock.get("lockfileVersion") != 3 or not isinstance(packages, dict):
        raise ValidationError("candidate markdownlint needs its npm v3 integrity lock")
    root = packages.get("")
    if not isinstance(root, dict) or declaration(root) != declared:
        raise ValidationError("candidate markdownlint package and lock root disagree")
    versions = {}
    for name in ("markdownlint-cli2", "markdownlint"):
        matches = [
            key
            for key in packages
            if key == "node_modules/" + name or key.endswith("/node_modules/" + name)
        ]
        if name == "markdownlint":
            # npm resolves the CLI's own nested dependency before its hoisted peer.
            matches = [
                key
                for key in (
                    "node_modules/markdownlint-cli2/node_modules/markdownlint",
                    "node_modules/markdownlint",
                )
                if key in packages
            ][:1]
        if len(matches) != 1 or (
            name == "markdownlint-cli2" and matches[0] != "node_modules/markdownlint-cli2"
        ):
            raise ValidationError("candidate markdownlint lock identity is absent or ambiguous")
        record = packages[matches[0]]
        if not isinstance(record, dict):
            raise ValidationError("candidate markdownlint lock record is malformed")
        version = record.get("version")
        if (
            not isinstance(version, str)
            or re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version) is None
            or record.get("resolved") != f"https://registry.npmjs.org/{name}/-/{name}-{version}.tgz"
            or not isinstance(record.get("integrity"), str)
            or re.fullmatch(r"sha512-[A-Za-z0-9+/]{86}==", record["integrity"]) is None
        ):
            raise ValidationError("candidate markdownlint version/source/integrity is inconsistent")
        versions[name] = version
    cli = packages["node_modules/markdownlint-cli2"]
    if (
        versions["markdownlint-cli2"] != declared[1]
        or not isinstance(cli.get("dependencies"), dict)
        or cli["dependencies"].get("markdownlint") != versions["markdownlint"]
        or cli.get("bin") != {"markdownlint-cli2": "markdownlint-cli2-bin.mjs"}
    ):
        raise ValidationError("candidate markdownlint executable/dependency graph disagrees")
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
    if mode != "hosted":
        raise ValidationError(
            "image qualification is hosted CI only; use VS Code to rebuild locally"
        )
    expected = "podman" if repository in {"podman-lens", "quadlet-lens"} else "docker"
    if selected != expected:
        raise ValidationError("backend differs from reviewed hosted runtime policy; no fallback")
    return expected


def validate_hosted_runner(values: Mapping[str, str]) -> None:
    if (
        values.get("GITHUB_ACTIONS") != "true"
        or values.get("RUNNER_ENVIRONMENT") != "github-hosted"
        or values.get("RUNNER_OS") != "Linux"
    ):
        raise ValidationError("image qualification requires a fresh GitHub-hosted Linux runner")


def validate_hosted_podman_host(environment_values: Mapping[str, str], release: str) -> None:
    validate_hosted_runner(environment_values)
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


def serialize_report(report: Mapping[str, object]) -> str:
    """Retain bounded identities, primary and cleanup causes when detail overflows."""
    encoded = json.dumps(report, sort_keys=True)
    if len(encoded.encode()) <= MAX_REPORT:
        return encoded
    compact: dict[str, object] = {
        "schema": 1,
        "status": "failure",
        "cause": "qualification report exceeds its byte bound",
    }
    for key in (
        "mode",
        "engine",
        "outer_mode",
        "caller_uid",
        "opaque_failure_mode",
        "opaque_failure_reason",
        "opaque_failure_status",
        "opaque_failure_argv_sha256",
        "opaque_failure_program_sha256",
        "opaque_failure_stdout_prefix_bytes",
        "opaque_failure_stderr_prefix_bytes",
        "opaque_failure_stdout_prefix_sha256",
        "opaque_failure_stderr_prefix_sha256",
        "command_failure_stage",
        "command_failure_status",
        "command_failure_cause",
        "abort_hook_failure",
        "engine_client_version",
        "engine_storage_driver",
        "engine_graph_root",
        "engine_run_root",
        "engine_remote",
        "candidate_sha",
        "policy_sha",
        "automation_sha",
        "input_sha256",
        "build_context_sha256",
        "architecture",
        "source_manifest_sha256",
        "run_nonce",
        "cleanup",
        "cleanup_cause",
        "retained_private_directory",
        "next_action",
    ):
        value = report.get(key)
        if isinstance(value, (str, int, bool)):
            compact[key] = (
                _bounded_text(redact(str(value), protected_values(os.environ)), 1024)
                if isinstance(value, str)
                else value
            )
    for name in ("docker_storage", "podman_storage"):
        facts = report.get(name)
        if isinstance(facts, dict):
            compact[name] = {
                key: _bounded_text(value, 1024) if isinstance(value, str) else value
                for key, value in facts.items()
                if isinstance(key, str)
                and len(key) < 64
                and (value is None or isinstance(value, (str, int, bool)))
            }
    if report.get("cause"):
        compact["primary_cause"] = _bounded_text(
            redact(str(report["cause"]), protected_values(os.environ)), 2048
        )
    return json.dumps(compact, sort_keys=True)


def _hosted_root_command(seconds: int) -> tuple[str, ...]:
    """Bound the privileged child itself and use ordinary rootful runner defaults."""
    return (
        "/usr/bin/sudo",
        "-n",
        "/usr/bin/timeout",
        "--signal=TERM",
        "--kill-after=2s",
        f"{seconds}s",
        "/usr/bin/env",
        "-i",
        "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        "HOME=/root",
        "LC_ALL=C.UTF-8",
    )


def _engine(private: Path, engine: str) -> str:
    if engine == "docker":
        return "/usr/bin/docker"
    launcher = private / "podman"
    # The official CLI expects one executable. Keep its reviewed hosted rootful
    # backend, without redirecting storage/configuration to qualification scratch.
    prefix = (
        '#!/bin/sh\ncase "$1" in\n'
        "build) seconds=3600 ;;\n"
        'buildx) if [ "${2:-}" = build ]; then seconds=3600; else seconds=30; fi ;;\n'
        "start) seconds=300 ;;\n"
        "*) seconds=30 ;;\nesac\n"
    )
    privilege = _hosted_root_command(30)
    fixed = shlex.join(privilege[:5]) + ' "${seconds}s" ' + shlex.join(privilege[6:])
    launcher.write_text(prefix + "exec " + fixed + ' /usr/bin/podman --remote=false "$@"\n')
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


def validate_engine(document: object, selected: str, architecture: str) -> None:
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
            or not isinstance(host.get("security"), dict)
            or host["security"].get("rootless") is not False
            or any(
                not isinstance(store.get(key), str)
                or not store[key].startswith("/")
                or "\x00" in store[key]
                for key in ("graphRoot", "runRoot")
            )
            or not isinstance(store.get("graphDriverName"), str)
            or not store["graphDriverName"]
        ):
            raise ValidationError(
                "Podman requires observed standard rootful local storage and native architecture"
            )


def validate_docker_budget_filesystem(
    private: Path, docker_root: str, report: dict[str, object]
) -> None:
    """Require the monitored directory and resolved hosted Docker store to share st_dev.

    This checks coverage of the existing Budget without rebaselining free space;
    it establishes no cold-build budget or native qualification evidence.
    """
    facts: dict[str, object] = {
        "reported_docker_root_dir": docker_root,
        "docker_root_dir": None,
        "budget_path": None,
        "budget_device": None,
        "docker_device": None,
        "same_filesystem": None,
    }
    report["docker_storage"] = facts
    try:
        budget = private.resolve(strict=True)
        facts["budget_path"] = str(budget)
        budget_stat = budget.stat()
        facts["budget_device"] = budget_stat.st_dev
        docker = Path(docker_root).resolve(strict=True)
        facts["docker_root_dir"] = str(docker)
        docker_stat = docker.stat()
        facts["docker_device"] = docker_stat.st_dev
    except (OSError, ValueError) as error:
        raise ValidationError(
            "Hosted Docker budget coverage needs existing observable DockerRootDir and "
            f"private budget directories on the same filesystem; verify runner storage: {error}"
        ) from error
    if not stat.S_ISDIR(budget_stat.st_mode) or not stat.S_ISDIR(docker_stat.st_mode):
        raise ValidationError(
            "DockerRootDir and private budget path must be existing directories; "
            "verify the hosted runner storage layout"
        )
    facts["same_filesystem"] = budget_stat.st_dev == docker_stat.st_dev
    if not facts["same_filesystem"]:
        raise ValidationError(
            "DockerRootDir and private budget path use different filesystems; "
            "use a hosted runner where both existing directories share a filesystem"
        )


PODMAN_STORAGE_STAT = """import json, os, stat, sys
path = os.path.realpath(sys.argv[1], strict=True)
observed = os.stat(path)
if not stat.S_ISDIR(observed.st_mode):
    raise ValueError("Podman graphRoot is not a directory")
print(json.dumps({"path": path, "device": observed.st_dev}))
"""


def validate_podman_budget_filesystem(
    commands: Commands, private: Path, graph_root: str, report: dict[str, object]
) -> None:
    """Observe only directory metadata; never enumerate or delete runner storage."""
    facts: dict[str, object] = {
        "reported_graph_root": graph_root,
        "graph_root": None,
        "budget_path": None,
        "budget_device": None,
        "podman_device": None,
        "same_filesystem": None,
    }
    report["podman_storage"] = facts
    try:
        if not graph_root.startswith("/") or "\x00" in graph_root:
            raise ValueError("graphRoot must be an absolute path")
        budget = private.resolve(strict=True)
        observed = budget.stat()
        if not stat.S_ISDIR(observed.st_mode):
            raise ValueError("budget path is not a directory")
        facts.update(budget_path=str(budget), budget_device=observed.st_dev)
        document = _json(
            commands.call(
                (
                    *_hosted_root_command(5),
                    "/usr/bin/python3",
                    "-I",
                    "-S",
                    "-c",
                    PODMAN_STORAGE_STAT,
                    graph_root,
                ),
                "podman-storage-stat",
                8,
                max_output_bytes=4096,
            )
        )
        if (
            set(document) != {"path", "device"}
            or not isinstance(document["path"], str)
            or not document["path"].startswith("/")
            or "\x00" in document["path"]
            or type(document["device"]) is not int
            or document["device"] < 0
        ):
            raise ValueError("actual graphRoot metadata is malformed")
        facts.update(graph_root=document["path"], podman_device=document["device"])
        facts["same_filesystem"] = observed.st_dev == document["device"]
        if not facts["same_filesystem"]:
            raise ValueError("actual graphRoot and budget path use different filesystems")
    except (OSError, ValueError) as error:
        raise ValidationError(
            "Hosted Podman budget coverage requires observable graphRoot and budget directories "
            f"on the same filesystem; verify runner storage: {error}"
        ) from error


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


def cleanup_image_tag(
    commands: Commands,
    engine: tuple[str, ...],
    tag: str,
    image_id: str | None,
    build_completed: bool,
    report: dict[str, object],
) -> None:
    # A failed build may leave layers without ever creating the requested tag.
    # A successful listing, rather than an arbitrary inspect failure, proves absence.
    listed = commands.call(
        (
            *engine,
            "image",
            "ls",
            "--all",
            "--no-trunc",
            "--filter",
            "reference=" + tag,
            "--format",
            "{{.ID}}",
        ),
        "cleanup-image-list",
    ).strip()
    if not listed:
        if build_completed:
            raise ValidationError("completed build is missing its exact image tag")
        report["cleanup_image_tag"] = "absent-after-failed-build"
        return
    listed_id = _image_identity(listed)
    inspected_id = _inspection(
        commands.call((*engine, "image", "inspect", tag), "cleanup-image-identity")
    ).get("Id")
    cleanup_image_id = _image_identity(inspected_id)
    report["cleanup_image_id"] = cleanup_image_id
    if cleanup_image_id != listed_id or (image_id is not None and cleanup_image_id != image_id):
        raise ValidationError("cleanup refuses substituted image tag")
    # Remove only our tag. Never force-remove a shared image ID or prune.
    commands.call((*engine, "image", "rm", tag), "cleanup-image-tag")
    remaining = commands.call(
        (
            *engine,
            "image",
            "ls",
            "--all",
            "--filter",
            "reference=" + tag,
            "--format",
            "{{.Repository}}:{{.Tag}}",
        ),
        "cleanup-image-absence",
    )
    if remaining.strip():
        raise ValidationError("owned image tag remains after cleanup")


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
        failure = (
            "printf 'Probe failed for %s: argv=%s; exit=%s\\n' "
            + shlex.quote(name)
            + " "
            + shlex.quote(command)
            + ' "$probe_status" >&2'
        )
        if name == "markdownlint-cli2":
            if argv != ("markdownlint-cli2", "--help") or any(
                re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", versions.get(tool, "")) is None
                for tool in ("markdownlint-cli2", "markdownlint")
            ):
                raise ValidationError(
                    "markdownlint help requires its exact candidate package/lock versions"
                )
            expected = (
                f"markdownlint-cli2 v{versions['markdownlint-cli2']} "
                f"(markdownlint v{versions['markdownlint']})"
            )
            lines.extend(
                (
                    "help_errors=$(mktemp /tmp/qualification-home/markdownlint-help.XXXXXX)",
                    "probe_status=0",
                    f'observed=$({command} 2>"$help_errors") || probe_status=$?',
                    "help_prefix=${observed:0:8192}",
                    "help_banner=; help_usage=",
                    "while IFS= read -r help_line; do",
                    '  if [[ -z "$help_banner" && -n "$help_line" ]]; then',
                    "    help_banner=$help_line",
                    "  fi",
                    '  if [[ "$help_line" == "Syntax: markdownlint-cli2 glob0 [glob1] "* ]]; then',
                    "    help_usage=$help_line",
                    "  fi",
                    'done <<< "$help_prefix"',
                    "expected=" + shlex.quote(expected),
                    'if [[ "$probe_status" != 2 || -s "$help_errors"',
                    '  || "$help_banner" != "$expected"',
                    "  || \"$help_prefix\" != *$'\\nhttps://github.com/DavidAnson/markdownlint-cli2\\n'*",
                    '  || "$help_usage" != *"[--help]"*',
                    '  || "$help_usage" != *"[--no-globs]"* ]]; then',
                    "  printf '%s\\n' \"$observed\"",
                    '  cat "$help_errors" >&2',
                    '  rm -f -- "$help_errors"',
                    "  " + failure.replace("Probe failed", "Probe help contract mismatch"),
                    '  if [[ "$probe_status" == 0 ]]; then exit 1; else exit "$probe_status"; fi',
                    "fi",
                    'rm -f -- "$help_errors"',
                )
            )
        elif name in versions:
            lines.extend(
                (
                    "probe_status=0",
                    f"observed=$({command}) || probe_status=$?",
                    'if [[ "$probe_status" != 0 ]]; then',
                    "  printf '%s\\n' \"$observed\"",
                    "  " + failure,
                    '  exit "$probe_status"',
                    "fi",
                )
            )
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
                    "  printf 'Version mismatch for %s: expected %.160s; observed %.160s; "
                    "argv=%s; exit=0\\n' "
                    + shlex.quote(name)
                    + ' "$expected" "$actual" '
                    + shlex.quote(command)
                    + " >&2",
                    "  exit 1",
                    "fi",
                )
            )
        else:
            lines.extend(
                (
                    "probe_status=0",
                    command + " || probe_status=$?",
                    'if [[ "$probe_status" != 0 ]]; then',
                    "  " + failure,
                    '  exit "$probe_status"',
                    "fi",
                )
            )
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
        f"https://github.com/Strukturpiloten/{repository}",
        f"https://github.com/Strukturpiloten/{repository}.git",
        f"git@github.com:Strukturpiloten/{repository}.git",
    }:
        raise ValidationError("source origin differs from its scoped workspace repository")


def probe_argv(
    engine_argv: tuple[str, ...],
    name: str,
    token: str,
    image_id: str,
    identity: str,
    group: str,
    workdir: str,
    mounts: list[str],
    runtime_environment: list[str],
    command: tuple[str, ...],
) -> tuple[str, ...]:
    """One exact-image probe profile, shared without changing its isolation flags."""
    return (
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
        workdir,
        *mounts,
        "--entrypoint",
        "/usr/bin/env",
        image_id,
        "-i",
        *runtime_environment,
        *command,
    )


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
) -> dict[str, object]:
    """Execute a fresh build and verifier; every error including cleanup blocks success."""
    if repository not in OWNERS:
        raise ValidationError("repository does not own a Dev Container")
    candidate = exact_sha(candidate, "candidate SHA")
    policy_sha = exact_sha(policy_sha, "policy SHA")
    automation_sha = exact_sha(automation_sha, "automation SHA")
    sources = parse_sources(manifest, repository, candidate)
    selected = backend(repository, mode, engine)
    validate_hosted_runner(os.environ)
    caller_uid = os.getuid()
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
    private = Path(tempfile.mkdtemp(prefix="devcontainer-", dir=work))
    private.chmod(0o700)
    commands = Commands(private)
    commands.evidence_paths = (source, policy, automation, work)
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
        "mode": mode,
        "caller_uid": caller_uid,
    }
    token = uuid.uuid4().hex
    name, tag = f"boxferry-devcontainer-{token}", f"localhost/boxferry-devcontainer:{token}"
    report["run_nonce"] = token
    executable = _engine(private, selected)
    engine_argv = (executable,)
    container_id: str | None = None
    image_id: str | None = None
    created = False
    build_started = False
    build_completed = False
    try:
        commands.budget = Budget.start(private, BUILD_SECONDS)
        _held_revision(source, candidate, commands, clean=True)
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
        validate_engine(info, selected, ARCHITECTURES[machine])
        if selected == "docker":
            validate_docker_budget_filesystem(private, info["DockerRootDir"], report)
        else:
            host, store = info["host"], info["store"]
            validate_podman_budget_filesystem(commands, private, store["graphRoot"], report)
            report.update(
                outer_mode="rootful",
                engine_client_version=version["Client"]["Version"],
                engine_storage_driver=store["graphDriverName"],
                engine_graph_root=store["graphRoot"],
                engine_run_root=store["runRoot"],
                engine_remote=host["serviceIsRemote"],
            )
            runtime = host.get("ociRuntime")
            if isinstance(runtime, dict):
                report["engine_runtime"] = {
                    key: _bounded_text(redact(runtime[key], protected_values(os.environ)), 512)
                    for key in ("name", "path", "package", "version")
                    if isinstance(runtime.get(key), str)
                }
        report["runner"] = {"os": "linux", "architecture": ARCHITECTURES[machine], "mode": mode}
        if selected == "podman":
            observed = info.get("version")
            if (
                not isinstance(observed, dict)
                or observed.get("Version") != version["Client"]["Version"]
            ):
                raise ValidationError("Podman info and client versions disagree")
            if prerequisite:
                prerequisite["version"] = observed["Version"]
        snapshots: dict[str, Path] = {}
        for name_in_manifest, revision in sources.items():
            destination = private / name_in_manifest
            if name_in_manifest == repository:
                remote = str(source.resolve())
            else:
                remote = f"https://github.com/Strukturpiloten/{name_in_manifest}.git"
            _checkout(destination, revision, commands, remote)
            snapshots[name_in_manifest] = destination
        snapshot = snapshots[repository]
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
        git_fixtures = (
            git_worktree_validation.prepare(private, automation, snapshot, commands)
            if repository == "boxferry"
            else None
        )
        if git_fixtures is not None:
            report["git_worktree_host"] = git_fixtures.expected
            host_evidence = work / f"git-worktree-host-{token}.json"
            with host_evidence.open("x") as stream:
                json.dump(git_fixtures.expected, stream, sort_keys=True)
        cli, cli_version = _cli(automation)
        node = shutil.which("node")
        if node is None or not Path(node).is_file():
            raise ValidationError("trusted Node executable is unavailable")
        probe_versions = canonical_versions(automation) | markdownlint_versions(snapshot)
        node_version = commands.call((node, "--version"), "node-version").strip()
        if node_version != "v" + probe_versions["node"]:
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
        build_completed = True
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
            target = config["containerEnv"][key]
            if selected == "podman":
                # Podman 4.9.3/6.1.4: tmpfs chown derives ownership from --user;
                # its --tmpfs parser rejects the Docker uid/gid option spelling.
                mounts.extend(
                    (
                        "--mount",
                        f"type=tmpfs,target={target},rw,nosuid,nodev,"
                        "tmpfs-size=268435456,tmpfs-mode=0700,chown=true",
                    )
                )
            else:
                mounts.extend(
                    (
                        "--tmpfs",
                        f"{target}:rw,nosuid,nodev,uid={identity},gid={group},mode=0700,size=256m",
                    )
                )
        argv = probe_argv(
            engine_argv,
            name,
            token,
            image_id,
            identity,
            group,
            config["workspaceFolder"],
            mounts,
            _runtime_environment(config, sources, repository),
            (
                "/bin/bash",
                "-c",
                probe_script(probe_versions, sources, repository),
                "qualification",
                machine,
            ),
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
        if git_fixtures is not None:
            # Reuse the exact image and original absolute 300-second probe deadline.
            remove_owned_container(commands, engine_argv, container_id, token)
            created, container_id = False, None
            program = (
                "import sys;sys.path.insert(0,'/opt/boxferry-git-contract');"
                "from boxferry_ci.git_worktree_validation import main;raise SystemExit(main())"
            )
            argv = probe_argv(
                engine_argv,
                name,
                token,
                image_id,
                identity,
                group,
                "/tmp",
                git_worktree_validation.mounts(git_fixtures, snapshot),
                ["PATH=/usr/local/bin:/usr/bin:/bin", "HOME=/tmp", "LC_ALL=C.UTF-8"],
                (
                    "python3",
                    "-I",
                    "-c",
                    git_worktree_validation.IMAGE_PYTHON,
                    automation_sha,
                    program,
                ),
            )
            created = True
            container_id = commands.call(argv, "create-git-worktree-probe").strip()
            if re.fullmatch(r"[a-f0-9]{64}", container_id) is None:
                raise ValidationError("engine returned an invalid Git probe container identity")
            raw = commands.call(
                (*engine_argv, "start", "--attach", container_id),
                "git-worktree-probe",
                PROBE_SECONDS,
                max_output_bytes=git_worktree_validation.MAX_RECEIPT,
            )
            raw_evidence = work / f"git-worktree-probe-{token}.json"
            with raw_evidence.open("x") as stream:
                stream.write(raw)
            state = _inspection(
                commands.call(
                    (*engine_argv, "container", "inspect", container_id), "git-probe-state"
                )
            )
            if (
                not isinstance(state.get("State"), dict)
                or state["State"].get("ExitCode") != 0
                or _image_identity(state.get("Image")) != image_id
            ):
                raise ValidationError("Git fixture probe failed or changed its exact image")
            report["git_worktree_contract"] = git_worktree_validation.verify_receipt(
                raw, git_fixtures, machine, int(identity), int(group)
            )
            contract = report["git_worktree_contract"]
            assert isinstance(contract, dict)
            contract.update(
                image_id=image_id,
                container_id=container_id,
                container_exit_code=0,
                candidate_sha=candidate,
                policy_sha=policy_sha,
                automation_sha=automation_sha,
            )
            report.pop("git_worktree_host", None)
            report["git_worktree_status"] = "success"
            report["git_worktree_spec_sha256"] = git_worktree_validation.SPEC_SHA256
        _held_revision(source, candidate, commands, clean=True)
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
            commands.budget.initial_free if commands.budget else 0,
            time.monotonic() + CLEANUP_SECONDS,
            check_disk=False,
        )
        try:
            if created:
                remove_owned_container(commands, engine_argv, container_id or name, token)
            if build_started:
                cleanup_image_tag(commands, engine_argv, tag, image_id, build_completed, report)
            report["cleanup"] = "success"
        except (OSError, ValueError, RunCancelled, KeyboardInterrupt) as error:
            report["status"], report["cleanup"] = "failure", "failure"
            report["cleanup_cause"] = redact(str(error), protected_values(os.environ))[:4096]
        report["outcomes"] = commands.outcomes
        try:
            if report["cleanup"] == "success":
                shutil.rmtree(private)
            else:
                report["retained_private_directory"] = str(private)
        except OSError as error:
            report["status"], report["cleanup"] = "failure", "failure"
            report["cleanup_cause"] = redact(str(error), protected_values(os.environ))[:4096]
    return report


def _replace_resolution_file(path: Path, data: bytes, mode: int = 0o600) -> None:
    """Publish a complete command/report file; remove only our uncommitted temporary."""
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="wb", dir=path.parent, prefix=".devcontainer-sources-", delete=False
        ) as stream:
            temporary = Path(stream.name)
            if stream.write(data) != len(data):
                raise OSError("source-resolution file write was incomplete")
            os.fchmod(stream.fileno(), mode)
        temporary.replace(path)
        temporary = None
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def _write_resolution_report(path: Path, report: Mapping[str, object]) -> None:
    """Atomically retain the bounded report in a private regular file."""
    _replace_resolution_file(path, (serialize_report(report) + "\n").encode())


def _append_resolution_outputs(path: Path, exports: str) -> None:
    """Preserve prior command-file bytes and commit all new outputs together."""
    previous = b""
    mode = 0o600
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK)
    except FileNotFoundError:
        pass
    else:
        try:
            info = os.fstat(descriptor)
            if not stat.S_ISREG(info.st_mode):
                raise ValidationError("GitHub output destination is not a regular file")
            with os.fdopen(descriptor, "rb", closefd=False) as stream:
                previous = stream.read(MAX_REPORT + 1)
            mode = stat.S_IMODE(info.st_mode) & 0o777
        finally:
            os.close(descriptor)
        if len(previous) > MAX_REPORT:
            raise ValidationError("existing GitHub output exceeds the source-report byte bound")
    _replace_resolution_file(path, previous + exports.encode(), mode)


def _log_resolution_fallback(report: Mapping[str, object]) -> None:
    """Preserve the safe original cause without emitting executable workflow commands."""
    fallback = dict(report)
    cause = str(report["cause"]).replace(":", ": ").replace("##[", "# #[")
    while True:
        fallback["cause"] = cause
        fallback["cause_truncated"] = cause != report["cause"]
        encoded = json.dumps(fallback, sort_keys=True)
        if len(encoded.encode()) <= MAX_FAILURE_STREAM:
            print(encoded, file=sys.stderr)
            return
        cause = _bounded_text(cause, max(1, len(cause.encode()) // 2))


def resolve_cli(args: argparse.Namespace) -> int:
    """A failed source resolution must retain diagnostics without exporting acceptance."""
    bindings: dict[str, object] = {"repository": args.repository}
    for name in ("candidate_sha", "policy_sha", "automation_sha"):
        value = getattr(args, name)
        bindings[name] = (
            value if isinstance(value, str) and re.fullmatch(r"[a-f0-9]{40}", value) else None
        )
    stage = "identity"
    try:
        candidate = exact_sha(args.candidate_sha, "candidate SHA")
        for name in ("policy_sha", "automation_sha"):
            if value := getattr(args, name):
                exact_sha(value, name)
        stage = "metadata"
        result = resolve_sources(args.repository, candidate)
        parse_sources(result, args.repository, candidate)
        exports = f"sources={serialize_report(result)}\nsource_digest={manifest_digest(result)}\n"
        stage = "tool-authority"
        if args.github_output and args.automation:
            exports += f"node_version={canonical_versions(args.automation)['node']}\n"
            if args.repository == "boxferry":
                exports += f"git_worktree_spec_sha256={git_worktree_validation.SPEC_SHA256}\n"
        stage = "report-write"
        _write_resolution_report(args.output, result)
        stage = "output-export"
        if args.github_output:
            _append_resolution_outputs(args.github_output, exports)
        return 0
    except (OSError, ValueError, RunCancelled, KeyboardInterrupt) as error:
        report = {
            "schema": 1,
            "status": "failure",
            "command": "resolve",
            **bindings,
            "stage": stage,
            "cause": _bounded_text(redact(str(error), protected_values(os.environ)), 4096),
            "next_action": (
                "Inspect the source-resolution diagnostic artifact and resolver/upload logs; "
                "repair the reported metadata, tool authority or report/output destination "
                "before retrying. Failed resolution cannot qualify native images."
            ),
        }
        try:
            _write_resolution_report(args.output, report)
        except OSError:
            print(
                "Dev Container source-resolution report could not be written. Check the runner "
                "temporary directory permissions and free space, then rerun resolution. "
                "Native acceptance remains blocked.",
                file=sys.stderr,
            )
        finally:
            # Preserve the original cause even when later artifact upload is unavailable.
            _log_resolution_fallback(report)
        print(
            "Dev Container source resolution failed. Inspect the bounded diagnostic artifact; "
            "if absent, inspect resolver and artifact-upload logs before retrying.",
            file=sys.stderr,
        )
        return 1


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
    parser.add_argument("--mode", choices=("hosted",), default="hosted")
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
            return resolve_cli(args)
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
    encoded = serialize_report(result)
    result = json.loads(encoded)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(encoded + "\n")
    if args.github_output and args.command == "qualify" and result.get("status") == "success":
        with args.github_output.open("a") as stream:
            for name in (
                "candidate_sha",
                "source_manifest_sha256",
                "input_sha256",
                "build_context_sha256",
                "architecture",
                "build_context_sha256",
                "architecture",
            ):
                stream.write(f"{name}={result[name]}\n")
            if args.repository == "boxferry":
                for name in ("git_worktree_status", "git_worktree_spec_sha256"):
                    stream.write(f"{name}={result[name]}\n")
    return 0 if result.get("status") == "success" else 1


if __name__ == "__main__":
    sys.exit(main())
