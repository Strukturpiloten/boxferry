# Diagnostic rules

Every BoxFerry condition has a stable code, short name, severity, explanation, and remediation.
Repeated findings share one code.

<!-- boxferry-example: rules-list -->

```console
boxferry rules
```

Explain a code or name:

<!-- boxferry-example: rule-explain -->

```console
boxferry explain BFQ0014
```

## Namespaces

| Prefix | Owner                                      |
| ------ | ------------------------------------------ |
| `BFC`  | Compose input and mapping                  |
| `BFQ`  | Quadlet input and mapping                  |
| `BFO`  | BoxFerry orchestration, files, and reports |

The pages below are generated from the same catalogue used by the CLI. Native ComposeLens and
QuadletLens codes remain source provenance; BoxFerry rule codes are the public remediation
contract.

<!-- boxferry-generated-rule-index -->
