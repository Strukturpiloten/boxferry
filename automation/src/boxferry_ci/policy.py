"""Repository policy parsing; candidates cannot silently invent job selections."""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from pathlib import Path

from boxferry_ci.contracts import ContractError, object_value, relative_path, strings

DEFAULT_TASKS: dict[str, tuple[tuple[str, ...], ...]] = {
    "rust-quality": (
        ("cargo", "fmt", "--all", "--", "--check"),
        ("cargo", "ci-check"),
        ("cargo", "ci-policy"),
        ("cargo", "ci-clippy"),
        ("cargo", "ci-test"),
        ("cargo", "ci-doctest"),
        ("cargo", "ci-doc"),
    ),
    "msrv": (("cargo", "ci-check"), ("cargo", "ci-policy")),
}


@dataclass(frozen=True)
class Task:
    commands: tuple[tuple[str, ...], ...]
    timeout_seconds: int = 900


def commands(value: object, label: str) -> tuple[tuple[str, ...], ...]:
    if not isinstance(value, list) or not value:
        raise ContractError(f"{label} must contain at least one argv array")
    result: list[tuple[str, ...]] = []
    for call in value:
        if (
            not isinstance(call, list)
            or not call
            or any(not isinstance(arg, str) or not arg or "\x00" in arg for arg in call)
        ):
            raise ContractError(f"{label} must use nonempty, NUL-free argv arrays")
        argv = tuple(call)
        executable = Path(argv[0]).name
        if executable in {"sh", "bash", "dash", "zsh", "fish", "cmd", "powershell", "pwsh"}:
            if len(argv) < 2 or argv[1].startswith("-"):
                raise ContractError(f"{label} permits repository scripts, not shell expressions")
            relative_path(argv[1], f"{label} script")
        result.append(argv)
    return tuple(result)


@dataclass(frozen=True)
class Policy:
    repository: str
    jobs: tuple[str, ...]
    profile_jobs: dict[str, tuple[str, ...]]
    public_documentation_root: str
    full_documentation_roots: tuple[str, ...]
    executable_documentation_roots: tuple[str, ...]
    executable_documentation_paths: tuple[str, ...]
    documentation_examples_manifest: str | None
    local_commands: dict[str, tuple[tuple[str, ...], ...]]
    path_profiles: dict[str, tuple[str, ...]]
    tasks: dict[str, Task]
    path: Path

    def selected_jobs(self, profiles: tuple[str, ...]) -> set[str]:
        if profiles == ("full",):
            return set(self.jobs)
        selected: set[str] = set()
        for profile in profiles:
            if profile not in self.profile_jobs:
                raise ContractError(f"unknown validation profile {profile!r}")
            selected.update(self.profile_jobs[profile])
        return selected

    def task(self, name: str) -> Task:
        if name in self.tasks:
            return self.tasks[name]
        if name in DEFAULT_TASKS:
            return Task(DEFAULT_TASKS[name])
        raise ContractError(f"task {name!r} is not configured for {self.repository}")


def load_policy(path: Path) -> Policy:
    try:
        raw: object = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ContractError(f"cannot load validation policy {path}: {error}") from error
    data = object_value(raw, "validation policy")
    required = {
        "schema",
        "repository",
        "jobs",
        "profile_jobs",
        "public_documentation_root",
        "full_documentation_roots",
        "executable_documentation_roots",
        "executable_documentation_paths",
        "documentation_examples_manifest",
        "local_commands",
    }
    if not required <= data.keys() or data.keys() - required - {"tasks", "path_profiles"}:
        raise ContractError("validation policy has an unsupported key set")
    if type(data["schema"]) is not int or data["schema"] not in {2, 3}:
        raise ContractError("validation policy has an unsupported schema")
    repository = data["repository"]
    if not isinstance(repository, str) or re.fullmatch(r"[\w.-]+/[\w.-]+", repository) is None:
        raise ContractError("validation policy repository must be owner/name")
    jobs = strings(data["jobs"], "policy jobs")
    if any(re.fullmatch(r"[a-zA-Z0-9_-]+", job) is None for job in jobs):
        raise ContractError("policy job names must be safe workflow identifiers")
    profile_jobs: dict[str, tuple[str, ...]] = {}
    for profile, selected in object_value(data["profile_jobs"], "profile_jobs").items():
        if profile in {"full", "mixed"} or re.fullmatch(r"[a-zA-Z0-9_-]+", profile) is None:
            raise ContractError("full/mixed profiles are reserved")
        profile_jobs[profile] = strings(selected, f"{profile} jobs")
        if not set(profile_jobs[profile]) <= set(jobs):
            raise ContractError(f"{profile} selects unknown jobs")
    if not profile_jobs:
        raise ContractError("policy needs at least one selective profile")
    local_commands = {
        profile: commands(calls, f"{profile} local commands")
        for profile, calls in object_value(data["local_commands"], "local_commands").items()
    }
    if set(local_commands) != {*profile_jobs, "full"}:
        raise ContractError("local command profiles must match profile_jobs plus full")
    if "prose" in profile_jobs and "executable-docs" in profile_jobs:
        if not set(profile_jobs["prose"]) <= set(profile_jobs["executable-docs"]):
            raise ContractError("executable documentation must include prose jobs")
        executable = local_commands["executable-docs"]
        if executable != local_commands["full"] and not all(
            call in executable for call in local_commands["prose"]
        ):
            raise ContractError("executable documentation must include prose commands")
    roots: dict[str, tuple[str, ...]] = {}
    for key in (
        "full_documentation_roots",
        "executable_documentation_roots",
        "executable_documentation_paths",
    ):
        roots[key] = tuple(
            relative_path(item, key) for item in strings(data[key], key, nonempty=False)
        )
    public_root = relative_path(data["public_documentation_root"], "public documentation root")
    manifest = data["documentation_examples_manifest"]
    if manifest is not None:
        manifest = relative_path(manifest, "documentation examples manifest")
    path_profiles: dict[str, tuple[str, ...]] = {}
    for profile, patterns in object_value(data.get("path_profiles", {}), "path_profiles").items():
        if profile not in profile_jobs:
            raise ContractError(f"path profile {profile!r} has no job declaration")
        path_profiles[profile] = tuple(
            relative_path(pattern, f"{profile} path pattern", pattern=True)
            for pattern in strings(patterns, f"{profile} patterns")
        )
    tasks: dict[str, Task] = {}
    for name, raw_task in object_value(data.get("tasks", {}), "tasks").items():
        task = object_value(raw_task, f"task {name}")
        if set(task) - {"commands", "timeout_seconds"} or "commands" not in task:
            raise ContractError(f"task {name!r} has unknown or missing keys")
        timeout = task.get("timeout_seconds", 900)
        if type(timeout) is not int or not 0 < timeout <= 7200:
            raise ContractError(f"task {name!r} timeout must be in [1, 7200] seconds")
        tasks[name] = Task(commands(task["commands"], f"task {name}"), timeout)
    return Policy(
        repository,
        jobs,
        profile_jobs,
        public_root,
        roots["full_documentation_roots"],
        roots["executable_documentation_roots"],
        roots["executable_documentation_paths"],
        manifest,
        local_commands,
        path_profiles,
        tasks,
        path.resolve(),
    )
