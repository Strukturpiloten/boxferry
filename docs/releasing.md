# Releases

After a release-plz PR merges, the protected Release workflow publishes lockstep crates,
tag, checksums and GitHub release.

## Commit classification contract

- Release-worthy code uses `feat`, `fix`, `perf`, `refactor`, or `revert`.
- Non-release work uses `docs`, `test`, `ci`, `build`, `style`, or `chore`.

Breaking pre-1.0 changes use `!`, a minor version and migration notes.

## Maintainer flow

1. Merge release-worthy code after local and hosted gates pass.
2. Review the release-plz PR, lockstep versions, internal requirements, and changelog.
3. Merge the release PR.
4. Run `Release` on that exact default-branch SHA. It runs the complete deterministic and
   pre-release validation contract before the protected publication job can begin.
5. Verify crates.io packages, checksums, tag, GitHub release, and installation.

`validation_only: true` uses the identical validation path without tags, GitHub releases,
crate publication, release credentials, or other publication mutations. Manually dispatched full
and focused migration-readiness artifacts remain diagnostic only.

## Automatic release validation

Release requires fresh complete CI and catalogue evidence for one immutable candidate.
Release and validation-only Release explicitly select published API compatibility against
an exact canonical registry version, using the actual chosen candidate manifest version.
Breaking commit subjects cannot waive an insufficient version. The aggregate requires
exactly one explicit `published_api_candidate_sha` for the exact candidate; a development
result or generic candidate output cannot substitute. The shared gate exposes that separate
validated published-API binding only after every prerequisite succeeds. [Development API checks](automation.md#api-comparison-purpose)
use trusted Git baselines for ordinary changes.

Main pushes and manual CI run completely; PRs may use reviewed change-aware selection.
Required CI rejects unexpected skips and missing results.

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

Release notes come only from `CHANGELOG.md`. Their dedicated validator job is required by the
aggregate gate. Yank a version only when unsafe or unusable.

Record pending changes under `Unreleased`. Only release-plz creates and dates numbered sections
or bumps lockstep versions. It inserts a heading with blank lines, assigning notes once.

Both validator modes require one dated, usable numbered section matching the workspace version.
Local/ordinary CI selects `--development`, permitting pending notes without publication.
Release-plz PRs and Release (including `validation_only`) select `--release`, requiring empty
`Unreleased` regardless of tags. No arguments default to release; invalid arguments fail.

## Publication order and recovery

The workflow publishes crates in manifest dependency order. Normal, build, and development
dependencies must all be available on crates.io before a dependent package is prepared.

The helper skips package versions already visible on crates.io. After correcting an interrupted
release, remove only the unpublished release tag and rerun from the corrected default branch.
Never move or remove a published GitHub release tag.

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
