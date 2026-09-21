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
4. Run `Release` on that exact default-branch SHA. It runs the complete deterministic and
   pre-release validation contract before the protected publication job can begin.
5. Verify crates.io packages, checksums, tag, GitHub release, and installation.

Use `validation_only: true` to exercise the identical validation path without tags, GitHub
releases, crate publication, release credentials, or another mutating publication step. Manually
dispatched full and focused migration-readiness artifacts remain diagnostic only.

## Automatic release validation

`Release` owns fresh current-run pre-release evidence. It runs the complete deterministic
CI workflow and the complete conformance catalogue for the immutable candidate. CI, default-branch,
and Release validation therefore share one deterministic task definition, including macOS
portability and offline documentation links. Conformance uses the same reusable definition as
manual diagnosis; the one-build/four-worker cap, exact-SHA/binary binding, budgets, privacy,
cleanup, and explicit limitations remain unchanged. Failed, timed-out, cancelled, missing, or
skipped prerequisites block the always-running release-validation result and publication.
Historical or focused evidence cannot be substituted for the current Release run. Artifact names
are stable within that run and replaced per task, so partial and complete retries cannot combine
another run's evidence or deadlock on an earlier successful worker.

Aggregate deadline accounting uses active worker intervals, so a later retry cannot consume the
execution budget solely through its inactive wait after retained successful jobs.
Release evidence records each positive GitHub attempt separately; every attempt must independently
meet the pre-release admission budget. Attempt intervals are parsed as RFC 3339 instants and must
remain chronologically non-overlapping.

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
