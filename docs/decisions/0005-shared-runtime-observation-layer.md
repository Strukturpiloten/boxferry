# ADR 0005: Shared runtime observation and reconstruction layer

- Status: accepted
- Date: 2026-08-04

## Context

Docker and Podman expose similar effective container, image, network, and volume information, but
their response schemas, API versioning, command behavior, and Podman pod concepts differ. Putting
native responses directly into the application model would violate format independence. Repeating
image-comparison, provenance, redaction, relationship, and uncertainty behavior in both adapters
would create drift.

Runtime access also introduces side effects and trust boundaries that a deterministic
reconstructor does not need. Tests should be able to exercise reconstruction without a daemon,
socket, executable, or production inspection output.

## Decision

1. `boxferry-runtime` owns runtime-neutral observations and pure reconstruction into the
   application model.
2. Planned `boxferry-docker` and `boxferry-podman` crates own API/command access, native response
   parsing, version negotiation, discovery, and source-side omissions.
3. Native adapters construct a duplicate-safe `RuntimeSnapshot` using caller-selected stable,
   redacted `SourceId` values. Native response types and raw JSON do not enter the shared crate.
4. Effective command, environment, and optional creation-command arguments are sensitive by
   default. A creation command may contribute provenance but cannot override effective fields.
5. Callers explicitly choose `PreserveObservedState` or `InferImageOverrides`. Image comparison
   creates conversion-decision provenance and non-exact outcomes because original author intent
   remains unknowable.
6. Inspected network and volume existence does not establish lifecycle ownership. Reconstruction
   records `ResourceOwnership::Uncertain` until a caller chooses application-owned or external
   behavior.
7. Relationships that the application model cannot yet represent remain in the observation
   snapshot and produce explicit unsupported outcomes. Pod membership follows this rule initially.
8. The facade exposes this component through an additive, non-default `runtime` feature. Future
   native runtime features enable it transitively.

## Consequences

- Docker and Podman adapters can share reconstruction behavior without sharing their native API
  dependencies or pretending their response schemas are identical.
- Pure tests cover provenance, inference, aliases, relationships, redaction, and uncertainty with
  no runtime installation.
- Native adapters must still provide sanitized exact-version fixtures and opt-in runtime
  conformance before inspection support is complete.
- `ResourceOwnership` gains an additive uncertain state. Target adapters fail closed until a
  caller resolves it.
- The observation contract may evolve before publication as both native adapters exercise it.

## Alternatives considered

Putting reconstruction in both native adapters was rejected because shared policy and diagnostics
would drift. Putting observations in `boxferry-model` was rejected because effective inspection
state is an import boundary, not application intent. Parsing Docker- or Podman-shaped JSON in the
shared crate was rejected because it would couple the common layer to native schemas and version
behavior.
