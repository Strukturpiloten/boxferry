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
- Commit lockfiles; CI uses locked resolution.

`deny.toml` permits Apache-2.0, MIT, MPL-2.0, and Unicode-3.0. Other licenses require review and
understood obligations. Exceptions must be narrow, versioned and explained beside configuration;
CI alone never justifies one. Project intent, not legal advice.

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

Exact constraints: manifests and ADRs.

## Automation

Each operational pin has one Renovate owner. Native managers cover Cargo, npm, Rust toolchains,
Dev Container features, Actions and supported literal hosted runners. Custom managers retain
annotated matrices and legacy runner ownership pending shared adoption, downloaded tools,
image version/digest pairs, Compose providers and selected application images. Reserved hosted
labels remain unquoted, unanchored scalars; dynamic matrices and self-hosted runners stay distinct.
Workflows use `scripts/lib/compose-provider.sh`. Fixtures, historical observations, compatibility
targets and invalid fixture registries are not update candidates; curated `images.tsv` catalogues
retain ownership.

### Rollout of shared dependency automation

[Issue #549](https://github.com/Strukturpiloten/boxferry/issues/549) coordinates shared BoxFerry
Renovate presets and validation. Policy automatically merges dependency/tool updates after
compatibility, integrity and required checks pass, without manual PR review. Python versions
and Renovate ownership remain explicit; repositories retain tests and local rules.

PRs and merge queues enforce 72 hours for third-party versions, including new transitive
lockfile versions. Verified first-party published-package updates waive age for the whole
closure, including attributable young transitive lockfile versions. Checksums, lockfile integrity,
audits, compatibility and required checks remain mandatory. Provider/image updates also need
applicable provenance, license, resource-budget and native-behavior evidence. Unreliable publication
times require a supported, trusted first-seen delay; malformed or future times fail. Main, manual
and release validation retain complete integrity/compatibility without restarting admission waiting.

Rollout remains incomplete. Adopt immutable common and Rust presets, manifest, workflow references,
explicit implementation inputs and annotated checkout refs together. Native GitHub Actions own
reusable references; shared regex managers own other declarations once. The trusted dependency
service validates complete first-party closures before waiving whole-update age; missing metadata
blocks admission. Fixed workload, runtime matrix, probe images and Compose provider remain
outside proposals. Privileged Docker admission/custody pins retain separate explicit security review.
Direct Dev Container image, feature and image-only tool proposals remain disabled pending
CI qualification under #563, without a Dashboard or manual-approval route. Ordinary shared
source, package and compiler inputs retain automatic ownership when Dockerfiles copy them;
this does not qualify the resulting image closure. Release preparation grants no publication
or deployment authority.

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

Renovate owns kind's paired per-architecture tags/digests and kubectl, Helm and Kustomize
versions. All four share one automatic merge rule without Dashboard approval. Trusted
`kubernetes-tool-checksum-refresh.yml` runs on schedule and CI: bounded official checksum
metadata, both architecture artifacts streamed for SHA-256 without storage, validated
Renovate PR and exact installer-only diff, then a metadata-only commit to that PR's exact head.
Candidate content and artifacts never execute. Required integrity reads the exact tested Git
installer; the complete local gate checks the explicitly selected working tree.

The installer records kind/Kustomize release-asset checksums, kubectl's `dl.k8s.io` checksum
and Helm's `get.helm.sh` checksum. Other tooling ownership remains.

### API baseline policy ownership

[ADR 0096](decisions/0096-development-and-published-api-baselines.md) changes no pin values.
SemVer and evidence-upload Actions retain native extraction, grouping and approvals.
Existing Cargo/compiler and consumer automation-pin owners remain authoritative;
add no custom manager or version inventory.
