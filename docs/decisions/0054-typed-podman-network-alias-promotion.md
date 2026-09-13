# ADR 0054: Promote typed Podman network aliases explicitly

- Status: accepted
- Date: 2026-09-13
- Amends: [ADR 0034](0034-podman-lens-adapter-boundary.md) and
  [ADR 0042](0042-bounded-podman-creation-evidence-and-intent-promotion.md)

## Context

Podman inspect reports aliases together with effective network attachments. Some aliases express
required application DNS names, while Podman also injects full and shortened container IDs. Raw
alias strings therefore cannot all be treated as authored portable intent. PodmanLens 0.2.4 adds a
typed, bounded classification that separates effective alias candidates from runtime container-ID
aliases without exposing raw native parsing to BoxFerry.

## Decision

1. `boxferry-podman` consumes the PodmanLens 0.2.4 typed standalone-container network-attachment
   observations. Pod-member networking remains governed by the existing pod-membership boundary.
   `NativeNetworkingObservation::networks()` remains the topology authority. Attachment evidence is
   correlated to that topology by native network reference, never incidental collection order.
2. Only `EffectiveCandidate` aliases may enter a neutral network attachment. Full and shortened
   current-container ID aliases remain runtime evidence and are never promoted.
3. Alias promotion requires both `--promote-podman-effective-named-networks`, which authorizes the
   network attachment, and `--promote-podman-portable-effective-settings`, which authorizes the
   effective alias spelling. Without both flags, BoxFerry emits an actionable, value-free outcome.
4. Promoted aliases are plain portable intent so Compose, applicable Quadlet units, and Podman
   deployment plans can preserve them. Compose output uses ComposeLens's generated-string
   sensitivity boundary so protected native aliases remain redacted across reimports instead of
   being declassified or discarded. Diagnostics never include alias values.
5. Missing, unavailable, or malformed alias evidence does not remove topology obtained from
   `networks()`. Inconsistent attachment references are invalid, and future alias classifications
   are unsupported until reviewed rather than silently discarded.

## Consequences

- Application-required DNS aliases can survive a Podman import and every exporter.
- Runtime identity aliases cannot accidentally become desired configuration.
- A malformed optional alias observation remains diagnosable without losing the named-network
  attachment.
- PodmanLens remains the sole owner of native alias decoding and classification.
