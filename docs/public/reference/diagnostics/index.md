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

Default human output groups diagnostics that have the same rule, reason, decision, required policy,
promotion, and observation origin. It prints affected counts plus at most three subjects and eight
native JSONPath samples. `--verbose` expands every occurrence. JSON output, report files, and support
bundles retain every original diagnostic.

PodmanLens retains at most 128 unmapped field-path descriptors per resource and 2,048 per inventory.
It never retains their native values. `PLN0023` reports retained descriptors that lack typed portable
mappings. `PLN0021` reports that a limit was reached, states the retained count, and gives a minimum
discarded count; the exact discarded paths are unavailable because the safety boundary deliberately
stops collecting them.

`PLN0021` truncates only the diagnostic path catalogue; typed observations used by conversion
remain intact.

<!-- boxferry-generated-rule-index -->
