# Repository guidance for coding agents

This file applies to the complete BoxFerry repository.

## Read before changing code

Always read `README.md`, `docs/architecture.md`, and the
[decision index](docs/decisions/). Then follow the task:

| Task                      | Additional source                                 |
| ------------------------- | ------------------------------------------------- |
| Public API                | `docs/api-stability.md` and owning Rustdoc        |
| Tests or fixtures         | `docs/testing.md` and `fixtures/README.md`        |
| Dependency or tool        | `docs/dependency-policy.md`                       |
| Platform behavior         | `docs/platform-support.md`                        |
| Release                   | `docs/releasing.md`                               |
| Public documentation      | `docs/README.md` and relevant `docs/public/` page |
| Local setup or submission | `docs/development-environment.md`                 |

Read only the ADRs relevant to the task. If a change contradicts an accepted ADR, add or supersede
the decision in the same change.

## Product boundaries

- BoxFerry owns orchestration, the neutral model, semantic adapters, loss policy, diagnostics, and
  the CLI.
- ComposeLens, PodmanLens, and QuadletLens own their native formats.
- Every input passes through an importer into the neutral model; every output comes from an
  exporter. Same-format shortcuts are forbidden.
- Podman acquisition is explicit and read-only. BoxFerry never applies output, invokes generated
  commands, deploys infrastructure, or sends mutating runtime requests.
- Docker and Kubernetes remain deferred. Do not add placeholder adapters.
- Format libraries must not depend on BoxFerry.

BoxFerry is implemented from scratch. External tools may be documented references or differential
oracles, but source code must not be copied or mechanically translated. Record oracle version,
command, provenance, license, and redistribution status.

## Engineering rules

- Put neutral types in `boxferry-model`, planning in `boxferry-engine`, mappings in their adapter
  crates, and presentation or file writes in the facade.
- Keep the facade usable without the CLI and keep native types out of the neutral model.
- Never silently discard configuration. Every non-exact decision needs a structured outcome and
  actionable diagnostic.
- Treat input as fallible, retain source evidence, keep target versions explicit, and redact
  protected values by default.
- Add positive, failure, unsupported, and version-boundary tests with behavior changes.
- Update machine capability evidence instead of duplicating it in prose.
- Start repository-owned complete YAML documents with `---`; parser fixtures may omit it.
- Pin GitHub Actions to a full commit SHA with the exact release tag in a comment.

## Verification

The workspace uses Rust 2024 and supports Rust 1.85.0 and newer. Focused `ci-*` aliases live in
`.cargo/config.toml`. Run `./scripts/check-all.sh` after the final edit; never weaken a lint or
replace the complete gate with a focused command before publication.

## GitHub issue-to-PR workflow

When the user authorizes Git and GitHub writes:

1. Inspect status and the complete diff; preserve unrelated work.
2. Search for a duplicate, then create one focused issue when needed.
3. Fetch `origin/main`, verify synchronization, and create
   `TheRealBecks/issue<NUMBER>`.
4. Complete and review the scoped change.
5. Run `./scripts/check-all.sh`. A failed or incomplete run is a hard gate against commit, push,
   and pull-request creation; a later edit invalidates the run.
6. Stage explicit paths, run `git diff --cached --check`, review the staged diff, and create one
   intentional commit.
7. Push and open a ready pull request containing `Closes #<NUMBER>`.
8. Read back the issue, commit, pull request, and required checks.
9. After an authorized, verified merge, restore the primary checkout to synchronized `main`,
   remove any temporary worktree with `git worktree remove <recorded-path>`, delete the verified
   merged local issue branch with `git branch --delete --force TheRealBecks/issue<NUMBER>`, run
   `git worktree prune --verbose`, and read back `git worktree list --porcelain` plus
   `git status --short --branch`. Never leave `target/*-issue*` worktree registrations behind.

Opening and reading back the ready pull request is the default stopping point. Authorization to run
the Git workflow or perform GitHub writes does not authorize a merge.

Merge only when the user explicitly authorizes merging the specific pull request or the scoped set
of pull requests in the current request. Immediately before merging, read back the exact head
commit and verify that the pull request is ready, mergeable, and has every required check
successful. Never bypass branch protection, use an administrator override, or infer authority for
an out-of-scope release, publication, or deployment pull request.

Use the repository's normal merge method with an exact-head safeguard, then read back and report
the merged state and merge commit.

Use release-worthy Conventional Commit types only for product changes. Use `docs`, `test`,
`ci`, `build`, `style`, or `chore` for non-release work.

The primary agent runs this workflow with high reasoning effort. The primary agent owns integration, final
verification, Git writes, and GitHub readback. Worker subagents may perform bounded research,
editing, or read-only review but never execute the Git or GitHub write steps. The complete gate
remains the primary agent's responsibility.

## Multi-repository work

The primary BoxFerry agent defines the shared contract before delegating. Agents may edit separate
repository checkouts concurrently but never the same checkout. The primary agent reviews and
verifies every final diff.

## Agent roles and verification

Model defaults belong in [`.codex/config.toml`](.codex/config.toml); task-specific models and
reasoning belong in [`.codex/agents/`](.codex/agents/). Use the repository's high-effort primary
default for normal work; explicitly request `xhigh` for unusually difficult architecture or
migration analysis. These are defaults, not permission grants.

- Delegate only when the user or applicable instructions request it, and assign a bounded task.
- Use at most three subagents. Define the shared contract and file ownership before delegation.
- Never run two writers in one checkout. Research and review remain read-only.
- The reviewer checks the original requirements and independent expected results, not just agreement
  between the implementation and its tests.
- After writing finishes, the verifier runs `./scripts/check-all.sh --check`. It reports failures
  without formatting or editing tracked files; ignored build artifacts and caches are allowed.
- Avoid concurrent full gates or heavy runtime tests. The primary agent owns integration, the final
  complete gate, and every authorized Git or GitHub write.

The default `./scripts/check-all.sh` still formats before checking. `--check` runs the same
complete gate without source formatting; it is not a reduced test tier. A later edit invalidates
either result. Neither mode grants release, publication, or deployment authority.

## Code discovery

For code discovery, use an available codebase-memory graph first; otherwise use CodeGraph only if
the repository already has a usable index. Do not create an index without user authorization.
If neither graph is available or a query cannot answer the question, use `rg` and targeted reads.
For string literals, configuration, scripts, and documentation, start with `rg` directly.
