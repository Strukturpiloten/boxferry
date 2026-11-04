# ADR 0093: Exact candidate Cargo tool selection

- Status: accepted
- Date: 2026-11-04
- Amends: [ADR 0092](0092-shared-development-automation.md)'s shared Rust tooling

## Context

A pinned Action commit does not necessarily select an immutable tool version. The admitted
SemVer Action reuses a binary from `PATH`, otherwise downloads the latest release, with an
unversioned Cargo-install fallback. Its default Rust channel also ignores the candidate's
pinned compiler. Independently updated Dev Container ARGs therefore did not establish which
CLI or compiler the hosted compatibility check executed.

## Decision

Read the candidate's existing, uniquely annotated Cargo-tool ARGs and exact Rust channel as
bounded inert data. A missing Dockerfile may use the selected immutable automation archive's
existing Cargo metadata; a present invalid or incomplete declaration cannot fall back.
Do not execute Dockerfile instructions or add another inventory of version literals.

Before the existing SemVer Action, install the declared crate version through exact, locked
canonical crates.io resolution in an isolated Cargo home. Verify the actual Cargo subcommand
selection and invoked version from the Action's workspace-root working directory. The installed
binary is first on `PATH`; a distinct Cargo-home plugin, applicable aliases, compiler overrides,
installation failure or a mismatched Cargo, rustc or plugin version stop before the Action.
Cargo command enumeration is not execution-path evidence. The Action's `rust-toolchain` input receives the
candidate's exact channel. Local complete validation verifies the same declared CLI/compiler
pair using the Cargo home of its final compatibility command. The Action and release intent
remain unchanged; no second complete gate is introduced.

The existing trusted metadata-only dependency reader also ingests standalone declarations for
the four already-owned Rust tooling crates. Each changed version needs canonical registry
identity, a valid checksum and a publication timestamp at least 72 hours old. Missing, malformed
or future timestamps fail; source-commit age never proves a tool's age. Tool updates cannot
inherit the whole-first-party-release exemption. That exemption remains unchanged for isolated
first-party releases and their necessary transitive lock resolution.

Existing crate-backed Renovate declarations retain sole version ownership. Tool and Action
pins, compiler source, candidate and trusted policy identities remain distinct. This decision
adds no product dependency, native runtime behavior or publication authority.

## Consequences

A declared tool update is tested with that version rather than an ambient or newly downloaded
latest executable. Consumers retain their current declarations, including different supported
SemVer versions. Installation relies on Cargo's canonical registry and published lockfile
integrity; it does not claim reproducible binary bytes. Caller adoption still requires each
consumer's complete checks and exact-candidate hosted evidence.

## Alternatives

The existing Action v2.9 behavior was researched at commit
[`6b69fcf40e9b5fb17adeb57e4b6ecd020649a239`](https://github.com/obi1kenobi/cargo-semver-checks-action/tree/6b69fcf40e9b5fb17adeb57e4b6ecd020649a239)
using read-only source retrieval and `cargo --version`, `rustc --version`, and
`cargo semver-checks --version` probes. Its MIT-licensed source was inspected, not
copied or redistributed in this change. Its installed-tool branch returns before the latest-release
query and Cargo-install fallback; verification failure stops the preceding workflow step.
Require an existing rustup executable as well, so the Action cannot enter its bootstrap path
and prepend a different Cargo binary directory.

A version input invented for the third-party Action would be ineffective. Checking only
`which` or Cargo's command enumeration does not prove actual invocation. Replacing the Action,
changing tool versions merely to make repositories agree, or adding a separate credential
service would expand this repair without resolving the existing authority boundary.
