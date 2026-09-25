# Repository guidance for coding agents

Applies throughout BoxFerry.

## Read before changing code

Read `README.md`, `docs/architecture.md`, and the [decision index](docs/decisions/). Read relevant
ADRs; add or supersede any contradicted accepted ADR in the same change. Also read:

- Public API: `docs/api-stability.md`, owning Rustdoc.
- Tests/fixtures: `docs/testing.md`, `fixtures/README.md`.
- Dependencies/tools: `docs/dependency-policy.md`.
- Workflows/shared tasks: `docs/releasing.md`, `docs/dependency-policy.md`.
- Platform behavior: `docs/platform-support.md`; releases: `docs/releasing.md`.
- Public docs: `docs/README.md`, relevant `docs/public/` page.
- Local setup/submission: `docs/development-environment.md`.

## Product boundaries

- BoxFerry owns orchestration, neutral model, semantic adapters, loss policy, diagnostics, CLI;
  ComposeLens, PodmanLens, and QuadletLens own native formats and cannot depend on BoxFerry.
- All inputs use an importer and the neutral model; all outputs use an exporter, including
  same-format routes.
- Podman acquisition is explicit, read-only. Never apply output, invoke generated commands, deploy,
  or send mutating runtime requests.
- Docker and Kubernetes remain deferred; do not add placeholder adapters.

Implement from scratch. External tools may serve as documented references or differential oracles;
never copy or mechanically translate source. Record oracle version, command, provenance, license,
and redistribution status.

## Engineering rules

- Neutral types: `boxferry-model`; planning: `boxferry-engine`; mappings: adapter crates;
  presentation/file writes: facade. Keep the facade CLI-independent and native types out of the
  neutral model.
- Never silently discard configuration: non-exact decisions need structured outcomes and actionable
  diagnostics. Treat input as fallible; retain source evidence, explicit target versions, and default
  redaction of protected values.
- Test positive, failure, unsupported, and version-boundary behavior changes. Update machine
  capability evidence rather than duplicating it in prose.
- Start complete repository-owned YAML documents with `---` (parser fixtures exempt). Pin GitHub
  Actions to full commit SHAs with exact release tags in comments.

## Verification

Rust 2024; MSRV 1.85.0. Focused `ci-*` aliases: `.cargo/config.toml`.
`./scripts/format-lint.sh --fix` formats and lints without tests; it is not validation evidence.
Run `./scripts/check-all.sh` after the final edit. Never weaken lints or substitute focused checks
for the complete pre-publication gate.

## GitHub issue-to-PR workflow

For authorized Git/GitHub work:

1. Inspect status/full diff; preserve unrelated work. Check duplicates; create one focused issue
   when needed.
2. Fetch `origin/main`, verify synchronization, create `TheRealBecks/issue<NUMBER>`; complete and
   review the scoped change.
3. Run `./scripts/format-lint.sh --fix`, then `./scripts/check-all.sh`. Failure or incomplete
   completion is a hard gate against commit, push, and pull-request creation; later edits invalidate
   the run.
4. Stage explicit paths; run `git diff --cached --check`, review staged diff, make one intentional
   commit. Push and open a ready PR with `Closes #<NUMBER>`.
5. Read back the issue, commit, PR, and required checks.
6. After an authorized, verified merge, synchronize primary `main` with `origin/main`; remove
   recorded worktrees via `git worktree remove <recorded-path>`, delete the verified merged branch
   via `git branch --delete --force TheRealBecks/issue<NUMBER>`, run `git worktree prune --verbose`,
   and read back `git worktree list --porcelain` and `git status --short --branch`. Leave no
   `target/*-issue*` registrations.

## Workspace scope and standing GitHub authorization

The maintainer grants standing authorization for task-related Git and GitHub work only in these
workspace repositories:

- `Strukturpiloten/boxferry`
- `Strukturpiloten/compose-lens`
- `Strukturpiloten/podman-lens`
- `Strukturpiloten/quadlet-lens`
- `Strukturpiloten/boxferry-website`
- `Strukturpiloten/docker-lens`

Do not work on or modify any repository outside this explicit allowlist, including its issues,
pull requests, branches, settings, or workflows. An upstream documentation reference is not
permission to operate on that upstream repository. A newly discovered checkout is not implicitly
in scope.

For user-requested work within this scope, the primary agent may create issues, branches, commits,
pushes, and pull requests and merge verified task-related pull requests without asking for renewed
approval. This permission does not authorize unrelated backlog work, implementation of
discussion-only proposals, or expansion of the requested product scope. A later user instruction
may narrow or revoke this permission.

Immediately before merging, read back the exact head commit and verify that the pull request is
ready, mergeable, independently reviewed, and has every required check successful. Use the normal
merge method with an exact-head safeguard; never bypass branch protection or use an administrator
override. Read back the merged state and merge commit, synchronize local `main` with `origin/main`,
and remove the task's recorded worktrees and verified merged local branches while preserving
unrelated work.

This standing permission does not authorize releases, publication, deployment operations, or
merging release/publication/deployment pull requests; those require a separate explicit request.
The primary agent owns all Git and GitHub writes. Subagents remain within their assigned task and
checkout and must not perform those writes.

Reserve release-worthy Conventional Commit types for product changes; use `docs`, `test`, `ci`,
`build`, `style`, or `chore` for non-release work.

The primary agent runs this workflow as GPT-6 Astra with `xhigh` reasoning, defines shared
contracts before delegation, and reviews/verifies every final diff. Worker subagents may research,
edit separate repository checkouts, or review bounded tasks but never execute the Git or GitHub
write steps or share a writer checkout. The complete gate remains the primary agent's
responsibility, as do integration, final verification, Git writes, and GitHub readback.

## Workflow and Renovate changes

- Align equivalent local/PR/main/release definitions across BoxFerry, ComposeLens, PodmanLens,
  QuadletLens, DockerLens, and the website. Identify canonical definitions and all consumers;
  coordinate updates, validate each consumer, and link justified differences/follow-ups.
- Reuse shared scripts/actions/workflows; preserve native conformance and thresholds. BoxFerry
  owns application suites; Lens products must not depend on it.
- Added/changed software needs explicit versions and supported integrity records: image
  version tags plus digests; Action/workflow full SHAs plus exact release-tag comments;
  downloaded-tool versions plus verified checksums; package declarations plus lockfile integrity.
  Document unavailable-integrity exceptions; never invent checksums or weaken reviewed pins.
- Added/changed/moved/removed pins or definitions require same-change Renovate ownership,
  paths/extraction, grouping, approvals, and regression review. Update configuration/consumers
  together or document verified no-change evidence. Avoid duplicate managers; preserve historical
  evidence and intentional fixtures. Follow `docs/dependency-policy.md`.
- Preserve least privilege, exact-candidate evidence, failure propagation, budgets, privacy, and
  cleanup. Follow `docs/releasing.md`; never claim planned automation is delivered. One passing
  repository does not prove rollout completion. These rules grant no additional authority.

## Agent roles and verification

Model defaults belong in [`.codex/config.toml`](.codex/config.toml); task-specific models and
reasoning belong in [`.codex/agents/`](.codex/agents/). The primary manager always uses
`gpt-6-astra` with `xhigh` reasoning. Implementation, specification research, and independent review
use `gpt-6-sol` with `high` reasoning; check-only verification uses `gpt-6-luna` with `high`
reasoning. Use Luna for bounded read-only exploration and Sol for difficult failure diagnosis.
These model settings do not expand the workspace scope or grant additional permissions.

- Delegate bounded tasks when independent work can usefully proceed in parallel. Define the shared
  contract and explicit repository, checkout, and file ownership before delegation.
- Use up to nine concurrent subagents plus the primary manager, subject to the session's actual
  runtime limit. Nine is a ceiling, not a target or nine distinct roles: several subagents may use
  the same role for independent tasks. Do not create nested agents to evade the limit.
- Never run two writers in one checkout. Use separate assigned repositories or worktrees for
  concurrent implementation. Research and review remain read-only.
- The reviewer checks the original requirements and independent expected results, not just agreement
  between the implementation and its tests.
- After writing finishes, the verifier runs `./scripts/check-all.sh --check`. It reports failures
  without formatting or editing tracked files; ignored build artifacts and caches are allowed.
- Run at most one complete gate or heavy runtime suite at a time across this workspace. Agent
  concurrency is not permission for competing builds. The primary owns integration, the final
  complete gate, and every authorized Git or GitHub write.

`./scripts/check-all.sh` formats by default; `--check` runs the same complete gate without source
formatting. `./scripts/format-lint.sh` has the same modes, defaults to `--fix`, caps Clippy at two
jobs unless `BOXFERRY_LINT_JOBS` is set, and runs no tests. Any edit invalidates complete-gate
results. Neither mode grants release, publication, or deployment authority.

## Code discovery

For code discovery, try an available codebase-memory graph first, then CodeGraph only with an
existing usable index; never create one without user authorization. If neither answers, use `rg`
and targeted reads. For literals, configuration, scripts, and docs, start with `rg`.
