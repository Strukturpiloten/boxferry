# ADR 0030: Native Compose same-format canonicalization

- Status: superseded
- Date: 2026-08-13
- Supersedes: [ADR 0029](0029-nested-input-output-cli-routes.md) decision 5 for the
  Compose-to-Compose route only
- Superseded by: [ADR 0033](0033-universal-neutral-model-pipeline.md)

## Context

Compose interpolation expressions are valid native values even when BoxFerry cannot place their
unevaluated spelling in a typed neutral field. Sending Compose input through the neutral exporter
therefore rejected expressions such as `restart: ${POLICY:-unless-stopped}` although Compose output
can preserve them exactly. It also made same-format normalization depend on the currently mapped
neutral subset instead of ComposeLens's broader native model.

## Decision

1. Compose-to-Compose uses the public `ComposeSource::canonicalize` adapter boundary. ComposeLens
   canonically renders the loaded, optionally interpolated, merged, and profile-selected native
   project.
2. Without `--interpolate`, canonicalization never reads environment values and retains valid
   Compose expressions, operators, and defaults. With `--interpolate`, evaluation still occurs per
   input document before merge and canonicalization writes the resolved project.
3. Native loader, interpolation, merge, profile, and rendering findings remain structured. An
   error suppresses output; warnings and notes remain visible. Source-native data preserved in the
   Compose output is exact and does not become a neutral-model loss.
4. This is canonical normalization, not byte passthrough. Formatting and merged structure follow
   ComposeLens canonical rendering.
5. Cross-format routes still use importer, neutral application model, loss policy, and exporter.
   Quadlet-to-Quadlet retains its existing neutral-model route until a separately reviewed native
   canonicalization boundary is implemented.

## Consequences

Compose-to-Compose retains expressions and extension data that the neutral model cannot type. The
public adapter and CLI have the same behavior, and Compose parsing/rendering remains owned by
ComposeLens. Canonical Compose syntax may differ from syntax selected by neutral-model export.

## Alternatives considered

Adding a Compose-expression variant to every typed neutral field was rejected because it would
spread one source language through the format-independent model. Storing a complete native
document inside the neutral application was rejected because later model edits could conflict with
opaque stale content. Byte passthrough was rejected because it would skip ordered merge, profile
selection, canonical rendering, and native diagnostics.
