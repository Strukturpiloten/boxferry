"""Independent release expectations; tests never contact a registry or publish."""

from __future__ import annotations

import json
import os
import tempfile
import unittest
from copy import deepcopy
from email.message import Message
from pathlib import Path
from typing import Any
from unittest.mock import patch
from urllib.error import HTTPError

from boxferry_ci import release

SHA = "a" * 40
AUTOMATION = "b" * 40


class ReleaseTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.policy: dict[str, Any] = {
            "schema": 1,
            "repository": "Strukturpiloten/boxferry",
            "packages": ["base", "app"],
            "required_jobs": ["deterministic", "native"],
            "publication_enabled": True,
        }
        self.config = self.root / "release.json"
        self.config.write_text(json.dumps(self.policy), encoding="utf-8")
        packages = []
        for name in ("base", "app"):
            manifest = self.root / name / "Cargo.toml"
            manifest.parent.mkdir()
            manifest.write_text("", encoding="utf-8")
            packages.append(
                {
                    "id": name,
                    "name": name,
                    "version": "0.1.1",
                    "manifest_path": str(manifest),
                    "publish": None,
                    "dependencies": [],
                }
            )
        self.metadata: dict[str, Any] = {"workspace_members": ["base", "app"], "packages": packages}
        self.needs: dict[str, dict[str, Any]] = {
            name: {"result": "success", "outputs": {"candidate_sha": SHA}}
            for name in self.policy["required_jobs"]
        }

    def args(self, command: str) -> list[str]:
        return [
            command,
            "--repository-root",
            str(self.root),
            "--config",
            str(self.config),
            "--candidate-sha",
            SHA,
            "--automation-sha",
            AUTOMATION,
            "--package",
            "app",
            "--validation-sha",
            SHA,
            "--validation-result",
            "success",
        ]

    def test_complete_gate_success_and_every_non_success_fails(self) -> None:
        release.verify_gate(self.policy, self.needs, SHA)
        for status in ("failure", "cancelled", "skipped", "", None):
            with self.subTest(status=status):
                self.needs["native"]["result"] = status
                with self.assertRaises(release.ReleaseError):
                    release.verify_gate(self.policy, self.needs, SHA)

    def test_queue_semver_preserves_exact_source_pr_breaking_title(self) -> None:
        binding = release.QueueBinding(SHA, AUTOMATION, 42, "c" * 40)
        pull = {
            "number": 42,
            "state": "open",
            "base": {"sha": SHA},
            "head": {"sha": binding.source_head},
            "title": "feat(api)!: replace public type",
        }
        for title, expected in ((pull["title"], "major"), ("fix: preserve public type", "")):
            with self.subTest(title=title):
                self.assertEqual(
                    release.queue_release_type(
                        SHA,
                        AUTOMATION,
                        lambda: binding,
                        lambda _number, title=title: {**pull, "title": title},
                    ),
                    expected,
                )

    def test_queue_semver_rejects_stale_missing_and_unbound_authority(self) -> None:
        binding = release.QueueBinding(SHA, AUTOMATION, 42, "c" * 40)
        pull = {
            "number": 42,
            "state": "open",
            "base": {"sha": SHA},
            "head": {"sha": binding.source_head},
            "title": "feat!: intentional break",
        }
        for value in (
            None,
            {},
            {**pull, "state": "closed"},
            {**pull, "head": {"sha": SHA}},
            {**pull, "base": {"sha": AUTOMATION}},
            {**pull, "title": "feat!:\nbreak"},
        ):
            with self.subTest(value=value), self.assertRaises(release.ReleaseError):
                release.queue_release_type(
                    SHA, AUTOMATION, lambda: binding, lambda _number, value=value: value
                )
        for wrong in (
            release.QueueBinding(AUTOMATION, AUTOMATION, 42, binding.source_head),
            release.QueueBinding(SHA, SHA, 42, binding.source_head),
        ):
            with self.subTest(wrong=wrong), self.assertRaises(release.ReleaseError):
                release.queue_release_type(
                    SHA, AUTOMATION, lambda wrong=wrong: wrong, lambda _number: pull
                )
        identities = iter((binding, release.QueueBinding(SHA, AUTOMATION, 43, binding.source_head)))
        with self.assertRaises(release.ReleaseError):
            release.queue_release_type(
                SHA, AUTOMATION, lambda: next(identities), lambda _number: pull
            )
        with self.assertRaisesRegex(release.ReleaseError, "unsupported queue"):
            release.queue_release_type(
                SHA,
                AUTOMATION,
                lambda: (_ for _ in ()).throw(release.ReleaseError("unsupported queue")),
                lambda _number: pull,
            )

    def test_queue_semver_rejects_title_changes_between_each_authoritative_read(self) -> None:
        title = "feat(api)!: replace public type"
        binding = release.QueueBinding(SHA, AUTOMATION, 42, "c" * 40, title)
        pull = {
            "number": 42,
            "state": "open",
            "base": {"sha": SHA},
            "head": {"sha": binding.source_head},
            "title": title,
        }
        changed = {**pull, "title": "fix: retain public type"}
        with self.assertRaises(release.ReleaseError):
            release.queue_release_type(SHA, AUTOMATION, lambda: binding, lambda _number: changed)
        identities = iter(
            (
                binding,
                release.QueueBinding(SHA, AUTOMATION, 42, binding.source_head, changed["title"]),
            )
        )
        with self.assertRaises(release.ReleaseError):
            release.queue_release_type(
                SHA, AUTOMATION, lambda: next(identities), lambda _number: pull
            )
        reads = iter((pull, changed))
        with self.assertRaises(release.ReleaseError):
            release.queue_release_type(
                SHA, AUTOMATION, lambda: binding, lambda _number: next(reads)
            )

    def test_current_queue_stale_base_requires_fixed_repository_target_source_author_and_title(
        self,
    ) -> None:
        title = "feat(api)!: change public type"
        repository = "Strukturpiloten/boxferry"
        binding = release.QueueBinding(
            SHA, AUTOMATION, 42, "c" * 40, title, "main", repository, "renovate[bot]", "Bot"
        )
        pull: dict[str, Any] = {
            "number": 42,
            "state": "open",
            "title": title,
            "base": {"sha": "d" * 40, "ref": "main", "repo": {"full_name": repository}},
            "head": {"sha": binding.source_head, "repo": {"full_name": repository}},
            "user": {"login": "renovate[bot]", "type": "Bot"},
        }
        self.assertEqual(
            release.queue_release_type(SHA, AUTOMATION, lambda: binding, lambda _number: pull),
            "major",
        )
        for field in ("source", "source-repo", "target", "target-repo", "author", "title"):
            changed = deepcopy(pull)
            if field == "source":
                changed["head"]["sha"] = "f" * 40
            elif field == "source-repo":
                changed["head"]["repo"]["full_name"] = "attacker/fork"
            elif field == "target":
                changed["base"]["ref"] = "another-target"
            elif field == "target-repo":
                changed["base"]["repo"]["full_name"] = "attacker/fork"
            elif field == "author":
                changed["user"]["login"] = "different-bot"
            else:
                changed["title"] = "fix: altered intent"
            reads = iter((pull, changed))
            with self.subTest(field=field), self.assertRaises(release.ReleaseError):
                release.queue_release_type(
                    SHA, AUTOMATION, lambda: binding, lambda _number, reads=reads: next(reads)
                )

    def test_missing_extra_and_wrong_candidate_prerequisites_fail(self) -> None:
        for needs in (
            {"deterministic": self.needs["deterministic"]},
            self.needs | {"unknown": self.needs["native"]},
            self.needs
            | {"native": {"result": "success", "outputs": {"candidate_sha": AUTOMATION}}},
            self.needs | {"native": {"result": "success"}},
        ):
            with self.subTest(needs=needs), self.assertRaises(release.ReleaseError):
                release.verify_gate(self.policy, needs, SHA)

    def test_all_dependency_kinds_require_predecessors(self) -> None:
        for kind in (None, "normal", "build", "dev"):
            with self.subTest(kind=kind):
                self.metadata["packages"][1]["dependencies"] = [
                    {"name": "base", "path": str(self.root / "base"), "kind": kind}
                ]
                _, dependencies = release.publication_inventory(
                    self.metadata, self.policy, self.root
                )
                self.assertEqual(dependencies["app"], {"base"})
                bad_order = self.policy | {"packages": ["app", "base"]}
                with self.assertRaises(release.ReleaseError):
                    release.publication_inventory(self.metadata, bad_order, self.root)

    def test_incomplete_inventory_and_mixed_version_fail(self) -> None:
        with self.assertRaises(release.ReleaseError):
            release.publication_inventory(
                self.metadata, self.policy | {"packages": ["app"]}, self.root
            )
        self.metadata["packages"][1]["version"] = "0.1.2"
        with self.assertRaises(release.ReleaseError):
            release.publication_inventory(self.metadata, self.policy, self.root)

    def test_invalid_policy_unknown_fields_and_duplicate_json_fail(self) -> None:
        for value in (
            self.policy | {"unknown": True},
            self.policy | {"schema": True},
            self.policy | {"required_jobs": []},
        ):
            self.config.write_text(json.dumps(value), encoding="utf-8")
            with self.assertRaises(release.ReleaseError):
                release.load_policy(self.config, "Strukturpiloten/boxferry")
        with self.assertRaises(release.ReleaseError):
            release.read_json('{"schema":1,"schema":1}')

    def test_validation_only_has_no_registry_or_mutating_command(self) -> None:
        with (
            patch.dict(os.environ, {"GITHUB_REPOSITORY": "Strukturpiloten/boxferry"}),
            patch.object(release, "verify_bindings"),
            patch.object(
                release.subprocess, "check_output", return_value=json.dumps(self.metadata)
            ),
            patch.object(release, "registry_status") as registry,
            patch.object(release.subprocess, "run") as mutate,
        ):
            self.assertEqual(release.main(self.args("prepare-crate")), 0)
            registry.assert_not_called()
            mutate.assert_not_called()
            with self.assertRaises(SystemExit):
                release.main(self.args("publish-crate"))
            mutate.assert_not_called()

    def test_missing_release_gate_and_wrong_bootstrap_version_fail(self) -> None:
        with (
            patch.dict(os.environ, {"GITHUB_REPOSITORY": "Strukturpiloten/boxferry"}),
            patch.object(release, "verify_bindings"),
            patch.object(
                release.subprocess, "check_output", return_value=json.dumps(self.metadata)
            ),
            patch.object(release.subprocess, "run") as mutate,
        ):
            for suffix in (
                ["--validation-result", "skipped"],
                ["--validation-sha", AUTOMATION],
                ["--bootstrap", "true"],
            ):
                with self.subTest(suffix=suffix), self.assertRaises(SystemExit):
                    release.main(self.args("prepare-crate") + suffix)
            mutate.assert_not_called()

    def test_publication_context_is_explicit_and_validation_only_by_default(self) -> None:
        with patch.dict(
            os.environ,
            {
                "GITHUB_EVENT_NAME": "workflow_dispatch",
                "GITHUB_SHA": SHA,
                "GITHUB_REF": "refs/heads/main",
                "BOXFERRY_DEFAULT_BRANCH": "main",
            },
            clear=True,
        ):
            release.verify_publication_context(self.policy, SHA)
            with self.assertRaises(release.ReleaseError):
                release.verify_publication_context(
                    self.policy | {"publication_enabled": False}, SHA
                )
            with (
                patch.dict(os.environ, {"GITHUB_EVENT_NAME": "push"}),
                self.assertRaises(release.ReleaseError),
            ):
                release.verify_publication_context(self.policy, SHA)

    def test_full_candidate_and_automation_sha_bindings_are_independent(self) -> None:
        with (
            patch.dict(os.environ, {"GITHUB_SHA": SHA}),
            patch.object(release.subprocess, "check_output", side_effect=[SHA, AUTOMATION]),
        ):
            release.verify_bindings(self.root, SHA, AUTOMATION)
        for candidate, automation in ((SHA[:12], AUTOMATION), (SHA, "main"), (SHA, "g" * 40)):
            with self.subTest(candidate=candidate, automation=automation):
                with self.assertRaises(release.ReleaseError):
                    release.verify_bindings(self.root, candidate, automation)
        with (
            patch.dict(os.environ, {"GITHUB_SHA": SHA}),
            patch.object(release.subprocess, "check_output", side_effect=[SHA, SHA]),
            self.assertRaises(release.ReleaseError),
        ):
            release.verify_bindings(self.root, SHA, AUTOMATION)

    def test_only_canonical_registry_absence_is_accepted(self) -> None:
        url = "https://crates.io/api/v1/crates/app/0.1.1"
        missing = HTTPError(url, 404, "not found", Message(), None)
        with patch.object(release, "urlopen", side_effect=missing):
            self.assertEqual(release.registry_status("app", "0.1.1"), 404)
        redirected = HTTPError("https://example.invalid/app", 404, "not found", Message(), None)
        with (
            patch.object(release, "urlopen", side_effect=redirected),
            self.assertRaises(release.ReleaseError),
        ):
            release.registry_status("app", "0.1.1")
        unavailable = HTTPError(url, 503, "unavailable", Message(), None)
        with (
            patch.object(release, "urlopen", side_effect=unavailable) as request,
            patch.object(release.time, "sleep"),
            self.assertRaises(release.ReleaseError),
        ):
            release.registry_status("app", "0.1.1")
        self.assertEqual(request.call_count, 4)

    def test_already_published_is_idempotent(self) -> None:
        with (
            patch.object(release, "registry_status", return_value=200),
            patch.object(release.subprocess, "run") as mutate,
        ):
            release.publish_crate(self.root, "app", "0.1.1")
            mutate.assert_not_called()

    def test_new_publication_waits_for_visibility_with_locked_cargo(self) -> None:
        with (
            patch.object(release, "registry_status", side_effect=[404, 404, 200]),
            patch.object(release.subprocess, "run") as mutate,
            patch.object(release.time, "sleep") as sleep,
        ):
            release.publish_crate(self.root, "app", "0.1.1")
            mutate.assert_called_once_with(
                ["cargo", "publish", "--locked", "--package", "app"],
                cwd=self.root,
                check=True,
                timeout=1800,
            )
            sleep.assert_called_once_with(5)

    def test_visibility_timeout_and_registry_failure_propagate(self) -> None:
        with (
            patch.object(release, "registry_status", return_value=404),
            patch.object(release.subprocess, "run"),
            patch.object(release.time, "sleep"),
        ):
            with self.assertRaises(release.ReleaseError):
                release.publish_crate(self.root, "app", "0.1.1")
        with (
            patch.object(release, "registry_status", side_effect=release.ReleaseError("503")),
            patch.object(release.subprocess, "run") as mutate,
        ):
            with self.assertRaises(release.ReleaseError):
                release.publish_crate(self.root, "app", "0.1.1")
            mutate.assert_not_called()


if __name__ == "__main__":
    unittest.main()
