# BoxFerry fixtures

Fixtures are executable evidence, not sample configuration. Store each case in
`fixtures/<suite>/<id>/` with a `fixture.toml` manifest and only the files named by that
manifest.

## Manifest

```toml
schema = 1
id = "minimal-conversion"
suite = "conversion"
description = "Protects a minimal lossless conversion."
secrets_reviewed = true
files = ["compose.yaml", "expected.container"]

[provenance]
source = "authored"
license = "MPL-2.0"
redistribution = "allowed"
modifications = "none"

[environment]
description = "No runtime or process environment is provided."

[expectations]
summary = "The workload converts without loss."
```

IDs and suite names use lowercase ASCII letters, digits, and hyphens. The ID matches the directory;
the suite matches its parent. Valid suites are `model`, `adapter-contract`, `conversion`,
`roundtrip`, `differential`, and `real-world`.

## Evidence and safety

- `authored`, `external`, and `generated` are the supported provenance sources.
- External evidence includes an immutable URL and revision.
- Generated evidence records the oracle implementation, exact version, and command.
- Every case records license, redistribution, modifications, environment, and expected behavior.
- Every listed path is relative, inside the fixture directory, present, and unique.
- Set `secrets_reviewed = true` only after inspecting every listed file.
- Never store credentials, private runtime output, or unredistributable upstream data.

The repository-policy suite validates these rules.

## Live conformance matrix

`fixtures/conformance/podman-live/matrix.tsv` is not a parser fixture and does not enter the
deterministic fixture corpus. It is the reviewed inventory of trusted Strukturpiloten nested-Podman
images used only by `scripts/podman-live-conformance.sh`. Each row names one image, its declared
Podman/package version, distribution family, inner root mode, and execution lane.

`scenarios.tsv` is the equally reviewed end-to-end route inventory. `limitations.tsv` records a
temporary, verified image-level reason why one container cell cannot run that route inventory. The
runner validates all three catalogues before it pulls an image, so changing a selector, exporter,
reimport, supported image, or coverage exception is an explicit review event. Its
external apply/reacquire case runs generated commands only inside a fresh disposable Podman 6.1
harness target; BoxFerry remains read-only and nonexecuting.
That target receives `podman-live/apply-target-containers.conf`, which disables Netavark firewall
rules because the pinned nested image has no `nft` binary. The case therefore proves resource
planning, isolated network membership, and reacquisition—not NAT or host firewall behavior.

The matrix has exactly 48 rootful/rootless cells. It is conformance evidence, not the public finite
compatibility contract; `boxferry capabilities` is the installed-build source of truth. Every image
column is a tag plus exact `@sha256:` digest; the runner rejects an unpinned row before it pulls
anything. Each row records its expected `amd64` architecture, and the evidence captures resolved
digest, package version, Podman/API version, and rootless state.

All 48 cells run as digest-pinned images inside disposable privileged outer Podman containers.
Forty-three execute the complete live-resource suite. The five UBI/openSUSE rootless rows in
`limitations.tsv` verify the published images' `newuidmap` permission failure and report no
resource coverage. Once corrected image digests initialize nested rootless Podman, remove their
limitation rows so they execute the same suite as every other cell.

The four-cell pull-request smoke profile uses the same runner and real workloads but a bounded
scenario subset. Every cell uses a minimal container/network/volume workload and exports one exact
selection to all targets. Version-independent CLI, policy, redaction, and malformed-input checks run
once on Podman 6.1 rootful. The complete workload and runtime matrix remain in `full-container`.
Only `full-container` evidence is labelled `full`; smoke evidence is labelled
`smoke`. Each cell
prints its planned test count, timestamped start/pass/fail events, and elapsed time. The host engine
verifies the pinned workload image and passes an archive to the nested engines, so live-resource
creation does not depend on nested registry networking.

Retained failure artifacts are local diagnostic evidence. Review them for environment values,
resource names, image references, paths, and topology before sharing; never commit raw live output.

## Route scenarios

Every positive adapter or conversion case declares one or more
`extensions.scenarios`. A scenario names its input, source files, and one expectation for every
exporter reported by `boxferry capabilities`. Export expectations define loss policy,
diagnostics, exact artifacts, and relevant target bounds.

Compose scenarios may provide explicit interpolation values. Quadlet scenarios name an
application and may select grouping or Podman bounds. Protected values never appear in reports but
may remain in explicitly authorized artifacts. `normalize-project-root = true` replaces only the
fixture checkout root with `<project>`.

The corpus runner tests stricter blocking policies, complete artifact and diagnostic sequences,
redaction, and applicable re-import or deterministic-output contracts. Expected artifacts are
never inferred as native input.

`real-world/corpus.toml` is a separate pinned-remote contract. Its opt-in test retrieves upstream
Compose files without vendoring them. See [testing](../docs/testing.md) and the
[application test map](../crates/boxferry/tests/README.md).
