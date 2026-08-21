# ADR 0029: Nested input/output CLI routes and same-format normalization

- Status: accepted
- Date: 2026-08-13
- Supersedes: [ADR 0019](0019-generic-cli-route-registry.md) for route selection, route scope,
  and Quadlet application naming
- Amends: [ADR 0018](0018-generic-cli-and-diagnostic-support-bundle.md) and
  [ADR 0020](0020-rolling-compose-specification-cli-target.md)
- Superseded for Compose-to-Compose execution by:
  [ADR 0030](0030-native-compose-same-format-canonicalization.md)
- Decision 5 restored for every route by:
  [ADR 0033](0033-universal-neutral-model-pipeline.md)

## Context

The flat `--input-type` and `--output-type` flags made unrelated format options appear together and
made contextual help difficult to scan. The implemented Compose and Quadlet importers and
exporters already support all four document combinations through the neutral model. Keeping only
the two cross-format routes would contradict BoxFerry's N-to-N architecture and would prevent
useful normalization and diagnostics for an unchanged format.

Quadlet document sets also have no Compose project identity. Calling their required neutral name a
project name exposes Compose terminology on a Quadlet input boundary.

## Decision

1. Document conversions use nested input and output subcommands:

   ```console
   boxferry convert <INPUT_TYPE> <OUTPUT_TYPE> [OPTIONS]
   boxferry validate <INPUT_TYPE> <OUTPUT_TYPE> [OPTIONS]
   ```

2. The document route registry contains `compose compose`, `compose quadlet`, `quadlet compose`,
   and `quadlet quadlet`. `capabilities` reports all four routes.
3. Each route leaf exposes only its input-specific, output-specific, policy, destination, and
   diagnostic options. Help groups consistently use “input” and “output.” Internal library types
   may retain source/target and importer/exporter terminology.
4. Quadlet input requires `--application-name`. Compose input retains optional `--project-name` as
   its native fallback name. Quadlet input still rejects stdin because each unit needs a filename.
5. Same-format conversion is not passthrough. Compose input is loaded, optionally interpolated,
   merged, imported into the neutral model, and exported as canonical `compose.yaml`. Quadlet input
   is parsed as a document set, imported into the neutral model, and exported as canonical
   BoxFerry Quadlet files. Every unsupported or changed intent remains governed by the normal loss
   policy and diagnostics.
6. Compose output uses the rolling Compose Specification target from ADR 0020. Quadlet output uses
   the explicit finite Podman range and grouping contract. These output contracts are identical
   whether the input format is the same or different.
7. `boxferry help` accepts a complete nested command path, such as
   `boxferry help convert compose quadlet`, and presents the same contextual contract as `--help`.
8. Remove `--input-type` and `--output-type` without aliases. This is an intentional pre-1.0 CLI
   break; the removed interface has no compatibility or deprecation path.
9. Structured reports retain the published `source_type` and `target_type` field names. This
   decision changes CLI vocabulary and routing, not the report schema.

## Consequences

- Users can discover the applicable options for one route without reading options from unrelated
  formats.
- Compose and Quadlet can each be normalized through the same public conversion engine used for
  cross-format output.
- Adding Docker, Podman, or Kubernetes document/runtime boundaries extends the same command tree
  without introducing pair-specific commands.
- Route-level black-box tests must cover help, rejected cross-route options, conversion, validation,
  loss policy, and output safety for every exposed pair.

## Alternatives considered

Keeping flat type flags was rejected because contextual help could not hide unrelated option
families cleanly. Naming subcommands `compose-input` and `quadlet-output` was rejected because the
command position already communicates the role. Same-format passthrough was rejected because it
would bypass the neutral-model contract, diagnostics, canonical output, and target validation.
