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

## Automation

Run `cargo deny check` after installing `cargo-deny`. CI checks advisories, licenses, bans, and sources. Renovate proposes Cargo, lockfile, Rust toolchain, and GitHub Actions updates; updates still require the same tests and review as human-authored dependency changes.
