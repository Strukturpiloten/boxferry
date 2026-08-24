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

All steps must pass before the change is committed, pushed, or submitted.

The primary Sol agent uses high reasoning effort and owns the final diff, complete gate, staging,
commit, push, and GitHub readback. Terra agents may perform bounded work.
Terra agents never perform Git or GitHub writes.
The complete gate remains Sol's final responsibility.

## GitHub authentication

The Dev Container stores `gh` authentication in a dedicated persistent volume. Run the workspace
authentication task when the token changes. No token is copied into the repository or host CLI
configuration.
