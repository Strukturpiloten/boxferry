# ADR 0038: Bounded human diagnostic presentation

- Status: accepted
- Date: 2026-08-26
- Amends: [ADR 0018](0018-generic-cli-and-diagnostic-support-bundle.md)
- Amends: [ADR 0026](0026-typed-diagnostic-rule-catalogue.md)
- Amends: [ADR 0028](0028-primary-remediation-guidance.md)

## Context

A large runtime inventory can produce many occurrences of the same conversion decision. Printing
every resource and field in the default terminal output hides the distinct actions a user can take.
Removing occurrences from structured reports would instead hide fidelity evidence.

Podman unknown-field diagnostics also need to distinguish bounded path descriptors from native
values and explain when the acquisition safety limit deliberately stopped collecting descriptors.

## Decision

1. The default human console groups occurrences that share a rule, native source rule, summary,
   reason, decision, required policy, available promotion, and observation origin.
2. Each group reports affected counts and bounded subject, resource, location, and native JSONPath
   samples. The output names omitted sample counts and points to `--verbose`.
3. `--verbose` prints every diagnostic occurrence. JSON console output, report files, and support
   bundles retain the complete diagnostics without presentation grouping.
4. Podman native-field diagnostics report retention limits, retained descriptor counts, minimum
   discarded counts, and bounded path samples, and state that native values are never retained.
5. Redaction distinguishes semantic resource and image subjects from filesystem paths. Protected
   values and actual paths remain redacted.

## Consequences

Human output remains bounded by the number of actionable causes rather than the application size.
Machine consumers retain full evidence for fidelity accounting. A descriptor discarded at the
acquisition limit cannot be named later, so the diagnostic must state that limitation instead of
inventing precision.

## Alternatives considered

Dropping repeated diagnostics was rejected because it changes fidelity evidence. Printing only a
total count was rejected because users still need bounded examples. Treating all strings containing
`/` as filesystem paths was rejected because it destroys safe Podman image and resource identities.
