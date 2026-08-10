# Parallel agent workflows

## BoxFerry key batch

Use this prompt for implementation inside BoxFerry:

```text
Implement the next 10 related BoxFerry keys using the documented parallel BoxFerry key-batch
workflow. Keep Sol responsible for the contract, integration, and final review, and use Terra for
specification research, the neutral-model foundation, the Compose and Quadlet adapter work, facade
coverage, and verification. Do not commit, push, publish, tag, or release. Report the completed
keys, fidelity boundaries, changed files, validation results, and remaining work.
```

Change `10` to the desired batch size. To select exact keys, replace “the next 10 related
BoxFerry keys” with an explicit list. State separately when release preparation is wanted; it is
not implied by implementation.

The primary agent will:

1. run specification research and freeze the shared contract;
2. delegate the neutral-model foundation to one writer;
3. prepare isolated checkouts and run the Compose and Quadlet writers concurrently;
4. integrate and review every uncommitted diff;
5. add or delegate public facade and golden coverage; and
6. run one independent verifier after final integration.

Before starting, use a committed baseline when practical and reload project agent definitions
after changing `.codex/` configuration. The primary agent creates and removes temporary
checkouts; the human does not need to manage them. After completion, the human reviews and commits
the integrated change.

This workflow follows the repository's [`AGENTS.md`](../AGENTS.md) coordination rules and the
[official Codex subagent guidance](https://developers.openai.com/codex/subagents/).

## ComposeLens and QuadletLens key batch

Use this prompt when both Lens repositories need their next native keys:

```text
Implement the next 10 related ComposeLens and QuadletLens keys using the documented parallel Lens
workflow. Keep Sol responsible for the cross-repository contract, task selection, integration
decisions, and final review. Run specification research first. Then run one Terra implementation
worker in ComposeLens and one in QuadletLens concurrently, with exclusive ownership of their
separate repositories. Wait for both, review every diff, and run one independent verifier per
changed repository. Do not commit, push, publish, tag, or release. Report the keys completed in
each repository, evidence boundaries, changed files, validation results, and remaining work.
```

Change `10` to the desired batch size or provide an explicit key list. The number describes keys,
not repeated workflow runs. The primary agent must not force symmetric behavior where the Compose
and Quadlet specifications differ; one-sided support and evidence gaps remain explicit.

ComposeLens and QuadletLens are sibling repositories, so their implementation workers may write
concurrently without BoxFerry's isolated-checkout setup. Each worker follows its repository's own
`AGENTS.md`, changes only that repository, and returns an uncommitted diff. Sol reviews the shared
contract and both diffs before the repository-specific verifiers run.

Request release preparation separately after implementation and verification. Preparing release
metadata does not authorize commits, publication, tags, or GitHub releases.
