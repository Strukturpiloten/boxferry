# How conversion works

BoxFerry uses one neutral application model between every supported input and output adapter. This
keeps conversion rules independent of a specific source-target pair.

```text
Compose ──┐                         ┌── Compose
          ├── application model ────┤
Quadlet ──┘                         └── Quadlet
```

The current CLI exposes all four document routes. Podman import and export are the next integration
phase through PodmanLens. Docker and Kubernetes remain deferred.

## Fidelity outcomes

| Outcome     | Meaning                                                                 |
| ----------- | ----------------------------------------------------------------------- |
| Exact       | Relevant intent is represented without a known semantic change.         |
| Approximate | Output is usable with a documented semantic difference or manual check. |
| Unsupported | Some intent has no safe target representation.                          |
| Invalid     | Input or target settings cannot be planned safely.                      |

The loss policy controls which candidate output may be written:

| Policy        | Authorizes                                                     |
| ------------- | -------------------------------------------------------------- |
| `exact`       | Exact output only.                                             |
| `approximate` | Exact and approximate output.                                  |
| `partial`     | Exact, approximate, and explicitly omitted unsupported intent. |

Invalid input always blocks output. A more permissive policy does not hide diagnostics.

## Fix first

Failed commands print every finding, then select one rule in a `fix first` section. Apply that
rule's help and rerun BoxFerry. Other findings may disappear after the primary problem is fixed.

Rule codes identify stable conditions:

- `BFC` — Compose input and Compose mapping
- `BFQ` — Quadlet input and Quadlet mapping
- `BFO` — orchestration, files, and reports

Use `boxferry explain CODE` or open the rule's reference page.

## Same-format routes

Every same-format route uses its importer, the neutral model, and its exporter. Native-only intent
is reported and governed by the same loss policy as a cross-format conversion. Unresolved typed
expressions must be resolved before they can enter the neutral model.

BoxFerry never infers target versions from the development machine. It writes inert artifacts and
never applies generated output or sends mutating runtime API requests.
