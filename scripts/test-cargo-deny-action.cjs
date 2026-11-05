// Existing pinned Renovate supplies YAML parsing and native Action extraction.
(async () => {
  const assert = require("node:assert/strict");
  const fs = require("node:fs");
  const [runtime, root] = process.argv.slice(2);
  const yaml = require(require.resolve("yaml", { paths: [runtime] }));
  const base = runtime + "/dist/";
  await require(base + "logger/index.js").init();
  const { GlobalConfig } = require(base + "config/global.js");
  GlobalConfig.set({ localDir: root });
  const { extractPackageFile } = require(base + "modules/manager/github-actions/index.js");
  const paths = [".github/workflows/shared-rust-dependencies.yml", ".github/workflows/ci.yml"];
  const contents = paths.map((file) => fs.readFileSync(root + "/" + file, "utf8"));
  const [shared, ci] = contents.map((text) => yaml.parse(text));
  const actionName = "EmbarkStudios/cargo-deny-action";
  const auditName = "Audit dependencies, licenses, bans, and sources";
  const fixtureName = "Audit nested dependency-action fixture with explicit policy";
  const prepareName = "Prepare nested dependency-action fixture";
  const step = (job, name) => {
    const matches = job.steps.filter((item) => item.name === name);
    assert.equal(matches.length, 1, name);
    return matches[0];
  };
  const productionAction = step(ci.jobs.dependencies, auditName).uses;
  assert.match(productionAction, /^EmbarkStudios\/cargo-deny-action@[a-f0-9]{40}$/);

  function validate(candidateShared, candidateCi) {
    const job = candidateCi.jobs.dependencies;
    assert.equal(job.needs, "validation-plan");
    assert.equal(job.if, "needs.validation-plan.outputs.select_dependencies == 'true'");
    assert.equal(job["timeout-minutes"], 15);
    const preparation = step(job, prepareName);
    assert.equal(
      preparation.run,
      "cp -R automation/tests/fixtures/cargo-deny-action/source source",
    );
    assert.equal(preparation.if, undefined);
    assert.equal(preparation["continue-on-error"], undefined);
    assert.ok(job.steps.indexOf(preparation) < job.steps.indexOf(step(job, fixtureName)));
    for (const [audit, config] of [
      [step(candidateShared.jobs.check, auditName), "source/deny.toml"],
      [step(job, fixtureName), "source/explicit-policy.toml"],
    ]) {
      assert.equal(audit.uses, productionAction);
      assert.equal(audit.if, undefined);
      assert.equal(audit["continue-on-error"], undefined);
      assert.deepEqual(audit.with, {
        "manifest-path": "source/Cargo.toml",
        command: "check",
        arguments: `--all-features --config ${config}`,
      });
      // Empty check-specific argv means all four default audit categories.
      assert.equal(audit.with["command-arguments"], undefined);
    }
    assert.deepEqual(candidateShared.permissions, { contents: "read" });
    assert.equal(candidateShared.jobs.check["timeout-minutes"], 15);
    assert.equal(
      candidateShared.jobs.check.outputs.candidate_sha,
      "${{ steps.result.outputs.candidate_sha }}",
    );
    assert.equal(
      step(candidateShared.jobs.check, "Expose successful candidate binding").id,
      "result",
    );
  }
  validate(shared, ci);
  for (const mutate of [
    (s) => {
      const audit = step(s.jobs.check, auditName);
      audit.with.arguments = "--all-features";
      audit.with["command-arguments"] = "--config source/deny.toml";
    },
    (_, c) => {
      step(c.jobs.dependencies, fixtureName).with.arguments = "--all-features";
    },
    (_, c) => {
      step(c.jobs.dependencies, fixtureName).with["manifest-path"] = "Cargo.toml";
    },
    (_, c) => {
      step(c.jobs.dependencies, fixtureName).with["command-arguments"] = "licenses";
    },
    (_, c) => {
      step(c.jobs.dependencies, fixtureName).uses = actionName + "@" + "a".repeat(40);
    },
    (_, c) => {
      step(c.jobs.dependencies, fixtureName).if = "false";
    },
    (_, c) => {
      step(c.jobs.dependencies, prepareName)["continue-on-error"] = true;
    },
  ]) {
    const proposedShared = structuredClone(shared),
      proposedCi = structuredClone(ci);
    mutate(proposedShared, proposedCi);
    assert.throws(() => validate(proposedShared, proposedCi));
  }
  const config = JSON.parse(fs.readFileSync(root + "/.github/renovate.json", "utf8"));
  assert.notEqual(config["github-actions"]?.enabled, false);
  const { extractPackageFile: regexExtract } = require(
    base + "modules/manager/custom/regex/index.js",
  );
  for (const [index, file] of paths.entries()) {
    const extracted = await extractPackageFile(contents[index], file, {});
    const pins = extracted.deps.filter((dep) => dep.depName === actionName);
    assert.equal(pins.length, index === 0 ? 1 : 2, file);
    for (const pin of pins) {
      assert.equal(pin.currentDigest || pin.currentValue, productionAction.split("@")[1]);
    }
    for (const manager of config.customManagers) {
      if (
        !manager.managerFilePatterns.some((pattern) => new RegExp(pattern.slice(1, -1)).test(file))
      )
        continue;
      const custom = regexExtract(contents[index], file, manager);
      assert.ok(
        !(custom?.deps || []).some((dep) => dep.depName === actionName),
        "duplicate custom Action owner",
      );
    }
  }
  console.log(
    "Nested dependency Action parity, seven failure controls and unique native pin ownership passed.",
  );
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
