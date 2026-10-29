# ADR 0088: Offline typed core semantic comparison

- Status: accepted for a private offline comparison seam only
- Date: 2026-10-29
- Extends: [ADR 0083](0083-bounded-docker-target-consumer-preparation.md) only for
  replacement of the existing core Compose line parser with a typed comparison
- Preserves: [ADR 0078](0078-trusted-docker-consumer-validation.md) volume-only
  authorization, [ADR 0079](0079-native-test-runtime-selection.md) native guards,
  [ADR 0085](0085-docker-cli-neutral-routes.md) product non-execution and
  [ADR 0087](0087-pure-nextcloud-input-contract.md) limited Nextcloud source scope

## Context

The existing, unqualified core path checks generated Compose with a handwritten
line parser. That couples semantic checks to presentation and cannot provide a
typed complete graph check. Its native report map deliberately remains empty;
the historical report expectations are immutable evidence.

The authored one-service core fixture and independently fetched BusyBox image
configuration provide a narrow comparison contract. The image index, unique
Linux/amd64 manifest and config digests are recorded in
`core-semantic-expectations.json`. Those image facts do not establish native
inspect availability, daemon defaults or complete Docker outcome expectations.

## Decision

1. Replace the line parser with the facade-owned, test-only
   `docker-core-semantic-projection` example. It accepts bounded Compose on stdin
   and closed image/context arguments. ComposeLens owns loading/merge and native
   values; ComposeImporter owns neutral mapping. There is no new YAML/native
   decoder, live socket, environment provider, executor or deployment operation.
   Context arguments are caller claims checked against the authored envelope,
   never observed daemon facts.
2. Compare the complete native field graph with an independently authored
   expectation, preserving missing/null, scalar types, sequence order and empty
   collections. Compare the complete neutral application and the authored
   Compose-import outcome/diagnostic inventory. Additional settings, including
   restart/health/network children, remain unreviewed and refuse even when empty,
   false or zero. No export/reimport agreement supplies expected truth.
3. Emit only a bounded private fixed comparison result. Literal environment,
   document, native IDs and error payloads never enter that result or diagnostics.
   The Python boundary validates complete result keys/outcomes/diagnostics and
   executes only a digest-checked, owner-private snapshot of source-attested
   helper bytes under the existing receipt/source/lock scheme.
4. Add the canonical `core-semantic-projection` build/snapshot profile solely for
   this pure helper. It is not a Docker runtime dispatcher profile. Cross-profile
   receipts refuse; the existing core invocation requires its separate helper
   evidence. The trusted volume-only source authorization gains no file, scope
   or execution permission from this change.
5. Keep `CORE_REACQUIRE_REPORT_EXPECTATIONS` empty and historical
   `core-reacquire-expectations.json` unchanged. Public ConversionReport v1,
   Lens APIs, capability admission and dependencies remain unchanged. All four
   real core lanes and all six application migrations remain unqualified.

## Evidence and ownership

The expectation is authored from `core.compose.yaml` and public immutable image
metadata: index `bdf57e528e45e4433820e045b29b4597825a1c9e38353532d90a01445013f82e`,
Linux/amd64 manifest `66a6306db78bf2dbf3487f293aa8d6990d8e506fdffab9cc43fe422becf886e4`,
and config `30ecbe1509090b38ef3260f3609d64d8261230464eaa4d235901db496e9647fb`.
Every descriptor digest/size was verified independently of BoxFerry output.
Only configuration facts are retained; no image/layer is redistributed. Authored
expectations use MPL-2.0; upstream image/component license obligations remain.

The single operational fixture image pin remains DockerLens-owned. These frozen
digests are evidence, not additional operational pins or Renovate extraction
targets. No software version, manager, workflow or shared consumer changes.
Python source contracts remain in the existing offline application runner;
the Rust example's unit tests run with the facade's existing all-target checks.

## Consequences

Offline agreement proves only the narrow typed comparison. Current native
daemon/default/serialization reviews, full outcome ledgers, exact-pair admission,
four-lane apply/reacquire behavior and cleanup remain separate required work.
Compatible published SDK/API/package evidence and the primary complete gates
remain necessary. No native, application, release or publication claim follows.
