# How conversion works

BoxFerry uses one neutral application model between every supported input and output adapter. This
keeps conversion rules independent of a specific source-target pair.

```text
Compose ──┐                         ┌── Compose
Podman ───┼── application model ────┼── Podman
Quadlet ──┘                         └── Quadlet
```

The current CLI exposes all nine routes. Docker and Kubernetes remain deferred.

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
- `BFP` — Podman input, Podman mapping, and Podman output planning

Use `boxferry explain CODE` or open the rule's reference page.

## Same-format routes

Every same-format route uses its importer, the neutral model, and its exporter. Native-only intent
is reported and governed by the same loss policy as a cross-format conversion. Unresolved typed
expressions must be resolved before they can enter the neutral model.

Podman-to-Podman is not a passthrough: an explicitly acquired runtime inventory is imported into the
neutral model and exported as a desired deployment plan. The resulting `podman.json` is not an
inventory snapshot and cannot be imported again. Compose and Quadlet output remain re-importable.

## Runtime safety

Podman input is read-only and requires an explicit Unix socket and selectors. BoxFerry does not
discover ambient connections or invoke the `podman` command. It never infers target versions or
rootful/rootless context from the development machine.

All output is inert. BoxFerry never applies generated output, executes `review.sh`, deploys
infrastructure, or sends mutating runtime API requests.
