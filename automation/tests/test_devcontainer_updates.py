"""Official-generator isolation, manager partition and fixed-branch regressions."""

from __future__ import annotations

import json
import os
import tempfile
import unittest
from collections.abc import Mapping
from email.message import Message
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
        self.assertNotIn("NODE_OPTIONS", environment)
        self.assertNotEqual(environment["DOCKER_CONFIG"], "personal-credentials")
        (root / updates.LOCK).write_text('{"features":{"root":{},"transitive":{}}}')
        return result()

    def test_scratch_full_graph_validation_and_idempotence(self) -> None:
        self.assertTrue(self.generate(self.command))
        self.assertEqual(len(self.validated), 1)
        self.assertFalse(self.generate(self.command))
        self.assertEqual(len(self.validated), 2)

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


class FakeGitHub(updates.GitHub):
    repository = "Strukturpiloten/boxferry"

    def __init__(self) -> None:
        self.calls: list[tuple[str, str, object]] = []
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
        if "/check-runs?" in path:
            return {
                "total_count": 1,
                "check_runs": [
                    {
                        "name": "Required CI",
                        "status": "completed",
                        "conclusion": "success" if self.ready else "failure",
                    }
                ],
            }
        if "/status?" in path:
            return {
                "total_count": 1,
                "statuses": [
                    {
                        "context": "Dependency eligibility",
                        "state": "success" if self.ready else "pending",
                    }
                ],
            }
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
        self.assertFalse(updates.maintenance(api, ROOT, lambda *_: False))
        self.assertFalse(any(method != "GET" for _, method, _ in api.calls))

    def test_digest_or_transitive_only_graph_change_publishes_lock_only(self) -> None:
        api = FakeGitHub()

        def changed(root: Path, automation: Path) -> bool:
            (root / updates.LOCK).write_text('{"features":{"new-transitive":{}}}')
            return True

        self.assertTrue(updates.maintenance(api, ROOT, changed))
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

    def test_base_race_does_not_publish(self) -> None:
        api = FakeGitHub()
        api.race = True
        with self.assertRaisesRegex(updates.UpdateError, "moved"):
            updates.maintenance(api, ROOT, lambda *_: True)
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
            updates.maintenance(api, ROOT, lambda *_: True)
        self.assertFalse(any(method != "GET" for _, method, _ in api.calls))

    def test_owned_stale_maintenance_branch_rebases_without_force(self) -> None:
        api = FakeGitHub()
        api.pulls = [api.owned()]
        api.comparison = "diverged"
        self.assertTrue(updates.maintenance(api, ROOT, lambda *_: True))
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

        self.assertTrue(updates.maintenance(api, ROOT, changed))
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
        self.assertTrue(updates.maintenance(api, ROOT, lambda *_: True))
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
                updates.maintenance(api, ROOT, lambda *_: True)
            self.assertFalse(any(method != "GET" for _, method, _ in api.calls))

    def test_same_proposal_is_idempotent_and_does_not_restart_ci(self) -> None:
        api = FakeGitHub()
        api.pulls = [api.owned()]
        self.assertFalse(updates.maintenance(api, ROOT, lambda *_: True))
        self.assertFalse(any(method != "GET" for _, method, _ in api.calls))

    def test_human_commit_on_bot_branch_is_refused(self) -> None:
        api = FakeGitHub()
        api.pulls = [api.owned()]
        api.owned_committer = "human"
        with self.assertRaisesRegex(updates.UpdateError, "owned"):
            updates.maintenance(api, ROOT, lambda *_: True)
        self.assertFalse(any(method != "GET" for _, method, _ in api.calls))

    def test_redundant_own_proposal_is_closed_when_main_already_has_graph(self) -> None:
        api = FakeGitHub()
        api.pulls = [api.owned()]
        self.assertFalse(updates.maintenance(api, ROOT, lambda *_: False))
        self.assertIn(("/pulls/3", "PATCH", {"state": "closed"}), api.calls)

    def test_versioned_stream_precedes_verified_maintenance(self) -> None:
        api = FakeGitHub()
        api.versioned = [api.owned(updates.BRANCH)]
        api.pulls = [api.owned()]
        self.assertFalse(updates.maintenance(api, ROOT, lambda *_: False))
        self.assertEqual(sum(method == "PATCH" for _, method, _ in api.calls), 1)
        self.assertIn(("/pulls/3", "PATCH", {"state": "closed"}), api.calls)


class QueueTests(unittest.TestCase):
    def test_pending_or_failed_required_results_do_not_enqueue(self) -> None:
        api = FakeGitHub()
        self.assertFalse(updates._queue_ready(api, 3, api.head, api.base, updates.MAINTENANCE))
        self.assertFalse(any(method != "GET" for _, method, _ in api.calls))

    def test_protected_queue_admission_uses_exact_head_and_reads_back(self) -> None:
        api = FakeGitHub()
        api.ready = True
        self.assertTrue(updates._queue_ready(api, 3, api.head, api.base, updates.MAINTENANCE))
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
            updates._queue_ready(api, 3, api.head, api.base, updates.MAINTENANCE)
        api = FakeGitHub()
        api.ready = True
        api.race = True
        with self.assertRaisesRegex(updates.UpdateError, "moved"):
            updates._queue_ready(api, 3, api.head, api.base, updates.MAINTENANCE)
        self.assertFalse(any(method != "GET" for _, method, _ in api.calls))


if __name__ == "__main__":
    unittest.main()
