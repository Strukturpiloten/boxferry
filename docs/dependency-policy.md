# Dependency and license policy

Keep dependency versions in their manifest, lockfile, workflow, or installer.

## Sources of truth

| Concern                      | Canonical source                       |
| ---------------------------- | -------------------------------------- |
| Rust requirements/features   | workspace and crate `Cargo.toml` files |
| Resolved Rust graph          | `Cargo.lock`                           |
| Allowed licenses/sources     | `deny.toml`                            |
| Node development tools       | `package.json` and `package-lock.json` |
| File-quality tools           | `scripts/install-file-tools.sh`        |
| Kubernetes development tools | `scripts/install-kubernetes-tools.sh`  |
| Workflow tools and Actions   | `.github/workflows/`                   |
| Dev Container tools          | `.devcontainer/`                       |

## Review rules

- Prefer the standard library and focused, actively maintained crates.
- Use explicit compatible Cargo requirements; wildcard requirements are denied.
- Use crates.io releases unless an accepted decision records another source.
- Keep default features only when their behavior is understood and needed.
- Avoid multiple crates for the same role without a documented reason.
- Record dependencies that shape architecture, representation, or public APIs in an ADR.
- Commit the lock file and use locked resolution in CI.

`deny.toml` allows only Apache-2.0, MIT, MPL-2.0, and Unicode-3.0. Additional licenses require reviewed
dependencies and understood obligations. This policy is project intent, not legal advice.

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

Every operational pin has exactly one Renovate owner. The `github-runners` regex manager owns
fixed `ubuntu-*`, `macos-*`, and `windows-*` labels, including numeric architecture/size suffixes.
These reserved hosted labels must be unquoted, unanchored scalars; dynamic matrices and self-hosted
forms stay distinct. Runner groups never auto-merge: review release notes and every affected
workflow. Native managers own Cargo, npm, Rust toolchains, Dev Container features, and Actions.
Regex managers own downloaded tools, atomic Dev Container image version/digest pairs,
checksum-pinned Compose providers, active application images, and live probe images.
Workflows call the canonical version/checksum installer `scripts/lib/compose-provider.sh`;
never duplicate its release URLs.

The Dockerfile's regex manager owns atomic release/digest updates. Compose and Quadlet documents are
fixtures. Their native managers stay disabled, preventing duplicate Dev Container proposals and
updates to intentionally invalid fixture registries. Curated application `images.tsv` catalogues
retain explicit management.

Lens conformance release refs use approval-gated `github-tags` extraction.
Checksum-bearing provider and live-image proposals require Dependency Dashboard approval, never
auto-merge. Review release assets/platform digests, provenance, license, resource budgets, and live
behavior before catalogue updates. Policy tests may assert Action identity and immutable-pin shape,
never Renovate-owned revisions.

The application dispatcher's canonical `ADMISSION_REVISION` pins DockerLens #42's reviewed merge
commit: unreleased infrastructure, not a package release. No helper tag or downloadable checksum
exists; invent neither. One `github-digest` manager owns this pin; a later rule requires Dashboard
approval and disables automerge. Review producer/consumer together: both actors' permissions,
exact-head admission, privacy, and fail-closed cases.

Captured cassettes, generated expectations, historical observations, and the Podman compatibility
matrix remain evidence; version changes require owning evidence/version-boundary revalidation.
API contract dates and fixed support targets are reviewed compatibility decisions, outside updates.

Renovate's global three-day direct-update age has one exact-name Cargo/crate exception: six BoxFerry
workspace packages plus `compose-lens`, `podman-lens`, `quadlet-lens`, and `docker-lens`. Prefixes,
wildcards, other managers, and same-name npm packages remain excluded.

Lock maintenance's unsupported synthetic Renovate age status is zero; auto-merge requires the
aggregate gate's shared check of every introduced Cargo/npm registry version. Only these four
canonical crates.io Cargo packages waive the guard's age threshold, after successful bounded registry
lookup. Missing, malformed, or future publication timestamps fail closed. Other versions, including
third-party and transitive dependencies, retain 72 hours. Only age changes: immutable pins, Cargo
checksums, lockfile integrity, audits, required checks, and manual approvals remain mandatory. The
existing shared-policy manager owns the guard's immutable commit; no new manager or extraction path
is added. Checksum, image, runner, provider, and Dev Container updates remain manual. Actions stay
SHA-pinned, release tools version/checksum-pinned, and release-plz only prepares. Formatting tools
cannot change the published graph or MSRV.

Run `cargo deny check` and the complete repository gate after dependency changes. Local link
checks are deterministic and offline; external URL checks run separately on a schedule or by
manual request.

### Workflow changes and Renovate ownership

Refactors require dependency-automation review even without version changes. For every added,
changed, moved, or removed operational pin:

1. Record canonical sources and affected scripts, CI/release workflows, Dev Containers, and
   cross-repository consumers in the issue or PR.
2. Check native/custom manager ownership, file patterns, extraction expressions, versioning,
   grouping, approval rules, and consumer reference updates. Remove obsolete matches; prove each
   operational pin is extracted exactly once at its intended source.
3. Keep full Action/reusable-workflow SHAs paired with exact release tags. Preserve reviewed
   version/checksum and image release/digest pairs; never relax approvals to make an update merge.
4. Validate Renovate and extraction/regression checks, including composite actions and shared scripts.
   Keep historical fixtures, compatibility anchors, and intentionally invalid inputs outside updates.
5. Verify every affected consumer and link coordinated changes. If Renovate needs no edit, record
   the extraction evidence and reason rather than assuming an existing regex still matches.

Shared workflow ownership and consumer adoption are tracked in
[issue #309](https://github.com/Strukturpiloten/boxferry/issues/309). Reuse validation logic without
coupling the independently published Lens packages to BoxFerry.
