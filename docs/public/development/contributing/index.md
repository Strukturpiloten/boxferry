# Contributing

Use the Dev Container, then run:

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

Pull requests use the repository's issue-to-PR workflow. The complete local gate must pass after
the final source or documentation change and before the commit is pushed.
