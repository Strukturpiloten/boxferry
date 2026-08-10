# ADR 0003: Explicit runtime-observation provenance

- Status: accepted
- Date: 2026-08-03

## Context

BoxFerry must reconstruct reusable definitions from deployed Docker and Podman resources. A
runtime inspection reports effective state, not the complete author intent that created it. Image
defaults, runtime defaults, API calls, generated orchestration, and later mutations can all produce
the same observed container. Podman's optional `CreateCommand` is useful supporting evidence but
is absent for resources created through some APIs and tools.

Treating every observed value as an authored source value would make diagnostics overconfident.
Treating inferred values as observations would hide BoxFerry's reconstruction decisions.

## Decision

1. Neutral-model provenance explicitly classifies authored source documents, runtime
   observations, user overrides, implementation defaults, and conversion decisions.
2. Runtime resource identities use caller-selected `SourceId` values and never require a byte
   span. Inspectors must choose stable, redacted identities suitable for diagnostics.
3. Effective inspection fields enter the model as runtime observations. Values inferred by
   comparing container and image inspection enter as conversion decisions and produce a
   non-exact outcome whenever original intent cannot be established.
4. Optional creation commands remain adapter evidence. They are never the required source of
   truth and do not override contradictory effective inspection data silently.
5. Docker and Podman command/API access stays in their separate adapters. The neutral model does
   not depend on either runtime's response types.

## Consequences

- Reports can distinguish “the runtime currently has this value” from “the original definition
  requested this value.”
- Runtime imports can reuse the existing `Sourced<T>`, conversion-outcome, redaction, and loss-policy
  contracts.
- Deterministic unit tests can construct observations without connecting to a daemon.
- Actual inspectors still need runtime-specific request interfaces, sanitized fixtures, image
  comparison logic, and uncertainty diagnostics before the runtime phase is complete.

## Alternatives considered

Using only free-form source identifiers was rejected because callers could not reliably determine
whether a value was authored, observed, defaulted, or inferred. Storing Docker or Podman inspect
objects in the neutral model was rejected because it would break format independence. Requiring
`CreateCommand` was rejected because real resources may not contain it and effective state is the
safer primary observation.
