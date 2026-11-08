"""Official-generator isolation, manager partition and fixed-branch regressions."""

from __future__ import annotations

import copy
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from collections.abc import Mapping
from dataclasses import replace
from email.message import Message
from http.client import HTTPMessage
from io import BytesIO
from pathlib import Path
from typing import Any, cast
from unittest.mock import patch

from boxferry_ci import devcontainer_updates as updates
from boxferry_ci.runner import Diagnostic, RunResult

ROOT = Path(__file__).resolve().parents[2]


def result(status: str = "success") -> RunResult:
    return RunResult(
        status,
        0 if status == "success" else 1,
        "",
        "registry unavailable",
        0,
        Diagnostic("generate", "network failure", "feature", "test", "retry"),
        ("node", "upgrade"),
    )


class GeneratorTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.candidate = self.root / "candidate"
        self.automation = self.root / "trusted"
        (self.candidate / ".devcontainer").mkdir(parents=True)
        cli = (
            self.automation
            / "automation/devcontainer/node_modules/@devcontainers/cli/devcontainer.js"
        )
        cli.parent.mkdir(parents=True)
        cli.write_text("trusted generator")
        self.node = self.root / "node"
        self.node.touch()
        self.declaration = b'{"features":{"ghcr.io/example/root:1.0.0":{}}}'
        (self.candidate / updates.DECLARATION).write_bytes(self.declaration)
        (self.candidate / updates.LOCK).write_text("original")
        self.environment = patch.dict(
            os.environ,
            {
                "BOXFERRY_NODE": str(self.node),
                "RENOVATE_TOKEN": "do-not-inherit-token",
                "BOXFERRY_FEATURE_READ_TOKEN": "do-not-inherit-reader",
                "NODE_OPTIONS": "malicious-hook",
                "DOCKER_CONFIG": "personal-credentials",
            },
        )
        self.environment.start()
        self.addCleanup(self.environment.stop)
        self.validated: list[Path] = []

    def generate(self, command: updates.Run) -> bool:
        return updates.generate_lock(
            self.candidate,
            self.automation,
            command=command,
            validate=lambda root: self.validated.append(root),
        )

    def command(self, argv: tuple[str, ...], **kwargs: object) -> RunResult:
        self.assertEqual(argv[2:4], ("upgrade", "--workspace-folder"))
        root = Path(argv[4])
        self.assertNotEqual(root, self.candidate)
        self.assertEqual((root / updates.DECLARATION).read_bytes(), self.declaration)
        self.assertFalse((root / updates.LOCK).exists())
        environment = cast(Mapping[str, str], kwargs["environment"])
        self.assertNotIn("RENOVATE_TOKEN", environment)
        self.assertNotIn("BOXFERRY_FEATURE_READ_TOKEN", environment)
        self.assertNotIn("NODE_OPTIONS", environment)
        self.assertNotEqual(environment["DOCKER_CONFIG"], "personal-credentials")
        (root / updates.LOCK).write_text('{"features":{"root":{},"transitive":{}}}')
        return result()

    def test_scratch_full_graph_validation_and_idempotence(self) -> None:
        self.assertTrue(self.generate(self.command))
        self.assertEqual(len(self.validated), 1)
        self.assertFalse(self.generate(self.command))
        self.assertEqual(len(self.validated), 2)

    def test_real_generator_child_receives_neither_parent_credential(self) -> None:
        observed = self.root / "child-observation.json"
        self.node.write_text(
            f"#!{sys.executable}\n"
            "import json, os, pathlib, sys\n"
            "root = pathlib.Path(sys.argv[sys.argv.index('--workspace-folder') + 1])\n"
            "keys = ['RENOVATE_TOKEN', 'BOXFERRY_FEATURE_READ_TOKEN',\n"
            "        'RENOVATE_CONFIG_FILE', 'NODE_OPTIONS']\n"
            "observed = {key: key in os.environ for key in keys}\n"
            f"pathlib.Path({str(observed)!r}).write_text(json.dumps(observed))\n"
            "output = root / '.devcontainer/devcontainer-lock.json'\n"
            "output.write_text(json.dumps(dict(features={})))\n"
        )
        self.node.chmod(0o755)
        self.assertTrue(self.generate(updates.runner.run))
        self.assertEqual(
            json.loads(observed.read_text()),
            dict.fromkeys(
                (
                    "RENOVATE_TOKEN",
                    "BOXFERRY_FEATURE_READ_TOKEN",
                    "RENOVATE_CONFIG_FILE",
                    "NODE_OPTIONS",
                ),
                False,
            ),
        )

    def test_failed_partial_generation_retries_once_and_preserves_lock(self) -> None:
        calls = []

        def fail(argv: tuple[str, ...], **kwargs: object) -> RunResult:
            calls.append(argv)
            (Path(argv[4]) / updates.LOCK).write_text("partial")
            return result("failed")

        with self.assertRaisesRegex(updates.UpdateError, "generation.*retry"):
            self.generate(fail)
        self.assertEqual(len(calls), 2)
        self.assertEqual((self.candidate / updates.LOCK).read_text(), "original")
        self.assertFalse(self.validated)

    def test_rejected_graph_never_replaces_existing_lock(self) -> None:
        def reject(root: Path) -> None:
            raise ValueError("missing transitive node")

        with self.assertRaisesRegex(ValueError, "transitive"):
            updates.generate_lock(
                self.candidate, self.automation, command=self.command, validate=reject
            )
        self.assertEqual((self.candidate / updates.LOCK).read_text(), "original")

    def test_unexpected_generator_declaration_edit_is_rejected(self) -> None:
        def edit(argv: tuple[str, ...], **kwargs: object) -> RunResult:
            self.command(argv, **kwargs)
            (Path(argv[4]) / updates.DECLARATION).write_text("changed")
            return result()

        with self.assertRaisesRegex(updates.UpdateError, "declarations"):
            self.generate(edit)
        self.assertEqual((self.candidate / updates.LOCK).read_text(), "original")

    def test_local_or_floating_feature_sources_are_rejected_before_execution(self) -> None:
        for feature in (
            "./local-feature",
            "ghcr.io/example/root:1",
            "https://example.invalid/feature.tgz",
        ):
            (self.candidate / updates.DECLARATION).write_text(
                json.dumps({"features": {feature: {}}})
            )
            with self.assertRaisesRegex(updates.UpdateError, "exact-version"):
                self.generate(self.command)
        self.assertFalse(self.validated)
        self.assertEqual((self.candidate / updates.LOCK).read_text(), "original")

    def test_symlink_document_is_refused(self) -> None:
        (self.candidate / updates.DECLARATION).unlink()
        (self.root / "outside").write_bytes(self.declaration)
        (self.candidate / updates.DECLARATION).symlink_to(self.root / "outside")
        with self.assertRaisesRegex(updates.UpdateError, "regular file"):
            self.generate(self.command)


class ConfigTests(unittest.TestCase):
    def config(self) -> dict[str, Any]:
        with patch.dict(os.environ, {"BOXFERRY_NODE": "/trusted/bin/node"}):
            return updates.limited_config(
                "Strukturpiloten/boxferry", Path("/trusted/source"), Path("/trusted/bin/python")
            )

    def test_fixed_trusted_config_no_candidate_hooks_or_unbounded_owners(self) -> None:
        config = self.config()
        self.assertEqual(config["requireConfig"], "ignored")
        self.assertFalse(config["onboarding"])
        self.assertEqual(config["enabledManagers"], ["devcontainer", "custom.regex"])
        self.assertEqual(config["minimumReleaseAge"], "3 days")
        self.assertEqual(config["internalChecksFilter"], "strict")
        self.assertEqual(config["minimumReleaseAgeBehaviour"], "timestamp-required")
        self.assertFalse(config["automerge"])
        self.assertFalse(config["platformAutomerge"])
        self.assertEqual(
            config["statusCheckNames"],
            dict.fromkeys(
                ("artifactError", "configValidation", "mergeConfidence", "minimumReleaseAge")
            ),
        )
        self.assertEqual(config["branchPrefix"], "renovate-devcontainer/")
        self.assertEqual(config["postUpgradeTasks"]["fileFilters"], [updates.LOCK])
        command = config["postUpgradeTasks"]["commands"][0]
        self.assertIn(" -I /trusted/source/automation/src/boxferry_ci/", command)
        self.assertNotIn("{{", command)
        self.assertRegex(command, config["allowedCommands"][0])
        self.assertEqual(config["lockFileMaintenance"], {"enabled": False})
        self.assertEqual(len(config["customManagers"]), 1)
        self.assertFalse(config["allowCommandTemplating"])
        self.assertEqual(config["allowedEnv"], [])
        self.assertNotIn("token", config)

    def test_outside_repo_and_shell_paths_refused(self) -> None:
        for repository in ("Strukturpiloten/docker-lens", "Other/boxferry"):
            with self.assertRaisesRegex(updates.UpdateError, "four"):
                updates.limited_config(repository, Path("/trusted"), Path("/python"))
        with self.assertRaisesRegex(updates.UpdateError, "characters"):
            updates.limited_config(
                "Strukturpiloten/boxferry", Path("/trusted;evil"), Path("/python")
            )

    def test_native_read_rule_is_literal_current_repo_and_exported_config_is_secret_free(
        self,
    ) -> None:
        for repository in updates.REPOSITORIES:
            rule = updates._native_read_rule(repository, "reader-sentinel")
            self.assertEqual(
                rule["matchHost"], f"https://api.github.com/repos/{repository}/commits/"
            )
            self.assertTrue(rule["readOnly"])
            self.assertTrue(rule["abortOnError"])
            self.assertEqual(rule["abortIgnoreStatusCodes"], [])
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "config.json"
            with (
                patch.dict(
                    os.environ,
                    {
                        "BOXFERRY_NODE": "/trusted/bin/node",
                        "RENOVATE_TOKEN": "writer-sentinel",
                        "BOXFERRY_FEATURE_READ_TOKEN": "reader-sentinel",
                    },
                ),
                patch.object(
                    sys,
                    "argv",
                    [
                        "updater",
                        "config",
                        "--repository",
                        "Strukturpiloten/boxferry",
                        "--automation",
                        str(ROOT),
                        "--output",
                        str(output),
                    ],
                ),
            ):
                self.assertEqual(updates.main(), 0)
            exported = output.read_text()
            self.assertNotIn("writer-sentinel", exported)
            self.assertNotIn("reader-sentinel", exported)
            self.assertNotIn("hostRules", json.loads(exported))

    def test_official_package_integrity_and_hosted_native_npm_owner(self) -> None:
        project = ROOT / "automation/devcontainer"
        manifest = json.loads((project / "package.json").read_text())
        lock = json.loads((project / "package-lock.json").read_text())
        package = lock["packages"]["node_modules/@devcontainers/cli"]
        self.assertEqual(manifest["devDependencies"]["@devcontainers/cli"], package["version"])
        self.assertTrue(package["integrity"].startswith("sha512-"))
        local = json.loads((ROOT / ".github/renovate.json").read_text())
        self.assertFalse(
            any(m.get("depNameTemplate") == "@devcontainers/cli" for m in local["customManagers"])
        )


class GitHubDiagnosticTests(unittest.TestCase):
    def test_permission_error_names_only_method_and_api_path(self) -> None:
        token = "app-token-must-stay-private"
        query = "query-credential-must-stay-private"
        body_secret = "body-secret-must-stay-private"
        response_secret = "response-secret-must-stay-private"
        path = "/git/trees?credential=" + query
        url = "https://api.github.com/repos/Strukturpiloten/boxferry" + path
        error = updates.urllib.error.HTTPError(url, 403, response_secret, Message(), None)
        with patch.object(updates.urllib.request, "build_opener") as opener:
            opener.return_value.open.side_effect = error
            with self.assertRaises(updates.UpdateError) as raised:
                updates.GitHub("Strukturpiloten/boxferry", token).request(
                    path, "POST", {"secret": body_secret}
                )
        message = str(raised.exception)
        self.assertIn(
            "GitHub API POST /repos/Strukturpiloten/boxferry/git/trees: HTTP 403", message
        )
        for secret in (token, query, body_secret, response_secret, "credential=", "Authorization"):
            self.assertNotIn(secret, message)


class FakeCommitReader(updates.CommitReader):
    def __init__(self, api: FakeGitHub) -> None:
        self.repository = api.repository
        self.owner = api
        self.calls: list[tuple[str, str]] = []
        self.check_document: dict[str, Any] | None = None
        self.status_document: dict[str, Any] | None = None
        self.failure: Exception | None = None

    def checks(self, head: str) -> dict[str, Any]:
        self.calls.append(("checks", head))
        if self.failure:
            raise self.failure
        if self.check_document is not None:
            return copy.deepcopy(self.check_document)
        return {
            "total_count": 1,
            "check_runs": [
                {
                    "name": "Required CI",
                    "head_sha": self.owner.head,
                    "url": f"https://api.github.com/repos/{self.repository}/check-runs/1",
                    "status": "completed",
                    "conclusion": "success" if self.owner.ready else "failure",
                }
            ],
        }

    def statuses(self, head: str) -> dict[str, Any]:
        self.calls.append(("statuses", head))
        if self.failure:
            raise self.failure
        if self.status_document is not None:
            return copy.deepcopy(self.status_document)
        return {
            "sha": self.owner.head,
            "url": f"https://api.github.com/repos/{self.repository}/commits/{self.owner.head}/status",
            "total_count": 1,
            "statuses": [
                {
                    "url": f"https://api.github.com/repos/{self.repository}/statuses/{self.owner.head}",
                    "context": "Dependency eligibility",
                    "state": "success" if self.owner.ready else "pending",
                }
            ],
        }


class FakeGitHub(updates.GitHub):
    repository = "Strukturpiloten/boxferry"

    def __init__(self) -> None:
        self.calls: list[tuple[str, str, object]] = []
        self.reader = FakeCommitReader(self)
        self.pulls: list[dict[str, Any]] = []
        self.base = "a" * 40
        self.head = "b" * 40
        self.race = False
        self.sha_reads = 0
        self.versioned: list[dict[str, Any]] = []
        self.comparison = "ahead"
        self.ready = False
        self.queue = True
        self.lock = b'{"features":{}}'
        self.owned_committer = updates.BOT
        self.tree_oids: dict[str, object] = {self.base: "1" * 40, self.head: "2" * 40}
        self.source_trees = {
            "1" * 40: {updates.DECLARATION: "main declaration", "unrelated.txt": "main sentinel"},
            "2" * 40: {
                updates.DECLARATION: "version declaration",
                "unrelated.txt": "version sentinel",
            },
        }
        self.created_tree: dict[str, str] = {}

    def sha(self, branch: str) -> str:
        self.sha_reads += 1
        if self.race and self.sha_reads > 1:
            return "c" * 40
        return self.base if branch == "main" else self.head

    def document(self, path: str, sha: str) -> bytes:
        return self.lock if sha == self.head and path == updates.LOCK else b'{"features":{}}'

    def request(self, path: str, method: str = "GET", body: object = None) -> dict[str, Any]:
        self.calls.append((path, method, body))
        if not path:
            return {"default_branch": "main"}
        if path.startswith("/pulls?"):
            return {"items": self.pulls if updates.MAINTENANCE in path else self.versioned}
        if path.startswith("/compare/"):
            return {
                "status": self.comparison,
                "files": [{"filename": updates.LOCK, "status": "modified"}],
                "commits": [{"committer": {"login": self.owned_committer}}],
                "total_commits": 1,
            }
        if path.startswith("/rules/"):
            return {"items": [{"type": "merge_queue"}] if self.queue else []}
        if "/check-runs?" in path or "/status?" in path:
            raise AssertionError("writer must not read required check/status evidence")
        if path.startswith("/git/commits/"):
            source = path.rsplit("/", 1)[-1]
            return {"sha": source, "tree": {"sha": self.tree_oids[source]}}
        if path == "/git/trees" and method == "POST":
            value = cast(dict[str, Any], body)
            # The Git data API accepts a tree object identity, never a commit OID.
            if value["base_tree"] not in self.source_trees:
                raise AssertionError("CreateTree received a commit or unknown tree object")
            self.created_tree = dict(self.source_trees[value["base_tree"]])
            for entry in value["tree"]:
                self.created_tree[entry["path"]] = entry["content"]
            return {"sha": "d" * 40}
        if path.startswith("/commits/"):
            return {"committer": {"login": self.owned_committer}}
        if path.startswith("/pulls/"):
            return self.versioned[0] if self.versioned else self.owned()
        return {"sha": "d" * 40}

    def owned(self, branch: str = updates.MAINTENANCE) -> dict[str, Any]:
        return {
            "number": 3,
            "node_id": "PR3",
            "state": "open",
            "draft": False,
            "user": {"login": updates.BOT},
            "base": {"ref": "main"},
            "head": {"repo": {"full_name": self.repository}, "sha": self.head, "ref": branch},
        }

    def graphql(self, query: str, variables: dict[str, str]) -> dict[str, Any]:
        self.calls.append(("/graphql", "POST", {"query": query, "variables": variables}))
        if query.startswith("mutation"):
            return {"data": {"enqueuePullRequest": {"mergeQueueEntry": {"id": "ENTRY"}}}}
        return {"data": {"node": {"headRefOid": self.head, "mergeQueueEntry": {"id": "ENTRY"}}}}


class MaintenanceTests(unittest.TestCase):
    def test_unchanged_graph_is_noop(self) -> None:
        api = FakeGitHub()
        self.assertFalse(updates.maintenance(api, api.reader, ROOT, lambda *_: False))
        self.assertFalse(any(method != "GET" for _, method, _ in api.calls))

    def test_digest_or_transitive_only_graph_change_publishes_lock_only(self) -> None:
        api = FakeGitHub()

        def changed(root: Path, automation: Path) -> bool:
            (root / updates.LOCK).write_text('{"features":{"new-transitive":{}}}')
            return True

        self.assertTrue(updates.maintenance(api, api.reader, ROOT, changed))
        tree = cast(
            dict[str, Any], next(body for path, _, body in api.calls if path == "/git/trees")
        )
        self.assertEqual([entry["path"] for entry in tree["tree"]], [updates.LOCK])
        ref = cast(dict[str, Any], next(body for path, _, body in api.calls if path == "/git/refs"))
        self.assertEqual(ref["ref"], "refs/heads/" + updates.MAINTENANCE)
        self.assertEqual(tree["base_tree"], "1" * 40)
        self.assertNotEqual(tree["base_tree"], api.base)
        self.assertEqual(api.created_tree["unrelated.txt"], "main sentinel")
        self.assertEqual(api.created_tree[updates.DECLARATION], "main declaration")
        commit = cast(
            dict[str, Any], next(body for path, _, body in api.calls if path == "/git/commits")
        )
        self.assertEqual(commit["parents"], [api.base])
        self.assertEqual(commit["tree"], "d" * 40)
        self.assertEqual(api.reader.calls, [])
        self.assertFalse(any(path == "/graphql" for path, _, _ in api.calls))

    def test_base_race_does_not_publish(self) -> None:
        api = FakeGitHub()
        api.race = True
        with self.assertRaisesRegex(updates.UpdateError, "moved"):
            updates.maintenance(api, api.reader, ROOT, lambda *_: True)
        self.assertFalse(any(method != "GET" for _, method, _ in api.calls))

    def test_human_owned_pr_is_refused(self) -> None:
        api = FakeGitHub()
        api.pulls = [
            {
                "user": {"login": "human"},
                "base": {"ref": "main"},
                "head": {"repo": {"full_name": api.repository}, "sha": api.head},
            }
        ]
        with self.assertRaisesRegex(updates.UpdateError, "owned"):
            updates.maintenance(api, api.reader, ROOT, lambda *_: True)
        self.assertFalse(any(method != "GET" for _, method, _ in api.calls))

    def test_owned_stale_maintenance_branch_rebases_without_force(self) -> None:
        api = FakeGitHub()
        api.pulls = [api.owned()]
        api.comparison = "diverged"
        self.assertTrue(updates.maintenance(api, api.reader, ROOT, lambda *_: True))
        tree = cast(
            dict[str, Any], next(body for path, _, body in api.calls if path == "/git/trees")
        )
        commit = cast(
            dict[str, Any], next(body for path, _, body in api.calls if path == "/git/commits")
        )
        ref = cast(
            dict[str, Any],
            next(body for path, _, body in api.calls if path.startswith("/git/refs/")),
        )
        self.assertEqual(tree["base_tree"], api.tree_oids[api.base])
        self.assertEqual(api.created_tree["unrelated.txt"], "main sentinel")
        self.assertEqual(api.created_tree[updates.DECLARATION], "main declaration")
        self.assertEqual(commit["parents"], [api.head, api.base])
        self.assertFalse(ref["force"])

    def test_version_proposal_preserves_its_head_tree_and_commit_parent(self) -> None:
        api = FakeGitHub()
        api.versioned = [api.owned(updates.BRANCH)]

        def changed(root: Path, automation: Path) -> bool:
            (root / updates.LOCK).write_text('{"features":{"new":{}}}')
            return True

        self.assertTrue(updates.maintenance(api, api.reader, ROOT, changed))
        tree = cast(
            dict[str, Any], next(body for path, _, body in api.calls if path == "/git/trees")
        )
        commit = cast(
            dict[str, Any], next(body for path, _, body in api.calls if path == "/git/commits")
        )
        self.assertEqual(tree["base_tree"], "2" * 40)
        self.assertNotEqual(tree["base_tree"], api.head)
        self.assertEqual(api.created_tree[updates.DECLARATION], "version declaration")
        self.assertEqual(api.created_tree["unrelated.txt"], "version sentinel")
        self.assertEqual(commit["parents"], [api.head])
        self.assertEqual(commit["tree"], "d" * 40)

    def test_behind_maintenance_branch_uses_live_main_tree_and_both_commit_parents(self) -> None:
        api = FakeGitHub()
        api.pulls = [api.owned()]
        api.comparison = "behind"
        self.assertTrue(updates.maintenance(api, api.reader, ROOT, lambda *_: True))
        tree = cast(
            dict[str, Any], next(body for path, _, body in api.calls if path == "/git/trees")
        )
        commit = cast(
            dict[str, Any], next(body for path, _, body in api.calls if path == "/git/commits")
        )
        self.assertEqual(tree["base_tree"], "1" * 40)
        self.assertEqual(api.created_tree[updates.DECLARATION], "main declaration")
        self.assertEqual(api.created_tree["unrelated.txt"], "main sentinel")
        self.assertEqual(commit["parents"], [api.head, api.base])

    def test_malformed_source_tree_refuses_publication(self) -> None:
        for value in ("not-an-object-id", "", None):
            api = FakeGitHub()
            api.tree_oids[api.base] = value
            with self.assertRaisesRegex(updates.UpdateError, "commit/tree"):
                updates.maintenance(api, api.reader, ROOT, lambda *_: True)
            self.assertFalse(any(method != "GET" for _, method, _ in api.calls))

    def test_same_proposal_is_idempotent_and_does_not_restart_ci(self) -> None:
        api = FakeGitHub()
        api.pulls = [api.owned()]
        self.assertFalse(updates.maintenance(api, api.reader, ROOT, lambda *_: True))
        self.assertFalse(any(method != "GET" for _, method, _ in api.calls))

    def test_human_commit_on_bot_branch_is_refused(self) -> None:
        api = FakeGitHub()
        api.pulls = [api.owned()]
        api.owned_committer = "human"
        with self.assertRaisesRegex(updates.UpdateError, "owned"):
            updates.maintenance(api, api.reader, ROOT, lambda *_: True)
        self.assertFalse(any(method != "GET" for _, method, _ in api.calls))

    def test_redundant_own_proposal_is_closed_when_main_already_has_graph(self) -> None:
        api = FakeGitHub()
        api.pulls = [api.owned()]
        self.assertFalse(updates.maintenance(api, api.reader, ROOT, lambda *_: False))
        self.assertIn(("/pulls/3", "PATCH", {"state": "closed"}), api.calls)

    def test_versioned_stream_precedes_verified_maintenance(self) -> None:
        api = FakeGitHub()
        api.versioned = [api.owned(updates.BRANCH)]
        api.pulls = [api.owned()]
        self.assertFalse(updates.maintenance(api, api.reader, ROOT, lambda *_: False))
        self.assertEqual(sum(method == "PATCH" for _, method, _ in api.calls), 1)
        self.assertIn(("/pulls/3", "PATCH", {"state": "closed"}), api.calls)


class QueueTests(unittest.TestCase):
    def test_pending_or_failed_required_results_do_not_enqueue(self) -> None:
        api = FakeGitHub()
        self.assertFalse(
            updates._queue_ready(api, api.reader, 3, api.head, api.base, updates.MAINTENANCE)
        )
        self.assertFalse(any(method != "GET" for _, method, _ in api.calls))

    def test_protected_queue_admission_uses_exact_head_and_reads_back(self) -> None:
        api = FakeGitHub()
        api.ready = True
        self.assertTrue(
            updates._queue_ready(api, api.reader, 3, api.head, api.base, updates.MAINTENANCE)
        )
        requests = [cast(dict[str, Any], body) for path, _, body in api.calls if path == "/graphql"]
        self.assertEqual(requests[0]["variables"], {"id": "PR3", "head": api.head})
        self.assertIn("expectedHeadOid:$head", requests[0]["query"])
        self.assertNotIn("jump:", requests[0]["query"])
        self.assertEqual(len(requests), 2)

    def test_missing_queue_and_changed_heads_fail_closed(self) -> None:
        api = FakeGitHub()
        api.ready = True
        api.queue = False
        with self.assertRaisesRegex(updates.UpdateError, "protected"):
            updates._queue_ready(api, api.reader, 3, api.head, api.base, updates.MAINTENANCE)
        api = FakeGitHub()
        api.ready = True
        api.race = True
        with self.assertRaisesRegex(updates.UpdateError, "moved"):
            updates._queue_ready(api, api.reader, 3, api.head, api.base, updates.MAINTENANCE)
        self.assertFalse(any(method != "GET" for _, method, _ in api.calls))


class AuthoritativeReaderTests(unittest.TestCase):
    def test_both_streams_use_independent_held_head_metadata_and_exact_queue(self) -> None:
        for branch in (updates.BRANCH, updates.MAINTENANCE):
            api = FakeGitHub()
            api.ready = True
            if branch == updates.BRANCH:
                api.versioned = [api.owned(branch)]
            else:
                api.pulls = [api.owned()]
            if branch == updates.BRANCH:
                self.assertFalse(updates.maintenance(api, api.reader, ROOT, lambda *_: False))
            # Maintenance reuses the existing coherent lock before queueing.
            if branch == updates.MAINTENANCE:
                self.assertFalse(updates.maintenance(api, api.reader, ROOT, lambda *_: True))
            self.assertEqual(api.reader.calls, [("checks", api.head), ("statuses", api.head)])
            mutations = [body for path, _, body in api.calls if path == "/graphql"]
            self.assertEqual(len(mutations), 2)
            self.assertEqual(
                cast(dict[str, Any], mutations[0])["variables"], {"id": "PR3", "head": api.head}
            )
            self.assertFalse(
                any("/check-runs?" in path or "/status?" in path for path, _, _ in api.calls)
            )

    def test_denied_truncated_malformed_wrong_resource_and_unknown_states_never_enqueue(
        self,
    ) -> None:
        api = FakeGitHub()
        api.ready = True
        good_checks, good_statuses = api.reader.checks(api.head), api.reader.statuses(api.head)
        cases: list[tuple[dict[str, Any], dict[str, Any]]] = []
        for key in ("total_count", "check_runs"):
            document = copy.deepcopy(good_checks)
            del document[key]
            cases.append((document, good_statuses))
        for count in (True, -1, 2, 100):
            document = copy.deepcopy(good_checks)
            document["total_count"] = count
            cases.append((document, good_statuses))
        for field, value in (
            ("head_sha", "c" * 40),
            ("url", "https://api.github.com/repos/Other/private/check-runs/1"),
            ("status", "unknown"),
            ("conclusion", "unknown"),
            ("name", None),
        ):
            document = copy.deepcopy(good_checks)
            document["check_runs"][0][field] = value
            cases.append((document, good_statuses))
        for field, value in (
            ("sha", "c" * 40),
            ("url", "https://api.github.com/repos/Other/private/commits/" + api.head + "/status"),
            ("total_count", 2),
            ("statuses", None),
        ):
            document = copy.deepcopy(good_statuses)
            document[field] = value
            cases.append((good_checks, document))
        for field, value in (
            ("state", "unknown"),
            ("context", None),
            ("url", "https://api.github.com/repos/Other/private/statuses/" + api.head),
        ):
            document = copy.deepcopy(good_statuses)
            document["statuses"][0][field] = value
            cases.append((good_checks, document))
        for checks, statuses in cases:
            api.reader.check_document, api.reader.status_document = checks, statuses
            api.calls.clear()
            with (
                self.subTest(checks=checks, statuses=statuses),
                self.assertRaises(updates.UpdateError),
            ):
                updates._queue_ready(api, api.reader, 3, api.head, api.base, updates.MAINTENANCE)
            self.assertFalse(any(method != "GET" for _, method, _ in api.calls))
        for code in (401, 403):
            api.reader.failure = updates.UpdateError(f"GitHub API GET held metadata: HTTP {code}")
            with self.assertRaisesRegex(updates.UpdateError, str(code)):
                updates._queue_ready(api, api.reader, 3, api.head, api.base, updates.MAINTENANCE)
            self.assertFalse(any(method != "GET" for _, method, _ in api.calls))

    def test_duplicate_success_cannot_override_failure_and_readback_is_mandatory(self) -> None:
        api = FakeGitHub()
        api.ready = True
        document = api.reader.statuses(api.head)
        document["statuses"].append(
            {
                "context": "Required CI",
                "state": "failure",
                "url": f"https://api.github.com/repos/{api.repository}/statuses/{api.head}",
            }
        )
        document["total_count"] = 2
        api.reader.status_document = document
        self.assertFalse(
            updates._queue_ready(api, api.reader, 3, api.head, api.base, updates.MAINTENANCE)
        )
        self.assertFalse(any(method != "GET" for _, method, _ in api.calls))
        api.reader.status_document = None
        real = api.graphql

        def moved(query: str, variables: dict[str, str]) -> dict[str, Any]:
            return (
                real(query, variables)
                if query.startswith("mutation")
                else {
                    "data": {"node": {"headRefOid": "c" * 40, "mergeQueueEntry": {"id": "ENTRY"}}}
                }
            )

        with (
            patch.object(api, "graphql", side_effect=moved),
            self.assertRaisesRegex(updates.UpdateError, "readback"),
        ):
            updates._queue_ready(api, api.reader, 3, api.head, api.base, updates.MAINTENANCE)

    def test_queue_mutation_and_readback_require_matching_nonempty_entry_ids(self) -> None:
        api = FakeGitHub()
        api.ready = True
        mutation = {"data": {"enqueuePullRequest": {"mergeQueueEntry": {"id": "MQE_fixture"}}}}
        observed = {
            "data": {"node": {"headRefOid": api.head, "mergeQueueEntry": {"id": "MQE_fixture"}}}
        }
        malformed = [
            {"unexpected": True},
            "not-an-entry",
            {"id": None},
            {"id": ""},
            {"id": "   "},
            {"id": True},
            {"id": 7},
            None,
            [],
        ]
        for entry in malformed:
            bad_mutation = {"data": {"enqueuePullRequest": {"mergeQueueEntry": entry}}}
            with (
                self.subTest(stage="mutation", entry=entry),
                patch.object(api, "graphql", side_effect=[bad_mutation, observed]) as call,
            ):
                with self.assertRaisesRegex(updates.UpdateError, "admission refused"):
                    updates._queue_ready(
                        api, api.reader, 3, api.head, api.base, updates.MAINTENANCE
                    )
                self.assertEqual(call.call_count, 1)
            bad_readback = {"data": {"node": {"headRefOid": api.head, "mergeQueueEntry": entry}}}
            with (
                self.subTest(stage="readback", entry=entry),
                patch.object(api, "graphql", side_effect=[mutation, bad_readback]) as call,
            ):
                with self.assertRaisesRegex(updates.UpdateError, "readback"):
                    updates._queue_ready(
                        api, api.reader, 3, api.head, api.base, updates.MAINTENANCE
                    )
                self.assertEqual(call.call_count, 2)
        malformed_readbacks = [
            None,
            [],
            {"data": None},
            {"data": {"node": "not-a-node"}},
            {"data": {"node": {"headRefOid": None, "mergeQueueEntry": {"id": "MQE_fixture"}}}},
            {"data": {"node": {"headRefOid": "c" * 40, "mergeQueueEntry": {"id": "MQE_fixture"}}}},
            {"data": {"node": {"headRefOid": api.head, "mergeQueueEntry": {"id": "MQE_other"}}}},
            {**observed, "errors": [{"message": "fixture failure"}]},
        ]
        for response in malformed_readbacks:
            with (
                self.subTest(response=response),
                patch.object(api, "graphql", side_effect=[mutation, response]),
                self.assertRaisesRegex(updates.UpdateError, "readback"),
            ):
                updates._queue_ready(api, api.reader, 3, api.head, api.base, updates.MAINTENANCE)
        for response in (
            None,
            [],
            {"data": None},
            {"data": {"enqueuePullRequest": "not-an-object"}},
            {**mutation, "errors": [{"message": "fixture failure"}]},
        ):
            with (
                patch.object(api, "graphql", return_value=response),
                self.assertRaisesRegex(updates.UpdateError, "admission refused"),
            ):
                updates._queue_ready(api, api.reader, 3, api.head, api.base, updates.MAINTENANCE)
        with patch.object(api, "graphql", side_effect=[mutation, observed]) as call:
            self.assertTrue(
                updates._queue_ready(api, api.reader, 3, api.head, api.base, updates.MAINTENANCE)
            )
            self.assertEqual(call.call_args_list[0].args[1], {"id": "PR3", "head": api.head})
            self.assertIn("expectedHeadOid:$head", call.call_args_list[0].args[0])
            self.assertEqual(call.call_count, 2)


class CredentialTransportTests(unittest.TestCase):
    def clients(self) -> tuple[updates.GitHub, updates.CommitReader, updates.ApiBudget]:
        budget = updates.ApiBudget()
        return (
            updates.GitHub("Strukturpiloten/boxferry", "writer-sentinel", budget=budget),
            updates.CommitReader("Strukturpiloten/boxferry", "reader-sentinel", budget=budget),
            budget,
        )

    def test_fixed_reader_interface_and_aggregate_sixty_attempt_budget(self) -> None:
        api, reader, budget = self.clients()
        with patch.object(updates.urllib.request, "build_opener") as opener:
            opener.return_value.open.return_value.__enter__.return_value.read.return_value = b"{}"
            for index in range(60):
                if index % 2:
                    reader.checks("a" * 40)
                else:
                    api.request("/git/refs", "POST", {"fixture": True})
            self.assertEqual(opener.return_value.open.call_count, 60)
            self.assertEqual(budget.attempts, 60)
            for call in (lambda: reader.statuses("a" * 40), lambda: api.request("")):
                with self.assertRaisesRegex(updates.UpdateError, "bound"):
                    call()
            self.assertEqual(opener.return_value.open.call_count, 60)
            requests = [call.args[0] for call in opener.return_value.open.call_args_list]
            self.assertEqual(
                {request.get_header("Authorization") for request in requests[1::2]},
                {"Bearer reader-sentinel"},
            )
            self.assertEqual({request.get_method() for request in requests[1::2]}, {"GET"})
            self.assertEqual(
                {request.get_header("Authorization") for request in requests[::2]},
                {"Bearer writer-sentinel"},
            )
        self.assertFalse(hasattr(reader, "request"))
        self.assertFalse(hasattr(reader, "graphql"))
        for head in (
            "main",
            "a" * 39,
            "A" * 40,
            "a" * 40 + "?redirect=other",
            "../other",
            "https://other.invalid",
        ):
            with self.assertRaisesRegex(updates.UpdateError, "held head"):
                reader.checks(head)

    def test_failures_consume_shared_budget_and_redirects_never_fallback(self) -> None:
        _api, reader, budget = self.clients()
        for code in (401, 403):
            error = updates.urllib.error.HTTPError(
                "https://api.github.com/fixture", code, "reader-sentinel", Message(), None
            )
            with patch.object(updates.urllib.request, "build_opener") as opener:
                opener.return_value.open.side_effect = error
                with self.assertRaisesRegex(updates.UpdateError, str(code)):
                    reader.checks("a" * 40)
                self.assertEqual(opener.return_value.open.call_count, 1)
        self.assertEqual(budget.attempts, 2)

        def redirect(handler: updates.urllib.request.HTTPRedirectHandler) -> object:
            class Opener:
                def open(self, request: updates.urllib.request.Request, timeout: int) -> object:
                    return handler.redirect_request(
                        request,
                        BytesIO(),
                        302,
                        "fixture",
                        HTTPMessage(),
                        "https://api.github.com/repos/Other/private/statuses/a",
                    )

            return Opener()

        with patch.object(updates.urllib.request, "build_opener", side_effect=redirect):
            with self.assertRaisesRegex(updates.UpdateError, "redirect refused"):
                reader.statuses("a" * 40)
        self.assertEqual(budget.attempts, 3)
        with patch.object(updates.urllib.request, "build_opener") as opener:
            opener.return_value.open.return_value.__enter__.return_value.read.return_value = (
                b"not json reader-sentinel"
            )
            with self.assertRaisesRegex(updates.UpdateError, "malformed JSON") as raised:
                reader.checks("a" * 40)
            self.assertNotIn("reader-sentinel", str(raised.exception))
        self.assertEqual(budget.attempts, 4)


class ServiceCredentialTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.runtime = self.root / "runtime"
        self.runtime.mkdir()
        pin = json.loads((ROOT / "automation/renovate-runtime.json").read_text())["image"]
        (self.runtime / "source-image.txt").write_text(pin)
        (self.runtime / "package.json").write_text(
            json.dumps({"version": pin.split(":", 1)[1].split("@", 1)[0]})
        )
        self.environment = patch.dict(
            os.environ,
            {
                "GITHUB_REF": "refs/heads/main",
                "GITHUB_EVENT_NAME": "workflow_dispatch",
                "RENOVATE_TOKEN": "writer-sentinel",
                "BOXFERRY_FEATURE_READ_TOKEN": "reader-sentinel",
                "BOXFERRY_NODE": "/trusted/bin/node",
            },
        )
        self.environment.start()
        self.addCleanup(self.environment.stop)

    def test_missing_or_aliased_reader_fails_before_api_or_native_effects(self) -> None:
        for token in ("", "writer-sentinel", "reader\nheader"):
            with (
                patch.dict(os.environ, {"BOXFERRY_FEATURE_READ_TOKEN": token}),
                patch.object(updates.urllib.request, "build_opener") as opener,
            ):
                with self.assertRaises(updates.UpdateError):
                    updates.service("Strukturpiloten/boxferry", ROOT, self.runtime)
                opener.assert_not_called()

    def test_private_native_config_cleanup_and_both_secret_diagnostics(self) -> None:
        for failure in (False, True):
            paths = []

            def command(
                argv: tuple[str, ...],
                *,
                failure: bool = failure,
                paths: list[Path] = paths,
                **kwargs: object,
            ) -> RunResult:
                env = cast(Mapping[str, str], kwargs["environment"])
                self.assertNotIn("BOXFERRY_FEATURE_READ_TOKEN", env)
                self.assertEqual(env["RENOVATE_TOKEN"], "writer-sentinel")
                self.assertEqual(kwargs["secrets"], ("reader-sentinel",))
                path = Path(env["RENOVATE_CONFIG_FILE"])
                paths.append(path)
                self.assertEqual(stat.S_IMODE(path.parent.stat().st_mode), 0o700)
                self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
                config = json.loads(path.read_text())
                self.assertFalse(config["automerge"])
                self.assertEqual(
                    config["hostRules"],
                    [updates._native_read_rule("Strukturpiloten/boxferry", "reader-sentinel")],
                )
                r = result("failed" if failure else "success")
                return replace(r, stderr="reader-sentinel writer-sentinel")

            with (
                patch.object(updates.GitHub, "request", return_value={"default_branch": "main"}),
                patch.object(updates, "maintenance", return_value=False) as maintain,
            ):
                if failure:
                    with self.assertRaises(updates.UpdateError) as raised:
                        updates.service(
                            "Strukturpiloten/boxferry", ROOT, self.runtime, command=command
                        )
                    self.assertNotIn("reader-sentinel", str(raised.exception))
                    self.assertNotIn("writer-sentinel", str(raised.exception))
                else:
                    updates.service("Strukturpiloten/boxferry", ROOT, self.runtime, command=command)
            if failure:
                maintain.assert_not_called()
            else:
                held_api, held_reader, _automation = maintain.call_args.args
                self.assertIs(held_api.budget, held_reader._api.budget)
            self.assertEqual(len(paths), 1)
            self.assertFalse(paths[0].parent.exists())


class NativeCredentialTests(unittest.TestCase):
    def test_pinned_native_headers_redirects_failures_and_effect_authority(self) -> None:
        runtime = Path(
            os.environ.get("BOXFERRY_RENOVATE_PACKAGE", ROOT / "target/tools/renovate")
        ).resolve()
        pin = json.loads((ROOT / "automation/renovate-runtime.json").read_text())["image"]
        self.assertEqual((runtime / "source-image.txt").read_text().strip(), pin)
        with patch.dict(os.environ, {"BOXFERRY_NODE": "/trusted/bin/node"}):
            config = updates.limited_config(
                "Strukturpiloten/boxferry", Path("/trusted/source"), Path("/trusted/bin/python")
            )
        payload = {
            "pin": pin,
            "config": config,
            "rule": updates._native_read_rule("Strukturpiloten/boxferry", "ghs_fixture_reader"),
        }
        run = subprocess.run(
            [
                "node",
                "--experimental-vm-modules",
                str(ROOT / "automation/tests/devcontainer_updates_native.cjs"),
                str(runtime),
            ],
            input=json.dumps(payload),
            text=True,
            capture_output=True,
            timeout=90,
            check=False,
        )
        self.assertEqual(run.returncode, 0, (run.stdout + run.stderr)[-12000:])
        report = json.loads(run.stdout.strip().splitlines()[-1])
        self.assertTrue(report["wrongResourceNativeGreen"])
        self.assertEqual(report["nativeIntegrationEffects"], 0)
        self.assertEqual(report["optionalStatusEffects"], 0)
        self.assertTrue(report["proposalProcessing"])
        self.assertTrue(report["oldAutomergeControlReached"])
        self.assertTrue(report["ageGatesPreserved"])
        self.assertEqual(report["metadata403"], "abort")


class WorkflowCredentialTests(unittest.TestCase):
    def test_caller_callee_only_read_scopes_and_explicit_two_scope_app(self) -> None:
        import yaml

        caller = yaml.safe_load(
            (ROOT / ".github/workflows/devcontainer-feature-qualification.yml").read_text()
        )
        callee = yaml.safe_load(
            (ROOT / ".github/workflows/shared-devcontainer-updates.yml").read_text()
        )
        expected = {"contents": "read", "checks": "read", "statuses": "read"}
        self.assertEqual(caller["permissions"], expected)
        self.assertEqual(callee["permissions"], expected)
        steps = callee["jobs"]["features"]["steps"]
        token = next(step for step in steps if step.get("id") == "app-token")["with"]
        self.assertEqual(
            {key: value for key, value in token.items() if key.startswith("permission-")},
            {"permission-contents": "write", "permission-pull-requests": "write"},
        )
        self.assertEqual(token["repositories"], "${{ github.event.repository.name }}")
        self.assertEqual(token["owner"], "${{ github.repository_owner }}")
        service = steps[-1]
        self.assertEqual(
            service["env"],
            {
                "RENOVATE_TOKEN": "${{ steps.app-token.outputs.token }}",
                "BOXFERRY_FEATURE_READ_TOKEN": "${{ github.token }}",
            },
        )
        for step in steps[:-1]:
            self.assertNotIn("BOXFERRY_FEATURE_READ_TOKEN", str(step))
        self.assertNotIn(
            "schedule:",
            (ROOT / ".github/workflows/devcontainer-feature-qualification.yml").read_text(),
        )


if __name__ == "__main__":
    unittest.main()
