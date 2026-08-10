# Repository guidance for coding agents

This file applies to the entire BoxFerry repository.

## Read before changing code

Read these documents in order:

1. `README.md`
2. `docs/implementation-plan.md`
3. `docs/architecture.md`
4. `docs/project-structure.md`
5. `docs/library-api.md`
6. `docs/api-stability.md`
7. `docs/conversion-model.md`
8. `docs/testing.md`
9. `docs/dependency-policy.md`
10. `docs/decisions/README.md` and all accepted ADRs

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
- Keep the `boxferry` facade usable as a library; the CLI must call the same public orchestration
  APIs available to external consumers.
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
cargo ci-core
cargo ci-compose
cargo ci-quadlet
cargo ci-podman
cargo ci-runtime
cargo ci-policy
cargo ci-clippy
cargo ci-test
cargo ci-doctest
RUSTDOCFLAGS="-D warnings" cargo ci-doc
cargo +1.85.0 ci-check
cargo +1.85.0 ci-policy
cargo deny check
```

The `ci-*` aliases in `.cargo/config.toml` use locked resolution and all workspace features and targets where applicable. Do not weaken an alias or CI lint to accommodate new code; address the finding or document a narrowly justified policy change.

## Multi-agent coordination

- The human-facing batch prompt and operator responsibilities are documented in
  `docs/agent-workflow.md`.
- Use the primary BoxFerry agent as the integrator for tasks spanning BoxFerry, ComposeLens, and
  QuadletLens.
- Delegate only concrete, bounded tasks with an independently verifiable result.
- Never run two source-writing agents in the same repository checkout concurrently.
- Agents may write concurrently in separate repository checkouts only after the public contract is
  defined by the primary agent.
- For a large BoxFerry key batch, prefer this pipeline:
  1. specification researchers establish the native behavior and evidence boundaries;
  2. one foundation writer adds the complete neutral-model contract and model tests;
  3. after that foundation is available in isolated checkouts, one Compose-adapter writer and one
     Quadlet-adapter writer may work concurrently on the same batch; and
  4. the primary agent integrates both uncommitted diffs, adds or delegates facade coverage, and
     reviews the combined change before verification.
- Batch related keys instead of assigning one writer per key. Keep each writer's crate and file
  ownership explicit; shared manifests, lockfiles, facade wiring, coverage documents, and release
  preparation remain owned by the primary agent unless assigned as a separate sequential task.
- The primary agent creates and removes isolated checkouts. Workers neither create branches nor
  assume that changes in another checkout are visible.
- Run at most two BoxFerry source-writing workers concurrently, leaving capacity for independent
  research or review. Run one repository verifier only after the integrated BoxFerry diff is final.
- Specification research and review agents remain read-only.
- Run a verifier only after the relevant repository's writing agent has finished.
- Verification agents report failures but do not modify source, tests, configuration, or
  documentation.
- The primary agent reviews every diff and owns architectural and cross-repository API decisions.
- Subagents never commit, push, publish crates, create tags, or create releases.
- Prefer subagents for specification research, focused implementation, review, test execution, and
  log analysis when those tasks would otherwise pollute the primary thread or can proceed
  independently.
