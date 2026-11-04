// Verify custom-manager extraction and native replacement with Renovate 44.139.0.
// Oracle: pinned local package from .ci-tools/renovate in KubernetesLens #75.
// Command: node scripts/test-kubernetes-tools-renovate.cjs <renovate-dist-root> <repo-root>.
// Source: github.com/renovatebot/renovate; MIT; no oracle source redistributed.
(async () => {
  const assert = require("node:assert/strict");
  const fs = require("node:fs");
  const os = require("node:os");
  const path = require("node:path");
  const [runtime, root] = process.argv.slice(2);
  const base = runtime + "/";
  await require(base + "logger/index.js").init();
  const { GlobalConfig } = require(base + "config/global.js");
  const { extractPackageFile } = require(base + "modules/manager/custom/regex/index.js");
  const { doAutoReplace } = require(base + "workers/repository/update/branch/auto-replace.js");
  const { applyPackageRules } = require(base + "util/package-rules/index.js");
  const { GithubReleaseAttachmentsDatasource } = require(
    base + "modules/datasource/github-release-attachments/index.js",
  );
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "kubernetes-renovate-"));
  GlobalConfig.set({ localDir: temporary });
  try {
    const configuration = JSON.parse(fs.readFileSync(root + "/.github/renovate.json", "utf8"));
    const file = "scripts/install-kubernetes-tools.sh";
    const contents = fs.readFileSync(root + "/" + file, "utf8");
    const managers = configuration.customManagers.filter((manager) =>
      manager.managerFilePatterns.some((pattern) => new RegExp(pattern.slice(1, -1)).test(file)),
    );
    const extracted = managers.flatMap((manager) => {
      const result = extractPackageFile(contents, file, manager);
      return result ? result.deps.map((dep) => ({ manager, dep })) : [];
    });
    const kind = extracted.filter((item) => item.dep.depName === "kubernetes-sigs/kind");
    assert.equal(kind.length, 2);
    assert.deepEqual(
      new Set(kind.map((item) => item.dep.depType)),
      new Set(["kind-linux-amd64", "kind-linux-arm64"]),
    );
    const currentDigestByArch = {
      amd64: kind.find((item) => item.dep.depType === "kind-linux-amd64").dep.currentDigest,
      arm64: kind.find((item) => item.dep.depType === "kind-linux-arm64").dep.currentDigest,
    };
    const nextDigestByArch = { amd64: "c".repeat(64), arm64: "d".repeat(64) };
    const asset = (tag, name, size) => ({
      name,
      size,
      browser_download_url: `https://fixtures/${tag}/${name}`,
    });
    const currentRelease = {
      tag_name: "v0.33.0",
      html_url: "https://github.com/kubernetes-sigs/kind/releases/tag/v0.33.0",
      assets: [
        asset("current", "SHA256SUMS", 128),
        asset("current", "kind-linux-amd64", 10000),
        asset("current", "kind-linux-arm64", 10000),
      ],
    };
    const nextRelease = {
      tag_name: "v0.34.0",
      html_url: "https://github.com/kubernetes-sigs/kind/releases/tag/v0.34.0",
      assets: [
        asset("next", "SHA256SUMS", 128),
        asset("next", "kind-linux-amd64", 10000),
        asset("next", "kind-linux-arm64", 10000),
      ],
    };
    const datasource = new GithubReleaseAttachmentsDatasource();
    datasource.http.getJsonUnchecked = async (url) => ({
      body: url.endsWith("/v0.33.0") ? currentRelease : nextRelease,
    });
    let includeArm64 = true;
    datasource.http.getText = async (url) => {
      const checksums = url.includes("/current/")
        ? currentDigestByArch
        : includeArm64
          ? nextDigestByArch
          : { amd64: nextDigestByArch.amd64 };
      const lines = [
        `${checksums.amd64}  kind-linux-amd64`,
        ...(checksums.arm64 ? [`${checksums.arm64}  kind-linux-arm64`] : []),
      ];
      return { body: `${lines.join("\n")}\n` };
    };
    const digestByArch = {};
    for (const arch of ["amd64", "arm64"]) {
      digestByArch[arch] = await datasource.getDigest(
        {
          packageName: "kubernetes-sigs/kind",
          currentValue: "v0.33.0",
          currentDigest: currentDigestByArch[arch],
          registryUrl: "https://github.com",
        },
        "v0.34.0",
      );
      assert.equal(digestByArch[arch], nextDigestByArch[arch]);
    }
    includeArm64 = false;
    assert.equal(
      await datasource.getDigest(
        {
          packageName: "kubernetes-sigs/kind",
          currentValue: "v0.33.0",
          currentDigest: currentDigestByArch.arm64,
          registryUrl: "https://github.com",
        },
        "v0.34.0",
      ),
      null,
    );
    for (const { manager, dep } of kind) {
      assert.equal(dep.datasource, "github-release-attachments");
      assert.match(dep.currentValue, /^v\d+\.\d+\.\d+$/);
      assert.match(dep.currentDigest, /^[a-f0-9]{64}$/);
      const replacement = await doAutoReplace(
        {
          ...manager,
          ...dep,
          manager: "regex",
          packageFile: file,
          depIndex: dep.depType.endsWith("arm64") ? 1 : 0,
          newValue: "v9.8.7",
          newDigest: digestByArch[dep.depType.endsWith("arm64") ? "arm64" : "amd64"],
        },
        contents,
        false,
      );
      const arch = dep.depType.endsWith("arm64") ? "arm64" : "amd64";
      const record = new RegExp(
        `^readonly kind_${arch}_release="[^"]+" kind_${arch}_checksum="[^"]+"$`,
        "m",
      );
      assert.match(replacement, record);
      assert.match(replacement, new RegExp(`readonly kind_${arch}_release="v9\\.8\\.7"`));
      assert.match(replacement, new RegExp(`kind_${arch}_checksum="${digestByArch[arch]}"`));
    }
    const rules = configuration.packageRules.find(
      (rule) =>
        rule.description === "Group Kubernetes development tool versions and integrity refreshes",
    );
    const policy = await applyPackageRules({
      ...configuration,
      ...kind[0].dep,
      manager: "regex",
      packageFile: file,
      packageName: kind[0].dep.depName,
      updateType: "patch",
    });
    assert.equal(policy.automerge, true);
    assert.notEqual(policy.dependencyDashboardApproval, true);
    assert.equal(policy.groupName, rules.groupName);
    assert.deepEqual(
      new Set(rules.matchPackageNames),
      new Set([
        "kubernetes-sigs/kind",
        "kubernetes/kubernetes",
        "helm/helm",
        "kubernetes-sigs/kustomize",
      ]),
    );
    console.log(
      "Native Renovate kind extraction, tag/digest replacement and scoped policy passed.",
    );
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
