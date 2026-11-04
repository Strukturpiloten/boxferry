// Run with the pinned native Renovate package: node script <renovate-root> <repository-root>.
(async () => {
  const assert = require("node:assert/strict");
  const crypto = require("node:crypto");
  const fs = require("node:fs");
  const os = require("node:os");
  const path = require("node:path");
  const { Readable } = require("node:stream");
  const [runtime, root] = process.argv.slice(2);
  const base = runtime + "/dist/";
  await require(base + "logger/index.js").init();
  assert.equal(require(base + "util/regex.js").regexEngineStatus.type, "available");
  const { GlobalConfig } = require(base + "config/global.js");
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), "provider-renovate-"));
  GlobalConfig.set({ localDir: temp });
  try {
    const { extractPackageFile } = require(base + "modules/manager/custom/regex/index.js");
    const { doAutoReplace } = require(base + "workers/repository/update/branch/auto-replace.js");
    const { applyPackageRules } = require(base + "util/package-rules/index.js");
    const { GithubReleaseAttachmentsDatasource } = require(
      base + "modules/datasource/github-release-attachments/index.js",
    );
    const config = JSON.parse(fs.readFileSync(root + "/.github/renovate.json"));
    const file = "scripts/lib/compose-provider.sh";
    const production = fs.readFileSync(root + "/" + file, "utf8");
    function owners(contents, filename) {
      return config.customManagers
        .filter((m) => m.managerFilePatterns.some((p) => new RegExp(p.slice(1, -1)).test(filename)))
        .flatMap((m) => {
          const extracted = extractPackageFile(contents, filename, m);
          return extracted ? extracted.deps.map((dep) => ({ manager: m, dep })) : [];
        });
    }
    const productionOwners = owners(production, file);
    assert.equal(productionOwners.length, 1);
    const { manager, dep: current } = productionOwners[0];
    assert.equal(current.datasource, "github-release-attachments");
    assert.equal(current.depName, "docker/compose");
    assert.match(current.currentValue, /^v\d+\.\d+\.\d+$/);
    assert.match(current.currentDigest, /^[a-f0-9]{64}$/);

    // Authored source values keep normal automated pin updates from rewriting tests.
    const oldVersion = "v2.3.4";
    const nextVersion = "v3.0.0";
    const binary = "docker-compose-linux-x86_64";
    const oldBytes = Buffer.from("authored previous x86_64 Compose asset");
    const newBytes = Buffer.from("authored next x86_64 Compose asset");
    const hash = (bytes) => crypto.createHash("sha256").update(bytes).digest("hex");
    const oldDigest = hash(oldBytes);
    const nextDigest = hash(newBytes);
    const contents = production
      .replace(current.currentValue, oldVersion)
      .replace(current.currentDigest, oldDigest);
    const [{ dep }] = owners(contents, file);
    assert.equal(dep.currentValue, oldVersion);
    assert.equal(dep.currentDigest, oldDigest);
    assert.equal(extractPackageFile(contents.replace(oldDigest, "invalid"), file, manager), null);

    function fixtureDatasource(
      scenario,
      { currentManifest = true, nextManifest = true, missingBinary = false } = {},
    ) {
      const datasource = new GithubReleaseAttachmentsDatasource();
      const streams = [];
      const prefix = `https://fixtures.invalid/${scenario}`;
      const release = (tag) => {
        const prefixTag = `${prefix}/${tag}`;
        const assets = [
          {
            name: "docker-compose-linux-aarch64",
            size: 70000000,
            browser_download_url: `${prefixTag}/arm64`,
          },
          ...(missingBinary && tag === nextVersion
            ? []
            : [{ name: binary, size: 80000000, browser_download_url: `${prefixTag}/x86_64` }]),
        ];
        if (tag === oldVersion ? currentManifest : nextManifest) {
          assets.push({
            name: binary + ".sha256",
            size: 97,
            browser_download_url: `${prefixTag}/checksums`,
          });
        }
        return { tag_name: tag, html_url: prefixTag, assets };
      };
      datasource.http.getJsonUnchecked = async (url) => ({ body: release(url.split("/").at(-1)) });
      datasource.http.getText = async (url) => ({
        body: `${"c".repeat(64)}  docker-compose-linux-aarch64\n${url.includes(oldVersion) ? oldDigest : nextDigest}  ${binary}\n`,
      });
      // Exercise Renovate's real stream hashing fallback, without external network or binaries.
      datasource.http.stream = (url) => {
        streams.push(url);
        return Readable.from([
          url.endsWith("/arm64")
            ? Buffer.from("unrelated architecture")
            : url.includes(oldVersion)
              ? oldBytes
              : newBytes,
        ]);
      };
      return { datasource, streams };
    }
    const options = {
      packageName: dep.depName,
      currentValue: oldVersion,
      currentDigest: oldDigest,
      registryUrl: "https://github.com",
    };
    const mapped = fixtureDatasource("manifest-map");
    assert.equal(await mapped.datasource.getDigest(options, nextVersion), nextDigest);
    assert.deepEqual(mapped.streams, []);
    const fallback = fixtureDatasource("all-binary-fallback", {
      currentManifest: false,
      nextManifest: false,
    });
    assert.equal(await fallback.datasource.getDigest(options, nextVersion), nextDigest);
    assert.equal(fallback.streams.length, 3); // Other architecture, matching current asset, matching next asset.
    const nextFallback = fixtureDatasource("next-binary-fallback", { nextManifest: false });
    assert.equal(await nextFallback.datasource.getDigest(options, nextVersion), nextDigest);
    assert.equal(nextFallback.streams.length, 1);
    const missing = fixtureDatasource("missing-next", { nextManifest: false, missingBinary: true });
    assert.equal(await missing.datasource.getDigest(options, nextVersion), null);
    assert.equal(
      await missing.datasource.getDigest({ ...options, currentDigest: "" }, nextVersion),
      null,
    );
    // The native datasource retains an unidentifiable old digest. Mandatory artifact verification
    // independently rejects this otherwise consistent version/URL/stale-digest candidate.
    const unidentified = fixtureDatasource("stale-unidentified", {
      currentManifest: false,
      nextManifest: false,
    });
    assert.equal(
      await unidentified.datasource.getDigest(
        { ...options, currentDigest: "a".repeat(64) },
        nextVersion,
      ),
      "a".repeat(64),
    );

    const updated = await doAutoReplace(
      {
        ...manager,
        ...dep,
        manager: "regex",
        packageFile: file,
        depIndex: 0,
        newValue: nextVersion,
        newDigest: nextDigest,
      },
      contents,
      false,
    );
    assert.equal(updated, contents.replace(oldVersion, nextVersion).replace(oldDigest, nextDigest));
    for (const updateType of ["patch", "minor", "major", "digest", "pinDigest"]) {
      const policy = await applyPackageRules({
        ...config,
        ...dep,
        manager: "regex",
        packageFile: file,
        packageName: dep.depName,
        updateType,
      });
      assert.equal(policy.enabled, false, updateType);
      assert.equal(policy.automerge, true, updateType);
      assert.equal(policy.dependencyDashboardApproval, false, updateType);
      assert.equal(policy.minimumReleaseAge, "3 days");
      assert.equal(policy.pinDigests, true);
      assert.equal(policy.groupName, "Docker Compose provider");
    }
    for (const app of [
      "forgejo",
      "immich",
      "nextcloud",
      "observability",
      "paperless-ngx",
      "supabase",
    ]) {
      const catalogueFile = `fixtures/conformance/${app}-application/providers.tsv`;
      const original = fs.readFileSync(root + "/" + catalogueFile, "utf8");
      const matches = owners(original, catalogueFile);
      assert.equal(matches.length, 1, catalogueFile);
      const row = matches[0];
      assert.equal(row.dep.currentValue, current.currentValue);
      assert.equal(row.dep.currentDigest, current.currentDigest);
      const replacement = await doAutoReplace(
        {
          ...row.manager,
          ...row.dep,
          manager: "regex",
          packageFile: catalogueFile,
          depIndex: 0,
          newValue: nextVersion,
          newDigest: nextDigest,
        },
        original,
        false,
      );
      assert.equal(
        replacement,
        original
          .replaceAll(current.currentValue.slice(1), nextVersion.slice(1))
          .replace(current.currentDigest, nextDigest),
      );
      for (const updateType of ["patch", "minor", "major", "digest", "pinDigest"]) {
        const rowPolicy = await applyPackageRules({
          ...config,
          ...row.dep,
          manager: "regex",
          packageFile: catalogueFile,
          packageName: "docker/compose",
          updateType,
        });
        assert.equal(rowPolicy.enabled, false, `${app}: ${updateType}`);
        assert.equal(rowPolicy.groupName, "Docker Compose provider");
        assert.equal(rowPolicy.automerge, true);
        assert.equal(rowPolicy.dependencyDashboardApproval, false);
      }
    }
    console.log(
      "Provider native Renovate extraction, asset mapping, stream fallback, replacement and provisional major/non-major hold passed.",
    );
  } finally {
    fs.rmSync(temp, { recursive: true, force: true });
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
