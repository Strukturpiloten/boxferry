// Read-only contract tests using the existing integrity-pinned native Renovate modules.
(async () => {
  const assert = require("node:assert/strict");
  const fs = require("node:fs");
  const [runtimeArg, rootArg, selectedArg] = process.argv.slice(2);
  const os = require("node:os");
  const path = require("node:path");
  const runtime = path.resolve(runtimeArg);
  const root = path.resolve(rootArg);
  const parent = fs.realpathSync(os.tmpdir());
  const scratch = fs.mkdtempSync(path.join(parent, "boxferry-native-policy-"));
  try {
    process.chdir(scratch);
    fs.mkdirSync(path.join(scratch, "tmp"));
    for (const key of Object.keys(process.env)) {
      if (key.startsWith("GIT_") || key === "LOG_FILE" || key === "RENOVATE_LOG_FILE")
        delete process.env[key];
    }
    Object.assign(process.env, {
      TMPDIR: path.join(scratch, "tmp"),
      XDG_CACHE_HOME: path.join(scratch, "cache"),
      RENOVATE_BASE_DIR: scratch,
      RENOVATE_CACHE_DIR: path.join(scratch, "cache"),
      GIT_CEILING_DIRECTORIES: scratch,
      GIT_TERMINAL_PROMPT: "0",
      GIT_CONFIG_NOSYSTEM: "1",
      GIT_CONFIG_GLOBAL: os.devNull,
      GIT_CONFIG_SYSTEM: os.devNull,
      GIT_CONFIG_COUNT: "0",
      GIT_OPTIONAL_LOCKS: "0",
      GIT_NO_REPLACE_OBJECTS: "1",
      GIT_NO_LAZY_FETCH: "1",
    });
    // Configure before logger/native initialization; direct entry is self-contained.
    const base = runtime + "/dist/";
    require(base + "config/global.js").GlobalConfig.set({
      localDir: scratch,
      baseDir: scratch,
      cacheDir: path.join(scratch, "cache"),
      presetCachePersistence: false,
    });
    const pin = JSON.parse(fs.readFileSync(root + "/.github/automation.json", "utf8")).revision;
    const selected = selectedArg
      ? path.resolve(selectedArg)
      : path.join(root, "target/boxferry-automation", pin, "renovate-presets");
    const refs = JSON.parse(fs.readFileSync(root + "/.github/renovate.json", "utf8")).extends;
    const cache = require(base + "util/cache/memory/index.js");
    cache.init();
    for (const name of ["common", "rust"]) {
      const ref = `github>Strukturpiloten/boxferry//renovate-presets/${name}#${pin}`;
      assert.ok(refs.includes(ref), "exact selected preset ref");
      cache.set(
        "preset:" + ref,
        JSON.parse(fs.readFileSync(path.join(selected, name + ".json"), "utf8")),
      );
    }
    await require(base + "logger/index.js").init();
    const { resolveConfigPresets } = require(base + "config/presets/index.js");
    const { applyPackageRules } = require(base + "util/package-rules/index.js");
    const { getConfig } = require(base + "config/defaults.js");
    const { mergeChildConfig } = require(base + "config/utils.js");
    const { massageConfig } = require(base + "config/massage.js");
    const { flattenUpdates } = require(base + "workers/repository/updates/flatten.js");
    // Native preset loading canonicalizes zero durations to null, not a duration string.
    const noReleaseAge = massageConfig({ minimumReleaseAge: "0 days" }).minimumReleaseAge;
    assert.equal(noReleaseAge, null, "native zero-age canonical representation");
    const local = JSON.parse(fs.readFileSync(root + "/.github/renovate.json", "utf8"));
    const admissionDescription =
      "Require explicit security review for the DockerLens application admission pin";
    const admissionPaths = [
      ".github/workflows/reviewed-application-validation.yml",
      ".github/workflows/reviewed-docker-consumer-validation.yml",
    ];
    const ordinary = [
      [
        "App-token Action",
        "github-actions",
        "github-tags",
        "actions/create-github-app-token",
        ".github/actions/reviewed-docker-admission/action.yml",
        "digest",
      ],
      ["cargo major", "cargo", "crate", "serde", "Cargo.toml", "major"],
      ["cargo minor", "cargo", "crate", "serde", "Cargo.toml", "minor"],
      ["compiler", "rust-toolchain", "rust-version", "rust", "rust-toolchain.toml", "major"],
      ["runner", "regex", "github-runners", "ubuntu", ".github/workflows/ci.yml", "major"],
      [
        "Cargo tool",
        "custom.regex",
        "crate",
        "cargo-llvm-cov",
        ".github/workflows/shared-rust-ci.yml",
        "major",
      ],
      ["Python tool", "pep621", "pypi", "ruff", "automation/pyproject.toml", "major"],
      [
        "standalone Node",
        "custom.regex",
        "node-version",
        "node",
        ".github/actions/shared-node/action.yml",
        "major",
      ],
      [
        "downloaded tool",
        "regex",
        "github-release-attachments",
        "rhysd/actionlint",
        "automation/tool-artifacts.json",
        "major",
      ],
      [
        "release emulator tag",
        "custom.regex",
        "docker",
        "docker.io/tonistiigi/binfmt",
        "scripts/lib/release-emulator.sh",
        "major",
      ],
      [
        "release emulator digest",
        "custom.regex",
        "docker",
        "docker.io/tonistiigi/binfmt",
        "scripts/lib/release-emulator.sh",
        "digest",
      ],
    ];
    const dependency = (manager, datasource, packageName, packageFile, updateType) => ({
      manager,
      datasource,
      packageName,
      depName: packageName,
      packageFile,
      updateType,
    });
    async function checkActualPolicy(candidate) {
      // Native preset resolution consumes the actual local configuration and its exact refs.
      // It does not grant authority from a hand-built projection or description metadata.
      const { config } = await resolveConfigPresets(candidate);
      for (const [label, ...fields] of ordinary) {
        const actual = await applyPackageRules({ ...config, ...dependency(...fields) });
        assert.equal(actual.automerge, true, label + " automerge");
        assert.equal(actual.dependencyDashboardApproval, false, label + " dashboard approval");
        assert.equal(actual.automergeType, "pr", label + " PR merge semantics");
        assert.equal(actual.platformAutomerge, false, label + " protected queue semantics");
        assert.notEqual(actual.enabled, false, label + " enabled");
        assert.equal(actual.minimumReleaseAge, "3 days", label + " ordinary age");
        if (label === "App-token Action") {
          assert.equal(actual.pinDigests, true, label + " digest pinning");
          assert.equal(actual.groupName, "GitHub Actions", label + " native group");
        }
      }
      for (const [datasource, packageName] of [
        ["docker", "docker.io/library/node"],
        ["docker", "mcr.microsoft.com/devcontainers/rust"],
        ["docker", "ghcr.io/astral-sh/uv"],
        ["crate", "cargo-deny"],
        ["crate", "cargo-llvm-cov"],
        ["crate", "cargo-semver-checks"],
        ["crate", "zizmor"],
      ]) {
        const actual = await applyPackageRules({
          ...config,
          ...dependency(
            "custom.regex",
            datasource,
            packageName,
            ".devcontainer/Dockerfile",
            "major",
          ),
        });
        assert.equal(actual.enabled, false, packageName + " deferred image definition");
        assert.equal(actual.dependencyDashboardApproval, false, "no manual image approval route");
        assert.equal(actual.minimumReleaseAge, "3 days", "image age policy remains ordinary");
      }
      // Use Renovate's actual versionless package-file maintenance proposal. It has no
      // serde/crate publication identity; introduced graph versions are checked by Required CI.
      // Commit-message discovery is irrelevant to this policy fixture and requires an actual
      // initialized repository; select its explicit native disabled mode instead of Git I/O.
      const maintenance = await flattenUpdates(
        mergeChildConfig(getConfig(), { ...config, semanticCommits: "disabled" }),
        {
          cargo: [{ packageFile: "Cargo.toml", deps: [], lockFiles: ["Cargo.lock"] }],
        },
      );
      assert.equal(maintenance.length, 1, "one native maintenance proposal");
      const [lock] = maintenance;
      assert.equal(lock.updateType, "lockFileMaintenance");
      assert.equal(lock.isLockFileMaintenance, true);
      for (const field of ["depName", "packageName", "datasource", "currentVersion", "newVersion"])
        assert.equal(lock[field], undefined, "versionless maintenance " + field);
      assert.equal(lock.automerge, true, "lock maintenance automerge");
      assert.equal(lock.dependencyDashboardApproval, false, "lock maintenance approval");
      assert.equal(lock.automergeType, "pr", "lock maintenance PR merge semantics");
      assert.equal(lock.platformAutomerge, false, "lock maintenance protected queue semantics");
      assert.equal(lock.minimumReleaseAge, noReleaseAge, "versionless lock maintenance age");
      for (const [packageName, age] of [
        ["compose-lens", noReleaseAge],
        ["compose-lens-lookalike", "3 days"],
        ["serde", "3 days"],
      ]) {
        const actual = await applyPackageRules({
          ...config,
          ...dependency("cargo", "crate", packageName, "Cargo.toml", "minor"),
        });
        assert.equal(actual.automerge, true, packageName + " Cargo automerge");
        assert.equal(actual.dependencyDashboardApproval, false, packageName + " Cargo approval");
        assert.equal(actual.minimumReleaseAge, age, packageName + " exact first-party age");
      }
      for (const packageName of ["Strukturpiloten/compose-lens", "Strukturpiloten/quadlet-lens"]) {
        const actual = await applyPackageRules({
          ...config,
          ...dependency(
            "custom.regex",
            "github-tags",
            packageName,
            "fixtures/conformance/migration-readiness/tiers.toml",
            "digest",
          ),
        });
        assert.equal(actual.automerge, true, packageName + " native conformance automerge");
        assert.equal(
          actual.dependencyDashboardApproval,
          false,
          packageName + " native conformance approval",
        );
        assert.equal(actual.groupName, "Native Lens conformance", packageName + " native group");
      }
      for (const packageName of [
        "Strukturpiloten/boxferry",
        "Strukturpiloten/boxferry-lookalike",
      ]) {
        const actual = await applyPackageRules({
          ...config,
          ...dependency("regex", "github-digest", packageName, ".github/automation.json", "digest"),
        });
        assert.equal(
          actual.minimumReleaseAge,
          packageName === "Strukturpiloten/boxferry" ? noReleaseAge : "3 days",
          "exact first-party source age",
        );
      }
      for (const fields of [
        [
          "regex",
          "github-release-attachments",
          "docker/compose",
          "scripts/lib/compose-provider.sh",
        ],
        ["regex", "docker", "quay.io/libpod/alpine", "fixtures/conformance/podman-live/matrix.tsv"],
        ["regex", "docker", "example/image", "fixtures/conformance/forgejo-application/images.tsv"],
      ]) {
        const actual = await applyPackageRules({ ...config, ...dependency(...fields, "major") });
        assert.equal(actual.enabled, false, "fixed target remains disabled");
        assert.equal(actual.dependencyDashboardApproval, false, "fixed target has no manual route");
      }
      const admissionRules = candidate.packageRules.filter(
        (rule) => rule.description === admissionDescription,
      );
      assert.equal(admissionRules.length, 1, "one privileged admission rule");
      assert.deepEqual(
        admissionRules[0].matchFileNames,
        admissionPaths,
        "both admission consumers",
      );
      for (const path of admissionPaths) {
        const actual = await applyPackageRules({
          ...config,
          ...dependency("regex", "github-digest", "Strukturpiloten/docker-lens", path, "digest"),
        });
        assert.equal(actual.automerge, false, "privileged admission automerge");
        assert.equal(actual.dependencyDashboardApproval, true, "privileged admission approval");
      }
    }
    await checkActualPolicy(local);
    for (const override of [{ pinDigests: false }, { groupName: "unreviewed group" }]) {
      const changed = structuredClone(local);
      changed.packageRules.push({
        matchManagers: ["github-actions"],
        matchPackageNames: ["actions/create-github-app-token"],
        matchUpdateTypes: ["digest"],
        ...override,
      });
      await assert.rejects(() => checkActualPolicy(changed), /App-token Action/);
    }
    for (const override of [
      { automerge: false },
      { dependencyDashboardApproval: true },
      { automergeType: "branch" },
      { platformAutomerge: true },
    ]) {
      const changed = structuredClone(local);
      changed.packageRules.push({
        matchManagers: ["cargo"],
        matchDatasources: ["crate"],
        matchPackageNames: ["serde"],
        matchUpdateTypes: ["major"],
        ...override,
      });
      await assert.rejects(() => checkActualPolicy(changed), /cargo major/);
    }
    for (const override of [{ automergeType: "branch" }, { platformAutomerge: true }]) {
      const late = structuredClone(local);
      late.packageRules.push({
        matchManagers: ["cargo"],
        matchUpdateTypes: ["lockFileMaintenance"],
        ...override,
      });
      await assert.rejects(() => checkActualPolicy(late), /lock maintenance/);
      const nested = structuredClone(local);
      nested.lockFileMaintenance = { ...nested.lockFileMaintenance, ...override };
      await assert.rejects(() => checkActualPolicy(nested), /lock maintenance/);
    }
    for (const mutation of ["late admission override", "missing", "duplicate", "consumer scope"]) {
      const changed = structuredClone(local);
      const index = changed.packageRules.findIndex(
        (rule) => rule.description === admissionDescription,
      );
      if (mutation === "late admission override") {
        changed.packageRules.push({
          matchManagers: ["custom.regex"],
          matchDatasources: ["github-digest"],
          matchPackageNames: ["Strukturpiloten/docker-lens"],
          automerge: true,
        });
      } else if (mutation === "missing") changed.packageRules.splice(index, 1);
      else if (mutation === "duplicate")
        changed.packageRules.push(structuredClone(changed.packageRules[index]));
      else changed.packageRules[index].matchFileNames = [admissionPaths[0]];
      await assert.rejects(() => checkActualPolicy(changed), /admission/);
    }
    console.log(
      "Actual native local policy and fourteen late-override/lock/admission counterfactuals passed.",
    );
  } finally {
    process.chdir(parent);
    fs.rmSync(scratch, { recursive: true, force: true });
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
