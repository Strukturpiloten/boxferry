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

## Automation

Each operational pin has one Renovate owner. Native managers cover Cargo, npm, Rust toolchains,
Dev Container features, Actions and supported literal hosted runners. Custom managers cover
annotated matrices and legacy runners pending adoption, downloaded tools, image version/digest
pairs, Compose providers and selected application images. Reserved hosted labels stay unquoted/unanchored scalars;
dynamic matrices/self-hosted runners remain distinct. Workflows use `scripts/lib/compose-provider.sh`.
Fixtures, historical evidence, compatibility targets and invalid fixture registries stay outside
updates; curated `images.tsv` catalogues retain ownership.

Coverage uses one local/CI/image version authority: the annotated
[candidate Dockerfile ARG](../.devcontainer/Dockerfile). Shared coverage requires it without fallback
and installs/verifies the exact CLI/compiler; local validation verifies actual Cargo dispatch. Only
this ARG leaves the direct-image/tool hold; its extractor retains automatic updates,
third-party age and native qualification. Other holds remain.
See [ADR 0099](decisions/0099-candidate-coverage-tool-authority.md).

### Rollout of shared dependency automation

[#549](https://github.com/Strukturpiloten/boxferry/issues/549) coordinates shared presets and
validation. Automatic dependency/tool merges require compatibility, integrity and protected checks,
without routine PR review. Python versions, Renovate ownership and repository-specific tests/rules
remain explicit.

PRs/merge queues enforce 72 hours for third-party versions, including introduced lockfile
transitives. Verified first-party published-package updates waive whole-closure age, including
necessary young transitives, only after trusted complete-closure proof; missing metadata blocks
admission. Checksums, lockfile integrity, audits, compatibility and required checks remain mandatory.
Provider/image updates retain provenance, license, resource-budget and native-behavior evidence.
Unreliable publication times require trusted first-seen delay; malformed/future times fail.
Main/manual/release validation remains complete without restarting admission waits.

Future schema-2 Git source reading requires canonical kernel.org identity/checksum matching
before the trusted 72-hour hold. It supplies no publication date, PGP verification,
operational pin, archive download or installation.

Rollout remains incomplete. Adopt immutable common and Rust presets, manifest, workflow references,
explicit implementation inputs and annotated checkout refs together. Native GitHub Actions own
reusable references; shared regex managers own other declarations once. Fixed workload, runtime
matrix, probe images and Compose provider remain
outside proposals. Privileged Docker admission/custody pins retain separate explicit security review.
Direct Dev Container image, feature and image-only tool proposals remain disabled pending
CI qualification under #563, without a Dashboard or manual-approval route. Ordinary shared
source, package and compiler inputs retain automatic ownership when Dockerfiles copy them;
this does not qualify the resulting image closure. Release preparation grants no publication
or deployment authority.

### Runtime evidence and pin changes

The dispatcher's `ADMISSION_REVISION` pins DockerLens #42's reviewed, unreleased merge commit;
its `github-digest` owner and approval differ from ordinary updates. Invent no helper tag/checksum.
The [consumer runtime](decisions/0079-native-test-runtime-selection.md#runner-prerequisites-and-renovate)
reuses it. Exact-pair execution requires renewed source/closure review, not automatic updates or
generated receipts. Historical evidence and version-boundary revalidation retain their owners.

For every added, changed, moved, or removed pin:

1. Identify its canonical source and affected local, CI, release, Dev Container and cross-repository consumers.
2. Verify one Renovate extraction, effective grouping and approvals, and matching version/checksum or tag/digest pairs.
3. Preserve full Action and reusable-workflow SHAs with exact release-tag comments, except documented infrastructure bootstrap. Exclude historical evidence and intentional fixtures.
4. Run extraction and regression checks for every affected consumer; record evidence when Renovate needs no change.

After dependency changes run `cargo deny check` and the complete gate. Local links are checked
offline; external URL checks run separately on schedule or request.

Compose provider extraction pairs its canonical release/checksum and all six active application
rows. Required integrity reads exact tested regular Git blobs and hashes the bounded official
Linux x86-64 asset without storing, installing or executing it; local validation uses
`--verify-working-tree`. Proposals remain disabled until approved native compatibility qualification is
delivered, without Dashboard/PR approval.

### Kubernetes development tools

Renovate owns kind's per-architecture tags/digests and kubectl, Helm and Kustomize versions,
under one automatic merge rule without Dashboard approval. Trusted
`kubernetes-tool-checksum-refresh.yml` runs on schedule/CI: bounded official checksum metadata,
both architecture artifacts streamed for SHA-256 without storage, validated Renovate PR and exact
installer-only diff, then a metadata-only commit to that PR's exact head. Candidate content and
artifacts never execute. Required integrity reads the exact tested Git installer; the complete
local gate checks the explicitly selected working tree.

The installer records kind/Kustomize release-asset checksums, kubectl's `dl.k8s.io` checksum
and Helm's `get.helm.sh` checksum. Other tooling ownership remains.

### API baseline policy ownership

[ADR 0096](decisions/0096-development-and-published-api-baselines.md) changes no pin values.
SemVer and evidence-upload Actions retain native extraction, grouping and approvals.
Existing Cargo/compiler and consumer automation-pin owners remain authoritative;
add no custom manager or version inventory.
