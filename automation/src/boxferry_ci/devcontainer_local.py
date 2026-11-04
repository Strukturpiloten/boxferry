"""Qualify dirty local input using clean HEAD policy and selected immutable automation.

Uses the existing native qualifier exactly once. BoxFerry task worktrees resolve
six clean siblings through their verified primary Git checkout's reviewed layout.
No prior receipt substitutes for current execution or authorizes publication.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import signal
import sys
import tempfile
from pathlib import Path

from . import bootstrap
from . import devcontainer_validation as validation
from .contracts import exact_sha, relative_path
from .policy import load_policy
from .runner import RunCancelled, protected_values, redact

PREPARATION_SECONDS = 900


def _owner(source: Path, commands: validation.Commands) -> tuple[str, str]:
    origin = commands.git(source, "remote", "get-url", "origin").strip()
    names = [
        name
        for name in validation.OWNERS
        if origin
        in {
            f"https://github.com/Strukturpiloten/{name}.git",
            f"git@github.com:Strukturpiloten/{name}.git",
        }
    ]
    if len(names) != 1:
        raise validation.ValidationError(
            "local source origin must identify one Dev Container owner"
        )
    candidate = exact_sha(commands.git(source, "rev-parse", "HEAD").strip(), "candidate HEAD")
    validation._held_revision(source, candidate, commands, clean=False)
    return names[0], candidate


def _primary(source: Path, commands: validation.Commands) -> Path:
    """The reviewed sibling layout belongs to the primary checkout, not a task worktree."""
    raw = commands.git(source, "rev-parse", "--path-format=absolute", "--git-common-dir").strip()
    common = Path(raw)
    if not common.is_absolute() or common.name != ".git":
        raise validation.ValidationError(
            "BoxFerry needs its reviewed primary .git workspace layout"
        )
    common = common.resolve()
    primary = common.parent
    git_directory = commands.git(
        primary, "rev-parse", "--path-format=absolute", "--git-dir"
    ).strip()
    if not Path(git_directory).is_absolute() or Path(git_directory).resolve() != common:
        raise validation.ValidationError(
            "common Git directory does not identify a primary checkout"
        )
    revision = exact_sha(commands.git(primary, "rev-parse", "HEAD").strip(), "primary HEAD")
    validation._held_revision(primary, revision, commands, clean=False)
    validation._verify_origin(primary, "boxferry", commands)
    return primary


def workspace_sources(
    repository: str, candidate: str, source: Path, commands: validation.Commands
) -> tuple[dict[str, object], dict[str, Path]]:
    primary = _primary(source, commands) if repository == "boxferry" else source
    # Reuse the qualifier's actual top-level, clean-HEAD and origin checks for siblings.
    manifest = validation.local_sources(repository, candidate, primary, commands)
    siblings = (
        {name: (primary.parent / name).resolve() for name in validation.SIBLINGS}
        if repository == "boxferry"
        else {}
    )
    if len(set(siblings.values()) | {source}) != len(siblings) + 1:
        raise validation.ValidationError("workspace repositories must be distinct real checkouts")
    return manifest, siblings


def _snapshot(
    target: Path, revision: str, remote: str, repository: str, commands: validation.Commands
) -> None:
    validation._checkout(target, revision, commands, remote)
    commands.git(
        target, "remote", "add", "origin", f"https://github.com/Strukturpiloten/{repository}.git"
    )
    validation._held_revision(target, revision, commands)
    validation._verify_origin(target, repository, commands)


def _producer_remote(source: Path, revision: str, commands: validation.Commands) -> str:
    cache = source / "target/boxferry-automation" / f"{revision}.git"
    if cache.exists() or cache.is_symlink():
        # The stable loader's cache contains the full verified objects; its extracted
        # automation/ archive is deliberately insufficient for a Git checkout.
        bootstrap.validate_cache(cache)
        if commands.budget is None:
            raise validation.ValidationError(
                "producer cache verification needs the preparation budget"
            )
        observed = ""
        for stage, arguments in (
            (
                "producer-cache-fsck",
                ("fsck", "--full", "--strict", "--no-reflogs", "--no-dangling"),
            ),
            ("producer-cache-revision", ("rev-parse", f"{revision}^{{commit}}")),
        ):
            observed = commands.call(
                (
                    "git",
                    "--no-replace-objects",
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-C",
                    str(cache),
                    *arguments,
                ),
                stage,
                120,
            ).strip()
        if observed != revision:
            raise validation.ValidationError("shared object cache differs from the selected SHA")
        return str(cache)
    return bootstrap.REMOTE


def _install_cli(automation: Path, private: Path, commands: validation.Commands) -> None:
    node, npm = shutil.which("node"), shutil.which("npm")
    if not node or not npm or not Path(node).is_file() or not Path(npm).is_file():
        raise validation.ValidationError(
            "canonical host Node and npm are required before qualification"
        )
    if commands.call((node, "--version"), "node-version").strip() != (
        "v" + validation.canonical_versions(automation)["node"]
    ):
        raise validation.ValidationError("host Node differs from the selected canonical authority")
    # npm's env-node launcher must use the exact Node whose version was verified.
    commands.env["PATH"] = str(Path(node).parent) + os.pathsep + commands.env["PATH"]
    commands.env.update(
        NPM_CONFIG_CACHE=str(private / "npm-cache"),
        NPM_CONFIG_USERCONFIG=os.devnull,
        NPM_CONFIG_GLOBALCONFIG=os.devnull,
    )
    commands.call(
        (npm, "ci", "--ignore-scripts", "--no-audit", "--no-fund"),
        "install-official-cli",
        PREPARATION_SECONDS,
        cwd=automation / validation.CLI_PROJECT,
    )


def execute(source: Path, config: Path, output: Path) -> dict[str, object]:
    """One bounded preparation and one existing native qualifier invocation."""
    report: dict[str, object] = {
        "schema": 1,
        "status": "failure",
        "mode": "local",
        "engine": "podman",
        "cleanup": "not-started",
        "preparation_cleanup": "pending",
        "next_action": "Correct local source, tool or cleanup failures; rerun this candidate.",
    }
    private = Path(tempfile.mkdtemp(prefix="devcontainer-local-", dir=output.parent))
    private.chmod(0o700)
    commands: validation.Commands | None = None
    try:
        commands = validation.Commands(private)
        commands.budget = validation.Budget.start(output.parent, PREPARATION_SECONDS)
        architecture = validation.ARCHITECTURES.get(platform.machine())
        if platform.system() != "Linux" or architecture is None:
            raise validation.ValidationError(
                "local qualification needs a supported native Linux host"
            )
        repository, candidate = _owner(source, commands)
        commands.git(source, "check-ignore", "--quiet", "--", str(output.relative_to(source)))
        held = validation.fingerprint(source, commands)
        selected = bootstrap.load_config(source)
        automation_sha = exact_sha(selected["revision"], "selected automation revision")
        relative = config.relative_to(source) if config.is_absolute() else config
        config_path = relative_path(relative.as_posix(), "local policy path")
        if config_path != selected["policy"]:
            raise validation.ValidationError("local policy path must match the selected manifest")
        manifest, siblings = workspace_sources(repository, candidate, source, commands)
        report.update(
            repository=f"Strukturpiloten/{repository}",
            candidate_sha=candidate,
            policy_sha=candidate,
            automation_sha=automation_sha,
            input_sha256=held,
            policy_config=config_path,
        )
        policy, automation = private / "policy", private / "automation"
        _snapshot(policy, candidate, str(source), repository, commands)
        policy_file = policy / config_path
        if (
            not policy_file.is_file()
            or policy_file.stat().st_size > 64 * 1024
            or any(
                path.is_symlink() for path in (policy_file, *policy_file.parents) if path != policy
            )
        ):
            raise validation.ValidationError("trusted policy must be bounded regular HEAD data")
        trusted = load_policy(policy_file)
        if trusted.repository != f"Strukturpiloten/{repository}":
            raise validation.ValidationError(
                "trusted HEAD policy belongs to a different repository"
            )
        report["policy_config_sha256"] = hashlib.sha256(
            (policy / config_path).read_bytes()
        ).hexdigest()
        _snapshot(
            automation,
            automation_sha,
            _producer_remote(source, automation_sha, commands),
            "boxferry",
            commands,
        )
        _install_cli(automation, private, commands)
        validation._held_revision(policy, candidate, commands)
        validation._held_revision(automation, automation_sha, commands)
        validation._held_revision(source, candidate, commands, clean=False)
        if validation.fingerprint(source, commands) != held:
            raise validation.ValidationError(
                "candidate input closure changed during local preparation"
            )
        commands.budget.check()
        outcome = validation.qualify(
            repository=repository,
            candidate=candidate,
            policy_sha=candidate,
            automation_sha=automation_sha,
            source=source,
            policy=policy,
            automation=automation,
            manifest=manifest,
            engine="podman",
            mode="local",
            work=output.parent,
            expected_architecture=architecture,
            local_sources=siblings if repository == "boxferry" else None,
        )
        report.update(outcome)
        validation._held_revision(source, candidate, commands, clean=False)
        if validation.fingerprint(source, commands) != held:
            raise validation.ValidationError("candidate input closure changed during qualification")
        current_manifest, current_siblings = workspace_sources(
            repository, candidate, source, commands
        )
        if current_manifest != manifest or current_siblings != siblings:
            raise validation.ValidationError(
                "local workspace identities changed during qualification"
            )
        if report.get("status") == "success" and any(
            report.get(key) != value
            for key, value in {
                "candidate_sha": candidate,
                "policy_sha": candidate,
                "automation_sha": automation_sha,
                "input_sha256": held,
                "source_manifest_sha256": validation.manifest_digest(manifest),
                "engine": "podman",
                "architecture": architecture,
                "cleanup": "success",
            }.items()
        ):
            raise validation.ValidationError(
                "qualification receipt differs from held local identities"
            )
    except (OSError, ValueError, RunCancelled, KeyboardInterrupt) as error:
        report.update(
            status="failure", cause=redact(str(error), protected_values(os.environ))[:4096]
        )
    finally:
        report["preparation_outcomes"] = commands.outcomes if commands is not None else []
        try:
            shutil.rmtree(private)
            report["preparation_cleanup"] = "success"
        except OSError as error:
            report.update(
                status="failure",
                preparation_cleanup="failure",
                preparation_cleanup_cause=redact(str(error), protected_values(os.environ))[:4096],
                retained_preparation_directory=str(private),
            )
    return report


def _cancel(_number: int, _frame: object) -> None:
    raise RunCancelled("local qualification cancelled")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--repository-root", type=Path, required=True, help="Actual local Git checkout or worktree."
    )
    parser.add_argument(
        "--config",
        type=Path,
        required=True,
        help="Manifest-selected policy path; read independently from clean HEAD.",
    )
    parser.add_argument(
        "--output",
        type=Path,
        required=True,
        help="Fresh bounded receipt inside the checkout's ignored target tree.",
    )
    options = parser.parse_args(argv)
    source = options.repository_root.resolve()
    output = options.output if options.output.is_absolute() else source / options.output
    previous = signal.signal(signal.SIGTERM, _cancel)
    writable = False
    try:
        if not output.absolute().is_relative_to(source / "target") or ".." in output.parts:
            raise validation.ValidationError(
                "local report must be in the repository-owned target tree"
            )
        bootstrap.safe_cache_directory(output.parent)
        if output.is_symlink() or (output.exists() and not output.is_file()):
            raise validation.ValidationError("local report must be a regular owned file")
        writable = True
        report = execute(source, options.config, output)
        encoded = json.dumps(report, sort_keys=True)
        if len(encoded.encode()) > validation.MAX_REPORT:
            report = {
                "schema": 1,
                "status": "failure",
                "cause": "local report exceeds its byte bound",
            }
            encoded = json.dumps(report)
        output.write_text(encoded + "\n")
        print(encoded)
        return 0 if report.get("status") == "success" else 1
    except (OSError, ValueError, RunCancelled, KeyboardInterrupt) as error:
        failure = {
            "schema": 1,
            "status": "failure",
            "mode": "local",
            "engine": "podman",
            "cause": redact(str(error), protected_values(os.environ))[:4096],
            "next_action": "Correct the named local input or infrastructure cause and rerun.",
        }
        encoded = json.dumps(failure, sort_keys=True)
        if writable:
            try:
                output.write_text(encoded + "\n")
            except OSError:
                pass  # Exit remains failure; never report a stale receipt as new success.
        print(encoded, file=sys.stderr)
        return 1
    finally:
        signal.signal(signal.SIGTERM, previous)
