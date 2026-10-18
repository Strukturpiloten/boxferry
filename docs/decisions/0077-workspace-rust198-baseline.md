# ADR 0077: Aligned workspace Rust baseline and fixed ZIP generation

- Status: accepted
- Date: 2026-10-18
- Amends: the Rust baseline and ZIP dependency in [ADR 0018](0018-generic-cli-and-diagnostic-support-bundle.md), decision 17

## Context

The normal compiler already selects Rust 1.98.1, while the public minimum remained 1.85.0.
ZIP extraction advisories RUSTSEC-2026-0367, RUSTSEC-2026-0368, and RUSTSEC-2026-0369 are
fixed in ZIP 9.x, whose compiler floor is 1.88. BoxFerry only creates diagnostic ZIP bundles,
but that does not waive dependency auditing. The maintainer approved Rust 1.98 consistently
for BoxFerry and every Lens repository.

## Decision

Set the public minimum to Rust 1.98.1, matching the existing normal compiler patch. Cargo owns
the minimum, Clippy agrees, and local/PR/main/release checks continue deriving their compiler
from Cargo metadata. Routine normal-toolchain updates must not implicitly raise the minimum.
Coordinate this baseline with ComposeLens, PodmanLens, QuadletLens, DockerLens and KubernetesLens.

Use ZIP 9.0.3 with the existing explicit compression feature and no defaults. Its registry publication
on 2026-10-13 satisfies the unchanged 72-hour dependency-age policy; newer 9.0.4/9.0.5 do not yet.
Keep ZIP CLI-only,
writer-only in product code, and retain archive bounds, redaction and no-clobber publication.
Remove the obsolete ZIP major ceiling and Rust-1.85-only yoke-derive constraint/exclusion. Preserve
unrelated lock resolutions and all advisory, license, source, dependency-age and full-validation gates.

## Consequences

Consumers need Rust 1.98.1 or newer. This support-floor change requires a minor release and clear
release notes, but merging the implementation neither publishes packages nor authorizes release.
Historical ADR statements and recorded evidence remain historical; this decision changes no native
runtime baseline and establishes no new native or application conformance.

Retaining 1.85 by adding audit exceptions was not selected. BoxFerry and the independently published
Lens libraries keep their dependency direction; aligned infrastructure creates no Lens dependency
on BoxFerry.
