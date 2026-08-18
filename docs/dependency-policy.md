# Dependency and license policy

Dependencies are design decisions, not incidental implementation details. Prefer the standard library and focused crates with active maintenance, a clear security history, and an API that fits the owning crate's boundary.

## Baseline rules

- Use explicit, compatible Cargo version requirements by default; wildcard requirements are denied.
- Use an exact pin only for a documented compatibility or representation reason.
- Use crates.io releases by default. Unapproved registries and Git dependencies are denied.
- Keep default features only when they are understood and useful.
- Avoid overlapping crates that solve the same problem without a documented reason.
- Record dependencies that constrain architecture, parsing, data representation, or public APIs in an ADR.
- Commit `Cargo.lock` and use locked dependency resolution in CI.

## License allowlist

`deny.toml` is the machine-readable source of truth. The current allowlist is deliberately narrow:
Apache-2.0, MIT, MPL-2.0, and Unicode-3.0. Add another license only when an actual reviewed
dependency requires it; unused allowances are removed so the audit remains intentional and
warning-free.

Adding a license is a compatibility and distribution decision. Review its obligations before changing the allowlist. This policy records project intent and is not legal advice.

## Exceptions

Do not silence an advisory, allow a Git source, clarify a license, or skip a duplicate merely to make CI pass. An exception must be narrowly versioned, include a reason in `deny.toml`, and be explained in the change that introduces it. Use an ADR when the exception has lasting architectural or distribution consequences.

## Reviewed direct dependencies

The `boxferry` facade uses optional Clap 4.6 for the installable command's typed subcommands,
value enums, version/help output, and consistent argument errors. The dependency is enabled only
by the `cli` feature; embedded users can disable default features. Cargo's compatible version
requirement and committed lock file let Renovate propose normal updates while CI verifies the exact
resolved graph. [ADR 0004](decisions/0004-first-cli-feature-and-write-safety.md) records the default
feature and command-safety decision.

The `boxferry` CLI feature uses ZIP 6.0.0 with default features disabled to create the local
diagnostic support bundle recorded in ADR 0018 and ADR 0021. It writes only fixed, stored entries in bounded
memory and enables no compression, encryption, timestamp, or runtime-inspection feature. ZIP is
MIT-licensed and supports Rust 1.83.0, below BoxFerry's 1.85.0 MSRV. The lockfile necessarily
records ZIP's mandatory crates (`arbitrary`, `crc32fast`, `indexmap`, and `memchr`), while a
no-default embedded build does not activate ZIP or those CLI-only dependencies.

The same CLI feature uses exactly pinned `jiff` 0.2.24 with default features disabled and only its
`std` and `tz-system` features. It supplies the fallible local wall clock required for automatic
error-report names and is unavailable to no-default embedded consumers. The exact pin documents
the reviewed Rust 1.85-compatible local-time API; its MIT licensing is allowed by the workspace
policy. For filename selection only, its system-time-zone support may inspect the standard `TZ`
setting (including a TZif path) and operating-system time-zone configuration. BoxFerry never
persists or reports a time-zone setting, name, path, or value. ADR 0024 supersedes the temporary
native-Windows dependency decision in ADR 0023.

## Automation

Release preparation uses release-plz Action 0.5.131 at immutable commit
`2eb1d8bcb770b4c48ccfaad919734b38b51958c9`, release-plz CLI 0.3.160, and SHA-pinned GitHub App
token Action 3.2.0. These are repository-only workflow dependencies. The configuration disables
publication, tags, and GitHub releases; the existing protected workflow retains those
responsibilities. All five published crates remain in one version group without package-level
commit filtering so release-plz can reconcile inherited workspace versions and internal Cargo
requirements atomically. A narrow default-branch path filter starts preparation for Rust source,
package, dependency, build, and release-configuration changes while excluding ordinary
documentation-only pushes. Renovate's GitHub Actions manager tracks the Action SHA and release
comment, and a custom workflow-tool manager tracks the release-plz CLI input. Review the Action,
CLI, least-privilege App token inputs, path filter, and preparation-only policy together.

Run `cargo deny check` after installing `cargo-deny`. CI checks advisories, licenses, bans, and
sources. Renovate proposes Cargo, npm development-tool, lockfile, Rust toolchain, Dev Container,
GitHub Actions, directly pinned workflow-tool, checksum-pinned file-tool, base-image, GitHub CLI,
and documented Dev Container CLI updates. Updates still require the same tests and review as human-authored
dependency changes.

Repository-only file quality uses pinned development tools outside the published Rust dependency
graph: markdownlint-cli2 plus Prettier for Markdown, Prettier for JSON and YAML, Tombi for TOML, shfmt and
ShellCheck for shell, and Hadolint for Dockerfiles. The Dev Container provides them, CI and release
validation run the same `scripts/check-files.sh --check` boundary, and Renovate tracks their pins.
`package-lock.json` fixes the complete markdownlint-cli2 and Prettier graph; CI and the Dev Container
install it with `npm ci --ignore-scripts`. The native Linux tools are release-asset and
SHA-256-pinned by `scripts/install-file-tools.sh`. These tools do not enter any crate package or
affect the library MSRV.

Lychee validates local documentation links without network access in local and pull-request gates.
External URL health is isolated in a weekly/manual workflow that caches only successful responses
and rate-limits requests per host. This keeps link evidence visible without turning external
availability into a deterministic build dependency or repeatedly loading third-party services.
