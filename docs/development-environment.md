# Development environment

Use the Dev Container. It provides the pinned Rust, Node, documentation, audit, and GitHub tools
used by CI for the five-repository workspace.

## Build the CLI

Cargo uses its default workspace target directory. Build the release binary and run it from the
expected repository-local path:

```console
cargo build --release --locked --package boxferry
./target/release/boxferry --version
```

After pulling the change from an older workspace, run **Dev Containers: Rebuild Container** in VS
Code. Existing containers retain their original environment until rebuilt. To use the default in
an already-open terminal before rebuilding, run `unset CARGO_TARGET_DIR`.

## Rust toolchain components

`rust-toolchain.toml` selects the workspace Rust version and its required components:
Clippy, rustfmt, and LLVM tools for coverage. The Dev Container preinstalls that toolchain;
Rustup also installs these components when a later checkout changes the pinned version.
Components installed for the image's default toolchain do not carry over to another version.

If an older container reports missing `llvm-tools-preview`, run these commands from the
BoxFerry repository root **inside the container**:

```console
rustup component add llvm-tools-preview
bash .devcontainer/verify-tools.sh
```

After pulling the fix, use **Dev Containers: Rebuild Container** for the updated image setup.

## Refresh the Dev Container feature lock

Renovate proposes Dev Container feature updates without rewriting the checksum-bearing lock file.
From the repository root, regenerate it with the pinned CLI before reviewing the resulting diff:

```console
npx --yes @devcontainers/cli@0.89.0 upgrade --workspace-folder .
```

Commit `.devcontainer/devcontainer.json` and `.devcontainer/devcontainer-lock.json` together. A
later CLI release is a separate Renovate-managed documentation update; do not replace the pin with
`latest`.

## Local verification

For fast pre-push cleanup on a smaller computer, run:

```console
./scripts/format-lint.sh --fix
```

The matching VS Code task is **BoxFerry: Format and lint only (no tests)**. The command formats
Rust and repository-owned files, checks staged and unstaged whitespace, runs the file and GitHub
Actions linters, and runs Clippy. It executes no tests. Clippy uses two Cargo jobs by default; use
`BOXFERRY_LINT_JOBS=1 ./scripts/format-lint.sh --fix` on a particularly constrained machine.
`--check` verifies without formatting. Run it in the Dev Container so every pinned linter is
available.

This task is a cleanliness aid, not evidence that tests passed. Run the complete gate after the
final edit when local resources permit:

```console
./scripts/check-all.sh
```

It formats before checking. Any later source, test, configuration, or documentation edit
invalidates the result. Focused aliases in `.cargo/config.toml` and the format/lint-only task help
during development but do not replace the complete gate. Contributors whose machines cannot
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

The lightweight task must pass before the change is pushed. Either the local complete gate or all
required GitHub checks must provide complete validation before merge.

The resource-constrained contributor option above does not waive the coding-agent rule in
`AGENTS.md`: agents must complete the local gate before committing, pushing, or creating a PR.

The primary agent uses GPT-6 Astra with `xhigh` reasoning and owns the final diff, complete gate, staging,
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
overrides aligned with Astra/xhigh.

`./scripts/check-all.sh --check` runs the full gate without formatting; the default or `--fix`
formats first. Ignored caches/build artifacts may change in either mode. Verifiers report failures
without edits; the primary owns the final gate and standing-authorized merges.

Shell-runner regression tests require the Linux Dev Container. Agent-configuration checks remain
platform-independent; macOS portability needs no Linux validation tools.
