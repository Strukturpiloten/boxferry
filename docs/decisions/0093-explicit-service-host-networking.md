# ADR 0093: Explicit service host networking

- Status: accepted
- Date: 2026-11-02
- Builds on: [ADR 0011](0011-neutral-service-group-relationships.md) and
  [ADR 0041](0041-portable-intent-and-retained-native-evidence.md)

## Context

Compose `network_mode: host` selects a network namespace rather than a named network. Treating
`host` as a service network attachment would invent resource ownership and conflate this intent
with existing service-group runtime settings. Omitting it during conversion would instead change
the service's connectivity and port exposure.

## Decision

The neutral service gains an optional provenance-bearing `NetworkMode` with a reviewed `Host`
variant. Absence retains ordinary target networking. This field does not imply service grouping,
change the structural service-group contract, or alter the existing service-group runtime network
representation.

Compose import/export maps `network_mode: host` and Quadlet container import/export maps
`Network=host` exactly through the neutral field. Native parsing and generation remain Lens-owned.
Quadlet output checks the existing `quadlet.container.network` capability across the complete
selected target range. Repeated Quadlet host declarations are invalid because they generate multiple
native non-bridge network selections. The invalid outcome retains every source origin; neither
exact nor partial policy silently collapses those declarations into one valid host selection.
Other Compose network modes and special Quadlet modes remain explicit unsupported outcomes.

Host networking conflicts with named-network attachments and published ports. Imports retain the
mapped declarations and report invalid outcomes; exporters validate applications constructed by
embedded callers as well. Host-network services cannot be moved into a generated pod by either
grouping policy. Quadlet container input combining host networking with pod membership is invalid.
Exposed ports are distinct from host publications and remain representable.

The Podman exporter reports the new field as unsupported with its origins until a native deployment
mapping is reviewed. Partial loss policy can authorize that omission explicitly; exact policy cannot.

## Consequences

Compose-to-Quadlet, Quadlet-to-Compose, and both same-format routes preserve host networking without
creating a `host.network` resource. Conflicts fail under every loss policy, while unknown modes
remain diagnosable. The facade reexports the neutral enum for embedded callers. Machine-readable
capability evidence describes the supported routes and grouping boundary without extending native
runtime qualification.

## Alternatives considered

A named `host` network attachment was rejected because namespace selection has no named-resource
lifecycle. Inferring group-level host networking was rejected because it would change grouping and
runtime contracts. Raw mode strings were rejected because unreviewed namespace and option syntax
must not become portable intent implicitly.
