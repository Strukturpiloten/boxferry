# BoxFerry cross-crate tests

This root directory organizes end-to-end scenarios shared by multiple workspace crates. Because the workspace root is a virtual Cargo manifest, executable integration-test entry points live in the owning crate rather than directly here.

Suites are introduced with the behavior they verify:

- `model` — format-independent application-model invariants
- `adapter-contract` — native/application mappings and capability boundaries
- `conversion` — golden cross-format scenarios and diagnostics
- `roundtrip` — deterministic and loss-aware model cycles
- `differential` — behavior from exact external tool versions
- `real-world` — licensed external projects and regression cases

The executable repository and fixture-contract checks live in [`crates/boxferry/tests/`](../crates/boxferry/tests/README.md). Do not add empty directories or test binaries merely to reserve suite names; add them with meaningful assertions and fixtures.

## Fast offline CLI feedback

Run `./scripts/check-cli-usability.sh` or the VS Code task **BoxFerry: Fast offline CLI usability**.
The command runs `cli_usability` with two Cargo build jobs by default and one test thread. It
checks all 18 Compose, Podman and Quadlet `validate`/`convert` route forms, their JSON reports,
artifact sets and reviewed image intent. A separate reviewed core scenario checks representative
port, environment, mount, identity and restart semantics across the outputs, plus selected
diagnostics for current losses.
The suite also checks input rejection and output no-clobber. Its Podman source uses a checked-in
read-only cassette. The task needs no running Podman, container stack or
coverage run. The same integration test is included in `cargo ci-test` for CI and release
validation. This focused task does not establish native runtime conformance or replace the
complete pre-PR and release gates.
