#!/usr/bin/env python3
"""Offline independent authority/observer controls; no native qualification."""
from __future__ import annotations

from contextlib import ExitStack, contextmanager, redirect_stderr, redirect_stdout
import copy
import errno
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import types
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


worker = load("main_custody_worker_test", "reviewed-docker-consumer-run.py")
admission = load("main_custody_admission_test", "reviewed-docker-consumer-admission.py")
mechanism = load("main_custody_entry_test", "verify-docker-paperless-custody.py")
auth, evidence = worker.auth, worker.proof
MAIN, SDK = "a" * 40, "b" * 40
ACTORS = {"GITHUB_ACTOR": "Maintainer", "GITHUB_TRIGGERING_ACTOR": "Reviewer"}


class SourceAuthority(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="main-custody-authority-")
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.main, self.sdk = self.base / "main", self.base / "sdk"
        self.main.mkdir()
        self.sdk.mkdir()
        for name in auth.CUSTODY_MAIN_FILES:
            path = self.main / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(("authored main bytes " + name).encode())
        (self.main / "Cargo.lock").write_bytes(b"unchanged published main lock\n")
        for name in auth.LENS_FILES - {"src/reviewed_catalog.rs"}:
            path = self.sdk / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(("authored SDK bytes " + name).encode())
        (self.sdk / "Cargo.lock").write_bytes(b"immutable SDK lock\n")
        self.dirty = set()

        def git(root, *arguments):
            if arguments == ("rev-parse", "--show-toplevel"):
                return str(root)
            if arguments == ("rev-parse", "HEAD"):
                return MAIN if root == self.main else SDK
            if arguments == ("status", "--porcelain", "--untracked-files=all"):
                return " M changed" if root in self.dirty else ""
            if arguments == ("ls-files", "--cached", "--others", "--exclude-standard", "-z"):
                return "\0".join(sorted(str(path.relative_to(root)) for path in root.rglob("*") if path.is_file())) + "\0"
            raise AssertionError(arguments)

        patch = mock.patch.object(auth, "git", side_effect=git)
        patch.start()
        self.addCleanup(patch.stop)
        self.policy = {"schema": 1, "repository": "Strukturpiloten/docker-lens", "revision": SDK,
            "source_sha256": auth.source_digest(self.sdk),
            "lock_sha256": hashlib.sha256((self.sdk / "Cargo.lock").read_bytes()).hexdigest(),
            "files": {name: hashlib.sha256((self.sdk / name).read_bytes()).hexdigest()
                      for name in sorted(auth.LENS_FILES - {"src/reviewed_catalog.rs"})}}
        policy = self.main / auth.CUSTODY_HELPERS
        policy.parent.mkdir(parents=True, exist_ok=True)
        policy.write_text(json.dumps(self.policy))
        workflow = self.main / ".github/workflows/reviewed-docker-consumer-validation.yml"
        workflow.parent.mkdir(parents=True, exist_ok=True)
        workflow.write_text("---\nenv:\n  CUSTODY_HELPER_REVISION: " + SDK + "\n")

    def test_exact_main_and_eight_helper_source_closure_precedes_copy(self):
        box, dependency, held = auth.custody_sources(self.main, self.sdk, MAIN)
        self.assertEqual(box["revision"], MAIN)
        self.assertEqual(dependency, self.policy)
        self.assertEqual(len(held), 27)
        target = self.base / "execution"
        auth.copy_closure(held, target)
        self.assertEqual({name: (target / name).read_bytes() for name in held}, held)
        self.assertEqual((self.main / "Cargo.lock").read_bytes(), b"unchanged published main lock\n")

    def test_candidate_revision_or_dirty_main_sdk_refuses(self):
        with self.assertRaises(ValueError):
            auth.custody_sources(self.main, self.sdk, "c" * 40)
        for root in (self.main, self.sdk):
            with self.subTest(root=root):
                self.dirty.add(root)
                with self.assertRaises(ValueError):
                    auth.custody_sources(self.main, self.sdk, MAIN)
                self.dirty.clear()

    def test_changed_sdk_component_or_lock_refuses_before_any_import(self):
        for name in ("scripts/native-unix-http.py", "Cargo.lock"):
            path = self.sdk / name
            original = path.read_bytes()
            path.write_bytes(original + b"substituted")
            with self.subTest(name=name), mock.patch("builtins.exec", side_effect=AssertionError("untrusted import")):
                with self.assertRaises(ValueError):
                    auth.custody_sources(self.main, self.sdk, MAIN)
            path.write_bytes(original)

    def test_missing_or_symlinked_transitive_main_source_refuses(self):
        path = self.main / "scripts/lib/docker-paperless-transport.py"
        saved = path.read_bytes()
        path.unlink()
        with self.assertRaises(OSError):
            auth.custody_sources(self.main, self.sdk, MAIN)
        outside = self.base / "foreign.py"
        outside.write_bytes(saved)
        path.symlink_to(outside)
        with self.assertRaises(ValueError):
            auth.custody_sources(self.main, self.sdk, MAIN)

    def test_pin_and_integrity_record_must_move_together(self):
        path = self.main / ".github/workflows/reviewed-docker-consumer-validation.yml"
        path.write_text(path.read_text().replace(SDK, "c" * 40))
        with self.assertRaises(ValueError):
            auth.custody_helper_policy(self.main)


class TrackedMainAuthority(unittest.TestCase):
    """Real tracked bytes and Git state; no main-source or Git mocks."""

    CANDIDATE_ONLY = frozenset({
        "crates/boxferry/examples/docker-host-metadata.rs",
        "crates/boxferry/examples/docker-volume-fixture-rehearsal.rs",
        "crates/boxferry/tests/docker_authored_bind_rehearsal.rs",
        "crates/boxferry/tests/support/docker_volume_fixtures.rs",
        "fixtures/conformance/docker-application/core-reacquire-expectations.json",
    })

    # Independent expected authority, rather than the implementation's set or
    # a count/subset assertion: canonical guard14 + launcher + reviewed support12.
    EXPECTED_GUARD_FILES = frozenset({
        "scripts/lib/docker-paperless-transport.py",
        "scripts/lib/docker-application-contract.py",
        "scripts/lib/docker-paperless-volume-custody.py",
        "scripts/lib/docker-core-artifact.py",
        "scripts/lib/docker-application-schedule.py",
        "scripts/lib/docker-application-expectations.py",
        "scripts/lib/docker-paperless-application.sh",
        "scripts/lib/paperless-application.sh",
        "scripts/lib/paperless-application-probes.sh",
        "fixtures/conformance/docker-application/expected-applications.json",
        "fixtures/conformance/paperless-ngx-application/compose.yaml",
        "fixtures/conformance/paperless-ngx-application/images.tsv",
        "fixtures/conformance/paperless-ngx-application/providers.tsv",
        "fixtures/conformance/paperless-ngx-application/document-probe.py",
    })
    EXPECTED_SUPPORT_FILES = frozenset({
        "scripts/docker-application-conformance.sh",
        "scripts/lib/bounded-native-read.py",
        "fixtures/scenarios/nextcloud-application/compose.yaml",
        "fixtures/scenarios/nextcloud-application/compose.acceptance.yaml",
        "fixtures/scenarios/nextcloud-application/nextcloud.env.example",
        "fixtures/conformance/docker-application/core-expectations.json",
        "fixtures/conformance/docker-application/core.compose.yaml",
        "fixtures/conformance/forgejo-application/compose.yaml",
        "fixtures/conformance/nextcloud-application/compose.yaml",
        "fixtures/conformance/immich-application/compose.yaml",
        "fixtures/conformance/observability-application/compose.yaml",
        "fixtures/conformance/supabase-application/compose.yaml",
    })
    EXPECTED_MAIN_FILES = EXPECTED_GUARD_FILES | EXPECTED_SUPPORT_FILES | {"scripts/verify-docker-paperless-custody.py"}

    @staticmethod
    def git(root, *arguments):
        command = ["git", "-c", "core.fsmonitor=false", "-c", "core.hooksPath=/dev/null",
            "-c", "commit.gpgsign=false", "-c", "user.name=tracked-source-fixture",
            "-c", "user.email=tracked-source-fixture@example.invalid", "-C", str(root), *arguments]
        result = subprocess.run(command, capture_output=True, check=True, timeout=20,
            env={"PATH": "/usr/bin:/bin", "LC_ALL": "C", "GIT_CONFIG_NOSYSTEM": "1",
                 "GIT_CONFIG_GLOBAL": os.devnull, "GIT_OPTIONAL_LOCKS": "0"})
        return result.stdout.decode().strip()

    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="custody-tracked-main-")
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.main = self.base / "main"
        self.main.mkdir()
        self.tracked = frozenset(name for name in self.git(ROOT, "ls-files", "--cached", "-z").split("\0") if name)
        # Never fabricate a declared dependency: every copied path is an actual
        # tracked working-tree file, including the pending source correction.
        for name in self.tracked:
            source, target = ROOT / name, self.main / name
            target.parent.mkdir(parents=True, exist_ok=True)
            if source.is_symlink():
                target.symlink_to(os.readlink(source))
            else:
                shutil.copy2(source, target)
        self.git(self.main, "init", "--quiet")
        self.revision = self.commit()
        snapshot_tracked = frozenset(name for name in self.git(self.main, "ls-files", "--cached", "-z").split("\0") if name)
        self.assertEqual(snapshot_tracked, self.tracked)

    def commit(self, *, empty=False):
        self.git(self.main, "add", "--all", "--force")
        arguments = ["commit", "--quiet", "--no-gpg-sign", "-m", "tracked source fixture"]
        if empty:
            arguments.append("--allow-empty")
        self.git(self.main, *arguments)
        self.assertEqual(self.git(self.main, "status", "--porcelain", "--untracked-files=all"), "")
        return self.git(self.main, "rev-parse", "HEAD")

    def entrypoint(self, *, revision=None, copy=False):
        # Import the production authority FROM the clean snapshot in a real
        # isolated Python process. No fabricated module, Git reply or source gate.
        program = """
import importlib.util,json,pathlib,sys
root=pathlib.Path(sys.argv[1])
spec=importlib.util.spec_from_file_location('tracked_main_authority',root/'scripts/reviewed-docker-consumer-authorization.py')
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
record,files=module.custody_main_source(root,sys.argv[2])
if sys.argv[3]!='-':module.copy_closure(files,pathlib.Path(sys.argv[3]))
print(json.dumps({'record':record,'files':sorted(files)},sort_keys=True))
"""
        return subprocess.run([sys.executable, "-I", "-B", "-c", program, str(self.main),
            revision or self.revision, str(self.base / "execution") if copy else "-"],
            capture_output=True, timeout=20, env={"PATH": "/usr/bin:/bin", "LC_ALL": "C"})

    def test_actual_clean_tracked_main_authenticates_and_copies_only_mechanism_closure(self):
        result = self.entrypoint(copy=True)
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        value = json.loads(result.stdout)
        self.assertEqual(value["record"]["revision"], self.revision)
        self.assertEqual(value["record"]["source_sha256"], auth.source_digest(ROOT))
        selected = set(value["files"])
        self.assertEqual(selected, self.EXPECTED_MAIN_FILES)
        self.assertEqual(selected, set(value["record"]["files"]))
        self.assertEqual(len(selected), 27)
        self.assertTrue(auth.CUSTODY_SOURCE_FILES <= selected)
        self.assertIn(auth.CUSTODY_LAUNCHER, selected)
        self.assertTrue(self.CANDIDATE_ONLY.isdisjoint(selected))
        execution = self.base / "execution"
        copied = {str(path.relative_to(execution)) for path in execution.rglob("*") if path.is_file()}
        self.assertEqual(copied, selected)
        for name in selected:
            self.assertEqual((execution / name).read_bytes(), (self.main / name).read_bytes())
            self.assertEqual(value["record"]["files"][name], hashlib.sha256((self.main / name).read_bytes()).hexdigest())
        program = """
import importlib.util,json,pathlib,sys
execution=pathlib.Path(sys.argv[1]).resolve(strict=True)
spec=importlib.util.spec_from_file_location('copied_custody_launcher',execution/'scripts/verify-docker-paperless-custody.py')
launcher=importlib.util.module_from_spec(spec);spec.loader.exec_module(launcher)
contract=launcher.contract()
# Follow the imported module attributes, not sys.modules: these source helpers
# use explicit specs and are not all registered as conventional imports.
modules={'launcher':launcher,'contract':contract,'core':contract.core_artifact,
    'schedule':contract.core_artifact.schedule,'expectations':contract.core_artifact.schedule.topology,
    'bounded_read':contract.native_read}
assert launcher.ROOT==execution and contract.REPOSITORY_ROOT==execution
origins={name:str(pathlib.Path(module.__file__).resolve(strict=True).relative_to(execution))
    for name,module in modules.items()}
print(json.dumps(origins,sort_keys=True))
"""
        launch = subprocess.run([sys.executable, "-I", "-B", "-c", program, str(execution)],
            capture_output=True, timeout=20, env={"PATH": "/usr/bin:/bin", "LC_ALL": "C"})
        self.assertEqual(launch.returncode, 0, launch.stderr.decode())
        self.assertEqual(json.loads(launch.stdout), {
            "launcher": "scripts/verify-docker-paperless-custody.py",
            "contract": "scripts/lib/docker-application-contract.py",
            "core": "scripts/lib/docker-core-artifact.py",
            "schedule": "scripts/lib/docker-application-schedule.py",
            "expectations": "scripts/lib/docker-application-expectations.py",
            "bounded_read": "scripts/lib/bounded-native-read.py",
        })
        self.assertEqual(self.git(self.main, "status", "--porcelain", "--untracked-files=all"), "")

    def test_candidate_only_data_presence_does_not_expand_mechanism_authority(self):
        # Deliberate unused data is added only AFTER the genuine snapshot. This
        # neither pads required main members nor assumes future candidate files
        # remain absent from the tracked project tree.
        self.assertTrue(self.CANDIDATE_ONLY.isdisjoint(self.EXPECTED_MAIN_FILES))
        for name in self.CANDIDATE_ONLY:
            path = self.main / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"deliberately unconsumed candidate-only test data\n")
        self.revision = self.commit()
        self.assertTrue(all((self.main / name).is_file() for name in self.CANDIDATE_ONLY))
        result = self.entrypoint(copy=True)
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        value = json.loads(result.stdout)
        self.assertEqual(set(value["files"]), self.EXPECTED_MAIN_FILES)
        self.assertEqual(set(value["record"]["files"]), self.EXPECTED_MAIN_FILES)
        execution = self.base / "execution"
        copied = {str(path.relative_to(execution)) for path in execution.rglob("*") if path.is_file()}
        self.assertEqual(copied, self.EXPECTED_MAIN_FILES)
        self.assertTrue(self.CANDIDATE_ONLY.isdisjoint(copied))

    def missing_member_refuses_even_in_clean_new_commit(self, name):
        (self.main / name).unlink()
        self.revision = self.commit()
        result = self.entrypoint(copy=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"FileNotFoundError", result.stderr)
        self.assertFalse((self.base / "execution").exists())

    def test_missing_original_guard_member_refuses_without_availability_filtering(self):
        self.missing_member_refuses_even_in_clean_new_commit("scripts/lib/docker-paperless-transport.py")

    def test_missing_main_import_support_refuses_without_padding(self):
        self.missing_member_refuses_even_in_clean_new_commit("scripts/lib/bounded-native-read.py")

    def test_untracked_source_refuses_before_copy(self):
        (self.main / "scripts/untracked-source.py").write_text("unexpected source\n")
        result = self.entrypoint(copy=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"Rejection", result.stderr)
        self.assertFalse((self.base / "execution").exists())

    def test_changed_actual_head_refuses_even_when_tracked_content_is_identical(self):
        old_revision = self.revision
        new_revision = self.commit(empty=True)
        self.assertNotEqual(old_revision, new_revision)
        result = self.entrypoint(revision=old_revision, copy=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"Rejection", result.stderr)
        self.assertFalse((self.base / "execution").exists())

    def test_candidate_and_legacy_closures_keep_their_original_members(self):
        self.assertEqual(len(auth.BOX_FILES), 23)
        self.assertEqual(len(auth.CUSTODY_BOX_FILES), 32)
        self.assertEqual(auth.CUSTODY_BOX_FILES, auth.BOX_FILES | auth.CUSTODY_SOURCE_FILES | {auth.CUSTODY_LAUNCHER})
        self.assertTrue(self.CANDIDATE_ONLY <= auth.BOX_FILES)
        self.assertTrue(self.CANDIDATE_ONLY <= auth.CUSTODY_BOX_FILES)
        self.assertTrue(self.CANDIDATE_ONLY.isdisjoint(auth.CUSTODY_MAIN_FILES))


class MainActorAuthority(unittest.TestCase):
    def setUp(self):
        self.env = {**ACTORS, "GITHUB_EVENT_NAME": "workflow_dispatch", "GITHUB_REF": "refs/heads/main",
            "GITHUB_REPOSITORY": "Strukturpiloten/boxferry", "GITHUB_REPOSITORY_ID": "1318220645",
            "GITHUB_WORKFLOW_REF": "Strukturpiloten/boxferry/.github/workflows/reviewed-docker-consumer-validation.yml@refs/heads/main",
            "GITHUB_SHA": MAIN, "GITHUB_WORKFLOW_SHA": MAIN, "GITHUB_RUN_ID": "123", "GITHUB_RUN_ATTEMPT": "2"}
        self.helper = types.SimpleNamespace(ACTOR_RE=re.compile(r"[A-Za-z0-9][A-Za-z0-9-]{0,38}\Z"),
                                            ALLOWED_ROLES={"write", "maintain", "admin"})
        self.calls = []

    def get(self, path, repository):
        self.calls.append((repository, path))
        return {"permission": "write"}

    def admit(self, env=None, get=None):
        with mock.patch.object(admission.auth, "custody_main_source"), \
             mock.patch.object(admission.auth, "custody_helper_policy"):
            admission.admit_main_mechanism(env or self.env, ROOT, MAIN, self.helper, get or self.get)

    def test_both_real_actors_have_fresh_permissions_in_both_literal_repositories(self):
        self.admit()
        self.assertEqual(self.calls, [(repository, f"/repos/{repository}/collaborators/{actor}/permission")
            for repository in ("Strukturpiloten/boxferry", "Strukturpiloten/docker-lens")
            for actor in ("Maintainer", "Reviewer")])
        self.assertFalse(any("/pulls/" in path for _repo, path in self.calls))

    def test_wrong_main_sha_event_ref_repository_actor_or_attempt_never_authorizes(self):
        for key, value in (("GITHUB_SHA", "c" * 40), ("GITHUB_WORKFLOW_SHA", "c" * 40),
                           ("GITHUB_REF", "refs/heads/feature"), ("GITHUB_EVENT_NAME", "pull_request"),
                           ("GITHUB_REPOSITORY", "foreign/boxferry"), ("GITHUB_REPOSITORY_ID", "1"),
                           ("GITHUB_TRIGGERING_ACTOR", "foreign/name"), ("GITHUB_RUN_ATTEMPT", "0")):
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.admit({**self.env, key: value})

    def test_either_actor_permission_failure_refuses(self):
        for rejected in ("Maintainer", "Reviewer"):
            def get(path, _repository):
                return {"permission": "read" if f"/{rejected}/" in path else "write"}
            with self.subTest(actor=rejected), self.assertRaises(ValueError):
                self.admit(get=get)

    def test_permission_branch_cannot_substitute_a_feature_pair(self):
        with mock.patch.object(admission.auth, "custody_main_source", side_effect=ValueError("dirty main")), \
             mock.patch.object(admission, "admit_pair", side_effect=AssertionError("candidate fallback")):
            with self.assertRaises(ValueError):
                admission.admit_main_mechanism(self.env, ROOT, MAIN, self.helper, self.get)


class EvidenceBoundary(unittest.TestCase):
    def setUp(self):
        self.box = {"revision": MAIN, "source_sha256": "1" * 64, "lock_sha256": "2" * 64, "files": {}}
        self.sdk = {"revision": SDK}
        self.record = {"schema": 1, "scope": "paperless-custody-mechanism", "result": "checks-passed", "case": "positive",
            "context": {"run_id": "123", "attempt": "2", "workflow_sha": MAIN, "box": self.box,
                        "actors": ACTORS, "checked_at": 1, "job": "lanes"}, "sdk": self.sdk,
            "observations": {"owner_pid": 10, "owner_start": 100, "guard_pid": 11, "guard_start": 101,
                "inner_peer": {"pid": 12, "start": 102, "uid": 1000, "gid": 1000},
                "role_objects": {role: [1, index] for index, role in enumerate(("data", "media", "consume", "export", "pgdata", "redisdata"), 1)},
                "view": {"root": [2, 1], "user_namespace": [4, 101], "mount_namespace": [4, 102],
                         "graph": [3, 7], "outer_graph": [3, 7]},
                "callbacks": [{"pid": 20, "start": 110, "sid": 10}, {"pid": 21, "start": 111, "sid": 10}],
                **{key: True for key in ("close_before_removal", "owner_exited", "guard_exited", "core_absent", "storage_absent", "private_absent")},
                "calls": 30, "read_bytes": 10000, "calls_upper_bound": 30, "read_bytes_upper_bound": 10000,
                "uncertain_attempt": False, "allowance_reused": False, "expiry_decision": None},
            "applications_accepted": 0, "full_driver": False, "target_recreation": False, "native_qualified": False}

    def validate(self, record):
        with mock.patch.object(auth, "custody_main_source", return_value=(self.box, {})), \
             mock.patch.object(auth, "custody_helper_policy", return_value=self.sdk):
            evidence.validate_mechanism(record, trusted=ROOT, revision=MAIN, run_id="123", attempt="2", case=record["case"], actors=ACTORS)

    def test_mechanism_only_record_is_not_application_acceptance(self):
        self.validate(self.record)
        for key, value in (("applications_accepted", 1), ("full_driver", True), ("target_recreation", True), ("native_qualified", True)):
            changed = copy.deepcopy(self.record)
            changed[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.validate(changed)

    def test_public_expiry_record_requires_original_guard_decision(self):
        record = copy.deepcopy(self.record)
        record["case"] = "expiry"
        record["observations"]["uncertain_attempt"] = True
        self.box["files"]["scripts/lib/docker-application-contract.py"] = "1" * 64
        record["context"]["box"] = self.box
        decision = {"schema": 1, "decision": "guard-selected-lease-expiry", "owner_pid": 10, "owner_start": 100,
            "guard_pid": 11, "guard_start": 101, "custody_sha256": "2" * 64, "guard_source_sha256": "1" * 64,
            "lease_binding_sha256": "3" * 64, "lease_sequence": 2, "lease_cutoff": 15, "lease_wall_cutoff": 15000,
            "execution_deadline": 800, "cleanup_deadline": 920, "decided_boottime": 15, "decided_wall_ms": 15000,
            "owner_alive": True, "cancelled": False}
        record["observations"]["expiry_decision"] = decision
        self.validate(record)
        # The existing valid record is exactly the closed 120-second boundary.
        for cleanup_deadline in (921, 1400):  # execution 800 + 121 / + 600
            changed = copy.deepcopy(record)
            changed["observations"]["expiry_decision"]["cleanup_deadline"] = cleanup_deadline
            with self.subTest(cleanup_deadline=cleanup_deadline), self.assertRaises(ValueError):
                self.validate(changed)
        for value in (None, {**decision, "guard_start": 102}, {**decision, "cancelled": True},
                      {**decision, "decided_boottime": 14, "decided_wall_ms": 14999}):
            record["observations"]["expiry_decision"] = value
            with self.assertRaises(ValueError):
                self.validate(record)

    def test_missing_cleanup_identity_callback_or_counter_evidence_refuses(self):
        for key in ("owner_start", "inner_peer", "callbacks", "role_objects", "close_before_removal", "private_absent", "calls"):
            changed = copy.deepcopy(self.record)
            del changed["observations"][key]
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.validate(changed)

    def test_changed_job_actor_attempt_alias_or_unclosed_storage_refuses(self):
        changes = []
        for key, value in (("job", "unrelated"), ("attempt", "1"), ("actors", {**ACTORS, "GITHUB_ACTOR": "Substitute"})):
            changed = copy.deepcopy(self.record); changed["context"][key] = value; changes.append(changed)
        changed = copy.deepcopy(self.record); changed["observations"]["role_objects"]["redisdata"] = [1, 1]; changes.append(changed)
        changed = copy.deepcopy(self.record); changed["observations"]["callbacks"][0]["sid"] = 99; changes.append(changed)
        changed = copy.deepcopy(self.record); changed["observations"]["storage_absent"] = False; changes.append(changed)
        changed = copy.deepcopy(self.record); changed["observations"]["allowance_reused"] = True; changes.append(changed)
        for index, changed in enumerate(changes):
            with self.subTest(index=index), self.assertRaises(ValueError):
                self.validate(changed)


class FixedEntrypoint(unittest.TestCase):
    def test_partial_custody_preparation_removes_only_original_private_tree(self):
        engine = mechanism.Mechanism.__new__(mechanism.Mechanism)
        engine.directory = Path("/tmp/original-private-tree")
        engine.directory_identity = "1:2"
        engine.cleanup_deadline = 900
        engine.prepared = False
        engine.path = engine.directory / "owner-custody.json"
        engine.owner = {"partially": "prepared"}
        engine.c = types.SimpleNamespace(owner_remove_private=mock.Mock(), cleanup_owner_custody=mock.Mock())
        engine.recover()
        engine.c.owner_remove_private.assert_called_once_with({
            "directory": str(engine.directory), "directory_identity": "1:2", "cleanup_deadline": 900})
        engine.c.cleanup_owner_custody.assert_not_called()

    def test_counter_never_refunds_or_grows_the_existing_work_allowance(self):
        engine = mechanism.Mechanism.__new__(mechanism.Mechanism)
        engine.calls, engine.bytes = 12, 1000
        for calls, size in ((-1, 0), (0, -1), (True, 0), (0, True)):
            with self.subTest(calls=calls, size=size), self.assertRaises(ValueError):
                engine.charge(calls, size)
            self.assertEqual((engine.calls, engine.bytes), (12, 1000))
        engine.charge(1, 5)
        self.assertEqual((engine.calls, engine.bytes), (13, 1005))
        with self.assertRaises(ValueError):
            engine.charge(244, 0)

    def test_only_six_literal_empty_owned_volume_creates_are_selected(self):
        engine = mechanism.Mechanism.__new__(mechanism.Mechanism)
        engine.owner = {"prefix": "owned", "run": "original"}
        calls = []
        def request(method, path, body, *, expected):
            value = json.loads(body)
            calls.append((method, path, value, expected))
            return {**value, "Mountpoint": "/home/docker/.local/share/docker/volumes/" + value["Name"] + "/_data"}
        engine.request = request
        for role in ("data", "media", "consume", "export", "pgdata", "redisdata"):
            engine.create_role(role)
        self.assertEqual(calls, [("POST", "/v1.56/volumes/create", {"Name": "owned-paper-" + role,
            "Driver": "local", "Labels": {"io.boxferry.live-run": "original", "io.boxferry.application": "owned-paperless"}}, 201)
            for role in ("data", "media", "consume", "export", "pgdata", "redisdata")])
        with self.assertRaises(ValueError):
            engine.create_role("foreign")
        self.assertEqual(len(calls), 6)

    def test_plain_user_refuses_before_binding_or_runtime_io(self):
        with mock.patch.object(mechanism.os, "getuid", return_value=1000), \
             mock.patch.object(mechanism, "read_private", side_effect=AssertionError("new I/O")):
            with self.assertRaises(ValueError):
                mechanism.Mechanism(Path("/not-a-binding"))

    def test_roles_cases_and_original_source_authority_are_exact(self):
        self.assertEqual(mechanism.ROLES, ("data", "media", "consume", "export", "pgdata", "redisdata"))
        self.assertEqual(mechanism.CASES, auth.CUSTODY_CASES)
        owner = mechanism.contract()
        self.assertEqual(set(owner.PAPERLESS_SOURCES), auth.CUSTODY_SOURCE_FILES)
        self.assertEqual(len(auth.CUSTODY_BOX_FILES), 32)
        self.assertEqual(len(auth.BOX_FILES), 23)
        self.assertEqual(len(auth.CUSTODY_MAIN_FILES), 27)
        self.assertLessEqual(auth.CUSTODY_SOURCE_FILES, auth.CUSTODY_MAIN_FILES)
        self.assertIn("scripts/lib/docker-paperless-transport.py", auth.CUSTODY_BOX_FILES)

    def test_canonical_registrations_and_renovate_extraction(self):
        config = json.loads((ROOT / ".github/renovate.json").read_bytes())
        managers = [manager for manager in config["customManagers"] if manager.get("description") ==
                    "Track the immutable DockerLens custody helper commit"]
        self.assertEqual(len(managers), 1)
        workflow = (ROOT / ".github/workflows/reviewed-docker-consumer-validation.yml").read_text()
        expression = re.sub(r"\(\?<([A-Za-z][A-Za-z0-9_]*)>", r"(?P<\1>", managers[0]["matchStrings"][0])
        matches = list(re.finditer(expression, workflow))
        self.assertEqual(len(matches), 1)
        policy = auth.custody_helper_policy(ROOT)
        self.assertEqual(matches[0]["currentDigest"], policy["revision"])
        self.assertEqual(matches[0]["currentValue"], "TheRealBecks/issue172")
        review = next(rule for rule in config["packageRules"] if rule.get("description") ==
                      "Require explicit security review for the DockerLens application admission pin")
        self.assertIs(review["dependencyDashboardApproval"], True)
        self.assertIs(review["automerge"], False)
        self.assertIn(".github/workflows/reviewed-docker-consumer-validation.yml", review["matchFileNames"])
        self.assertIn("main-custody-admission:", workflow)
        self.assertIn("main-custody-evidence:", workflow)
        self.assertEqual(workflow.count("source-kind: trusted-main-mechanism"), 2)


class MechanismReceiptHandoff(unittest.TestCase):
    """Real no-follow IO, with ownership emulation only on non-root hosts."""
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="mechanism-receipt-handoff-")
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.base.chmod(0o755)
        checkout = ROOT.stat()
        self.uid, self.gid = checkout.st_uid, checkout.st_gid
        self.assertGreater(self.uid, 0)
        self.assertGreater(self.gid, 0)
        self.real_root = os.geteuid() == 0
        self.staged = self.base / "staged"
        self.publication = self.staged / "custody"
        self.receipt = self.publication / "mechanism.json"
        self.record = {"schema": 1, "scope": "paperless-custody-mechanism", "result": "checks-passed",
            "case": "positive", "context": {}, "sdk": {}, "observations": {}, "applications_accepted": 0,
            "full_driver": False, "target_recreation": False, "native_qualified": False}
        self.raw = evidence.encoded(self.record)
        self.staged.mkdir(mode=0o700)
        self.publication.mkdir(mode=0o700)
        self.receipt.write_bytes(self.raw)
        self.receipt.chmod(0o600)
        self.unrelated = self.base / "private"
        self.unrelated.mkdir(mode=0o700)
        (self.unrelated / "secret").write_bytes(b"never read")
        self.changes = []
        stack = ExitStack()
        self.addCleanup(stack.close)
        stack.enter_context(mock.patch.object(worker.transfer, "WORKSPACE", self.base))
        if self.real_root:
            os.chown(self.staged, self.uid, self.gid)
            original_chown = os.fchown
            def change(fd, uid, gid):
                self.changes.append((os.fstat(fd).st_ino, uid, gid))
                original_chown(fd, uid, gid)
        else:
            # Preserve real inode/mode/size/time metadata and no-follow lookups.
            # Only root identity and ownership operations need emulation here.
            original_stat, original_fstat = os.stat, os.fstat
            original_open = os.open
            # A normal user cannot request O_NOATIME on root's ancestors.
            def privileged_open(name, flags, **kwargs):
                try:
                    return original_open(name, flags, **kwargs)
                except PermissionError:
                    self.assertTrue(flags & os.O_DIRECTORY)
                    return original_open(name, flags & ~os.O_NOATIME, **kwargs)
            stack.enter_context(mock.patch.object(worker.os, "open", side_effect=privileged_open))
            self.owners = {self.publication.stat().st_ino: (0, 0), self.receipt.stat().st_ino: (0, 0)}
            def owned(item):
                fields = {name: getattr(item, name) for name in dir(item) if name.startswith("st_")}
                fields["st_uid"], fields["st_gid"] = self.owners.get(item.st_ino, (item.st_uid, item.st_gid))
                return types.SimpleNamespace(**fields)
            stack.enter_context(mock.patch.object(worker.os, "stat", side_effect=lambda *a, **kw: owned(original_stat(*a, **kw))))
            stack.enter_context(mock.patch.object(worker.os, "fstat", side_effect=lambda fd: owned(original_fstat(fd))))
            for name in ("getuid", "geteuid", "getgid", "getegid"):
                stack.enter_context(mock.patch.object(worker.os, name, return_value=0))
            def change(fd, uid, gid):
                inode = original_fstat(fd).st_ino
                self.changes.append((inode, uid, gid))
                self.owners[inode] = (uid, gid)
        stack.enter_context(mock.patch.object(worker.os, "fchown", side_effect=change))

    def handoff(self):
        error = io.StringIO()
        with redirect_stderr(error):
            result = worker.restore_mechanism_receipt(self.uid, self.gid)
        self.assertEqual(error.getvalue(), "" if result == 0 else
            "reviewed mechanism receipt handoff rejected; no passing evidence\n")
        return result

    def test_fixed_receipt_is_transferred_before_publication_without_mode_changes(self):
        before = {path: path.stat() for path in (self.staged, self.publication, self.receipt, self.unrelated)}
        self.assertEqual(self.handoff(), 0)
        self.assertEqual(self.changes, [(before[self.receipt].st_ino, self.uid, self.gid),
                                        (before[self.publication].st_ino, self.uid, self.gid)])
        for path in before:
            self.assertEqual(path.stat().st_mode, before[path].st_mode)
        self.assertEqual(self.receipt.read_bytes(), self.raw)
        self.assertEqual(self.unrelated.stat(), before[self.unrelated])

    def test_real_original_permission_condition_and_intended_runner_read(self):
        if not self.real_root:
            self.skipTest("real root permission regression also run explicitly under sudo")
        def runner():
            os.setgid(self.gid)
            os.setuid(self.uid)
        command = [sys.executable, "-c", "import pathlib,sys;sys.stdout.buffer.write(pathlib.Path(sys.argv[1]).read_bytes())", str(self.receipt)]
        before = subprocess.run(command, capture_output=True, check=False, preexec_fn=runner)
        self.assertNotEqual(before.returncode, 0)
        self.assertIn(b"PermissionError", before.stderr)
        self.assertEqual(self.handoff(), 0)
        after = subprocess.run(command, capture_output=True, check=False, preexec_fn=runner)
        self.assertEqual((after.returncode, after.stdout, after.stderr), (0, self.raw, b""))

    def test_missing_symlink_hardlink_and_special_receipts_refuse(self):
        self.receipt.unlink()
        self.assertEqual(self.handoff(), 1)
        self.receipt.symlink_to(self.unrelated / "secret")
        self.assertEqual(self.handoff(), 1)
        self.receipt.unlink()
        os.link(self.unrelated / "secret", self.receipt)
        self.assertEqual(self.handoff(), 1)
        self.receipt.unlink()
        self.receipt.mkdir(mode=0o600)
        self.assertEqual(self.handoff(), 1)
        self.receipt.rmdir()
        os.mkfifo(self.receipt, 0o600)
        self.assertEqual(self.handoff(), 1)
        self.assertEqual(self.changes, [])

    def test_publication_symlink_and_unexpected_entry_refuse(self):
        (self.publication / "extra").write_bytes(b"private")
        self.assertEqual(self.handoff(), 1)
        (self.publication / "extra").unlink()
        moved = self.staged / "moved"
        self.publication.rename(moved)
        self.publication.symlink_to(moved, target_is_directory=True)
        self.assertEqual(self.handoff(), 1)
        self.assertEqual(self.changes, [])

    def test_wrong_modes_and_recipient_refuse_without_mutation(self):
        for path, wrong, original in ((self.staged, 0o755, 0o700), (self.publication, 0o750, 0o700),
                                       (self.receipt, 0o640, 0o600)):
            path.chmod(wrong)
            with self.subTest(path=path.name):
                self.assertEqual(self.handoff(), 1)
            path.chmod(original)
        self.uid += 1
        self.assertEqual(self.handoff(), 1)
        self.assertEqual(self.changes, [])

    def test_wrong_root_ownership_refuses(self):
        for path in (self.publication, self.receipt):
            if self.real_root:
                os.chown(path, self.uid, self.gid)
            else:
                self.owners[path.stat().st_ino] = (self.uid, self.gid)
            self.assertEqual(self.handoff(), 1)
            if self.real_root:
                os.chown(path, 0, 0)
            else:
                self.owners[path.stat().st_ino] = (0, 0)
        self.assertEqual(self.changes, [])

    def test_empty_oversized_noncanonical_invalid_and_duplicate_json_refuse(self):
        for raw in (b"", b"x" * (worker.transfer.MAX_JSON + 1), b"not json", b"{}\n",
                    self.raw + b"\n", b'{"result":"refused","result":"checks-passed"}\n',
                    self.raw.replace(b'"schema":1', b'"schema":NaN'),
                    self.raw.replace(b'"native_qualified":false', b'"native_qualified":true')):
            self.receipt.write_bytes(raw)
            with self.subTest(size=len(raw)):
                self.assertEqual(self.handoff(), 1)
        self.assertEqual(self.changes, [])

    def test_refusal_receipt_remains_refused_and_cannot_grant_teardown(self):
        self.record["result"] = "refused"
        del self.record["observations"]
        self.record["residuals"] = {key: None for key in ("last_verified_phase", "owner_pid", "owner_start",
            "guard_pid", "guard_start", "core_id", "core_name", "storage_name", "private_directory",
            "expiry_observation", "expiry_observation_pending")}
        self.record["residuals"].update(verification="incomplete", worker_teardown_authority=True)
        self.receipt.write_bytes(evidence.encoded(self.record))
        self.assertEqual(self.handoff(), 1)
        self.record["residuals"]["worker_teardown_authority"] = False
        self.receipt.write_bytes(evidence.encoded(self.record))
        self.assertEqual(self.handoff(), 0)
        self.assertEqual(json.loads(self.receipt.read_bytes()), self.record)
        with self.assertRaises(ValueError):
            evidence.validate_mechanism(self.record, trusted=ROOT, revision=MAIN,
                run_id="123", attempt="2", case="positive", actors=ACTORS)

    def test_invalid_or_mixed_envelopes_do_not_transfer(self):
        for key, value in (("schema", True), ("applications_accepted", False), ("scope", "foreign"),
                           ("case", "foreign"), ("context", []), ("sdk", []), ("observations", []),
                           ("residuals", {}), ("native_qualified", 0), ("target_recreation", True)):
            changed = {**self.record, key: value}
            self.receipt.write_bytes(evidence.encoded(changed))
            with self.subTest(key=key):
                self.assertEqual(self.handoff(), 1)
        self.assertEqual(self.changes, [])

    def test_read_time_receipt_substitution_refuses(self):
        original = os.read
        def swapped(fd, size):
            raw = original(fd, size)
            if raw:
                self.receipt.rename(self.publication / "original")
                self.receipt.write_bytes(self.raw)
                self.receipt.chmod(0o600)
            return raw
        with mock.patch.object(worker.os, "read", side_effect=swapped):
            self.assertEqual(self.handoff(), 1)
        self.assertEqual(self.changes, [])

    def test_read_time_ancestor_substitution_refuses(self):
        original = os.read
        def swapped(fd, size):
            raw = original(fd, size)
            if raw:
                self.staged.rename(self.base / "original-staged")
                self.staged.mkdir(mode=0o700)
            return raw
        with mock.patch.object(worker.os, "read", side_effect=swapped):
            self.assertEqual(self.handoff(), 1)
        self.assertEqual(self.changes, [])

    def test_old_atimes_are_preserved_by_directory_listing_and_payload_read(self):
        ancient = 1_000_000_000
        for path in (self.publication, self.receipt):
            item = path.stat()
            os.utime(path, ns=(ancient, item.st_mtime_ns))
        self.assertEqual(self.handoff(), 0)
        self.assertEqual(self.publication.stat().st_atime_ns, ancient)
        self.assertEqual(self.receipt.stat().st_atime_ns, ancient)

    def test_handoff_time_directory_substitution_fails_before_directory_chown(self):
        original = worker.os.fchown
        def swapped(fd, uid, gid):
            original(fd, uid, gid)
            self.publication.rename(self.staged / "original")
            self.publication.mkdir(mode=0o700)
        with mock.patch.object(worker.os, "fchown", side_effect=swapped):
            self.assertEqual(self.handoff(), 1)
        self.assertEqual(len(self.changes), 1)

    def test_hostile_errors_and_all_opened_descriptors_close(self):
        original_open, original_close = os.open, os.close
        for failed in ("open", "read", "fchown", "close"):
            opened, closed = [], []
            def tracked_open(*args, **kwargs):
                fd = original_open(*args, **kwargs)
                opened.append(fd)
                if failed == "open" and len(opened) == 2:
                    original_close(fd)
                    closed.append(fd)
                    raise OSError("PRIVATE_HOSTILE_PATH_TOKEN")
                return fd
            def tracked_close(fd):
                closed.append(fd)
                original_close(fd)
                if failed == "close":
                    raise OSError("PRIVATE_HOSTILE_PATH_TOKEN")
            with ExitStack() as stack:
                stack.enter_context(mock.patch.object(worker.os, "open", side_effect=tracked_open))
                stack.enter_context(mock.patch.object(worker.os, "close", side_effect=tracked_close))
                if failed in ("read", "fchown"):
                    stack.enter_context(mock.patch.object(worker.os, failed, side_effect=OSError("PRIVATE_HOSTILE_PATH_TOKEN")))
                self.assertEqual(self.handoff(), 1)
            self.assertEqual(sorted(opened), sorted(closed))

    def test_workflow_uses_credential_free_fixed_privileged_handoff_and_upload_gate(self):
        text = (ROOT / ".github/workflows/reviewed-docker-consumer-worker.yml").read_text()
        handoff = text.split("      - name: Restore only sanitized mechanism record ownership\n", 1)[1]
        restore, upload = handoff.split("      - name: Upload only bounded mechanism evidence\n", 1)
        self.assertIn("id: mechanism-receipt-handoff", restore)
        self.assertIn("if: always() && inputs.mode == 'mechanism'", restore)
        self.assertIn('sudo env -i PATH="/usr/local/bin:/usr/bin:/bin" HOME=/root PYTHONDONTWRITEBYTECODE=1', restore)
        self.assertIn('python3 -I -B - "$(id -u)" "$(id -g)"', restore)
        self.assertIn('"/home/runner/work/boxferry/boxferry/trusted/scripts/reviewed-docker-consumer-run.py"', restore)
        self.assertIn("module.restore_mechanism_receipt(int(sys.argv[1]), int(sys.argv[2]))", restore)
        for denied in ("[[", "chown", "chmod", "GITHUB_TOKEN", "GH_TOKEN", "$PWD", "--output"):
            self.assertNotIn(denied, restore)
        self.assertIn("if: always() && inputs.mode == 'mechanism' && steps.mechanism-receipt-handoff.outcome == 'success'", upload)
        self.assertIn("path: staged/custody/mechanism.json\n", upload)
        self.assertIn("if-no-files-found: error", upload)

        # Import/argument failures must also stay closed before the helper runs.
        inline = restore.split("<<'PY'\n", 1)[1].rsplit("          PY\n", 1)[0]
        inline = "\n".join(line[10:] for line in inline.splitlines())
        error = io.StringIO()
        with mock.patch.object(importlib.util, "spec_from_file_location", side_effect=OSError("PRIVATE_HOSTILE_TOKEN")), \
             redirect_stderr(error), self.assertRaises(SystemExit) as stopped:
            exec(compile(inline, "trusted-handoff-inline", "exec"), {})
        self.assertEqual(stopped.exception.code, 1)
        self.assertEqual(error.getvalue(), "reviewed mechanism receipt handoff rejected; no passing evidence\n")

    def test_fresh_handoff_interpreter_excludes_foreign_modules_and_disables_bytecode(self):
        text = (ROOT / ".github/workflows/reviewed-docker-consumer-worker.yml").read_text()
        restore = text.split("      - name: Restore only sanitized mechanism record ownership\n", 1)[1]
        restore = restore.split("      - name: Upload only bounded mechanism evidence\n", 1)[0]
        invocation = next(line.strip().split() for line in restore.splitlines()
                          if line.strip().startswith("python3 "))
        flags = invocation[1:4]
        self.assertEqual(flags, ["-I", "-B", "-"])
        shadow = self.base / "foreign-working-directory"
        shadow.mkdir()
        marker = shadow / "executed"
        (shadow / "json.py").write_text(
            f"open({str(marker)!r}, 'w').write('foreign executed')\nraise RuntimeError('foreign module')\n")
        environment = {"PATH": os.defpath, "PYTHONDONTWRITEBYTECODE": "1", "PYTHONPATH": str(shadow)}
        source = ROOT / "scripts/reviewed-docker-consumer-run.py"
        probe = ("import importlib.util,sys\n"
                 f"spec=importlib.util.spec_from_file_location('trusted_receipt_handoff', {str(source)!r})\n"
                 "module=importlib.util.module_from_spec(spec)\nspec.loader.exec_module(module)\n"
                 "print(int(sys.flags.isolated), int(sys.dont_write_bytecode))\n")
        # A fresh interpreter is essential: this process already imported json.
        exposed = subprocess.run([sys.executable, "-B", "-"], input=probe, text=True,
            cwd=shadow, env=environment, capture_output=True, check=False)
        self.assertNotEqual(exposed.returncode, 0)
        self.assertEqual(marker.read_text(), "foreign executed")
        marker.unlink()
        isolated = subprocess.run([sys.executable, *flags], input=probe, text=True,
            cwd=shadow, env=environment, capture_output=True, check=False)
        self.assertEqual((isolated.returncode, isolated.stdout, isolated.stderr), (0, "1 1\n", ""))
        self.assertFalse(marker.exists())
        self.assertFalse((shadow / "__pycache__").exists())
        # -I ignores PYTHONDONTWRITEBYTECODE; the explicit -B is required.
        without_b = subprocess.run([sys.executable, "-I", "-"],
            input="import sys; print(int(sys.dont_write_bytecode))\n", text=True,
            cwd=shadow, env=environment, capture_output=True, check=False)
        self.assertEqual((without_b.returncode, without_b.stdout, without_b.stderr), (0, "0\n", ""))


class SocketMetadataContract(unittest.TestCase):
    FIELDS = ("st_dev", "st_ino", "st_mode", "st_uid", "st_gid", "st_nlink", "st_size",
              "st_mtime_ns", "st_ctime_ns")

    def test_both_consumers_use_exact_independent_nine_field_contract(self):
        guard = load("socket_fingerprint_guard_contract", "lib/docker-paperless-volume-custody.py")
        values = {"st_dev": 101, "st_ino": 202, "st_mode": mechanism.stat.S_IFSOCK | 0o660,
            "st_uid": 303, "st_gid": 404, "st_nlink": 5, "st_size": 6,
            "st_mtime_ns": 700000001, "st_ctime_ns": 800000002, "st_atime_ns": 1}
        expected = (101, 202, mechanism.stat.S_IFSOCK | 0o660, 303, 404, 5, 6, 700000001, 800000002)
        for fingerprint in (guard.socket_fingerprint, mechanism.socket_fingerprint):
            self.assertEqual(fingerprint(types.SimpleNamespace(**values)), expected)
            self.assertEqual(fingerprint(types.SimpleNamespace(**{**values, "st_atime_ns": 999999999})), expected)
            for field in self.FIELDS:
                with self.subTest(consumer=fingerprint.__module__, field=field):
                    changed = {**values, field: values[field] ^ (0o020 if field == "st_mode" else 1)}
                    self.assertNotEqual(fingerprint(types.SimpleNamespace(**changed)), expected)

    @contextmanager
    def connected_peer(self):
        with tempfile.TemporaryDirectory(prefix="mechanism-peer-socket-") as temporary:
            path = Path(temporary) / "docker.sock"
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
                listener.bind(str(path)); listener.listen(1)
                original = path.lstat()
                os.utime(path, ns=(1_000_000_000, original.st_mtime_ns))
                caller = mechanism.Mechanism.__new__(mechanism.Mechanism)
                caller.socket, caller.socket_stamp = path, path.lstat()
                contract = mechanism.contract()
                caller.inner_peer = {"pid": os.getpid(), "start": contract.process_start(os.getpid()),
                    "uid": os.getuid(), "gid": os.getgid()}
                caller.owner = {}
                caller.boundary = mock.Mock()
                caller.c = types.SimpleNamespace(verified_parent_fd=contract.verified_parent_fd,
                    parent_alive=contract.parent_alive, owner_validate=mock.Mock())
                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as stream:
                    stream.settimeout(1); stream.connect(str(path))
                    peer = caller.peer(stream)
                    try:
                        yield caller, peer
                    finally:
                        peer.close()

    def test_actual_old_atime_connect_keeps_authenticated_launcher_peer(self):
        with self.connected_peer() as (caller, peer):
            peer.check()
            after = caller.socket.lstat()
            self.assertEqual(tuple(getattr(after, field) for field in self.FIELDS),
                             tuple(getattr(caller.socket_stamp, field) for field in self.FIELDS))
            caller.boundary.assert_called_once_with()
            caller.c.owner_validate.assert_called_once_with(caller.owner)

    def test_launcher_peer_security_field_or_path_change_refuses(self):
        for field in self.FIELDS:
            with self.subTest(field=field), self.connected_peer() as (caller, peer):
                original_lstat = Path.lstat
                def changed(path, *arguments, **keywords):
                    info = original_lstat(path, *arguments, **keywords)
                    if path == caller.socket:
                        values = {name: getattr(info, name) for name in self.FIELDS}
                        values[field] ^= 0o020 if field == "st_mode" else 1
                        return types.SimpleNamespace(**values)
                    return info
                with mock.patch.object(Path, "lstat", new=changed):
                    with self.assertRaises(mechanism.Refused):
                        peer.check()
        with self.connected_peer() as (caller, peer), socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as replacement:
            caller.socket.rename(caller.socket.with_name("original.sock"))
            replacement.bind(str(caller.socket)); replacement.listen(1)
            with self.assertRaises(mechanism.Refused):
                peer.check()


class PrivateReadIntegrity(unittest.TestCase):
    SECURITY_FIELDS = ("st_dev", "st_ino", "st_mode", "st_uid", "st_gid", "st_nlink", "st_size",
                       "st_mtime_ns", "st_ctime_ns")

    def private_file(self, directory):
        path = directory / "protected-source.json"
        value = {"protected": "private-read-secret-canary"}
        mechanism.write_new(path, value)
        # Deliberately stale atime reproduces ordinary relatime access updates.
        # It is never warmed or moved into the future to suppress the defect.
        original = path.stat()
        os.utime(path, ns=(1_000_000_000, original.st_mtime_ns))
        return path, mechanism.encoded(value)

    def test_real_old_atime_read_preserves_all_stat_fields_and_returns_exact_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            path, expected = self.private_file(Path(temporary))
            before = path.stat()
            self.assertEqual(mechanism.read_private(path), expected)
            after = path.stat()
            self.assertEqual(before, after)
            self.assertEqual(before.st_atime_ns, after.st_atime_ns)
            for field in self.SECURITY_FIELDS:
                with self.subTest(field=field):
                    self.assertEqual(getattr(before, field), getattr(after, field))

    def test_metadata_or_path_mutation_during_read_still_refuses(self):
        for change in ("mode", "links", "size", "mtime", "path"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                path, _expected = self.private_file(directory)
                read = os.read
                changed = False
                def mutate(descriptor, maximum):
                    nonlocal changed
                    raw = read(descriptor, maximum)
                    if raw and not changed:
                        changed = True
                        if change == "mode":
                            path.chmod(0o640)
                        elif change == "links":
                            os.link(path, directory / "unexpected-link")
                        elif change == "size":
                            with path.open("ab") as output:
                                output.write(b"modified")
                        elif change == "mtime":
                            stamp = path.stat()
                            os.utime(path, ns=(stamp.st_atime_ns, stamp.st_mtime_ns + 1_000_000_000))
                        else:
                            replacement = directory / "replacement.json"
                            mechanism.write_new(replacement, {"protected": "private-read-secret-canary"})
                            replacement.replace(path)
                    return raw
                with mock.patch.object(mechanism.os, "read", side_effect=mutate):
                    with self.assertRaises(mechanism.Refused):
                        mechanism.read_private(path)
                self.assertTrue(changed)

    def test_private_uid_mode_and_link_requirements_are_not_relaxed(self):
        for change in ("uid", "mode", "links"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as temporary, ExitStack() as stack:
                directory = Path(temporary)
                path, _expected = self.private_file(directory)
                if change == "uid":
                    stack.enter_context(mock.patch.object(mechanism.os, "geteuid", return_value=os.geteuid() + 1))
                elif change == "mode":
                    path.chmod(0o644)
                else:
                    os.link(path, directory / "unexpected-link")
                read = stack.enter_context(mock.patch.object(mechanism.os, "read", side_effect=AssertionError("private bytes read before validation")))
                with self.assertRaises(mechanism.Refused):
                    mechanism.read_private(path)
                read.assert_not_called()


class MechanismDiagnostics(unittest.TestCase):
    class HostileError(Exception):
        def __str__(self):
            raise AssertionError("exception text must never be consulted")

        def __repr__(self):
            raise AssertionError("exception representation must never be consulted")

    @contextmanager
    def harness(self, fault=None, *, close_fault=False, persist_fault=False, case="positive"):
        with tempfile.TemporaryDirectory(prefix="mechanism-diagnostics-") as temporary, ExitStack() as stack:
            base = Path(temporary)
            args = types.SimpleNamespace(command="mechanism", trusted_root=base / "trusted-secret-canary",
                box_root=base / "trusted-secret-canary", lens_root=base / "sdk-secret-canary",
                execution_root=base / "execution-secret-canary", private=base / "private-secret-canary",
                output=base / "output-secret-canary", trusted_sha=MAIN, box_sha=MAIN, lens_sha=SDK,
                run_id=123, attempt=2, box_pr=None, lens_pr=None, build_artifact_id=None,
                case=case, lane="upstream-rootless", checked_at=1000)
            environment = {"GITHUB_SHA": MAIN, "GITHUB_WORKFLOW_SHA": MAIN, "GITHUB_RUN_ID": "123",
                "GITHUB_RUN_ATTEMPT": "2", "GITHUB_ACTOR": "Alice", "GITHUB_TRIGGERING_ACTOR": "Bob",
                "GITHUB_JOB": "lanes", "PRIVATE_VALUE": "environment-secret-canary"}
            if fault == "context":
                args.box_sha = "c" * 40
            stack.enter_context(mock.patch.dict(os.environ, environment, clear=True))
            for name in ("getuid", "geteuid", "getgid", "getegid"):
                stack.enter_context(mock.patch.object(worker.os, name, return_value=0))
            stack.enter_context(mock.patch.object(worker.time, "time", return_value=1000))
            error = self.HostileError("exception-secret-canary /private/secret-canary\n::error::hostile")
            runtime = stack.enter_context(mock.patch.object(worker, "runtime_environment",
                side_effect=error if fault == "runtime" else None))
            files = {auth.CUSTODY_LAUNCHER: b"reviewed copied source"}
            source_calls = []
            def sources(*arguments):
                source_calls.append(arguments)
                if fault == "source" and len(source_calls) == 1 or fault == "evidence-source" and len(source_calls) == 2:
                    raise error
                return {"revision": MAIN}, {"revision": SDK}, files
            stack.enter_context(mock.patch.object(auth, "custody_sources", side_effect=sources))
            copied = stack.enter_context(mock.patch.object(auth, "copy_closure", side_effect=error if fault == "copy" else None))
            original_mkdir = Path.mkdir
            def mkdir(path, *arguments, **keywords):
                if fault == "binding-directory" and path == args.private or fault == "observer-directory" and path == args.output:
                    raise error
                return original_mkdir(path, *arguments, **keywords)
            stack.enter_context(mock.patch.object(Path, "mkdir", autospec=True, side_effect=mkdir))
            writes = []
            def write(path, value):
                kind = value.get("result", "binding")
                writes.append((path, kind))
                if fault == "binding-write" and kind == "binding" or fault == "evidence-write" and kind == "checks-passed":
                    raise error
                if persist_fault and kind == "refused":
                    raise error
            stack.enter_context(mock.patch.object(worker, "write", side_effect=write))
            loader = types.SimpleNamespace(exec_module=mock.Mock(side_effect=error if fault == "import-load" else None))
            spec = types.SimpleNamespace(loader=loader)
            stack.enter_context(mock.patch.object(worker.importlib.util, "spec_from_file_location",
                side_effect=error if fault == "import-spec" else None, return_value=spec))
            stack.enter_context(mock.patch.object(worker.importlib.util, "module_from_spec", return_value=types.SimpleNamespace()))
            process, tick_result = object(), object()
            observer = types.SimpleNamespace(owner=None, guard=None, cid=None, phase=0,
                tick=mock.Mock(side_effect=error if fault == "observer-tick" else None, return_value=tick_result),
                finish=mock.Mock(side_effect=error if fault == "observer-finish" else None, return_value={}),
                close=mock.Mock(side_effect=error if close_fault else None))
            constructor = stack.enter_context(mock.patch.object(worker, "CustodyObserver",
                side_effect=error if fault == "observer-construction" else None, return_value=observer))
            executions = []
            def execute(arguments, **keywords):
                executions.append((arguments, keywords))
                if fault == "execution":
                    raise error
                self.assertIs(keywords["observe"](process), tick_result)
                return b""
            stack.enter_context(mock.patch.object(worker, "execute", side_effect=execute))
            read = stack.enter_context(mock.patch.object(auth, "read",
                side_effect=error if fault == "evidence-read" else None, return_value=files[auth.CUSTODY_LAUNCHER]))
            validation = stack.enter_context(mock.patch.object(evidence, "validate_mechanism",
                side_effect=error if fault == "evidence-validation" else None))
            stack.enter_context(mock.patch.object(worker, "arguments", return_value=args))
            yield types.SimpleNamespace(args=args, observer=observer, process=process, writes=writes,
                source_calls=source_calls, executions=executions, copied=copied, loader=loader,
                runtime=runtime, constructor=constructor, read=read, validation=validation)

    def capture_main(self):
        stdout, stderr = io.StringIO(), io.StringIO()
        with redirect_stdout(stdout), redirect_stderr(stderr):
            status = worker.main()
        return status, stdout.getvalue(), stderr.getvalue()

    def test_actual_mechanism_boundaries_attribute_failures_without_private_text(self):
        for fault, expected in (
            ("runtime", "mechanism-runtime"), ("context", "mechanism-context"),
            ("source", "mechanism-source"), ("copy", "mechanism-copy"),
            ("binding-directory", "mechanism-binding"), ("binding-write", "mechanism-binding"),
            ("import-spec", "mechanism-import"), ("import-load", "mechanism-import"),
            ("observer-construction", "mechanism-observer"), ("observer-directory", "mechanism-observer"),
            ("execution", "mechanism-execution"), ("observer-tick", "mechanism-observer"),
            ("observer-finish", "mechanism-observer"), ("evidence-source", "mechanism-evidence"),
            ("evidence-read", "mechanism-evidence"), ("evidence-validation", "mechanism-evidence"),
            ("evidence-write", "mechanism-evidence"),
        ):
            with self.subTest(fault=fault), self.harness(fault) as setup:
                expected_message = "reviewed Docker consumer execution rejected: phase=" + expected + " reason=rejected; no passing evidence\n"
                self.assertEqual(self.capture_main(), (1, "", expected_message))
                if fault in {"execution", "observer-tick", "observer-finish", "evidence-source",
                             "evidence-read", "evidence-validation", "evidence-write"}:
                    setup.observer.close.assert_called_once_with()
                else:
                    setup.observer.close.assert_not_called()

    def test_refusal_persistence_and_close_failures_preserve_originating_phase(self):
        for persist, close in ((True, False), (False, True), (True, True)):
            with self.subTest(persist=persist, close=close), self.harness("execution", persist_fault=persist, close_fault=close) as setup:
                self.assertEqual(self.capture_main(), (1, "", "reviewed Docker consumer execution rejected: phase=mechanism-execution reason=rejected; no passing evidence\n"))
                setup.observer.close.assert_called_once_with()
                self.assertEqual([kind for _path, kind in setup.writes], ["binding", "refused"])

    def test_success_is_silent_and_keeps_original_calls_budgets_and_statuses(self):
        for case, statuses in (("positive", (0,)), ("guard-loss", (0,)), ("owner-loss", (-mechanism.signal.SIGKILL,))):
            with self.subTest(case=case), self.harness(case=case) as setup:
                self.assertEqual(self.capture_main(), (0, "", ""))
                self.assertEqual([kind for _path, kind in setup.writes], ["binding", "checks-passed"])
                setup.observer.tick.assert_called_once_with(setup.process)
                setup.observer.finish.assert_called_once_with()
                setup.observer.close.assert_called_once_with()
                self.assertEqual(len(setup.source_calls), 2)
                self.assertEqual(len(setup.executions), 1)
                arguments, options = setup.executions[0]
                self.assertEqual(arguments, [sys.executable, str(setup.args.execution_root / auth.CUSTODY_LAUNCHER),
                    "run", "--binding", str(setup.args.private / "mechanism-binding.json")])
                self.assertEqual((options["seconds"], options["cleanup_seconds"], options["preserve_guard"], options["accepted_statuses"]),
                                 (900, 125, True, statuses))

    def test_phase_order_and_observer_callback_argument_return_are_preserved(self):
        with self.harness() as setup:
            phases = []
            self.assertIsNone(worker.mechanism(setup.args, phase=phases.append))
            self.assertEqual(phases, ["mechanism-runtime", "mechanism-context", "mechanism-source", "mechanism-copy",
                "mechanism-binding", "mechanism-import", "mechanism-observer", "mechanism-execution",
                "mechanism-observer", "mechanism-execution", "mechanism-observer", "mechanism-evidence"])
            setup.observer.tick.assert_called_once_with(setup.process)

    def test_denied_or_unsupported_private_open_never_retries_or_leaks(self):
        for code in (errno.EPERM, errno.EINVAL, errno.EOPNOTSUPP):
            with self.subTest(errno=code), self.harness() as setup:
                path = setup.args.private.parent / "protected-private-path-secret-canary.json"
                mechanism.write_new(path, {"protected": "private-content-secret-canary"})
                setup.observer.tick.side_effect = lambda _process: mechanism.read_private(path)
                failure = OSError(code, "private-open-error-secret-canary", str(path))
                with mock.patch.object(mechanism.os, "open", side_effect=failure) as opened, \
                     mock.patch.object(mechanism.os, "read", side_effect=AssertionError("read after refused open")) as read:
                    self.assertEqual(self.capture_main(), (1, "", "reviewed Docker consumer execution rejected: phase=mechanism-observer reason=rejected; no passing evidence\n"))
                opened.assert_called_once_with(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_NOATIME)
                read.assert_not_called()
                setup.observer.close.assert_called_once_with()

    def test_unknown_or_hostile_phase_inputs_use_only_fixed_fallback(self):
        for phase in ("private-secret-canary\n::error::injected", self.HostileError("exception-secret-canary"), Path("/private/secret-canary")):
            self.assertEqual(worker.failure_message(phase),
                "reviewed Docker consumer execution rejected: phase=arguments reason=rejected; no passing evidence")


class ObserverAuthority(unittest.TestCase):
    def test_polling_tolerates_transient_duplicate_fds_but_stable_checkpoint_refuses(self):
        observer = worker.CustodyObserver.__new__(worker.CustodyObserver)
        observer.guard, observer.guard_fd = {"pid": 123}, 1
        observer.owner = {"prefix": "owned"}
        observer.module = mechanism
        observer.alive = lambda _fd: True
        observer.reserve, observer.charge = mock.Mock(), mock.Mock()
        entries = [types.SimpleNamespace(name="10"), types.SimpleNamespace(name="11")]
        metadata = types.SimpleNamespace(st_dev=1, st_ino=2, st_mode=mechanism.stat.S_IFDIR)
        target = "/home/docker/.local/share/docker/volumes/owned-paper-data/_data"
        with mock.patch.object(worker.os, "scandir") as scan, \
             mock.patch.object(worker.os, "readlink", return_value=target), \
             mock.patch.object(worker.os, "stat", return_value=metadata) as metadata_read:
            scan.return_value.__enter__.return_value = entries
            self.assertEqual(observer.inventory(), {"data": [1, 2]})
            self.assertEqual(metadata_read.call_count, 2)
            with self.assertRaises(ValueError):
                observer.inventory(strict=True)

    def test_foreign_or_partial_journal_cannot_grant_owner_identity_or_teardown(self):
        with tempfile.TemporaryDirectory(prefix="custody-observer-") as temporary:
            private = Path(temporary)
            observer = worker.CustodyObserver(mechanism, private, private / "sdk", {}, "positive")
            self.addCleanup(observer.close)
            row = {"stage": "prepared", "owner": {"parent_pid": os.getpid() + 1, "parent_start": 1},
                "custody_sha256": "1" * 64, "guard": {}, "cid": None, "case": "positive", "calls": 0,
                "read_bytes": 1, "callback_pids": [], "callbacks": [], "inner_peer": None}
            mechanism.write_new(private / "phase-0.json", row)
            with mock.patch.object(observer.c, "owner_read", side_effect=AssertionError("foreign custody lookup")) as lookup:
                with self.assertRaises(ValueError):
                    observer.tick(types.SimpleNamespace(pid=os.getpid()))
                lookup.assert_not_called()
            self.assertIsNone(observer.owner)
            self.assertIsNone(observer.guard)

    def observer(self, private):
        observer = worker.CustodyObserver.__new__(worker.CustodyObserver)
        observer.module = mechanism
        observer.private, observer.case = private, "positive"
        observer.owner = {"parent_pid": 10, "parent_start": 100, "cleanup_deadline": 999,
            "directory": str(private / "absent-original"), "outer_name": "bf-docker-core-original",
            "volume_name": "bf-docker-core-data-original"}
        observer.guard = {"pid": 11, "start": 101}
        observer.cid = "c" * 64
        observer.owner_fd, observer.guard_fd = 1, 2
        observer.alive = lambda _fd: False
        observer.inventory = lambda **_kwargs: {}
        observer.closed_before_removal = True
        observer.c = types.SimpleNamespace(owner_clock=lambda: 1)
        observer.roles = {role: [1, index] for index, role in enumerate(mechanism.ROLES, 1)}
        observer.held_view = {"root": [2, 1], "user_namespace": [4, 101], "mount_namespace": [4, 102],
                              "graph": [3, 7], "outer_graph": [3, 7]}
        observer.inner_peer = {"pid": 12, "start": 102, "uid": 1000, "gid": 1000}
        observer.callbacks = [{"pid": 20, "start": 110, "sid": 10}, {"pid": 21, "start": 111, "sid": 10}]
        observer.known_calls, observer.known_bytes = 20, 1000
        observer.calls = observer.bytes = observer.ack_calls = observer.ack_bytes = 0
        observer.read_phase = lambda _path: {"case": "positive", "calls": 20, "read_bytes": 1000,
                                            "callbacks": observer.callbacks}
        observer.command = mock.Mock(return_value=(1, b""))
        return observer

    def test_final_verifier_queries_only_exact_retained_resources_and_never_tears_down(self):
        with tempfile.TemporaryDirectory() as temporary:
            observer = self.observer(Path(temporary))
            result = observer.finish()
            self.assertEqual(observer.command.call_args_list, [mock.call(["presence", kind, identity])
                for kind, identity in (("container", "c" * 64), ("container", "bf-docker-core-original"),
                                       ("volume", "bf-docker-core-data-original"))])
            self.assertIs(result["core_absent"], True)
            self.assertIs(result["private_absent"], True)

    def test_living_owner_missing_close_or_original_directory_cannot_pass(self):
        for failure in ("owner-alive", "close-unobserved", "directory-present", "core-present"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as temporary:
                observer = self.observer(Path(temporary))
                if failure == "owner-alive":
                    observer.alive = lambda fd: fd == 1
                elif failure == "close-unobserved":
                    observer.closed_before_removal = False
                elif failure == "directory-present":
                    Path(observer.owner["directory"]).mkdir()
                else:
                    observer.command.return_value = (0, b"")
                with self.assertRaises(ValueError):
                    observer.finish()

    def expiry_fixture(self, private):
        observer = self.observer(private)
        observer.case = "expiry"
        observer.owner.update(execution_deadline=800, cleanup_deadline=920, guard_source_sha256="1" * 64)
        observer.c = types.SimpleNamespace(owner_clock=lambda: 100,
            owner_expiry_observation_path=lambda _owner: private / "guard-expiry.json")
        observer.custody_sha = "2" * 64
        observer.lease_cutoff, observer.lease_wall_cutoff = 15, 15000
        observer.lease_binding_hash = "3" * 64
        lease = types.SimpleNamespace(closed=False, cutoff=15, wall_cutoff=15000, binding_sha="3" * 64, read_bytes=0)
        guard = mechanism.contract()
        with mock.patch.object(guard.os, "getpid", return_value=11), \
             mock.patch.object(guard, "process_start", return_value=101):
            decision = guard.owner_expiry_decision(observer.owner, observer.custody_sha, lease, 2,
                    guard_identity={"pid": 11, "start": 101},
                cancelled=False, parent_live=True, clock=15, wall_ms=15000)
        return observer, guard, lease, decision

    def test_decision_uses_authenticated_cached_guard_incarnation_without_proc_reads(self):
        with tempfile.TemporaryDirectory() as temporary:
            observer, guard, lease, expected = self.expiry_fixture(Path(temporary))
            with mock.patch.object(guard, "process_start", side_effect=OSError("fresh proc read forbidden")) as read:
                decision = guard.owner_expiry_decision(observer.owner, observer.custody_sha, lease, 2,
                    guard_identity={"pid": 11, "start": 101}, cancelled=False,
                    parent_live=True, clock=15, wall_ms=15000)
            self.assertEqual(decision, expected)
            read.assert_not_called()

    def test_premature_owner_loss_with_cleanup_after_cutoff_cannot_qualify_expiry(self):
        with tempfile.TemporaryDirectory() as temporary:
            observer, _guard, _lease, _decision = self.expiry_fixture(Path(temporary))
            # Both authentic processes have exited and cleanup is complete, but
            # the original guard never selected lease expiry. A late verifier
            # clock must not turn this owner-loss history into expiry success.
            self.assertGreater(observer.c.owner_clock(), observer.lease_cutoff)
            with self.assertRaises(FileNotFoundError):
                observer.finish()

    def test_actual_guard_selected_immediate_expiry_survives_cleanup_and_is_charged(self):
        with tempfile.TemporaryDirectory() as temporary:
            observer, guard, lease, decision = self.expiry_fixture(Path(temporary))
            path = observer.c.owner_expiry_observation_path(observer.owner)
            with mock.patch.object(guard, "owner_expiry_observation_path", return_value=path):
                guard.owner_save_expiry_observation(observer.owner, lease, decision)
            size = path.stat().st_size
            self.assertEqual(lease.read_bytes, size)
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            # Guard cleanup already removed every original owned resource.
            result = observer.finish()
            self.assertEqual(result["expiry_decision"], decision)
            self.assertEqual(observer.expiry_decision, decision)
            self.assertEqual(result["read_bytes"], 1000 + 2 * size)
            self.assertFalse(path.exists())
            self.assertEqual(result["calls"], 20)

    def test_cancellation_owner_loss_and_global_deadline_never_emit_expiry(self):
        with tempfile.TemporaryDirectory() as temporary:
            observer, guard, lease, _decision = self.expiry_fixture(Path(temporary))
            for cancelled, parent_live, clock in ((True, True, 16), (False, False, 16),
                                                  (False, True, 800), (True, False, 900)):
                with self.subTest(cancelled=cancelled, parent_live=parent_live, clock=clock):
                    decision = guard.owner_expiry_decision(observer.owner, observer.custody_sha, lease, 2,
                    guard_identity={"pid": 11, "start": 101},
                        cancelled=cancelled, parent_live=parent_live, clock=clock, wall_ms=16000)
                    self.assertIsNone(decision)

    def test_malformed_stale_wrong_guard_source_or_lease_decision_refuses(self):
        changes = [("guard_pid", 12), ("guard_start", 102), ("owner_start", 999),
            ("custody_sha256", "4" * 64), ("guard_source_sha256", "4" * 64),
            ("lease_binding_sha256", "4" * 64), ("lease_cutoff", 16), ("lease_wall_cutoff", 16000),
            ("execution_deadline", 801), ("cleanup_deadline", 921), ("lease_sequence", 1),
            ("owner_alive", False), ("cancelled", True), ("schema", 2), ("decision", "owner-loss")]
        for key, value in changes:
            with self.subTest(key=key), tempfile.TemporaryDirectory() as temporary:
                observer, _guard, _lease, decision = self.expiry_fixture(Path(temporary))
                decision[key] = value
                mechanism.write_new(observer.c.owner_expiry_observation_path(observer.owner), decision)
                with self.assertRaises(ValueError):
                    observer.finish()
        for invalid in ("missing", "stale", "oversized"):
            with self.subTest(invalid=invalid), tempfile.TemporaryDirectory() as temporary:
                observer, _guard, _lease, decision = self.expiry_fixture(Path(temporary))
                if invalid == "missing":
                    del decision["guard_start"]
                elif invalid == "stale":
                    decision.update(decided_boottime=14, decided_wall_ms=14999)
                else:
                    decision["extra"] = "x" * 4096
                mechanism.write_new(observer.c.owner_expiry_observation_path(observer.owner), decision)
                with self.assertRaises(ValueError):
                    observer.finish()

    def test_existing_wall_cutoff_can_select_expiry_without_a_later_verifier_clock(self):
        with tempfile.TemporaryDirectory() as temporary:
            observer, guard, lease, _decision = self.expiry_fixture(Path(temporary))
            observer.c.owner_clock = lambda: 14
            with mock.patch.object(guard.os, "getpid", return_value=11), \
                 mock.patch.object(guard, "process_start", return_value=101):
                decision = guard.owner_expiry_decision(observer.owner, observer.custody_sha, lease, 2,
                    guard_identity={"pid": 11, "start": 101},
                    cancelled=False, parent_live=True, clock=14, wall_ms=15000)
            mechanism.write_new(observer.c.owner_expiry_observation_path(observer.owner), decision)
            self.assertEqual(observer.finish()["expiry_decision"], decision)

    def test_worker_read_retention_or_unlink_failure_cannot_publish_expiry_success(self):
        class RefusedRetention(worker.CustodyObserver):
            def __setattr__(self, name, value):
                if name == "expiry_decision":
                    raise MemoryError("controlled decision retention failure")
                super().__setattr__(name, value)
        for fault in ("read", "retain", "unlink"):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as temporary:
                observer, _guard, _lease, decision = self.expiry_fixture(Path(temporary))
                path = observer.c.owner_expiry_observation_path(observer.owner)
                mechanism.write_new(path, decision)
                if fault == "retain":
                    observer.__class__ = RefusedRetention
                target = mechanism if fault == "read" else Path
                operation = "read_private" if fault == "read" else "unlink"
                if fault == "retain":
                    with self.assertRaises(MemoryError):
                        observer.finish()
                else:
                    with mock.patch.object(target, operation, side_effect=OSError("controlled observation I/O failure")):
                        with self.assertRaises(OSError):
                            observer.finish()
                self.assertFalse(observer.alive(observer.owner_fd))
                self.assertFalse(observer.alive(observer.guard_fd))
                self.assertFalse(Path(observer.owner["directory"]).exists())
                self.assertTrue(path.exists())
                observer.command.assert_not_called()  # Never worker-side runtime teardown.

    def test_observation_write_collision_or_exhausted_allowance_cannot_replace_data(self):
        with tempfile.TemporaryDirectory() as temporary:
            observer, guard, lease, decision = self.expiry_fixture(Path(temporary))
            path = observer.c.owner_expiry_observation_path(observer.owner)
            path.write_bytes(b"existing unrelated observation")
            with mock.patch.object(guard, "owner_expiry_observation_path", return_value=path):
                with self.assertRaises(FileExistsError):
                    guard.owner_save_expiry_observation(observer.owner, lease, decision)
                self.assertEqual(path.read_bytes(), b"existing unrelated observation")
                path.unlink()
                lease.read_bytes = 16 * mechanism.LIMIT
                with self.assertRaises(guard.ContractError):
                    guard.owner_save_expiry_observation(observer.owner, lease, decision)
                self.assertFalse(path.exists())

    def test_reservation_is_availability_only_and_covers_uncertain_guard_attempt(self):
        observer = self.observer(Path("/unused"))
        observer.case = "replace-pgdata"
        before = observer.calls, observer.bytes
        observer.reserve(1, 32)
        self.assertEqual((observer.calls, observer.bytes), before)
        observer.known_bytes = 16 * mechanism.LIMIT - mechanism.CHECK_BYTES
        observer.replacement_observed = True
        with self.assertRaises(ValueError):
            observer.reserve(0, 1)


if __name__ == "__main__":
    unittest.main()
