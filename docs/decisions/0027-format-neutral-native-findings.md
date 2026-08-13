# ADR 0027: Format-neutral native finding provenance

- Status: accepted
- Date: 2026-08-13
- Amends: [ADR 0002](0002-public-library-facade.md), [ADR 0017](0017-n-to-n-adapter-matrix.md), and [ADR 0026](0026-typed-diagnostic-rule-catalogue.md)

## Context

ComposeLens and QuadletLens have intentionally different processing pipelines, but BoxFerry was
also treating their findings differently. Successful Compose processing could lose loader,
interpolation, merge, or profile findings, while successful Quadlet parsing required a CLI-only
diagnostic side channel. That would not scale to Docker, Podman, and Kubernetes adapters.

## Decision

BoxFerry owns a format-neutral `NativeFinding` envelope. It retains source format, producer and
optional version, native code, processing stage, severity, value-free summary, protected fields,
ordered labelled source ranges, notes, and optional native help. Native libraries remain
independent and keep ownership of their types and codes.

Every source boundary retains its native findings and every importer forwards them through the
public engine result. A BoxFerry `Diagnostic` may attach one native finding as provenance; the
BoxFerry rule code remains the public policy identity. Report schema v1 adds the complete bounded,
redacted envelope while retaining `source_code` for simple consumers. Normal human output stays
grouped by BoxFerry rule and does not dump the full native envelope.

Repository layouts are not made identical. Compose keeps its load/interpolate/merge/profile
pipeline; Quadlet keeps its syntax/model/document-set/capability pipeline. Future Docker, Podman,
and Kubernetes adapters use the same BoxFerry envelope at their boundaries.

## Consequences

Embedded callers and the CLI receive the same successful-source findings. Native codes, stages,
label roles, notes, and protected fields survive JSON reports without exposing native types in the
neutral model. Adding adapter provenance no longer requires another format-specific report DTO.

QuadletLens currently emits one value-free label per diagnostic, so the adapter classifies it as
primary. A future native multi-label API can add primary/secondary roles without changing the
BoxFerry envelope.

## Alternatives considered

Making both Lens repositories file-for-file symmetric was rejected because their formats require
different pipelines. Putting BoxFerry types in either Lens was rejected because it reverses the
dependency direction. Keeping CLI-only native diagnostics was rejected because embedded callers
would receive less evidence than CLI users.
