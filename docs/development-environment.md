# Development environment

Use the Dev Container. It provides the pinned Rust, Node, documentation, audit, and GitHub tools
used by CI for the six-repository workspace, including the DockerLens checkout.

## Build the CLI

Cargo uses its default workspace target directory. Build the release binary and run it from the
expected repository-local path:

```console
cargo build --release --locked --package boxferry
./target/release/boxferry --version
```

After pulling from an older workspace, run **Dev Containers: Rebuild Container** in VS Code.
Existing terminals may use `unset CARGO_TARGET_DIR` until rebuilt.

## Rust toolchain components

`rust-toolchain.toml` selects Rust, Clippy, rustfmt, and LLVM coverage tools. The Dev Container
preinstalls them; Rustup installs components for later pinned versions.

If an older container reports missing `llvm-tools-preview`, run these commands from the
BoxFerry repository root **inside the container**:

```console
rustup component add llvm-tools-preview
bash .devcontainer/verify-tools.sh
```

Rebuild the Dev Container after pulling the fix.

## Refresh the Dev Container feature lock

Renovate proposes Dev Container feature updates without rewriting the checksum-bearing lock file.
From the repository root, regenerate it with the pinned CLI before reviewing the resulting diff:

```console
npx --yes @devcontainers/cli@0.89.0 upgrade --workspace-folder .
```

Commit the manifest and lock file together; never replace the Renovate-managed CLI pin with
`latest`.

## Local verification

For fast pre-push cleanup on a smaller computer, run:

```console
./scripts/format-lint.sh --fix
```

The matching VS Code task is **BoxFerry: Format and lint only (no tests)**. It checks files,
Actions, Clippy and whitespace without tests. Clippy defaults to two jobs; use
`BOXFERRY_LINT_JOBS=1 ./scripts/format-lint.sh --fix` on a particularly constrained machine.
`--check` verifies without formatting. Run it in the Dev Container so every pinned linter is
available.

`python3 scripts/validation-plan.py run-local` provides change-aware feedback; `plan --event local`
previews it. Use `--docs-only` or `--full`; VS Code tasks match. For this task and the complete
gate, unset a shared `CARGO_TARGET_DIR`: explicit targets must be inside this worktree to prevent
stale fixture paths. Cargo download caches remain reusable.

This task is a cleanliness aid, not evidence that tests passed. Run the complete gate after the
final edit when local resources permit:

```console
./scripts/check-all.sh
```

It formats first; later edits invalidate the result. Focused aliases and tasks cannot replace
the complete gate. Contributors whose machines cannot
complete the gate may push after the lightweight task succeeds and rely on required GitHub checks;
the pull request is not ready to merge until those checks pass.

## Issue-to-PR contribution workflow

1. Inspect the worktree and preserve unrelated changes.
2. Create or reuse one focused GitHub issue.
3. Synchronize `main` and create `TheRealBecks/issue<NUMBER>`.
4. Implement and review the complete scoped diff.
5. Run `./scripts/format-lint.sh --fix`; run `./scripts/check-all.sh` locally when resources permit.
6. Stage explicit paths, run `git diff --cached --check`, and review the staged diff.
7. Commit once, push, and open a ready pull request containing `Closes #<NUMBER>`.
8. Read the issue and pull request back and monitor required checks.
9. After a verified merge, return the primary checkout to synchronized `main`, remove any temporary
   worktree with `git worktree remove <recorded-path>`, delete the verified merged local issue branch
   with `git branch --delete --force TheRealBecks/issue<NUMBER>`, and run
   `git worktree prune --verbose`. Verify `git worktree list --porcelain` and
   `git status --short --branch` show no stale issue checkout.

Run the lightweight task before pushing. The selected GitHub checks and
fail-closed `PR gate` must pass before merge. Code and unknown changes still require complete
deterministic validation; `main` pushes and releases always run the complete plan.

The resource-constrained contributor option above does not waive the coding-agent rule in
`AGENTS.md`: agents must complete the local gate before committing, pushing, or creating a PR.

The primary agent uses GPT-6 Sol with `xhigh` reasoning and owns the final diff, complete gate, staging,
commit, push, and GitHub readback. Worker agents may perform bounded work.
Worker agents never perform Git or GitHub writes.
The complete gate remains the primary agent's final responsibility.

## GitHub authentication

The Dev Container stores `gh` authentication in a dedicated persistent volume. Run the workspace
authentication task when the token changes. No token is copied into the repository or host CLI
configuration.

## Agent-assisted verification

Models and roles: [`.codex/`](../.codex/); permissions and workflow: [`AGENTS.md`](../AGENTS.md).
Reload or start a trusted session after configuration changes. Keep explicit primary-session
overrides aligned with Sol/xhigh.

`./scripts/check-all.sh --check` runs the full gate without formatting; the default or `--fix`
formats first. Ignored caches/build artifacts may change in either mode. Verifiers report failures
without edits; the primary owns the final gate and standing-authorized merges.

Shell-runner regression tests require the Linux Dev Container. Agent-configuration checks remain
platform-independent; hosted validation runs on Linux only.
