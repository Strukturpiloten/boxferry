# Development environment

Use the BoxFerry Dev Container. It provides the pinned Rust, Node, documentation, lint, audit, and
GitHub tools used by CI.

## Local work

Open the five-repository workspace for BoxFerry, ComposeLens, PodmanLens, QuadletLens, and the
BoxFerry website, then run:

```console
./scripts/check-all.sh
```

The task formats files before checking them. Any source, test, configuration, or documentation
change after a successful run invalidates the result.

Useful focused commands are listed in `AGENTS.md` and `.cargo/config.toml`. Do not replace the
complete gate with a focused command before publication.

## Issue-to-PR contribution workflow

1. Inspect the complete worktree and preserve unrelated changes.
2. Create or reuse one focused GitHub issue.
3. Synchronize `main` and create `TheRealBecks/issue<NUMBER>`.
4. Implement and review the change.
5. Run `./scripts/check-all.sh` after the final edit.
6. Stage only explicit paths and run `git diff --cached --check`.
7. Commit, push, and open a ready pull request containing `Closes #<NUMBER>`.
8. Read the issue and pull request back and monitor required checks.

All steps must pass before the change is committed, pushed, or submitted.

The primary Sol agent uses high reasoning effort. Sol owns the final diff, complete gate, staging,
commit, push, and GitHub readback. Terra agents may perform bounded research, implementation,
review, or non-mutating verification. They never perform Git or GitHub writes. The complete final
gate remains Sol's final responsibility.

## Personal GitHub authentication

The Dev Container stores `gh` authentication in its dedicated persistent volume. Run the workspace
authentication task when the token changes. The token is not copied into the repository or the host
CLI configuration.
