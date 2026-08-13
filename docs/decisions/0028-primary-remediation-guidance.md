# ADR 0028: Primary remediation guidance

- Status: accepted
- Date: 2026-08-13
- Amends: [ADR 0026](0026-typed-diagnostic-rule-catalogue.md) and [ADR 0027](0027-format-neutral-native-findings.md)

## Context

Sorted diagnostic groups make every finding visible, but they do not tell a user or automated
consumer which condition is most useful to address first. The existing primary code and failure
summary omit the catalogue remediation. Unresolved source expressions also demonstrate why this
must remain format neutral: Compose understands the variable name, while Quadlet only knows that
the preserved target value cannot be emitted.

## Decision

1. Source adapters identify source syntax. A preserved unresolved Compose image produces `BFC0105`
   with its variable name and subject. A target adapter does not parse that syntax; Quadlet uses
   `BFQ0014` for its independent inability to emit the same subject.
2. Every blocked or failed conversion report selects one retained BoxFerry rule as `fix_first`.
   The object contains the rule code, name, value-free description, static help, and a conservative
   next step to apply the help and rerun BoxFerry. Successful reports use `null`.
3. Human output prints the same guidance after all diagnostic groups and before the conclusive
   final result. JSON console output, report files, and support bundles share the public report DTO.
4. All findings remain visible and sorted. `fix_first` is a remediation priority, not proof that
   every other finding was caused by it; the rerun message therefore says remaining findings may
   disappear or change. Explicit machine-readable causal edges remain deferred until adapters can
   provide evidence for them.
5. Guidance fields come from the typed rule catalogue and contain no input, output, environment,
   runtime, or path values.

## Consequences

Humans and automation receive the same first action without parsing terminal prose. Compose and
future source adapters can expose native variable identity without coupling Quadlet, Docker,
Podman, or Kubernetes targets to source grammars. Tests cover human placement, JSON shape, report
schema, and the paired source/target unresolved-image findings.

## Alternatives considered

Printing only the primary code was rejected because automation would still need another catalogue
lookup for remediation. Suppressing likely dependent findings was rejected because BoxFerry must
never hide configuration or diagnostics. Parsing Compose expressions in the Quadlet exporter was
rejected because it would break the N:N adapter boundary.
