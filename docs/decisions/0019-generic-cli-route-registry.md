# ADR 0019: Generic CLI route registry

- Status: accepted
- Date: 2026-08-11
- Amends: [ADR 0017](0017-n-to-n-adapter-matrix.md) and
  [ADR 0018](0018-generic-cli-and-diagnostic-support-bundle.md)
- Amended by: [ADR 0020](0020-rolling-compose-specification-cli-target.md)

## Context

The public conversion engine already composes independent Compose and Quadlet importers and
exporters through the neutral model. The generic CLI introduced by ADR 0018 exposed only
Compose-to-Quadlet and temporarily retained an unreleased pair-specific command. That command is
not needed, and a hard-coded pair does not express ADR 0017's N-to-N architecture.

N-to-N describes independent adapter composition, not a claim that every possible route is
implemented. The CLI needs one finite authority for its currently available routes, accepted
option families, target evidence, output layout, capability reporting, and dispatch.

## Decision

1. Remove the unreleased `compose-to-quadlet` command without a deprecation window. The generic
   `convert` and `validate` commands are the only document-conversion interface.
2. Add a typed CLI route registry. Its initial entries are Compose input to Quadlet output and
   Quadlet input to Compose output. Parsing, route validation, dispatch, `capabilities`, and report
   initialization use the same registry. Other pairs fail closed as unavailable.
3. Repeatable `--input-file` and `--input-directory` occurrences retain one global order. Compose
   directory discovery keeps the conventional single-file priority from ADR 0018. Quadlet
   directory discovery is non-recursive and contributes every supported lower-case unit file in
   lexical filename order: `.container`, `.pod`, `.network`, `.volume`, `.build`, and `.image`.
   Unsupported entries are ignored with discovery detail. Duplicate resolved paths and duplicate
   Quadlet unit basenames are errors.
4. Quadlet input does not accept stdin because a native unit filename is required. It requires an
   explicit `--project-name`, because a Quadlet document set has no project identity and ordered
   inputs may come from multiple directories. Compose interpolation, environment, profile, and
   project-directory options are rejected for Quadlet input.
5. Compose output is one deterministic, parse-back-validated document written as `compose.yaml`
   inside the required new output directory. Per ADR 0020, the generic Quadlet-to-Compose route
   targets the rolling Compose Specification and requires no provider or backend-runtime flags.
6. The route does not infer installed providers or runtimes, and reports the selected Compose
   target as `rolling`. Quadlet target selectors and grouping options are rejected for Compose
   output. Exact provider-aware targets remain available to embedded callers under ADR 0013.
7. Compose-to-Quadlet keeps the finite Podman target bounds and grouping contract from ADR 0018.
   Loss policy and the absent output-directory requirement apply to both routes. Route-specific
   options that were not explicitly supplied may have CLI defaults, but they must not affect or
   appear as choices for another route.
8. Reports derive source type, target type, requested and resolved target versions, choices,
   fidelity outcomes, and artifact metadata from the selected route. Quadlet source failures add
   `quadlet-parse` and `quadlet-document-set` to the unreleased report schema version 1 before its
   first publication. The privacy-safe stored ZIP contract remains unchanged.

## Consequences

- The CLI represents the current two-route matrix without pair-specific conversion logic.
- Adding an importer or exporter still requires an explicit registry entry and route contract;
  adapter existence alone does not silently expose a new CLI behavior.
- Quadlet-to-Compose uses the same structured loss policy as every public engine conversion and
  can report unsupported pod runtime data or approximate environment-file reconstruction.
- Target-specific flags remain discoverable on the generic command but fail closed when supplied
  to a route where they have no meaning.
- Same-format normalization and runtime deployment-plan routes remain unavailable until their
  importer, exporter, safety, and output contracts are complete.

## Alternatives considered

Keeping `compose-to-quadlet` was rejected because it was unreleased, duplicated the generic
surface, and encouraged pair-specific CLI growth. Advertising the full Cartesian product was
rejected because N-to-N is an architecture and several exporters or runtime safety contracts are
not complete. Deriving a Quadlet project name from one directory was rejected because inputs can
be interleaved across directories. Inferring a Compose provider or backend from installed tools
was rejected because compatibility evidence must remain explicit and reproducible.
