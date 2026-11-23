# ADR 0102: Image-less candidate coverage source

- Status: proposed
- Date: 2026-11-22
- Amends: [ADR 0093](0093-exact-candidate-cargo-tools.md) and
  [ADR 0099](0099-candidate-coverage-tool-authority.md) for image-less callers

## Context

New Rust callers can use shared coverage without owning a Dev Container image.
Requiring a Dockerfile just to select coverage introduces an unrelated image contract.
The immutable producer's fallback metadata cannot establish candidate coverage authority.

## Decision

An image-less caller owns `scripts/cargo-llvm-cov.version`. Its complete contents are
one ASCII `X.Y.Z` release with each component in `0..999999`, without leading zeroes,
optionally followed by one LF or CRLF. Six decimal digits per component gives a simple,
maintainable domain: at most 20 version characters and 22 file bytes, well within the
pinned native Renovate SemVer versioner's numeric and string limits. The selector and
sole extractor accept this same domain. Comments, spaces, extra lines and other syntax
are invalid. The existing regular-file reader enforces the 22-byte bound for this file
and rejects symlinks; its default bound and archived Dockerfile syntax remain unchanged.
No Cargo metadata parser is added. Existing image callers retain their uniquely annotated
Dockerfile ARG; the version file cannot coexist with an image Dockerfile, even when the
values agree.

The same selector serves local and hosted CLI entry points. Coverage continues to
require candidate ownership, the exact Rust channel and verified actual Cargo dispatch.
The no-image default SemVer selector retains its independent pinned producer archive
fallback. Present invalid input never permits fallback. No second version literal,
Dev Container file, dependency inventory or installation path is introduced.

Dependency eligibility reads the file as a standalone crates.io tool. Adding or
changing it requires canonical identity, authoritative SHA-256 integrity and its own
72-hour publication age. Removal remains visible to whole-first-party isolation;
an unchanged version preserves its identity across supported line endings. Absent
historical files remain valid. Exact remote and local Git acquisition includes the
file even when unchanged, through the existing fixed snapshot inventory and budgets.
Operational read/snapshot errors at the existing per-candidate acquisition boundary
become the service's typed refusal, allowing its next candidate and durable state/receipt
writes to proceed. Batch-wide failures outside that acquisition boundary retain their
existing behavior.

Protected ordinary recovery may update only an existing valid version file, preserving
its mode and line ending, with a genuine version change. Addition, removal, rename,
image coexistence, formatting changes and malformed values cannot enter that path.
Cargo manifest version-only recovery retains its existing dependency-section scope.

One shared native regex manager owns only this exact path and entire file contents.
Its replacement changes only the version and preserves the supported line ending.
A package rule limited to this root file, regex manager, crate datasource and exact
`cargo-llvm-cov` package/dependency restricts offered versions to the same bounded
numeric grammar before native lookup selects a release. An unrepresentable newer
release therefore cannot displace an eligible update. Other sources remain unchanged.
Native crate normalization still precedes lookup; the trusted age/integrity reader
continues to require the exact resulting canonical registry version identity.
The ordinary common third-party age, grouping, automatic merge and approval rules
apply. Existing image extractors, holds, ordinary Cargo dependencies, planner and
Required CI contracts remain unchanged; history and other files gain no new owner.

## Consequences

Producer admission must precede caller adoption. First complete producer validation,
independent review and immutable distribution admission; then adopt that admitted
revision across the caller's manifest, presets, workflow references and implementation
inputs. Add the single version file and local/hosted strict coverage commands in the
caller change, validate extraction and its complete local gate, and retain actual
exact-candidate hosted evidence. Four image-backed callers need no source migration.

An older trusted base cannot understand this new source. Keep its original immutable
reader and evidence; do not claim it evaluated the new contract. Land producer support
first, update caller automation authority on a compatible base, and rebase/recreate the
caller adoption candidate for normal current-policy validation if historical evaluation
refuses it. Do not silently fall back or broaden historical recovery. Caller adoption
and native/image qualification are separate from producer source admission.

## Alternatives

Adding a Dockerfile solely for one tool, copying a workflow version, or selecting the
producer's coverage version would introduce unnecessary authority or conceal a missing
candidate declaration. The unshipped Cargo metadata proposal is withdrawn: a complete
one-line version file gives the selector and native extractor the same finite syntax.
