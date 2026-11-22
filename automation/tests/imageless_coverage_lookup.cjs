// Authored registry-driven regression for the pinned native Renovate implementation.
(async () => {
  const fs = require("fs"),
    assert = require("assert/strict"),
    os = require("os");
  const base = process.argv[2] + "/dist/",
    root = process.argv[3];
  await require(base + "logger/index.js").init();
  const fixture = root + "/automation/tests/fixtures/imageless-coverage-lookup/";
  const read = (path) => JSON.parse(fs.readFileSync(path, "utf8"));
  const common = read(root + "/renovate-presets/common.json");
  const rust = read(root + "/renovate-presets/rust.json");
  const local = read(fixture + "kubernetes-lens-renovate.json.txt");
  const { resolveConfigPresets } = require(base + "config/presets/index.js");
  const { mergeChildConfig } = require(base + "config/utils.js");
  const { getConfig } = require(base + "config/defaults.js");
  const { applyPackageRules } = require(base + "util/package-rules/index.js");
  const { extractPackageFile } = require(base + "modules/manager/custom/regex/index.js");
  const { lookupUpdates } = require(base + "workers/repository/process/lookup/index.js");
  const { doAutoReplace } = require(base + "workers/repository/update/branch/auto-replace.js");
  const { getDatasourceFor } = require(base + "modules/datasource/common.js");
  const { GlobalConfig } = require(base + "config/global.js");
  const memory = require(base + "util/cache/memory/index.js");
  const { config: resolved } = await resolveConfigPresets({
    ...common,
    ...local,
    extends: common.extends,
    customManagers: [...common.customManagers, ...local.customManagers],
    packageRules: [...common.packageRules, ...rust.packageRules, ...local.packageRules],
  });
  const config = mergeChildConfig(getConfig(), resolved);
  const file = "scripts/cargo-llvm-cov.version";
  const manager = config.customManagers.find(
    (m) =>
      m.description === "Own the single candidate coverage version file for image-less callers.",
  );
  const original = fs.readFileSync(fixture + "cargo-llvm-cov.version.txt", "utf8");
  assert.equal(original, "0.9.0\n");
  const extracted = extractPackageFile(original, file, manager).deps;
  assert.equal(extracted.length, 1);
  const dependency = {
    ...config,
    ...extracted[0],
    packageName: "cargo-llvm-cov",
    manager: "regex",
    packageFile: file,
  };
  const policy = await applyPackageRules(dependency, "test-dependency");
  assert.equal(policy.minimumReleaseAge, "3 days");
  assert.equal(policy.minimumReleaseAgeBehaviour, "timestamp-required");
  assert.equal(policy.updatePinnedDependencies, true);
  assert.equal(policy.ignoreUnstable, true);
  assert.ok(policy.allowedVersions);
  for (const changed of [
    { packageFile: ".devcontainer/Dockerfile" },
    { packageFile: "nested/" + file },
    { manager: "cargo" },
    { datasource: "npm" },
    { packageName: "serde" },
    { depName: "serde" },
  ]) {
    const other = await applyPackageRules({ ...dependency, ...changed }, "test-neighbor");
    assert.equal(other.allowedVersions, undefined, JSON.stringify(changed));
  }
  const datasource = getDatasourceFor("crate");
  const rows = read(fixture + "registry-rows.json");
  let offered = rows;
  const acquired = [];
  // Only acquisition and registry caches are substituted; normalization and lookup are native.
  datasource.caching = false;
  datasource.getReleases = (config) => datasource._getReleases(config);
  datasource.fetchCrateRecordsPayload = async (info, packageName) => {
    assert.equal(info.flavor, "crates.io");
    assert.equal(packageName, "cargo-llvm-cov");
    acquired.push(offered.map((row) => row.vers));
    return offered.map((row) => JSON.stringify(row)).join("\n");
  };
  datasource.getCrateMetadata = async () => null;
  const temporary = fs.mkdtempSync(os.tmpdir() + "/boxferry708-native-lookup-");
  GlobalConfig.set({ localDir: temporary, cachePrivatePackages: false });
  const report = { cases: [], scoped: true };
  try {
    for (const [name, versions, changed, expected] of [
      ["competing", ["0.9.0", "0.9.1", "0.9.1000000", "0.9.1-rc.1"], {}, "0.9.1"],
      ["outside-only", ["0.9.0", "0.9.1000000"], {}, null],
      ["build-normalized", ["0.9.0", "0.9.1+rebuilt.1"], {}, "0.9.1"],
      [
        "image-neighbor",
        ["0.9.0", "0.9.1", "0.9.1000000"],
        { packageFile: ".devcontainer/Dockerfile" },
        "0.9.1000000",
      ],
    ]) {
      offered = rows.filter((row) => versions.includes(row.vers));
      memory.reset();
      const effective = await applyPackageRules({ ...dependency, ...changed }, "test-lookup");
      const result = await (await lookupUpdates(effective)).unwrap();
      assert.equal(result.ok, true);
      assert.deepEqual(result.val.warnings, []);
      assert.equal(result.val.updates.length, expected ? 1 : 0, name);
      const record = { name, selected: result.val.updates[0]?.newValue ?? null };
      assert.equal(record.selected, expected, name);
      if (expected && !changed.packageFile) {
        const update = result.val.updates[0];
        const written = await doAutoReplace(
          { ...manager, ...effective, ...update, depIndex: 0 },
          original,
          false,
        );
        assert.equal(written, "0.9.1\n");
        const again = extractPackageFile(written, file, manager).deps;
        assert.equal(again.length, 1);
        assert.equal(again[0].currentValue, expected);
        assert.equal(fs.readFileSync(temporary + "/" + file, "utf8"), written);
        record.written = written;
      }
      report.cases.push(record);
    }
    assert.equal(acquired.length, 4, "each lookup acquired only its fixture rows");
    report.acquired = acquired;
    console.log(JSON.stringify(report));
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
