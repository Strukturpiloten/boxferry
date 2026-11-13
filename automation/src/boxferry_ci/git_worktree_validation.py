"""Admitted BoxFerry-only host-created Git fixtures and exact-image mount receipts.

Preparation executes only source-owned code. The later image probe has no network,
receives finite read-only mounts, and shares the qualifier's original probe budget.
"""

from __future__ import annotations

import hashlib
import json
import os
import platform
import re
import stat
import subprocess
import sys
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

from boxferry_ci import git_worktree as source

TARGET = Path("/workspaces/.boxferry-git-contract")
CANDIDATE = Path("/workspaces/boxferry")
PACKAGE_TARGET = Path("/opt/boxferry-git-contract")
MAX_FILES = 512
MAX_BYTES = 2 * 1024 * 1024
MAX_RECEIPT = 32 * 1024
BASE = {
    "payload.bin": ("100644", b"baseline\0payload\n"),
    "executable.sh": ("100755", b"#!/bin/sh\nexit 0\n"),
    "staged-deleted.txt": ("100644", b"staged deletion\n"),
    "working-deleted.txt": ("100644", b"working deletion\n"),
    "link": ("120000", b"payload.bin"),
}
STAGED = {
    **BASE,
    "payload.bin": ("100644", b"staged\0payload\n"),
    "new.txt": ("100644", b"new staged blob\n"),
}
del STAGED["staged-deleted.txt"]
WORKING = {
    **STAGED,
    "payload.bin": ("100644", b"working\0payload\n"),
    "untracked.txt": ("100644", b"untracked bytes\n"),
}
del WORKING["working-deleted.txt"]
CASES = {
    "primary": "normal/checkout",
    "relative-leaf": "leaf/relative",
    "relative-topology": "topology/relative",
    "legacy": "legacy/checkout",
    "standalone": "standalone/checkout",
}
REFUSALS = {"relative-leaf": "relative-metadata-unmounted", "legacy": "legacy-metadata-unmounted"}
SPEC = {
    "schema": 1,
    "cases": CASES,
    "refusals": REFUSALS,
    "baseline": {name: [mode, data.hex()] for name, (mode, data) in BASE.items()},
    "staged": {name: [mode, data.hex()] for name, (mode, data) in STAGED.items()},
    "working": {name: [mode, data.hex()] for name, (mode, data) in WORKING.items()},
    "max_files": MAX_FILES,
    "max_bytes": MAX_BYTES,
}
SPEC_SHA256 = hashlib.sha256(json.dumps(SPEC, sort_keys=True).encode()).hexdigest()


# System Python can be 3.11 in the immutable base. Re-exec only the image's
# retained locked interpreter after binding its manifest to the admitted source.
IMAGE_PYTHON = r"""
import json, os, pathlib, re, sys
root = pathlib.Path('/opt/boxferry-tooling')
manifest = root / '.github/automation.json'
if manifest.stat().st_size > 4096:
    raise ValueError('retained automation manifest exceeds its bound')
document = json.loads(manifest.read_text())
if not isinstance(document, dict):
    raise ValueError('retained automation manifest must be an object')
revision = document['revision']
if not isinstance(revision, str) or re.fullmatch(r'[a-f0-9]{40}', revision) is None:
    raise ValueError('retained automation revision is not an exact commit')
if revision != sys.argv[1]:
    raise ValueError('image interpreter source differs from admitted automation')
python = root / 'target/boxferry-automation' / revision / 'automation/.venv/bin/python'
if not python.is_file() or not os.access(python, os.X_OK):
    raise ValueError('retained locked Python is unavailable')
check = (
    "import sys\n"
    "if sys.version_info < (3,13):\n"
    " raise SystemExit('Retained locked Python must be 3.13 or newer; rebuild the image')\n"
)
os.execv(python, [str(python), '-I', '-c', check + sys.argv[2]])
"""


class FixtureError(ValueError):
    """Native fixture custody, execution or expected evidence is invalid."""


class Commands(Protocol):
    def call(
        self,
        argv: tuple[str, ...],
        stage: str,
        timeout: int = 30,
        *,
        cwd: Path | None = None,
        max_output_bytes: int = 2_000_000,
    ) -> str: ...
    def git(self, root: Path, *argv: str) -> str: ...


def strict_json(raw: str) -> object:
    def unique(pairs: list[tuple[str, object]]) -> dict[str, object]:
        result: dict[str, object] = {}
        for key, value in pairs:
            if key in result:
                raise FixtureError("duplicate native receipt/expectation field")
            result[key] = value
        return result

    return json.loads(raw, object_pairs_hook=unique)


def object_id(kind: str, content: bytes) -> str:
    return hashlib.sha1(
        kind.encode() + b" " + str(len(content)).encode() + b"\0" + content
    ).hexdigest()


def entries(values: Mapping[str, tuple[str, bytes]]) -> bytes:
    return b"".join(
        f"{mode} {object_id('blob', data)} 0\t{name}\0".encode()
        for name, (mode, data) in sorted(values.items())
    )


def baseline_head() -> str:
    tree = b"".join(
        f"{mode} {name}\0".encode() + bytes.fromhex(object_id("blob", data))
        for name, (mode, data) in sorted(BASE.items())
    )
    identity = "BoxFerry fixture <fixture@example.invalid> 946684800 +0000\n"
    commit = (
        f"tree {object_id('tree', tree)}\nauthor {identity}committer {identity}\n"
        "Authored Git worktree fixture\n"
    ).encode()
    return object_id("commit", commit)


def working_files() -> dict[str, dict[str, object]]:
    result: dict[str, dict[str, object]] = {
        name: {
            "kind": "symlink" if mode == "120000" else "file",
            "mode": 0o777 if mode == "120000" else 0o755 if mode == "100755" else 0o644,
            "sha256": hashlib.sha256(data).hexdigest(),
        }
        for name, (mode, data) in WORKING.items()
    }
    result["working-deleted.txt"] = {"kind": "deleted"}
    return result


def digest_file(path: Path) -> str:
    if path.is_symlink() or not path.is_file() or path.stat().st_size > MAX_BYTES:
        raise FixtureError("fixture/source file must be bounded and regular")
    return hashlib.sha256(path.read_bytes()).hexdigest()


def fingerprint(root: Path) -> str:
    values: dict[str, object] = {}
    total = 0
    for path in sorted(root.rglob("*")):
        mode = path.lstat().st_mode
        name = path.relative_to(root).as_posix()
        if len(values) >= MAX_FILES:
            raise FixtureError("authored fixture exceeds its file bound")
        if stat.S_ISLNK(mode):
            values[name] = ["link", os.readlink(path)]
        elif stat.S_ISDIR(mode):
            values[name] = ["directory", stat.S_IMODE(mode)]
        elif stat.S_ISREG(mode):
            total += path.stat().st_size
            if total > MAX_BYTES:
                raise FixtureError("authored fixture exceeds its byte bound")
            values[name] = ["file", stat.S_IMODE(mode), digest_file(path)]
        else:
            raise FixtureError("authored fixture has an unsupported member")
    return hashlib.sha256(json.dumps(values, sort_keys=True).encode()).hexdigest()


def public_fixture(root: Path) -> None:
    """Only authored task fixtures become readable by the different image UID."""
    for path in (root, *root.rglob("*")):
        if not path.is_symlink():
            path.chmod(0o755 if path.is_dir() or path.stat().st_mode & 0o111 else 0o644)


def write_members(root: Path, members: Mapping[str, tuple[str, bytes]]) -> None:
    for name, (mode, data) in members.items():
        path = root / name
        if mode == "120000":
            path.symlink_to(os.fsdecode(data))
        else:
            path.write_bytes(data)
            path.chmod(0o755 if mode == "100755" else 0o644)


def initialize(root: Path, empty: Path, commands: Commands) -> None:
    root.mkdir()
    commands.git(
        root, "init", "--initial-branch=main", "--object-format=sha1", f"--template={empty}"
    )
    write_members(root, BASE)
    commands.git(root, "add", "--", *BASE)
    commands.call(
        (
            "/usr/bin/env",
            "GIT_AUTHOR_DATE=2000-01-01T00:00:00+00:00",
            "GIT_COMMITTER_DATE=2000-01-01T00:00:00+00:00",
            "git",
            "-c",
            "user.name=BoxFerry fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
            "-C",
            str(root),
            "commit",
            "--no-verify",
            "--no-gpg-sign",
            "--cleanup=verbatim",
            "-m",
            "Authored Git worktree fixture",
        ),
        "git-fixture-commit",
    )
    if commands.git(root, "rev-parse", "HEAD").strip() != baseline_head():
        raise FixtureError(
            "authored baseline HEAD differs from independent Git object specification"
        )


def dirty(root: Path, commands: Commands) -> None:
    write_members(root, {key: STAGED[key] for key in ("payload.bin", "new.txt")})
    commands.git(root, "add", "--", "payload.bin", "new.txt")
    commands.git(root, "rm", "--", "staged-deleted.txt")
    write_members(root, {key: WORKING[key] for key in ("payload.bin", "untracked.txt")})
    (root / "working-deleted.txt").unlink()
    if commands.git(root, "ls-files", "--stage", "-z").encode() != entries(STAGED):
        raise FixtureError("authored staged blobs/modes differ from independent expectations")


@dataclass(frozen=True)
class Fixtures:
    root: Path
    expected_file: Path
    package: Path
    frozen: str
    expected: dict[str, object]


def prepare(private: Path, automation: Path, candidate: Path, commands: Commands) -> Fixtures:
    package = Path(__file__).resolve().parent
    if package != (automation / "automation/src/boxferry_ci").resolve():
        raise FixtureError("host fixture helper is not the explicit immutable automation source")
    root = private / "git-worktree-contract"
    root.mkdir()
    empty = root / "empty-template"
    empty.mkdir()
    host_version = commands.git(root, "--version").strip()
    primary, relative = root / "primary", root / "relative"
    legacy_primary, legacy, standalone = (
        root / name for name in ("legacy-primary", "legacy", "standalone")
    )
    initialize(primary, empty, commands)
    try:
        commands.git(
            primary, "worktree", "add", "--relative-paths", "-b", "relative", str(relative)
        )
    except ValueError as error:
        raise FixtureError(
            f"native host {host_version} must create real relative worktrees; "
            "upgrade the runner prerequisite, never emulate metadata"
        ) from error
    initialize(legacy_primary, empty, commands)
    commands.git(
        legacy_primary, "worktree", "add", "--no-relative-paths", "-b", "legacy", str(legacy)
    )
    for path in (primary, relative, legacy):
        dirty(path, commands)
    # Invoke only admitted source, under the existing runner's finite environment,
    # absolute budget monitor, process-group cleanup and output bound.
    program = (
        "import sys;from pathlib import Path;sys.path.insert(0,sys.argv[1]);"
        "from boxferry_ci.git_worktree import standalone;"
        "standalone(Path(sys.argv[2]),Path(sys.argv[3]))"
    )
    commands.call(
        (sys.executable, "-I", "-c", program, str(package.parent), str(legacy), str(standalone)),
        "git-fixture-standalone",
        120,
    )
    for path in (primary, relative, legacy, standalone):
        if commands.git(path, "rev-parse", "HEAD").strip() != baseline_head():
            raise FixtureError("host fixture changed baseline identity")
        if commands.git(path, "ls-files", "--stage", "-z").encode() != entries(STAGED):
            raise FixtureError("host fixture lost staged bytes or modes")
    relative_pointer = (relative / ".git").read_text()
    legacy_pointer = (legacy / ".git").read_text()
    if (
        relative_pointer != "gitdir: ../primary/.git/worktrees/relative\n"
        or not legacy_pointer.startswith("gitdir: /")
    ):
        raise FixtureError("host Git did not produce untouched expected relative/absolute metadata")
    public_fixture(root)
    helper_hashes = {
        name: digest_file(package / name)
        for name in ("__init__.py", "git_worktree.py", "git_worktree_validation.py")
    }
    candidate_hashes = {
        name: digest_file(candidate / name)
        for name in ("scripts/git-worktree.py", "automation/src/boxferry_ci/git_worktree.py")
    }
    from boxferry_ci.git_source import inventory

    _document, git_sources = inventory((candidate / "automation/tool-artifacts.json").read_bytes())
    expected_version = git_sources[0].version if len(git_sources) == 1 else None
    formats = {
        name: int(commands.git(path, "config", "--get", "core.repositoryformatversion").strip())
        for name, path in (
            ("primary", primary),
            ("relative", relative),
            ("legacy", legacy),
            ("standalone", standalone),
        )
    }
    if formats != {"primary": 1, "relative": 1, "legacy": 0, "standalone": 0}:
        raise FixtureError("native host produced unsupported fixture repository formats")
    expected: dict[str, object] = {
        "schema": 1,
        "spec_sha256": SPEC_SHA256,
        "head": baseline_head(),
        "index_sha256": hashlib.sha256(entries(STAGED)).hexdigest(),
        "files": working_files(),
        "host_git": host_version,
        "host_repository_formats": formats,
        "expected_git_version": expected_version,
        "helper_sha256": helper_hashes,
        "candidate_helper_sha256": candidate_hashes,
        "relative_pointer": relative_pointer,
        "legacy_pointer": legacy_pointer,
        "host_root": str(root),
        "host_index_sha256": {
            name: digest_file(
                (path / ".git/index")
                if name in {"primary", "standalone"}
                else primary / ".git/worktrees/relative/index"
                if name == "relative"
                else legacy_primary / ".git/worktrees/legacy/index"
            )
            for name, path in (
                ("primary", primary),
                ("relative", relative),
                ("legacy", legacy),
                ("standalone", standalone),
            )
        },
    }
    frozen = fingerprint(root)
    expected_file = private / "git-worktree-expected.json"
    expected_file.write_text(json.dumps(expected, sort_keys=True))
    expected_file.chmod(0o644)
    return Fixtures(root, expected_file, package, frozen, expected)


def mounts(fixtures: Fixtures, candidate: Path) -> list[str]:
    sources = (
        (fixtures.root / "primary", TARGET / CASES["primary"]),
        (fixtures.root / "relative", TARGET / CASES["relative-leaf"]),
        (fixtures.root / "relative", TARGET / CASES["relative-topology"]),
        (fixtures.root / "primary/.git", TARGET / "topology/primary/.git"),
        (fixtures.root / "legacy", TARGET / CASES["legacy"]),
        (fixtures.root / "standalone", TARGET / CASES["standalone"]),
        (fixtures.package, PACKAGE_TARGET / "boxferry_ci"),
        (fixtures.expected_file, PACKAGE_TARGET / "expected.json"),
        (candidate, CANDIDATE),
    )
    return [
        value
        for origin, target in sources
        for value in ("--mount", f"type=bind,source={origin},target={target},readonly")
    ]


def _configuration(root: Path) -> dict[str, str]:
    return {
        "PATH": "/usr/local/bin:/usr/bin:/bin",
        "HOME": "/tmp",
        "LC_ALL": "C.UTF-8",
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": os.devnull,
        "GIT_CONFIG_SYSTEM": os.devnull,
        "GIT_OPTIONAL_LOCKS": "0",
        "GIT_NO_REPLACE_OBJECTS": "1",
        "GIT_NO_LAZY_FETCH": "1",
        "GIT_CONFIG_COUNT": "1",
        "GIT_CONFIG_KEY_0": "safe.directory",
        "GIT_CONFIG_VALUE_0": str(root),
    }


def _case(root: Path, expected: Mapping[str, object]) -> dict[str, object]:
    original = dict(os.environ)
    os.environ.clear()
    os.environ.update(_configuration(root))
    try:
        actual_git = subprocess.run(
            [
                "git",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "core.hooksPath=/dev/null",
                "-C",
                str(root),
                "rev-parse",
                "--verify",
                "HEAD",
            ],
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )
        try:
            observation = source.inspect(root)
            category, code = "success", 0
        except source.SourceError as error:
            observation = None
            category, code = error.category, 1
        cli = subprocess.run(
            [
                sys.executable,
                str(CANDIDATE / "scripts/git-worktree.py"),
                "inspect",
                "--source",
                str(root),
            ],
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )
        if actual_git.returncode != (128 if code else 0) or (
            not code and actual_git.stdout.strip() != expected["head"]
        ):
            raise FixtureError(
                "actual mounted Git HEAD/refusal differs from independently expected behavior"
            )
        if cli.returncode != code:
            raise FixtureError("candidate diagnostic caller differs from admitted inspection")
        if code:
            guidance = (
                "experimentalMountGitWorktreeCommonDir"
                if category == "relative-metadata-unmounted"
                else "standalone --source SOURCE --destination NEW_TASK_CLONE"
            )
            if f"[{category}]" not in cli.stderr or guidance not in cli.stderr:
                raise FixtureError(
                    "generic failure cannot substitute for actionable metadata refusal"
                )
        else:
            if len(cli.stdout.encode()) > MAX_RECEIPT:
                raise FixtureError("candidate inspection exceeds its byte bound")
            shipped = strict_json(cli.stdout)
            assert observation is not None
            if shipped != observation:
                raise FixtureError("candidate caller changed the exact source/index inspection")
            if (
                observation["head"] != expected["head"]
                or observation["index_sha256"] != expected["index_sha256"]
                or observation["files"] != expected["files"]
            ):
                raise FixtureError(
                    "mounted source/index differs from independent expected bytes/modes"
                )
        return {
            "exit": code,
            "category": category,
            "inspection": observation,
            "candidate_cli_exit": cli.returncode,
            "git_exit": actual_git.returncode,
        }
    finally:
        os.environ.clear()
        os.environ.update(original)


def probe(expected: Mapping[str, object]) -> dict[str, object]:
    if expected.get("spec_sha256") != SPEC_SHA256:
        raise FixtureError("fixture specification differs from admitted source")
    package = Path(__file__).resolve().parent
    helpers = {
        name: digest_file(package / name)
        for name in ("__init__.py", "git_worktree.py", "git_worktree_validation.py")
    }
    candidate = {
        name: digest_file(CANDIDATE / name)
        for name in ("scripts/git-worktree.py", "automation/src/boxferry_ci/git_worktree.py")
    }
    if helpers != expected["helper_sha256"] or candidate != expected["candidate_helper_sha256"]:
        raise FixtureError("mounted helper bytes differ from held producer/candidate")
    if os.getuid() == 0:
        raise FixtureError("fixture probe requires the configured non-root user")
    cases = {name: _case(TARGET / location, expected) for name, location in CASES.items()}
    system_git = subprocess.check_output(
        ["/usr/bin/git", "--version"], text=True, timeout=10
    ).strip()
    return {
        "schema": 1,
        "spec_sha256": SPEC_SHA256,
        "architecture": platform.machine(),
        "uid": os.getuid(),
        "gid": os.getgid(),
        "helper_sha256": helpers,
        "candidate_helper_sha256": candidate,
        "cases": cases,
        "system_git": system_git,
    }


def verify_receipt(
    raw: str, fixtures: Fixtures, machine: str, uid: int, gid: int
) -> dict[str, object]:
    if len(raw.encode()) > MAX_RECEIPT:
        raise FixtureError("native Git receipt exceeds its byte bound")
    receipt = strict_json(raw)
    if (
        not isinstance(receipt, dict)
        or set(receipt)
        != {
            "schema",
            "spec_sha256",
            "architecture",
            "uid",
            "gid",
            "helper_sha256",
            "candidate_helper_sha256",
            "cases",
            "system_git",
        }
        or type(receipt.get("schema")) is not int
        or type(receipt.get("uid")) is not int
        or type(receipt.get("gid")) is not int
        or not isinstance(receipt.get("system_git"), str)
        or re.fullmatch(r"git version [0-9]+\.[0-9]+\.[0-9]+", receipt.get("system_git", ""))
        is None
        or receipt.get("schema") != 1
        or receipt.get("spec_sha256") != SPEC_SHA256
        or receipt.get("architecture") != machine
        or receipt.get("uid") != uid
        or receipt.get("gid") != gid
        or receipt.get("helper_sha256") != fixtures.expected["helper_sha256"]
        or receipt.get("candidate_helper_sha256") != fixtures.expected["candidate_helper_sha256"]
    ):
        raise FixtureError("native Git receipt has wrong source/architecture/user/specification")
    cases = receipt.get("cases")
    if not isinstance(cases, dict) or set(cases) != set(CASES):
        raise FixtureError("native Git receipt has missing or extra cases")
    for name, observation in cases.items():
        refusal = REFUSALS.get(name)
        if (
            not isinstance(observation, dict)
            or set(observation)
            != {"exit", "category", "inspection", "candidate_cli_exit", "git_exit"}
            or type(observation.get("git_exit")) is not int
            or observation.get("git_exit") != (128 if refusal else 0)
            or type(observation.get("exit")) is not int
            or type(observation.get("candidate_cli_exit")) is not int
            or observation.get("exit") != (1 if refusal else 0)
            or observation.get("candidate_cli_exit") != (1 if refusal else 0)
            or observation.get("category") != (refusal or "success")
        ):
            raise FixtureError("native Git case has wrong success or refusal identity")
        facts = observation.get("inspection")
        if refusal:
            if facts is not None:
                raise FixtureError("refused metadata must not produce a successful inspection")
        elif (
            not isinstance(facts, dict)
            or set(facts)
            != {
                "schema",
                "source",
                "git_dir",
                "index",
                "git_version",
                "head",
                "index_sha256",
                "raw_index_sha256",
                "repository_format",
                "extensions",
                "files_sha256",
                "bytes",
                "files",
            }
            or type(facts.get("schema")) is not int
            or facts.get("schema") != 1
            or type(facts.get("repository_format")) is not int
            or type(facts.get("bytes")) is not int
            or facts.get("bytes") != sum(len(data) for _, data in WORKING.values())
            or facts.get("files_sha256")
            != hashlib.sha256(
                json.dumps(fixtures.expected["files"], sort_keys=True).encode()
            ).hexdigest()
            or facts.get("head") != fixtures.expected["head"]
            or facts.get("index_sha256") != fixtures.expected["index_sha256"]
            or facts.get("files") != fixtures.expected["files"]
            or facts.get("source") != str(TARGET / CASES[name])
            or not isinstance(facts.get("git_version"), str)
            or re.fullmatch(r"git version [0-9]+\.[0-9]+\.[0-9]+", facts.get("git_version", ""))
            is None
        ):
            raise FixtureError(
                "native Git positive case changed source, HEAD, index, bytes or modes"
            )
    expected_version = fixtures.expected["expected_git_version"]
    versions = {
        facts["inspection"]["git_version"] for name, facts in cases.items() if name not in REFUSALS
    }
    if len(versions) != 1 or (
        expected_version is not None
        and (
            versions != {"git version " + str(expected_version)}
            or receipt["system_git"] != "git version " + str(expected_version)
        )
    ):
        raise FixtureError("native Git executables differ from the operational canonical version")
    for name, facts in cases.items():
        if name in REFUSALS:
            continue
        observed = facts["inspection"]
        original = "relative" if name == "relative-topology" else name
        formats = fixtures.expected["host_repository_formats"]
        indexes = fixtures.expected["host_index_sha256"]
        assert isinstance(formats, dict) and isinstance(indexes, dict)
        directory = (
            TARGET / "topology/primary/.git/worktrees/relative"
            if original == "relative"
            else TARGET / CASES[name] / ".git"
        )
        if (
            observed.get("git_dir") != str(directory)
            or observed.get("index") != str(directory / "index")
            or observed.get("repository_format") != formats[original]
            or observed.get("raw_index_sha256") != indexes[original]
            or observed.get("extensions")
            != ({"relativeworktrees": "true"} if original in {"primary", "relative"} else {})
        ):
            raise FixtureError("native Git repository format/extensions or exact raw index changed")
    if fingerprint(fixtures.root) != fixtures.frozen:
        raise FixtureError("native image probe changed host fixture metadata, index or source")
    return {
        "status": "success",
        "spec_sha256": SPEC_SHA256,
        "host": fixtures.expected,
        "runtime": receipt,
        "host_metadata_unchanged": True,
    }


def main() -> int:
    try:
        path = PACKAGE_TARGET / "expected.json"
        if path.stat().st_size > MAX_RECEIPT:
            raise FixtureError("fixture expectations exceed their bound")
        expected = strict_json(path.read_text())
        if not isinstance(expected, dict):
            raise FixtureError("fixture expectations require an object")
        result = probe(expected)
        encoded = json.dumps(result, sort_keys=True)
        if len(encoded.encode()) > MAX_RECEIPT:
            raise FixtureError("fixture receipt exceeds its bound")
        print(encoded)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"BoxFerry native Git contract failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
