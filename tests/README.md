# BoxFerry cross-crate tests

This root directory organizes end-to-end scenarios and runtime harnesses shared by multiple workspace crates. Because the workspace root is a virtual Cargo manifest, executable integration-test entry points live in the owning crate rather than directly here.

Suites are introduced with the behavior they verify:

- `model` — format-independent application-model invariants
- `adapter-contract` — native/application mappings and capability boundaries
- `conversion` — golden cross-format scenarios and diagnostics
- `roundtrip` — deterministic and loss-aware model cycles
- `differential` — behavior from exact external tool versions
- `runtime` — opt-in Docker, Podman, systemd, and Kubernetes environments
- `real-world` — licensed external projects and regression cases

The executable repository and fixture-contract checks live in [`crates/boxferry/tests/`](../crates/boxferry/tests/README.md). Do not add empty directories or test binaries merely to reserve suite names; add them with meaningful assertions and fixtures.
