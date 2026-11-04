# Releases

Release-plz prepares release PRs. After merge, the protected Release workflow publishes
the lockstep crates, tag, checksums and GitHub release.

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

Release requires fresh complete CI and catalogue evidence for one immutable candidate.
Main pushes and manual CI also run completely; ordinary PRs may use reviewed change-aware
selection. Required CI rejects unexpected skips and missing results.

The catalogue retains one bound binary, at most four workers, budgets, privacy, cleanup and limitations. Every non-success blocks publication, including failed, timed-out,
cancelled, missing or skipped prerequisites. Focused or historical diagnostics cannot replace
current-run evidence. Artifact replacement keeps partial retries within that run.

Each GitHub attempt independently meets admission budgets. Deadline accounting excludes inactive
retry waits. Attempt intervals use RFC 3339 instants and cannot overlap. See the
[evidence contract](../fixtures/conformance/migration-readiness/README.md).

Reviewed PR application diagnostics bind the exact open PR and candidate catalogue, recheck both
actors before and after, and require successful persistence. Record the URL, SHA, task, and bounded
JSON ([ADR 0066](decisions/0066-metadata-only-reviewed-pr-catalogue-admission.md)). Docker consumer
diagnostics additionally require staged exact-pair authorization and current-attempt verified and
removed Box receipts ([ADR 0079](decisions/0079-native-test-runtime-selection.md)). Infrastructure
alone proves neither native acceptance nor readiness
([ADR 0078](decisions/0078-trusted-docker-consumer-validation.md)).

[Issue #549](https://github.com/Strukturpiloten/boxferry/issues/549) tracks shared CI and release
rollout. Each repository retains its own thresholds and native suites, and Lens products remain
independent of BoxFerry. A shared workflow does not establish consumer or release readiness.

## Release notes

Release notes come only from `CHANGELOG.md`. The aggregate runs its validator as a dedicated job
required by the aggregate gate. Yank a version only when it is unsafe or unusable.

Record pending changes under `Unreleased`. Only release-plz creates numbered sections,
dates them or bumps lockstep versions. It inserts a heading with blank lines,
assigning notes to that release once.

Both validator modes require one dated, usable numbered section matching the workspace version.
Local/ordinary CI selects `--development`, permitting pending notes without publication.
Release-plz PRs and Release (including `validation_only`) select `--release`, requiring empty
`Unreleased` regardless of tags. No arguments default to release; invalid arguments fail.

## Publication order and recovery

The workflow publishes crates in manifest dependency order. Normal, build, and development
dependencies must all be available on crates.io before a dependent package is prepared.

The publication helper skips package versions already visible on crates.io. After correcting an
interrupted release, remove only the unpublished release tag and rerun the workflow from the
corrected default branch. Never move or remove a tag belonging to a published GitHub release.

## CLI binary assets

The binary workflow tests six [targets](platform-support.md) at the exact revision
and run. 64-bit lanes execute natively; i686 and ARMv7 use explicit QEMU user-mode execution.
Version, capabilities and Compose → Quadlet → Compose checks precede packaging. Collection and
current-run verification require every target; missing or failed evidence blocks publication.

Archives contain the executable, LICENSE, README and BUILD-INFO. All archives share SHA256SUMS and attestations. Apple Developer ID signing/notarization is not implemented.

Initial provenance: `docker.io/tonistiigi/binfmt:qemu-v10.2.3-68` at
`sha256:400a4873b838d1b89194d982c45e5fb3cda4593fbfd7e08a02e76b03b21166f0`.
GPL-licensed QEMU is verification-only, never redistributed. The helper and Renovate own updates;
this provenance is historical. Lens and website workflows have no CLI-asset
consumer. Configuration grants no publication authority.
