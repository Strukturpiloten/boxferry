# ADR 0097: Read-only zero-age-delta bootstrap proof

- Status: accepted; source admission and hosted consumer recovery remain separate
- Date: 2026-11-06
- Amends: [ADR 0092](0092-shared-development-automation.md), solely for the two exact bootstrap repairs
- Preserves: [ADR 0095](0095-bounded-older-base-dependency-recovery.md)'s normal trusted-service path

## Context

The Podman PR159 and Quadlet issue174/PR175 permission repairs target bases whose
trusted eligibility service cannot produce a receipt. Their reviewed repairs
change no dependency identity. The existing exact-base reader must retain its
policy authority while a newly admitted reader computes this limited fact.

## Decision

The source-owned registry accepts only those two exact broken bases and reviewed
permission/test postimages. It compares complete bounded immutable consumer trees,
including directories, modes, blobs and all unknown dependency-bearing inputs.
A standard isolated anonymous fetch acquires only exact N at depth one into a
temporary bare object database, with no checkout, filters or promisor/lazy fetch.
Ambient netrc files/links refuse acquisition; credential helpers, tokens, redirects
and inherited Git configuration are disabled. CPU, memory, file, aggregate disk,
output and wall bounds apply. The trusted policy checkout supplies B.

Every other input stays equal; only coherent substitutions in existing operational
BoxFerry source slots may accompany the exact repair. Policy bytes stay identical;
the manifest's sole owned revision slot can change. The existing byte-preserving
source grammar is reused, with exact whole-file-hash/line anchors for its six
already-owned shared-node/shared-crate-publish inputs in the two known bases.
Unknown files/slots and residual or mixed source references refuse proof. Inert reviewed templates and
provenance live under `automation/tests/fixtures/zero-age-delta/`.

Policy A is exactly `1cf96f5a667d96e66c119753cf7814e22e0dee7e`. Proof implementation S
must equal its own immutable checkout, GitHub's referenced-workflow metadata and
the live canonical `automation-v1` branch and dereferenced commit. An ancestor is
insufficient. The complete producer A/S trees must be equal except the three exact
reviewed release-preparation issue604 postimages and the closed issue606 paths.
The waiter must match the fixed integration template; dependency modules, locks,
Actions, runners, tools and other producer inputs stay A-exact.

A fresh isolated interpreter imports only held A and verifies every loaded module's
origin and hash. It checks the complete consumer snapshots with empty unchanged
first-seen state, neutral actor context and both acquisition callbacks denied and
counted. Success requires no introduced identities or new/changed coverage gaps.
Known byte-equal legacy Feature inputs can remain opaque data; this does not qualify
their graphs or excuse changed inputs.

PR custody binds GitHub's source-head run metadata to the event's tested synthetic
merge and its exact ordered base/head parents. Queue custody accepts only the
observed single-PR squash shape: tested N has sole parent B, and its complete tree
equals current PR head H. Fixed anonymous public metadata reads prove public
visibility, exact PR identity, B ancestry and current H; they never infer H from a
queue-ref suffix or claim H was N's parent/historical queued head. At most six
anonymous reads are allowed, with no redirects, credential inheritance or retry.
The exact PR and canonical source are reread immediately before success.

Only `proved-zero` bypasses the original exact-A receipt wait. `needs-service` uses
that original path; `invalid-context` fails. Reports bind repository, event,
base/tested/source, run/attempt, A/S, complete tree digests, opaque evidence and zero
identity counts. They identify computed proof, never a trusted-service receipt.
Unknown owners/bases/authorities/variants return early to normal admission.

## Consequences

No service/status/artifact impersonation, persistent state, actor exemption,
controller, dispatch, rerun, extra permission or candidate execution is introduced.
Every changed package/transitive/Feature/tool/Action/image/policy input retains
normal 72-hour service admission. Other selected CI checks and Required CI remain
unchanged. The waiter keeps its five-minute job budget and 90-second original
receipt wait; the proof has a 65-second wall deadline. Source/hosted validation and
consumer adoption are evidence to acquire separately, not outcomes of this ADR.
