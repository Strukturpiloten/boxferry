"""Shared workflow entry point. All repository configuration is explicit."""

from __future__ import annotations

import argparse
import json
import os
import signal
import sys
import tempfile
import time
import tomllib
from pathlib import Path

from boxferry_ci.contracts import ContractError, exact_sha, object_value, relative_path
from boxferry_ci.gate import GateFailure, verify_gate
from boxferry_ci.planner import Comparison, git, git_text, make_plan, verify_policy_revision
from boxferry_ci.policy import DEFAULT_TASKS, Policy, Task, load_policy
from boxferry_ci.runner import RunCancelled, RunResult, protected_values, redact, run


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("plan", "gate", "run", "run-local"):
        command = sub.add_parser(name)
        command.add_argument("--repository-root", type=Path, required=True)
        location = command.add_mutually_exclusive_group(required=True)
        location.add_argument("--config", type=Path)
        location.add_argument("--policy-root", type=Path)
        command.add_argument("--event", default="local" if name == "run-local" else "")
        command.add_argument("--base-sha", default="")
        command.add_argument("--head-sha", default="")
        command.add_argument("--tested-sha", "--candidate-sha", dest="tested_sha", default="")
        command.add_argument("--policy-sha", default="")
        command.add_argument("--automation-sha", default="")
        command.add_argument("--report-json", type=Path)
        command.add_argument("--summary", type=Path)
        if name in {"plan", "run-local"}:
            command.add_argument("--base-ref", default="origin/main")
            command.add_argument("--force-full", "--full", dest="force_full", action="store_true")
        if name == "plan":
            command.add_argument("--github-output", type=Path)
        elif name == "gate":
            command.add_argument("--plan-json", required=True)
            command.add_argument("--needs-json", required=True)
        elif name == "run":
            command.add_argument("--task", required=True)
        else:
            command.add_argument("--docs-only", action="store_true")
    dependencies = sub.add_parser("dependencies")
    dependencies.add_argument("--base", type=Path, required=True)
    dependencies.add_argument("--head", type=Path, required=True)
    dependencies.add_argument("--repository-root", type=Path)
    dependencies.add_argument("--state", type=Path)
    dependencies.add_argument("--output", type=Path, required=True)
    sub.add_parser("tools", help="Install canonical integrity-pinned Linux development tools")
    return parser


def _policy(args: argparse.Namespace) -> Policy:
    config = args.config or args.policy_root / "scripts/validation-policy.json"
    policy = load_policy(config)
    if args.automation_sha:
        expected = exact_sha(args.automation_sha, "automation SHA")
        actual = git_text(Path(__file__).resolve().parents[3], "rev-parse", "HEAD")
        if actual != expected:
            raise ContractError("automation checkout does not match the requested immutable SHA")
    if args.command in {"run", "run-local"}:
        if args.tested_sha and git_text(args.repository_root, "rev-parse", "HEAD") != exact_sha(
            args.tested_sha, "tested SHA"
        ):
            raise ContractError("source checkout does not match the tested SHA")
        if args.event in {"pull_request", "merge_group"}:
            verify_policy_revision(
                policy,
                args.policy_sha,
                Comparison(
                    args.event,
                    exact_sha(args.base_sha, "base SHA"),
                    exact_sha(args.head_sha, "head SHA"),
                    exact_sha(args.tested_sha, "tested SHA"),
                ),
            )
    return policy


def _task(args: argparse.Namespace, policy: Policy) -> Task:
    if args.task != "rust-quality" or args.task in policy.tasks:
        return policy.task(args.task)
    # Optional aliases are selected from trusted policy history, never PR source.
    trusted_root = Path(git_text(policy.path.parent, "rev-parse", "--show-toplevel"))
    revision = exact_sha(
        args.policy_sha or git_text(trusted_root, "rev-parse", "HEAD"), "policy SHA"
    )
    raw = git(trusted_root, "show", f"{revision}:.cargo/config.toml")
    if raw is None:
        raise ContractError("trusted Rust alias inventory is missing")
    aliases = object_value(tomllib.loads(raw.decode()).get("alias"), "trusted cargo aliases")
    mandatory = ("ci-check", "ci-policy", "ci-clippy", "ci-test", "ci-doctest", "ci-doc")
    if not set(mandatory) <= aliases.keys():
        raise ContractError("trusted Rust alias inventory lacks the standard quality contract")
    optional = ("ci-core", "ci-compose", "ci-podman", "ci-quadlet", "ci-application", "ci-package")
    calls = [DEFAULT_TASKS["rust-quality"][0]]
    calls.extend(("cargo", alias) for alias in (*mandatory, *optional) if alias in aliases)
    return Task(tuple(calls))


def _snapshot(root: Path, revision: str, target: Path) -> None:
    from .operational import MAX_FILE_BYTES, SnapshotCache, inventory_blobs

    revision = exact_sha(revision, "dependency revision")
    listing = git(root, "ls-tree", "-r", "-z", revision) or b""
    if len(listing) > 4 * 1024 * 1024:
        raise ContractError("dependency tree exceeded its metadata byte budget")
    entries: list[dict[str, str]] = []
    for raw in listing.split(b"\0"):
        if not raw:
            continue
        metadata, raw_path = raw.split(b"\t", 1)
        mode, kind, sha = metadata.decode("ascii").split(" ")
        entries.append(
            {
                "mode": mode,
                "type": kind,
                "sha": sha,
                "path": relative_path(raw_path.decode("utf-8"), "dependency metadata path"),
            }
        )
    operational = inventory_blobs({"truncated": False, "tree": entries})
    cache = SnapshotCache()
    started = time.monotonic()
    count, total = 0, 0
    for entry in entries:
        path = entry["path"]
        if path not in {
            "Cargo.lock",
            "package-lock.json",
            "uv.lock",
            "automation/uv.lock",
            "Cargo.toml",
        }:
            if path not in operational and (
                not path.startswith("crates/") or not path.endswith("/Cargo.toml")
            ):
                continue
        if entry["type"] != "blob" or entry["mode"] not in {"100644", "100755"}:
            raise ContractError("dependency input is not a regular immutable Git blob")
        count += 1
        if count > 128 or time.monotonic() - started > 60:
            raise ContractError("dependency snapshot exceeded its file/time budget")
        content = git(root, "show", f"{revision}:{path}")
        if content is None:
            raise ContractError(f"dependency metadata unavailable at {revision}: {path}")
        if len(content) > MAX_FILE_BYTES:
            raise ContractError("dependency snapshot file exceeded its byte budget")
        total += len(content)
        if total > 8_000_000:
            raise ContractError("dependency snapshot exceeded its total byte budget")
        cache.put(entry["sha"], content)
        destination = target / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(content)


def _plan(args: argparse.Namespace, policy: Policy) -> dict[str, object]:
    return make_plan(
        args.repository_root,
        policy,
        event=args.event,
        base_sha=args.base_sha,
        head_sha=args.head_sha,
        tested_sha=args.tested_sha,
        policy_sha=args.policy_sha,
        base_ref=getattr(args, "base_ref", "origin/main"),
        force_full=getattr(args, "force_full", False),
    )


def _write_report(args: argparse.Namespace, report: dict[str, object]) -> None:
    path: Path | None = getattr(args, "report_json", None)
    if path is not None:
        path.parent.mkdir(parents=True, exist_ok=True)
        encoded = redact(json.dumps(report, indent=2, sort_keys=True), protected_values(os.environ))
        path.write_text(encoded + "\n", encoding="utf-8")
    summary: Path | None = getattr(args, "summary", None)
    if summary is None and os.environ.get("GITHUB_STEP_SUMMARY"):
        summary = Path(os.environ["GITHUB_STEP_SUMMARY"])
    if summary is not None:
        summary.parent.mkdir(parents=True, exist_ok=True)
        with summary.open("a", encoding="utf-8") as stream:
            stream.write(f"Validation: {report.get('status', 'unknown')}\n\n")
            results = report.get("results", [])
            if isinstance(results, list):
                for result in results:
                    if isinstance(result, dict) and isinstance(result.get("diagnostic"), dict):
                        if isinstance(result.get("argv"), list):
                            command = redact(
                                json.dumps(result["argv"]), protected_values(os.environ)
                            )
                            stream.write(f"- argv: {command}\n")
                        diagnostic = result["diagnostic"]
                        for key in (
                            "stage",
                            "cause",
                            "affected_input",
                            "evidence_link",
                            "next_action",
                        ):
                            text = redact(
                                str(diagnostic.get(key, "")), protected_values(os.environ)
                            )
                            text = text.replace("\n", " ")
                            stream.write(f"- {key}: {text}\n")
            if "cause" in report:
                stream.write(f"Cause: {report['cause']}\n")


def _execute(args: argparse.Namespace, calls: tuple[tuple[str, ...], ...], timeout: int) -> int:
    root = args.repository_root.resolve()
    target = os.environ.get("CARGO_TARGET_DIR")
    if target:
        resolved = (root / target).resolve()
        if resolved == root or not resolved.is_relative_to(root):
            raise ContractError("CARGO_TARGET_DIR must stay inside the current candidate worktree")
    results: list[RunResult] = []
    for index, call in enumerate(calls):
        result = run(
            call,
            cwd=args.repository_root.resolve(),
            stage=f"{getattr(args, 'task', 'local')}:{index + 1}",
            timeout_seconds=timeout,
            affected_input=args.tested_sha or "local checkout",
            evidence_link=str(args.report_json or "captured command output"),
        )
        results.append(result)
        sys.stdout.write(result.stdout)
        sys.stderr.write(result.stderr)
        if result.status != "success":
            break
    status = results[-1].status
    _write_report(args, {"schema": 1, "status": status, "results": [r.to_dict() for r in results]})
    if status == "cancelled":
        return 130
    return 0 if status == "success" else 1


def _cancel(_signum: int, _frame: object) -> None:
    raise RunCancelled("workflow cancellation")


def main(argv: list[str] | None = None) -> int:
    arguments = list(sys.argv[1:] if argv is None else argv)
    if arguments and arguments[0] == "devcontainer-local":
        from boxferry_ci.devcontainer_local import main as devcontainer_local_main

        return devcontainer_local_main(arguments[1:])
    if arguments and arguments[0] == "cargo-tools":
        from boxferry_ci.cargo_tools import main as cargo_tools_main

        return cargo_tools_main(arguments[1:])
    if arguments and arguments[0] == "tools":
        from boxferry_ci.tooling import main as tools_main

        return tools_main(arguments[1:])
    args = _parser().parse_args(arguments)
    previous = signal.signal(signal.SIGTERM, _cancel)
    try:
        if args.command == "dependencies":
            from boxferry_ci.dependencies import check_dependencies

            state = None
            if args.state and args.state.exists():
                state = object_value(json.loads(args.state.read_text()), "dependency state")
            if args.repository_root:
                with tempfile.TemporaryDirectory(prefix="boxferry-dependencies-") as temporary:
                    base, head = Path(temporary) / "base", Path(temporary) / "head"
                    base.mkdir()
                    head.mkdir()
                    _snapshot(args.repository_root, str(args.base), base)
                    _snapshot(args.repository_root, str(args.head), head)
                    report = check_dependencies(base, head, state=state)
            else:
                report = check_dependencies(args.base, args.head, state=state)
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
            print(json.dumps(report, sort_keys=True))
            return 0 if report["status"] == "pass" else 1
        policy = _policy(args)
        if args.command == "plan":
            plan = _plan(args, policy)
            encoded = json.dumps(plan, sort_keys=True, separators=(",", ":"))
            if args.github_output:
                with args.github_output.open("a", encoding="utf-8") as stream:
                    stream.write(f"plan={encoded}\nprofile={plan['profile']}\n")
                    jobs = object_value(plan["jobs"], "plan jobs")
                    for job, selected in jobs.items():
                        stream.write(f"select_{job.replace('-', '_')}={str(selected).lower()}\n")
            print(encoded)
            _write_report(args, {"schema": 1, "status": "success", "plan": plan})
        elif args.command == "gate":
            comparison = Comparison(
                args.event,
                exact_sha(args.base_sha or args.tested_sha, "base SHA"),
                exact_sha(args.head_sha or args.tested_sha, "head SHA"),
                exact_sha(args.tested_sha, "tested SHA"),
            )
            results = verify_gate(
                json.loads(args.plan_json),
                json.loads(args.needs_json),
                policy,
                comparison,
                expected_plan=_plan(args, policy),
            )
            gate_report: dict[str, object] = {"status": "success", "results": results}
            print(json.dumps(gate_report, sort_keys=True))
            _write_report(args, gate_report)
        elif args.command == "run":
            task = _task(args, policy)
            return _execute(args, task.commands, task.timeout_seconds)
        else:
            plan = _plan(args, policy)
            profiles = plan["profiles"]
            if not isinstance(profiles, list):
                raise ContractError("invalid local plan profiles")
            if args.docs_only and profiles == ["full"]:
                raise ContractError(
                    "docs-only cannot omit checks required by the actual changed paths"
                )
            calls: list[tuple[str, ...]] = []
            for profile in profiles:
                if not isinstance(profile, str):
                    raise ContractError("invalid local profile")
                for call in policy.local_commands[profile]:
                    if call not in calls:
                        calls.append(call)
            return _execute(args, tuple(calls), 7200)
    except (OSError, ValueError, RunCancelled) as error:
        cause = redact(str(error), protected_values(os.environ))
        diagnostic = {
            "stage": args.command,
            "cause": cause,
            "affected_input": str(getattr(args, "repository_root", "validation input")),
            "evidence_link": str(getattr(args, "report_json", None) or "captured command output"),
            "next_action": (
                "Correct the named input or infrastructure cause and rerun the same candidate."
            ),
        }
        report = {
            "schema": 1,
            "status": "failed" if isinstance(error, GateFailure) else "infrastructure",
            "stage": args.command,
            "cause": cause,
            "results": [{"diagnostic": diagnostic}],
        }
        _write_report(args, report)
        print(json.dumps(report, sort_keys=True), file=sys.stderr)
        return 1
    finally:
        signal.signal(signal.SIGTERM, previous)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
