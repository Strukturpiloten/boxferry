# Repository guidance for coding agents

This file applies to the entire BoxFerry repository.

## Read before changing code

Read these documents in order:

1. `README.md`
2. `docs/implementation-plan.md`
3. `docs/architecture.md`
4. `docs/project-structure.md`
5. `docs/library-api.md`
6. `docs/testing.md`
7. `docs/development-environment.md`
8. `docs/dependency-policy.md`
9. `docs/decisions/README.md` and all accepted ADRs

If a change contradicts an accepted ADR, update or supersede the ADR in the same change. Do not let code silently redefine the architecture.

## Project boundaries

- BoxFerry owns orchestration, the application model, conversion planning, adapters, diagnostics, and the CLI.
- Compose parsing and rendering belong in `compose-lens`.
- Quadlet parsing, rendering, and native version validation belong in `quadlet-lens`.
- Native Docker protocol handling, version evidence, acquisition, and rendering belong in the
  future independent `docker-lens` project; BoxFerry owns only the semantic mapping.
- Native Podman protocol handling, version evidence, read-only acquisition, and deterministic
  rendering belong in the independent `podman-lens` project; BoxFerry owns only the semantic
  mapping.
- Native Kubernetes resource handling and version validation belong in the future independent
  `kubernetes-lens` project; BoxFerry owns only the semantic mapping and supported-target policy.
- Podman integration is Phase 2 and proceeds independently of Docker. Docker and Kubernetes
  integrations remain deferred. Do not add placeholders or private replacements for Lens
  responsibilities.
- Helm and Kustomize input should initially be rendered by their native tools and then handled as Kubernetes resources.
- Format libraries must not depend on BoxFerry.

## Origin policy

BoxFerry is implemented from scratch. Do not copy or mechanically translate source code from Podlet, `compose_spec_rs`, or another converter. External tools may be used as documented behavioral references or differential-test oracles. Record the tool, version, command, and fixture provenance when doing so.

Using a third-party dependency is allowed when its license, maintenance state, and architectural role have been reviewed. Record consequential dependency choices in an ADR.

## Non-negotiable behavior

- Never silently discard user configuration.
- Every native input must pass through its importer into the neutral model, and every native output
  must be produced from that model by its exporter. Same-format native shortcuts are forbidden.
- Every non-exact conversion must produce a structured outcome and an actionable diagnostic.
- Preserve source locations and source-specific data long enough to explain conversion decisions.
- Treat user input as fallible; malformed input must not panic the process.
- Keep target compatibility explicit. Do not infer a target version from the development machine.
- Keep file parsing free of runtime side effects. Read-only native acquisition must be explicit and
  replaceable.
- BoxFerry never applies generated artifacts, invokes generated commands, deploys infrastructure,
  or sends mutating runtime API requests.
- Secrets must be redacted from diagnostics, snapshots, and logs by default.

## Development rules

- Put code in the target crate described by `docs/project-structure.md`; do not grow a root-level monolith.
- Keep the `boxferry` facade usable as a library; the CLI must call the same public orchestration
  APIs available to external consumers.
- Keep the neutral model independent of Compose, Quadlet, Docker, Podman, and Kubernetes types.
- Add tests with every behavior change. Prefer small unit tests plus an end-to-end fixture when a mapping changes.
- Update capability data and evidence when target-version behavior changes.
- Update documentation in the same change as user-visible behavior or architectural changes.
- Start every repository-owned complete YAML document with `---`; marker-free YAML belongs only in
  explicit parser test data.
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

## GitHub issue-to-PR workflow

When the user authorizes creating an issue, branch, commit, push, and pull request, follow this
sequence:

1. Inspect `git status` and the complete diff. Identify the exact pull-request scope and preserve
   unrelated changes.
2. Search for an existing issue, then create a focused GitHub issue when no duplicate exists.
3. Fetch `origin/main`, verify local `main` is synchronized, and create
   `TheRealBecks/issue<NUMBER>` from it.
4. Complete and review the change without staging unrelated files.
5. Run `./scripts/check-all.sh` from the repository root for normal changes. For an ADR-recorded
   intentional public break, run `BOXFERRY_SEMVER_RELEASE_TYPE=major ./scripts/check-all.sh` only
   when the eventual commit and pull-request title use a release-worthy breaking `!`. Do not commit,
   push, or create the pull request unless the applicable complete gate passes. Any source, test,
   configuration, or documentation change made after the successful run invalidates it and requires
   the complete task to run again.
6. Stage only explicit in-scope paths, run `git diff --cached --check`, review the staged diff, and
   create one intentional commit.
7. Push the issue branch and open a ready-for-review pull request containing `Closes #<NUMBER>`.
   Use a `feat`, `fix`, `perf`, `refactor`, or `revert` Conventional Commit title only for
   release-worthy code; classify documentation, tests, CI, build tooling, formatting, and other
   maintenance as `docs`, `test`, `ci`, `build`, `style`, or `chore` so release-plz ignores it.
8. Read the pull request back from GitHub and report its issue, branch, commit, validation result,
   URL, and current check state.

The issue may be created before local validation so failed work remains traceable, but a failed or
incomplete `./scripts/check-all.sh` run is a hard gate against commit, push, and pull-request
creation.

The primary Sol agent runs this workflow with high reasoning effort. Sol owns the issue and branch,
final integration and diff review, complete local gate, explicit staging, commit, push, pull-request
creation, and GitHub readback. Terra subagents may perform bounded research, implementation,
read-only review, or non-mutating verification assigned by Sol, but never execute the Git or GitHub
write steps. Because `./scripts/check-all.sh` formats repository files, its mandatory final run
remains Sol's responsibility and is not replaced by read-only Terra verification.

## Multi-agent coordination

- The human-facing batch prompt and operator responsibilities are documented in
  `docs/agent-workflow.md`.
- Use the primary BoxFerry agent as the integrator for tasks spanning BoxFerry, ComposeLens,
  PodmanLens, and QuadletLens.
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
