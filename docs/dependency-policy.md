# Dependency and license policy

Use this guide when a change adds, removes, pins, or enables a dependency. Exact versions belong in
the manifest, lock file, workflow, or installer that uses them—not in this page.

## Sources of truth

| Concern                    | Canonical source                       |
| -------------------------- | -------------------------------------- |
| Rust requirements/features | workspace and crate `Cargo.toml` files |
| Resolved Rust graph        | `Cargo.lock`                           |
| Allowed licenses/sources   | `deny.toml`                            |
| Node development tools     | `package.json` and `package-lock.json` |
| File-quality tools         | `scripts/install-file-tools.sh`        |
| Workflow tools and Actions | `.github/workflows/`                   |
| Dev Container tools        | `.devcontainer/`                       |

## Review rules

- Prefer the standard library and focused, actively maintained crates.
- Use explicit compatible Cargo requirements; wildcard requirements are denied.
- Use crates.io releases unless an accepted decision records another source.
- Keep default features only when their behavior is understood and needed.
- Avoid multiple crates for the same role without a documented reason.
- Record dependencies that shape architecture, representation, or public APIs in an ADR.
- Commit the lock file and use locked resolution in CI.

`deny.toml` intentionally allows only Apache-2.0, MIT, MPL-2.0, and Unicode-3.0. Add a license
only for a reviewed dependency whose obligations are understood. This policy records project
intent and is not legal advice.

An advisory, license clarification, duplicate allowance, or source exception must be narrow,
versioned, and explained where it is configured. Never add an exception only to make CI green.

## Architectural dependencies

- Clap supplies the optional `cli` feature; embedded callers can disable defaults.
- PodmanLens owns Podman acquisition, observations, evidence, planning, and rendering.
  `boxferry-podman` owns only semantic mapping.
- ZIP and Jiff are CLI-only implementation details for privacy-safe diagnostic archives and local
  filenames. They do not enter no-default embedded builds. ZIP stays on the newest release line
  compatible with Rust 1.85.0; its Renovate ceiling moves only with an intentional MSRV review.
- `boxferry-podman` carries an exact development-only `yoke-derive` constraint so Cargo lock-file
  maintenance cannot select the upstream release that fails on Rust 1.85.0. The Cargo native
  manager owns the manifest pin, and a matching Renovate exclusion suppresses known-broken proposals.
  Remove both constraints only after a newer upstream release passes the unchanged MSRV gate;
  never replace the constraint by making that gate optional.
- ComposeLens and QuadletLens own their native document semantics.

Review the relevant manifest and ADR for exact features and constraints.

## Automation

Every operational software pin used by CI, the Dev Container, or a live harness has one Renovate
owner. Canonical fixed GitHub-hosted `ubuntu-*`, `macos-*`, and `windows-*` workflow labels belong
to the Renovate `github-runners` regex manager, including numeric architecture or size suffixes.
Those prefixes are reserved for Renovate-owned hosted labels, which use an unquoted and unanchored
scalar so extraction stays explicit. Dynamic matrix expressions and self-hosted forms remain
visibly distinct. Grouped runner proposals never auto-merge: review hosted environment release
notes and every affected workflow before merging. Native managers own Cargo, npm, Rust toolchains,
Dev Container features, and
GitHub Actions. Explicit regex managers own downloaded CLI versions, atomic Dev Container base
image release/digest pairs, the checksum-pinned Docker Compose provider, actively executed
application images, and the live workload probe image. The shared
`scripts/lib/compose-provider.sh` file is the canonical Compose provider version and checksum;
workflows call its installer rather than duplicating release URLs.

The repository-owned Dockerfile is already managed atomically by its release/digest regex manager,
and every Compose or Quadlet document is fixture input. Their native managers are disabled so they
cannot duplicate Dev Container proposals or treat intentionally invalid fixture registries as live
dependencies. Curated application `images.tsv` catalogues remain visible through their explicit
manager.

Checksum-bearing provider and live-image proposals require Dependency Dashboard approval and never
auto-merge. Review the new release asset or platform manifest digest, provenance, license, resource
budget, and live behavior before updating the corresponding catalogues. Semantic policy tests may
assert action identity and immutable-pin shape, but must not embed a Renovate-owned action revision.

Captured cassettes, generated expectations, historical observations, and the Podman compatibility
matrix are retained evidence rather than floating dependencies. Their recorded versions change only
through the owning evidence or version-boundary revalidation workflow. API contract dates and fixed
support targets are likewise reviewed compatibility decisions, not update streams.

Renovate proposes updates, but every proposal requires the normal tests and review. GitHub Actions
remain SHA-pinned; downloaded release tools remain version-and-checksum pinned; release-plz remains
preparation-only. Repository formatting tools never enter the published Rust graph or change the
MSRV.

Run `cargo deny check` and the complete repository gate after dependency changes. Local link
checks are deterministic and offline; external URL checks run separately on a schedule or by
manual request.

### Workflow changes and Renovate ownership

A task refactor is also a dependency-automation review, even when version numbers do not change.
For every added, changed, moved, or removed operational pin:

1. Identify its canonical source and affected local scripts, CI/release workflows, Dev Containers,
   and cross-repository consumers. Record the impact in the issue or PR.
2. Check native/custom manager ownership, file patterns, extraction expressions, versioning,
   grouping, approval rules, and consumer reference updates. Remove obsolete matches; prove each
   operational pin is extracted exactly once at its intended source.
3. Keep full Action/reusable-workflow SHAs paired with exact release tags. Preserve reviewed
   version/checksum and image release/digest pairs; never relax approvals to make an update merge.
4. Validate Renovate configuration and exercise extraction/regression checks against the new
   layout, including composite actions and shared scripts. Ensure immutable historical fixtures,
   compatibility anchors, and intentionally invalid inputs remain outside update streams.
5. Verify every affected consumer and link coordinated changes. If Renovate needs no edit, record
   the extraction evidence and reason rather than assuming an existing regex still matches.

Shared workflow ownership and consumer adoption are tracked in
[issue #309](https://github.com/Strukturpiloten/boxferry/issues/309). Reuse validation logic without
coupling the independently published Lens packages to BoxFerry.
