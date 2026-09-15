# Contributing

Use the Dev Container. For quick pre-push cleanup on a smaller computer, run:

```console
./scripts/format-lint.sh --fix
```

The VS Code task **BoxFerry: Format and lint only (no tests)** runs the same command. It executes
no tests and is not validation evidence. Run the complete gate locally when resources permit:

```console
./scripts/check-all.sh
```

Before changing code:

1. Read `AGENTS.md` and the accepted architecture decisions relevant to the change.
2. Keep native parsing in the Lens repository that owns the format.
3. Keep the neutral model free of native format types.
4. Add positive and negative tests with every behavior change.
5. Update the public documentation when user-visible behavior changes.

Do not silently discard configuration, infer a target version from the development machine, or
put conversion rules in the CLI.

Pull requests use the repository's issue-to-PR workflow. The lightweight task must pass before a
push. Either the complete local gate or every required GitHub check must pass after the final
source or documentation change before merge.
