# ADR 0092: Shared development automation in BoxFerry

- Status: accepted; consumer rollout tracked separately
- Date: 2026-11-03
- Amends: [ADR 0001](0001-project-boundaries-and-origin.md) and
  [ADR 0032](0032-future-native-lens-boundaries.md) solely for development tooling
- Supersedes in part: [ADR 0056](0056-change-aware-development-validation.md)'s
  repository-local Python implementation, automatic full selection for every
  mixed known impact, and PR base-to-head changed-path comparison
- Builds on: [ADR 0079](0079-native-test-runtime-selection.md)

## Context

The Rust repositories repeat formatting, compiler, audit, API, and release
orchestration while retaining different product tests and native evidence.
Copying those mechanics causes version and validation drift. The maintainer
selected BoxFerry as the canonical shared automation home; no new repository or
write scope is needed. Lens product dependency independence remains mandatory.

Change-aware selection already distinguishes trusted comparison policy from the
tested candidate. Reusable workflows add a third revision: the shared automation
source. GitHub's caller workflow context does not identify the reusable callee's
source, so conflating those commits could execute or attest the wrong scripts.

## Decision

1. BoxFerry owns the locked Python development package, shared Rust workflows,
   and shared setup/release components. Lens and website checkouts consume them
   only as CI/development dependencies. Product Cargo graphs stay independent.
2. Callers bind the exact tested candidate, independently trusted policy revision,
   and canonical automation revision explicitly. Full immutable workflow refs and
   automation checkout pins must match. PR and merge queue narrowing reads the
   trusted base policy and compares that base with the actual tested candidate,
   rather than assuming a PR head describes its tested merge. The original head
   remains separately recorded evidence. Main/manual/release validation and the
   complete local pre-publication gate remain complete.
3. Common Rust mechanics use bounded typed inputs and validated argv task data.
   Repository-specific feature boundaries, package tests, thresholds, native
   suites, binaries, budgets, cleanup, and evidence remain with their product.
   Shared orchestration must not turn an absent policy into fewer checks.
4. `Required CI` retains its stable caller-visible check. Its aggregate accepts
   only successful selected jobs and explicitly planned skips. Missing, failed,
   cancelled, and unexpectedly skipped results fail closed. Mixed narrow changes
   retain the exact union of prerequisites only when every constituent impact is
   independently known and safe to narrow. Classifier, policy, unknown, unsafe,
   and unrecognized changes still select the complete plan. A prior narrow PR
   result never authorizes a release.
5. Release preparation is automatic under the existing repository triggers.
   Shared release validation requires every configured repository-specific job
   to succeed and identify the exact release candidate. The protected caller
   release environment and explicit approval still gate publication. A bounded
   shared crate component validates the complete typed package inventory and
   normal/build/dev order, refreshes credentials per crate, publishes idempotently,
   and waits for canonical registry visibility. Product-specific tag/draft,
   binary, attestation, and GitHub release hooks retain their existing guards.
6. Validation-only mode has no authentication or mutating publication path.
   Publication is disabled by default in the typed policy. Previously unpublished
   products remain validation-only until separately authorized. Initial public API
   compatibility requires positive contract evidence and verified canonical
   absence; an arbitrary 404 must not create success. Existing #485 owns first
   publication parity rather than a competing exception.
7. Routine tool and dependency pins have one automatic Renovate owner, explicit extraction/integrity,
   producer/consumer regressions, and rollback to the prior reviewed immutable
   revision. Distribution and merge occur automatically after required checks as
   each consumer adopts the verified shared policy. This replaces manual
   propagation of the newly shared definitions;
   unrelated manual-update requirements retain their existing ownership.

### Initial infrastructure pin exception

The first reusable automation version has no published release tag. A reviewed,
complete-gate-tested infrastructure commit may be consumed by its full SHA with
an honest bootstrap provenance comment. This narrow exception replaces the
otherwise required exact release-tag comment only for that unreleased internal
automation source. It does not authorize publishing a tag, release, software,
or site; inventing a tag/comment/checksum; using floating `main`; or weakening
protection and approval rules. Existing external Action pins keep exact release
tags and integrity. A later authorized tagged version can replace the bootstrap
through the same coordinated pin update.

### Explicit trusted task opt-in (2026-11-09)

Shared Rust callers may set the typed `require_configured_task` input to `true`.
After shared-source admission and locked Python setup, but before Rust setup,
Cargo metadata or coverage-tool installation, the read-only `check-task` CLI
requires an explicit task in the trusted policy. It verifies the exact tested
checkout, immutable automation source and trusted policy bytes using their
separate supplied revisions. It runs no task commands and emits no successful
task-execution receipt. Final execution repeats this strict selection and
identity verification with `run --require-configured-task`.

Strict selection preserves configured argv order, coverage thresholds and task
timeout; missing or malformed tasks refuse without reconstructing Cargo aliases
or defaults. The input defaults to `false`: legacy callers retain the existing
trusted-alias quality fallback and MSRV defaults. This additive contract does
not change snapshot acquisition, dependency-age policy or infrastructure bootstrap.

No software version, integrity pin, extraction location or Renovate manager changes
are needed for this interface. Existing native Action ownership and shared-source
pin managers remain authoritative. Consumer adoption is separate: consumers may
opt in and advance matching references only after an actual admitted immutable
producer successor and distribution have been observed. This decision does not
claim that rollout or native/App acceptance has completed.

## Consequences

Common fixes and version upgrades have one reviewed source. Consumers still need
their own complete gates and evidence; one successful repository is not rollout
completion. Old trusted policies require an explicit complete-behavior fallback
until new definitions are available. The shared package itself gains strict
Python formatting, type checking, tests, lock integrity, and failure-contract
coverage, without becoming a runtime product dependency.

The caller-owned release hooks remain deliberate extension boundaries. This ADR
does not claim those hooks are shared, native acceptance has passed, unpublished
products are publishable, or any publication/deployment is authorized.

## Alternatives considered

Another repository was rejected because BoxFerry was the maintainer's selected
home and the authorization scope is already sufficient. Copying workflows was
rejected because it preserves drift. A single generic native suite was rejected
because provider, Podman, Quadlet, Docker, and application evidence are distinct.
A floating shared branch or fabricated release tag was rejected because neither
provides the required reviewed immutable source binding.
