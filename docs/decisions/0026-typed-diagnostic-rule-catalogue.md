# ADR 0026: Typed diagnostic rule catalogue and causal presentation

- Status: accepted
- Date: 2026-08-12
- Amends: [ADR 0018](0018-generic-cli-and-diagnostic-support-bundle.md)
- Amends: [ADR 0020](0020-rolling-compose-specification-cli-target.md)
- Amends: [ADR 0025](0025-empty-output-directories-and-final-human-status.md)

## Context

BoxFerry diagnostics used stable-looking strings, but their names, explanations, remediation, and
ownership were distributed across adapters and CLI presentation. Some codes also represented both
an approximation and an invalid condition. Native ComposeLens diagnostics could appear as the
primary code while QuadletLens diagnostics used another presentation path. Detached terminal hints
made it difficult to determine which rule they addressed, and a final failure line named only the
stage rather than the causal rule.

## Decision

1. `boxferry-engine` owns one typed catalogue containing every BoxFerry rule's code, stable name,
   default severity, subsystem, value-free explanation, and remediation text. Official adapters
   construct diagnostic codes from this catalogue. Public `RuleId` variants use explicit stable
   discriminants, so inserting or reordering catalogue entries cannot renumber existing variants.
2. Prefixes identify ownership: `BFC` Compose, `BFQ` Quadlet, `BFD` Docker, `BFP` Podman, `BFR`
   runtime reconstruction, and `BFO` orchestration, files, and reports. A code is never reassigned
   to another condition.
3. Distinct actionable or severity-changing conditions receive distinct codes. Repeated instances
   of one condition share a code and are occurrences, not new rules. Incompatible grouping,
   invalid dependency graphs, and deprecated capabilities therefore no longer overload their
   approximation, unsupported, or unavailable counterparts.
4. Native Lens identifiers remain `source_code` provenance. Reports always use a BoxFerry rule as
   the primary `code`; native codes are not discarded or presented as BoxFerry-owned rules.
5. Human diagnostics are sorted by BoxFerry code and then source input and byte position. Equal
   rules are grouped and finding-counted. Their shared explanation and fields print once, followed
   by a numbered list containing only varying evidence. Native Lens identifiers remain report
   provenance rather than normal terminal output. Remediation and `boxferry explain CODE` stay with
   the group.
6. `boxferry rules` lists the build's catalogue, and `boxferry explain CODE_OR_NAME` returns one
   rule. Both support JSON presentation. The catalogue is the generated reference; documentation
   does not duplicate a hand-maintained complete table.
7. Version-one reports add rule name, optional native source code, rule help, primary diagnostic
   code, and a causal failure summary. The final human blocked or failure line names that causal
   rule and explanation.
8. This is an intentional pre-1.0 redesign. Removed or split codes receive no compatibility aliases.

## Consequences

Users can identify, group, sort, and explain diagnostics without guessing which hint belongs to
which finding. Scripts use BoxFerry codes while retaining native provenance for deeper format
debugging. Adding a rule requires a catalogue entry and uniqueness tests, and report schema
additions remain subject to the existing privacy and size limits.

## Alternatives considered

Keeping free-form codes beside each emitter was rejected because metadata and presentation would
continue to drift. Giving every occurrence a new code was rejected because a code identifies a
condition, not one source location. Presenting native Lens codes as BoxFerry rules was rejected
because ownership and stability guarantees differ. Retaining aliases for the unreleased codes was
rejected because it would permanently preserve overloaded meanings before the 0.2 contract ships.
