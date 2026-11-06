"""Independent expected decisions; scratch Git objects and labelled metadata mocks.

The snapshots are inert source-owned data. Generated Git IDs are substituted only
in tests; production has no registry/API override or candidate-provided proof path.
"""

from __future__ import annotations

import argparse
import ast
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from collections.abc import Iterator
from dataclasses import replace
from email.message import Message
from pathlib import Path
from types import SimpleNamespace
from typing import Any, ClassVar, cast
from unittest.mock import patch
from urllib.error import HTTPError

from test_source_slot_parity import legacy_transform

from boxferry_ci.admission import AdmissionContext, AdmissionSelection
from boxferry_ci.zero_age_delta import (
    ADDITIONAL_SOURCE_SLOTS,
    AUTHORITY,
    CALLER,
    CONTRACT,
    FIXTURES,
    IMPLEMENTATION_PATHS,
    MANIFEST,
    POLICY,
    PRODUCER,
    PROOF_PATH,
    REGISTRY,
    REQUIRED_604,
    SOURCE_PATHS,
    WAITER,
    Entry,
    Git,
    Metadata,
    ProofCancelled,
    ProofError,
    Repair,
    Tree,
    _blocked_cancellation,
    acquire_consumer,
    bind_context,
    canonical,
    changed_paths,
    confirm_exact_A,
    expected_waiter,
    match_repair_shape,
    prove_zero,
    public_custody,
    repin,
    verify_producer,
)

ROOT = Path(__file__).resolve().parents[2]
FIXTURE_ROOT = ROOT / FIXTURES
S = "b" * 40
H = "c" * 40
N = "d" * 40


def entry(data: bytes, mode: int = 0o100644) -> Entry:
    import hashlib

    oid = hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()
    return Entry(mode, oid, data)


def fixture_producer() -> Tree:
    return Tree(
        "e" * 40,
        {
            p.relative_to(ROOT).as_posix(): entry(p.read_bytes())
            for p in FIXTURE_ROOT.rglob("*.txt")
        },
    )


def git(root: Path, *arguments: str) -> str:
    result = subprocess.run(
        [
            "git",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-C",
            str(root),
            *arguments,
        ],
        capture_output=True,
        check=True,
        timeout=10,
        env={k: v for k, v in os.environ.items() if not k.startswith("GIT_")},
    )
    return result.stdout.decode().strip()


def read_tree(root: Path, sha: str) -> Tree:
    reader = Git(root, time.monotonic() + 10)
    try:
        return reader.tree(sha)
    finally:
        reader.close()


def files(name: str, *, after: bool, source: str = AUTHORITY) -> dict[str, tuple[bytes, int]]:
    repair = REGISTRY[f"Strukturpiloten/{name}-lens"]
    directory = FIXTURE_ROOT / name
    data = {
        CALLER: ((directory / f"caller-{'after' if after else 'before'}.txt").read_bytes(), 0o644),
        CONTRACT: (
            (directory / f"contract-{'after' if after else 'before'}.txt").read_bytes(),
            repair.test_mode & 0o777,
        ),
        MANIFEST: ((directory / "manifest.txt").read_bytes(), 0o644),
        POLICY: ((directory / "policy.txt").read_bytes(), 0o644),
        ".github/renovate.json": (
            json.dumps(
                {
                    "extends": [
                        f"github>Strukturpiloten/boxferry//renovate-presets/{p}#{AUTHORITY}"
                        for p in ("common", "rust")
                    ]
                }
            ).encode(),
            0o644,
        ),
        "Cargo.lock": (b"# unchanged complete synthetic dependency input\n", 0o644),
        "tools/download.sh": (b"# unchanged tool and integrity evidence\n", 0o644),
    }
    workflow = (
        "---\njobs:\n  selected:\n"
        f"    uses: {PRODUCER}/.github/workflows/shared-rust-ci.yml@{AUTHORITY}\n"
        "    with:\n" + f"      automation_sha: {AUTHORITY}\n"
    ).encode()
    for path in SOURCE_PATHS:
        data.setdefault(path, (workflow, 0o644))
    if source != AUTHORITY:
        for path in SOURCE_PATHS:
            data[path] = repin(data[path][0], path, source), data[path][1]
    return data


def write_files(root: Path, data: dict[str, tuple[bytes, int]]) -> None:
    for path, (content, mode) in data.items():
        target = root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content)
        target.chmod(mode)


class RepairTests(unittest.TestCase):
    def test_real_reviewed_repairs_and_coherent_repins_from_real_git(self) -> None:
        for name in ("podman", "quadlet"):
            for source in (AUTHORITY, S):
                with (
                    self.subTest(name=name, source=source),
                    tempfile.TemporaryDirectory() as temporary,
                ):
                    root = Path(temporary)
                    git(root, "init", "-q")
                    write_files(root, files(name, after=False))
                    git(root, "add", ".")
                    git(root, "commit", "-qm", "known-base")
                    before_sha = git(root, "rev-parse", "HEAD")
                    write_files(root, files(name, after=True, source=source))
                    git(root, "add", ".")
                    git(root, "commit", "-qm", "reviewed-repair")
                    after_sha = git(root, "rev-parse", "HEAD")
                    before, after = read_tree(root, before_sha), read_tree(root, after_sha)
                    report = match_repair_shape(
                        before,
                        after,
                        fixture_producer(),
                        REGISTRY[f"Strukturpiloten/{name}-lens"],
                        S,
                    )
                    self.assertEqual(report["source_mode"], "A" if source == AUTHORITY else "S")
                    self.assertEqual(
                        report["consumer_before_tree"],
                        git(root, "rev-parse", before_sha + "^{tree}"),
                    )
                    self.assertEqual(
                        report["consumer_after_tree"], git(root, "rev-parse", after_sha + "^{tree}")
                    )
                    self.assertEqual(changed_paths(before, after), set(report["changed_paths"]))

    def snapshots(self, name: str = "podman") -> tuple[Tree, Tree]:
        before, after = tuple(
            Tree(
                "a" * 40,
                {
                    p: entry(content, mode | 0o100000)
                    for p, (content, mode) in files(name, after=after).items()
                },
            )
            for after in (False, True)
        )
        return before, after

    def test_every_extra_dependency_tool_action_feature_path_and_mode_refuses(self) -> None:
        repair = REGISTRY["Strukturpiloten/podman-lens"]
        before, after = self.snapshots()
        counterexamples = {
            "Cargo.lock": b"transitive source checksum changed",
            "package-lock.json": b"new npm transitive integrity",
            "uv.lock": b"new Python dependency",
            ".devcontainer/devcontainer-lock.json": b"changed feature graph",
            "tools/download.sh": b"new standalone tool checksum",
            ".github/workflows/other.yml": b"third-party action or runner change",
            "unknown.input": b"unknown coverage input",
        }
        for path, content in counterexamples.items():
            with self.subTest(path=path):
                changed = replace(after, entries={**after.entries, path: entry(content)})
                with self.assertRaisesRegex(ProofError, "consumer-extra-tree-delta"):
                    match_repair_shape(before, changed, fixture_producer(), repair, S)
        for path in (CALLER, CONTRACT, "Cargo.lock"):
            with self.subTest(mode=path):
                changed = replace(
                    after,
                    entries={**after.entries, path: replace(after.entries[path], mode=0o100755)},
                )
                with self.assertRaises(ProofError):
                    match_repair_shape(before, changed, fixture_producer(), repair, S)
        for path in ("Cargo.lock", CALLER):
            changed = replace(after, entries={p: e for p, e in after.entries.items() if p != path})
            with self.assertRaises(ProofError):
                match_repair_shape(before, changed, fixture_producer(), repair, S)

    def test_policy_manifest_mixed_or_fake_source_slots_never_pass(self) -> None:
        repair = REGISTRY["Strukturpiloten/podman-lens"]
        before, after = self.snapshots()
        for path, content in (
            (POLICY, after.entries[POLICY].content + b" "),
            (MANIFEST, after.entries[MANIFEST].content + b" "),
            (CALLER, after.entries[CALLER].content + b"# permissions: {}\n"),
            (CALLER, after.entries[CALLER].content.replace(AUTHORITY.encode(), H.encode())),
            (
                ".github/workflows/ci.yml",
                repin(
                    after.entries[".github/workflows/ci.yml"].content, ".github/workflows/ci.yml", S
                ),
            ),
        ):
            with self.subTest(path=path), self.assertRaises(ProofError):
                match_repair_shape(
                    before,
                    replace(after, entries={**after.entries, path: entry(content)}),
                    fixture_producer(),
                    repair,
                    S,
                )

    def test_wrong_anchors_and_templates_refuse(self) -> None:
        before, after = self.snapshots()
        repair = REGISTRY["Strukturpiloten/podman-lens"]
        for path in (CALLER, CONTRACT, POLICY, MANIFEST):
            with self.subTest(path=path), self.assertRaises(ProofError):
                match_repair_shape(
                    replace(before, entries={**before.entries, path: entry(b"unknown")}),
                    after,
                    fixture_producer(),
                    repair,
                    S,
                )
        producer = fixture_producer()
        path = FIXTURES + "podman/caller-after.txt"
        with self.assertRaisesRegex(ProofError, "reviewed-template-hash"):
            match_repair_shape(
                before,
                after,
                replace(producer, entries={**producer.entries, path: entry(b"fake")}),
                repair,
                S,
            )

    def test_additional_local_action_slots_require_exact_whole_file_anchor(self) -> None:
        # This standalone609 contract uses its original A-exact parser.610 is outside
        # that closed producer; the integration refusal is checked independently below.
        with patch("boxferry_ci.eligibility.expected_ci_upgrade", legacy_transform()):
            self.additional_local_action_slots_require_exact_whole_file_anchor()

    def additional_local_action_slots_require_exact_whole_file_anchor(self) -> None:
        data = (
            "---\njobs:\n  selected:\n"
            f"    uses: {PRODUCER}/.github/workflows/shared-rust-ci.yml@{AUTHORITY}\n"
            "    steps:\n      - uses: ./_automation/.github/actions/shared-node\n"
            "        with:\n" + f"          automation-sha: {AUTHORITY}\n"
        ).encode()
        import hashlib

        path = ".github/workflows/ci.yml"
        key = path, hashlib.sha256(data).hexdigest()
        with self.assertRaisesRegex(ProofError, "unrecognized-local-action-source-slot"):
            repin(data, path, S)
        with patch.dict(ADDITIONAL_SOURCE_SLOTS, {key: (7,)}):
            result = repin(data, path, S)
            self.assertNotIn(AUTHORITY.encode(), result)
            self.assertEqual(result.count(S.encode()), 2)
            with self.assertRaises(ProofError):
                repin(data + b"# changed anchor\n", path, S)
        for changed in (
            data.replace(AUTHORITY.encode(), H.encode(), 1),
            data + ("          automation-sha: " + AUTHORITY + "\n").encode(),
        ):
            with self.assertRaises(ProofError):
                repin(changed, path, S)
        # An unowned lookalike in a block scalar cannot qualify a new slot.
        fake = data + b"env:\n  TEXT: |\n    automation-sha: " + AUTHORITY.encode() + b"\n"
        with self.assertRaises(ProofError):
            repin(fake, path, S)

    def test_historical_and_block_scalar_source_strings_are_preserved(self) -> None:
        data = files("podman", after=False)[".github/workflows/ci.yml"][0]
        history = ("env:\n  HISTORY: |\n    old reviewed source " + AUTHORITY + "\n").encode()
        result = repin(data + history, ".github/workflows/ci.yml", S)
        self.assertTrue(result.endswith(history))
        self.assertIn(S.encode(), result)


class GitReaderTests(unittest.TestCase):
    def test_complete_tree_accounts_empty_directories_links_and_gitlinks(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            git(root, "init", "-q")
            write_files(root, {"safe/file": (b"payload", 0o644)})
            (root / "link").symlink_to("safe/file")
            git(root, "add", ".")
            git(root, "commit", "-qm", "tree")
            head = git(root, "rev-parse", "HEAD")
            tree = read_tree(root, head)
            self.assertEqual(set(tree.entries), {"safe", "safe/file", "link"})
            self.assertEqual(tree.entries["link"].mode, 0o120000)
            self.assertEqual(tree.entries["link"].content, b"safe/file")
            git(root, "update-index", "--add", "--cacheinfo", f"160000,{head},module")
            git(root, "commit", "-qm", "gitlink")
            self.assertEqual(
                read_tree(root, git(root, "rev-parse", "HEAD")).entries["module"].mode, 0o160000
            )

    def test_object_size_and_deadline_are_hard_bounds(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            git(root, "init", "-q")
            write_files(root, {"large": (b"x" * (2 * 1024 * 1024 + 1), 0o644)})
            git(root, "add", ".")
            git(root, "commit", "-qm", "oversized")
            with self.assertRaisesRegex(ProofError, "git-object-size-budget"):
                read_tree(root, git(root, "rev-parse", "HEAD"))
            reader = Git(root, time.monotonic() - 1)
            try:
                with self.assertRaisesRegex(ProofError, "proof-time-budget"):
                    reader.object(git(root, "rev-parse", "HEAD"), "commit")
            finally:
                reader.close()

    def test_hash_type_and_truncated_or_malformed_tree_reject(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            git(root, "init", "-q")
            write_files(root, {"safe": (b"data", 0o644)})
            git(root, "add", ".")
            git(root, "commit", "-qm", "base")
            reader = Git(root, time.monotonic() + 5)
            try:
                head = git(root, "rev-parse", "HEAD")
                with self.assertRaisesRegex(ProofError, "git-object-kind"):
                    reader.object(head, "blob")
                with self.assertRaises(ProofError):
                    reader.object("0" * 40, "commit")
            finally:
                reader.close()

    def test_malformed_duplicate_and_unsafe_real_git_tree_objects_refuse(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            git(root, "init", "-q")
            write_files(root, {"safe": (b"payload", 0o644)})
            git(root, "add", ".")
            git(root, "commit", "-qm", "safe")
            blob = bytes.fromhex(git(root, "rev-parse", "HEAD:safe"))

            def store(kind: str, data: bytes) -> str:
                result = subprocess.run(
                    [
                        "git",
                        "-C",
                        str(root),
                        "hash-object",
                        "-w",
                        "--literally",
                        "-t",
                        kind,
                        "--stdin",
                    ],
                    input=data,
                    capture_output=True,
                    check=True,
                    timeout=5,
                )
                return result.stdout.decode().strip()

            for payload in (
                b"100644 ../escape\0" + blob,
                b"100644 .git\0" + blob,
                b"100644 unsafe/name\0" + blob,
                b"100644 newline\npath\0" + blob,
                b"100644 same\0" + blob + b"100644 same\0" + blob,
                b"100664 wrong-mode\0" + blob,
                b"100644 truncated\0" + blob[:-1],
            ):
                tree = store("tree", payload)
                commit = store("commit", f"tree {tree}\n\nmalformed fixture\n".encode())
                with self.subTest(payload=payload[:24]), self.assertRaises(ProofError) as caught:
                    read_tree(root, commit)
                self.assertTrue(caught.exception.invalid)


class ObjectAcquisitionTests(unittest.TestCase):
    def test_standard_fetch_never_materializes_candidate_files_or_symlinks(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            origin, destination, home = root / "origin", root / "destination", root / "home"
            origin.mkdir()
            destination.mkdir()
            home.mkdir()
            git(origin, "init", "-q")
            write_files(
                origin, {"scripts/inert.py": (b"raise RuntimeError('never execute')\n", 0o755)}
            )
            (origin / "candidate-link").symlink_to("/must-never-materialize")
            git(origin, "add", ".")
            git(origin, "commit", "-qm", "candidate-data")
            tested = git(origin, "rev-parse", "HEAD")
            original = subprocess.Popen
            commands: list[tuple[list[str], dict[str, Any]]] = []

            def transport(command: list[str], **kwargs: object) -> subprocess.Popen[bytes]:
                # Explicit test-only transport substitution: production command
                # and environment are captured before local fixture transport.
                if "fetch" in command:
                    commands.append((command, dict(cast("dict[str, str]", kwargs["env"]))))
                    expected = "https://github.com/Strukturpiloten/podman-lens.git"
                    self.assertIn(expected, command)
                    actual = [str(origin) if part == expected else part for part in command]
                    command = [actual[0], "-c", "protocol.file.allow=always", *actual[1:]]
                return cast("subprocess.Popen[bytes]", original(command, **cast("Any", kwargs)))

            with (
                patch.dict(
                    os.environ,
                    {
                        "GH_TOKEN": "secret",
                        "GITHUB_TOKEN": "secret",
                        "GIT_ASKPASS": "/evil",
                        "HTTPS_PROXY": "http://evil",
                    },
                ),
                patch("pathlib.Path.home", return_value=home),
                patch("pwd.getpwuid", return_value=SimpleNamespace(pw_dir=str(home))),
                patch("subprocess.Popen", side_effect=transport),
            ):
                bare = acquire_consumer(
                    destination,
                    REGISTRY["Strukturpiloten/podman-lens"],
                    tested,
                    time.monotonic() + 10,
                )
            tree = read_tree(bare, tested)
            self.assertEqual(tree.entries["candidate-link"].mode, 0o120000)
            self.assertEqual(
                tree.entries["scripts/inert.py"].content, b"raise RuntimeError('never execute')\n"
            )
            self.assertFalse((bare / "candidate-link").exists())
            self.assertFalse((bare / "candidate-link").is_symlink())
            self.assertFalse((bare / "scripts").exists())
            self.assertEqual(len(commands), 1)
            command, environment = commands[0]
            self.assertIn("--depth=1", command)
            self.assertNotIn("checkout", command)
            self.assertNotIn("--filter", " ".join(command))
            for key in ("GH_TOKEN", "GITHUB_TOKEN", "HTTPS_PROXY"):
                self.assertNotIn(key, environment)
            self.assertEqual(environment["GIT_ASKPASS"], "/bin/false")
            self.assertIn("http.extraHeader=Authorization:", command)
            self.assertIn("credential.helper=", command)

    def test_ambient_netrc_file_or_symlink_refuses_before_any_process(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            for name in (".netrc", "_netrc"):
                path = home / name
                path.symlink_to("/unread-secret")
                with (
                    patch("pathlib.Path.home", return_value=home),
                    patch("subprocess.Popen", side_effect=AssertionError("no process permitted")),
                    self.assertRaisesRegex(ProofError, "ambient-netrc"),
                ):
                    acquire_consumer(
                        home, REGISTRY["Strukturpiloten/podman-lens"], N, time.monotonic() + 5
                    )
                path.unlink()

    def test_spawn_pending_signal_and_selector_setup_failures_reap_owned_child(self) -> None:
        original = subprocess.Popen
        for mode in (
            "selector-create",
            "selector-register",
            "pending-alarm",
            "pending-term",
            "pending-int",
        ):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as temporary:
                processes: list[subprocess.Popen[bytes]] = []

                def transport(
                    command: list[str],
                    *,
                    mode: str = mode,
                    processes: list[subprocess.Popen[bytes]] = processes,
                    **kwargs: object,
                ) -> subprocess.Popen[bytes]:
                    if "fetch" in command:
                        process = cast(
                            "subprocess.Popen[bytes]",
                            original(
                                [sys.executable, "-I", "-c", "import time; time.sleep(60)"],
                                **cast("Any", kwargs),
                            ),
                        )
                        processes.append(process)
                        if mode.startswith("pending-"):
                            signal.raise_signal(
                                signal.SIGALRM
                                if mode == "pending-alarm"
                                else (signal.SIGINT if mode == "pending-int" else signal.SIGTERM)
                            )
                        return process
                    return cast("subprocess.Popen[bytes]", original(command, **cast("Any", kwargs)))

                def alarm(signum: int, frame: object) -> None:
                    raise ProofError("test-pending-alarm")

                def terminate(signum: int, frame: object) -> None:
                    raise ProofCancelled(signum)

                previous_alarm = signal.signal(signal.SIGALRM, alarm)
                previous_term = signal.signal(signal.SIGTERM, terminate)
                previous_int = signal.signal(signal.SIGINT, signal.default_int_handler)
                selector = __import__("selectors").DefaultSelector()
                try:
                    with (
                        patch("subprocess.Popen", side_effect=transport),
                        patch("pathlib.Path.home", return_value=Path(temporary)),
                        patch("pwd.getpwuid", return_value=SimpleNamespace(pw_dir=temporary)),
                    ):
                        expected = (
                            ProofCancelled
                            if mode == "pending-term"
                            else (
                                KeyboardInterrupt
                                if mode == "pending-int"
                                else (ProofError if mode == "pending-alarm" else RuntimeError)
                            )
                        )
                        if mode == "selector-create":
                            with (
                                patch(
                                    "boxferry_ci.zero_age_delta.selectors.DefaultSelector",
                                    side_effect=RuntimeError("selector-create"),
                                ),
                                self.assertRaises(expected),
                            ):
                                acquire_consumer(
                                    Path(temporary),
                                    REGISTRY["Strukturpiloten/podman-lens"],
                                    N,
                                    time.monotonic() + 5,
                                )
                        else:
                            with (
                                patch(
                                    "boxferry_ci.zero_age_delta.selectors.DefaultSelector",
                                    return_value=selector,
                                ),
                                patch.object(
                                    selector,
                                    "register",
                                    side_effect=RuntimeError("selector-register")
                                    if mode == "selector-register"
                                    else selector.register,
                                ),
                                self.assertRaises(expected),
                            ):
                                acquire_consumer(
                                    Path(temporary),
                                    REGISTRY["Strukturpiloten/podman-lens"],
                                    N,
                                    time.monotonic() + 5,
                                )
                    self.assertEqual(len(processes), 1)
                    self.assertIsNotNone(processes[0].poll())
                    self.assertTrue(processes[0].stdout is not None and processes[0].stdout.closed)
                    with self.assertRaises(ProcessLookupError):
                        os.killpg(processes[0].pid, 0)
                finally:
                    signal.signal(signal.SIGALRM, previous_alarm)
                    signal.signal(signal.SIGTERM, previous_term)
                    signal.signal(signal.SIGINT, previous_int)
                    selector.close()
                    for process in processes:
                        if process.poll() is None:
                            os.killpg(process.pid, signal.SIGKILL)
                            process.wait(timeout=3)

    def test_fast_exit_terminal_output_files_count_and_links_are_always_checked(self) -> None:
        original = subprocess.Popen
        reasons = {
            "clean": None,
            "output": "candidate-object-acquisition-output-budget",
            "stderr-overflow": "candidate-object-acquisition-stderr-budget",
            "http-552": "candidate-object-acquisition-unavailable",
            "single-file": "candidate-object-acquisition-disk-budget",
            "aggregate": "candidate-object-acquisition-disk-budget",
            "file-count": "candidate-object-acquisition-disk-budget",
            "file-link": "candidate-object-db-link",
            "directory-link": "candidate-object-db-link",
        }
        for phase in ("immediate-exit", "last-burst"):
            for mode, reason in reasons.items():
                with (
                    self.subTest(phase=phase, mode=mode),
                    tempfile.TemporaryDirectory() as temporary,
                ):
                    descriptor, writer = os.pipe()
                    error_descriptor, error_writer = os.pipe()
                    stream = os.fdopen(descriptor, "rb")
                    error_stream = os.fdopen(error_descriptor, "rb")
                    state = {"polls": 0, "finished": False}

                    def poll(
                        mode: str = mode,
                        phase: str = phase,
                        state: dict[str, int | bool] = state,
                        writer: int = writer,
                        error_writer: int = error_writer,
                        temporary: str = temporary,
                    ) -> int | None:
                        state["polls"] += 1
                        if phase == "last-burst" and state["polls"] == 1:
                            return None
                        if not state["finished"]:
                            bare = Path(temporary) / "objects.git"
                            if mode in {"single-file", "aggregate"}:
                                for index in range(1 if mode == "single-file" else 3):
                                    with (bare / f"last-{index}").open("wb") as target:
                                        target.truncate(
                                            32 * 1024 * 1024 + 1
                                            if mode == "single-file"
                                            else 24 * 1024 * 1024
                                        )
                            elif mode == "file-count":
                                for index in range(8193):
                                    (bare / f"last-{index}").touch()
                            elif mode.endswith("link"):
                                (bare / "last-link").symlink_to(
                                    "/untrusted-directory"
                                    if mode == "directory-link"
                                    else "/untrusted-file",
                                    target_is_directory=mode == "directory-link",
                                )
                            elif mode == "output":
                                os.write(writer, b"x" * 4097)
                            elif mode in {"stderr-overflow", "http-552"}:
                                private = (
                                    b"error: RPC failed; HTTP 552 curl 22; "
                                    b"https://SECRET-SENTINEL@github.com/private "
                                    b"Authorization: Bearer SECRET-SENTINEL /home/private-secret"
                                )
                                os.write(
                                    error_writer,
                                    private if mode == "http-552" else private + b"x" * 4097,
                                )
                            os.close(writer)
                            os.close(error_writer)
                            state["finished"] = True
                        return 128 if mode == "http-552" else 0

                    fake = SimpleNamespace(
                        pid=987654321,
                        stdout=stream,
                        stderr=error_stream,
                        returncode=128 if mode == "http-552" else 0,
                        poll=poll,
                        wait=lambda timeout, mode=mode: 128 if mode == "http-552" else 0,
                    )

                    def transport(
                        command: list[str], *, fake: SimpleNamespace = fake, **kwargs: object
                    ) -> object:
                        if "fetch" not in command:
                            return original(command, **cast("Any", kwargs))
                        return fake

                    try:
                        with (
                            patch("subprocess.Popen", side_effect=transport),
                            patch("pathlib.Path.home", return_value=Path(temporary)),
                            patch("pwd.getpwuid", return_value=SimpleNamespace(pw_dir=temporary)),
                            patch("os.killpg", side_effect=ProcessLookupError),
                        ):
                            if reason is None:
                                bare = acquire_consumer(
                                    Path(temporary),
                                    REGISTRY["Strukturpiloten/podman-lens"],
                                    N,
                                    time.monotonic() + 10,
                                )
                                self.assertEqual(bare, Path(temporary) / "objects.git")
                            else:
                                with self.assertRaises(ProofError) as caught:
                                    acquire_consumer(
                                        Path(temporary),
                                        REGISTRY["Strukturpiloten/podman-lens"],
                                        N,
                                        time.monotonic() + 10,
                                    )
                                self.assertEqual(caught.exception.reason, reason)
                                if mode in {"http-552", "stderr-overflow"}:
                                    details = caught.exception.details
                                    serialized = json.dumps(details)
                                    self.assertIn(552, cast("list[int]", details["http_statuses"]))
                                    self.assertEqual(
                                        details["stderr_truncated"], mode == "stderr-overflow"
                                    )
                                    if mode == "http-552":
                                        self.assertEqual(details["git_exit_code"], 128)
                                    self.assertNotIn("SECRET-SENTINEL", serialized)
                                    self.assertNotIn("/home/private-secret", serialized)
                                    self.assertNotIn("Authorization", serialized)
                        self.assertTrue(stream.closed)
                        self.assertTrue(error_stream.closed)
                        self.assertTrue(state["finished"])
                        self.assertEqual(state["polls"], 1 if phase == "immediate-exit" else 2)
                    finally:
                        stream.close()
                        error_stream.close()
                        if not state["finished"]:
                            os.close(writer)
                            os.close(error_writer)

    def test_main_int_and_term_reap_live_fetch_session_and_return_cancellation(self) -> None:
        driver = r"""
import json, subprocess, sys, time
from pathlib import Path
from unittest.mock import patch
from types import SimpleNamespace
sys.path.insert(0, sys.argv[1])
from boxferry_ci import zero_age_delta as Z
root, marker = Path(sys.argv[2]), Path(sys.argv[3])
original = subprocess.Popen
child_code = (
    "import json,os,subprocess,sys,time; from pathlib import Path; "
    "child=subprocess.Popen([sys.executable,'-I','-c','import time; time.sleep(60)']); "
    "Path(sys.argv[1]).write_text(json.dumps([os.getpid(),child.pid])); time.sleep(60)"
)
def transport(command, **kwargs):
    if 'fetch' in command:
        return original([sys.executable, '-I', '-c', child_code, str(marker)], **kwargs)
    return original(command, **kwargs)
def proof(options):
    Z.acquire_consumer(root, Z.REGISTRY['Strukturpiloten/podman-lens'], 'd'*40, time.monotonic()+30)
    raise AssertionError('cancellation must not return proof')
args = []
for name, value in {'repository':'Strukturpiloten/podman-lens', 'event-name':'merge_group',
    'candidate-sha':'d'*40, 'tested-sha':'d'*40, 'automation-sha':'b'*40,
    'policy-sha':'a'*40, 'ref':'queue', 'implementation-root':'.', 'authority-root':'.',
    'policy-root':'.', 'event-path':'.', 'run-id':'1', 'run-attempt':'1'}.items():
    args += ['--'+name, value]
with patch('subprocess.Popen', side_effect=transport), patch.object(
    Z, 'prove_zero', side_effect=proof
), patch('pathlib.Path.home', return_value=root), patch(
    'pwd.getpwuid', return_value=SimpleNamespace(pw_dir=str(root))
):
    raise SystemExit(Z.main(args))
"""
        for cancellation_signal in (signal.SIGINT, signal.SIGTERM):
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                marker = root / "owned-pids.json"
                process = subprocess.Popen(
                    [
                        sys.executable,
                        "-I",
                        "-B",
                        "-c",
                        driver,
                        str(ROOT / "automation/src"),
                        str(root),
                        str(marker),
                    ],
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                )
                owned: list[int] = []
                try:
                    deadline = time.monotonic() + 8
                    while (
                        not marker.exists()
                        and process.poll() is None
                        and time.monotonic() < deadline
                    ):
                        time.sleep(0.01)
                    self.assertTrue(marker.exists())
                    owned = json.loads(marker.read_text())
                    process.send_signal(cancellation_signal)
                    output, errors = process.communicate(timeout=5)
                    self.assertEqual(process.returncode, 128 + cancellation_signal, errors.decode())
                    self.assertEqual(output, b"")
                    self.assertIsNotNone(process.poll())
                    deadline = time.monotonic() + 3
                    active = list(owned)
                    while active and time.monotonic() < deadline:
                        active = [
                            pid
                            for pid in owned
                            if Path(f"/proc/{pid}/stat").exists()
                            and Path(f"/proc/{pid}/stat").read_text().split()[2] != "Z"
                        ]
                        if active:
                            time.sleep(0.01)
                    self.assertEqual(active, [])
                finally:
                    for pid in owned[:1]:
                        try:
                            os.killpg(pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                    if process.poll() is None:
                        process.kill()
                    process.communicate(timeout=3)


class RemainingChildOwnershipTests(unittest.TestCase):
    def test_reader_metadata_and_confirmation_setup_and_pending_signals_reap_children(self) -> None:
        original = subprocess.Popen
        modes = (
            "selector-create",
            "selector-register",
            "pending-alarm",
            "pending-term",
            "pending-int",
        )
        for target in ("reader", "metadata", "confirmation", "bare-init"):
            for mode in modes if target != "bare-init" else modes[2:]:
                with (
                    self.subTest(target=target, mode=mode),
                    tempfile.TemporaryDirectory() as temporary,
                ):
                    root = Path(temporary)
                    git(root, "init", "-q")
                    reader = Git(root, time.monotonic() + 10) if target == "metadata" else None
                    processes: list[subprocess.Popen[bytes]] = []

                    def transport(
                        command: list[str],
                        *,
                        target: str = target,
                        mode: str = mode,
                        processes: list[subprocess.Popen[bytes]] = processes,
                        **kwargs: object,
                    ) -> subprocess.Popen[bytes]:
                        selected = (
                            (target == "reader" and "cat-file" in command)
                            or (target == "metadata" and "rev-parse" in command)
                            or (target == "confirmation" and command[0] == sys.executable)
                            or (target == "bare-init" and "init" in command)
                        )
                        if selected:
                            process = cast(
                                "subprocess.Popen[bytes]",
                                original(
                                    [sys.executable, "-I", "-c", "import time; time.sleep(60)"],
                                    **cast("Any", kwargs),
                                ),
                            )
                            processes.append(process)
                            if mode.startswith("pending-"):
                                signal.raise_signal(
                                    {
                                        "pending-alarm": signal.SIGALRM,
                                        "pending-term": signal.SIGTERM,
                                        "pending-int": signal.SIGINT,
                                    }[mode]
                                )
                            return process
                        return cast(
                            "subprocess.Popen[bytes]", original(command, **cast("Any", kwargs))
                        )

                    def alarm(signum: int, frame: object) -> None:
                        raise ProofError("test-pending-alarm")

                    def terminate(signum: int, frame: object) -> None:
                        raise ProofCancelled(signum)

                    previous = {
                        signal.SIGALRM: signal.signal(signal.SIGALRM, alarm),
                        signal.SIGTERM: signal.signal(signal.SIGTERM, terminate),
                        signal.SIGINT: signal.signal(signal.SIGINT, signal.default_int_handler),
                    }
                    selector = __import__("selectors").DefaultSelector()
                    expected = {
                        "pending-alarm": ProofError,
                        "pending-term": ProofCancelled,
                        "pending-int": KeyboardInterrupt,
                    }.get(mode, RuntimeError)

                    def invoke(
                        target: str = target, reader: Git | None = reader, root: Path = root
                    ) -> None:
                        if target == "reader":
                            Git(root, time.monotonic() + 5)
                        elif target == "metadata":
                            assert reader is not None
                            reader.small(("rev-parse", "HEAD"))
                        elif target == "confirmation":
                            empty = Tree("b" * 40, {"input": entry(b"complete inert input")})
                            confirm_exact_A(
                                ConfirmationTests().authority(), empty, empty, time.monotonic() + 5
                            )
                        else:
                            acquire_consumer(
                                root,
                                REGISTRY["Strukturpiloten/podman-lens"],
                                N,
                                time.monotonic() + 5,
                            )

                    try:
                        with (
                            patch("subprocess.Popen", side_effect=transport),
                            patch("pathlib.Path.home", return_value=root),
                            patch("pwd.getpwuid", return_value=SimpleNamespace(pw_dir=temporary)),
                        ):
                            if mode == "selector-create":
                                with (
                                    patch(
                                        "boxferry_ci.zero_age_delta.selectors.DefaultSelector",
                                        side_effect=RuntimeError("selector-create"),
                                    ),
                                    self.assertRaises(expected),
                                ):
                                    invoke()
                            else:
                                with (
                                    patch(
                                        "boxferry_ci.zero_age_delta.selectors.DefaultSelector",
                                        return_value=selector,
                                    ),
                                    patch.object(
                                        selector,
                                        "register",
                                        side_effect=RuntimeError("selector-register")
                                        if mode == "selector-register"
                                        else selector.register,
                                    ),
                                    self.assertRaises(expected),
                                ):
                                    invoke()
                        self.assertEqual(len(processes), 1)
                        self.assertIsNotNone(processes[0].poll())
                        for stream in (
                            processes[0].stdin,
                            processes[0].stdout,
                            processes[0].stderr,
                        ):
                            self.assertTrue(stream is None or stream.closed)
                        with self.assertRaises(ProcessLookupError):
                            os.kill(processes[0].pid, 0)
                    finally:
                        for signum, handler in previous.items():
                            signal.signal(signum, handler)
                        selector.close()
                        if reader is not None:
                            reader.close()
                        for process in processes:
                            if process.poll() is None:
                                process.kill()
                                process.wait(timeout=3)

    def test_nested_reader_and_metadata_restore_true_inherited_child_mask(self) -> None:
        original = subprocess.Popen
        previous = signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGUSR1})
        expected = set(previous) | {signal.SIGUSR1}
        code = (
            "import json,signal; print(json.dumps(sorted(signal.pthread_sigmask("
            "signal.SIG_BLOCK, set()))), flush=True)"
        )
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            git(root, "init", "-q")
            reader: Git | None = None

            def transport(command: list[str], **kwargs: object) -> subprocess.Popen[bytes]:
                if "cat-file" in command or "rev-parse" in command:
                    return cast(
                        "subprocess.Popen[bytes]",
                        original([sys.executable, "-I", "-c", code], **cast("Any", kwargs)),
                    )
                return cast("subprocess.Popen[bytes]", original(command, **cast("Any", kwargs)))

            try:
                with patch("subprocess.Popen", side_effect=transport):
                    with _blocked_cancellation() as inherited:
                        self.assertEqual(set(inherited), expected)
                        reader = Git(root, time.monotonic() + 5, child_signal_mask=inherited)
                        assert reader.process.stdout is not None
                        child_mask = set(json.loads(reader.process.stdout.readline()))
                    self.assertEqual(child_mask, expected)
                    self.assertEqual(set(json.loads(reader.small(("rev-parse", "HEAD")))), expected)
            finally:
                if reader is not None:
                    reader.close()
                signal.pthread_sigmask(signal.SIG_SETMASK, previous)


class CustodyTests(unittest.TestCase):
    def context(self, event: str, repair_name: str = "podman") -> tuple[AdmissionContext, Any]:
        repair = REGISTRY[f"Strukturpiloten/{repair_name}-lens"]
        return AdmissionContext(
            event,
            repair.repository,
            N,
            S,
            repair.base,
            N,
            repair.base if event == "pull_request" else "",
            repair.base if event == "merge_group" else "",
        ), repair

    def event_run(
        self, context: AdmissionContext, repair: Repair
    ) -> tuple[dict[str, Any], dict[str, Any], str]:
        repo = {"full_name": repair.repository}
        branch = f"gh-readonly-queue/main/pr-{repair.number}-{repair.base}"
        event = {
            "repository": repo,
            "merge_group": {
                "head_sha": N,
                "base_sha": repair.base,
                "base_ref": "refs/heads/main",
                "head_ref": "refs/heads/" + branch,
            },
        }
        run = {
            "id": 123,
            "run_attempt": 2,
            "repository": repo,
            "event": context.event,
            "head_sha": N,
            "head_branch": branch,
            "path": ".github/workflows/ci.yml",
            "pull_requests": [],
            "referenced_workflows": [{"path": f"{PRODUCER}/{WAITER}@{S}", "sha": S}],
        }
        return event, run, "refs/heads/" + branch

    def test_both_exact_single_pr_queue_bindings(self) -> None:
        for name, number in (("podman", 159), ("quadlet", 175)):
            context, repair = self.context("merge_group", name)
            event, run, ref = self.event_run(context, repair)
            bind_context(context, event, run, repair, 123, 2, ref)
            self.assertEqual(repair.number, number)

    def test_wrong_run_attempt_workflow_sha_ref_or_source_is_invalid(self) -> None:
        context, repair = self.context("merge_group")
        event, run, ref = self.event_run(context, repair)
        for key, value in (
            ("id", 124),
            ("run_attempt", 3),
            ("head_sha", H),
            ("event", "push"),
            ("path", "evil.yml"),
            ("head_branch", "other"),
            ("referenced_workflows", []),
        ):
            with self.subTest(key=key), self.assertRaises(ProofError) as caught:
                bind_context(context, event, {**run, key: value}, repair, 123, 2, ref)
            self.assertTrue(caught.exception.invalid)
        for suffix in ("-extra", "%2F", "/pr-160", "-" + H):
            altered = ref + suffix
            altered_event = {**event, "merge_group": {**event["merge_group"], "head_ref": altered}}
            altered_run = {**run, "head_branch": altered.removeprefix("refs/heads/")}
            with self.assertRaisesRegex(ProofError, "unsupported-queue-shape"):
                bind_context(context, altered_event, altered_run, repair, 123, 2, altered)

    def test_pr_run_names_source_H_while_event_and_git_bind_tested_N(self) -> None:
        for name in ("podman", "quadlet"):
            context, repair = self.context("pull_request", name)
            repo = {"full_name": repair.repository}
            event = {
                "repository": repo,
                "number": repair.number,
                "pull_request": {
                    "number": repair.number,
                    "base": {"sha": repair.base, "repo": repo},
                    "head": {"sha": H, "ref": "reviewed-repair", "repo": repo},
                },
            }
            _, run, _ = self.event_run(context, repair)
            run.update(
                head_sha=H, head_branch="reviewed-repair", pull_requests=[{"number": repair.number}]
            )
            ref = f"refs/pull/{repair.number}/merge"
            bind_context(context, event, run, repair, 123, 2, ref)
            with self.assertRaisesRegex(ProofError, "run-pr-ref"):
                bind_context(context, event, {**run, "head_sha": N}, repair, 123, 2, ref)
            data = self.public_data(context, repair)
            data[2] = {**data[2], "parents": [{"sha": repair.base}, {"sha": H}]}
            api = Metadata(time.monotonic() + 5, public=True)
            with (
                patch.object(api, "get", side_effect=data),
                patch.object(Git, "commit", return_value=("e" * 40, [repair.base, H])),
            ):
                source, reread = public_custody(api, context, repair, event, Git.__new__(Git))
                self.assertEqual(source, H)
                reread()
            for parents in ([H, repair.base], [repair.base], [repair.base, S]):
                with (
                    patch.object(api, "get", side_effect=data),
                    patch.object(Git, "commit", return_value=("e" * 40, parents)),
                    self.assertRaises(ProofError),
                ):
                    public_custody(api, context, repair, event, Git.__new__(Git))

    def test_recorded_podman_pr_H_and_N_are_distinct_independent_anchors(self) -> None:
        # Historical public run42264263741 facts from the independent preflight,
        # not generated from this implementation. S is explicitly mocked because
        # that historical run predates this new proof implementation.
        source = "e124c73c93190ab2376322c3ee0715c7d90ffde3"
        tested = "3f92bc6a59adfad0e84a22411cae8a51ba52da6d"
        context, repair = self.context("pull_request")
        context = replace(context, candidate_sha=tested, tested_sha=tested)
        repository = {"full_name": repair.repository}
        event = {
            "repository": repository,
            "number": 159,
            "pull_request": {
                "number": 159,
                "base": {"sha": repair.base},
                "head": {"sha": source, "ref": "TheRealBecks/issue158"},
            },
        }
        _, run, _ = self.event_run(context, repair)
        run.update(
            id=42264263741,
            run_attempt=1,
            head_sha=source,
            head_branch="TheRealBecks/issue158",
            pull_requests=[{"number": 159}],
        )
        self.assertNotEqual(source, tested)
        bind_context(context, event, run, repair, 42264263741, 1, "refs/pull/159/merge")
        for wrong in (tested, S):
            with self.assertRaises(ProofError):
                bind_context(
                    context,
                    event,
                    {**run, "head_sha": wrong},
                    repair,
                    42264263741,
                    1,
                    "refs/pull/159/merge",
                )

    def public_data(self, context: AdmissionContext, repair: Repair) -> list[dict[str, Any]]:
        repo = {"full_name": repair.repository}
        pull = {
            "number": repair.number,
            "state": "open",
            "merged": False,
            "merged_at": None,
            "base": {"repo": repo, "ref": "main", "sha": H},
            "head": {"repo": repo, "sha": H},
            "merge_commit_sha": N,
        }
        return [
            {**repo, "private": False, "visibility": "public", "default_branch": "main"},
            pull,
            {"sha": N, "tree": {"sha": "e" * 40}, "parents": [{"sha": repair.base}]},
            {"sha": H, "tree": {"sha": "e" * 40}, "parents": [{"sha": repair.base}]},
            {
                "status": "ahead",
                "base_commit": {"sha": repair.base},
                "merge_base_commit": {"sha": repair.base},
                "total_commits": 1,
                "commits": [{"sha": H}],
            },
            pull,
        ]

    def test_public_queue_binding_uses_six_fixed_gets_and_final_head_reread(self) -> None:
        for name in ("podman", "quadlet"):
            context, repair = self.context("merge_group", name)
            api = Metadata(time.monotonic() + 5, public=True)
            calls: list[str] = []
            data = iter(self.public_data(context, repair))

            def get(
                path: str, calls: list[str] = calls, data: Iterator[dict[str, Any]] = data
            ) -> dict[str, Any]:
                calls.append(path)
                return next(data)

            with (
                patch.object(api, "get", side_effect=get),
                patch.object(Git, "commit", return_value=("e" * 40, [repair.base])),
            ):
                source, recheck = public_custody(api, context, repair, {}, Git.__new__(Git))
                self.assertEqual(source, H)
                recheck()
            self.assertEqual(len(calls), 6)
            self.assertEqual(calls[1], calls[-1])
            self.assertTrue(all(path.startswith(f"/repos/{repair.repository}") for path in calls))

    def test_private_unavailable_stale_head_tree_parent_and_ancestry_refuse(self) -> None:
        context, repair = self.context("merge_group")
        mutations = (
            (0, "private", True),
            (0, "visibility", "private"),
            (1, "state", "closed"),
            (2, "parents", [{"sha": repair.base}, {"sha": H}]),
            (3, "tree", {"sha": "f" * 40}),
            (4, "merge_base_commit", {"sha": H}),
            (4, "total_commits", 2),
            (5, "head", {"repo": {"full_name": repair.repository}, "sha": S}),
        )
        for index, key, value in mutations:
            data = self.public_data(context, repair)
            data[index] = {**data[index], key: value}
            api = Metadata(time.monotonic() + 5, public=True)
            with (
                self.subTest(index=index, key=key),
                patch.object(api, "get", side_effect=data),
                patch.object(Git, "commit", return_value=("e" * 40, [repair.base])),
                self.assertRaises(ProofError),
            ):
                _, recheck = public_custody(api, context, repair, {}, Git.__new__(Git))
                recheck()

    def test_canonical_commit_is_dereferenced_without_ancestor_alternative(self) -> None:
        api = Metadata(time.monotonic() + 5)
        with patch.object(
            api,
            "get",
            side_effect=[
                {"ref": "refs/heads/automation-v1", "object": {"type": "commit", "sha": S}},
                {"sha": S},
            ],
        ) as read:
            self.assertEqual(canonical(api), S)
            self.assertEqual(read.call_count, 2)
        with (
            patch.object(
                api,
                "get",
                return_value={
                    "ref": "refs/tags/automation-v1",
                    "object": {"type": "commit", "sha": S},
                },
            ),
            self.assertRaises(ProofError),
        ):
            canonical(api)

    def test_http_403_and_429_have_distinct_safe_status_and_error_class(self) -> None:
        for public in (False, True):
            for status in (403, 429):
                api = Metadata(time.monotonic() + 5, public=public)
                error = HTTPError(
                    "https://api.github.com/SECRET-SENTINEL",
                    status,
                    "SECRET-SENTINEL private response",
                    Message(),
                    None,
                )
                with (
                    self.subTest(public=public, status=status),
                    patch.object(api.opener, "open", side_effect=error),
                    self.assertRaises(ProofError) as caught,
                ):
                    api.get("/repos/Strukturpiloten/podman-lens")
                self.assertEqual(caught.exception.details["http_status"], status)
                self.assertEqual(caught.exception.details["error_class"], "HTTPError")
                self.assertEqual(
                    caught.exception.details["transport"],
                    "public-metadata" if public else "source-metadata",
                )
                self.assertNotIn("SECRET-SENTINEL", json.dumps(caught.exception.details))

    def test_public_reader_never_inherits_auth_proxy_or_follows_redirects(self) -> None:
        class Response:
            status = 200

            def __enter__(self) -> Response:
                return self

            def __exit__(self, *args: object) -> None:
                pass

            def read(self, size: int) -> bytes:
                return b"{}"

        with patch.dict(
            os.environ,
            {"GH_TOKEN": "secret", "GITHUB_TOKEN": "secret", "HTTPS_PROXY": "http://invalid"},
        ):
            api = Metadata(time.monotonic() + 5, public=True)
            with patch.object(api.opener, "open", return_value=Response()) as opened:
                api.get("/repos/Strukturpiloten/podman-lens")
            request = opened.call_args.args[0]
            self.assertIsNone(request.get_header("Authorization"))
            self.assertEqual(request.get_method(), "GET")
            for _ in range(5):
                with patch.object(api.opener, "open", return_value=Response()):
                    api.get("/repos/Strukturpiloten/podman-lens")
            with self.assertRaisesRegex(ProofError, "metadata-request-budget"):
                api.get("/repos/Strukturpiloten/podman-lens")
            with self.assertRaisesRegex(ProofError, "metadata-endpoint"):
                api.get("/repos/Strukturpiloten/podman-lens/dispatches")


class ConfirmationTests(unittest.TestCase):
    def authority(self, variant: str = "") -> Tree:
        modules = {
            "automation/src/boxferry_ci/__init__.py": b"",
            "automation/src/boxferry_ci/operational.py": (
                b"def _features(root): return {}, {}, {}\n"
            ),
            "automation/src/boxferry_ci/dependencies.py": (
                "def check_dependencies(base, head, *, state, context, fetch,\n"
                "                       fetch_request):\n"
                " assert context is None\n"
                + (" fetch('forbidden')\n" if variant == "fetch" else "")
                + (" fetch_request('forbidden')\n" if variant == "fetch_request" else "")
                + (" state['first_seen']['forged'] = 'now'\n" if variant == "state" else "")
                + " return {'status':'pass','results':[], 'first_party_exempt':"
                + ("True" if variant == "actor" else "False")
                + ", 'state':state, 'operational_coverage_gaps':[]}\n"
            ).encode(),
        }
        return Tree("a" * 40, {p: entry(data) for p, data in modules.items()})

    def test_isolated_held_modules_and_both_refused_callbacks(self) -> None:
        empty = Tree("b" * 40, {"input": entry(b"complete fixture input")})
        # Labelled synthetic A API fixture proves isolation mechanics. Author
        # integration evidence separately invokes actual exact-A on both full trees.
        with patch.dict(os.environ, {"PYTHONPATH": "/evil", "GH_TOKEN": "secret"}):
            result = confirm_exact_A(self.authority(), empty, empty, time.monotonic() + 10)
        self.assertTrue(result["confirmed"])
        self.assertEqual(result["calls"], {"fetch": 0, "fetch_request": 0})
        self.assertTrue(result["state_unchanged_empty"])
        for variant in ("fetch", "fetch_request", "state", "actor"):
            with self.subTest(variant=variant), self.assertRaises(ProofError):
                confirm_exact_A(self.authority(variant), empty, empty, time.monotonic() + 10)

    def test_changed_coverage_and_opaque_feature_errors_refuse(self) -> None:
        before = Tree("b" * 40, {"input": entry(b"before")})
        after = Tree("c" * 40, {"input": entry(b"after")})
        authority = self.authority()
        path = "automation/src/boxferry_ci/operational.py"
        module = b"def _features(root): return {}, {}, {'opaque': (root/'input').read_text()}\n"
        authority = replace(authority, entries={**authority.entries, path: entry(module)})
        with self.assertRaisesRegex(ProofError, "exact-A-confirmation-refused"):
            confirm_exact_A(authority, before, after, time.monotonic() + 10)
        result = confirm_exact_A(authority, before, before, time.monotonic() + 10)
        self.assertTrue(result["confirmed"])
        self.assertEqual(result["opaque_feature_errors_before"], {"opaque": "before"})

    def test_confirmation_child_preserves_inherited_mask_without_guard_signals(self) -> None:
        previous = signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGUSR1})
        expected = sorted({int(value) for value in previous} | {int(signal.SIGUSR1)})
        authority = self.authority()
        path = "automation/src/boxferry_ci/dependencies.py"
        original = authority.entries[path].content
        check = (
            " assert sorted(signal.pthread_sigmask(signal.SIG_BLOCK, set())) == "
            + repr(expected)
            + "\n"
        ).encode()
        source = b"import signal\n" + original.replace(
            b" assert context is None\n", check + b" assert context is None\n", 1
        )
        authority = replace(authority, entries={**authority.entries, path: entry(source)})
        empty = Tree("b" * 40, {"input": entry(b"complete inert input")})
        try:
            self.assertTrue(
                confirm_exact_A(authority, empty, empty, time.monotonic() + 5)["confirmed"]
            )
        finally:
            signal.pthread_sigmask(signal.SIG_SETMASK, previous)

    def test_nonregular_confirmation_inputs_refuse_without_materialization(self) -> None:
        symlink = Tree("b" * 40, {"unsafe": entry(b"/outside", 0o120000)})
        with self.assertRaisesRegex(ProofError, "nonregular-confirmation-input"):
            confirm_exact_A(self.authority(), symlink, symlink, time.monotonic() + 10)


class OrchestrationTests(unittest.TestCase):
    """Labelled custody/helper mocks test decisions, not full closure correctness."""

    def test_explicit_zero_only_and_live_ref_advancement_invalid_context_fallback(self) -> None:
        helper = CustodyTests()
        context, repair = helper.context("merge_group")
        event, run, ref = helper.event_run(context, repair)
        before, after = RepairTests().snapshots()
        with tempfile.TemporaryDirectory() as temporary:
            event_path = Path(temporary) / "event.json"
            event_path.write_text(json.dumps(event))
            options = argparse.Namespace(
                event_name=context.event,
                repository=context.repository,
                candidate_sha=N,
                tested_sha=N,
                automation_sha=S,
                policy_sha=repair.base,
                pr_base_sha="",
                queue_base_sha=repair.base,
                implementation_root=Path("S"),
                policy_root=Path("B"),
                consumer_root=Path("N"),
                authority_root=Path("A"),
                event_path=event_path,
                run_id=123,
                run_attempt=2,
                ref=ref,
            )
            environment = {
                "GITHUB_REPOSITORY": context.repository,
                "GITHUB_EVENT_NAME": context.event,
                "GITHUB_SHA": N,
                "GITHUB_RUN_ID": "123",
                "GITHUB_RUN_ATTEMPT": "2",
                "GITHUB_REF": ref,
                "GITHUB_EVENT_PATH": str(event_path),
            }
            for scenario, expected in (
                ("zero", "proved-zero"),
                ("advance", "needs-service"),
                ("invalid", "invalid-context"),
                ("unavailable", "needs-service"),
            ):
                with (
                    self.subTest(scenario=scenario),
                    patch.dict(os.environ, environment),
                    patch("boxferry_ci.zero_age_delta.Git") as constructor,
                    patch("boxferry_ci.zero_age_delta.acquire_consumer", return_value=Path("N")),
                    patch(
                        "boxferry_ci.zero_age_delta.resolve_admission",
                        return_value=AdmissionSelection(
                            "required", N, repair.base, S, AUTHORITY, POLICY
                        ),
                    ),
                    patch("boxferry_ci.zero_age_delta.verify_producer", return_value={}),
                    patch(
                        "boxferry_ci.zero_age_delta.confirm_exact_A",
                        return_value={"introduced_identities": 0},
                    ),
                    patch(
                        "boxferry_ci.zero_age_delta.canonical",
                        side_effect=[S, H if scenario == "advance" else S],
                    ),
                    patch.object(
                        Metadata, "get", side_effect=[run, *helper.public_data(context, repair)]
                    ) as metadata,
                ):
                    reader = constructor.return_value
                    reader.head.return_value = AUTHORITY
                    reader.tree.side_effect = [
                        before,
                        after,
                        Tree(AUTHORITY, {}),
                        fixture_producer(),
                    ]
                    reader.commit.return_value = ("e" * 40, [repair.base])
                    if scenario == "invalid":
                        metadata.side_effect = [{**run, "head_sha": H}]
                    if scenario == "unavailable":
                        metadata.side_effect = ProofError(
                            "metadata-unavailable",
                            details={"http_status": 429, "error_class": "HTTPError"},
                        )
                    report = prove_zero(options)
                    self.assertEqual(report["status"], expected)
                    if scenario == "unavailable":
                        self.assertEqual(
                            report["diagnostics"], {"http_status": 429, "error_class": "HTTPError"}
                        )
                    if scenario == "zero":
                        self.assertEqual(
                            report["source_provenance"], "current-pr-head-with-complete-tested-tree"
                        )
                        self.assertEqual(report["policy_implementation_sha"], AUTHORITY)
                        self.assertEqual(report["proof_implementation_sha"], S)

    def test_unknown_owner_base_authority_and_variant_never_acquire_api_or_producer(self) -> None:
        helper = CustodyTests()
        context, repair = helper.context("merge_group")
        before, after = RepairTests().snapshots()
        variants = (
            (
                "Strukturpiloten/compose-lens",
                repair.base,
                AUTHORITY,
                after,
                "unregistered-repository",
            ),
            (repair.repository, H, AUTHORITY, after, "unregistered-base"),
            (repair.repository, repair.base, H, after, "unsupported-authority-A"),
            (
                repair.repository,
                repair.base,
                AUTHORITY,
                replace(after, entries={**after.entries, "new.lock": entry(b"new identity")}),
                "unregistered-repair-variant",
            ),
            (
                repair.repository,
                repair.base,
                AUTHORITY,
                replace(after, entries={**after.entries, CONTRACT: entry(b"unknown test repair")}),
                "unregistered-repair-variant",
            ),
        )
        for repository, base, authority, candidate, reason in variants:
            options = argparse.Namespace(
                event_name=context.event,
                repository=repository,
                candidate_sha=N,
                tested_sha=N,
                automation_sha=S,
                policy_sha=base,
                pr_base_sha="",
                queue_base_sha=base,
                implementation_root=Path("S"),
                policy_root=Path("B"),
                consumer_root=Path("N"),
                authority_root=Path("A"),
                event_path=Path("unused"),
                run_id=123,
                run_attempt=2,
                ref="queue",
            )
            environment = {
                "GITHUB_REPOSITORY": repository,
                "GITHUB_EVENT_NAME": context.event,
                "GITHUB_SHA": N,
                "GITHUB_RUN_ID": "123",
                "GITHUB_RUN_ATTEMPT": "2",
                "GITHUB_REF": "queue",
                "GITHUB_EVENT_PATH": "unused",
            }
            with (
                self.subTest(reason=reason),
                patch.dict(os.environ, environment),
                patch("boxferry_ci.zero_age_delta.Git") as constructor,
                patch("boxferry_ci.zero_age_delta.acquire_consumer", return_value=Path("N")),
                patch(
                    "boxferry_ci.zero_age_delta.resolve_admission",
                    return_value=AdmissionSelection("required", N, base, S, authority, POLICY),
                ),
                patch.object(
                    Metadata, "get", side_effect=AssertionError("API must stay unused")
                ) as api,
            ):
                reader = constructor.return_value
                reader.head.return_value = N
                reader.tree.side_effect = [before, candidate]
                report = prove_zero(options)
                self.assertEqual(report["status"], "needs-service")
                self.assertEqual(report["reason"], reason)
                api.assert_not_called()
                self.assertLessEqual(reader.tree.call_count, 2)


class ProducerClosureTests(unittest.TestCase):
    # Independent review anchors, deliberately not read back from the checker.
    EXPECTED_604: ClassVar[dict[str, str]] = {
        ".github/workflows/shared-release-prepare.yml": (
            "9b0d8b73511a5d924dbb7349dd6d7fe35fefb0e59ad97f8db41973fa60b3a6e5"
        ),
        "automation/src/boxferry_ci/release.py": (
            "bc4fb17aee42d4ffca9e56c73f534bfdb95c9401cb872572a103b380e52f28f3"
        ),
        "automation/tests/test_release.py": (
            "44a52a8558b99dfe8e0918b5ab76a34d2f96512d9e201da51645135bbdb13c20"
        ),
    }

    @classmethod
    def setUpClass(cls) -> None:
        # A bounded complete minimal producer fixture, committed only in an owned
        # scratch repository. Binding the real authority commit is tested above;
        # this fixture independently exercises every closure/refusal branch.
        old_waiter = b"""---
name: Inert producer closure fixture
on: workflow_call
permissions:
  contents: read
jobs:
  check:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
      # This artifact is an untrusted fixture, never execution provenance.
      - name: Read the exact candidate receipt under A's policy
        if: steps.policy.outputs.required == 'true'
        env:
          ADMISSION_REQUIRED: ${{ steps.policy.outputs.required }}
"""
        before_files = {
            p.relative_to(ROOT).as_posix(): (p.read_bytes(), 0o644)
            for p in (ROOT / "automation/src/boxferry_ci").rglob("*.py")
            if p.relative_to(ROOT).as_posix() != PROOF_PATH
        }
        before_files[WAITER] = old_waiter, 0o644
        for path in (
            "automation/uv.lock",
            "automation/pyproject.toml",
            ".github/workflows/shared-rust-ci.yml",
        ):
            before_files[path] = (ROOT / path).read_bytes(), 0o644
        for path in cls.EXPECTED_604:
            before_files[path] = b"inert pre-604 fixture bytes\n", 0o644
        after_files = dict(before_files)
        for path in IMPLEMENTATION_PATHS | cls.EXPECTED_604.keys():
            after_files[path] = (ROOT / path).read_bytes(), 0o644
        for path in FIXTURE_ROOT.rglob("*.txt"):
            after_files[path.relative_to(ROOT).as_posix()] = path.read_bytes(), 0o644
        after_files[WAITER] = expected_waiter(old_waiter), 0o644
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            git(root, "init", "-q")
            write_files(root, before_files)
            git(root, "add", ".")
            git(root, "commit", "-qm", "minimal-complete-A-fixture")
            cls.before = read_tree(root, git(root, "rev-parse", "HEAD"))
            write_files(root, after_files)
            git(root, "add", ".")
            git(root, "commit", "-qm", "exact-reviewed-composition-fixture")
            cls.after = read_tree(root, git(root, "rev-parse", "HEAD"))

    def test_complete_minimal_producer_accepts_exact_604_and_closed_606_delta(self) -> None:
        self.assertEqual(set(REQUIRED_604), set(self.EXPECTED_604))
        for path, digest in self.EXPECTED_604.items():
            self.assertEqual(self.after.entries[path].digest, digest)
        report = verify_producer(self.before, self.after)
        self.assertEqual(report["required_604"], "exact-postimages")
        self.assertEqual(report["producer_A_tree"], self.before.oid)
        self.assertEqual(report["producer_S_tree"], self.after.oid)
        self.assertEqual(
            set(report["producer_changed_paths"]), changed_paths(self.before, self.after)
        )
        self.assertIn(PROOF_PATH, report["producer_changed_paths"])

    def test_current_source_slot_change_refuses_before_repair_or_evidence_authorization(
        self,
    ) -> None:
        path = "automation/src/boxferry_ci/eligibility.py"
        current = self.before.entries[path].content
        node = next(
            node
            for node in ast.parse(current).body
            if isinstance(node, ast.FunctionDef) and node.name == "expected_ci_upgrade"
        )
        lines = current.splitlines(keepends=True)
        legacy_transform()  # Verify the independent held-A/609 function hash/provenance.
        legacy = (
            ROOT / "automation/tests/fixtures/source-slot-parity/legacy-expected-ci-upgrade.py.txt"
        ).read_bytes()
        old = b"".join(lines[: node.lineno - 1]) + legacy + b"".join(lines[node.end_lineno :])
        self.assertNotEqual(old, current)
        authority = replace(self.before, entries={**self.before.entries, path: entry(old)})
        with self.assertRaises(ProofError) as caught:
            verify_producer(authority, self.after)
        self.assertEqual(caught.exception.reason, "producer-extra-tree-delta")
        self.assertFalse(caught.exception.invalid)

        helper = CustodyTests()
        context, repair = helper.context("merge_group")
        event, run, ref = helper.event_run(context, repair)
        before, after = RepairTests().snapshots()
        with tempfile.TemporaryDirectory() as temporary:
            event_path = Path(temporary) / "event.json"
            event_path.write_text(json.dumps(event))
            options = argparse.Namespace(
                event_name=context.event,
                repository=context.repository,
                candidate_sha=N,
                tested_sha=N,
                automation_sha=S,
                policy_sha=repair.base,
                pr_base_sha="",
                queue_base_sha=repair.base,
                implementation_root=Path("S"),
                policy_root=Path("B"),
                consumer_root=Path("N"),
                authority_root=Path("A"),
                event_path=event_path,
                run_id=123,
                run_attempt=2,
                ref=ref,
            )
            environment = {
                "GITHUB_REPOSITORY": context.repository,
                "GITHUB_EVENT_NAME": context.event,
                "GITHUB_SHA": N,
                "GITHUB_RUN_ID": "123",
                "GITHUB_RUN_ATTEMPT": "2",
                "GITHUB_REF": ref,
                "GITHUB_EVENT_PATH": str(event_path),
            }
            with (
                patch.dict(os.environ, environment),
                patch("boxferry_ci.zero_age_delta.Git") as constructor,
                patch("boxferry_ci.zero_age_delta.acquire_consumer", return_value=Path("N")),
                patch(
                    "boxferry_ci.zero_age_delta.resolve_admission",
                    return_value=AdmissionSelection(
                        "required",
                        N,
                        repair.base,
                        S,
                        AUTHORITY,
                        POLICY,
                    ),
                ),
                patch("boxferry_ci.zero_age_delta.canonical", return_value=S),
                patch.object(Metadata, "get", return_value=run),
                patch("boxferry_ci.zero_age_delta.match_repair_shape") as shape,
                patch("boxferry_ci.zero_age_delta.repin") as pin,
                patch("boxferry_ci.zero_age_delta.public_custody") as custody,
                patch("boxferry_ci.zero_age_delta.confirm_exact_A") as confirmation,
            ):
                reader = constructor.return_value
                reader.head.return_value = AUTHORITY
                reader.tree.side_effect = [before, after, authority, self.after]
                result = prove_zero(options)
            self.assertEqual(result["status"], "needs-service")
            self.assertEqual(result["reason"], "producer-extra-tree-delta")
            for operation in (shape, pin, custody, confirmation):
                operation.assert_not_called()

    def test_independent_twelve_producer_and_607_postimage_negatives(self) -> None:
        cases = [
            (
                "age-engine",
                "automation/src/boxferry_ci/dependencies.py",
                "change",
                "producer-extra-tree-delta",
            ),
            (
                "operational-engine",
                "automation/src/boxferry_ci/operational.py",
                "change",
                "producer-extra-tree-delta",
            ),
            ("lock", "automation/uv.lock", "change", "producer-extra-tree-delta"),
            (
                "dependency-declaration",
                "automation/pyproject.toml",
                "change",
                "producer-extra-tree-delta",
            ),
            (
                "third-party-actions",
                ".github/workflows/shared-rust-ci.yml",
                "change",
                "producer-extra-tree-delta",
            ),
            (
                "unknown-tool",
                "tools/new-download-integrity.json",
                "add",
                "producer-extra-tree-delta",
            ),
        ]
        for path in self.EXPECTED_604:
            cases.extend(
                (
                    ("changed-604", path, "change", "required-604-postimage"),
                    ("missing-604", path, "delete", "required-regular-file"),
                )
            )
        self.assertEqual(len(cases), 12)
        for label, path, mutation, reason in cases:
            changed = dict(self.after.entries)
            if mutation == "delete":
                del changed[path]
            else:
                changed[path] = entry(
                    (changed[path].content if path in changed else b"")
                    + b"\nnew unreviewed dependency-bearing bytes\n"
                )
            with self.subTest(label=label, path=path), self.assertRaises(ProofError) as caught:
                verify_producer(self.before, replace(self.after, entries=changed))
            self.assertEqual(caught.exception.reason, reason)
            self.assertFalse(caught.exception.invalid)

    def test_wrong_mode_waiter_template_and_running_implementation_refuse_specifically(
        self,
    ) -> None:
        cases = [
            (path, replace(self.after.entries[path], mode=0o100755), "required-regular-file", False)
            for path in self.EXPECTED_604
        ]
        cases.extend(
            (
                (
                    WAITER,
                    entry(self.after.entries[WAITER].content + b"# arbitrary waiter edit\n"),
                    "producer-waiter-shape",
                    False,
                ),
                (
                    FIXTURES + "podman/caller-after.txt",
                    entry(b"not the reviewed caller"),
                    "reviewed-template-hash",
                    False,
                ),
                (
                    PROOF_PATH,
                    entry(b"not the executing immutable proof implementation"),
                    "executing-proof-source",
                    True,
                ),
            )
        )
        for path, value, reason, invalid in cases:
            with self.subTest(path=path), self.assertRaises(ProofError) as caught:
                verify_producer(
                    self.before, replace(self.after, entries={**self.after.entries, path: value})
                )
            self.assertEqual(caught.exception.reason, reason)
            self.assertEqual(caught.exception.invalid, invalid)


class IntegrationTests(unittest.TestCase):
    def test_waiter_preserves_exact_receipt_and_required_ci_result_contract(self) -> None:
        workflow = (ROOT / WAITER).read_text()
        self.assertIn("timeout-minutes: 5", workflow)
        self.assertIn("steps.zero.outputs.status != 'proved-zero'", workflow)
        self.assertIn("python -m boxferry_ci.eligibility wait", workflow)
        self.assertIn("admission_implementation_sha=%s", workflow)
        self.assertIn("admission_evidence_kind=read-only-zero-age-delta", workflow)
        self.assertNotIn("pull-requests:", workflow)
        self.assertNotIn("contents: write", workflow)
        self.assertNotIn("statuses: write", workflow)
        self.assertNotIn("dependency-eligibility-state-v1", workflow)
        self.assertIn("admission_status=eligible", workflow)
        # Selected-job aggregator behavior remains owned by untouched A CI, not
        # this proof; a proof does not mark selected failures/skips successful.
        self.assertNotIn("not-applicable'", workflow)

    def test_fixture_provenance_and_renovate_exclusion(self) -> None:
        import hashlib

        provenance = json.loads((FIXTURE_ROOT / "provenance.json").read_text())
        self.assertIn("MPL-2.0", provenance["license"])
        self.assertEqual(len(provenance["files"]), 12)
        for path, record in provenance["files"].items():
            data = (ROOT / path).read_bytes()
            self.assertEqual(hashlib.sha256(data).hexdigest(), record["sha256"])
            self.assertEqual(len(data), record["size"])
        renovate = json.loads((ROOT / ".github/renovate.json").read_text())
        self.assertIn("**/tests/**", renovate["ignorePaths"])

    def test_arbitrary_waiter_has_no_integration_anchor(self) -> None:
        original = b"not the known A waiter"
        with self.assertRaisesRegex(ProofError, "waiter-integration-anchor"):
            expected_waiter(original)


if __name__ == "__main__":
    unittest.main()
