# Project structure

| Path                      | Owner                                              |
| ------------------------- | -------------------------------------------------- |
| `crates/boxferry-model`   | Neutral application graph and protected values     |
| `crates/boxferry-engine`  | Planning, diagnostics, loss policy, adapter traits |
| `crates/boxferry-compose` | Compose semantic mapping                           |
| `crates/boxferry-quadlet` | Quadlet semantic mapping                           |
| `crates/boxferry`         | Public facade and CLI                              |
| `fixtures/`               | Reviewed test inputs and expected results          |
| `docs/public/`            | Published BoxFerry documentation sources           |
| `docs/decisions/`         | Accepted and superseded architecture decisions     |
| `scripts/`                | Complete local validation and release helpers      |

Put native syntax behavior in the Lens repository that owns the format. Put cross-format semantics
in the corresponding BoxFerry adapter. Do not add conversion logic to the CLI or native types to
the neutral model.

New Docker, Podman, and Kubernetes adapter crates are forbidden until their independent Lens
libraries exist.
