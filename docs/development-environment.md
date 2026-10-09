# Development environment

Use the Dev Container's pinned Rust, Node, documentation, audit, and GitHub tools, shared with CI
across the seven-repository workspace, including DockerLens and KubernetesLens.

Keep sibling checkouts beside `boxferry`; open `boxferry-lenses.code-workspace` in the Dev Container.
KubernetesLens remains editing-only until it has project and verification scripts; aggregate checks
remain unchanged. After mount changes, use **Dev Containers: Rebuild and Reopen in Container**;
reloading VS Code alone cannot add mounts.

## Build the CLI

Cargo uses its default workspace target directory. Build and run the repository-local release binary:

```console
cargo build --release --locked --package boxferry
./target/release/boxferry --version
```

After pulling from an older workspace, run **Dev Containers: Rebuild Container** in VS Code.
Existing terminals may use `unset CARGO_TARGET_DIR` until rebuilt.

## Shared website toolchain

The `.devcontainer/Dockerfile` uv image must match the website's exact `tool.uv.required-version`
in `boxferry-website/pyproject.toml` and CI/deployment inputs. Renovate tracks all nine Dockerfile
pins, grouping non-major toolchain updates while preserving image release/tag/digest pairs and
stage aliases. Website uv pins are grouped separately: coordinate repository updates, retain
the exact requirement, and rebuild after Dockerfile changes.

## Rust toolchain components

`rust-toolchain.toml` selects Rust, Clippy, rustfmt, and LLVM coverage tools, preinstalled in the
Dev Container. Rustup installs later pinned components.

For missing `llvm-tools-preview` in older containers, run from the BoxFerry root **inside the container**:

```console
rustup component add llvm-tools-preview
bash .devcontainer/verify-tools.sh
```

Rebuild the Dev Container after pulling the fix.

## Refresh the Dev Container feature lock

Renovate's Dev Container feature updates leave the checksum-bearing lock unchanged.
Regenerate it from the repository root with the pinned CLI before reviewing:

```console
npx --yes @devcontainers/cli@0.89.0 upgrade --workspace-folder .
```

Commit the manifest and lock file together; never replace the Renovate-managed CLI pin with
`latest`.

## Local verification

For lightweight pre-push cleanup, run:

```console
./scripts/format-lint.sh --fix
```

VS Code task **BoxFerry: Format and lint only (no tests)** checks files, Actions, Clippy and
whitespace without tests. Clippy defaults to two jobs; constrained machines can use
`BOXFERRY_LINT_JOBS=1 ./scripts/format-lint.sh --fix`. `--check` skips formatting.
Run in the Dev Container for pinned linters.

`python3 scripts/validation-plan.py run-local` provides change-aware feedback; `plan --event local`
previews it. Use `--docs-only` or `--full`; VS Code tasks match. Unset shared `CARGO_TARGET_DIR`
for this task and the complete gate: explicit targets must stay inside this worktree to avoid
stale fixture paths. Download caches remain reusable.

Cleanup is not test evidence. Run the complete gate after the final edit when resources permit:

```console
./scripts/check-all.sh
```

It formats first; edits invalidate results. Focused checks cannot replace it. Contributors unable
to complete it may push after lightweight cleanup succeeds and rely on required GitHub checks;
merging requires those checks to pass.

## Issue-to-PR contribution workflow

1. Inspect the worktree and preserve unrelated changes.
2. Create or reuse one focused GitHub issue.
3. Synchronize `main` and create `TheRealBecks/issue<NUMBER>`.
4. Implement and review the scoped diff.
5. Run `./scripts/format-lint.sh --fix`; run `./scripts/check-all.sh` locally when resources permit.
6. Stage explicit paths; run `git diff --cached --check` and review.
7. Commit once, push, and open a ready pull request containing `Closes #<NUMBER>`.
8. Read back the issue and pull request; monitor required checks.
9. After a verified merge, return the primary checkout to synchronized `main`, remove any temporary
   worktree with `git worktree remove <recorded-path>`, delete the verified merged local issue branch
   with `git branch --delete --force TheRealBecks/issue<NUMBER>`, and run
   `git worktree prune --verbose`. Verify `git worktree list --porcelain` and
   `git status --short --branch` show no stale issue checkout.

Run the lightweight task before pushing. The selected GitHub checks and
fail-closed `PR gate` must pass before merge. Code and unknown changes still require complete
deterministic validation; `main` pushes and releases always run the complete plan.

The contributor exception does not waive `AGENTS.md`: agents must complete the local gate
before committing, pushing, or creating a PR.

The primary agent uses GPT-6 Sol with `xhigh` reasoning. Worker agents never perform Git or GitHub
writes. The complete gate remains the primary agent's final responsibility. See [`AGENTS.md`](../AGENTS.md).

## GitHub authentication

The Dev Container stores `gh` authentication in a dedicated persistent volume. Run the workspace
authentication task after token changes. Tokens never enter repository or host CLI configuration.

## Agent-assisted verification

Models/roles: [`.codex/`](../.codex/); permissions/workflow: [`AGENTS.md`](../AGENTS.md).
Reload or start a trusted session after configuration changes; align primary-session overrides
with Sol/xhigh.

`./scripts/check-all.sh --check` runs the full gate without formatting; default/`--fix` formats
first. Both modes may change ignored caches/build artifacts. Verifiers report failures without
edits; the primary owns the final gate and standing-authorized merges.

Shell-runner regression tests require the Linux Dev Container. Agent-configuration checks remain
platform-independent; hosted validation runs on Linux only.
