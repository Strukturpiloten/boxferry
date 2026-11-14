#!/usr/bin/env python3
"""Observe admitted source-report fixtures; these are never native acceptance receipts."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import stat
import subprocess
import sys
from pathlib import Path, PurePosixPath
from typing import Any

CASES = {
    "success": ("success", 0, 12),
    "unavailable": ("unavailable", 1, 1),
    "malformed": ("malformed", 1, 1),
    "invalid-revision": ("invalid-revision", 1, 2),
    "missing-report": ("missing-report", 0, 12),
    "unavailable-report-loss": ("unavailable", 1, 1),
}
# Independent expected causes for these finite authored HTTP controls, not resolver policy.
# They fit the log bound and contain no command punctuation, so no truncation/neutralization
# may silently substitute another cause in either retained channel.
EXPECTED_CAUSES = {
    "unavailable": "controlled metadata unavailable [REDACTED]",
    "malformed": "public repository has an unsupported default branch",
    "invalid-revision": "resolved sibling SHA must be an exact lowercase 40-character Git SHA",
    "unavailable-report-loss": "controlled metadata unavailable [REDACTED]",
}
KIND = "diagnostic-fixture-not-native-acceptance"
MODULE = "automation/src/boxferry_ci/devcontainer_validation.py"
FIXTURE = "automation/tests/source_report_fixture.py"
WORKFLOW = ".github/workflows/shared-devcontainer-validation.yml"
MAX_FILE = 64 * 1024


def require(condition: object, message: str) -> None:
    if not condition:
        raise ValueError(message)


def regular(path: Path, limit: int = MAX_FILE) -> bytes:
    """Refuse links in every component and nonregular or oversized evidence."""
    for part in (path, *path.parents):
        require(not part.is_symlink(), "evidence path contains a symlink")
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as stream:
        info = os.fstat(stream.fileno())
        require(stat.S_ISREG(info.st_mode) and info.st_size <= limit, "invalid evidence file")
        data = stream.read(limit + 1)
    require(len(data) <= limit, "evidence exceeds byte limit")
    return data


def unique(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        require(key not in value, "duplicate JSON field")
        value[key] = item
    return value


def document(path: Path, limit: int = MAX_FILE) -> dict[str, Any]:
    value = json.loads(regular(path, limit), object_pairs_hook=unique)
    require(isinstance(value, dict), "expected JSON object")
    return value


def sha(value: str) -> str:
    require(re.fullmatch(r"[a-f0-9]{40}", value), "expected full lowercase revision")
    return value


def git(root: Path, *args: str) -> bytes:
    result = subprocess.run(
        ["git", "--no-replace-objects", "-c", "core.hooksPath=/dev/null", "-C", str(root), *args],
        capture_output=True,
        check=True,
        timeout=5,
    )
    require(len(result.stdout) <= 512 * 1024, "Git evidence exceeds byte limit")
    return result.stdout


def manifest(candidate: Path, candidate_sha: str) -> dict[str, Any]:
    """Only the exact candidate's existing interface-1 manifest selects automation."""
    sha(candidate_sha)
    require(
        git(candidate, "rev-parse", "HEAD").decode().strip() == candidate_sha, "wrong candidate"
    )
    data = regular(candidate / ".github/automation.json", 8192)
    require(
        data == git(candidate, "show", f"{candidate_sha}:.github/automation.json"), "dirty manifest"
    )
    value = json.loads(data, object_pairs_hook=unique)
    require(
        isinstance(value, dict)
        and set(value) == {"schema", "repository", "revision", "interface", "policy"},
        "invalid manifest keys",
    )
    require(type(value["schema"]) is int and value["schema"] == 1, "invalid manifest schema")
    require(type(value["interface"]) is int and value["interface"] == 1, "invalid interface")
    require(value["repository"] == "Strukturpiloten/boxferry", "invalid repository")
    require(isinstance(value["revision"], str), "invalid manifest revision")
    sha(value["revision"])
    policy = value["policy"]
    require(
        isinstance(policy, str)
        and re.fullmatch(r"[a-zA-Z0-9_./-]+", policy)
        and not PurePosixPath(policy).is_absolute()
        and all(part not in {"", ".", ".."} for part in policy.split("/")),
        "invalid policy path",
    )
    regular(candidate / policy)
    require(
        regular(candidate / policy) == git(candidate, "show", f"{candidate_sha}:{policy}"),
        "dirty policy",
    )
    return value


def location(temp: Path, run: str, attempt: str, case: str) -> Path:
    require(case in CASES, "unknown diagnostic case")
    require(re.fullmatch(r"[1-9][0-9]{0,19}", run), "invalid run identity")
    require(re.fullmatch(r"[1-9][0-9]{0,5}", attempt), "invalid attempt identity")
    require(temp.is_absolute() and temp.is_dir() and not temp.is_symlink(), "invalid runner temp")
    for part in temp.parents:
        require(not part.is_symlink(), "runner temp ancestry contains a symlink")
    return temp / f"boxferry-source-report-{run}-{attempt}-{case}"


def write(path: Path, value: dict[str, Any]) -> None:
    with path.open("x", encoding="utf-8") as stream:
        stream.write(json.dumps(value, sort_keys=True) + "\n")


def prepare(
    root: Path, automation: Path, candidate: str, selected: str, run: str, attempt: str, case: str
) -> dict[str, Any]:
    sha(candidate)
    sha(selected)
    require(git(automation, "rev-parse", "HEAD").decode().strip() == selected, "wrong source HEAD")
    require(not git(automation, "status", "--porcelain"), "dirty selected automation")
    hashes = {}
    for name in (MODULE, FIXTURE, WORKFLOW):
        data = regular(automation / name, 512 * 1024)
        require(
            data == git(automation, "show", f"{selected}:{name}"), "source differs from Git blob"
        )
        hashes[name] = hashlib.sha256(data).hexdigest()
    identity = {
        "schema": 1,
        "kind": KIND,
        "candidate_sha": candidate,
        "policy_sha": candidate,
        "automation_sha": selected,
        "run_id": run,
        "run_attempt": attempt,
        "case": case,
        "source_sha256": hashes,
    }
    root.mkdir(exist_ok=False)
    write(root / "identity.json", identity)
    return identity


def owner(
    root: Path, candidate: str, selected: str, run: str, attempt: str, case: str
) -> dict[str, Any]:
    require(root.is_dir() and not root.is_symlink(), "missing owned diagnostic directory")
    identity = document(root / "identity.json", 8192)
    for key, value in {
        "schema": 1,
        "kind": KIND,
        "candidate_sha": sha(candidate),
        "policy_sha": candidate,
        "automation_sha": sha(selected),
        "run_id": run,
        "run_attempt": attempt,
        "case": case,
    }.items():
        require(
            type(identity.get(key)) is type(value) and identity[key] == value, "foreign identity"
        )
    hashes = identity.get("source_sha256")
    if not isinstance(hashes, dict) or set(hashes) != {MODULE, FIXTURE, WORKFLOW}:
        raise ValueError("invalid hashes")
    require(
        all(
            isinstance(value, str) and re.fullmatch(r"[a-f0-9]{64}", value)
            for value in hashes.values()
        ),
        "invalid source digest",
    )
    return identity


def safe_log(data: bytes, identity: dict[str, Any], failed: bool) -> str:
    require(len(data) <= 4096, "resolver stderr exceeds log budget")
    text = data.decode("utf-8")
    require(
        "authored-source-report-canary" not in text
        and "::" not in text
        and "##[" not in text
        and all(ord(char) >= 32 or char == "\n" for char in text),
        "unsafe resolver stderr",
    )
    if failed:
        lines = text.splitlines()
        require(
            len(lines) == 2
            and lines[1]
            == (
                "Dev Container source resolution failed. Inspect the bounded diagnostic artifact; "
                "if absent, inspect resolver and artifact-upload logs before retrying."
            ),
            "expected one original failure record and canonical static guidance",
        )
        record = json.loads(lines[0])
        require(isinstance(record, dict), "invalid original failure record")
        for key in ("candidate_sha", "policy_sha", "automation_sha"):
            require(record.get(key) == identity[key], "log binding mismatch")
        require(
            type(record.get("schema")) is int
            and record["schema"] == 1
            and record.get("command") == "resolve"
            and record.get("status") == "failure"
            and record.get("repository") == "boxferry"
            and record.get("stage") == "metadata"
            and record.get("cause") == EXPECTED_CAUSES[identity["case"]]
            and record.get("cause_truncated") is False
            and record.get("next_action"),
            "missing original cause or stage",
        )
    else:
        require(not text, "successful fixture unexpectedly logged a failure")
    return text


def observe(root: Path, identity: dict[str, Any]) -> None:
    case = identity["case"]
    fixture_case, code, requests = CASES[case]
    directory = root / "fixture"
    summary = document(directory / "fixture.json", 8192)
    for key, value in {
        "schema": 1,
        "kind": KIND,
        "case": fixture_case,
        "candidate_sha": identity["candidate_sha"],
        "policy_sha": identity["policy_sha"],
        "automation_sha": identity["automation_sha"],
        "actual_cli_exit": code,
        "metadata_requests": requests,
        "report_present": case != "missing-report",
        "exports_present": code == 0,
        "controlled_report_removal": case == "missing-report",
        "module_sha256": identity["source_sha256"][MODULE],
    }.items():
        require(type(summary.get(key)) is type(value) and summary[key] == value, "fixture mismatch")
    stderr = safe_log(regular(directory / "resolver.stderr", 4096), identity, bool(code))
    report = directory / "devcontainer-sources.json"
    if case == "missing-report":
        require(not report.exists() and not report.is_symlink(), "missing-report still exists")
    else:
        data = regular(report)
        require(
            b"authored-source-report-canary" not in data, "report contains protected fixture data"
        )
        parsed = json.loads(data)
        require(isinstance(parsed, dict), "invalid report object")
        if code:
            for key in ("candidate_sha", "policy_sha", "automation_sha"):
                require(parsed.get(key) == identity[key], "report binding mismatch")
            require(
                type(parsed.get("schema")) is int
                and parsed["schema"] == 1
                and parsed.get("status") == "failure"
                and parsed.get("command") == "resolve"
                and parsed.get("repository") == "boxferry"
                and parsed.get("stage") == "metadata"
                and parsed.get("cause") == EXPECTED_CAUSES[case]
                and parsed.get("next_action")
                and "repositories" not in parsed,
                "invalid failure report",
            )
        else:
            sources = parsed.get("repositories")
            require(
                parsed.get("schema") == 1
                and isinstance(sources, dict)
                and len(sources) == 7
                and sources.get("boxferry") == identity["candidate_sha"]
                and all(value == identity["candidate_sha"] for value in sources.values()),
                "invalid synthetic success report",
            )
        if case == "unavailable-report-loss":
            write(
                root / "controlled-file-loss.json",
                {
                    "schema": 1,
                    "kind": KIND,
                    "operation": "controlled-file-loss-not-provider-outage",
                    "report_sha256_before_removal": hashlib.sha256(data).hexdigest(),
                    "candidate_sha": identity["candidate_sha"],
                    "policy_sha": identity["policy_sha"],
                    "automation_sha": identity["automation_sha"],
                    "run_id": identity["run_id"],
                    "run_attempt": identity["run_attempt"],
                    "case": case,
                },
            )
            report.unlink()
    exports = directory / "github-output"
    if code:
        require(not exports.exists() and not exports.is_symlink(), "failure exported acceptance")
    else:
        value = regular(exports).decode()
        require("sources=" in value and "source_digest=" in value, "successful exports absent")
    # Forward the actual captured CLI bytes, after validation, to the real hosted job log.
    sys.stdout.write(stderr)
    sys.stdout.flush()


def sidecars(root: Path, identity: dict[str, Any]) -> None:
    """Require every fixed expected sidecar before the multi-path upload."""
    paths = [
        root / "identity.json",
        root / "fixture/fixture.json",
        root / "fixture/resolver.stderr",
    ]
    loss = root / "controlled-file-loss.json"
    if identity["case"] == "unavailable-report-loss":
        removed = document(loss, 8192)
        require(
            removed.get("kind") == KIND
            and removed.get("operation") == "controlled-file-loss-not-provider-outage"
            and re.fullmatch(r"[a-f0-9]{64}", removed.get("report_sha256_before_removal", ""))
            and all(
                removed.get(key) == identity[key]
                for key in (
                    "candidate_sha",
                    "policy_sha",
                    "automation_sha",
                    "run_id",
                    "run_attempt",
                    "case",
                )
            ),
            "invalid controlled file loss sidecar",
        )
        require(not (root / "fixture/devcontainer-sources.json").exists(), "report loss incomplete")
        paths.append(loss)
    else:
        require(not loss.exists() and not loss.is_symlink(), "unexpected file loss sidecar")
    for path in paths:
        data = regular(path, 8192)
        require(b"authored-source-report-canary" not in data, "unsafe diagnostic sidecar")
    safe_log(
        regular(root / "fixture/resolver.stderr", 4096), identity, bool(CASES[identity["case"]][1])
    )


def canonical(automation: Path) -> tuple[dict[str, Any], dict[str, Any]]:
    import yaml

    try:
        workflow = yaml.safe_load(regular(automation / WORKFLOW, 512 * 1024))
    except yaml.YAMLError:
        raise ValueError("invalid selected canonical workflow YAML") from None
    steps = workflow["jobs"]["sources"]["steps"]
    retain = next(step for step in steps if step.get("id") == "retain")
    final = steps[-1]
    require(
        re.fullmatch(r"actions/upload-artifact@[a-f0-9]{40}", retain.get("uses", ""))
        and retain.get("if") == "always()"
        and retain["with"].get("path") == "${{ runner.temp }}/devcontainer-sources.json"
        and retain["with"].get("if-no-files-found") == "error"
        and retain["with"].get("retention-days") == 7,
        "canonical upload contract changed",
    )
    require(
        final.get("if") == "always()"
        and final.get("shell") == "bash"
        and final.get("env")
        == {
            "RESOLVE_OUTCOME": "${{ steps.resolve.outcome }}",
            "RETAIN_OUTCOME": "${{ steps.retain.outcome }}",
        }
        and isinstance(final.get("run"), str)
        and 0 < len(final["run"].encode()) <= 8192,
        "canonical final step contract changed",
    )
    return retain, final


def final_check(automation: Path, resolve: str, retain: str, selected: str | None = None) -> int:
    if selected is not None:
        sha(selected)
        require(
            git(automation, "rev-parse", "HEAD").decode().strip() == selected, "wrong final source"
        )
        require(
            regular(automation / WORKFLOW, 512 * 1024)
            == git(automation, "show", f"{selected}:{WORKFLOW}"),
            "canonical final source differs from selected Git blob",
        )
    for value in (resolve, retain):
        require(value in {"success", "failure", "cancelled", "skipped"}, "invalid actual outcome")
    _, final = canonical(automation)
    result = subprocess.run(
        ["bash", "--noprofile", "--norc", "-c", final["run"]],
        env={"PATH": os.defpath, "RESOLVE_OUTCOME": resolve, "RETAIN_OUTCOME": retain},
        capture_output=True,
        check=False,
        timeout=5,
    )
    require(
        len(result.stdout) + len(result.stderr) <= 16384, "canonical final output exceeds bound"
    )
    sys.stdout.write(result.stdout.decode("utf-8"))
    sys.stderr.write(result.stderr.decode("utf-8"))
    return result.returncode


def cleanup(root: Path) -> None:
    """Preflight the entire small owned tree before deleting any entry."""
    allowed = {
        "identity.json",
        "controlled-file-loss.json",
        "fixture/fixture.json",
        "fixture/resolver.stderr",
        "fixture/devcontainer-sources.json",
        "fixture/github-output",
    }
    entries = list(root.iterdir())
    fixture = root / "fixture"
    if fixture.exists() or fixture.is_symlink():
        require(fixture.is_dir() and not fixture.is_symlink(), "invalid fixture directory")
        entries.extend(fixture.iterdir())
    files = []
    for entry in entries:
        if entry == fixture:
            continue
        require(entry.relative_to(root).as_posix() in allowed, "unknown diagnostic evidence entry")
        regular(entry)
        files.append(entry)
    for entry in files:
        entry.unlink()
    if fixture.exists():
        fixture.rmdir()
    root.rmdir()
    require(not root.exists(), "diagnostic cleanup incomplete")
    print("Owned diagnostic fixture cleanup succeeded.")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "command", choices=("manifest", "prepare", "observe", "sidecars", "final", "cleanup")
    )
    parser.add_argument("--candidate", type=Path, default=Path("candidate"))
    parser.add_argument("--automation", type=Path, default=Path("_automation"))
    parser.add_argument("--case", choices=tuple(CASES))
    args = parser.parse_args()
    candidate = os.environ["GITHUB_SHA"]
    if args.command == "manifest":
        value = manifest(args.candidate, candidate)
        with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as output:
            output.write(f"revision={value['revision']}\npolicy={value['policy']}\n")
        return 0
    selected = sha(os.environ["AUTOMATION_SHA"])
    if args.command == "final":
        return final_check(
            args.automation, os.environ["RESOLVE_OUTCOME"], os.environ["RETAIN_OUTCOME"], selected
        )
    require(args.case is not None, "case required")
    run, attempt = os.environ["GITHUB_RUN_ID"], os.environ["GITHUB_RUN_ATTEMPT"]
    root = location(Path(os.environ["RUNNER_TEMP"]), run, attempt, args.case)
    if args.command == "prepare":
        value = manifest(args.candidate, candidate)
        require(value["revision"] == selected, "selected source differs from candidate manifest")
        prepare(root, args.automation, candidate, selected, run, attempt, args.case)
        canonical(args.automation)
        with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as output:
            output.write(f"directory={root}\n")
    else:
        identity = owner(root, candidate, selected, run, attempt, args.case)
        if args.command == "observe":
            observe(root, identity)
        elif args.command == "sidecars":
            sidecars(root, identity)
        elif args.command == "cleanup":
            cleanup(root)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (
        OSError,
        ValueError,
        TypeError,
        KeyError,
        StopIteration,
        RecursionError,
        subprocess.SubprocessError,
    ):
        # Never forward raw exceptions, paths, environment or subprocess output to Actions.
        print(
            "Source-report diagnostic contract failed; inspect the bounded retained evidence.",
            file=sys.stderr,
        )
        raise SystemExit(1) from None
