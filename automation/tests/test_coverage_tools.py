"""Candidate coverage authority, actual Cargo dispatch and native updater ownership."""

from __future__ import annotations

import contextlib
import io
import json
import os
import re
import shutil
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path
from unittest.mock import patch

import test_presets as presets

from boxferry_ci import cargo_tools
from boxferry_ci.contracts import ContractError

ROOT = Path(__file__).resolve().parents[2]


class CoverageTools(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.candidate = self.root / "source"
        (self.candidate / ".devcontainer").mkdir(parents=True)
        self.metadata = self.candidate / cargo_tools.DOCKERFILE
        self.metadata.write_text(
            "# renovate: datasource=crate depName=cargo-llvm-cov\n"
            "ARG CARGO_LLVM_COV_VERSION=1.2.3\n"
        )
        shutil.copyfile(ROOT / "rust-toolchain.toml", self.candidate / "rust-toolchain.toml")

    def test_candidate_version_boundaries_and_missing_conflicting_metadata(self) -> None:
        original = self.metadata.read_text()
        fallback = self.root / "automation"
        (fallback / ".devcontainer").mkdir(parents=True)
        (fallback / cargo_tools.DOCKERFILE).write_text(original)
        for version in ("0.9.0", "0.10.0", "1.0.0", "9.99.999"):
            self.metadata.write_text(original.replace("1.2.3", version))
            self.assertEqual(
                cargo_tools.select(
                    self.candidate, "cargo-llvm-cov", fallback, require_candidate=True
                ).version,
                version,
            )
        for invalid in (
            "",
            "FROM invalid\n",
            original * 2,
            original + "ARG CARGO_LLVM_COV_VERSION=1.2.4\n",
            original.replace("1.2.3", "latest"),
            original.replace("1.2.3", "^1.2.3"),
            original.replace("1.2.3", "01.2.3"),
            original.replace("1.2.3", "1.2"),
            original.replace("depName=cargo-llvm-cov", "depName=other"),
        ):
            self.metadata.write_text(invalid)
            with self.subTest(invalid=invalid), self.assertRaises(ContractError):
                cargo_tools.select(
                    self.candidate, "cargo-llvm-cov", fallback, require_candidate=True
                )
        self.metadata.unlink()
        with self.assertRaisesRegex(ContractError, "candidate Dockerfile"):
            cargo_tools.select(self.candidate, "cargo-llvm-cov", fallback, require_candidate=True)

    def test_real_cargo_dispatch_precedence_version_mismatch_and_alias_conflict(self) -> None:
        selection = cargo_tools.select(self.candidate, "cargo-llvm-cov", require_candidate=True)
        binary = self.root / "bin"
        binary.mkdir()
        plugin = binary / "cargo-llvm-cov"
        plugin.write_text(
            '#!/bin/sh\n[ "$#" -eq 2 ] && [ "$1" = llvm-cov ] && [ "$2" = --version ] '
            "|| exit 17\nprintf 'cargo-llvm-cov 1.2.3\\n'\n"
        )
        plugin.chmod(0o755)
        environment = {
            **os.environ,
            "PATH": str(binary) + os.pathsep + os.environ["PATH"],
            "CARGO_HOME": str(self.root / "home"),
        }
        self.assertEqual(cargo_tools.verify(selection, self.candidate, environment), plugin)
        later = self.root / "later"
        later.mkdir()
        wrong = later / plugin.name
        wrong.write_text("#!/bin/sh\nprintf 'cargo-llvm-cov 1.2.4\\n'\n")
        wrong.chmod(0o755)
        environment["PATH"] += os.pathsep + str(later)
        self.assertEqual(cargo_tools.verify(selection, self.candidate, environment), plugin)
        with self.assertRaisesRegex(ContractError, "declared"):
            cargo_tools.verify(
                selection,
                self.candidate,
                {**environment, "PATH": str(later) + os.pathsep + environment["PATH"]},
            )
        shadow = Path(environment["CARGO_HOME"]) / "bin" / plugin.name
        shadow.parent.mkdir(parents=True)
        shutil.copyfile(plugin, shadow)
        with self.assertRaisesRegex(ContractError, "Cargo home shadows"):
            cargo_tools.verify(selection, self.candidate, environment)
        shadow.unlink()
        (self.candidate / ".cargo").mkdir()
        (self.candidate / ".cargo/config.toml").write_text('[alias]\nllvm-cov="!false"\n')
        with self.assertRaisesRegex(ContractError, "overrides"):
            cargo_tools.verify(selection, self.candidate, environment)

    def test_installation_exports_only_successful_verified_selection(self) -> None:
        values = {
            "executable": str(self.root / "owned/bin/cargo-llvm-cov"),
            "bin_directory": str(self.root / "owned/bin"),
            "CARGO_HOME": str(self.root / "owned"),
            "RUSTUP_TOOLCHAIN": cargo_tools.select(
                self.candidate, "cargo-llvm-cov", require_candidate=True
            ).rust_channel,
        }
        paths = {name: self.root / name for name in ("output", "path", "env")}
        argv = [
            "install",
            "--repository-root",
            str(self.candidate),
            "--package",
            "cargo-llvm-cov",
            "--require-candidate",
            "--destination",
            str(self.root / "owned"),
            "--github-output",
            str(paths["output"]),
            "--github-path",
            str(paths["path"]),
            "--github-env",
            str(paths["env"]),
        ]
        with (
            patch.object(cargo_tools, "install", return_value=values) as install,
            patch.object(Path, "cwd", return_value=self.candidate),
            contextlib.redirect_stdout(io.StringIO()),
        ):
            self.assertEqual(cargo_tools.main(argv), 0)
        selection = install.call_args.args[0]
        self.assertEqual(selection.version, "1.2.3")
        self.assertEqual(install.call_args.args[2], self.candidate)
        self.assertEqual(paths["path"].read_text(), values["bin_directory"] + "\n")
        self.assertEqual(paths["output"].read_text(), f"rust_channel={selection.rust_channel}\n")
        self.assertEqual(
            paths["env"].read_text(),
            f"CARGO_HOME={values['CARGO_HOME']}\nRUSTUP_TOOLCHAIN={values['RUSTUP_TOOLCHAIN']}\n",
        )
        for output in paths.values():
            output.unlink()
        for reason in ("installation failed", "version mismatch", "PATH conflict"):
            with (
                patch.object(cargo_tools, "install", side_effect=ContractError(reason)),
                contextlib.redirect_stderr(io.StringIO()),
            ):
                self.assertEqual(cargo_tools.main(argv), 1)
            self.assertTrue(all(not path.exists() for path in paths.values()))
        self.metadata.unlink()
        with (
            patch.object(cargo_tools, "install") as install,
            contextlib.redirect_stderr(io.StringIO()),
        ):
            self.assertEqual(cargo_tools.main(argv), 1)
        install.assert_not_called()

    def test_workflow_uses_verified_candidate_environment_without_new_inputs(self) -> None:
        workflow = (ROOT / ".github/workflows/shared-rust-ci.yml").read_text()
        inputs = workflow.split("    inputs:\n", 1)[1].split("    outputs:\n", 1)[0]
        self.assertEqual(
            re.findall(r"(?m)^      ([a-z_]+):$", inputs),
            ["candidate_sha", "automation_sha", "policy_sha", "config_path", "task"],
        )
        self.assertIn("timeout-minutes: 30", workflow)
        self.assertIn("RUSTDOCFLAGS: -D warnings", workflow)
        self.assertNotIn("cargo install --locked --version", workflow)
        self.assertNotIn("depName=cargo-llvm-cov", workflow)
        blocks = re.split(r"(?m)^      - name: ", workflow)[1:]
        selected = [block for block in blocks if "cargo_tools" in block]
        self.assertEqual(len(selected), 2)
        for block in selected:
            self.assertIn("if: inputs.task == 'coverage'", block)
            self.assertIn("working-directory: source", block)
            self.assertIn("--repository-root . --package cargo-llvm-cov --require-candidate", block)
            self.assertIn("--project ../_automation/automation", block)
        install, verify = selected
        self.assertIn("cargo_tools install", install)
        self.assertIn('--github-env "${GITHUB_ENV}"', install)
        self.assertIn('--github-path "${GITHUB_PATH}"', install)
        self.assertIn("cargo_tools verify", verify)
        self.assertLess(
            workflow.index(install), workflow.index("rustup component add llvm-tools-preview")
        )
        self.assertLess(workflow.index(verify), workflow.index("python -m boxferry_ci run"))
        self.assertIn("RUST_CHANNEL: ${{ steps.coverage-tool.outputs.rust_channel }}", workflow)
        self.assertIn('--toolchain "${RUST_CHANNEL}"', workflow)

    def test_static_coverage_owner_is_unique_automatic_and_other_image_holds_remain(self) -> None:
        config = presets.consumer_config(ROOT / ".github/renovate.json")
        source = (ROOT / ".devcontainer/Dockerfile").read_text()
        owners = []
        for manager in config["customManagers"]:
            if not presets._matches(
                ".devcontainer/Dockerfile", manager["managerFilePatterns"], file=True
            ):
                continue
            for pattern in manager["matchStrings"]:
                pattern = re.sub(r"\(\?<([A-Za-z][A-Za-z0-9_]*)>", r"(?P<\1>", pattern)
                owners.extend(
                    manager["description"]
                    for match in re.finditer(pattern, source)
                    if match.groupdict().get("depName") == "cargo-llvm-cov"
                )
        self.assertEqual(len(owners), 1, owners)
        for package in ("cargo-llvm-cov", "cargo-deny", "cargo-semver-checks", "zizmor"):
            result = presets.effective(
                config,
                {
                    "manager": "custom.regex",
                    "datasource": "crate",
                    "packageName": package,
                    "depName": package,
                    "fileName": ".devcontainer/Dockerfile",
                    "updateType": "patch",
                },
            )
            self.assertEqual(result.get("enabled", True), package == "cargo-llvm-cov")
            self.assertTrue(result["automerge"])
            self.assertFalse(result["dependencyDashboardApproval"])
            self.assertEqual(result["minimumReleaseAge"], "3 days")
        local = json.loads((ROOT / ".github/renovate.json").read_text())
        manager = next(
            m
            for m in local["customManagers"]
            if m["description"] == "Update directly pinned workflow tool versions"
        )
        self.assertEqual(len(manager["matchStrings"]), 2)
        self.assertTrue(all("cargo install" not in pattern for pattern in manager["matchStrings"]))


class NativeCoverageOwnership(unittest.TestCase):
    def test_native_coverage_arg_extraction_replacement_and_effective_policy(self) -> None:
        runtime = Path(
            os.environ.get("BOXFERRY_RENOVATE_PACKAGE", ROOT / "target/tools/renovate")
        ).resolve()
        presets._validate_native_runtime(runtime)
        script = r"""
(async () => {
  const fs=require("fs"), os=require("os"), assert=require("assert/strict");
  const base=process.argv[1]+"/dist/", root=process.argv[2];
  await require(base+"logger/index.js").init();
  const {extractPackageFile:extract}=require(base+"modules/manager/custom/regex/index.js");
  const {doAutoReplace}=require(base+"workers/repository/update/branch/auto-replace.js");
  const {applyPackageRules}=require(base+"util/package-rules/index.js");
  const {resolveConfigPresets}=require(base+"config/presets/index.js");
  const {GlobalConfig}=require(base+"config/global.js");
  const {matchRegexOrGlobList}=require(base+"util/string-match.js");
  const temporary=fs.mkdtempSync(os.tmpdir()+"/boxferry-coverage-owner-");
  GlobalConfig.set({localDir:temporary});
  try {
    const local=JSON.parse(fs.readFileSync(root+"/.github/renovate.json"));
    const {config}=await resolveConfigPresets(local);
    const file=".devcontainer/Dockerfile", original=fs.readFileSync(root+"/"+file,"utf8");
    const matches=[];
    for(const manager of config.customManagers || []) {
      if(!matchRegexOrGlobList(file,manager.managerFilePatterns || [])) continue;
      const result=extract(original,file,manager);
      for(const [index,dep] of (result?.deps || []).entries())
        if(dep.depName==="cargo-llvm-cov") matches.push({manager,dep,index});
    }
    assert.equal(matches.length,1,"one native extraction owner");
    const {manager,dep,index}=matches[0], next="99.7.3";
    assert.equal(dep.datasource,"crate");
    const updated=await doAutoReplace({...manager,...dep,manager:"regex",depIndex:index,
      packageFile:file,newValue:next},original,false);
    assert.equal(updated,original.replace("ARG CARGO_LLVM_COV_VERSION="+dep.currentValue,
      "ARG CARGO_LLVM_COV_VERSION="+next),"only existing declaration may change");
    const old="# renovate: datasource=crate depName=cargo-llvm-cov\n"+
      "        run: cargo install --locked --version 1.2.3 cargo-llvm-cov\n";
    for(const item of config.customManagers || []) {
      const workflow=".github/workflows/shared-rust-ci.yml";
      if(!matchRegexOrGlobList(workflow,item.managerFilePatterns || [])) continue;
      assert.equal((extract(old,".github/workflows/shared-rust-ci.yml",item)?.deps || [])
        .filter(d=>d.depName==="cargo-llvm-cov").length,0,"obsolete install route removed");
    }
    for(const [name,datasource,enabled] of [["cargo-llvm-cov","crate",true],
      ["cargo-deny","crate",false],["cargo-semver-checks","crate",false],
      ["zizmor","crate",false],["docker.io/library/node","docker",false],
      ["mcr.microsoft.com/devcontainers/rust","docker",false]]) {
      const policy=await applyPackageRules({...config,manager:"regex",datasource,
        packageName:name,depName:name,packageFile:file,updateType:"patch"},config.packageRules);
      assert.equal(policy.enabled!==false,enabled,name);
      assert.equal(policy.minimumReleaseAge,"3 days",name);
      assert.equal(policy.automerge,true,name);
      assert.equal(policy.dependencyDashboardApproval,false,name);
    }
    console.log(JSON.stringify({coverageOwners:matches.length,exactReplacement:true,
      obsoleteInstallOwners:0,preservedHolds:5}));
  } finally { fs.rmSync(temporary,{recursive:true,force:true}); }
})().catch(error=>{console.error(error);process.exit(1)});
"""
        result = subprocess.run(
            [
                "node",
                "--experimental-vm-modules",
                "-e",
                textwrap.dedent(script),
                str(runtime),
                str(ROOT),
            ],
            capture_output=True,
            text=True,
            timeout=180,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        report = json.loads(result.stdout.splitlines()[-1])
        self.assertEqual(report["coverageOwners"], 1)
        self.assertTrue(report["exactReplacement"])


if __name__ == "__main__":
    unittest.main()
