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
| `BFP`  | Podman input, mapping, and output planning |
| `BFO`  | BoxFerry orchestration, files, and reports |

The pages below are generated from the same catalogue used by the CLI. Native ComposeLens,
PodmanLens, and QuadletLens codes remain source provenance; BoxFerry rule codes are the public
remediation contract.

Podman mapping findings name the neutral-model `subject`, the `reason`, and the conversion
`decision`. Non-exact findings also state the minimum `required_loss_policy`; promotion findings
name the available CLI flag or `none`. Native `PLN` source rules, safe resource identities, native
paths, observation origin/state, and source versions appear when PodmanLens supplied them. Values,
credentials, secret payloads, label values, and raw unknown JSON remain excluded.

Repeated acquisition findings for the same native rule, resource, kind, and observation state are
one diagnostic. `occurrence_count` preserves the total; `native_path_count` and at most eight sorted
`native_path_samples` show where the condition occurred without flooding the console or report.

<!-- boxferry-generated-rule-index -->
