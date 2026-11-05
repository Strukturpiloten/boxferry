"""One explicit API baseline policy for development and published-release checks."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import signal
import sys
import tarfile
import tempfile
import time
from collections.abc import Generator, Mapping
from contextlib import contextmanager
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

from boxferry_ci import cargo_tools
from boxferry_ci.bootstrap import automation_identity
from boxferry_ci.contracts import ContractError
from boxferry_ci.runner import RunCancelled, run

SHA = re.compile(r"[0-9a-f]{40}\Z")
PACKAGE = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*\Z")
VERSION = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\Z")
SEMVER = re.compile(
    VERSION.pattern.removesuffix(r"\Z")
    + r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
    + r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?\Z"
)
REPOSITORIES = frozenset(
    f"Strukturpiloten/{name}"
    for name in (
        "boxferry",
        "compose-lens",
        "podman-lens",
        "quadlet-lens",
        "docker-lens",
        "kubernetes-lens",
    )
)
MAX_SOURCE_BYTES = 512 * 1024 * 1024
FETCH_HELP = "Run git fetch origin --prune; use git fetch --unshallow for shallow history."


@dataclass(frozen=True)
class Context:
    """Purpose and independently held event identities; API baseline is separate from policy."""

    purpose: str
    event: str
    candidate: str
    policy: str
    automation: str
    repository: str

    def validate(self) -> None:
        if self.purpose not in {"development", "published"} or self.repository not in REPOSITORIES:
            raise ContractError("API purpose or caller repository is outside the reviewed contract")
        if any(
            not SHA.fullmatch(value) for value in (self.candidate, self.policy, self.automation)
        ):
            raise ContractError("API candidate, policy and automation require full commit SHAs")
        if self.event not in {"pull_request", "merge_group", "push", "workflow_dispatch", "local"}:
            raise ContractError("unsupported API baseline event")


class APICommandError(ContractError):
    """Preserve bounded, redacted command findings rather than replacing them with Git advice."""

    def __init__(self, message: str, diagnostic: dict[str, object]) -> None:
        super().__init__(message)
        self.diagnostic = diagnostic


def command(
    root: Path,
    argv: tuple[str, ...],
    *,
    timeout: int = 30,
    environment: Mapping[str, str] | None = None,
) -> bytes:
    execution = os.environ if environment is None else environment
    forbidden = {
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_SHALLOW_FILE",
        "GIT_GRAFT_FILE",
        "GIT_REPLACE_REF_BASE",
        "GIT_INDEX_FILE",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
    }
    if any(execution.get(name) for name in forbidden):
        raise ContractError(
            "API history refuses Git object, index or configuration environment overrides"
        )
    result = run(
        argv,
        cwd=root,
        environment=dict(
            execution,
            GIT_NO_REPLACE_OBJECTS="1",
            GIT_CONFIG_NOSYSTEM="1",
            GIT_CONFIG_GLOBAL=os.devnull,
        ),
        stage="API baseline policy",
        timeout_seconds=timeout,
    )
    if result.status != "success":
        next_action = (
            FETCH_HELP if argv[0] == "git" else "Inspect the retained API command findings."
        )
        raise APICommandError(
            f"API command failed ({result.status}): {result.stdout[-1000:]} "
            f"{result.stderr[-1000:]}; {next_action}",
            result.to_dict(),
        )
    return result.stdout_bytes


def git(root: Path, *arguments: str) -> str:
    return (
        command(root, ("git", "--no-replace-objects", "-c", "core.fsmonitor=false", *arguments))
        .decode("utf-8")
        .strip()
    )


def git_history(root: Path, candidate: str) -> None:
    if git(root, "rev-parse", "--is-shallow-repository") != "false":
        raise ContractError(f"API baseline needs complete, nonshallow history. {FETCH_HELP}")
    grafts = Path(git(root, "rev-parse", "--git-path", "info/grafts"))
    if not grafts.is_absolute():
        grafts = root / grafts
    if grafts.exists() or grafts.is_symlink():
        raise ContractError("API history must not use Git grafts")
    configuration = git(root, "config", "--list")
    if re.search(r"(?mi)^(extensions\.partialclone|remote\..*\.promisor)=", configuration):
        raise ContractError(
            "API history must not be a partial/promisor clone; fetch a complete clone"
        )
    if git(root, "rev-parse", "HEAD") != candidate:
        raise ContractError("held API checkout differs from exact tested candidate")
    git(root, "rev-list", "--objects", "--missing=error", candidate)


def tracked_paths(root: Path) -> list[str]:
    raw = command(
        root,
        (
            "git",
            "--no-replace-objects",
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ),
    )
    if raw and not raw.endswith(b"\0"):
        raise ContractError("API source inventory is incomplete")
    paths = sorted(set(raw.decode("utf-8").split("\0")[:-1]))
    if len(paths) > 100_000:
        raise ContractError("API source inventory exceeded its path bound")
    return paths


def fingerprint(root: Path, paths: list[str] | None = None) -> str:
    """Hash tracked and nonignored untracked source bytes; never expose protected values."""
    digest = hashlib.sha256()
    total = 0
    deadline = time.monotonic() + 60
    for relative in tracked_paths(root) if paths is None else paths:
        if time.monotonic() > deadline:
            raise ContractError("API source fingerprint exceeded its read deadline")
        path = root / relative
        if Path(relative).is_absolute() or ".." in Path(relative).parts:
            raise ContractError("API source inventory escaped its checkout")
        if path.is_symlink() or any(
            parent.is_symlink() for parent in path.parents if parent != root
        ):
            raise ContractError("API source fingerprint refuses source symlinks")
        if not path.exists():
            # Phantom staged paths absent from both HEAD and source do not change source.
            # Real deletions still differ because the baseline contains the missing path.
            continue
        digest.update(relative.encode() + b"\0")
        if not path.is_file():
            raise ContractError("API source inventory contains a nonregular file")
        if total + path.stat().st_size > MAX_SOURCE_BYTES:
            raise ContractError("API source fingerprint exceeded its byte bound")
        digest.update(b"executable\0" if path.stat().st_mode & 0o111 else b"regular\0")
        with path.open("rb") as stream:
            while chunk := stream.read(1024 * 1024):
                total += len(chunk)
                if total > MAX_SOURCE_BYTES or time.monotonic() > deadline:
                    raise ContractError("API source fingerprint exceeded its actual read bound")
                digest.update(chunk)
        digest.update(b"\0")
    return digest.hexdigest()


def dirty(root: Path) -> bool:
    return bool(git(root, "status", "--porcelain", "--untracked-files=normal"))


def first_parent_history(root: Path, revision: str) -> list[str]:
    return git(root, "rev-list", "--first-parent", revision).splitlines()


def default_authority(root: Path, context: Context) -> tuple[str, str]:
    """Read canonical GitHub default HEAD; stale local tracking refs never become authority."""
    remote = git(root, "remote", "get-url", "origin")
    if remote not in {
        f"https://github.com/{context.repository}",
        f"https://github.com/{context.repository}.git",
        f"git@github.com:{context.repository}.git",
        f"ssh://git@github.com/{context.repository}.git",
    }:
        raise ContractError(
            "API default-branch authority requires this allowlisted repository's canonical origin"
        )
    # Run outside the candidate repository, with global/system config disabled:
    # local insteadOf, uploadpack, sshCommand and TLS settings cannot forge authority.
    with tempfile.TemporaryDirectory(prefix="boxferry-api-authority-") as directory:
        rows = git(
            Path(directory),
            "-c",
            "credential.helper=",
            "ls-remote",
            "--symref",
            f"https://github.com/{context.repository}.git",
            "HEAD",
        ).splitlines()
    symbolic = [
        row.split("\t")[0][5:] for row in rows if row.startswith("ref: ") and row.endswith("\tHEAD")
    ]
    tips = [
        row.split("\t")[0] for row in rows if row.endswith("\tHEAD") and not row.startswith("ref: ")
    ]
    if (
        len(symbolic) != 1
        or len(tips) != 1
        or not symbolic[0].startswith("refs/heads/")
        or not SHA.fullmatch(tips[0])
    ):
        raise ContractError(
            f"canonical default branch has missing or ambiguous authority. {FETCH_HELP}"
        )
    branch = symbolic[0].removeprefix("refs/heads/")
    tracking = f"refs/remotes/origin/{branch}"
    if git(root, "rev-parse", "--verify", tracking) != tips[0]:
        raise ContractError(
            f"fetched default branch differs from canonical advertised HEAD. {FETCH_HELP}"
        )
    git(root, "rev-list", "--objects", "--missing=error", tips[0])
    return branch, tips[0]


def select_git(
    root: Path, context: Context, event: Mapping[str, Any]
) -> tuple[str, dict[str, str]]:
    context.validate()
    git_history(root, context.candidate)
    candidate = context.candidate
    authority: dict[str, str] = {}
    if context.event in {"pull_request", "merge_group"}:
        key = "pull_request" if context.event == "pull_request" else "merge_group"
        group = event.get(key, {})
        base = group.get("base", {}).get("sha") if key == "pull_request" else group.get("base_sha")
        if base != context.policy or not isinstance(base, str) or not SHA.fullmatch(base):
            raise ContractError("API event base does not equal independently trusted policy SHA")
        if base == candidate:
            raise ContractError("PR/queue API baseline must differ from tested merge")
        git(root, "merge-base", "--is-ancestor", base, candidate)
        authority = {"event_base": base}
    elif context.event == "push":
        repository = event.get("repository", {})
        branch = repository.get("default_branch")
        base = event.get("before")
        if (
            not branch
            or event.get("ref") != f"refs/heads/{branch}"
            or not isinstance(base, str)
            or not SHA.fullmatch(base)
            or base == "0" * 40
        ):
            raise ContractError(
                "development main API check requires an existing default-branch push before SHA"
            )
        if base == candidate or base not in first_parent_history(root, candidate)[1:]:
            raise ContractError(
                "push API baseline must precede tested candidate "
                "on trusted first-parent main history"
            )
        authority = {"default_branch": branch, "push_before": base}
    else:
        branch, tip = default_authority(root, context)
        authority = {"default_branch": branch, "advertised_default_sha": tip}
        main_history = first_parent_history(root, tip)
        if candidate in main_history:
            if context.event == "local" and dirty(root):
                base = candidate
            else:
                history = first_parent_history(root, candidate)
                if len(history) < 2:
                    raise ContractError(
                        "clean trusted-main API check needs a previous first-parent commit"
                    )
                base = history[1]
        else:
            bases = git(root, "merge-base", "--all", candidate, tip).splitlines()
            if len(bases) != 1 or not SHA.fullmatch(bases[0]) or bases[0] == candidate:
                raise ContractError(
                    f"proposed branch needs one non-self trusted default-branch fork base. "
                    f"{FETCH_HELP}"
                )
            base = bases[0]
        if default_authority(root, context) != (branch, tip):
            raise ContractError(
                "canonical default branch moved during API baseline selection; retry after fetching"
            )
    git(root, "cat-file", "-e", f"{base}^{{commit}}")
    git(root, "rev-list", "--objects", "--missing=error", base)
    if context.event != "local" and dirty(root):
        raise ContractError("hosted API checks require a clean exact candidate source")
    return base, authority


@contextmanager
def snapshot(root: Path, revision: str) -> Generator[Path]:
    """Materialize only a previously validated immutable Git baseline, never refs/worktrees."""
    with tempfile.TemporaryDirectory(prefix="boxferry-api-baseline-") as directory:
        temporary = Path(directory)
        archive = temporary / "baseline.tar"
        git(root, "archive", "--format=tar", f"--output={archive}", revision)
        if archive.stat().st_size > MAX_SOURCE_BYTES:
            raise ContractError("trusted API baseline archive exceeded its byte bound")
        destination = temporary / "source"
        destination.mkdir()
        with tarfile.open(archive) as stream:
            members = stream.getmembers()
            if len(members) > 100_000 or sum(member.size for member in members) > MAX_SOURCE_BYTES:
                raise ContractError("trusted API baseline archive exceeded extraction bounds")
            if any(
                not (member.isfile() or member.isdir())
                or Path(member.name).is_absolute()
                or ".." in Path(member.name).parts
                for member in members
            ):
                raise ContractError("trusted API baseline archive contains links or unsafe paths")
            stream.extractall(destination, members=members, filter="data")
        yield destination


def metadata(root: Path, channel: str) -> dict[str, dict[str, Any]]:
    """Collect workspace libraries, independently of their separate publication policy."""
    value = json.loads(
        command(
            root,
            (
                "cargo",
                f"+{channel}",
                "metadata",
                "--manifest-path",
                str(root / "Cargo.toml"),
                "--locked",
                "--no-deps",
                "--format-version",
                "1",
            ),
        )
    )
    if (
        not isinstance(value, dict)
        or not isinstance(value.get("packages"), list)
        or not isinstance(value.get("workspace_members"), list)
    ):
        raise ContractError("API workspace metadata is malformed")
    packages = {}
    for package in value["packages"]:
        if not isinstance(package, dict):
            raise ContractError("API workspace package metadata is malformed")
        if package.get("id") not in value["workspace_members"]:
            continue
        targets = package.get("targets")
        if not isinstance(targets, list) or any(
            not isinstance(target, dict)
            or not isinstance(target.get("kind"), list)
            or any(not isinstance(kind, str) for kind in target["kind"])
            for target in targets
        ):
            raise ContractError("API workspace target metadata is malformed")
        if not any(
            set(target.get("kind", []))
            & {"lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"}
            for target in targets
        ):
            continue
        name, version = package.get("name"), package.get("version")
        if (
            not isinstance(name, str)
            or not PACKAGE.fullmatch(name)
            or not isinstance(version, str)
            or not SEMVER.fullmatch(version)
            or name in packages
        ):
            raise ContractError("API package name/version is malformed or duplicated")
        packages[name] = package
    return packages


def registry_version(package: str) -> str | None:
    """Only canonical 404 establishes absence; any historical publication prevents initial API."""
    if not PACKAGE.fullmatch(package):
        raise ContractError("invalid canonical API package")
    request = Request(
        f"https://crates.io/api/v1/crates/{package}",
        headers={"User-Agent": "Strukturpiloten shared Rust CI"},
    )
    try:
        with urlopen(request, timeout=60) as response:
            if response.status != 200 or response.url != request.full_url:
                raise ContractError(
                    "canonical API registry changed identity or returned unexpected status"
                )
            raw = response.read(2_000_001)
    except HTTPError as error:
        if error.code == 404 and error.url == request.full_url:
            return None
        raise ContractError(f"canonical API lookup failed with HTTP {error.code}") from error
    except (URLError, TimeoutError) as error:
        raise ContractError("canonical API lookup unavailable; absence is unproven") from error
    if len(raw) > 2_000_000:
        raise ContractError("canonical API metadata exceeded its byte bound")
    value = json.loads(raw)
    if (
        not isinstance(value, dict)
        or not isinstance(value.get("crate"), dict)
        or value["crate"].get("name") != package
        or not isinstance(value.get("versions"), list)
        or not value["versions"]
    ):
        raise ContractError("canonical API metadata lacks publication identity/history")
    versions = []
    for row in value["versions"]:
        if (
            not isinstance(row, dict)
            or row.get("crate") != package
            or type(row.get("yanked")) is not bool
            or not isinstance(row.get("num"), str)
            or not SEMVER.fullmatch(row["num"])
        ):
            raise ContractError("canonical API version history is malformed")
        if "-" not in row["num"].split("+", 1)[0] and not row["yanked"]:
            versions.append(row["num"])
    if not versions:
        raise ContractError(
            "crate has prior publication but no usable non-yanked stable API baseline; "
            "initial API is forbidden"
        )
    return max(
        versions,
        key=lambda version: tuple(int(part) for part in version.split("+", 1)[0].split(".")),
    )


def release_allowance(purpose: str, release_type: str) -> str:
    if release_type not in {"", "major", "minor", "patch"}:
        raise ContractError("API release type must be empty, major, minor or patch")
    if purpose == "published":
        if release_type:
            raise ContractError(
                "published API must infer compatibility from the chosen manifest version, "
                "without intent overrides"
            )
        return ""
    return release_type or "minor"


def prepare(
    root: Path,
    context: Context,
    event: Mapping[str, Any],
    package: str | None,
    release_type: str,
    allow_initial: bool,
) -> dict[str, Any]:
    context.validate()
    allowance = release_allowance(context.purpose, release_type)
    tool = cargo_tools.select(root, "cargo-semver-checks")
    candidate_packages = metadata(root, tool.rust_channel)
    selected = (
        [package]
        if package
        else sorted(name for name, data in candidate_packages.items() if data.get("publish") != [])
    )
    if not selected or any(name not in candidate_packages for name in selected):
        raise ContractError(
            "API package must be a tested workspace library; "
            "select private libraries explicitly with --package"
        )
    if git(root, "rev-parse", "HEAD") != context.candidate:
        raise ContractError("API checkout differs from tested candidate")
    before = fingerprint(root)
    base, authority = None, {}
    baseline_packages: dict[str, dict[str, Any]] = {}
    if context.purpose == "development":
        base, authority = select_git(root, context, event)
        with snapshot(root, base) as baseline:
            baseline_paths = sorted(
                git(root, "ls-tree", "-r", "--name-only", "-z", base).split("\0")[:-1]
            )
            if any(not (baseline / path).is_file() for path in baseline_paths):
                raise ContractError(
                    "trusted API archive omitted source paths; complete baseline is required"
                )
            if base == context.candidate and before == fingerprint(baseline, baseline_paths):
                raise ContractError(
                    "dirty index without actual source changes "
                    "cannot authorize a self API comparison"
                )
            baseline_packages = (
                metadata(baseline, tool.rust_channel) if (baseline / "Cargo.toml").exists() else {}
            )
    elif dirty(root):
        raise ContractError("published API evidence requires a clean exact committed candidate")
    checks = []
    for name in selected:
        if base is not None and name in baseline_packages:
            kind, baseline_version = "git", ""
        else:
            baseline_version = registry_version(name)
            kind = "published" if baseline_version is not None else "initial-contract"
            if kind == "initial-contract" and not allow_initial:
                raise ContractError(
                    "API baseline is genuinely absent; "
                    "explicit positive initial-api-contract (#485) is required"
                )
        checks.append(
            {
                "package": name,
                "candidate_version": candidate_packages[name]["version"],
                "kind": kind,
                "baseline_rev": base if kind == "git" else "",
                "baseline_version": baseline_version or "",
                "release_type": allowance,
            }
        )
    if before != fingerprint(root):
        raise ContractError("candidate source changed while selecting API baseline")
    return {
        "schema": 1,
        "context": asdict(context),
        "source_fingerprint": before,
        "authority": authority,
        "tool": asdict(tool),
        "checks": checks,
        "status": "selected",
    }


def argv_for(root: Path, check: Mapping[str, str]) -> tuple[str, ...]:
    if check["kind"] not in {"git", "published"}:
        raise ContractError(
            "initial API requires the positive contract task, not compatibility omission"
        )
    argv = [
        "cargo",
        "semver-checks",
        "check-release",
        "--manifest-path",
        str(root / "Cargo.toml"),
        "--package",
        check["package"],
        "--all-features",
    ]
    if check["kind"] == "git":
        argv.extend(("--baseline-rev", check["baseline_rev"]))
    else:
        argv.extend(("--baseline-version", check["baseline_version"]))
    if check["release_type"]:
        argv.extend(("--release-type", check["release_type"]))
    return tuple(argv)


def finish(root: Path, evidence: dict[str, Any], verification_root: Path) -> dict[str, str]:
    context = Context(**evidence["context"])
    context.validate()
    identity = automation_identity(
        root,
        context.automation,
        allow_dirty_self=context.event == "local" and context.purpose == "development",
    )
    if "automation_source" in evidence and identity != evidence["automation_source"]:
        raise ContractError("automation implementation changed during API compatibility checking")
    if (
        fingerprint(root) != evidence["source_fingerprint"]
        or git(root, "rev-parse", "HEAD") != context.candidate
    ):
        raise ContractError("API candidate source changed during compatibility checking")
    if context.purpose == "published" and dirty(root):
        raise ContractError(
            "published API evidence cannot identify dirty local source as a clean commit"
        )
    tool = cargo_tools.select(root, "cargo-semver-checks")
    if asdict(tool) != evidence["tool"]:
        raise ContractError("API candidate tool/compiler declaration changed")
    executable = cargo_tools.verify(tool, verification_root, os.environ)
    evidence["tool"]["executable"] = str(executable)
    evidence["status"] = "success"
    return {
        "candidate_sha": context.candidate,
        "published_api_candidate_sha": context.candidate if context.purpose == "published" else "",
    }


def write_outputs(path: Path | None, values: Mapping[str, str]) -> None:
    if any("\n" in value or "\r" in value for value in values.values()):
        raise ContractError("API evidence output contains control characters")
    if path is not None:
        with path.open("a", encoding="utf-8") as stream:
            stream.write("".join(f"{key}={value}\n" for key, value in values.items()))


def selection_digest(evidence: Mapping[str, Any]) -> str:
    return hashlib.sha256(
        json.dumps(evidence, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()


def _cancel(_signum: int, _frame: object) -> None:
    raise RunCancelled("API workflow cancellation")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("prepare", "finish", "run"))
    parser.add_argument("--repository-root", type=Path, required=True)
    parser.add_argument("--api-purpose", choices=("development", "published"), default="published")
    parser.add_argument("--event", default=os.environ.get("GITHUB_EVENT_NAME", "local"))
    parser.add_argument("--candidate-sha")
    parser.add_argument("--policy-sha")
    parser.add_argument("--automation-sha")
    parser.add_argument(
        "--repository", default=os.environ.get("GITHUB_REPOSITORY", "Strukturpiloten/boxferry")
    )
    parser.add_argument("--package")
    parser.add_argument("--release-type", default="")
    parser.add_argument("--allow-initial", action="store_true")
    parser.add_argument("--config", type=Path)
    parser.add_argument("--report-json", type=Path, required=True)
    parser.add_argument("--github-output", type=Path)
    parser.add_argument("--selection-sha256")
    options = parser.parse_args(argv)
    previous_handler = signal.signal(signal.SIGTERM, _cancel)
    evidence: dict[str, Any] = {}
    try:
        root = options.repository_root.resolve(strict=True)
        if options.operation == "finish":
            evidence = json.loads(options.report_json.read_text())
            if (
                not options.selection_sha256
                or selection_digest(evidence) != options.selection_sha256
            ):
                raise ContractError(
                    "API selection differs from immutable pre-check workflow-step digest"
                )
            outputs = finish(root, evidence, Path.cwd())
            write_outputs(options.github_output, outputs)
        else:
            candidate = options.candidate_sha or git(root, "rev-parse", "HEAD")
            if options.event != "local" and candidate != os.environ.get("GITHUB_SHA"):
                raise ContractError("hosted API candidate must equal exact GITHUB_SHA")
            identity = automation_identity(
                root,
                options.automation_sha,
                allow_dirty_self=options.event == "local" and options.api_purpose == "development",
            )
            context = Context(
                options.api_purpose,
                options.event,
                candidate,
                options.policy_sha or candidate,
                identity["revision"],
                options.repository,
            )
            event: dict[str, Any] = {}
            if options.event != "local":
                event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())
                if event.get("repository", {}).get("full_name") != context.repository:
                    raise ContractError("API event repository differs from allowlisted caller")
            evidence = prepare(
                root, context, event, options.package, options.release_type, options.allow_initial
            )
            evidence["automation_source"] = identity
            if options.operation == "prepare":
                if len(evidence["checks"]) != 1:
                    raise ContractError("hosted API preparation requires exactly one package")
                check = evidence["checks"][0]
                write_outputs(
                    options.github_output,
                    {
                        "selection_sha256": selection_digest(evidence),
                        **{
                            key: check[key]
                            for key in ("kind", "baseline_rev", "baseline_version", "release_type")
                        },
                    },
                )
            else:
                tool = cargo_tools.select(root, "cargo-semver-checks")
                execution = dict(os.environ, RUSTUP_TOOLCHAIN=tool.rust_channel)
                cargo_tools.verify(tool, root, execution)
                for check in evidence["checks"]:
                    if check["kind"] == "initial-contract":
                        if options.config is None:
                            raise ContractError(
                                "initial API requires its trusted positive task policy"
                            )
                        command(
                            root,
                            (
                                sys.executable,
                                "-m",
                                "boxferry_ci",
                                "run",
                                "--repository-root",
                                str(root),
                                "--config",
                                str(options.config.resolve(strict=True)),
                                "--task",
                                "initial-api-contract",
                                "--tested-sha",
                                context.candidate,
                                "--automation-sha",
                                context.automation,
                                "--report-json",
                                str(root / "target/automation-initial-api-contract.json"),
                            ),
                            timeout=1200,
                            environment=execution,
                        )
                    else:
                        command(root, argv_for(root, check), timeout=1200, environment=execution)
                finish(root, evidence, root)
        print(json.dumps(evidence, sort_keys=True))
        return 0
    except (KeyboardInterrupt, RunCancelled):
        evidence["status"] = "cancelled"
        return 130
    except (OSError, ValueError, KeyError, TypeError, tarfile.TarError) as error:
        evidence["status"] = "failure"
        evidence["error"] = str(error)
        if isinstance(error, APICommandError):
            evidence["command_failure"] = error.diagnostic
            if error.diagnostic["status"] == "cancelled":
                evidence["status"] = "cancelled"
                return 130
        print(f"API baseline validation failed: {error}", file=sys.stderr)
        return 1
    finally:
        try:
            options.report_json.parent.mkdir(parents=True, exist_ok=True)
            options.report_json.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n")
        finally:
            signal.signal(signal.SIGTERM, previous_handler)


if __name__ == "__main__":
    raise SystemExit(main())
