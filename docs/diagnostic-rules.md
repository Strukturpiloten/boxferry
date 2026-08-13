# Diagnostic rules

BoxFerry assigns one stable code and name to each actionable diagnostic condition. The typed
catalogue in `boxferry-engine` is the source of truth; it also supplies the default severity,
explanation, and remediation used by the CLI and reports.

List or inspect the catalogue installed with a build:

```console
boxferry rules
boxferry explain BFQ0009
boxferry explain quadlet-restart-policy-approximation
boxferry rules --console-format json
boxferry explain BFQ0009 --console-format json
```

## Namespaces

| Prefix | Owner                                          |
| ------ | ---------------------------------------------- |
| `BFC`  | Compose adapter and Compose preprocessing      |
| `BFQ`  | Quadlet adapter and native Quadlet diagnostics |
| `BFD`  | Docker inspection adapter                      |
| `BFP`  | Podman inspection adapter                      |
| `BFR`  | Runtime reconstruction                         |
| `BFO`  | BoxFerry orchestration, files, and reports     |

A code identifies one condition and is not reused. Multiple source locations that trigger the same
condition are grouped as findings. A condition with different semantics or normal severity
receives another code.

ComposeLens and QuadletLens keep ownership of their native diagnostic identifiers. BoxFerry maps
them to a BoxFerry rule and retains the original identifier as `source_code` in JSON. Normal human
output uses only the BoxFerry rule and actionable finding evidence.

## Presentation and reports

Human diagnostics are sorted by BoxFerry code, then by input and source position. Each group prints
its shared explanation and any field common to every finding once. The numbered findings contain
only varying evidence such as a variable, subject, message, or location. Help stays with its group
and links to `boxferry explain CODE`. Loss authorization permits eligible output but never hides
its diagnostic.

JSON diagnostics contain `code`, `name`, `source_code`, `severity`, `summary`, `help`, fields, and
spans. Blocked and failed reports also identify `primary_diagnostic_code` and `failure_summary`.
Sensitive values and host paths remain governed by the error-report redaction contract.

## Adding or changing a rule

- Add the typed rule and metadata to the central catalogue.
- Use a new code for a new condition; do not repurpose an existing code.
- Assign a new explicit `RuleId` discriminant; never renumber an existing variant.
- Construct official adapter codes through `RuleId`, not a string literal.
- Add positive, negative, ordering, help, and redaction coverage as applicable.
- Update an ADR when the change alters ownership, compatibility, severity, or report semantics.
