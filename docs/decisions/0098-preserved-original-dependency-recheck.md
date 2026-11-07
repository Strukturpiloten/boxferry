# ADR 0098: Original dependency recheck before fresh protected queue validation

- Status: proposed; implementation and admission remain separate
- Date: 2026-11-07
- Builds on: [ADR 0095](0095-bounded-older-base-dependency-recovery.md), issue #613

## Context

GitHub may retain a PR merge `O` with ordered parents `[P,H]` after main advances to `B`.
The strict current-candidate resolver correctly refuses that merge. Forcing a rebase can change
lock resolutions and restart dependency holds. The maintainer approved preserving versions,
rechecking the original graph/base, then independently validating the fresh protected queue.
ADR 0095 remains proposed history; this proposal changes only the retained-original sequence
and the finite static event exclusions below.

## Decision

Keep live PR, `still_current` and queue candidate guards unchanged. Only an affirmative,
stable stale-graph proof selects a separate historical binding. It requires an unchanged owned
Renovate source, exact ordered parents, original-base ancestry, matching closed workflow
catalogs/policies and the same admitted immutable reader authority. The source fork `M` may predate tested `P`;
its authenticated comparison and ancestry bind only Renovate source custody. Its catalog/policy
must match the same closed set; reports and the waiter still use `O/P`, never `M`. Unknown, malformed,
conflicted, pending, closed or moving inputs cannot authorize recovery.

The initial mode reuses ordinary Cargo/npm manifest and known lock-file ownership proof.
Locked Python updates permit only exact dotted package pins at existing supported declaration
locations in root or automation pyproject files. Each changed name needs one unique canonical
old/new same-project uv-lock identity with valid hashes; an unrelated lock change is insufficient.
Build requirements must mirror a changed locked normal/dev pin. Metadata, includes, markers,
extras, source settings and tool/compiler constraints stay fixed; other syntax refuses.

It evaluates inert `O/P` metadata with normal integrity, publication-time and first-seen rules.
Third-party direct/transitive updates retain 72 hours; the whole attributable first-party Lens
update exception remains unchanged. No branch, manifest or lock resolution is rewritten.

The hint locates `O`; it cannot prove the actually executed `X`. Reports identify only `O/P`.
Only the original immutable waiter can consume its exact `X/P/A` receipt and permit the
original authentic Required CI aggregate to succeed. A different valid hint may waste the
finite retry, but cannot approve `X`, source `H` or a future queue candidate.

Completed durable preparation and a later completed matching receipt may authorize one
existing original-run `rerun-failed-jobs` request. Fresh source, target, provider, original
attempt, protection and checkpoint reads precede it. Existing normal retries share the same
allowance. Base churn cannot reset consumed or uncertain budgets. Timeout, cancellation,
failed or missing history, changed attempts and incomplete artifacts keep their existing
fences; no ambiguous POST is repeated.

Original green permits only normal admission into the protected ALLGREEN queue with build
and merge limits one. A real fresh combined candidate must independently pass its complete
reviewed gate and current candidate/base age/integrity checks. Historical reports cannot
replace that evidence. The service neither enqueues nor merges, and changes no protections.

## Static event exclusions and compatibility

Only three closed-source non-policy job shapes can be skipped on an authenticated original
PR event: DockerLens's dispatch diagnostic and push-only native matrix, and the website's
push-only publisher. Exact observed names, literal conditions and aggregate branches must
match. The native aggregate still requires skipped native jobs on PR/queue and successful
native jobs on main push. Unknown, missing, duplicate, failed or cancelled jobs refuse.
There is no expression interpreter, generic matrix expansion or trusted candidate plan artifact.
Dynamic planned skips, including image selection `false`, remain unsupported.

Expected-base reader contract 2, immutable source closure, existing scopes and workflow inputs
suffice. The original base authority must equal the service source. A legacy reader cannot
become capable retroactively or accept a receipt relabelled under another authority. Only
existing genuine authority/workflow transitions retain the one fixed native Renovate rebase
label fallback; ordinary stale graphs do not trigger it. A fallback may change resolutions
and restart the ordinary hold. The limited feature updater's behind-base policy is unchanged.

## Consequences and evidence

Focused authored and recorded regressions cover source binding, exact static skip shapes,
wrong hints, ledger union, drift and transport ambiguity. Recorded Docker Rust-failure and
website-success runs prove presentation only and remain ineligible recovery examples.
Complete primary/independent gates, exact-source admission and coherent consumer adoption
remain required. Genuine registry evaluation, automated post-maturity original retry and
subsequent fresh protected queue success are separate acceptance evidence, not established
by source or mocked tests. Release/publication approvals remain unchanged.
