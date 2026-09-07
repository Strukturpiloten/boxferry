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

## Local verification

Run the complete gate after the final edit:

```console
./scripts/check-all.sh
```

It formats before checking. Any later source, test, configuration, or documentation edit
invalidates the result. Focused aliases in `.cargo/config.toml` help during development but do not
replace this gate.

## Issue-to-PR contribution workflow

1. Inspect the worktree and preserve unrelated changes.
2. Create or reuse one focused GitHub issue.
3. Synchronize `main` and create `TheRealBecks/issue<NUMBER>`.
4. Implement and review the complete scoped diff.
5. Run `./scripts/check-all.sh`.
6. Stage explicit paths, run `git diff --cached --check`, and review the staged diff.
7. Commit once, push, and open a ready pull request containing `Closes #<NUMBER>`.
8. Read the issue and pull request back and monitor required checks.
9. After a verified merge, return the primary checkout to synchronized `main`, remove any temporary
   worktree with `git worktree remove <recorded-path>`, delete the verified merged local issue branch
   with `git branch --delete --force TheRealBecks/issue<NUMBER>`, and run
   `git worktree prune --verbose`. Verify `git worktree list --porcelain` and
   `git status --short --branch` show no stale issue checkout.

All steps must pass before the change is committed, pushed, or submitted.

The primary agent uses high reasoning effort and owns the final diff, complete gate, staging,
commit, push, and GitHub readback. Worker agents may perform bounded work.
Worker agents never perform Git or GitHub writes.
The complete gate remains the primary agent's final responsibility.

## GitHub authentication

The Dev Container stores `gh` authentication in a dedicated persistent volume. Run the workspace
authentication task when the token changes. No token is copied into the repository or host CLI
configuration.

## Agent-assisted verification

Repository model defaults and role overrides live in [`.codex/`](../.codex/); permissions and
workflow ownership remain defined in [`AGENTS.md`](../AGENTS.md). Reload or start a new trusted
project session after updating configuration; an explicit session override can take precedence.

Use `./scripts/check-all.sh --check` to run the complete gate without formatting repository-owned
files. The default command (or `--fix`) still formats first. Both modes run the same validation;
ignored caches and build artifacts may change. Verifiers report failures without fixing files,
and the primary agent owns the final complete gate and any explicitly authorized merge.

The shell-runner regression tests target the Linux Dev Container gate. Agent-configuration checks
remain platform-independent; the macOS portability lane does not require Linux validation tools.
