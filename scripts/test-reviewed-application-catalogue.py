#!/usr/bin/env python3
"""Adversarial admission tests for reviewed PR application catalogues."""

from __future__ import annotations

import hashlib
import copy
import importlib.util
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
import types
import unittest
from unittest.mock import patch


ROOT = pathlib.Path(__file__).resolve().parent.parent
HELPER = ROOT / "scripts/reviewed-application-catalogue.py"
SPEC = importlib.util.spec_from_file_location("reviewed_application_catalogue", HELPER)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("could not load reviewed application catalogue admission")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ReviewedApplicationCatalogueTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        root = pathlib.Path(self.temporary.name)
        self.trusted = root / "trusted"
        self.candidate = root / "candidate"
        self.verifier = root / "verifier"
        for checkout in (self.trusted, self.candidate):
            for label in (MODULE.CATALOGUE, MODULE.RUNNER, MODULE.SCHEMA):
                destination = checkout / label
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / label, destination)

    def candidate_source(self) -> str:
        return (self.candidate / MODULE.CATALOGUE).read_text(encoding="utf-8")

    def replace_task_field(self, task_id: str, field: str, value: str | list[str]) -> None:
        source = self.candidate_source()
        marker = f'[[tasks]]\nid = "{task_id}"\n'
        self.assertEqual(source.count(marker), 1)
        before, block = source.split(marker, 1)
        task, separator, after = block.partition("\n[[tasks]]")
        matches = [line for line in task.splitlines() if line.startswith(f"{field} = ")]
        self.assertEqual(len(matches), 1)
        task = task.replace(matches[0], f"{field} = {json.dumps(value)}", 1)
        (self.candidate / MODULE.CATALOGUE).write_text(
            before + marker + task + separator + after, encoding="utf-8"
        )

    def admit(self, task: str = "nextcloud-application") -> None:
        MODULE.admit(self.trusted, self.candidate, self.verifier, task)

    def test_metadata_and_full_sha_lens_revisions_use_candidate_evidence_contract(self) -> None:
        tasks = tomllib.loads(self.candidate_source())["tasks"]
        selected = next(task for task in tasks if task["id"] == "nextcloud-application")
        self.replace_task_field(
            selected["id"], "sources", ["reviewed-native-podman", *selected["sources"][1:]]
        )
        self.replace_task_field(
            selected["id"], "targets", ["reviewed-compose-specification", *selected["targets"][1:]]
        )
        self.replace_task_field("compose-lens-candidate", "revision", "a" * 40)
        self.replace_task_field("quadlet-lens-candidate", "revision", "b" * 40)
        self.admit()
        for label in (MODULE.RUNNER, MODULE.SCHEMA):
            self.assertEqual((self.verifier / label).read_bytes(), (self.trusted / label).read_bytes())
        candidate_bytes = (self.candidate / MODULE.CATALOGUE).read_bytes()
        self.assertEqual((self.verifier / MODULE.CATALOGUE).read_bytes(), candidate_bytes)
        plan = subprocess.run(
            [sys.executable, str(self.verifier / MODULE.RUNNER), "plan", "--tier",
             "pre-release", "--task", "nextcloud-application", "--format", "json"],
            check=True, text=True, capture_output=True,
        )
        self.assertEqual(json.loads(plan.stdout)["tasks"][0]["id"], "nextcloud-application")

        # The final independent validator must accept only candidate-bound evidence.
        test_spec = importlib.util.spec_from_file_location(
            "test_migration_readiness_fixture", ROOT / "scripts/test-migration-readiness.py"
        )
        assert test_spec is not None and test_spec.loader is not None
        fixture_module = importlib.util.module_from_spec(test_spec)
        test_spec.loader.exec_module(fixture_module)
        evidence, revision = fixture_module.MigrationReadinessTests().evidence_fixture(
            tier_id="pre-release", task_id="nextcloud-application"
        )
        admitted = MODULE.trusted_runner(self.verifier).load_catalogue(self.verifier / MODULE.CATALOGUE)
        candidate_task = next(task for task in admitted["tasks"] if task["id"] == "nextcloud-application")
        evidence["tasks"][0]["sources"] = candidate_task["sources"]
        evidence["tasks"][0]["targets"] = candidate_task["targets"]
        evidence["run"]["lens_revisions"] = {"compose-lens": "a" * 40, "quadlet-lens": "b" * 40}
        evidence["run"]["catalogue_sha256"] = hashlib.sha256(candidate_bytes).hexdigest()
        evidence_path = pathlib.Path(self.temporary.name) / "evidence.json"
        evidence_path.write_text(json.dumps(evidence), encoding="utf-8")
        validated = subprocess.run(
            [sys.executable, str(self.verifier / MODULE.RUNNER), "validate-evidence",
             "--evidence", str(evidence_path), "--tier", "pre-release", "--task",
             "nextcloud-application", "--revision", revision, "--require-success"],
            check=False, text=True, capture_output=True,
        )
        self.assertEqual(validated.returncode, 0, validated.stderr)
        evidence["run"]["catalogue_sha256"] = "0" * 64
        evidence_path.write_text(json.dumps(evidence), encoding="utf-8")
        rejected = subprocess.run(
            [sys.executable, str(self.verifier / MODULE.RUNNER), "validate-evidence",
             "--evidence", str(evidence_path), "--tier", "pre-release", "--task",
             "nextcloud-application", "--revision", revision, "--require-success"],
            check=False, text=True, capture_output=True,
        )
        self.assertNotEqual(rejected.returncode, 0)

    def test_execution_policy_and_shape_changes_are_rejected(self) -> None:
        tasks = tomllib.loads(self.candidate_source())["tasks"]
        lens = next(task for task in tasks if task["id"] == "compose-lens-candidate")
        forbidden = (
            ('privileged = true', 'privileged = false'),
            ('deadline-seconds = 3600', 'deadline-seconds = 7200'),
            ('maximum-disk-growth-mib = 10240', 'maximum-disk-growth-mib = 20480'),
            ('required-tools = ["podman"]', 'required-tools = ["bash"]'),
            ('required-environment = ["BOXFERRY_BIN", "BOXFERRY_COMPOSE_BIN"]',
             'required-environment = ["BOXFERRY_BIN"]'),
            ('command = ["bash", "scripts/podman-live-conformance.sh", "--profile", "application",',
             'command = ["bash", "scripts/unsafe.sh", "--profile", "application",'),
            ('max-concurrency = 4', 'max-concurrency = 3'),
            ('tier-deadline-seconds = 1200', 'tier-deadline-seconds = 2400'),
            ('approved-losses = "reviewed application assertions in scripts/lib/nextcloud-application.sh"',
             'approved-losses = "anything"'),
            ('runtime-claim = "WebDAV, database/cache use, HTTP publication, selection, persistence, and cleanup"',
             'runtime-claim = "unbounded live deployment"'),
            ('repository = "https://github.com/Strukturpiloten/compose-lens.git"',
             'repository = "https://example.invalid/malicious.git"'),
            ('commands = [["cargo", "ci-application"]]', 'commands = [["bash", "unsafe.sh"]]'),
            ('schema = 2', 'schema = 3'),
            ('schema = 2', 'schema = 2\nunexpected = "candidate"'),
            ('state = "not-executed"', 'state = "passed"'),
            ('id = "nextcloud-application"', 'id = "renamed-application"'),
            (f'revision = "{lens["revision"]}"', 'revision = "main"'),
        )
        original = self.candidate_source()
        for before, after in forbidden:
            with self.subTest(change=after):
                (self.candidate / MODULE.CATALOGUE).write_text(original.replace(before, after, 1), encoding="utf-8")
                self.assertNotEqual(original, self.candidate_source())
                with self.assertRaises(MODULE.AdmissionError):
                    self.admit()

    def test_reordered_task_array_is_not_metadata(self) -> None:
        source = self.candidate_source()
        head, first, second_and_rest = source.split("[[tasks]]", 2)
        second, rest = second_and_rest.split("[[tasks]]", 1)
        (self.candidate / MODULE.CATALOGUE).write_text(
            head + "[[tasks]]" + second + "[[tasks]]" + first + "[[tasks]]" + rest,
            encoding="utf-8",
        )
        with self.assertRaises(MODULE.AdmissionError):
            self.admit()

    def test_runner_schema_symlink_size_and_task_guards(self) -> None:
        with self.assertRaises(MODULE.AdmissionError):
            self.admit("supabase-application")
        for label in (MODULE.RUNNER, MODULE.SCHEMA):
            with self.subTest(label=label):
                path = self.candidate / label
                original = path.read_bytes()
                path.write_bytes(original + b"\n")
                with self.assertRaises(MODULE.AdmissionError):
                    self.admit()
                path.write_bytes(original)
        path = self.candidate / MODULE.CATALOGUE
        original = path.read_bytes()
        path.unlink()
        path.symlink_to(self.trusted / MODULE.CATALOGUE)
        with self.assertRaises(MODULE.AdmissionError):
            self.admit()
        path.unlink()
        path.write_bytes(original + b" " * MODULE.MAX_CONTRACT_BYTES)
        with self.assertRaises(MODULE.AdmissionError):
            self.admit()


def bridge_module(name):
    path = ROOT / ("scripts/reviewed-docker-consumer-" + name + ".py")
    spec = importlib.util.spec_from_file_location("reviewed_docker_test_" + name, path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ReviewedDockerConsumerTests(unittest.TestCase):
    """Mandatory step188 coverage; all source/PR/proof fixtures are synthetic."""

    def setUp(self):
        self.authorization = bridge_module("authorization")
        self.admission = bridge_module("admission")
        self.evidence = bridge_module("evidence")
        self.launcher = bridge_module("run")
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = pathlib.Path(self.temporary.name)
        self.trusted, self.box, self.lens = (self.root / name for name in ("trusted", "box", "lens"))
        for root in (self.trusted, self.box, self.lens):
            root.mkdir()
        self.box_files = {name: ("reviewed synthetic " + name + "\n").encode() for name in self.authorization.BOX_FILES}
        images = []
        for lane, variable in zip((*self.authorization.LANES, "fixture"),
                                  ("DEBIAN_ROOTFUL_IMAGE", "DEBIAN_ROOTLESS_IMAGE",
                                   "UPSTREAM_ROOTFUL_IMAGE", "UPSTREAM_ROOTLESS_IMAGE", "FIXTURE_IMAGE"), strict=True):
            images.append(f"{variable}='registry.invalid/test/{lane}:v1@sha256:{hashlib.sha256(lane.encode()).hexdigest()}'")
        self.lens_files = {"scripts/native-conformance.sh": ("\n".join(images) + "\n").encode(),
                           "scripts/native-unix-http.py": b"# synthetic approved helper, never executed\n"}
        declarations = []
        for lane, variant in zip(self.authorization.LANES,
                                 ("Debian11Rootful", "Debian11Rootless", "UpstreamRootful", "UpstreamRootless"), strict=True):
            debian = lane.startswith("debian11")
            record = {"schema_version": 1, "lane": lane, "identity": {
                "build": {"kind": "debian-package", "distribution": "debian11", "package_name": "docker.io",
                          "package_revision": "20.10.5+dfsg1-1+deb11u2"} if debian else {"kind": "upstream"},
                "engine_release": "20.10.5+dfsg1" if debian else "29.8.1",
                "advertised_api": "1.41" if debian else "1.56", "acquisition_api": "1.41" if debian else "1.49",
                "rendering_api": "1.41" if debian else "1.49", "mode": lane.rsplit("-", 1)[1]}}
            raw = self.evidence.encoded(record)
            digest = hashlib.sha256(raw).hexdigest()
            name = f"docs/evidence/reviewed/sha256/{digest}.json"
            self.lens_files[name] = raw
            declarations.append(f'    (NativeEvidenceLane::{variant}, "{digest}", include_str!("../{name}"),),')
        self.lens_files["src/reviewed_catalog.rs"] = (
            "const RECORDS: [(NativeEvidenceLane, &str, &str); 4] = [\n" + "\n".join(declarations) + "\n];\n").encode()
        for root, files in ((self.box, self.box_files), (self.lens, self.lens_files)):
            for name, raw in files.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(raw)
            (root / "Cargo.lock").write_bytes(b"synthetic lock\n")
        self.pair = {
            kind: {"repository": repository, "repository_id": repository_id, "pr": pr, "revision": revision,
                   "source_sha256": hashlib.sha256(kind.encode()).hexdigest(),
                   "lock_sha256": hashlib.sha256(b"synthetic lock\n").hexdigest(),
                   "files": {name: hashlib.sha256(raw).hexdigest() for name, raw in files.items()}}
            for kind, repository, repository_id, pr, revision, files in (
                ("boxferry", "Strukturpiloten/boxferry", 101, 77, "a" * 40, self.box_files),
                ("docker_lens", "Strukturpiloten/docker-lens", 202, 88, "b" * 40, self.lens_files))}
        self.manifest = {"schema": 1, "policy": copy.deepcopy(self.authorization.POLICY), "authorizations": [self.pair]}
        self.manifest_path = self.trusted / self.authorization.MANIFEST
        self.manifest_path.parent.mkdir()
        self.manifest_path.write_bytes(self.evidence.encoded(self.manifest))
        self.context = self.evidence.context(self.pair, hashlib.sha256(self.manifest_path.read_bytes()).hexdigest(),
                                             "c" * 40, 1234, 2)
        self.env = {"GITHUB_EVENT_NAME": "workflow_dispatch", "GITHUB_REF": "refs/heads/main",
                    "GITHUB_REPOSITORY": "Strukturpiloten/boxferry", "GITHUB_REPOSITORY_ID": "101",
                    "GITHUB_ACTOR": "Alice", "GITHUB_TRIGGERING_ACTOR": "Bob"}
        self.pulls = {}
        for key in ("boxferry", "docker_lens"):
            entry = self.pair[key]
            repository = {"id": entry["repository_id"], "full_name": entry["repository"]}
            self.pulls[entry["repository"]] = {"number": entry["pr"], "state": "open",
                "head": {"sha": entry["revision"], "repo": copy.deepcopy(repository)},
                "base": {"ref": "main", "repo": copy.deepcopy(repository)}}
        self.roles, self.requests, self.box_environments = {}, [], []
        self.helper = types.SimpleNamespace(
            PR_RE=re.compile(r"[1-9][0-9]{0,8}\Z"), SHA_RE=re.compile(r"[0-9a-f]{40}\Z"),
            ID_RE=re.compile(r"[1-9][0-9]*\Z"), ACTOR_RE=re.compile(r"[A-Za-z0-9][A-Za-z0-9-]{0,38}\Z"),
            ALLOWED_ROLES={"write", "maintain", "admin"})
        def normal_box_admit(env, get, repository):
            self.box_environments.append(dict(env))
            self.assertEqual(repository, "Strukturpiloten/boxferry")
            self.assertTrue(all(env[key] == self.env[key] for key in self.env))
            for actor in set((env["GITHUB_ACTOR"], env["GITHUB_TRIGGERING_ACTOR"])):
                if get(f"/repos/{repository}/collaborators/{actor}/permission")["permission"] not in self.helper.ALLOWED_ROLES:
                    raise ValueError("synthetic normal Box admission denied")
            pull = get(f"/repos/{repository}/pulls/{env['PR_NUMBER']}")
            if pull["head"]["sha"] != env["EXPECTED_SHA"] or pull["state"] != "open":
                raise ValueError("synthetic normal Box head moved")
        self.helper.admit = normal_box_admit

    def get(self, path, repository):
        self.assertIn(repository, ("Strukturpiloten/boxferry", "Strukturpiloten/docker-lens"))
        prefix = "/repos/" + repository + "/"
        self.assertTrue(path.startswith(prefix))
        self.requests.append((repository, path))
        if "/collaborators/" in path:
            actor = path.split("/")[-2]
            self.assertIn(actor, (self.env["GITHUB_ACTOR"], self.env["GITHUB_TRIGGERING_ACTOR"]))
            return {"permission": self.roles.get((repository, actor), "write")}
        self.assertEqual(path, prefix + "pulls/" + str(self.pulls[repository]["number"]))
        return copy.deepcopy(self.pulls[repository])

    def authorize(self):
        return self.authorization.authorize(self.trusted, 77, "a" * 40, 88, "b" * 40)

    def lane_fixture(self, lane, token=None):
        evidence, auth = self.evidence, self.authorization
        token = token or "A000000" + str(auth.LANES.index(lane))
        builds = {profile: evidence.sanitized_build(self.pair, self.lens, profile, hashlib.sha256(profile.encode()).hexdigest())
                  for profile in ("volume-fixtures", "host-metadata")}
        consumer = {"schema": 1, "scope": "volume-fixtures-only", "lane": lane, "result": "checks-passed",
                    "context": copy.deepcopy(self.context), "builds": builds,
                    "inert": {"kind": "inert-only-no-runtime", "binary_sha256": "d" * 64,
                              "tests": [{"name": name, "selected": 1, "passed": 1, "failed": 0, "ignored": 0}
                                        for name in auth.INERT_TESTS]}}
        catalogue = evidence.native_catalogue(self.lens_files)
        target = catalogue["profiles"][lane]
        producer = builds["volume-fixtures"]
        volume = {
            "schema": 1, "scope": "volume-only", "lane": lane, "outer_run": token,
            "volume_run": "bf-" + token.lower(), "volume_prefix": "bf-volume-" + token.lower(),
            "api_version": target["rendering_api_version"], "candidate": {
                "receipt_schema": 2, "receipt_sha256": producer["receipt_sha256"], "build_profile": "volume-fixtures",
                **{key: producer[key] for key in ("boxferry_revision", "boxferry_source_sha256", "boxferry_lock_sha256",
                                                "docker_lens_revision", "docker_lens_source_sha256",
                                                "docker_lens_lock_sha256", "binary_sha256")}},
            "native": {"native_script_sha256": self.pair["docker_lens"]["files"]["scripts/native-conformance.sh"],
                       "catalogue_sha256": evidence.identity(catalogue), "target": target},
            "observed": {"release": target["engine_release"], "api": target["advertised_api_version"],
                         "package": target["build"].get("revision", "")},
            "manifest_sha256": "e" * 64, "fixtures": [], "result": "checks-passed", "volumes": [],
            "closure": {"outer": {"ownership_sha256": evidence.identity({"kind": "container", "name": "bf-docker-core-" + token, "run": token}), "absence": "verified"},
                        "storage": {"ownership_sha256": evidence.identity({"kind": "volume", "name": "bf-docker-core-data-" + token, "run": token}), "absence": "verified"},
                        "disposable": {"identity_sha256": "f" * 64, "absence": "verified"}, "interruption": "none"}}
        self.assertEqual([len(volumes) for _, _, volumes in auth.FIXTURES], [2, 3, 6, 4, 5, 3])
        for name, owner, volumes in auth.FIXTURES:
            path = f"fixtures/conformance/{name}-application/compose.yaml"
            volume["fixtures"].append({"id": name, "source_sha256": hashlib.sha256(self.box_files[path]).hexdigest(),
                                       "artifact_sha256": hashlib.sha256(name.encode()).hexdigest(), "volume_count": len(volumes)})
            for suffix in volumes:
                body = {"Name": volume["volume_prefix"] + "-" + suffix, "Labels": {
                    "io.boxferry.live-run": volume["volume_run"], "io.boxferry.application": volume["volume_prefix"] + "-" + owner}}
                volume["volumes"].append({"identity_sha256": evidence.identity(body), "cleanup": "removed", "absence": "verified"})
        consumer["volume_proof_sha256"] = hashlib.sha256(evidence.encoded(volume)).hexdigest()
        return consumer, volume

    def validate(self, consumer, volume, lane="debian11-rootful"):
        self.evidence.validate_lane(consumer, volume, expected_context=self.context, lane=lane, pair=self.pair,
                                   lens=self.lens, files=self.lens_files, box_files=self.box_files)

    def test_positive_authorized_pair_closure_is_copied_without_executing_candidate_imports(self):
        pair, digest = self.authorize()
        self.assertEqual(pair, self.pair)
        self.assertEqual(digest, self.context["manifest_sha256"])
        with patch.object(self.authorization, "git", side_effect=lambda root, *args:
                          {"rev-parse": self.pair["boxferry"]["revision"], "status": ""}[args[0]]), \
                patch.object(self.authorization, "source_digest", return_value=pair["boxferry"]["source_sha256"]):
            files = self.authorization.verify_source(self.box, pair["boxferry"])
        copied = self.root / "execution"
        self.authorization.copy_closure(files, copied)
        for name, raw in files.items():
            self.assertEqual((copied / name).read_bytes(), raw)
        self.assertTrue((copied / "scripts/lib/docker-core-artifact.py").is_file())
        with self.assertRaises((ValueError, OSError)):
            self.authorization.copy_closure(files, copied)

    def test_manifest_identity_is_not_authority_and_budget_profile_head_changes_fail_closed(self):
        for mutation in ("box-head", "lens-head", "missing-import", "traversal", "budget", "profile", "boolean-id", "extra"):
            value = copy.deepcopy(self.manifest)
            pair = value["authorizations"][0]
            if mutation == "box-head":
                pair["boxferry"]["revision"] = "d" * 40
            elif mutation == "lens-head":
                pair["docker_lens"]["revision"] = "e" * 40
            elif mutation == "missing-import":
                pair["boxferry"]["files"].pop("scripts/lib/docker-application-schedule.py")
            elif mutation == "traversal":
                pair["boxferry"]["files"]["../candidate-validator.py"] = "f" * 64
            elif mutation == "budget":
                value["policy"]["outer_pids"] = 1024
            elif mutation == "profile":
                value["policy"]["scope"] = "forgejo-application"
            elif mutation == "boolean-id":
                pair["docker_lens"]["repository_id"] = True
            else:
                value["untrusted_claim"] = "authorized"
            self.manifest_path.write_bytes(self.evidence.encoded(value))
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.authorize()

    def test_manifest_and_transitive_source_custody_reject_duplicates_symlinks_oversize_and_changed_imports(self):
        original = self.manifest_path.read_bytes()
        for raw in (b'{"schema":1,"schema":1}', original + b" " * (128 * 1024)):
            self.manifest_path.write_bytes(raw)
            with self.assertRaises(ValueError):
                self.authorize()
        self.manifest_path.unlink()
        backing = self.root / "backing.json"
        backing.write_bytes(original)
        self.manifest_path.symlink_to(backing)
        with self.assertRaises(ValueError):
            self.authorize()
        self.manifest_path.unlink()
        self.manifest_path.write_bytes(original)
        for name in ("scripts/lib/docker-core-artifact.py", "scripts/lib/docker-application-schedule.py",
                     "scripts/lib/docker-application-expectations.py", "scripts/lib/bounded-native-read.py"):
            path = self.box / name
            before = path.read_bytes()
            path.write_bytes(before + b"# changed transitive code\n")
            with patch.object(self.authorization, "git", side_effect=lambda root, *args:
                              self.pair["boxferry"]["revision"] if args[0] == "rev-parse" else ""), \
                    patch.object(self.authorization, "source_digest", return_value=self.pair["boxferry"]["source_sha256"]), \
                    self.subTest(name=name), self.assertRaises(ValueError):
                self.authorization.verify_source(self.box, self.pair["boxferry"])
            path.write_bytes(before)

    def test_positive_dual_admission_keeps_real_box_context_and_distinct_literal_request_paths(self):
        before = copy.deepcopy(self.env)
        with patch.dict(os.environ, self.env, clear=True):
            original = dict(os.environ)
            self.admission.admit_pair(os.environ, self.pair, self.helper, self.get)
            self.assertEqual(dict(os.environ), original)
        self.assertEqual(self.env, before)
        self.assertEqual(len(self.box_environments), 1)
        self.assertEqual(self.box_environments[0]["GITHUB_REPOSITORY"], "Strukturpiloten/boxferry")
        self.assertEqual(self.box_environments[0]["GITHUB_REPOSITORY_ID"], "101")
        self.assertEqual(len(self.requests), 6)
        self.assertEqual({repository for repository, _ in self.requests},
                         {"Strukturpiloten/boxferry", "Strukturpiloten/docker-lens"})

    def test_either_real_actor_or_either_moved_head_or_context_substitution_is_rejected(self):
        for repository in self.pulls:
            for actor in ("Alice", "Bob"):
                self.roles[(repository, actor)] = "read"
                with self.subTest(repository=repository, actor=actor), self.assertRaises(ValueError):
                    self.admission.admit_pair(self.env, self.pair, self.helper, self.get)
                self.roles.clear()
        for repository in self.pulls:
            old = self.pulls[repository]["head"]["sha"]
            self.pulls[repository]["head"]["sha"] = "f" * 40
            with self.subTest(repository=repository), self.assertRaises(ValueError):
                self.admission.admit_pair(self.env, self.pair, self.helper, self.get)
            self.pulls[repository]["head"]["sha"] = old
        for field, value in (("GITHUB_REPOSITORY", "Strukturpiloten/docker-lens"),
                             ("GITHUB_REPOSITORY_ID", "202"), ("GITHUB_REF", "refs/heads/other"),
                             ("GITHUB_EVENT_NAME", "pull_request_target"), ("GITHUB_ACTOR", "Alice%2Fother"),
                             ("GITHUB_TRIGGERING_ACTOR", "")):
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.admission.admit_pair({**self.env, field: value}, self.pair, self.helper, self.get)

    def test_dependency_open_main_same_repo_exact_id_number_and_typed_responses_are_required(self):
        repository = "Strukturpiloten/docker-lens"
        original = copy.deepcopy(self.pulls[repository])
        for mutation in ("closed", "number", "bool-number", "base", "fork", "name", "id", "bool-id", "string-id", "shape"):
            pull = copy.deepcopy(original)
            if mutation == "closed": pull["state"] = "closed"
            elif mutation == "number": pull["number"] = 89
            elif mutation == "bool-number": pull["number"] = True
            elif mutation == "base": pull["base"]["ref"] = "other"
            elif mutation == "fork": pull["head"]["repo"]["full_name"] = "Other/docker-lens"
            elif mutation == "name": pull["base"]["repo"]["full_name"] = "Strukturpiloten/boxferry"
            elif mutation == "id": pull["head"]["repo"]["id"] = 203
            elif mutation == "bool-id": pull["base"]["repo"]["id"] = True
            elif mutation == "string-id": pull["head"]["repo"]["id"] = "202"
            else: pull["head"] = None
            def get(path, expected):
                if expected == repository and "/pulls/" in path:
                    return pull
                return self.get(path, expected)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.admission.admit_pair(self.env, self.pair, self.helper, get)
        for error in (ValueError("oversized response"), OSError("denied API"), ValueError("malformed response")):
            def failed(path, repository):
                raise error
            with self.assertRaises((ValueError, OSError)):
                self.admission.admit_pair(self.env, self.pair, self.helper, failed)

    def test_all_four_completed_lane_proofs_and_inert_receipts_validate_independently(self):
        artifacts = self.root / "artifacts"
        artifacts.mkdir()
        for lane in self.authorization.LANES:
            consumer, volume = self.lane_fixture(lane)
            self.validate(consumer, volume, lane)
            directory = artifacts / self.evidence.artifact_name(lane, self.context)
            directory.mkdir()
            (directory / "consumer.json").write_bytes(self.evidence.encoded(consumer))
            (directory / "volume-proof.json").write_bytes(self.evidence.encoded(volume))
        self.evidence.aggregate(artifacts, expected_context=self.context, pair=self.pair,
                                lens=self.lens, files=self.lens_files, box_files=self.box_files)
        for mutation in ("missing", "extra", "symlink", "oversize"):
            with self.subTest(mutation=mutation):
                first = artifacts / self.evidence.artifact_name(self.authorization.LANES[0], self.context)
                path = first / "volume-proof.json"
                raw = path.read_bytes()
                if mutation == "missing": path.unlink()
                elif mutation == "extra": (first / "raw-protected.log").write_text("not uploadable")
                elif mutation == "symlink":
                    path.unlink()
                    (self.root / "outside-proof.json").write_bytes(raw)
                    path.symlink_to(self.root / "outside-proof.json")
                else:
                    self.assertLess(len(raw), 16 * 1024)
                    # Valid JSON with trailing whitespace, just over the writer's
                    # original bound (not merely a malformed oversized payload).
                    path.write_bytes(raw + b" " * (16 * 1024 + 1 - len(raw)))
                with self.assertRaises((ValueError, OSError)):
                    self.evidence.aggregate(artifacts, expected_context=self.context, pair=self.pair,
                                            lens=self.lens, files=self.lens_files, box_files=self.box_files)
                if path.is_symlink() or path.exists(): path.unlink()
                path.write_bytes(raw)
                if (first / "raw-protected.log").exists(): (first / "raw-protected.log").unlink()

    def test_same_head_completed_proof_from_another_run_cannot_replace_current_proof(self):
        lane = self.authorization.LANES[0]
        current_consumer, current_volume = self.lane_fixture(lane)
        old_consumer, old_volume = self.lane_fixture(lane, token="B0000000")
        self.validate(current_consumer, current_volume, lane)
        self.validate(old_consumer, old_volume, lane)
        self.assertNotEqual(current_volume["closure"], old_volume["closure"])
        with self.assertRaises(ValueError):
            self.validate(current_consumer, old_volume, lane)
        artifacts = self.root / "cross-file-replay"
        artifacts.mkdir()
        for selected in self.authorization.LANES:
            consumer, volume = self.lane_fixture(selected)
            directory = artifacts / self.evidence.artifact_name(selected, self.context)
            directory.mkdir()
            (directory / "consumer.json").write_bytes(self.evidence.encoded(consumer))
            (directory / "volume-proof.json").write_bytes(self.evidence.encoded(old_volume if selected == lane else volume))
        with self.assertRaises(ValueError):
            self.evidence.aggregate(artifacts, expected_context=self.context, pair=self.pair,
                                    lens=self.lens, files=self.lens_files, box_files=self.box_files)

    def test_invalid_dependency_inputs_and_identical_actor_deduplication_never_substitute_context(self):
        for field, value in (("pr", True), ("pr", 0), ("pr", "88%2Fother"),
                             ("revision", "B" * 40), ("revision", "main"),
                             ("repository_id", "202"), ("repository_id", False),
                             ("repository", "Other/docker-lens")):
            pair = copy.deepcopy(self.pair)
            pair["docker_lens"][field] = value
            self.requests.clear()
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                self.admission.admit_pair(self.env, pair, self.helper, self.get)
            self.assertEqual(self.requests, [])
        self.env["GITHUB_TRIGGERING_ACTOR"] = self.env["GITHUB_ACTOR"]
        self.admission.admit_pair(self.env, self.pair, self.helper, self.get)
        self.assertEqual(len(self.requests), 4)
        self.assertTrue(all("Bob" not in path for _, path in self.requests))

    def test_admission_source_is_not_read_or_imported_before_exact_pin_head_checks(self):
        admission = self.root / "admission-source"
        admission.mkdir()
        def git(root, *arguments):
            if arguments == ("rev-parse", "--show-toplevel"):
                return str(root)
            return "d" * 40 if arguments[0] == "rev-parse" else ""
        with patch.object(self.admission.auth, "git", side_effect=git), \
                patch.object(self.admission.auth, "read") as read, self.assertRaises(ValueError):
            self.admission.shared_helper(admission, "e" * 40)
        read.assert_not_called()

    def test_tampered_dependency_records_and_missing_or_failed_lanes_are_not_aggregate_success(self):
        consumer, volume = self.lane_fixture("debian11-rootful")
        changed = dict(self.lens_files)
        name = next(path for path in changed if path.startswith("docs/evidence/reviewed/sha256/"))
        changed[name] += b" "
        with self.assertRaises(ValueError):
            self.evidence.validate_lane(consumer, volume, expected_context=self.context, lane="debian11-rootful",
                                        pair=self.pair, lens=self.lens, files=changed, box_files=self.box_files)
        for result in ("failed", "skipped", "cancelled", "missing"):
            consumer, volume = self.lane_fixture("debian11-rootful")
            consumer["result"] = result
            with self.subTest(result=result), self.assertRaises(ValueError):
                self.validate(consumer, volume)
        empty = self.root / "missing-lanes"
        empty.mkdir()
        with self.assertRaises(ValueError):
            self.evidence.aggregate(empty, expected_context=self.context, pair=self.pair,
                                    lens=self.lens, files=self.lens_files, box_files=self.box_files)

    def test_replays_wrong_lanes_candidates_counts_cleanup_and_binary_bindings_are_rejected(self):
        for mutation in ("replay", "bool-attempt", "lane", "candidate", "binary", "host-binary", "missing-fixture",
                         "partial-volumes", "count", "cleanup", "outer", "storage", "interrupted", "zero-test", "native-inert", "extra"):
            consumer, volume = self.lane_fixture("debian11-rootful")
            if mutation == "replay": consumer["context"]["run_id"] = 1233
            elif mutation == "bool-attempt": consumer["context"]["run_attempt"] = True
            elif mutation == "lane": volume["lane"] = "debian11-rootless"
            elif mutation == "candidate": volume["candidate"]["boxferry_revision"] = "e" * 40
            elif mutation == "binary": volume["candidate"]["binary_sha256"] = "a" * 64
            elif mutation == "host-binary": consumer["builds"]["host-metadata"]["binary_sha256"] = "a" * 64
            elif mutation == "missing-fixture": volume["fixtures"].pop()
            elif mutation == "partial-volumes": volume["volumes"].pop()
            elif mutation == "count": volume["fixtures"][0]["volume_count"] = 23
            elif mutation == "cleanup": volume["volumes"][0]["absence"] = "unknown"
            elif mutation == "outer": volume["closure"]["outer"]["absence"] = "unknown"
            elif mutation == "storage": volume["closure"]["storage"]["ownership_sha256"] = "a" * 64
            elif mutation == "interrupted": volume["closure"]["interruption"] = "cancelled"
            elif mutation == "zero-test": consumer["inert"]["tests"][0]["passed"] = 0
            elif mutation == "native-inert": consumer["inert"]["kind"] = "native-isolation-proved"
            else: consumer["raw_protected"] = "not uploadable"
            # Exercise the independent content checks as well as the digest bind.
            consumer["volume_proof_sha256"] = hashlib.sha256(self.evidence.encoded(volume)).hexdigest()
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.validate(consumer, volume)

    def test_exact_inert_test_discovery_and_nonzero_execution_are_mandatory(self):
        name = self.authorization.INERT_TESTS[0]
        valid = b"test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 17 filtered out;\n"
        for listing in (b"", b"foreign: test\n", ((name + ": test\n") * 2).encode()):
            with patch.object(self.launcher, "execute", return_value=listing), self.assertRaises(ValueError):
                self.launcher.exact_test(self.root / "fake", name, root=self.root, private=self.root,
                                         index=0, deadline=10**12)
        for output in (valid.replace(b"1 passed", b"0 passed"), valid.replace(b"1 passed", b"2 passed"), valid + valid):
            with patch.object(self.launcher, "execute", side_effect=[(name + ": test\n").encode(), output]), \
                    self.assertRaises(ValueError):
                self.launcher.exact_test(self.root / "fake", name, root=self.root, private=self.root, index=0, deadline=10**12)
        with patch.object(self.launcher, "execute", side_effect=[(name + ": test\n").encode(), valid]):
            result = self.launcher.exact_test(self.root / "fake", name, root=self.root, private=self.root, index=0, deadline=10**12)
        self.assertEqual(result, {"name": name, "selected": 1, "passed": 1, "failed": 0, "ignored": 0})

    def test_workflow_pin_ownership_closed_matrix_tokens_and_existing_gates(self):
        path = ".github/workflows/reviewed-docker-consumer-validation.yml"
        workflow = (ROOT / path).read_text()
        self.assertTrue(workflow.startswith("---\n"))
        self.assertIn("lane: [debian11-rootful, debian11-rootless, upstream-rootful, upstream-rootless]", workflow)
        self.assertIn("needs: [admission, lanes]", workflow)
        self.assertIn('test "$ADMISSION_RESULT" = success', workflow)
        self.assertIn('test "$LANES_RESULT" = success', workflow)
        self.assertEqual(workflow.count("GITHUB_TOKEN:"), 4)
        self.assertEqual(workflow.count("persist-credentials: false"), workflow.count("uses: actions/checkout@"))
        for line in workflow.splitlines():
            if "uses:" in line:
                self.assertRegex(line, r"uses: actions/[a-z-]+@[0-9a-f]{40} # v[0-9]+\.[0-9]+\.[0-9]+$")
        for forbidden in ("pull_request_target:", "secrets: inherit", "contents: write", "id-token: write",
                          "cargo publish", "forgejo-application", "--profile core-journey", "apparmor=unconfined"):
            self.assertNotIn(forbidden, workflow)
        configuration = json.loads((ROOT / ".github/renovate.json").read_text())
        matches = []
        for manager in configuration["customManagers"]:
            if any(re.search(pattern[1:-1], path) for pattern in manager.get("managerFilePatterns", [])):
                for pattern in manager["matchStrings"]:
                    python = re.sub(r"\(\?<([A-Za-z][A-Za-z0-9_]*)>", r"(?P<\1>", pattern)
                    matches.extend(match for match in re.finditer(python, workflow)
                                   if match.groupdict().get("depName") == "Strukturpiloten/docker-lens")
        self.assertEqual(len(matches), 1)
        self.assertEqual(matches[0]["depName"], "Strukturpiloten/docker-lens")
        rules = [rule for rule in configuration["packageRules"]
                 if path in rule.get("matchFileNames", []) and "github-digest" in rule.get("matchDatasources", [])]
        self.assertEqual(len(rules), 1)
        self.assertTrue(rules[0]["dependencyDashboardApproval"])
        self.assertFalse(rules[0]["automerge"])

    def test_private_output_collector_caps_only_logs_not_compiler_artifact_file_sizes(self):
        # Tiny Python-only subprocess controls: no compiler, Rust test or runtime.
        result = self.launcher.execute([sys.executable, "-c", "print('safe')"], root=self.root,
                                       log=self.root / "safe.log", seconds=2, maximum=128)
        self.assertEqual(result, b"safe\n")
        with self.assertRaises(ValueError):
            self.launcher.execute([sys.executable, "-c", "print('x' * 4096)"], root=self.root,
                                  log=self.root / "overflow.log", seconds=2, maximum=32, cleanup_seconds=0.1)
        self.assertLessEqual((self.root / "overflow.log").stat().st_size, 32)
        with self.assertRaises(ValueError):
            self.launcher.execute([sys.executable, "-c", "import time; time.sleep(1)"], root=self.root,
                                  log=self.root / "timeout.log", seconds=0.01, cleanup_seconds=0.1)

    def test_private_collector_terminates_and_reaps_owned_descendant_holding_stdout(self):
        for failure in ("timeout", "overflow"):
            marker = self.root / (failure + "-reaped")
            # Leader reaps its descendant only if that descendant receives the
            # owned-group TERM. Leader-only TERM cannot complete this handler.
            program = (
                "import pathlib, signal, subprocess, sys, time\n"
                "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'])\n"
                "def stop(signum, frame):\n"
                "    status = child.wait(timeout=1)\n"
                "    pathlib.Path(sys.argv[1]).write_text(str(status))\n"
                "    raise SystemExit(0)\n"
                "signal.signal(signal.SIGTERM, stop)\n"
                "print('x' * 4096, flush=True)\n"
                "time.sleep(30)\n"
            )
            with self.subTest(failure=failure), self.assertRaises(ValueError):
                self.launcher.execute([sys.executable, "-c", program, str(marker)], root=self.root,
                                      log=self.root / (failure + "-tree.log"), seconds=0.3,
                                      maximum=32 if failure == "overflow" else 8192, cleanup_seconds=2)
            self.assertEqual(marker.read_text(), str(-15))

    def cancellation_fixture(self, name, *, preserve_guard=True, failure="cancel", acquire=False, allocation=False):
        """Real signals/processes only; private synthetic Python, never native work."""
        directory = self.root / name
        directory.mkdir()
        descendant = directory / "descendant.py"
        descendant.write_text(
            "import pathlib, signal, sys, time\n"
            "root = pathlib.Path(sys.argv[1])\n"
            "signal.alarm(8)\n"
            "def stop(kind, frame):\n"
            "    (root / 'descendant-term').write_text(str(kind))\n"
            "    raise SystemExit(0)\n"
            "signal.signal(signal.SIGTERM, stop)\n"
            "(root / 'descendant-ready').write_text('ready')\n"
            "time.sleep(8)\n")
        leader = directory / "leader.py"
        leader.write_text(
            "import json, os, pathlib, signal, subprocess, sys, time\n"
            "root = pathlib.Path(sys.argv[1])\n"
            "signal.alarm(8)\n"
            "child = subprocess.Popen([sys.executable, str(root / 'descendant.py'), str(root)])\n"
            "def stop(kind, frame):\n"
            "    with (root / 'leader-term').open('a') as stream: stream.write(str(kind) + '\\n')\n"
            + ("    time.sleep(0.2)\n"
               "    if child.poll() is not None or (root / 'descendant-term').exists(): raise SystemExit(9)\n"
               if preserve_guard else
               "    if child.wait(timeout=1) != 0: raise SystemExit(9)\n") +
            "    (root / 'cleanup').write_text('complete')\n"
            "    raise SystemExit(0)\n"
            "signal.signal(signal.SIGTERM, stop)\n"
            "deadline = time.monotonic() + 2\n"
            "while not (root / 'descendant-ready').exists():\n"
            "    if time.monotonic() > deadline: raise SystemExit(8)\n"
            "    time.sleep(0.01)\n"
            "(root / 'ready').write_text(json.dumps({'leader': os.getpid(), 'descendant': child.pid}))\n"
            + ("print('x' * 4096, flush=True)\n" if failure == "overflow" else
               "print('ready', flush=True)\n" if failure == "exception" else "") +
            "time.sleep(8)\n")
        wrapper = directory / "wrapper.py"
        wrapper.write_text(
            "import ctypes, errno, importlib.util, json, os, pathlib, signal, subprocess, sys, time\n"
            "root = pathlib.Path(sys.argv[1])\n"
            # Reap adopted lingering descendants too; no dependency on host init.
            "assert ctypes.CDLL(None).prctl(36, 1, 0, 0, 0) == 0\n"
            "spec = importlib.util.spec_from_file_location('cancel_launcher', sys.argv[2])\n"
            "launcher = importlib.util.module_from_spec(spec); spec.loader.exec_module(launcher)\n"
            "def previous(kind, frame): raise AssertionError('previous handler ran during owned work')\n"
            "signal.signal(signal.SIGTERM, previous); signal.signal(signal.SIGINT, previous)\n"
            "events, report = [], {}\n"
            "kill, killpg, popen = os.kill, os.killpg, subprocess.Popen\n"
            "def owned(pid, kind, group):\n"
            # WNOWAIT guards every actual signal against post-reap PID reuse.
            "    os.waitid(os.P_PID, pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)\n"
            "    events.append(['group' if group else 'leader', kind])\n"
            "    return (killpg if group else kill)(pid, kind)\n"
            "os.kill = lambda pid, kind: owned(pid, kind, False)\n"
            "os.killpg = lambda pid, kind: owned(pid, kind, True)\n"
            + ("def acquiring(*args, **kwargs):\n"
               "    process = popen(*args, **kwargs)\n"
               "    deadline = time.monotonic() + 2\n"
               "    while not (root / 'ready').exists():\n"
               "        if time.monotonic() > deadline: raise RuntimeError('fixture startup deadline')\n"
               "        time.sleep(0.01)\n"
               "    signal.raise_signal(signal.SIGTERM)\n"
               "    return process\n"
               "launcher.subprocess.Popen = acquiring\n" if acquire else "") +
            ("read = os.read\n"
             "def broken_read(*args):\n"
             "    raw = read(*args)\n"
             "    if raw: raise OSError('original collector failure')\n"
             "    return raw\n"
             "launcher.os.read = broken_read\n" if failure == "exception" else "") +
            ("def allocation_failure():\n"
             "    deadline = time.monotonic() + 2\n"
             "    while not (root / 'ready').exists():\n"
             "        if time.monotonic() > deadline: raise RuntimeError('fixture startup deadline')\n"
             "        time.sleep(0.01)\n"
             "    raise OSError(errno.EMFILE, 'original selector allocation failure')\n"
             "launcher.selectors.DefaultSelector = allocation_failure\n" if allocation else "") +
            "try:\n"
            "    launcher.execute([sys.executable, str(root / 'leader.py'), str(root)], root=root,\n"
            "        log=root / 'private.log', seconds=" + ("0.8" if failure == "timeout" else "5") +
            ", maximum=" + ("32" if failure == "overflow" else "8192") +
            ", cleanup_seconds=1, preserve_guard=" + repr(preserve_guard) + ")\n"
            "    report['result'] = 'unexpected success'\n"
            "except BaseException as error:\n"
            "    report.update(result=type(error).__name__, error=str(error))\n"
            "    raise\n"
            "finally:\n"
            "    report['restored'] = all(signal.getsignal(kind) is previous for kind in (signal.SIGTERM, signal.SIGINT))\n"
            "    report['events'] = events\n"
            "    report['adopted'] = []\n"
            "    deadline = time.monotonic() + 2\n"
            "    while time.monotonic() < deadline:\n"
            "        try: pid, status = os.waitpid(-1, os.WNOHANG)\n"
            "        except ChildProcessError: break\n"
            "        if pid: report['adopted'].append([pid, status])\n"
            "        else: time.sleep(0.01)\n"
            "    (root / 'report').write_text(json.dumps(report))\n")
        process = subprocess.Popen([sys.executable, str(wrapper), str(directory), str(ROOT / "scripts/reviewed-docker-consumer-run.py")],
                                   stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.addCleanup(self.finish_cancellation_wrapper, process)
        return directory, process

    def finish_cancellation_wrapper(self, process):
        try:
            process.communicate(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.communicate(timeout=2)

    def await_cancellation_marker(self, directory, name, process):
        import time
        deadline = time.monotonic() + 4
        while not (directory / name).exists():
            self.assertIsNone(process.poll(), f"wrapper exited before {name}")
            self.assertLess(time.monotonic(), deadline, f"missing synthetic marker {name}")
            time.sleep(0.01)

    def test_wrapper_term_and_int_preserve_native_cleanup_and_kill_lingering_descendants(self):
        import signal
        for kind in (signal.SIGTERM, signal.SIGINT):
            with self.subTest(signal=kind):
                directory, process = self.cancellation_fixture("native-" + str(kind))
                self.await_cancellation_marker(directory, "ready", process)
                process.send_signal(kind)
                self.await_cancellation_marker(directory, "leader-term", process)
                # Repeated cancellation must neither re-enter the native handler
                # nor terminate its still-needed watchdog before cleanup finishes.
                process.send_signal(signal.SIGINT if kind == signal.SIGTERM else signal.SIGTERM)
                process.communicate(timeout=4)
                self.assertEqual(process.returncode, 128 + kind)
                report = json.loads((directory / "report").read_text())
                self.assertEqual(report["result"], "SystemExit")
                self.assertTrue(report["restored"])
                self.assertEqual(report["events"], [["leader", signal.SIGTERM], ["group", signal.SIGKILL]])
                self.assertEqual((directory / "leader-term").read_text(), "15\n")
                self.assertEqual((directory / "cleanup").read_text(), "complete")
                self.assertFalse((directory / "descendant-term").exists())
                descendant = json.loads((directory / "ready").read_text())["descendant"]
                self.assertEqual(report["adopted"], [[descendant, signal.SIGKILL]])

    def test_wrapper_cancellation_during_acquisition_is_latched_until_native_cleanup(self):
        import signal
        directory, process = self.cancellation_fixture("acquiring", acquire=True)
        process.communicate(timeout=5)
        self.assertEqual(process.returncode, 128 + signal.SIGTERM)
        report = json.loads((directory / "report").read_text())
        self.assertTrue(report["restored"])
        self.assertEqual(report["events"], [["leader", signal.SIGTERM], ["group", signal.SIGKILL]])
        self.assertEqual((directory / "cleanup").read_text(), "complete")
        self.assertEqual(len(report["adopted"]), 1)

    def test_wrapper_build_cancellation_terms_the_whole_owned_group(self):
        import signal
        for kind in (signal.SIGTERM, signal.SIGINT):
            with self.subTest(signal=kind):
                directory, process = self.cancellation_fixture("build-" + str(kind), preserve_guard=False)
                self.await_cancellation_marker(directory, "ready", process)
                process.send_signal(kind)
                process.communicate(timeout=4)
                self.assertEqual(process.returncode, 128 + kind)
                report = json.loads((directory / "report").read_text())
                self.assertTrue(report["restored"])
                self.assertEqual(report["events"], [["group", signal.SIGTERM], ["group", signal.SIGKILL]])
                self.assertEqual((directory / "cleanup").read_text(), "complete")
                self.assertEqual((directory / "descendant-term").read_text(), "15")
                self.assertEqual(report["adopted"], [])

    def test_wrapper_cancellation_keeps_first_failure_and_original_absolute_reserve(self):
        import signal
        import time
        for failure in ("timeout", "overflow"):
            with self.subTest(failure=failure):
                directory, process = self.cancellation_fixture(failure, failure=failure)
                self.await_cancellation_marker(directory, "leader-term", process)
                start = time.monotonic()
                while process.poll() is None and time.monotonic() - start < 0.7:
                    process.send_signal(signal.SIGTERM)
                    process.send_signal(signal.SIGINT)
                    time.sleep(0.05)
                process.communicate(timeout=3)
                self.assertLess(time.monotonic() - start, 1.6)
                self.assertEqual(process.returncode, 1)
                report = json.loads((directory / "report").read_text())
                self.assertEqual(report["result"], "Rejection")
                self.assertTrue(report["restored"])
                self.assertEqual(report["events"], [["leader", signal.SIGTERM], ["group", signal.SIGKILL]])
                self.assertEqual((directory / "cleanup").read_text(), "complete")
                self.assertEqual(len(report["adopted"]), 1)
                self.assertLessEqual((directory / "private.log").stat().st_size, 32 if failure == "overflow" else 8192)

    def test_wrapper_restores_both_handlers_when_process_creation_fails(self):
        import signal
        original = {kind: signal.getsignal(kind) for kind in (signal.SIGTERM, signal.SIGINT)}
        with patch.object(self.launcher.subprocess, "Popen", side_effect=OSError("original acquisition failure")), \
                self.assertRaisesRegex(OSError, "original acquisition failure"):
            self.launcher.execute(["never-executed"], root=self.root, log=self.root / "acquisition-error.log", seconds=1)
        self.assertEqual({kind: signal.getsignal(kind) for kind in original}, original)

    def test_wrapper_cancellation_during_exception_cleanup_preserves_original_error(self):
        import signal
        directory, process = self.cancellation_fixture("original-exception", failure="exception")
        self.await_cancellation_marker(directory, "leader-term", process)
        process.send_signal(signal.SIGINT)
        process.send_signal(signal.SIGTERM)
        process.communicate(timeout=4)
        self.assertEqual(process.returncode, 1)
        report = json.loads((directory / "report").read_text())
        self.assertEqual(report["result"], "OSError")
        self.assertEqual(report["error"], "original collector failure")
        self.assertTrue(report["restored"])
        self.assertEqual(report["events"], [["leader", signal.SIGTERM], ["group", signal.SIGKILL]])
        self.assertEqual((directory / "cleanup").read_text(), "complete")
        self.assertEqual(len(report["adopted"]), 1)

    def test_wrapper_selector_allocation_failure_still_cleans_owned_native_tree(self):
        import errno
        import signal
        directory, process = self.cancellation_fixture("selector-allocation", allocation=True)
        process.communicate(timeout=5)
        self.assertEqual(process.returncode, 1)
        report = json.loads((directory / "report").read_text())
        self.assertEqual(report["result"], "OSError")
        self.assertEqual(report["error"], str(OSError(errno.EMFILE, "original selector allocation failure")))
        self.assertTrue(report["restored"])
        self.assertEqual(report["events"], [["leader", signal.SIGTERM], ["group", signal.SIGKILL]])
        self.assertEqual((directory / "leader-term").read_text(), "15\n")
        self.assertEqual((directory / "cleanup").read_text(), "complete")
        self.assertFalse((directory / "descendant-term").exists())
        descendant = json.loads((directory / "ready").read_text())["descendant"]
        self.assertEqual(report["adopted"], [[descendant, signal.SIGKILL]])


if __name__ == "__main__":
    unittest.main()
