# Implementation status

| Task  | Status      | Result                                                              |
| ----- | ----------- | ------------------------------------------------------------------- |
| T1–T4 | Complete    | Test foundation, native syntax kernels, neutral conversion core     |
| T5–T7 | In progress | Typed mappings and expanded conformance continue by supported field |
| T8    | Blocked     | Runtime matrix waits for DockerLens and PodmanLens                  |
| T9    | Complete    | Four Compose/Quadlet document routes and released Lens integration  |
| T10   | Complete    | Concise public documentation and executable examples                |

## Current product

BoxFerry supports Compose-to-Compose, Compose-to-Quadlet, Quadlet-to-Compose, and
Quadlet-to-Quadlet document routes. Every route has positive, negative, loss-policy, output-safety,
report, and executable documentation coverage.

Known native API gaps remain explicit diagnostics. BoxFerry does not privately implement Compose
include processing, environment-file materialization, or speculative runtime behavior.

## Next product boundary

T8 remains blocked until independent DockerLens and PodmanLens projects publish reviewed native
import, deployment-plan, version, diagnostic, and execution contracts. KubernetesLens follows the
same boundary later.

The website's Lens documentation rewrite and production deployment are tracked in the
`boxferry-website` repository.
