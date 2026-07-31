# ADR 0001: Project boundaries and from-scratch origin

- Status: accepted
- Date: 2026-07-31

## Context

BoxFerry was conceived after experience with existing container-definition converters, including Podlet. Its intended scope is broader than a Compose-to-Quadlet command: it includes multiple native formats, runtime inspection, explicit compatibility planning, and structured loss reporting.

Starting with an existing converter would also start with that converter's domain model, coupling, public behavior, and historical constraints. The project has separate repositories for reusable Compose and Quadlet libraries.

## Decision

BoxFerry, ComposeLens, and QuadletLens are independent repositories and independent implementations written from scratch.

BoxFerry depends on the Lens libraries through their public APIs. The Lens libraries do not depend on BoxFerry. Kubernetes and runtime integrations remain BoxFerry adapters unless an independently useful library boundary is demonstrated later.

Source code will not be copied or mechanically translated from Podlet, `compose_spec_rs`, or other converters. External implementations may be used for documented behavioral research, compatibility comparison, and differential testing.

## Consequences

- The architecture can be designed around loss-aware conversion from the beginning.
- Edge cases must be rediscovered through specifications, documentation, user reports, and systematic tests rather than inherited accidentally.
- Initial implementation takes longer than changing a narrow existing path.
- Provenance for external fixtures and behavior oracles must be recorded.
- Compatibility claims must be supported by project-owned tests.

## Alternatives considered

### Fork Podlet

Rejected because BoxFerry would need to replace the Compose model, split native libraries, introduce an application model, and reorganize the CLI and adapters. Long-term upstream merges would not remain practical.

### Selectively port Podlet implementation code

Rejected in favor of a clear from-scratch rule. Behavioral comparison and independently created regression fixtures remain allowed.
