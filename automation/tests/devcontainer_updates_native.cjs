/* Authored offline regression for the unmodified, integrity-pinned Renovate SDK.
 * Oracle: automation/renovate-runtime.json; AGPL-3.0-only, verification only.
 * No upstream implementation is copied. Native modules are loaded from the SDK.
 * HTTPS is intercepted before import; every transport goes to this local fixture.
 */
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const http = require("node:http");
const https = require("node:https");
const vm = require("node:vm");
const { createRequire, syncBuiltinESMExports } = require("node:module");
const input = JSON.parse(fs.readFileSync(0, "utf8"));
const sdk = process.argv[2];
const base = path.join(sdk, "dist");
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "boxferry-updater-native-"));
for (const key of Object.keys(process.env)) {
  if (!["PATH", "LANG", "LC_ALL", "TMPDIR"].includes(key)) delete process.env[key];
}
process.env.HOME = scratch;
process.env.NODE_ENV = "test";
const app = "x-access-token:ghs_fixture_app";
const read = "ghs_fixture_reader";
const repo = "Strukturpiloten/boxferry";
const prefix = `https://api.github.com/repos/${repo}/commits/`;
function canonicalVersion(pin) {
  assert.equal(typeof pin, "string", "canonical runtime pin must be a string");
  const match =
    /^ghcr\.io\/renovatebot\/renovate:([0-9]+\.[0-9]+\.[0-9]+)@sha256:[a-f0-9]{64}$/.exec(pin);
  assert.ok(match, "canonical version and OCI digest required");
  return match[1];
}
function verifiedVersion(pin, packageVersion) {
  const expected = canonicalVersion(pin);
  assert.equal(packageVersion, expected, "runtime package must match canonical pin");
  return expected;
}
const requests = [];
let port;
let mode = "green";
const head = "a".repeat(40);
const label = (value) => {
  if (!value) return "none";
  if (value.endsWith(read)) return "reader";
  if (value.endsWith("ghs_fixture_app")) return "app";
  return "unexpected";
};
const server = http.createServer(async (request, response) => {
  const url = new URL(request.headers["x-fixture-url"] || `http://127.0.0.1:${port}${request.url}`);
  let body = "";
  for await (const chunk of request) body += chunk;
  requests.push({
    url: url.href,
    method: request.method,
    credential: label(request.headers.authorization),
    body,
  });
  response.setHeader("content-type", "application/json");
  response.setHeader("x-ratelimit-remaining", "4999");
  if (url.pathname.endsWith("/redirect")) {
    response.statusCode = 302;
    response.setHeader(
      "location",
      "https://api.github.com/repos/Other/private/commits/other/status",
    );
    return response.end("{}");
  }
  if (url.pathname.endsWith("/return-chain")) {
    response.statusCode = 302;
    response.setHeader("location", "https://unrelated.invalid/return-hop");
    return response.end("{}");
  }
  if (url.pathname === "/return-hop") {
    response.statusCode = 302;
    response.setHeader("location", `${prefix}${head}/returned`);
    return response.end("{}");
  }
  if (url.pathname.endsWith("/chain")) {
    response.statusCode = 302;
    response.setHeader("location", "https://api.github.com/same-origin-hop");
    return response.end("{}");
  }
  if (url.pathname === "/same-origin-hop") {
    response.statusCode = 302;
    response.setHeader("location", "https://unrelated.invalid/final");
    return response.end("{}");
  }
  if (url.pathname.endsWith("/downgrade")) {
    response.statusCode = 302;
    response.setHeader("location", "http://api.github.com/downgraded");
    return response.end("{}");
  }
  if (url.pathname.endsWith("/port-change")) {
    response.statusCode = 302;
    response.setHeader("location", "https://api.github.com:444/final");
    return response.end("{}");
  }
  if (
    (mode === "status403" && url.pathname.endsWith("/status")) ||
    (mode === "checks403" && url.pathname.endsWith("/check-runs"))
  ) {
    response.statusCode = 403;
    return response.end(JSON.stringify({ message: "Resource not accessible by integration" }));
  }
  if (mode === "invalid-json" && url.pathname.endsWith("/status")) return response.end("not-json");
  if (url.pathname === "/graphql") {
    return response.end(
      JSON.stringify({
        data: {
          repository: {
            id: "REPO",
            nameWithOwner: repo,
            defaultBranchRef: { name: "main", target: { oid: head } },
            squashMergeAllowed: true,
            autoMergeAllowed: true,
            mergeQueue: { id: "QUEUE" },
            issues: { nodes: [] },
          },
          viewer: { login: "strukturpiloten-release-plz[bot]", databaseId: 317908760 },
        },
      }),
    );
  }
  if (mode === "malformed-body" && url.pathname.endsWith("/check-runs"))
    return response.end(JSON.stringify({ check_runs: "not-an-array" }));
  if (url.pathname.endsWith("/check-runs")) {
    if (mode === "pages" && !url.searchParams.has("page"))
      response.setHeader("link", `<${url.href}&page=2>; rel="next"`);
    return response.end(
      JSON.stringify({
        total_count: 1,
        check_runs: [
          {
            name: "Required CI",
            status: "completed",
            conclusion: "success",
            head_sha: "f".repeat(40),
            url: "https://api.github.com/repos/Other/private/check-runs/1",
          },
        ],
      }),
    );
  }
  if (url.pathname.endsWith("/statuses")) return response.end("[]");
  return response.end(
    JSON.stringify({ state: "success", statuses: [], sha: "f".repeat(40), ok: true }),
  );
});
const realRequest = http.request;
http.request = function (urlOrOptions, options, callback) {
  const url =
    typeof urlOrOptions === "string" || urlOrOptions instanceof URL
      ? new URL(urlOrOptions)
      : new URL(
          `http://${urlOrOptions.hostname || urlOrOptions.host}:${urlOrOptions.port || 80}${urlOrOptions.path || "/"}`,
        );
  if (url.hostname === "127.0.0.1" && Number(url.port) === port) {
    return realRequest.call(http, urlOrOptions, options, callback);
  }
  assert.equal(url.href, "http://api.github.com/downgraded", "external HTTP refused");
  const suppliedUrl = typeof urlOrOptions === "string" || urlOrOptions instanceof URL;
  const opts = suppliedUrl ? (typeof options === "object" ? options : {}) : urlOrOptions;
  const cb = suppliedUrl ? (typeof options === "function" ? options : callback) : options;
  return realRequest.call(
    http,
    {
      ...opts,
      protocol: "http:",
      hostname: "127.0.0.1",
      host: undefined,
      port,
      path: url.pathname,
      agent: undefined,
      lookup: undefined,
      createConnection: undefined,
      headers: { ...opts.headers, "x-fixture-url": url.href },
    },
    cb,
  );
};
http.get = (...args) => {
  const request = http.request(...args);
  request.end();
  return request;
};
https.request = function (urlOrOptions, options, callback) {
  let original, opts, cb;
  if (typeof urlOrOptions === "string" || urlOrOptions instanceof URL) {
    original = new URL(urlOrOptions);
    opts = typeof options === "object" ? { ...options } : {};
    cb = typeof options === "function" ? options : callback;
  } else {
    opts = { ...urlOrOptions };
    cb = options;
    original = new URL(
      `https://${opts.hostname || opts.host}${opts.port ? `:${opts.port}` : ""}${opts.path || "/"}`,
    );
  }
  return realRequest.call(
    http,
    {
      ...opts,
      protocol: "http:",
      hostname: "127.0.0.1",
      host: undefined,
      port,
      path: original.pathname + original.search,
      agent: undefined,
      lookup: undefined,
      createConnection: undefined,
      headers: { ...opts.headers, "x-fixture-url": original.href },
    },
    cb,
  );
};
https.get = (...args) => {
  const request = https.request(...args);
  request.end();
  return request;
};
syncBuiltinESMExports();

// Evaluate the original installed module with explicitly bounded environmental
// collaborators. The module source is neither rewritten nor copied into the repo.
async function nativeModule(file, overrides = {}) {
  const filename = path.join(base, file);
  const localRequire = createRequire(filename);
  const context = vm.createContext({ console, Buffer, process, URL, setTimeout, clearTimeout });
  const module = new vm.SourceTextModule(fs.readFileSync(filename, "utf8"), {
    identifier: filename,
    context,
  });
  await module.link((specifier) => {
    const imported = localRequire(specifier);
    const values = { default: imported.default ?? imported, ...imported, ...overrides[specifier] };
    return new vm.SyntheticModule(
      Object.keys(values),
      function () {
        for (const [key, value] of Object.entries(values)) this.setExport(key, value);
      },
      { context },
    );
  });
  await module.evaluate();
  return module.namespace;
}

(async () => {
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  port = server.address().port;
  try {
    assert.equal(fs.readFileSync(path.join(sdk, "source-image.txt"), "utf8").trim(), input.pin);
    const version = verifiedVersion(
      input.pin,
      JSON.parse(fs.readFileSync(path.join(sdk, "package.json"))).version,
    );
    // Shape/package agreement controls are synthetic; no alternate SDK is acquired.
    for (const malformed of [
      null,
      "",
      "ghcr.io/renovatebot/renovate:latest",
      input.pin.replace("@sha256:", "@sha512:"),
      input.pin.replace("ghcr.io/renovatebot/renovate", "unrelated.invalid/renovate"),
      input.pin + "/suffix",
      input.pin.slice(0, -1),
    ]) {
      assert.throws(() => canonicalVersion(malformed));
    }
    for (const mismatch of [undefined, "", version + "-other", 1]) {
      assert.throws(() => verifiedVersion(input.pin, mismatch));
    }
    const syntheticVersion = "0.0.0";
    const syntheticPin = input.pin.replace(`:${version}@`, `:${syntheticVersion}@`);
    assert.equal(verifiedVersion(syntheticPin, syntheticVersion), syntheticVersion);

    await require(path.join(base, "logger/index.js")).init();
    const { GlobalConfig } = require(path.join(base, "config/global.js"));
    GlobalConfig.set({
      platform: "github",
      localDir: scratch,
      ignorePrAuthor: true,
      productLinks: { documentation: "https://docs.example.invalid/" },
    });
    const { mergeChildConfig } = require(path.join(base, "config/utils.js"));
    const defaults = require(path.join(base, "config/defaults.js")).getConfig();
    const effective = mergeChildConfig(defaults, input.config);
    assert.equal(effective.automerge, false);
    assert.equal(effective.platformAutomerge, false);
    assert.deepEqual(Object.values(effective.statusCheckNames), [null, null, null, null]);
    assert.equal(effective.minimumReleaseAge, "3 days");
    assert.equal(effective.internalChecksFilter, "strict");
    const rules = require(path.join(base, "util/host-rules.js"));
    rules.clear();
    const platform = require(path.join(base, "modules/platform/index.js"));
    const initialized = await platform.initPlatform({
      ...effective,
      token: app,
      endpoint: "https://api.github.com/",
      hostRules: [input.rule],
      gitAuthor: "Fixture <fixture@example.invalid>",
    });
    initialized.hostRules.forEach((rule) => rules.add(rule));
    const { GithubHttp } = require(path.join(base, "util/http/github.js"));
    const client = new GithubHttp();
    const native = await nativeModule("modules/platform/github/index.js", {
      "../../../util/git/index.js": { initRepo: async () => {}, getBranchCommit: () => head },
    });
    await native.initRepo({ repository: repo, renovateUsername: effective.username });
    const wrongGreen = await native.getBranchStatus(head);
    assert.equal(
      wrongGreen,
      "green",
      "do not claim upstream validates wrong-resource green metadata",
    );
    await native.getBranchStatusCheck("fixture-branch", "Required CI");
    const metadata = requests.filter((r) => r.url.startsWith(prefix));
    assert.ok(metadata.some((r) => r.url.endsWith("/status")));
    assert.ok(metadata.some((r) => r.url.includes("/check-runs")));
    assert.ok(metadata.some((r) => r.url.endsWith("/statuses")));
    assert.ok(metadata.every((r) => r.credential === "reader"));
    for (const [state, hex] of [
      ["status403", "b"],
      ["checks403", "c"],
      ["invalid-json", "d"],
    ]) {
      mode = state;
      await assert.rejects(
        native.getBranchStatus(hex.repeat(40)),
        (error) => error.constructor.name === "ExternalHostError",
      );
    }
    mode = "malformed-body";
    assert.equal(
      await native.getBranchStatus("9".repeat(40)),
      "yellow",
      "malformed native metadata remains advisory",
    );
    mode = "pages";
    await native.getBranchStatus("e".repeat(40));
    assert.ok(requests.some((r) => r.url.includes("page=2") && r.credential === "reader"));
    mode = "green";
    const before = requests.length;
    await client.postJson(`repos/${repo}/git/refs`, { body: { fixture: true } });
    await client.patchJson(`repos/${repo}/pulls/1`, { body: { title: "fixture" } });
    await client.getJsonUnchecked(`repos/${repo}/commits/${head}/status`, {
      token: app,
      memCache: false,
    });
    await client.getJsonUnchecked(`repos/${repo}/commits/${head}/status`, {
      repository: repo,
      memCache: false,
    });
    await client.requestGraphql("query { viewer { login } }", { readOnly: true });
    await client.requestGraphql("mutation { fixture }", {});
    await client.getJsonUnchecked("repos/Other/private/commits/a/status");
    assert.ok(requests.slice(before).every((r) => r.credential === "app"));
    for (const url of [
      "https://api.github.com.unrelated.invalid/fixture",
      "https://registry.example.invalid/fixture",
    ]) {
      await client.getJsonUnchecked(url);
      assert.equal(requests.at(-1).credential, "none");
    }
    for (const suffix of ["redirect", "chain", "downgrade", "port-change", "return-chain"]) {
      await client.getJsonUnchecked(`${prefix}${head}/${suffix}`);
    }
    assert.equal(
      requests.find((r) => r.url.includes("Other/private/commits/other")).credential,
      "reader",
    );
    assert.equal(
      requests.find((r) => r.url === "https://api.github.com/same-origin-hop").credential,
      "reader",
    );
    for (const r of requests.filter(
      (r) =>
        r.url.includes("unrelated.invalid/final") ||
        r.url.includes("/downgraded") ||
        r.url === "https://api.github.com:444/final",
    ))
      assert.equal(r.credential, "none");

    assert.equal(
      requests.find((request) => request.url === `${prefix}${head}/returned`).credential,
      "none",
      "cross-origin chain never reattaches credentials on return",
    );
    const effects = [];
    const proposal = { number: 1, title: "fixture", state: "open", labels: [], bodyStruct: {} };
    const mockPlatform = {
      getBranchPr: async () => proposal,
      getBranchStatus: async () => wrongGreen,
      getBranchStatusCheck: async () => null,
      setBranchStatus: async () => effects.push("status"),
      isBranchMergeQueueEnabled: async () => true,
      isPrInMergeQueue: async () => false,
      mergePr: async () => {
        effects.push("queue-or-merge");
        return true;
      },
      reattemptPlatformAutomerge: async () => effects.push("enable-automerge"),
      refreshPr: async () => {},
      massageMarkdown: (value) => value,
    };
    const mockScm = {
      branchExists: async () => true,
      isBranchModified: async () => false,
      isBranchConflicted: async () => false,
      deleteBranch: async () => effects.push("delete"),
      mergeAndPush: async () => effects.push("merge-push"),
      checkoutBranch: async () => {},
    };
    const common = {
      "../../../../modules/platform/index.js": { platform: mockPlatform },
      "../../../../modules/platform/scm.js": { scm: mockScm },
      "../../../../modules/platform/comment.js": {
        ensureComment: async () => effects.push("comment"),
        ensureCommentRemoval: async () => {},
      },
      "./schedule.js": { isScheduledNow: () => true },
      "../branch/schedule.js": { isScheduledNow: () => true },
    };
    const checks = await nativeModule("workers/repository/update/branch/status-checks.js", common);
    const artifacts = await nativeModule("workers/repository/update/branch/artifacts.js", common);
    const reconfigure = await nativeModule("workers/repository/reconfigure/utils.js", {
      "../../../modules/platform/index.js": { platform: mockPlatform },
    });
    for (const stabilityStatus of ["green", "yellow", "red"]) {
      const config = {
        ...effective,
        branchName: "fixture",
        stabilityStatus,
        confidenceStatus: "green",
        artifactErrors: [{ fileName: "lock", stderr: "graph failure" }],
      };
      await checks.setStability(config);
      await checks.setConfidence(config);
      await artifacts.setArtifactErrorStatus(config);
      await reconfigure.setBranchStatus(
        "fixture",
        "invalid config",
        "red",
        effective.statusCheckNames.configValidation,
      );
    }
    assert.deepEqual(effects, []);
    const publishControl = {
      ...effective,
      statusCheckNames: defaults.statusCheckNames,
      branchName: "fixture",
      stabilityStatus: "yellow",
      confidenceStatus: "green",
      artifactErrors: [{ fileName: "lock", stderr: "graph failure" }],
    };
    await checks.setStability(publishControl);
    await checks.setConfidence(publishControl);
    await artifacts.setArtifactErrorStatus(publishControl);
    await reconfigure.setBranchStatus(
      "fixture",
      "invalid config",
      "red",
      defaults.statusCheckNames.configValidation,
    );
    assert.equal(
      effects.filter((effect) => effect === "status").length,
      4,
      "all four original publisher paths are exercised",
    );
    effects.length = 0;
    const prMerge = await nativeModule("workers/repository/update/pr/automerge.js", {
      ...common,
      "../branch/status-checks.js": checks,
    });
    const branchMerge = await nativeModule("workers/repository/update/branch/automerge.js", {
      ...common,
      "./status-checks.js": checks,
    });
    let proposals = 0,
      generations = 0;
    const worker = await nativeModule("workers/repository/update/branch/index.js", {
      ...common,
      "./status-checks.js": checks,
      "./artifacts.js": artifacts,
      "../pr/automerge.js": prMerge,
      "./automerge.js": branchMerge,
      "../../../global/limits.js": { getCount: () => 0, isLimitReached: () => false },
      "./check-existing.js": { prAlreadyExisted: async () => null },
      "./reuse.js": {
        shouldReuseExistingBranch: async (config) => ({ ...config, reuseExistingBranch: true }),
      },
      "../pr/index.js": {
        ensurePr: async () => {
          proposals++;
          return { type: "with-pr", pr: proposal };
        },
      },
      "./get-updated.js": {
        getUpdatedPackageFiles: async () => ({ updatedPackageFiles: [], updatedArtifacts: [] }),
      },
      "../../../../modules/manager/npm/post-update/index.js": {
        getAdditionalFiles: async () => ({ artifactErrors: [], updatedArtifacts: [] }),
      },
      "../../../../modules/manager/github-actions/artifacts.js": {
        updateActionsLockfile: async () => ({ artifactErrors: [], updatedArtifacts: [] }),
      },
      "../../changelog/index.js": { embedChangelogs: async () => {} },
      "./execute-post-upgrade-commands.js": {
        default: async () => {
          generations++;
          return null;
        },
      },
      "./bump-versions.js": { bumpVersions: async () => {} },
      "./commit.js": { commitFilesToBranch: async () => "1".repeat(40) },
    });
    const branchConfig = {
      ...effective,
      baseBranch: "main",
      branchName: "renovate-devcontainer/features",
      cacheFingerprintMatch: "matched",
      errors: [],
      warnings: [],
      upgrades: [
        {
          minimumReleaseAge: "3 days",
          minimumReleaseAgeBehaviour: "timestamp-required",
          releaseTimestamp: new Date(Date.now() - 4 * 86400000).toISOString(),
        },
      ],
    };
    const outcome = await worker.processBranch(branchConfig);
    assert.equal(outcome.result, "done");
    assert.equal(proposals, 1, "native proposal processing remains reachable");
    assert.deepEqual(effects, [], "advisory green metadata cannot authorize native integration");
    for (const automergeType of ["branch", "pr-comment"]) {
      assert.equal((await worker.processBranch({ ...branchConfig, automergeType })).result, "done");
    }
    assert.equal(
      (await worker.processBranch({ ...branchConfig, cacheFingerprintMatch: "no-match" })).result,
      "done",
    );
    assert.equal(generations, 1, "fixed generation and new commit path remain reachable");
    assert.deepEqual(
      effects,
      [],
      "no status, merge, queue, automerge enablement, comment or branch deletion",
    );
    // Positive control: the same original worker can reach native queue effects
    // with the old flag. This prevents an early/no-work fixture from passing.
    await worker.processBranch({ ...branchConfig, automerge: true });
    assert.ok(effects.includes("queue-or-merge"));
    effects.length = 0;
    const age = require(path.join(base, "util/minimum-release-age.js"));
    assert.equal(
      age.checkMinimumReleaseAge(effective, new Date(Date.now() - 4 * 86400000).toISOString())
        .isPending,
      false,
    );
    assert.equal(age.checkMinimumReleaseAge(effective, new Date().toISOString()).isPending, true);
    assert.equal(age.checkMinimumReleaseAge(effective, undefined).isPending, true);
    assert.equal(
      age.checkMinimumReleaseAge(
        { ...effective, minimumReleaseAgeBehaviour: "timestamp-optional" },
        undefined,
      ).isPending,
      false,
    );
    mockScm.branchExists = async () => false;
    mockPlatform.getBranchPr = async () => null;
    const pending = await worker.processBranch({ ...branchConfig, pendingChecks: true });
    assert.equal(pending.result, "pending");
    assert.deepEqual(effects, []);
    console.log(
      JSON.stringify({
        version,
        metadata403: "abort",
        wrongResourceNativeGreen: true,
        nativeIntegrationEffects: 0,
        optionalStatusEffects: 0,
        proposalProcessing: true,
        oldAutomergeControlReached: true,
        ageGatesPreserved: true,
        requests: requests.map(({ body, ...request }) => request),
      }),
    );
  } finally {
    server.closeAllConnections();
    await new Promise((resolve) => server.close(resolve));
    fs.rmSync(scratch, { recursive: true, force: true });
  }
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
