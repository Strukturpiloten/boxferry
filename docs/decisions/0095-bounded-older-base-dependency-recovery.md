# ADR 0095: Bounded dependency recovery across target-base movement

- Status: Proposed; producer code may be admitted before consumer activation.
- Date: 2026-11-04
- Related: [ADR 0092](0092-shared-development-automation.md),
  [ADR 0094](0094-official-devcontainer-feature-updates.md), issue #566.

## Context

A dependency PR can retain the same source head H while its target advances from
P to B. GitHub can retain the original tested merge X for an old CI run while
REST identifies that run by H and the live PR supplies a new merge N. Repeatedly
rebasing every behind branch creates extra CI; blindly retrying an old run or
assigning N's eligibility to H would confuse different tested graphs.

## Decision

The existing equal-base retry remains unchanged. Older-base recovery uses the
trusted default-branch listener, never candidate commands or a dispatcher.
A bounded artifact names an untrusted hinted merge O and base P for one original
run/attempt. It establishes no producing-job provenance and cannot establish X.
The service independently requires O's direct parents, in order, to be exactly
[P,H], verifies P on current target history, and verifies N's parents [B,H].

Retaining the old graph requires the base-selected authority A to equal current
trusted service implementation C. Tracked manifests, policies, complete workflow
catalogs at P/O/H/B and the closed canonical reader implementation must agree.
Both the actual immutable caller reader and A must support the expected-base
contract. Legacy reader arguments remain unchanged; unknown declarations fail.
Unsupported workflow shapes refuse recovery rather than execute candidate YAML.
Both older-base modes conservatively refuse any skipped job in the original CI
attempt: this implementation has no independently authenticated selected-skip plan.
A legitimately planned skip therefore remains outside supported recovery until
that additional proof exists; the ordinary equal-base retry is unchanged.

The normal dependency evaluator checks O/P and N/B separately, including all
integrity, operational and age policy. Reports and dependency statuses identify
their own exact merge and base. The actual old waiter still consumes only X/P/A;
a forged O different from X cannot approve that waiter. The service never writes
Required CI or substitutes a success on source H.

Recovery requires a current mandatory ALLGREEN queue with build/merge limits one,
its full combined-candidate gate, effective Required CI rules, and read-back
ruleset bypass facts. An omitted rule integration ID is recorded as an any-app
rule, not described as server-side provider binding. Separately, the exact
original run and attempt must contain a unique fixed Required CI job whose
fixed-origin check-run URL resolves to GitHub Actions app 15368, slug
github-actions, and the run's actual REST head. Explicit wrong providers,
integration bypasses, incomplete metadata and unsupported shapes refuse recovery.
This mechanism changes no repository protection settings and uses no admin path.

A completed trusted artifact first persists separate reports and a prepared
reservation. A later completed receipt can authorize one logical effect, keyed
by original run, unchanged source H and authority C across base churn. Effect-time
run/attempt, queue, provider and PR reads precede the request. A timeout never
causes an HTTP retry. Increased attempts, consumed reservations, failed service
history, missing artifacts or incomplete bounded history suspend the budget.
This is conservative crash reconciliation, not a claim of exactly-once HTTP.

The canonical immutable commit C's committer date E is a self-enforced effect
floor, not source-creation provenance or dependency age. Every normal retry and
new handoff refuses actors whose server creation time predates E. Clean genesis
requires complete bounded history from E; it cannot reset lost earlier evidence.
Healthy completed same-C checkpoints then move the inclusive history window
forward, retaining all original budgets and uncertainty across a 72-hour hold.
A checkpoint must bind its server run attempt and must have both creation time
no later than the current actor and a strictly smaller run ID. Older queued
actors cannot act or mint clean authority. Every later success, failure,
cancellation and earlier attempt is reconciled; only provably unstarted queued
jobs are harmless. Active possible effects defer the frontier until closure.
An authenticated first-attempt skipped outer reusable call is also no-effect
history only when complete jobs/artifact metadata proves the known caller,
no runner, steps, labels or artifacts. Placeholder timestamps are not dispatch
proof; this exception provides neither receipt nor checkpoint authority.

A genuine authority/workflow transition can request one native Renovate `rebase`
label handoff. The label must already exist with its fixed name; the service does
not create labels, select candidate-controlled labels, remove/readd labels,
close/reopen, merge, or install a permanent keep-updated label. Label history reads only label/unlabel events since the original CI run, requiring
both pagination flags to prove completeness; unrelated force-push noise is excluded.
Label presence or prior label effects consume the handoff; requesting a label is not successful
recovery. Any fresh graph still receives ordinary age, integrity and CI checks.

A separate native no-hint transition uses only current N/B eligibility. Complete
artifact metadata must prove true hint absence; malformed or conflicting present
hints never select this branch. Authoritative compare B...H supplies merge-base M;
H must preserve M's native workflow catalog and manifest presence, mode and bytes.
An old manifest may be present and does not establish old P/X/A. B must actually
adopt C's new reader, so B equal to M refuses before adoption. Exact owned ordinary
Renovate changes, original-run provider/policy-step failure, current pass and a
later completed matching current receipt precede the same bounded fixed-label
request. Comparison commits and independently read tip require the Bot author and
either the same Bot committer or verified/valid GitHub web-flow signature. No old
candidate, eligibility report, status or original-run rerun is created. New head
and fresh CI remain necessary; requesting a label is not successful recovery.

The common preset explicitly couples `rebaseLabel: rebase` to the trusted service
constant and has no permanent keep-updated label. Consumer overrides must remain
coherent during adoption; candidate settings never choose the write target.

The common preset may use native Renovate `rebaseWhen: auto` with these coupled
proofs and regressions. Native Renovate retains unchanged branches behind a
merge queue, still rebases conflicts and preserves queue/automerge/pruning
behavior. The separate limited DevContainer updater retains its explicit
behind-base policy. The 72-hour hold, complete first-party Lens exemption,
fixed qualification targets and release approvals remain in force.

## Evidence and activation

Focused tests cover wrong/missing base receipts, legacy argv, ordered parents,
forged O versus actual X, all seven prepared consumer reader/gate shapes, actual
run/job/check provenance, immutable source/archive closure and finite effects.
The native regression executes unmodified Renovate 44.139.0 modules from the
integrity-pinned runtime recorded in `automation/renovate-runtime.json`; its
platform effects are mocked. Command:
`BOXFERRY_RENOVATE_PACKAGE=<verified-runtime> PYTHONPATH=automation/src python3 -m unittest discover -s automation/tests -p test_recovery_renovate.py`.
Renovate is AGPL-3.0-only; no oracle source is copied or redistributed by this
change. Existing runtime provenance and integrity records remain authoritative.

Producer admission alone does not activate consumers. Each final consumer must
adopt the immutable listener/reader/preset together, grant only trusted-listener
checks:read and pull-requests:write, and validate its exact candidate. The
pre-adoption BoxFerry local behind-base override and its assertion remain until
that later adoption. Missing proof remains an actionable refusal; rollout and
complete publication gates are independently verified by the primary agent.
