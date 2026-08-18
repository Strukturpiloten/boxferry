# ADR 0031: Quadlet semantic native reporting without speculative systemd context

- Status: accepted
- Date: 2026-08-17
- Amends: [ADR 0017](0017-n-to-n-adapter-matrix.md) and [ADR 0027](0027-format-neutral-native-findings.md)

## Context

Some future Quadlet capabilities may depend on the destination systemd release, while the
development host is not a trustworthy target. No currently emitted BoxFerry capability uses that
context. QuadletLens 0.2.0 also exposes a safe semantic view of container `Environment=`
directives and typed `.kube`/`.artifact` keys that BoxFerry does not yet model.

## Decision

1. BoxFerry exposes no systemd-version selector until an emitted capability has reviewed,
   version-dependent behavior. It never probes a host; a future selector must remain explicit and
   affect planning before it is added to the public or CLI contract.
2. Quadlet import obtains one document-level `container_environment()` semantic view per container.
   Literal decoded assignments enter the protected neutral environment model with provenance.
   Resets, bare names, deferred specifiers, and unmodeled forms remain structured outcomes; Lens
   semantic diagnostics are retained as native findings without creating a duplicate generic loss.
3. Every entry in `.kube` and `.artifact` documents produces a value-free, per-key native-only
   outcome until it has a reviewed neutral mapping. This includes typed native keys, generic
   systemd sections, `[Quadlet]`, unknown sections/keys, and repeated entries. Subjects include
   document and section occurrence identity. BoxFerry does not synthesize a Kubernetes, artifact,
   raw Podman, or systemd meaning from those keys.

## Consequences

Users receive precise reporting without source values leaking. The remaining native-only keys
remain an explicit backlog rather than silent loss; a systemd selector is deferred until it can
change a supported result.

## Alternatives considered

Ambient systemd probing was rejected because it makes builds and reports machine-dependent.
Publishing a selector with no current capability effect was rejected because it suggests a fidelity
guarantee that BoxFerry cannot provide. Keeping the former one-entry Environment parser was
rejected because it discards safe systemd quoting and multi-assignment semantics. One
document-level outcome for native-only unit families was rejected because it hides which key needs
a future model decision.
