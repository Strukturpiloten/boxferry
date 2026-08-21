# ADR 0033: Universal neutral-model pipeline

- Status: accepted
- Date: 2026-08-21
- Supersedes: [ADR 0030](0030-native-compose-same-format-canonicalization.md)
- Amends: [ADR 0029](0029-nested-input-output-cli-routes.md) decision 5 and
  [ADR 0032](0032-future-native-lens-boundaries.md) decisions 1, 2, 7, and 8

## Context

BoxFerry's product contract is independent importers and exporters around one neutral application
model. The Compose-to-Compose native canonicalization exception violated that contract: it bypassed
the Compose importer, the neutral `Application`, the Compose exporter, conversion outcomes, and
normal loss-policy authorization.

The exception preserved source-native expressions and extension fields, but made same-format
behavior structurally different from every other route. It also prevented the route matrix from
proving the central N:N invariant.

PodmanLens now exists with reviewed read-only acquisition, discovery, version evidence, diagnostics,
planning, and deterministic rendering contracts. Podman integration can therefore follow the same
neutral architecture independently of Docker. BoxFerry remains a conversion and planning tool, not
an infrastructure deployment tool.

## Decision

1. Every native input crosses its native importer into the neutral `Application` model. Every
   native output is produced from that model by its native exporter.
2. Same-format conversion is not passthrough and has no native canonicalization shortcut.
   Native-only or unresolved intent follows the normal outcome, diagnostic, and loss-policy
   contract.
3. Compose-to-Compose uses `ComposeImporter -> Application -> ComposeExporter`. The public
   `ComposeSource::canonicalize`, `ComposeCanonicalization`, and
   `CanonicalComposeDocument` BoxFerry APIs are removed.
4. Each importer fixture must be exercised against every registered exporter. Re-importable outputs
   also receive chained conversion, fixed-point, semantic-equivalence, and path-consistency tests.
5. BoxFerry may perform explicit read-only acquisition for a native importer. It never applies
   generated artifacts, invokes generated commands, deploys infrastructure, or sends mutating
   runtime API requests.
6. Podman integration is Phase 2. It reuses the `boxferry-podman` crate name without legacy API or
   behavior and adds one Podman importer and one Podman exporter through PodmanLens.
7. Phase 2 expands the current two-by-two Compose and Quadlet matrix to the full three-by-three
   Compose, Quadlet, and Podman matrix. Podman output defaults to the newest version reviewed by the
   linked PodmanLens release, initially 6.1, with an explicit maximum-version override.
8. Docker integration remains deferred and does not block Podman integration.

## Consequences

- All route behavior can be reasoned about and tested through one public orchestration pipeline.
- Compose-to-Compose can now report or reject native-only fields that the neutral model does not
  represent instead of preserving them silently.
- Removing the published canonicalization API is an intentional pre-1.0 public break.
- The scenario corpus grows by importer and exporter dimensions instead of pair-specific
  implementations.
- Podman import and export can proceed without adding Docker placeholders or execution behavior.

## Alternatives considered

### Keep the Compose native shortcut

Rejected because preserving more source syntax does not justify bypassing the product's neutral
model, fidelity policy, and N:N test contract.

### Put native Compose expressions in every neutral field

Rejected because one source language would leak into a format-independent model and complicate
every exporter.

### Wait for Docker before integrating Podman

Rejected because PodmanLens has an independent reviewed contract and Docker is not required for any
Podman mapping boundary.

### Add an apply or deploy command

Rejected because BoxFerry produces reviewable inert artifacts. Infrastructure mutation belongs to
an external system with its own authorization and rollback contract.
