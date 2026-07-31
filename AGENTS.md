# Repository guidance for coding agents

This file applies to the entire BoxFerry repository.

## Read before changing code

Read these documents in order:

1. `README.md`
2. `docs/architecture.md`
3. `docs/project-structure.md`
4. `docs/conversion-model.md`
5. `docs/testing.md`
6. `docs/dependency-policy.md`
7. `docs/decisions/README.md` and all accepted ADRs

If a change contradicts an accepted ADR, update or supersede the ADR in the same change. Do not let code silently redefine the architecture.

## Project boundaries

- BoxFerry owns orchestration, the application model, conversion planning, adapters, diagnostics, and the CLI.
- Compose parsing and rendering belong in `compose-lens`.
- Quadlet parsing, rendering, and native version validation belong in `quadlet-lens`.
- Kubernetes schemas come from maintained upstream Kubernetes crates; BoxFerry owns only its mappings and supported-target policy.
- Helm and Kustomize input should initially be rendered by their native tools and then handled as Kubernetes resources.
- Format libraries must not depend on BoxFerry.

## Origin policy

BoxFerry is implemented from scratch. Do not copy or mechanically translate source code from Podlet, `compose_spec_rs`, or another converter. External tools may be used as documented behavioral references or differential-test oracles. Record the tool, version, command, and fixture provenance when doing so.

Using a third-party dependency is allowed when its license, maintenance state, and architectural role have been reviewed. Record consequential dependency choices in an ADR.

## Non-negotiable behavior

- Never silently discard user configuration.
- Every non-exact conversion must produce a structured outcome and an actionable diagnostic.
- Preserve source locations and source-specific data long enough to explain conversion decisions.
- Treat user input as fallible; malformed input must not panic the process.
- Keep target compatibility explicit. Do not infer a target version from the development machine.
- Keep file parsing free of runtime side effects.
- Runtime inspection and external command execution must pass through replaceable interfaces.
- Secrets must be redacted from diagnostics, snapshots, and logs by default.

## Development rules

- Put code in the target crate described by `docs/project-structure.md`; do not grow a root-level monolith.
- Keep the neutral model independent of Compose, Quadlet, Docker, Podman, and Kubernetes types.
- Add tests with every behavior change. Prefer small unit tests plus an end-to-end fixture when a mapping changes.
- Update capability data and evidence when target-version behavior changes.
- Update documentation in the same change as user-visible behavior or architectural changes.
- Avoid adding empty abstractions for hypothetical formats. New adapters need a defined input/output contract and tests.
- Pin every GitHub Action to its full commit SHA and append its exact release tag as a comment. Verify new pins upstream; Renovate must preserve and update both values.

## Canonical development commands

The workspace uses Rust 2024, supports Rust 1.85.0 and newer, and pins the normal development toolchain in `rust-toolchain.toml`.

```shell
cargo fmt --all -- --check
cargo ci-check
cargo ci-clippy
cargo ci-test
cargo ci-doctest
RUSTDOCFLAGS="-D warnings" cargo ci-doc
cargo +1.85.0 ci-check
cargo deny check
```

The `ci-*` aliases in `.cargo/config.toml` use locked resolution and all workspace features and targets where applicable. Do not weaken an alias or CI lint to accommodate new code; address the finding or document a narrowly justified policy change.
