# Development environment

Use the Dev Container's pinned Rust, Node, documentation, audit and GitHub tools shared with CI
across the seven-repository workspace.

Keep sibling checkouts beside `boxferry`; open `boxferry-lenses.code-workspace` in the Dev Container.
After mount changes, use **Dev Containers: Rebuild and Reopen in Container**; reloading VS Code
alone cannot add mounts.

## Build the CLI

Cargo uses its default workspace target directory.
Build and run the repository-local release binary:

```console
cargo build --release --locked --package boxferry
./target/release/boxferry --version
```

After pulling from an older workspace, run **Dev Containers: Rebuild Container** in VS Code. Existing terminals may use
`unset CARGO_TARGET_DIR` until rebuilt.

## Shared development tools

The [shared Python project](../automation/pyproject.toml) owns uv, Ruff and ty. Coordinate the Dev
Container uv image with each consumer's selected automation revision; historical website deployment
sources retain their verified toolchain contract. Renovate tracks Dockerfile image and Cargo pins,
including image release/tag/digest pairs and stage aliases. The [shared inventory](../automation/tool-artifacts.json)
owns downloaded file-tool versions and checksums used by the [installer](../scripts/install-file-tools.sh).
Rebuild after toolchain updates.

## Kubernetes tooling

Git comes from the pinned base; `gh` uses the locked feature. Rebuild after updates.
[The installer](../scripts/install-kubernetes-tools.sh) supplies checksum-pinned kind, kubectl,
Helm, and Kustomize. Renovate tracks their versions and per-architecture integrity records. A
trusted scheduled and post-CI workflow refreshes kubectl, Helm, and Kustomize checksums from their
official metadata sources, verifies both architecture artifact digests, and updates only an eligible
Renovate PR; kind's release-attachment manager updates its asset digests. No Dashboard approval or
manual refresh is required for these four development tools. The workflow treats candidate content
as inert text, hashes streamed artifacts without storing them, and never executes candidate code or
artifacts. The complete local gate verifies the installer working tree, and the required integrity
job verifies the installer from the exact tested Git commit. [Provisioning and consumer audit](https://github.com/Strukturpiloten/boxferry/issues/445)
covers host/CI reuse and separate Podman access;
[arm64 execution](https://github.com/Strukturpiloten/boxferry/issues/446) remains pending.

## Rust toolchain components

`rust-toolchain.toml` selects Rust, Clippy, rustfmt and LLVM coverage tools; the Dev Container
preinstalls them. Rustup installs later pinned components.

For missing `llvm-tools-preview`, run inside the container from the BoxFerry root:

```console
rustup component add llvm-tools-preview
bash .devcontainer/verify-tools.sh
```

Rebuild the Dev Container after pulling the fix.

## Refresh the Dev Container feature lock

Renovate feature updates leave the checksum-bearing lock unchanged. Regenerate it before review:

```console
npx --yes @devcontainers/cli@0.89.0 upgrade --workspace-folder .
```

Commit the manifest and lock file together; never replace the Renovate-managed CLI pin with
`latest`.

## Local verification

Follow [Podman-only policy](decisions/0079-native-test-runtime-selection.md); pre-push cleanup:

```console
./scripts/format-lint.sh --fix
```

VS Code task **BoxFerry: Format and lint only (no tests)** checks files, Actions, Clippy and
whitespace. Clippy defaults to two jobs; use `BOXFERRY_LINT_JOBS=1 ./scripts/format-lint.sh --fix`
for constrained machines. `--check` skips formatting. Use the Dev Container for pinned linters.

`python3 scripts/validation-plan.py run-local` provides change-aware feedback; `plan --event local`
previews it. Use `--docs-only` or `--full`; VS Code tasks match. Unset shared `CARGO_TARGET_DIR`
for this task and the complete gate: explicit targets must stay inside this worktree to avoid
stale fixture paths. Download caches remain reusable.

The [shared-workflow rollout](https://github.com/Strukturpiloten/boxferry/issues/549) will select PR and
merge queue checks from trusted base policy and tested changes. Pure prose can skip Rust; executable
documentation retains tests, known mixed changes use the union, and unknown changes run the full
plan. `Required CI` verifies selected jobs and planned skips. Adoption is tracked per repository.

Cleanup is not test evidence. Run the complete gate after the final edit when resources permit:

```console
./scripts/check-all.sh
```

It formats first; edits invalidate results. Focused checks cannot replace it. Contributors unable
to complete it may push after lightweight cleanup succeeds and rely on required GitHub checks;
merging requires those checks to pass.

## Issue-to-PR contribution workflow

Use a focused issue and branch and review the diff. The fail-closed `Required CI` check must pass
before merge; main pushes and releases run complete validation. See [`AGENTS.md`](../AGENTS.md)
for the complete agent Git and GitHub workflow; agents must complete the local gate before committing,
pushing, or opening a PR.

## GitHub authentication

The Dev Container stores `gh` authentication in a dedicated persistent volume. Run the workspace
authentication task after token changes; tokens never enter repository or host CLI configuration.

## Agent-assisted verification

Models/roles: [`.codex/`](../.codex/); permissions/workflow: [`AGENTS.md`](../AGENTS.md).
After configuration changes, reload or start a trusted session; align primary overrides with Sol/xhigh.

`./scripts/check-all.sh --check` runs the full gate without formatting; default/`--fix` formats
first. Both modes may change ignored caches/build artifacts. Verifiers report failures without
edits; the primary owns the final gate and standing-authorized merges.

Shell-runner regression tests require the Linux Dev Container; agent-configuration checks are
platform-independent. Hosted validation runs on Linux.
