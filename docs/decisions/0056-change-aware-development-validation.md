# ADR 0056: Change-aware development validation with a complete release override

- Status: accepted
- Date: 2026-09-26
- Builds on: [ADR 0047](0047-bounded-migration-readiness-tiers.md) and
  [ADR 0055](0055-parallel-readiness-and-local-quality-feedback.md)
- Supersedes in part: ADR 0055's statement that every protected pull-request check runs the
  complete deterministic workload; [ADR 0022](0022-sole-quadlet-parser-and-deterministic-test-contract.md)'s
  macOS test lane; and [ADR 0024](0024-linux-cli-and-wsl-on-windows.md)'s decision to retain it

## Context

The complete local and hosted deterministic gates take minutes even for prose edits. Moving the
same set of checks into more jobs would not lower total runner work or make a smaller development
machine more usable. Public documentation also contains executable CLI examples, so a broad
Markdown exemption would lose real behavior coverage.

## Decision

Use a single versioned, repository-owned validation plan for local feedback and pull-request job
selection. The initial reduced plans cover public prose and public executable documentation.
Every other change, mixed impact, classifier or policy change, missing comparison, or unknown path
selects the complete plan. Exact pull-request base and head commits determine changed paths; the
tested workflow revision is recorded separately. The classifier and policy used to narrow a pull
request come from its trusted base. The first rollout, whose base has no classifier, runs all jobs.
Its workflow-inlined bootstrap plan and aggregate do not execute candidate classifier code.

The always-running `PR gate` verifies the plan and every controlled job result. Selected jobs must
succeed; only explicitly unselected jobs may be skipped. A failed, cancelled, missing, malformed,
or unexpectedly skipped prerequisite fails closed. The prose plan runs repository file checks and
offline links without Rust compilation. The executable-documentation plan additionally runs the
existing independent CLI documentation-example suite. Code, dependencies, fixtures, schemas,
configuration, workflow definitions, acceptance policy, and unknown paths retain full checks.
Fenced, indented, inline, and HTML code blocks are treated as executable documentation; a
new command must not escape the focused suite merely because it lacks a fenced block.

The complete `scripts/check-all.sh` remains the local publication gate. Every `main` push, manual
CI dispatch, and reusable CI invocation from `Release` selects the complete plan. Release also
retains fresh, exact-revision native and application pre-release evidence; a prior PR's narrow
result cannot authorize publication. Local narrow tasks are development feedback, not complete
publication evidence. Coding agents keep the complete local pre-PR gate required by `AGENTS.md`.
The complete gate and local planner require an explicit `CARGO_TARGET_DIR` inside the current worktree;
otherwise a shared target can retain binaries with paths to a removed or moved worktree. The
default worktree-local Cargo target remains valid, and download caches remain reusable.

Validation is Linux-only. The previous macOS check/test job duplicated Linux Rust checks and is
removed; no macOS runner or hardware is required. This does not alter Linux CLI support, Windows
use through WSL2, defensive platform guards, or handling of Windows-authored input. macOS
client-side compatibility remains an intention without tested evidence. Compiling components on
other platforms is incidental and unverified, not a new support claim.

## Consequences and alternatives

Narrow PRs use fewer hosted jobs, while the aggregate check name remains stable for branch
protection. The planner and its tests add a small always-on cost. We chose a conservative first
allowlist over broad path globs; additional plans need measured benefit, independent semantic
coverage, and the same fail-closed regressions. We did not alter full native/application release
thresholds or add deployments to ordinary PRs.
