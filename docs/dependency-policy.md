# Dependency and license policy

Dependencies are design decisions, not incidental implementation details. Prefer the standard library and focused crates with active maintenance, a clear security history, and an API that fits the owning crate's boundary.

## Baseline rules

- Use an exact, reviewable Cargo version requirement; wildcard requirements are denied.
- Use crates.io releases by default. Unapproved registries and Git dependencies are denied.
- Keep default features only when they are understood and useful.
- Avoid overlapping crates that solve the same problem without a documented reason.
- Record dependencies that constrain architecture, parsing, data representation, or public APIs in an ADR.
- Commit `Cargo.lock` and use locked dependency resolution in CI.

## License allowlist

`deny.toml` is the machine-readable source of truth. The initial allowlist is deliberately narrow: Apache-2.0, Apache-2.0 with LLVM exception, BSD-2-Clause, BSD-3-Clause, ISC, MIT, MPL-2.0, Unicode-3.0, Unicode-DFS-2016, and Zlib.

Adding a license is a compatibility and distribution decision. Review its obligations before changing the allowlist. This policy records project intent and is not legal advice.

## Exceptions

Do not silence an advisory, allow a Git source, clarify a license, or skip a duplicate merely to make CI pass. An exception must be narrowly versioned, include a reason in `deny.toml`, and be explained in the change that introduces it. Use an ADR when the exception has lasting architectural or distribution consequences.

## Automation

Run `cargo deny check` after installing `cargo-deny`. CI checks advisories, licenses, bans, and sources. Renovate proposes Cargo, lockfile, Rust toolchain, and GitHub Actions updates; updates still require the same tests and review as human-authored dependency changes.
