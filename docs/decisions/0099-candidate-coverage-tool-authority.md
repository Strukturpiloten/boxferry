# ADR 0099: Candidate coverage tool authority

- Status: accepted
- Date: 2026-11-08
- Amends: [ADR 0093](0093-exact-candidate-cargo-tools.md) for coverage execution

## Context

The shared coverage workflow installed a literal crate version independently of the candidate's
Dev Container declaration. Equal current values concealed two authorities: a later recipe update
could change local/image tooling while hosted coverage retained its older version.

## Decision

Coverage requires the candidate's existing, uniquely annotated `CARGO_LLVM_COV_VERSION` ARG and
exact Rust channel. The existing Cargo-tool selector reads this bounded inert data; coverage
explicitly disallows the absent-Dockerfile fallback retained for other supported callers.

The shared workflow installs through locked canonical crates.io resolution into its owned Cargo
home. It exports the verified executable path, Cargo home and compiler channel, installs LLVM for
that channel, then verifies actual Cargo dispatch from the candidate working directory before
running the unchanged trusted coverage task. Missing declarations, conflicting aliases/config,
compiler or plugin mismatches and installation failures stop execution.

The complete local gate verifies the same declaration against its actual ambient Cargo home and
working directory, and runs coverage with the verified compiler channel. It never installs a
replacement coverage binary silently. Coverage argv, ratchets, budgets, identities, task outcomes,
native qualification and release boundaries remain unchanged.

The Dockerfile ARG is the sole local/CI/image coverage version authority and keeps its existing
Renovate extractor. Remove the redundant workflow declaration and its unused extraction route.
Only this coverage ARG leaves the direct-image/tool proposal hold: automatic updates retain the
ordinary third-party age, integrity, required-check and native-image qualification contracts.
Other image, Feature and tool holds remain in place; this decision activates no updater service.

## Consequences

Coverage updates exercise the version declared by the candidate rather than a producer literal or
an unrelated installed binary. Consumers must adopt an actually admitted producer revision and
verify their own local coverage entry points; a producer source test does not complete adoption.
Cargo's canonical registry and published lockfile provide package integrity, not reproducible
binary bytes. No new version inventory, runtime, publication or deployment authority is added.

## Alternatives

Keeping two version literals and testing equality still permits independent updater drift.
Disabling the remaining ARG owner would silently freeze a formerly automatic coverage update.
A fallback to producer metadata would conceal a missing candidate declaration.
