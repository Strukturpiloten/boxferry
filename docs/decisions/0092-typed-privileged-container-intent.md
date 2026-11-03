# ADR 0092: Typed privileged container intent

- Status: accepted
- Date: 2026-11-02
- Amends: [ADR 0041](0041-portable-intent-and-retained-native-evidence.md)
- Builds on: [ADR 0040](0040-quadlet-native-value-decoder-boundary.md)

## Context

Compose service `privileged` is an explicit boolean container setting. Discarding it prevents
exact conversion of workloads requiring privileged execution; replacing omission with false loses
source intent. Quadlet has no dedicated container `Privileged` key in the supported range.
QuadletLens 0.3.0 provides finite native evidence for the exact container
`PodmanArgs=--privileged` and `PodmanArgs=--privileged=false` forms.

ADR 0041 deliberately prevents arbitrary retained native arguments from becoming desired state.
A fixed argument generated from typed intent needs a narrower reviewed exception to that rule.

## Decision

`Service::privileged` stores `Option<Sourced<bool>>`. Importers preserve omission, explicit false,
explicit true and source provenance independently. Compose import/export maps the typed native
boolean. An unresolved Compose expression remains invalid rather than selecting a default.

Quadlet export derives only `--privileged` or `--privileged=false` from that boolean. It uses
QuadletLens's `quadlet.container.podman-args.privileged` capability, including its complete finite
version bounds and native evidence, without substituting broader generic argument support.
Existing security options remain independent typed settings and retain their authored values.

Quadlet import promotes only a sole container `PodmanArgs` entry whose physical value, after
trimming surrounding whitespace, is exactly one of those two forms and has no continuation.
This is finite literal recognition, not command-line parsing. Repetitions, resets, mixed arguments,
quoted/escaped forms, and other spellings remain protected opaque native evidence on every route.
The importer never combines a promoted privileged setting with unreviewed arguments that could
alter it. Exporters never forward retained raw argument text.

The Podman deployment exporter has no reviewed mapping for this setting and reports an unsupported
outcome with the setting's origins, including explicit false. Exact policy blocks that omission;
partial policy can authorize the separately supported candidate.

## Consequences

Compose → Quadlet → Compose and same-format conversions preserve supported explicit boolean
intent. The guarantee concerns configuration and generated command text; it neither grants host
privileges nor proves rootless/rootful, kernel, LSM, systemd, or workload runtime behavior.
Quadlet inputs outside the finite recognition subset remain visibly unsupported.

The native evidence is owned by the pinned QuadletLens 0.3.0 catalogue, whose privileged capability
links documentation, tagged command construction, and generator tests across Podman 5.4.0–6.1.2.
BoxFerry's public facade tests independently assert the literal output, model values, provenance,
version boundaries, round trips and loss policy. No oracle source is copied into BoxFerry.

## Alternatives considered

A generic desired-state raw argument collection would reopen the opaque execution channel rejected
by ADR 0041. Synthesizing individual security options would not preserve privileged semantics.
Inventing a native `Privileged` key would produce unsupported Quadlet documents. Parsing arbitrary
native argument syntax in BoxFerry would duplicate Lens ownership and introduce precedence hazards.
