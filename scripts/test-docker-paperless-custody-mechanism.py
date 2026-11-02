#!/usr/bin/env python3
"""Offline independent authority/observer controls; no native qualification."""
from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
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
        for name in auth.CUSTODY_BOX_FILES:
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
        self.assertEqual(len(held), 32)
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
