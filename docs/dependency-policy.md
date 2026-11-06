# Dependency and license policy

Canonical manifests, lockfiles, workflows and installers own dependency versions.

## Sources of truth

| Concern                               | Canonical source                               |
| ------------------------------------- | ---------------------------------------------- |
| Rust requirements, features and graph | workspace and crate `Cargo.toml`; `Cargo.lock` |
| Licenses and sources                  | `deny.toml`                                    |
| Node tools                            | `package.json`, `package-lock.json`            |
| File-quality tools                    | `automation/tool-artifacts.json`               |
| Kubernetes tools                      | `scripts/install-kubernetes-tools.sh`          |
| Workflows and Actions                 | `.github/workflows/`, `.github/actions/`       |
| Dev Container tools                   | `.devcontainer/`                               |

## Review rules

- Prefer the standard library and focused, maintained crates; justify duplicate roles.
- Use explicit compatible Cargo requirements, not wildcards; use crates.io unless an ADR says otherwise.
- Enable only understood, necessary default features. Record architectural, representation and
  public-API dependencies in an ADR.
- Commit the lock file and use locked resolution in CI.

`deny.toml` permits Apache-2.0, MIT, MPL-2.0, and Unicode-3.0. Other licenses require review and
understood obligations. Exceptions must be narrow, versioned and explained beside configuration;
CI alone never justifies one. This is project intent, not legal advice.

## Architectural dependencies

- Clap supplies optional `cli`; embedded callers can disable defaults.
- PodmanLens owns Podman acquisition, observations, evidence, planning and rendering;
  `boxferry-podman` owns semantic mapping.
- ZIP and Jiff are CLI-only for diagnostic archives and local filenames, excluded from no-default
  builds. ZIP uses advisory-fixed 9.x with Renovate floor 9.0.3; online audits have no ZIP exceptions.
- Cargo owns the Rust 1.98.1 minimum, aligned with Lens and Clippy; local, CI and release checks
  read Cargo metadata. `rust-toolchain.toml` has one Renovate owner; updates never implicitly raise
  the minimum.
- The former Rust-1.85-only direct `yoke-derive` constraint and Renovate exclusion are removed.
  Retain transitive lock resolutions and full MSRV and dependency gates; claim no new native compatibility.
- ComposeLens and QuadletLens own their native document semantics.

Review manifests and ADRs for exact constraints.

## Automation

Every operational pin has one Renovate owner. Native managers cover Cargo, npm, Rust toolchains,
Dev Container features, Actions and supported literal hosted runner selectors. Custom managers
retain annotated runner matrices and legacy runner ownership pending shared adoption, alongside
downloaded tools, image version/digest pairs, Compose providers and selected application images.
Reserved hosted runner labels are unquoted, unanchored scalars; dynamic matrices and self-hosted runners remain distinct.
Workflows use `scripts/lib/compose-provider.sh`. Fixtures, historical observations, compatibility
targets and invalid fixture registries are not update candidates; curated `images.tsv` catalogues
retain ownership.

### Rollout of shared dependency automation

[Issue #549](https://github.com/Strukturpiloten/boxferry/issues/549) coordinates the move to shared
BoxFerry Renovate presets and validation. The intended policy automatically merges dependency and
tool updates after compatibility, integrity and required checks pass, without manual PR review.
Python tools have explicit versions and Renovate ownership; repositories retain tests and local rules.

For PRs and merge queue candidates, third-party versions, including newly introduced transitive
lockfile versions, wait at least 72 hours. A verified first-party published-package update is exempt
as a whole, including attributable young transitive lockfile changes. Only age is exempt: checksums,
lockfile integrity, audits, compatibility and required checks still apply. Provider and image updates
also need applicable provenance, license, resource-budget and native-behavior evidence. Without
reliable publication times, use a supported, trusted first-seen delay; malformed or future times fail.
Main, manual and release validation retain complete integrity and compatibility checks without
restarting admission waiting.

This is the target, not completed rollout. Until adoption, each repository's actual Renovate
configuration and required age check remain authoritative. BoxFerry's existing lockfile check exempts
first-party crate versions, not their third-party transitive updates. Some runner, image and tool
proposals still need manual approval. Release preparation does not authorize publication; releases
and deployments retain maintainer approval.

### Runtime evidence and pin changes

The application dispatcher's `ADMISSION_REVISION` pins DockerLens #42's reviewed, unreleased merge
commit. Its `github-digest` owner and approval rule differ from ordinary updates; invent no helper tag
or downloadable checksum. The [consumer runtime](decisions/0079-native-test-runtime-selection.md#runner-prerequisites-and-renovate)
reuses it. Exact-pair execution requires renewed source and closure review, never an automatic update
or generated receipt. Historical evidence and version-boundary revalidation remain with their owners.

For every added, changed, moved, or removed pin:

1. Identify its canonical source and affected local, CI, release, Dev Container and cross-repository consumers.
2. Verify one Renovate extraction, effective grouping and approvals, and matching version/checksum or tag/digest pairs.
3. Preserve full Action and reusable-workflow SHAs with exact release-tag comments, except documented infrastructure bootstrap. Exclude historical evidence and intentional fixtures.
4. Run extraction and regression checks for every affected consumer; record evidence when Renovate needs no change.

After dependency changes run `cargo deny check` and the complete gate. Local links are checked
offline; external URL checks run separately on schedule or request.

Compose provider extraction pairs the canonical release/checksum with all six active application
provider rows. Required integrity verification reads the exact tested commit's regular Git blobs and
hashes the bounded official Linux x86-64 asset without storing, installing or executing it; the complete
local gate selects `--verify-working-tree`. Integrity verification is ready. Provider proposals remain
provisionally disabled until the native compatibility qualification decision and its delivery, without
a Dashboard or PR approval route.

### Kubernetes development tools

Renovate owns kind's paired per-architecture tags and digests and kubectl, Helm and Kustomize
versions. These four packages share one auto-merge rule without Dashboard approval. Trusted
`kubernetes-tool-checksum-refresh.yml` runs on schedule and after CI. It reads bounded official
checksum metadata, streams both architecture artifacts to SHA-256 without storing them, validates
the Renovate PR and exact installer-only diff, and updates only that PR's exact head with a
metadata-only commit. Candidate content and artifacts are never executed. Required integrity reads
the installer from the exact tested Git commit; the complete local gate verifies the explicitly
selected working tree.

The installer records kind and Kustomize release-asset checksums, kubectl's `dl.k8s.io` checksum,
and Helm's `get.helm.sh` checksum. Other tooling ownership remains.

### API baseline policy ownership

[ADR 0096](decisions/0096-development-and-published-api-baselines.md) changes no pin values.
The reused SemVer Action and added evidence-upload occurrence retain native GitHub Actions
extraction, grouping and approvals. Existing Cargo/compiler and consumer automation-pin
owners remain authoritative; no custom manager or version inventory is added.
