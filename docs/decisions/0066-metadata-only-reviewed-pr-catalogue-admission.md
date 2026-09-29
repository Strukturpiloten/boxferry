# ADR 0066: Admit reviewed application catalogue metadata without changing execution policy

- Status: accepted
- Date: 2026-09-29
- Supersedes: [ADR 0065](0065-trusted-reviewed-pr-application-validation.md), decision 3's
  byte-identical catalogue requirement

## Context

ADR 0065 binds reviewed-PR application diagnostics to a trusted dispatcher revision. Requiring
the whole candidate tier catalogue to equal trusted main byte-for-byte also rejects changes to
descriptive source/target evidence and pinned revisions of the two Lens consumer tasks. Those
fields can be reviewed at the candidate head without changing which privileged application
command runs or its budgets. The evidence validator must nevertheless use the candidate's exact
catalogue bytes: its digest, source/target metadata, and Lens revisions are part of the evidence
contract.

## Decision

The dispatcher still runs only on trusted `main` for an open, same-repository, exact-head PR with
both actors admitted by the pinned helper. It still selects exactly one of Nextcloud, Paperless,
or Immich. Before candidate execution, trusted code parses and shape-validates both trusted and
candidate TOML catalogues, including closed IDs and array order. It compares their complete
parsed structures after normalizing only each task's `sources` and `targets` to trusted values,
and the `revision` of the two unselected Lens consumer tasks to trusted values. Changed Lens
revisions must be full lowercase 40-character Git SHAs. Every other field, including task
selection, commands, privilege, tools, environment, budgets, tier deadlines, gaps, losses,
runtime claims, repositories, and schema paths, must equal trusted main. The runner and evidence
schema remain byte-identical to trusted main. Nonregular or oversized catalogue files fail
admission.

The trusted helper constructs a verifier tree with the trusted runner and schema at their
canonical paths and the exact admitted candidate catalogue bytes at its canonical path. The
application job validates bounded evidence with that verifier before artifact upload. Its final
aggregate separately rechecks the current PR actors/head, fetches the exact candidate, repeats
catalogue admission, reconstructs the verifier, and requires successful evidence against it.
This preserves candidate catalogue digest and metadata binding without letting candidate code
define its own evidence validator. The selected application command still runs from the exact
reviewed candidate and retains ADR 0065's privilege, resource, cleanup, and privacy limits.

## Consequences

Reviewed metadata and Lens revision changes can accompany application fixes in one diagnostic
dispatch. A policy change to the runner, schema, task command, budget, selection, or other
protected catalogue field still requires a trusted-main bootstrap before application dispatch.
This diagnostic remains separate from complete PR, main, and Release validation.
