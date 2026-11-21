# ADR 0101: Retire the zero-age-delta bootstrap

- Status: accepted
- Date: 2026-11-21
- Supersedes: [ADR 0097](0097-read-only-zero-age-delta-bootstrap.md) for new shared workflow revisions

## Context

The zero-age proof required one of two registered, open repair pull requests at its exact broken
base. Both repairs merged. The current callers establish eligibility through the ordinary
exact-base receipt route. Active candidate records and scoped unknowns are recorded in the
[#552 lifecycle audit](https://github.com/Strukturpiloten/boxferry/issues/552).

## Decision

New shared waiter revisions require the trusted-base service receipt whenever the event requires
dependency admission. The workflow checks the tested candidate, derives the authority from the
exact event base, verifies the immutable checkouts, and uses the authority's reader capability.
Contract 2 binds the receipt to the expected base. Contract 1 keeps its original invocation for
historical readers. A failed or missing receipt prevents the required job from succeeding.

Retain the existing candidate hint and admission outputs. The historical proof implementation
output stays empty in the new workflow. Complete non-admission events retain their
`not-applicable` result. Historical pinned workflows, the proof source, regression fixtures,
and receipt and recovery records remain available.

## Consequences

Current consumers stay on their reviewed immutable pins until ordinary adoption. Existing
in-flight and held runs continue to use the revision recorded for each run. Producer admission,
caller adoption, and successful service runs are separate evidence.

## Alternatives

Keeping the proof in newly admitted workflows would keep executing it for candidates outside the
closed repair registry. Widening the registry would introduce a new eligibility policy decision.
