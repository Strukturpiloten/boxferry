# ADR 0065: Trusted exact-head application diagnostics for reviewed pull requests

- Status: superseded by [ADR 0066](0066-metadata-only-reviewed-pr-catalogue-admission.md)
- Date: 2026-09-29
- Builds on: [ADR 0047](0047-bounded-migration-readiness-tiers.md),
  [ADR 0055](0055-parallel-readiness-and-local-quality-feedback.md), and
  [ADR 0057](0057-run-owned-podman-live-storage.md)

## Context

The complete `Release` path validates a default-branch candidate before publication. Its
`Migration readiness` dispatcher deliberately refuses non-default-branch native execution. A
reviewed application change may nevertheless need fresh Nextcloud, Paperless-ngx, or CPU-only
Immich evidence at its exact pull-request head before it is merged. Neither a local run, a prior
head, a skipped persistence check, nor an offline policy test establishes that evidence.

## Decision

1. Add a separate `workflow_dispatch` diagnostic workflow, selectable only on trusted `main`,
   with an open same-repository PR number, its full current lowercase head SHA, and one closed
   application task choice. Do not alter the existing default-branch `Migration readiness`, PR,
   main, or Release guards. Distinct task dispatches may run in parallel, but each job runs at
   most one application under its existing catalogue and host budgets.
2. Use DockerLens's canonical fail-closed reviewed-PR admission helper at one immutable checkout
   revision. The workflow supplies the literal BoxFerry repository policy; the helper checks
   repository ID/name, base `main`, head repository, exact current SHA, and current write-level
   permissions of both original and rerun actors. Admission is repeated in the application job
   before candidate checkout and by the final trusted aggregate. A moved/closed PR, inaccessible
   API, unauthorized actor, or changed head is failure, not a waiver.
3. Keep admission and final validation code outside the candidate checkout. The exact candidate
   is checked out separately without persisted credentials, built with `--locked`, and run through
   `migration-readiness.py` with one catalogue task and exact revision. Reuse its provider
   installer, bounded resource/deadline checks, run-owned cleanup, and sanitized evidence schema.
   Compare the candidate evidence schema byte-for-byte with trusted main before execution. Reject
   non-file, symlink, or over-128-KiB evidence before parsing; stage a bounded plain JSON copy
   outside the candidate checkout, and use the trusted-main validator on that staged copy before
   uploading it. A structurally valid failed run may remain diagnostic, but malformed evidence is
   never uploaded. The application job rejects a failed run, and an always-running trusted
   aggregate applies `--require-success` and rejects
   failed, skipped, cancelled, stale, or missing task evidence. Never upload native captures or logs.
   Before candidate scripts run, require the catalogue and outer readiness orchestrator to match
   the trusted dispatcher revision byte-for-byte. A change to either safety boundary needs its own
   trusted-main bootstrap first. This does not make reviewed candidate application code harmless:
   privileged live scripts still need exact-head human review and retain their bounded cleanup.
   This same-job trusted checkout is not a sandbox against a malicious root-running candidate;
   the final aggregate validates in a separate job with its own trusted checkout.
4. The immutable reviewed DockerLens #42 merge commit is unreleased CI infrastructure, not a new
   software release. No helper version tag or downloadable-asset checksum exists; do not invent
   either or claim BoxFerry hosted success before its own consumer dispatch. Exactly one custom
   `github-digest` Renovate manager owns the canonical `ADMISSION_REVISION`; updates require
   Dependency Dashboard approval, security review, and no automerge.

## Consequences

The workflow can provide independently reviewable, exact-PR-head application diagnostics on a
hosted disposable runner without weakening normal branch protection or granting release,
publication, deployment, or merge authority. Its evidence is not a complete Release gate. Hosted
runtime success must be observed separately; workflow syntax and offline counterfactual tests do
not establish Nextcloud, Paperless, or Immich migration success. A runner or nested rootless
network-helper failure remains a failed application until corrected under existing safety limits.
