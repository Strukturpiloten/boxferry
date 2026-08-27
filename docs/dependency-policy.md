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
- ComposeLens and QuadletLens own their native document semantics.

Review the relevant manifest and ADR for exact features and constraints.

## Automation

Renovate proposes updates, but every proposal requires the normal tests and review. GitHub Actions
remain SHA-pinned; downloaded release tools remain version-and-checksum pinned; release-plz remains
preparation-only. Repository formatting tools never enter the published Rust graph or change the
MSRV.

Run `cargo deny check` and the complete repository gate after dependency changes. Local link
checks are deterministic and offline; external URL checks run separately on a schedule or by
manual request.
