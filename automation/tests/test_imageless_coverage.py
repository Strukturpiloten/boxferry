"""Exact image-less version files, authenticated snapshots, and producer entry points."""

from __future__ import annotations

import contextlib
import hashlib
import io
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from datetime import timedelta
from pathlib import Path
from unittest.mock import patch

import test_coverage_tools as coverage_tests
import test_dependencies as dependency_cases
import test_presets as presets
import test_recovery as recovery_cases
import test_retained_recovery as retained_cases
from test_dependencies import HASH, NOW, OLD, YOUNG, cargo_metadata
from test_dependency_snapshots import BASE, TESTED

from boxferry_ci import __main__ as cli
from boxferry_ci import cargo_tools, eligibility, recovery
from boxferry_ci.contracts import ContractError, object_value
from boxferry_ci.dependencies import DependencyContext, check_dependencies
from boxferry_ci.gate import verify_gate
from boxferry_ci.operational import SnapshotCache
from boxferry_ci.planner import Comparison, make_plan
from boxferry_ci.policy import load_policy

ROOT = Path(__file__).resolve().parents[2]
VERSION_FILE = cargo_tools.COVERAGE_FILE
# Authored cases shared with the pinned native extractor/versioner test below.
COVERAGE_VALID_VERSIONS = (
    "0.0.0",
    "0.9.0",
    "0.10.0",
    "1.2.3",
    "999998.999999.999999",
    "999999.999998.999999",
    "999999.999999.999998",
    "999999.999999.999999",
)
COVERAGE_OUTSIDE_DOMAIN = (
    "1000000.0.0",
    "0.1000000.0",
    "0.0.1000000",
    "-1.0.0",
    "0.-1.0",
    "0.0.-1",
    "000000.0.0",
    "999999.999999.999999\r\n\n",
)


class ImagelessCoverage(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.candidate = self.root / "candidate"
        (self.candidate / "scripts").mkdir(parents=True)
        self.metadata = self.candidate / VERSION_FILE
        self.metadata.write_bytes(b"1.2.3\n")
        shutil.copyfile(ROOT / "rust-toolchain.toml", self.candidate / "rust-toolchain.toml")

    def coverage_case(self) -> coverage_tests.CoverageTools:
        case = coverage_tests.CoverageTools()
        case.root, case.candidate, case.metadata = self.root, self.candidate, self.metadata
        return case

    def test_real_cargo_dispatch(self) -> None:
        self.coverage_case().test_real_cargo_dispatch_precedence_version_mismatch_and_alias_conflict()

    def test_hosted_install_cli(self) -> None:
        self.coverage_case().test_installation_exports_only_successful_verified_selection()

    def test_exact_ascii_file_and_independent_archive_semver(self) -> None:
        archive = self.root / "archive"
        (archive / ".devcontainer").mkdir(parents=True)
        shutil.copyfile(ROOT / cargo_tools.DOCKERFILE, archive / cargo_tools.DOCKERFILE)
        semver = cargo_tools.declarations(archive)["cargo-semver-checks"]
        # Coverage selection never parses root Cargo metadata.
        (self.candidate / "Cargo.toml").write_text("deliberately not TOML [")
        for version in COVERAGE_VALID_VERSIONS:
            for ending in ("", "\n", "\r\n"):
                self.metadata.write_bytes((version + ending).encode("ascii"))
                self.assertEqual(
                    cargo_tools.select(
                        self.candidate, "cargo-llvm-cov", archive, require_candidate=True
                    ).version,
                    version,
                )
                self.assertEqual(
                    cargo_tools.select(self.candidate, "cargo-semver-checks", archive).version,
                    semver,
                )
        self.metadata.unlink()
        self.assertEqual(cargo_tools.declarations(self.candidate), {})
        self.assertEqual(
            cargo_tools.select(self.candidate, "cargo-semver-checks", archive).version, semver
        )
        with self.assertRaises(ContractError):
            cargo_tools.select(self.candidate, "cargo-llvm-cov", archive, require_candidate=True)
        with self.assertRaises(ContractError):
            cargo_tools.select(self.candidate, "cargo-deny", archive, require_candidate=True)

    def test_numeric_and_file_length_domain_matches_bounded_read(self) -> None:
        maximum = "999999.999999.999999"
        self.assertEqual(len(maximum), 20)
        self.assertEqual(cargo_tools.MAX_COVERAGE_FILE_BYTES, 22)
        for ending in ("", "\n", "\r\n"):
            data = (maximum + ending).encode("ascii")
            self.metadata.write_bytes(data)
            self.assertEqual(cargo_tools.declarations(self.candidate)["cargo-llvm-cov"], maximum)
        for text in COVERAGE_OUTSIDE_DOMAIN:
            self.metadata.write_text(text, newline="")
            with self.subTest(text=text), self.assertRaises(ContractError):
                cargo_tools.declarations(self.candidate)
        self.metadata.write_bytes((maximum + "\r\n\n").encode("ascii"))
        with (
            patch.object(Path, "open", side_effect=AssertionError("oversized file was opened")),
            self.assertRaises(ContractError),
        ):
            cargo_tools.declarations(self.candidate)

    def test_malformed_nonregular_oversized_and_linked_file_fail_closed(self) -> None:
        for data in (
            b"",
            b"latest",
            b"^1.2.3",
            b"01.2.3",
            b"1.02.3",
            b"1.2.03",
            b"1.2",
            b" 1.2.3",
            b"1.2.3 ",
            b"1.2.3\n\n",
            b"1.2.3\r",
            b"1.2.3\r\r\n",
            b"# 1.2.3\n",
            b"1.2.3 # comment",
            b'"1.2.3"',
            b"\xff",
            "\u0661.2.3".encode(),
            b"\xef\xbb\xbf1.2.3",
            b"1.2.3\x00",
            b"1" * (cargo_tools.MAX_DECLARATION_BYTES + 1),
        ):
            self.metadata.write_bytes(data)
            with self.subTest(data=data[:40]), self.assertRaises(ValueError):
                cargo_tools.select(self.candidate, "cargo-llvm-cov", require_candidate=True)
        self.metadata.unlink()
        self.metadata.mkdir()
        with self.assertRaises(ContractError):
            cargo_tools.declarations(self.candidate)
        self.metadata.rmdir()
        self.metadata.symlink_to(self.candidate / "rust-toolchain.toml")
        with self.assertRaises(ContractError):
            cargo_tools.declarations(self.candidate)
        self.metadata.unlink()
        self.metadata.parent.rmdir()
        self.metadata.parent.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(ContractError):
            cargo_tools.declarations(self.candidate)

    def test_real_image_dockerfile_retains_four_tools_and_refuses_version_file(self) -> None:
        (self.candidate / ".devcontainer").mkdir()
        shutil.copyfile(ROOT / cargo_tools.DOCKERFILE, self.candidate / cargo_tools.DOCKERFILE)
        for data in (b"1.2.3\n", b"invalid"):
            self.metadata.write_bytes(data)
            with self.assertRaisesRegex(ContractError, "image-less"):
                cargo_tools.declarations(self.candidate)
        self.metadata.unlink()
        versions = cargo_tools.declarations(self.candidate)
        self.assertEqual(set(versions), set(cargo_tools.DECLARATIONS))
        for package in ("cargo-llvm-cov", "cargo-semver-checks", "cargo-deny"):
            self.assertEqual(
                cargo_tools.select(self.candidate, package, require_candidate=True).version,
                versions[package],
            )

    def test_public_cli_and_simulated_reader_failure_use_typed_outcomes(self) -> None:
        for entry in (cargo_tools.main, lambda argv: cli.main(["cargo-tools", *argv])):
            argv = [
                "verify",
                "--repository-root",
                str(self.candidate),
                "--package",
                "cargo-llvm-cov",
                "--require-candidate",
            ]
            with (
                patch.object(
                    cargo_tools, "verify", return_value=self.root / "cargo-llvm-cov"
                ) as verify,
                contextlib.redirect_stdout(io.StringIO()) as output,
            ):
                self.assertEqual(entry(argv), 0)
            self.assertEqual(verify.call_args.args[0].version, "1.2.3")
            self.assertEqual(json.loads(output.getvalue())["version"], "1.2.3")
            # Exercise the existing failure boundary without a new candidate TOML parser.
            with (
                patch.object(cargo_tools, "read_data", side_effect=ValueError("simulated reader")),
                patch.object(cargo_tools, "verify") as verify,
                contextlib.redirect_stderr(io.StringIO()) as error,
            ):
                self.assertEqual(entry(argv), 1)
            verify.assert_not_called()
            self.assertIn("Exact Cargo tool selection failed", error.getvalue())
        base = self.root / "base"
        base.mkdir()
        with patch.object(cargo_tools, "read_data", side_effect=ValueError("simulated reader")):
            report = check_dependencies(base, self.candidate, now=NOW)
        self.assertEqual(report["status"], "failure")
        self.assertIn("simulated reader", report["error"])

    def test_remote_and_local_exact_snapshots_track_absence_add_update_remove(self) -> None:
        for before, after in (
            (None, None),
            (None, b"1.2.3\n"),
            (b"1.2.3\n", b"1.2.4\n"),
            (b"1.2.4\n", None),
        ):
            with (
                self.subTest(before=before, after=after),
                tempfile.TemporaryDirectory() as directory,
            ):
                api = recovery_cases.API()
                for revision, data in ((BASE, before), (TESTED, after)):
                    files = {"Cargo.toml": b"[workspace]\n"}
                    if data is not None:
                        files[VERSION_FILE] = data
                    api.files(revision, files)
                changed = before != after
                api.responses[f"compare/{BASE}...{TESTED}?per_page=100"] = {
                    "merge_base_commit": {"sha": BASE},
                    "files": [{"filename": VERSION_FILE}] if changed else [],
                }
                paths = eligibility.changed_paths(
                    api,
                    eligibility.Candidate(
                        BASE, TESTED, 17, TESTED, "renovate[bot]", "Bot", "pull_request"
                    ),
                )
                self.assertEqual(paths, [VERSION_FILE] if changed else [])
                cache = SnapshotCache()
                remote_roots = []
                for revision, data in ((BASE, before), (TESTED, after)):
                    remote = Path(directory) / ("remote-" + revision)
                    remote.mkdir()
                    eligibility.snapshot(api, revision, remote, paths, cache)
                    local = Path(directory) / ("local-" + revision)
                    local.mkdir()

                    def git(
                        _root: Path,
                        *args: str,
                        revision: str = revision,
                        api: recovery_cases.API = api,
                    ) -> bytes:
                        tree = api.responses[f"git/trees/{revision}?recursive=1"]["tree"]
                        if args == ("ls-tree", "-r", "-z", revision):
                            return b"".join(
                                f"{item['mode']} blob {item['sha']}\t{item['path']}\0".encode()
                                for item in tree
                            )
                        if args[0] == "show":
                            path = args[1].split(":", 1)[1]
                            result = eligibility.file_bytes(api, path, revision)
                            assert result is not None
                            return result
                        raise AssertionError("unexpected Git acquisition")

                    with patch.object(cli, "git", side_effect=git):
                        cli._snapshot(self.candidate, revision, local)
                    for root in (remote, local):
                        self.assertEqual((root / VERSION_FILE).exists(), data is not None)
                        if data is not None:
                            self.assertEqual((root / VERSION_FILE).read_bytes(), data)
                    remote_roots.append(remote)
                for timestamp, expected in ((OLD, "pass"), (YOUNG, "waiting")):
                    version = after.decode().strip() if after else ""
                    report = check_dependencies(
                        *remote_roots,
                        now=NOW,
                        context=DependencyContext(
                            "renovate[bot]", "Bot", api.repository, BASE, TESTED, tuple(paths)
                        ),
                        fetch=lambda _, version=version, stamp=timestamp: cargo_metadata(
                            "cargo-llvm-cov", version, stamp
                        ),
                    )
                    self.assertEqual(report["status"], expected if after and changed else "pass")
                    self.assertEqual(len(report["results"]), int(bool(after and changed)))
                    if after and changed:
                        self.assertEqual(report["results"][0]["integrity"], [HASH])
                        self.assertFalse(report["first_party_exempt"])

    def test_new_snapshot_path_rejects_linked_or_corrupt_git_blobs(self) -> None:
        for fault in ("symlink", "tampered"):
            api = recovery_cases.API()
            api.files(BASE, {VERSION_FILE: b"1.2.3\n"})
            tree = api.responses[f"git/trees/{BASE}?recursive=1"]["tree"]
            if fault == "symlink":
                tree[0]["mode"] = "120000"
            else:
                api.responses[f"contents/{VERSION_FILE}?ref={BASE}"]["content"] = "MS4yLjQK"
            target = self.root / fault
            target.mkdir()
            with self.subTest(fault=fault), self.assertRaises(ValueError):
                eligibility.snapshot(api, BASE, target, [VERSION_FILE])
            self.assertFalse((target / VERSION_FILE).exists())

    def test_retained_proof_uses_exact_original_base_and_version_file(self) -> None:
        case = retained_cases.RetainedOriginalTests()
        case.setUp()
        for sha in (recovery_cases.P, recovery_cases.B, recovery_cases.H, recovery_cases.OLD_MERGE):
            version = b"1.2.3\n" if sha in {recovery_cases.P, recovery_cases.B} else b"1.2.4\n"
            case.api.files(sha, {**case.files, VERSION_FILE: version})
        case.api.responses[f"compare/{recovery_cases.P}...{recovery_cases.H}"]["files"] = [
            {"filename": VERSION_FILE, "status": "modified"}
        ]
        previous = case.prepared()
        case.call(previous)
        self.assertEqual(case.effects(), [("actions/runs/50/rerun-failed-jobs", {})])
        self.assertEqual(set(case.reports), {recovery_cases.OLD_MERGE})
        self.assertEqual(case.reports[recovery_cases.OLD_MERGE]["base_sha"], recovery_cases.P)

    def test_publication_age_boundary_integrity_and_unchanged_version(self) -> None:
        base = self.root / "base"
        base.mkdir()
        for metadata in (
            cargo_metadata("other", "1.2.3"),
            cargo_metadata("cargo-llvm-cov", "1.2.3", None),
            cargo_metadata("cargo-llvm-cov", "1.2.3", "2027-01-01T00:00:00Z"),
            cargo_metadata("cargo-llvm-cov", "1.2.3", digest="invalid"),
        ):
            report = check_dependencies(
                base, self.candidate, now=NOW, fetch=lambda _, m=metadata: m
            )
            self.assertEqual(report["status"], "failure")
            self.assertEqual(report["state"]["first_seen"], {})
        for delta, expected in (
            (timedelta(hours=71, seconds=-1), "waiting"),
            (timedelta(hours=71), "pass"),
        ):
            report = check_dependencies(
                base,
                self.candidate,
                now=NOW + delta,
                fetch=lambda _: cargo_metadata("cargo-llvm-cov", "1.2.3", YOUNG),
            )
            self.assertEqual(report["status"], expected)
        (base / "scripts").mkdir()
        (base / VERSION_FILE).write_bytes(b"1.2.3\r\n")
        report = check_dependencies(
            base, self.candidate, now=NOW, fetch=lambda _: self.fail("unchanged version fetched")
        )
        self.assertEqual(report["status"], "pass")
        self.assertEqual(report["results"], [])

    def test_file_add_update_remove_cannot_inherit_first_party_exemption(self) -> None:
        case = dependency_cases.DependencyTests()
        case.setUp()
        self.addCleanup(case.doCleanups)
        case.first_party()
        for previous, current in (
            (None, "1.2.3"),
            ("1.2.3", "1.2.4"),
            ("1.2.3", None),
            ("1.2.3", "1.2.3"),
        ):
            for root, version in ((case.base, previous), (case.head, current)):
                target = root / VERSION_FILE
                target.parent.mkdir(exist_ok=True)
                target.unlink(missing_ok=True)
                if version:
                    target.write_text(version + "\n")
            changed = previous != current
            paths = (
                ("Cargo.toml", "Cargo.lock", VERSION_FILE)
                if changed
                else ("Cargo.toml", "Cargo.lock")
            )
            report = case.check(case.any_cargo, context=case.context(paths=paths))
            self.assertEqual(report["first_party_exempt"], not changed)
            self.assertEqual(report["status"], "waiting" if changed else "pass")

    def test_ordinary_recovery_requires_existing_version_only_file_and_rejects_toml_scope(
        self,
    ) -> None:
        api = recovery_cases.API()
        candidate = eligibility.Candidate(
            BASE, TESTED, 17, TESTED, "renovate[bot]", "Bot", "pull_request"
        )
        bot = {"login": "renovate[bot]", "type": "Bot"}
        commit = {"sha": TESTED, "author": bot, "committer": bot}
        api.responses[f"commits/{TESTED}"] = commit
        comparison = {
            "commits": [commit],
            "total_commits": 1,
            "ahead_by": 1,
            "files": [{"filename": VERSION_FILE, "status": "modified"}],
        }
        for previous, current, accepted in (
            (b"1.2.3\n", b"1.2.4\n", True),
            (b"1.2.3", b"1.2.4", True),
            (b"1.2.3\r\n", b"1.2.4\r\n", True),
            (None, b"1.2.4\n", False),
            (b"1.2.3\n", None, False),
            (b"1.2.3\n", b"1.2.4\r\n", False),
            (b"1.2.3\n", b"1.2.3\n", False),
            (b"1.2.3\n", b"latest\n", False),
            (b"1.2.3\n", b"1.2.4\n\n", False),
        ):
            for revision, data in ((BASE, previous), (TESTED, current)):
                api.files(revision, {VERSION_FILE: data} if data is not None else {})
            modes: dict[str, dict[str, str]] = {BASE: {}, TESTED: {}}
            trees = {sha: recovery.inventory(api, sha, modes=modes[sha]) for sha in modes}
            with self.subTest(previous=previous, current=current):
                if accepted:
                    recovery.ordinary_native_changes(api, comparison, candidate, BASE, trees, modes)
                else:
                    with self.assertRaises(recovery.RecoveryRefused):
                        recovery.ordinary_native_changes(
                            api, comparison, candidate, BASE, trees, modes
                        )
        api.files(BASE, {VERSION_FILE: b"1.2.3\n"})
        api.files(TESTED, {VERSION_FILE: b"1.2.4\n"})
        modes = {BASE: {}, TESTED: {}}
        trees = {sha: recovery.inventory(api, sha, modes=modes[sha]) for sha in modes}
        for fault in ("mode", "rename", "image", "parser"):
            with self.subTest(fault=fault):
                changed_modes = {sha: dict(values) for sha, values in modes.items()}
                changed_trees = {sha: dict(values) for sha, values in trees.items()}
                changed_comparison = dict(comparison)
                if fault == "mode":
                    changed_modes[TESTED][VERSION_FILE] = "100755"
                elif fault == "rename":
                    changed_comparison["files"] = [
                        {
                            "filename": VERSION_FILE,
                            "status": "renamed",
                            "previous_filename": "other",
                        }
                    ]
                elif fault == "image":
                    changed_trees[TESTED][cargo_tools.DOCKERFILE] = "c" * 40
                with contextlib.ExitStack() as stack:
                    if fault == "parser":
                        stack.enter_context(
                            patch.object(
                                cargo_tools,
                                "coverage_version",
                                side_effect=ValueError("simulated parser"),
                            )
                        )
                    with self.assertRaises(recovery.RecoveryRefused):
                        recovery.ordinary_native_changes(
                            api, changed_comparison, candidate, BASE, changed_trees, changed_modes
                        )
        old = {"workspace": {"metadata": {"boxferry-ci": {"cargo-llvm-cov": "1.2.3"}}}}
        new = {"workspace": {"metadata": {"boxferry-ci": {"cargo-llvm-cov": "1.2.4"}}}}
        self.assertFalse(recovery.version_only(old, new))
        self.assertTrue(
            recovery.version_only(
                {"dependencies": {"serde": "1.0.0"}}, {"dependencies": {"serde": "1.0.1"}}
            )
        )

    def test_normal_planner_and_required_ci_keep_coverage_and_age_selected(self) -> None:
        policy = load_policy(ROOT / "scripts/shared-validation-policy.json")
        for status in ("A", "M", "D"):
            diff = f"{status}\0{VERSION_FILE}\0".encode()

            def git(_root: Path, *args: str, diff: bytes = diff, **_kwargs: object) -> bytes:
                if args[0] == "diff":
                    return diff
                if args[0] == "rev-parse":
                    return str(ROOT).encode()
                if args[0] == "ls-tree":
                    return b"100644 blob " + b"a" * 40 + b"\towned\0"
                if args[0] == "show":
                    revision, path = args[1].split(":", 1)
                    if path == VERSION_FILE:
                        return b"1.2.3\n" if revision == BASE else b"1.2.4\n"
                    return (ROOT / path).read_bytes()
                if args[0] == "merge-base":
                    return b""
                raise AssertionError("unexpected planner Git call")

            with patch("boxferry_ci.planner.git", side_effect=git):
                plan = make_plan(
                    self.candidate,
                    policy,
                    event="pull_request",
                    base_sha=BASE,
                    head_sha=TESTED,
                    tested_sha=TESTED,
                    policy_sha=BASE,
                )
            self.assertEqual(plan["profiles"], ["full"])
            self.assertNotIn("classification unavailable", str(plan["reasons"]))
            self.assertTrue(object_value(plan["jobs"], "jobs")["coverage"])
            self.assertTrue(object_value(plan["jobs"], "jobs")["lockfile-release-age"])
            needs = {job: {"result": "success"} for job in policy.jobs}
            needs["validation-plan"] = {"result": "success"}
            comparison = Comparison("pull_request", BASE, TESTED, TESTED)
            verify_gate(plan, needs, policy, comparison, expected_plan=plan)
            for job in ("coverage", "lockfile-release-age"):
                blocked = {**needs, job: {"result": "skipped"}}
                with self.assertRaises(ContractError):
                    verify_gate(plan, blocked, policy, comparison, expected_plan=plan)


class NativeImagelessCoverage(unittest.TestCase):
    def test_native_exact_file_owner_replacement_policy_and_other_paths(self) -> None:
        runtime = Path(os.environ.get("BOXFERRY_RENOVATE_PACKAGE", ROOT / "target/tools/renovate"))
        presets._validate_native_runtime(runtime)
        script = r"""
(async () => {
  const fs=require("fs"), assert=require("assert/strict"), os=require("os");
  const base=process.argv[1]+"/dist/", root=process.argv[2];
  await require(base+"logger/index.js").init();
  const {extractPackageFile:extract}=require(base+"modules/manager/custom/regex/index.js");
  const cargo=require(base+"modules/manager/cargo/index.js");
  const versioner=require(base+"modules/versioning/semver/index.js").api;
  const cases=JSON.parse(fs.readFileSync(0,"utf8"));
  const {doAutoReplace}=require(base+"workers/repository/update/branch/auto-replace.js");
  const {applyPackageRules}=require(base+"util/package-rules/index.js");
  const {resolveConfigPresets}=require(base+"config/presets/index.js");
  const {GlobalConfig}=require(base+"config/global.js");
  const {matchRegexOrGlobList}=require(base+"util/string-match.js");
  const temporary=fs.mkdtempSync(os.tmpdir()+"/boxferry-imageless-owner-");
  GlobalConfig.set({localDir:temporary});
  try {
    const read=p=>JSON.parse(fs.readFileSync(root+p));
    const common=read("/renovate-presets/common.json"), rust=read("/renovate-presets/rust.json");
    const local=read("/.github/renovate.json");
    const {config}=await resolveConfigPresets({...common,...local,extends:common.extends,
      customManagers:[...common.customManagers,...local.customManagers],
      packageRules:[...common.packageRules,...rust.packageRules,...local.packageRules]});
    const file="scripts/cargo-llvm-cov.version";
    const owners=config.customManagers.filter(
      m=>matchRegexOrGlobList(file,m.managerFilePatterns||[]));
    assert.equal(owners.length,1,"one exact-path owner");
    const manager=owners[0];
    fs.mkdirSync(temporary+"/scripts",{recursive:true});
    const neighbors={"Cargo.toml":'[workspace.dependencies]\nserde="1.0.228"\n',
      "scripts/other.version":"7.8.9\n"};
    for(const [path,content] of Object.entries(neighbors))
      fs.writeFileSync(temporary+"/"+path,content);
    for(const content of cases.accepted) {
      const result=extract(content,file,manager);
      assert.equal(result.deps.length,1);
      assert.equal(result.deps[0].currentValue,content.trim());
      assert.equal(versioner.isValid(result.deps[0].currentValue),true,"native valid version");
      assert.equal(versioner.isVersion(result.deps[0].currentValue),true,"native exact version");
      assert.equal(versioner.isVersion("1.2.4"),true,"native valid replacement");
      const updated=await doAutoReplace({...manager,...result.deps[0],manager:"regex",depIndex:0,
        packageFile:file,newValue:"1.2.4"},content,false);
      assert.equal(updated,content.replace(content.trim(),"1.2.4"),"only the version changes");
    }
    for(const content of [...cases.rejected,"","01.2.3","1.02.3","1.2.03","1.2.3\n\n","1.2.3\r",
      " 1.2.3","1.2.3 ","# 1.2.3\n","1.2.3 # comment","1.2.3\n2.3.4",
      '"1.2.3"',"\u0661.2.3","\ufeff1.2.3","1.2.3\x00"]) {
      assert.equal((extract(content,file,manager)?.deps||[]).length,0,JSON.stringify(content));
    }
    for(const path of ["Cargo.toml","history/scripts/cargo-llvm-cov.version",
      "scripts/cargo-llvm-cov.version.bak","scripts/other.version","fixtures/cargo-llvm-cov.version"])
      assert.equal(matchRegexOrGlobList(path,manager.managerFilePatterns||[]),false,path);
    const native=await cargo.extractPackageFile('[workspace.dependencies]\nserde="1.0.228"\n',
      "Cargo.toml",{});
    assert.equal(native.deps.filter(d=>d.depName==="serde").length,1);
    const policy=await applyPackageRules({...config,manager:"regex",datasource:"crate",
      packageName:"cargo-llvm-cov",depName:"cargo-llvm-cov",packageFile:file,
      updateType:"patch"},config.packageRules);
    assert.notEqual(policy.enabled,false);
    assert.equal(policy.minimumReleaseAge,"3 days");
    assert.equal(policy.automerge,true);
    assert.equal(policy.dependencyDashboardApproval,false);
    assert.notEqual(policy.groupName,"First-party Rust releases");
    const offered=require(base+"util/string-match.js").getRegexPredicate(policy.allowedVersions);
    assert.ok(offered,"offered-version rule uses the same bounded numeric domain");
    for(const content of cases.accepted) assert.equal(offered(content.trim()),true,content);
    for(const content of cases.rejected) assert.equal(offered(content),false,content);
    for(const [path,content] of Object.entries(neighbors))
      assert.equal(fs.readFileSync(temporary+"/"+path,"utf8"),content,"other files unchanged");
    console.log(JSON.stringify({owners:1,exactReplacement:true,ordinaryCargo:1,
      accepted:cases.accepted.length,versionerAccepted:cases.accepted.length,
      thirdPartyAge:"3 days",automerge:true,dashboardApproval:false}));
  } finally {fs.rmSync(temporary,{recursive:true,force:true});}
})().catch(error=>{console.error(error);process.exit(1)});
"""
        result = subprocess.run(
            [
                str(runtime / "node"),
                "--experimental-vm-modules",
                "-e",
                script,
                str(runtime),
                str(ROOT),
            ],
            input=json.dumps(
                {
                    "accepted": [
                        version + ending
                        for version in COVERAGE_VALID_VERSIONS
                        for ending in ("", "\n", "\r\n")
                    ],
                    "rejected": list(COVERAGE_OUTSIDE_DOMAIN),
                }
            ),
            capture_output=True,
            text=True,
            timeout=180,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(json.loads(result.stdout.splitlines()[-1])["owners"], 1)

    def test_native_registry_lookup_keeps_offered_versions_in_reader_domain(self) -> None:
        runtime = Path(os.environ.get("BOXFERRY_RENOVATE_PACKAGE", ROOT / "target/tools/renovate"))
        presets._validate_native_runtime(runtime)
        fixture = ROOT / "automation/tests/fixtures/imageless-coverage-lookup"
        provenance = json.loads((fixture / "provenance.json").read_text())
        for record in provenance["files"]:
            self.assertEqual(
                hashlib.sha256((fixture / record["fixture"]).read_bytes()).hexdigest(),
                record["sha256"],
            )
        result = subprocess.run(
            [
                str(runtime / "node"),
                "--experimental-vm-modules",
                str(ROOT / "automation/tests/imageless_coverage_lookup.cjs"),
                str(runtime),
                str(ROOT),
            ],
            capture_output=True,
            text=True,
            timeout=180,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        report = json.loads(result.stdout.splitlines()[-1])
        cases = {record["name"]: record for record in report["cases"]}
        self.assertEqual(cases["competing"]["selected"], "0.9.1")
        self.assertIsNone(cases["outside-only"]["selected"])
        self.assertEqual(cases["image-neighbor"]["selected"], "0.9.1000000")
        for name in ("competing", "build-normalized"):
            written = cases[name]["written"]
            self.assertEqual(cargo_tools.coverage_version(written), "0.9.1")
            with tempfile.TemporaryDirectory() as directory:
                base, head = (Path(directory) / name for name in ("base", "head"))
                for root, data in ((base, "0.9.0\n"), (head, written)):
                    (root / "scripts").mkdir(parents=True)
                    (root / VERSION_FILE).write_text(data, newline="")
                self.assertEqual(cargo_tools.declarations(head), {"cargo-llvm-cov": "0.9.1"})
                for metadata, expected in (
                    (cargo_metadata("cargo-llvm-cov", "0.9.1", OLD), "pass"),
                    (cargo_metadata("cargo-llvm-cov", "0.9.1", YOUNG), "waiting"),
                    (cargo_metadata("cargo-llvm-cov", "0.9.1", digest="invalid"), "failure"),
                    (cargo_metadata("cargo-llvm-cov", "0.9.1+rebuilt.1"), "failure"),
                ):
                    acquired: list[str] = []

                    def fetch(
                        url: str, metadata: bytes = metadata, acquired: list[str] = acquired
                    ) -> bytes:
                        acquired.append(url)
                        return metadata

                    outcome = check_dependencies(base, head, now=NOW, fetch=fetch)
                    self.assertEqual(
                        acquired, ["https://crates.io/api/v1/crates/cargo-llvm-cov/0.9.1"]
                    )
                    self.assertEqual(outcome["status"], expected)
                    self.assertFalse(outcome["first_party_exempt"])
                    if expected == "pass":
                        self.assertEqual(outcome["results"][0]["integrity"], [HASH])
                    if b"0.9.1+rebuilt.1" in metadata:
                        self.assertIn("identity mismatch", outcome["results"][0]["reason"])


if __name__ == "__main__":
    unittest.main()
