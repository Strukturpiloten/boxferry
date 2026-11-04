"""Independent Git graphs for the current single-entry squash queue contract."""

from __future__ import annotations

import os
import shutil
import signal
import subprocess
import tempfile
import time
import unittest
from collections.abc import Callable, Generator
from contextlib import contextmanager
from copy import deepcopy
from pathlib import Path
from unittest.mock import patch

from boxferry_ci import eligibility, release
from boxferry_ci import queue_tree as subject

REPOSITORY = "Strukturpiloten/boxferry"


class QueueTreeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="boxferry-queue-fixture-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.git("init", "--bare", "--quiet", "--template=")
        # P is old main; B contains the unrelated npm-integrity fix. H branched
        # from P and contains only the workflow recovery. Author Q's expected
        # contents independently: never ask merge-tree to generate our oracle.
        self.p_files = {"package-lock.json": b"old npm integrity\n", "workflow": b"old\n"}
        self.b_files = {**self.p_files, "package-lock.json": b"fixed npm integrity\n"}
        self.h_files = {**self.p_files, "workflow": b"recovered workflow\n"}
        self.q_files = {
            "package-lock.json": b"fixed npm integrity\n",
            "workflow": b"recovered workflow\n",
        }
        self.p = self.commit(self.p_files, label="old main")
        self.b = self.commit(self.b_files, self.p, label="npm fix on main")
        self.h = self.commit(self.h_files, self.p, label="recovery source")
        self.m = self.commit(self.h_files, self.p, self.h, label="stale synthetic PR merge")
        self.q = self.commit(self.q_files, self.b, label="actual tested squash queue")
        self.scratches: list[Path] = []

    def git(self, *arguments: str, data: bytes | None = None) -> str:
        environment = {
            **subject._environment(self.root),
            "GIT_AUTHOR_NAME": "fixture",
            "GIT_AUTHOR_EMAIL": "fixture@example.invalid",
            "GIT_COMMITTER_NAME": "fixture",
            "GIT_COMMITTER_EMAIL": "fixture@example.invalid",
            "GIT_AUTHOR_DATE": "2026-11-04T00:00:00Z",
            "GIT_COMMITTER_DATE": "2026-11-04T00:00:00Z",
        }
        return (
            subprocess.check_output(
                ["/usr/bin/git", "-C", str(self.root), *arguments],
                input=data,
                env=environment,
                stderr=subprocess.DEVNULL,
                timeout=10,
            )
            .decode()
            .strip()
        )

    def commit(self, files: dict[str, bytes], *parents: str, label: str) -> str:
        records = []
        for name, content in sorted(files.items()):
            blob = self.git("hash-object", "-w", "--stdin", data=content)
            records.append(f"100644 blob {blob}\t{name}\n")
        tree = self.git("mktree", data="".join(records).encode())
        arguments = ["commit-tree", tree]
        for parent in parents:
            arguments.extend(("-p", parent))
        return self.git(*arguments, data=f"{label}\n".encode())

    @contextmanager
    def objects(self) -> Generator[None]:
        def acquire(
            root: Path, _deadline: float, repository: str, revisions: tuple[str, ...]
        ) -> None:
            self.assertEqual(repository, REPOSITORY)
            self.assertEqual(len(revisions), len(set(revisions)))
            self.assertFalse((root / "workflow").exists())
            self.assertFalse((root / "hooks").exists())
            self.scratches.append(root)
            # Test-only transfer of immutable objects; no fixture Git config,
            # refs, hooks, alternates, checkout or credentials cross this seam.
            shutil.copytree(self.root / "objects", root / "objects", dirs_exist_ok=True)

        with patch.object(subject, "_fetch", side_effect=acquire):
            yield
        self.assertTrue(all(not root.exists() for root in self.scratches))

    def verify(
        self, candidate: str | None = None, *, base: str | None = None, source: str | None = None
    ) -> None:
        subject.verify_queue_tree(
            REPOSITORY,
            base or self.b,
            source or self.h,
            candidate or self.q,
            deadline=time.monotonic() + 30,
        )

    def test_stale_pr_merge_and_source_are_not_ancestors_of_valid_squash_queue(self) -> None:
        self.assertEqual(
            self.git("rev-list", "--parents", "--max-count=1", self.q).split(), [self.q, self.b]
        )
        self.assertEqual(self.git("merge-base", self.b, self.h), self.p)
        self.assertNotEqual(self.git("merge-base", self.h, self.q), self.h)
        self.assertNotEqual(self.git("merge-base", self.b, self.m), self.b)
        with self.objects():
            self.verify()

    def test_full_tree_rejects_missing_npm_fix_missing_source_extra_and_modified_files(
        self,
    ) -> None:
        wrong = (
            self.h_files,
            self.b_files,
            {**self.q_files, "unrelated": b"not from either input\n"},
            {**self.q_files, "package-lock.json": b"substituted integrity\n"},
        )
        with self.objects():
            for index, files in enumerate(wrong):
                candidate = self.commit(files, self.b, label=f"wrong tree {index}")
                with (
                    self.subTest(index=index),
                    self.assertRaisesRegex(subject.QueueTreeError, "tree differs"),
                ):
                    self.verify(candidate)

    def test_wrong_base_source_and_queue_parents_fail(self) -> None:
        multi_parent = self.commit(self.q_files, self.b, self.h, label="unreviewed queue shape")
        wrong_base = self.commit(self.q_files, self.p, label="wrong queue parent")
        with self.objects():
            for candidate in (self.b, self.h, self.m, multi_parent, wrong_base):
                with self.subTest(candidate=candidate), self.assertRaises(subject.QueueTreeError):
                    self.verify(candidate)
            with self.assertRaises(subject.QueueTreeError):
                self.verify(base=self.p)
            with self.assertRaises(subject.QueueTreeError):
                self.verify(source=self.p)

    def test_real_merge_conflict_and_multiple_merge_bases_fail(self) -> None:
        conflict = self.commit(
            {**self.p_files, "package-lock.json": b"conflicting source\n"}, self.p, label="conflict"
        )
        left = self.commit({"file": b"left\n"}, self.p, label="left")
        right = self.commit({"file": b"right\n"}, self.p, label="right")
        criss_b = self.commit(self.b_files, left, right, label="crisscross target")
        criss_h = self.commit(self.h_files, right, left, label="crisscross source")
        criss_q = self.commit(self.q_files, criss_b, label="crisscross queue")
        with self.objects():
            with self.assertRaisesRegex(subject.QueueTreeError, "tree differs"):
                self.verify(source=conflict)
            with self.assertRaisesRegex(subject.QueueTreeError, "unambiguous merge base"):
                self.verify(criss_q, base=criss_b, source=criss_h)

    def test_backlog_uses_one_acquisition_for_clean_conflict_and_unequal_sources(self) -> None:
        conflict = self.commit(
            {**self.p_files, "package-lock.json": b"conflicting source\n"}, self.p, label="conflict"
        )
        other = self.commit({**self.p_files, "other": b"waiting\n"}, self.p, label="waiting")
        with self.objects(), patch.object(subject, "_fetch", wraps=subject._fetch) as fetch:
            self.assertEqual(
                subject.matching_queue_sources(
                    REPOSITORY,
                    self.b,
                    (conflict, other, self.h),
                    self.q,
                    deadline=time.monotonic() + 30,
                ),
                {self.h},
            )
            self.assertEqual(fetch.call_count, 1)
            self.assertEqual(len(self.scratches), 1)

    def test_false_conflict_backlog_and_stale_base_classify_through_real_tree_proof(self) -> None:
        conflict = self.commit(
            {**self.p_files, "package-lock.json": b"conflict\n"}, self.p, label="waiting conflict"
        )
        api = eligibility.GitHub(REPOSITORY, "never-transmitted-fixture-token")
        pulls = {
            number: {
                "number": number,
                "state": "open",
                "mergeable": number == 17,
                "merge_commit_sha": self.m,
                "base": {"sha": self.p, "ref": "main", "repo": {"full_name": REPOSITORY}},
                "head": {"sha": source, "repo": {"full_name": REPOSITORY}},
                "user": {"login": "renovate[bot]", "type": "Bot"},
                "title": "feat(api)!: change public type",
            }
            for number, source in ((17, self.h), (18, conflict))
        }
        nodes = [
            {
                "baseCommit": {"oid": self.b},
                "headCommit": {"oid": source},
                "pullRequest": {"number": number, "headRefOid": source},
            }
            for number, source in ((18, conflict), (17, self.h))
        ]
        graph = {
            "data": {
                "repository": {
                    "mergeQueue": {
                        "entries": {
                            "totalCount": 2,
                            "nodes": nodes,
                        }
                    }
                }
            }
        }
        responses = {
            "git/ref/heads/main": {
                "ref": "refs/heads/main",
                "object": {"type": "commit", "sha": self.b},
            },
            f"compare/{self.b}...{self.q}": {
                "status": "ahead",
                "merge_base_commit": {"sha": self.b},
            },
            **{f"pulls/{number}": pull for number, pull in pulls.items()},
        }
        rules = [
            {
                "type": "merge_queue",
                "parameters": {
                    "max_entries_to_build": 1,
                    "max_entries_to_merge": 1,
                },
            }
        ]

        def resolve() -> release.QueueBinding:
            candidate = eligibility.queue_candidate(api, self.q, "main")
            return release.QueueBinding(
                candidate.base,
                candidate.head,
                candidate.number,
                candidate.source_head,
                candidate.title,
                candidate.queue_target,
                REPOSITORY,
                candidate.actor_login,
                candidate.actor_type,
            )

        with (
            self.objects(),
            patch.object(api, "get", side_effect=lambda path: deepcopy(responses[path])),
            patch.object(api, "items", side_effect=lambda _path: deepcopy(rules)),
            patch.object(api, "request", side_effect=lambda *_args: deepcopy(graph)),
        ):
            self.assertEqual(
                release.queue_release_type(
                    self.b, self.q, resolve, lambda number: deepcopy(pulls[number])
                ),
                "major",
            )
        self.assertEqual(len(self.scratches), 2)  # Fresh proof on each metadata readback.

    def test_distinct_sources_with_identical_clean_trees_both_remain_matches(self) -> None:
        same = self.commit(self.h_files, self.p, label="different source same intent")
        with self.objects():
            self.assertEqual(
                subject.matching_queue_sources(
                    REPOSITORY, self.b, (self.h, same), self.q, deadline=time.monotonic() + 30
                ),
                {self.h, same},
            )

    def test_entire_waiting_set_fails_for_unknown_or_ambiguous_source(self) -> None:
        left = self.commit({"file": b"left\n"}, self.p, label="left")
        right = self.commit({"file": b"right\n"}, self.p, label="right")
        base = self.commit(self.b_files, left, right, label="ambiguous base")
        ambiguous = self.commit(self.h_files, right, left, label="ambiguous source")
        candidate = self.commit(self.q_files, base, label="ambiguous queue")
        for source in ("f" * 40, ambiguous):
            with (
                self.subTest(source=source),
                self.objects(),
                self.assertRaises(subject.QueueTreeError),
            ):
                subject.matching_queue_sources(
                    REPOSITORY, base, (self.h, source), candidate, deadline=time.monotonic() + 30
                )

    def test_source_bound_never_truncates_or_acquires_a_prefix(self) -> None:
        with patch.object(subject, "_fetch") as fetch:
            with self.assertRaisesRegex(subject.QueueTreeError, "bounded"):
                subject.matching_queue_sources(
                    REPOSITORY,
                    self.b,
                    (self.h,) * (subject.MAX_SOURCES + 1),
                    self.q,
                    deadline=time.monotonic() + 30,
                )
            fetch.assert_not_called()

    def test_all_sources_share_one_deadline_and_storage_budget(self) -> None:
        other = self.commit({**self.p_files, "other": b"waiting\n"}, self.p, label="waiting")
        original = subject._git
        deadlines: list[float] = []

        def consume(root: Path, deadline: float, *arguments: str) -> str:
            deadlines.append(deadline)
            result = original(root, deadline, *arguments)
            if arguments[0] == "merge-tree":
                # Resources left by one source must count against the next.
                (root / "accumulated-budget").write_bytes(b"x" * subject.MAX_BYTES)
            return result

        with self.objects(), patch.object(subject, "_git", side_effect=consume):
            with self.assertRaisesRegex(subject.QueueTreeError, "storage"):
                subject.matching_queue_sources(
                    REPOSITORY, self.b, (self.h, other), self.q, deadline=time.monotonic() + 30
                )
        self.assertEqual(len(set(deadlines)), 1)

    def test_malformed_or_fatal_merge_output_cannot_eliminate_source(self) -> None:
        for output in ("", self.q, self.q + "\0", self.q + "\0garbage\0"):
            with self.subTest(output=output):
                self.assertFalse(subject._conflict_output(output))
        original = subprocess.Popen

        def fail(
            _command: list[str],
            *,
            env: dict[str, str],
            stdin: int,
            stdout: int,
            stderr: int,
            start_new_session: bool,
            preexec_fn: Callable[[], None],
        ) -> subprocess.Popen[bytes]:
            # Exit=1 alone does not establish a merge conflict.
            return original(
                ["/usr/bin/python3", "-c", "raise SystemExit(1)"],
                env=env,
                stdin=stdin,
                stdout=stdout,
                stderr=stderr,
                start_new_session=start_new_session,
                preexec_fn=preexec_fn,
            )

        with patch.object(subject.subprocess, "Popen", side_effect=fail):
            with self.assertRaisesRegex(subject.QueueTreeError, "no queue tree proof"):
                subject._git(
                    self.root,
                    time.monotonic() + 10,
                    "merge-tree",
                    "--write-tree",
                    "-z",
                    "--no-messages",
                    self.b,
                    self.h,
                )

    def test_missing_and_corrupt_immutable_objects_fail(self) -> None:
        with self.objects():
            with self.assertRaises(subject.QueueTreeError):
                self.verify(source="f" * 40)
        blob = self.git("rev-parse", f"{self.b}:package-lock.json")
        object_path = self.root / "objects" / blob[:2] / blob[2:]
        object_path.chmod(0o600)
        object_path.write_bytes(b"invalid git data")
        with self.objects(), self.assertRaisesRegex(subject.QueueTreeError, "verify-objects"):
            self.verify()

    def test_shallow_partial_and_alternate_graphs_fail(self) -> None:
        original = subject._fetch
        for kind in ("shallow", "alternates", "promisor"):
            with self.subTest(kind=kind), self.objects():
                acquire = subject._fetch

                def incomplete(
                    root: Path,
                    deadline: float,
                    repository: str,
                    revisions: tuple[str, ...],
                    *,
                    kind: str = kind,
                    acquire: Callable[[Path, float, str, tuple[str, ...]], None] = acquire,
                ) -> None:
                    acquire(root, deadline, repository, revisions)
                    path = {
                        "shallow": root / "shallow",
                        "alternates": root / "objects/info/alternates",
                        "promisor": root / "objects/pack/fake.promisor",
                    }[kind]
                    path.write_text(self.p + "\n")

                with (
                    patch.object(subject, "_fetch", side_effect=incomplete),
                    self.assertRaises(subject.QueueTreeError),
                ):
                    self.verify()
        self.assertIs(subject._fetch, original)

    def test_environment_and_git_configuration_hooks_credentials_are_isolated(self) -> None:
        marker = self.root / "executed"
        malicious = {
            "GH_TOKEN": "private-token",
            "HTTPS_PROXY": "https://private-token@invalid",
            "GIT_CONFIG_COUNT": "1",
            "GIT_CONFIG_KEY_0": "core.hooksPath",
            "GIT_CONFIG_VALUE_0": str(self.root),
            "GIT_EXEC_PATH": str(self.root),
            "GIT_OBJECT_DIRECTORY": str(self.root / "bad-objects"),
            "GIT_ALTERNATE_OBJECT_DIRECTORIES": str(self.root),
            "GIT_TRACE": str(marker),
            "CURL_HOME": str(self.root),
            "SSH_ASKPASS": str(marker),
        }
        (self.root / ".gitconfig").write_text(
            f"[core]\n hooksPath={self.root}\n[credential]\n helper=!touch {marker}\n"
        )
        (self.root / "info").mkdir(exist_ok=True)
        (self.root / "info/attributes").write_text("* merge=evil\n")
        with patch.dict(os.environ, malicious), self.objects():
            environment = subject._environment(Path("/private/scratch"))
            self.assertEqual(environment["PATH"], "/usr/bin:/bin")
            self.assertFalse(any("private-token" in value for value in environment.values()))
            self.assertNotIn("GIT_EXEC_PATH", environment)
            self.assertNotIn("GIT_OBJECT_DIRECTORY", environment)
            self.verify()
        self.assertFalse(marker.exists())

    def test_fixed_allowlisted_fetch_has_no_token_remote_config_or_partial_history(self) -> None:
        with patch.object(subject, "_git") as command:
            subject._fetch(self.root, 123.0, REPOSITORY, (self.b, self.h, self.q))
        self.assertEqual(command.call_count, 1)
        arguments = command.call_args.args[2:]
        self.assertIn(f"https://github.com/{REPOSITORY}.git", arguments)
        for required in (
            "--no-tags",
            "--no-write-fetch-head",
            "--no-recurse-submodules",
            "--no-filter",
        ):
            self.assertIn(required, arguments)
        self.assertFalse(any(argument.startswith("--depth") for argument in arguments))
        with self.assertRaises(subject.QueueTreeError):
            subject.verify_queue_tree("attacker/project", self.b, self.h, self.q, deadline=123.0)
        with self.assertRaises(subject.QueueTreeError):
            subject.verify_queue_tree(REPOSITORY, "main", self.h, self.q, deadline=123.0)

    def test_time_disk_file_and_output_budgets_fail_and_remove_scratch(self) -> None:
        with self.objects(), self.assertRaisesRegex(subject.QueueTreeError, "time budget"):
            subject.verify_queue_tree(
                REPOSITORY, self.b, self.h, self.q, deadline=time.monotonic() - 1
            )
        with (
            patch.object(subject, "MAX_BYTES", 10),
            self.assertRaisesRegex(subject.QueueTreeError, "storage"),
        ):
            subject._storage(self.root)
        with (
            patch.object(subject, "MAX_FILES", 1),
            self.assertRaisesRegex(subject.QueueTreeError, "storage"),
        ):
            subject._storage(self.root)
        with (
            patch.object(subject, "MAX_OUTPUT", 1),
            self.assertRaisesRegex(subject.QueueTreeError, "output"),
        ):
            subject._git(self.root, time.monotonic() + 10, "--version")

    def test_running_process_deadline_reaps_entire_process_group(self) -> None:
        original = subprocess.Popen
        processes = []

        def slow(
            _command: list[str],
            *,
            env: dict[str, str],
            stdin: int,
            stdout: int,
            stderr: int,
            start_new_session: bool,
            preexec_fn: Callable[[], None],
        ) -> subprocess.Popen[bytes]:
            process = original(
                ["/usr/bin/python3", "-c", "import time; time.sleep(30)"],
                env=env,
                stdin=stdin,
                stdout=stdout,
                stderr=stderr,
                start_new_session=start_new_session,
                preexec_fn=preexec_fn,
            )
            processes.append(process)
            return process

        with patch.object(subject.subprocess, "Popen", side_effect=slow):
            with self.assertRaisesRegex(subject.QueueTreeError, "time budget"):
                subject._git(self.root, time.monotonic() + 0.15, "--version")
        self.assertEqual(len(processes), 1)
        self.assertIsNotNone(processes[0].poll())
        with self.assertRaises(ProcessLookupError):
            os.killpg(processes[0].pid, 0)

    def test_cancelled_proof_removes_scratch_and_restores_signal_handler(self) -> None:
        previous = signal.getsignal(signal.SIGTERM)
        scratches = []

        def cancel(
            root: Path, _deadline: float, _repository: str, _revisions: tuple[str, ...]
        ) -> None:
            scratches.append(root)
            os.kill(os.getpid(), signal.SIGTERM)

        with patch.object(subject, "_fetch", side_effect=cancel):
            with self.assertRaisesRegex(subject.QueueTreeError, "cancelled"):
                self.verify()
        self.assertEqual(signal.getsignal(signal.SIGTERM), previous)
        self.assertTrue(all(not root.exists() for root in scratches))


if __name__ == "__main__":
    unittest.main()
