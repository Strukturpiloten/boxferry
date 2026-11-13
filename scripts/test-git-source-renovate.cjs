// Native Renovate tests of the operational source owner in an isolated fixture.
(async () => {
  const assert = require("node:assert/strict");
  const fs = require("node:fs");
  const path = require("node:path");
  const [runtime, root] = process.argv.slice(2);
  const base = runtime + "/dist/";
  await require(base + "logger/index.js").init();
  const memory = require(base + "util/cache/memory/index.js");
  memory.init();
  const local = JSON.parse(fs.readFileSync(root + "/.github/renovate.json"));
  for (const name of ["common", "rust"]) {
    const reference = local.extends.find((ref) => ref.includes("/" + name + "#"));
    assert.ok(reference);
    memory.set(
      "preset:" + reference,
      JSON.parse(fs.readFileSync(root + "/.native-presets/" + name + ".json")),
    );
  }
  const defaults = require(base + "config/defaults.js").getConfig();
  const { mergeChildConfig } = require(base + "config/utils.js");
  const { config: resolved } = await require(base + "config/presets/index.js").resolveConfigPresets(
    local,
  );
  const config = mergeChildConfig(defaults, resolved);
  const { extractPackageFile } = require(base + "modules/manager/custom/regex/index.js");
  const { applyPackageRules } = require(base + "util/package-rules/index.js");
  const { doAutoReplace } = require(base + "workers/repository/update/branch/auto-replace.js");
  const { CustomDatasource } = require(base + "modules/datasource/custom/index.js");
  const packageFile = "automation/tool-artifacts.json";
  const content = fs.readFileSync(root + "/" + packageFile, "utf8");
  const original = JSON.parse(content);
  const declared = original.sources[0];
  const owners = [];
  for (const manager of config.customManagers) {
    if (
      !require(base + "util/string-match.js").matchRegexOrGlobList(
        packageFile,
        manager.managerFilePatterns || [],
      )
    )
      continue;
    const deps = extractPackageFile(content, packageFile, manager)?.deps || [];
    for (const dep of deps.filter((value) => value.depName === "git"))
      owners.push({ manager, dep });
  }
  assert.equal(owners.length, 1, "exactly one Renovate owner for the operational Git declaration");
  const { manager, dep } = owners[0];
  assert.equal(dep.currentValue, declared.currentVersion);
  assert.equal(dep.currentDigest, declared.sha256);
  assert.equal(dep.datasource, "custom.kernel-git");
  const effective = await applyPackageRules({
    ...config,
    ...dep,
    manager: "custom.regex",
    packageName: "git",
    packageFile,
    updateType: "patch",
  });
  for (const updateType of ["patch", "minor", "major", "digest", "pinDigest"]) {
    const policy = await applyPackageRules({
      ...config,
      ...dep,
      manager: "custom.regex",
      packageName: "git",
      packageFile,
      updateType,
    });
    for (const [key, expected] of Object.entries({
      groupName: "Canonical downloaded tools",
      minimumReleaseAge: "3 days",
      minimumReleaseAgeBehaviour: "timestamp-optional",
      pinDigests: true,
      automerge: true,
      dependencyDashboardApproval: false,
      platformAutomerge: false,
    }))
      assert.equal(policy[key], expected, `${updateType}: ${key}`);
  }
  // Synthetic versions/digests are confined to native test metadata and a copied manifest.
  const target = "99.0.1",
    digest = "a".repeat(64);
  const datasource = new CustomDatasource();
  datasource.http.getPlain = async (url) => {
    assert.equal(url, "https://www.kernel.org/pub/software/scm/git/sha256sums.asc");
    return {
      body: `${digest}  git-${target}.tar.xz\n${"b".repeat(64)}  git-${target}.tar.gz\n${"c".repeat(64)}  git-99.0.2-rc0.tar.xz\ninvalid\n`,
    };
  };
  const releases = await datasource.getReleases({
    ...effective,
    customDatasources: config.customDatasources,
  });
  assert.deepEqual(
    releases.releases.map((r) => [r.version, r.newDigest]),
    [[target, digest]],
  );
  assert.equal(releases.releases[0].releaseTimestamp, null, "no fabricated publication date");
  require(base + "modules/datasource/index.js")
    .getDatasources()
    .get("custom").http.getPlain = datasource.http.getPlain;
  const lookup = (
    await require(base + "workers/repository/process/lookup/index.js").lookupUpdates({
      ...effective,
      customDatasources: config.customDatasources,
      versioning: "semver",
      rangeStrategy: "replace",
      updatePinnedDependencies: true,
    })
  ).unwrapOrThrow();
  const update = lookup.updates.find((value) => value.newValue === target);
  assert.ok(update, "actual native lookup produces an update");
  assert.equal(update.newDigest, digest, "actual lookup retains the matching official digest");
  fs.mkdirSync(path.join(root, "automation"), { recursive: true });
  const replaced = await doAutoReplace(
    { ...manager, ...dep, ...update, manager: "regex", packageFile, depIndex: 0 },
    content,
    false,
  );
  assert.ok(replaced && replaced !== content, "actual replacement changes the isolated fixture");
  const after = JSON.parse(replaced);
  assert.deepEqual(after.sources, [
    {
      ...declared,
      currentVersion: target,
      filename: `git-${target}.tar.xz`,
      url: `https://www.kernel.org/pub/software/scm/git/git-${target}.tar.xz`,
      sha256: digest,
    },
  ]);
  assert.deepEqual(
    after.assets,
    original.assets,
    "all six binary profiles and their integrity remain unchanged",
  );
  assert.deepEqual(after.provenance, original.provenance);
  const wrongPath = await applyPackageRules({
    ...config,
    ...dep,
    manager: "custom.regex",
    packageName: "git",
    packageFile: "other.json",
  });
  assert.equal(
    wrongPath.minimumReleaseAgeBehaviour,
    "timestamp-required",
    "timestamp exception stays within the canonical owner",
  );
  console.log(
    "Git source native ownership, official digest pairing, lookup, atomic replacement and effective policy passed",
  );
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
