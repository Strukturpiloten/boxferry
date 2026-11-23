"""Independent release expectations; tests never contact a registry or publish."""

from __future__ import annotations

import json
import os
import subprocess
import sys
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
        self.needs["deterministic"]["outputs"]["published_api_candidate_sha"] = SHA

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
        self.assertEqual(release.verify_gate(self.policy, self.needs, SHA), SHA)
        for name in self.policy["required_jobs"]:
            for status in ("failure", "cancelled", "skipped", "", None):
                needs = deepcopy(self.needs)
                needs[name]["result"] = status
                with (
                    self.subTest(name=name, status=status),
                    self.assertRaises(release.ReleaseError),
                ):
                    release.verify_gate(self.policy, needs, SHA)

    def test_published_evidence_can_come_from_exactly_one_configured_prerequisite(self) -> None:
        proof = self.needs["deterministic"]["outputs"].pop("published_api_candidate_sha")
        self.needs["native"]["outputs"]["published_api_candidate_sha"] = proof
        self.assertEqual(release.verify_gate(self.policy, self.needs, SHA), SHA)

    def test_single_published_proof_preserves_current_caller_prerequisite_shapes(self) -> None:
        for jobs, api_job in (
            (["validate"], "validate"),
            (["candidate", "native-conformance", "dependencies", "api"], "api"),
        ):
            needs: dict[str, dict[str, Any]] = {
                name: {"result": "success", "outputs": {"candidate_sha": SHA}} for name in jobs
            }
            needs[api_job]["outputs"]["published_api_candidate_sha"] = SHA
            with self.subTest(jobs=jobs):
                self.assertEqual(
                    release.verify_gate(self.policy | {"required_jobs": jobs}, needs, SHA), SHA
                )

    def test_malformed_tested_candidate_cannot_be_accepted_by_either_output(self) -> None:
        for candidate in ("", SHA[:12], "g" * 40, SHA.upper(), SHA + "\n"):
            needs = deepcopy(self.needs)
            for prerequisite in needs.values():
                prerequisite["outputs"]["candidate_sha"] = candidate
            needs["deterministic"]["outputs"]["published_api_candidate_sha"] = candidate
            with self.subTest(candidate=candidate), self.assertRaises(release.ReleaseError):
                release.verify_gate(self.policy, needs, candidate)

    def test_generic_candidate_and_development_evidence_cannot_replace_published_proof(
        self,
    ) -> None:
        self.needs["deterministic"]["outputs"].pop("published_api_candidate_sha")
        for other_evidence in (
            {},
            {"api_purpose": "published"},
            {"api_purpose": "development", "development_api_candidate_sha": SHA},
        ):
            needs = deepcopy(self.needs)
            needs["deterministic"]["outputs"].update(other_evidence)
            with (
                self.subTest(other_evidence=other_evidence),
                self.assertRaises(release.ReleaseError),
            ):
                release.verify_gate(self.policy, needs, SHA)

    def test_explicit_published_evidence_must_be_one_full_exact_candidate_sha(self) -> None:
        for value in (
            None,
            "",
            False,
            1,
            [],
            {},
            SHA[:12],
            "g" * 40,
            SHA.upper(),
            SHA + "\n",
            AUTOMATION,
        ):
            needs = deepcopy(self.needs)
            needs["deterministic"]["outputs"]["published_api_candidate_sha"] = value
            with self.subTest(value=value), self.assertRaises(release.ReleaseError):
                release.verify_gate(self.policy, needs, SHA)

    def test_repeated_contradictory_and_empty_additional_published_evidence_fail(self) -> None:
        for value in (SHA, AUTOMATION, "", None):
            needs = deepcopy(self.needs)
            needs["native"]["outputs"]["published_api_candidate_sha"] = value
            with self.subTest(value=value), self.assertRaises(release.ReleaseError):
                release.verify_gate(self.policy, needs, SHA)

    def test_real_gate_cli_writes_both_outputs_only_after_all_evidence_succeeds(self) -> None:
        # Stub only immutable Git identity reads. The CLI, JSON reader, gate and
        # GITHUB_OUTPUT writer run in a real child process with no publication command.
        automation_root = Path(release.__file__).resolve().parents[3]
        git = self.root / "git"
        git.write_text(
            f"#!{sys.executable}\n"
            "import sys\n"
            f"identities = { {str(self.root): SHA, str(automation_root): AUTOMATION}!r}\n"
            "args = sys.argv[1:]\n"
            "if len(args) != 4 or args[0] != '-C' or args[2:] != ['rev-parse', 'HEAD']:\n"
            "    raise SystemExit(99)\n"
            "print(identities[args[1]])\n",
            encoding="utf-8",
        )
        git.chmod(0o755)
        output = self.root / "github-output"
        env = os.environ | {
            "PATH": str(self.root) + os.pathsep + os.environ.get("PATH", ""),
            "PYTHONPATH": str(automation_root / "automation/src"),
            "GITHUB_REPOSITORY": "Strukturpiloten/boxferry",
            "GITHUB_SHA": SHA,
            "GITHUB_OUTPUT": str(output),
        }
        cases: list[tuple[str, dict[str, Any] | None]] = [("success", self.needs)]
        for label in ("generic-only", "empty", "stale", "multiple", "failed", "missing"):
            needs = deepcopy(self.needs)
            if label == "generic-only":
                needs["deterministic"]["outputs"].pop("published_api_candidate_sha")
            elif label in {"empty", "stale"}:
                needs["deterministic"]["outputs"]["published_api_candidate_sha"] = (
                    "" if label == "empty" else AUTOMATION
                )
            elif label == "multiple":
                needs["native"]["outputs"]["published_api_candidate_sha"] = SHA
            elif label == "failed":
                needs["native"]["result"] = "failure"
            else:
                needs.pop("native")
            cases.append((label, needs))
        cases.append(("no-needs", None))
        for label, needs in cases:
            with self.subTest(label=label):
                output.write_text("existing=retained\n", encoding="utf-8")
                args = self.args("gate")
                if needs is not None:
                    args += ["--needs-json", json.dumps(needs)]
                child = subprocess.run(
                    [sys.executable, "-m", "boxferry_ci.release", *args],
                    env=env,
                    capture_output=True,
                    text=True,
                    check=False,
                    timeout=30,
                )
                if label == "success":
                    self.assertEqual(child.returncode, 0, child.stderr)
                    self.assertEqual(
                        output.read_text(),
                        f"existing=retained\ncandidate_sha={SHA}\npublished_api_candidate_sha={SHA}\n",
                    )
                else:
                    self.assertNotEqual(child.returncode, 0)
                    self.assertIn("release contract failed:", child.stderr)
                    self.assertEqual(output.read_text(), "existing=retained\n")

    def test_shared_workflow_forwards_the_validated_cli_published_api_output(self) -> None:
        root = Path(__file__).resolve().parents[2]
        workflow = (root / ".github/workflows/shared-release-validation.yml").read_text()
        self.assertIn("value: ${{ jobs.validate.outputs.published_api_candidate_sha }}", workflow)
        self.assertIn(
            "published_api_candidate_sha: ${{ steps.gate.outputs.published_api_candidate_sha }}",
            workflow,
        )
        steps = workflow.split("      - name: ")
        gate_step = next(step for step in steps if "python -m boxferry_ci.release gate" in step)
        self.assertIn("        id: gate\n", gate_step)
        self.assertNotIn("continue-on-error", workflow)
        self.assertNotIn("published_api_candidate_sha: ${{ inputs.candidate_sha }}", workflow)
        self.assertIn("candidate_sha: ${{ steps.result.outputs.candidate_sha }}", workflow)

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
        with self.assertRaises(release.ReleaseError):
            release.read_json(
                '{"published_api_candidate_sha":"'
                + SHA
                + '","published_api_candidate_sha":"'
                + SHA
                + '"}'
            )

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


class PreparationBranchTests(unittest.TestCase):
    """Actual Git in owned scratch repositories; no release-plz, network or publication."""

    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory(prefix="release-preparation-branch-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name) / "source"
        self.root.mkdir()
        self.git("init", "--quiet", "--initial-branch=main", "--template=")
        self.git("config", "user.email", "fixture@example.invalid")
        self.git("config", "user.name", "Release branch fixture")
        (self.root / "README.md").write_text("validated source\n")
        self.git("add", "README.md")
        self.git("commit", "--quiet", "-m", "chore: owned fixture")
        self.candidate = self.git("rev-parse", "HEAD")
        self.context = {
            "GITHUB_REPOSITORY": "Strukturpiloten/podman-lens",
            "GITHUB_EVENT_NAME": "push",
            "GITHUB_REF": "refs/heads/main",
            "GITHUB_SHA": self.candidate,
        }
        self.environment = patch.dict(os.environ, self.context)
        self.environment.start()
        self.addCleanup(self.environment.stop)

    def git(self, *arguments: str) -> str:
        from boxferry_ci.bootstrap import git_environment

        result = subprocess.run(
            [
                "git",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgSign=false",
                "-C",
                str(self.root),
                *arguments,
            ],
            env=git_environment(),
            capture_output=True,
            text=True,
            check=True,
            timeout=10,
        )
        return result.stdout.strip()

    def snapshot(self) -> tuple[str, str, str, str, bytes]:
        return (
            self.git("rev-parse", "HEAD"),
            self.git("rev-parse", "--symbolic-full-name", "HEAD"),
            self.git("for-each-ref", "--format=%(refname) %(objectname)"),
            self.git("status", "--porcelain=v1", "--untracked-files=all"),
            (self.root / "README.md").read_bytes(),
        )

    def detach(self, *, remove_default: bool = False) -> None:
        self.git("checkout", "--quiet", "--detach", self.candidate)
        if remove_default:
            self.git("branch", "-D", "main")

    def other_commit(self) -> str:
        self.git("checkout", "--quiet", "-b", "other")
        (self.root / "README.md").write_text("other source\n")
        self.git("add", "README.md")
        self.git("commit", "--quiet", "-m", "chore: different candidate")
        return self.git("rev-parse", "HEAD")

    def assert_attached_candidate(self) -> None:
        self.assertEqual(self.git("symbolic-ref", "HEAD"), "refs/heads/main")
        self.assertEqual(self.git("rev-parse", "HEAD"), self.candidate)
        self.assertEqual(self.git("rev-parse", "refs/heads/main"), self.candidate)
        self.assertEqual(self.git("status", "--porcelain=v1", "--untracked-files=all"), "")
        self.assertEqual((self.root / "README.md").read_text(), "validated source\n")

    def test_detached_candidate_creates_absent_branch_without_remote_tracking(self) -> None:
        self.detach(remove_default=True)
        self.assertEqual(self.git("for-each-ref", "refs/remotes/"), "")
        release.prepare_branch(self.root, self.candidate, "main")
        self.assert_attached_candidate()

    def test_detached_candidate_reuses_matching_existing_branch(self) -> None:
        self.detach()
        self.git("update-ref", "refs/remotes/origin/main", self.candidate)
        with patch.dict(os.environ, {"GITHUB_EVENT_NAME": "workflow_dispatch"}):
            release.prepare_branch(self.root, self.candidate, "main")
        self.assert_attached_candidate()

    def test_nested_default_branch_attaches_the_exact_candidate(self) -> None:
        self.detach(remove_default=True)
        with patch.dict(os.environ, {"GITHUB_REF": "refs/heads/releases/stable"}):
            release.prepare_branch(self.root, self.candidate, "releases/stable")
        self.assertEqual(self.git("symbolic-ref", "HEAD"), "refs/heads/releases/stable")
        self.assertEqual(self.git("rev-parse", "HEAD"), self.candidate)
        self.assertEqual(self.git("status", "--porcelain=v1"), "")

    def test_invalid_git_branch_with_matching_event_ref_rejects_without_mutation(self) -> None:
        for branch in ("HEAD", "bad..name", "main\n", "@{-1}"):
            with (
                self.subTest(branch=branch),
                patch.dict(os.environ, {"GITHUB_REF": f"refs/heads/{branch}"}),
            ):
                before = self.snapshot()
                with self.assertRaisesRegex(release.ReleaseError, "Git operation failed"):
                    release.prepare_branch(self.root, self.candidate, branch)
                self.assertEqual(self.snapshot(), before)

    def test_already_correct_attached_branch_is_idempotent_without_metadata_writes(self) -> None:
        before = self.snapshot()
        with patch.object(release, "run_git", wraps=release.run_git) as commands:
            release.prepare_branch(self.root, self.candidate, "main")
            release.prepare_branch(self.root, self.candidate, "main")
        self.assertEqual(self.snapshot(), before)
        self.assertTrue(commands.call_args_list)
        for call in commands.call_args_list:
            self.assertNotIn(
                call.args[1], {"update-ref", "symbolic-ref", "checkout", "reset", "fetch", "push"}
            )

    def test_conflicting_local_default_branch_is_never_rewritten(self) -> None:
        other = self.other_commit()
        self.git("update-ref", "refs/heads/main", other)
        self.detach()
        before = self.snapshot()
        with self.assertRaisesRegex(release.ReleaseError, "moved or conflicts"):
            release.prepare_branch(self.root, self.candidate, "main")
        self.assertEqual(self.snapshot(), before)

    def test_existing_moved_tracking_ref_rejects_without_fetching(self) -> None:
        other = self.other_commit()
        self.git("update-ref", "refs/remotes/origin/main", other)
        self.detach()
        before = self.snapshot()
        with self.assertRaisesRegex(release.ReleaseError, "moved or conflicts"):
            release.prepare_branch(self.root, self.candidate, "main")
        self.assertEqual(self.snapshot(), before)

    def test_wrong_attached_branch_and_wrong_head_reject_without_mutation(self) -> None:
        self.git("checkout", "--quiet", "-b", "issue")
        before = self.snapshot()
        with self.assertRaisesRegex(release.ReleaseError, "another branch"):
            release.prepare_branch(self.root, self.candidate, "main")
        self.assertEqual(self.snapshot(), before)
        self.git("checkout", "--quiet", "main")
        (self.root / "README.md").write_text("moved HEAD\n")
        self.git("add", "README.md")
        self.git("commit", "--quiet", "-m", "chore: moved HEAD")
        before = self.snapshot()
        with self.assertRaisesRegex(release.ReleaseError, "moved from the validated candidate"):
            release.prepare_branch(self.root, self.candidate, "main")
        self.assertEqual(self.snapshot(), before)

    def test_dirty_tracked_index_and_untracked_source_reject_without_mutation(self) -> None:
        for kind in ("tracked", "staged", "untracked"):
            with self.subTest(kind=kind):
                path = self.root / ("new.txt" if kind == "untracked" else "README.md")
                path.write_text("dirty source\n")
                if kind == "staged":
                    self.git("add", "README.md")
                before = self.snapshot()
                with self.assertRaisesRegex(release.ReleaseError, "clean source and index"):
                    release.prepare_branch(self.root, self.candidate, "main")
                self.assertEqual(self.snapshot(), before)
                if kind == "untracked":
                    path.unlink()
                else:
                    path.write_text("validated source\n")
                    self.git("add", "README.md")

    def test_invalid_candidate_branch_and_caller_context_reject_before_mutation(self) -> None:
        for candidate, branch in (
            (self.candidate[:12], "main"),
            ("g" * 40, "main"),
            (self.candidate, ""),
            (self.candidate, "-option"),
            (self.candidate, "../main"),
        ):
            with self.subTest(candidate=candidate, branch=branch):
                before = self.snapshot()
                with self.assertRaises(release.ReleaseError):
                    release.prepare_branch(self.root, candidate, branch)
                self.assertEqual(self.snapshot(), before)
        for key, value in (
            ("GITHUB_REPOSITORY", "unreviewed/repository"),
            ("GITHUB_EVENT_NAME", "pull_request"),
            ("GITHUB_REF", "refs/heads/issue"),
            ("GITHUB_SHA", "f" * 40),
        ):
            with self.subTest(key=key), patch.dict(os.environ, {key: value}):
                before = self.snapshot()
                with self.assertRaisesRegex(release.ReleaseError, "exact default-branch event"):
                    release.prepare_branch(self.root, self.candidate, "main")
                self.assertEqual(self.snapshot(), before)

    def test_atomic_branch_creation_rejects_a_concurrent_conflict(self) -> None:
        other = self.other_commit()
        self.detach(remove_default=True)
        original = release._attach_preparation_head

        def concurrent_create(
            root: Path, candidate: str, branch: str, tracking: str, refs: dict[str, str]
        ) -> None:
            self.git("update-ref", "refs/heads/main", other)
            original(root, candidate, branch, tracking, refs)

        with patch.object(release, "_attach_preparation_head", side_effect=concurrent_create):
            with self.assertRaisesRegex(release.ReleaseError, "Git symref transaction failed"):
                release.prepare_branch(self.root, self.candidate, "main")
        self.assertEqual(self.git("rev-parse", "HEAD"), self.candidate)
        self.assertEqual(self.git("rev-parse", "--symbolic-full-name", "HEAD"), "HEAD")
        self.assertEqual(self.git("rev-parse", "refs/heads/main"), other)

    def test_post_attachment_rechecks_candidate_and_does_not_undo_concurrent_ref(self) -> None:
        other = self.other_commit()
        self.detach()
        original = release._attach_preparation_head

        def move_after_attachment(
            root: Path, candidate: str, branch: str, tracking: str, refs: dict[str, str]
        ) -> None:
            original(root, candidate, branch, tracking, refs)
            self.git("update-ref", "refs/heads/main", other)

        with patch.object(release, "_attach_preparation_head", side_effect=move_after_attachment):
            with self.assertRaisesRegex(release.ReleaseError, "moved from the validated candidate"):
                release.prepare_branch(self.root, self.candidate, "main")
        self.assertEqual(self.git("rev-parse", "refs/heads/main"), other)

    def test_post_attachment_rechecks_source_without_discarding_concurrent_edits(self) -> None:
        self.detach()
        original = release._attach_preparation_head

        def dirty_after_attachment(
            root: Path, candidate: str, branch: str, tracking: str, refs: dict[str, str]
        ) -> None:
            original(root, candidate, branch, tracking, refs)
            (self.root / "README.md").write_text("concurrent source edit\n")

        with patch.object(release, "_attach_preparation_head", side_effect=dirty_after_attachment):
            with self.assertRaisesRegex(release.ReleaseError, "clean source and index"):
                release.prepare_branch(self.root, self.candidate, "main")
        self.assertEqual((self.root / "README.md").read_text(), "concurrent source edit\n")
        self.assertEqual(self.git("rev-parse", "HEAD"), self.candidate)

    def test_git_command_failure_propagates_without_attachment(self) -> None:
        self.detach(remove_default=True)
        before = self.snapshot()
        original = release._preparation_exchange
        processes = []

        def unsupported_command(
            process: subprocess.Popen[bytes], payload: bytes, reply: bytes, deadline: float
        ) -> None:
            processes.append(process)
            original(
                process, payload.replace(b"symref-update", b"unsupported-command"), reply, deadline
            )

        with patch.object(release, "_preparation_exchange", side_effect=unsupported_command):
            with self.assertRaisesRegex(release.ReleaseError, "failed or is unsupported"):
                release.prepare_branch(self.root, self.candidate, "main")
        self.assertTrue(processes)
        self.assertTrue(all(process.poll() is not None for process in processes))
        self.assertEqual(self.snapshot(), before)

    def test_detached_head_move_before_attachment_is_never_rewound(self) -> None:
        other = self.other_commit()
        self.detach()
        original = release._attach_preparation_head

        def move_head(
            root: Path, candidate: str, branch: str, tracking: str, refs: dict[str, str]
        ) -> None:
            # Metadata-only HEAD move leaves the A worktree; unconditional attach would hide it.
            self.git("update-ref", "--no-deref", "HEAD", other)
            original(root, candidate, branch, tracking, refs)

        with patch.object(release, "_attach_preparation_head", side_effect=move_head):
            with self.assertRaisesRegex(release.ReleaseError, "Git symref transaction failed"):
                release.prepare_branch(self.root, self.candidate, "main")
        self.assertEqual(self.git("rev-parse", "HEAD"), other)
        self.assertEqual(self.git("rev-parse", "--symbolic-full-name", "HEAD"), "HEAD")
        self.assertEqual((self.root / "README.md").read_text(), "validated source\n")

    def test_foreign_symbolic_head_at_same_oid_is_never_overwritten_and_cli_fails(self) -> None:
        self.detach()
        self.git("branch", "foreign", self.candidate)
        original = release._attach_preparation_head

        def move_head(
            root: Path, candidate: str, branch: str, tracking: str, refs: dict[str, str]
        ) -> None:
            self.git("symbolic-ref", "HEAD", "refs/heads/foreign")
            original(root, candidate, branch, tracking, refs)

        argv = [
            "prepare-branch",
            "--repository-root",
            str(self.root),
            "--candidate-sha",
            self.candidate,
            "--automation-sha",
            AUTOMATION,
            "--default-branch",
            "main",
        ]
        with (
            patch.object(release, "_attach_preparation_head", side_effect=move_head),
            patch.object(release, "verify_bindings") as bindings,
            self.assertRaises(SystemExit) as failed,
        ):
            release.main(argv)
        self.assertEqual(failed.exception.code, 1)
        self.assertEqual(bindings.call_count, 1)
        self.assertEqual(self.git("symbolic-ref", "--no-recurse", "HEAD"), "refs/heads/foreign")
        self.assertEqual(self.git("rev-parse", "HEAD"), self.candidate)
        self.assertEqual(self.git("status", "--porcelain=v1"), "")

    def test_prepared_transaction_aborts_and_reaps_on_keyboard_or_term_cancellation(self) -> None:
        import signal

        self.detach(remove_default=True)
        before = self.snapshot()
        handler = signal.getsignal(signal.SIGTERM)
        original = release._preparation_exchange
        for kind in ("keyboard", "term"):
            processes = []

            def cancel_prepared(
                process: subprocess.Popen[bytes],
                payload: bytes,
                reply: bytes,
                deadline: float,
                cancellation: str = kind,
                observed: list[subprocess.Popen[bytes]] = processes,
            ) -> None:
                observed.append(process)
                original(process, payload, reply, deadline)
                if reply == b"prepare: ok\n":
                    if cancellation == "keyboard":
                        raise KeyboardInterrupt
                    signal.raise_signal(signal.SIGTERM)

            expected = KeyboardInterrupt if kind == "keyboard" else release.ReleaseError
            with (
                self.subTest(kind=kind),
                patch.object(release, "_preparation_exchange", side_effect=cancel_prepared),
                self.assertRaises(expected),
            ):
                release.prepare_branch(self.root, self.candidate, "main")
            self.assertTrue(processes)
            self.assertTrue(all(process.poll() is not None for process in processes))
            self.assertEqual(self.snapshot(), before)
            self.assertEqual(signal.getsignal(signal.SIGTERM), handler)
            # A fresh native transaction must succeed: cancellation released Git's locks.
            release.prepare_branch(self.root, self.candidate, "main")
            self.assert_attached_candidate()
            self.detach(remove_default=True)
            self.assertEqual(self.snapshot(), before)

    def test_transaction_deadline_aborts_reaps_and_releases_locks(self) -> None:
        self.detach(remove_default=True)
        before = self.snapshot()
        original = release._preparation_exchange
        processes = []

        def expired_preparation(
            process: subprocess.Popen[bytes], payload: bytes, reply: bytes, deadline: float
        ) -> None:
            processes.append(process)
            if reply == b"prepare: ok\n":
                deadline = release.time.monotonic() - 1
            original(process, payload, reply, deadline)

        with patch.object(release, "_preparation_exchange", side_effect=expired_preparation):
            with self.assertRaisesRegex(release.ReleaseError, "timed out"):
                release.prepare_branch(self.root, self.candidate, "main")
        self.assertTrue(processes)
        self.assertTrue(all(process.poll() is not None for process in processes))
        self.assertEqual(self.snapshot(), before)
        release.prepare_branch(self.root, self.candidate, "main")
        self.assert_attached_candidate()

    def test_cli_preserves_candidate_automation_pre_and_post_bindings(self) -> None:
        argv = [
            "prepare-branch",
            "--repository-root",
            str(self.root),
            "--candidate-sha",
            self.candidate,
            "--automation-sha",
            AUTOMATION,
            "--default-branch",
            "main",
        ]
        with patch.object(release, "verify_bindings") as bindings:
            self.assertEqual(release.main(argv), 0)
        self.assertEqual(bindings.call_count, 2)
        bindings.assert_called_with(self.root, self.candidate, AUTOMATION)
        self.assert_attached_candidate()
        with (
            patch.object(
                release,
                "verify_bindings",
                side_effect=[None, release.ReleaseError("moved automation")],
            ),
            self.assertRaises(SystemExit) as failed,
        ):
            release.main(argv)
        self.assertEqual(failed.exception.code, 1)

    def test_shared_preparation_workflow_attaches_before_app_credentials_and_action(self) -> None:
        root = Path(__file__).resolve().parents[2]
        workflow = (root / ".github/workflows/shared-release-prepare.yml").read_text()
        ordered = [
            "uses: ./_automation/.github/actions/shared-source",
            "name: Require the exact default-branch candidate",
            "uses: ./_automation/.github/actions/shared-python",
            "name: Attach the exact candidate to the local default branch",
            "name: Create repository-scoped release preparation token",
            "uses: release-plz/action@",
        ]
        self.assertEqual(
            sorted(workflow.index(token) for token in ordered),
            [workflow.index(token) for token in ordered],
        )
        step = workflow.split("name: Attach the exact candidate to the local default branch\n", 1)[
            1
        ].split("- name:", 1)[0]
        for required in (
            "uv run --locked --project _automation/automation "
            "python -m boxferry_ci.release prepare-branch",
            '--repository-root source --candidate-sha "${CANDIDATE_SHA}"',
            '--automation-sha "${AUTOMATION_SHA}" --default-branch "${DEFAULT_BRANCH}"',
            "CANDIDATE_SHA: ${{ inputs.candidate_sha }}",
            "AUTOMATION_SHA: ${{ inputs.automation_sha }}",
            "DEFAULT_BRANCH: ${{ github.event.repository.default_branch }}",
        ):
            self.assertIn(required, step)
        for required in (
            'test "${POLICY_SHA}" = "${CANDIDATE_SHA}"',
            'test "${GITHUB_REF}" = "refs/heads/${DEFAULT_BRANCH}"',
            'case "${GITHUB_EVENT_NAME}" in push|workflow_dispatch)',
            "client-id: ${{ inputs.app_client_id }}",
            "private-key: ${{ secrets.app_private_key }}",
            "permission-contents: write",
            "permission-pull-requests: write",
            "manifest_path: source/Cargo.toml",
            "config: source/release-plz.toml",
        ):
            self.assertIn(required, workflow)
        self.assertNotIn("continue-on-error", workflow)
        self.assertNotIn("git checkout", step)
        self.assertNotIn("git reset", step)
        self.assertNotIn("git fetch", step)
        self.assertNotIn("git push", step)


if __name__ == "__main__":
    unittest.main()
