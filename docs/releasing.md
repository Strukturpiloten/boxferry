# Releases

Release-plz prepares release pull requests. The protected release workflow publishes the lockstep
crate set, tag, checksums, and GitHub release after that pull request is merged.

## Commit classification contract

- Release-worthy code uses `feat`, `fix`, `perf`, `refactor`, or `revert`.
- Non-release work uses `docs`, `test`, `ci`, `build`, `style`, or `chore`.

Breaking pre-1.0 changes use `!`, a minor version, and concise migration notes.

## Maintainer flow

1. Merge release-worthy code after local and hosted gates pass.
2. Review the release-plz PR, lockstep versions, internal requirements, and changelog.
3. Merge the release PR.
4. Run `migration-readiness.yml` with `pre-release` on that exact default-branch SHA and retain its
   successful aggregate evidence artifact. Focused worker artifacts are diagnostic and never
   release evidence. The catalogue aggregate deadline leaves a margin before the workflow
   job timeout so failed or unfinished tasks can still be recorded and uploaded.
5. Run the protected release workflow for that same SHA. Publication is blocked unless the shared
   helper validates successful pre-release evidence whose embedded revision matches exactly.
6. Verify crates.io packages, checksums, tag, GitHub release, and installation.

## Planned automatic release validation

[Issue #310](https://github.com/Strukturpiloten/boxferry/issues/310) tracks the agreed replacement
for manual step 4 above; it is not implemented yet. Starting `Release` should automatically run
the complete deterministic gate and full pre-release conformance for the immutable candidate,
then publish only after every required job and aggregate evidence check succeeds. Earlier runs
or focused artifacts must not substitute for that release attempt's complete evidence. Failure,
timeout, cancellation, missing results, and unexpected skips must block publication.

Use one reusable conformance definition for Release and manual diagnosis. Preserve the four-worker
cap, exact-SHA/binary binding, budgets, privacy, cleanup, and explicit limitations. Supersede
ADR 0047's manual orchestration when implementing this change; retain ADR 0055's scheduling and
evidence contract.

[Issue #309](https://github.com/Strukturpiloten/boxferry/issues/309) coordinates common CI/release
validation, including portability, and Renovate-aware adoption across all five repositories.
Keep thresholds and native suites repository-specific. ComposeLens owns provider conformance,
PodmanLens owns API/replay conformance, and QuadletLens owns generator conformance; BoxFerry owns
full application migrations. The website keeps its own check/build/deployment gate.

Follow the [dependency-policy checklist](dependency-policy.md#workflow-changes-and-renovate-ownership)
for every shared definition change. Updating or testing release workflows does not authorize an
actual release or deployment.

## Release notes

Release notes come only from `CHANGELOG.md`. The PR gate runs its validator as a dedicated job
required by the aggregate gate. Yank a version only when it is unsafe or unusable.

Record user-visible changes in the single `Unreleased` section. Ordinary product pull requests
must not create a future numbered section, set its date, or bump the lockstep workspace version.
Release-plz inserts only the dated version heading, with blank lines on both sides; the curated
notes below `Unreleased` thereby become that release's notes exactly once. During release
preparation, the validator requires an empty `Unreleased` section and
one dated, usable numbered section matching the workspace version.

## Publication order and recovery

The workflow publishes crates in manifest dependency order. Normal, build, and development
dependencies must all be available on crates.io before a dependent package is prepared.

The publication helper skips package versions already visible on crates.io. After correcting an
interrupted release, remove only the unpublished release tag and rerun the workflow from the
corrected default branch. Never move or remove a tag belonging to a published GitHub release.
