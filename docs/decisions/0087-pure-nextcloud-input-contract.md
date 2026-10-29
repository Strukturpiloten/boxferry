# ADR 0087: Pure Nextcloud input-request consistency contract

- Status: accepted for inert test-only request validation; runtime activation,
  full capture integration and native/application admission remain deferred
- Date: 2026-10-29
- Supersedes only: the ownership clause of
  [ADR 0083](0083-bounded-docker-target-consumer-preparation.md) insofar as it
  defers the pure #366 Nextcloud input-request checker and its offline tests
- Preserves: all other ADR 0083 module deferrals,
  [ADR 0078](0078-trusted-docker-consumer-validation.md) exact-pair admission,
  [ADR 0079](0079-native-test-runtime-selection.md) runtime selection,
  [ADR 0085](0085-docker-cli-neutral-routes.md) product non-execution and
  [ADR 0086](0086-prepared-docker-forgejo-application-profile.md) Forgejo scope

## Context

The retained authored Nextcloud source includes a self-contained request checker
and offline controls. Recovering that consistency boundary does not require the
old application profile, capture producer or runtime dispatcher. The current
exporter still refuses the Redis password in command and healthcheck fields;
topology-only evidence cannot establish a complete executable application plan.

## Decision

1. Adopt only the pure `scripts/lib/docker-nextcloud-input-contract.py` checker
   and its independently authored tests. Callers supply held bindings; the helper
   performs no acquisition, custody verification, profile admission, artifact
   validation, file access, transport, receipts or command execution.
2. Require six exact distinct role identities, distinct excluded proxy/peer
   identities, and recreated identities disjoint from every prior source role.
   Match the explicit application name, socket, source profile and all four
   target profiles to held values. Lane, source and target modes remain separate
   caller decisions; naming never supplies a missing profile.
3. Require phase-bound resource decisions, exact public-label scopes and each
   separate environment-value, protected-environment promotion and same-host-bind
   consent. These checks authorize no secret-bearing command/healthcheck export
   and cannot override exporter refusal or loss policy.
4. Bound harness records to 64 KiB of JSON, depth 16, 4096 nodes including keys,
   256 members per collection, 64 KiB aggregate UTF-8 strings and 256-bit integers.
   Reject duplicate keys, floats, invalid Unicode and malformed input with
   value-free errors. These limits describe harness records, not Docker semantics.
5. Register the fast Python suite in the existing shared offline application
   probe runner, including its order and failure-propagation controls. Existing
   local and CI consumers continue to use that runner; no new job, dependency,
   software pin or Renovate extraction is introduced.

## Recovery provenance and ownership

The two repository-authored MPL-2.0 files were recovered from the sealed
`boxferry-issue343-reconciled-r3` manifest, retained head
`ebb1cf18548218c90fcfb5b8414f0b9fa80d73b9`, without adopting the old branch.
Their retained SHA-256 values before reconciliation are:

| Path                                              | SHA-256                                                            |
| ------------------------------------------------- | ------------------------------------------------------------------ |
| `scripts/lib/docker-nextcloud-input-contract.py`  | `1e9a72a731a1e55ddb87a71d9e104cc328cc43a2f33da135316538330d619f01` |
| `scripts/test-docker-nextcloud-input-contract.py` | `afc66ffb1561a38855e3b301d5235ca038a6c9d7c2f2e0aa13d4fe065791b1bf` |

Reconciliation adds integer bounds, role/origin/profile boundary tests, full
formatted-traceback privacy controls and inert loader checks. Synthetic profile
versions in these tests are independent values, not native compatibility claims
or software pins. This is a BoxFerry-owned harness seam; Lens products and the
website require no consumer changes or BoxFerry dependency.

## Consequences

Passing offline controls proves request consistency only. It does not prove
native observations, complete image/default semantics, full application export,
runtime acceptance, persistence, migration, deployment or release readiness.
The Redis protected-field blocker remains open. Runtime profile activation,
capture integration, dispatcher registration and native/admission changes require
separate work. The primary still owns independent review and the complete gate.
