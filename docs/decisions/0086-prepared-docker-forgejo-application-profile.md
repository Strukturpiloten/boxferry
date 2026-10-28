# ADR 0086: Prepared Docker Forgejo profile and offline source contracts

- Status: accepted for test-only source preparation and hermetic contracts;
  complete integration gates and native/application qualification remain separate
- Date: 2026-10-28
- Supersedes only: [ADR 0083](0083-bounded-docker-target-consumer-preparation.md)
  insofar as its ownership/consequences clause defers this prepared #366 Forgejo
  test-only profile and its offline contracts; all other module deferrals and
  qualification boundaries remain
- Preserves: [ADR 0078](0078-trusted-docker-consumer-validation.md) exact-pair
  admission, [ADR 0079](0079-native-test-runtime-selection.md) staged bootstrap,
  and [ADR 0085](0085-docker-cli-neutral-routes.md) product non-execution

## Context

The full Docker application integration goal needs a reusable Forgejo profile
before a separately admitted inner runtime can execute it. Existing shared
Forgejo behavior probes and independent authored graph/plan/sidecar contracts
provide that source seam. Preparing it does not qualify a Docker target,
application migration, acquisition route or current source pair.

The dedicated hermetic suite measures 275.571 seconds locally for 76 controls and
28 representative boundary/error classes. Adding that work to always-on file
quality or the shared application-probe aggregator would serialize it with
documentation and run it on prose/example-only pull requests unnecessarily.

## Decision

1. Prepare the source-only test profile and its explicit caller-owned transport
   contract. It applies independently validated emitted requests through the
   isolated test boundary; BoxFerry's product remains non-executing. Image
   provisioning, external prerequisites, test starts, finite deadlines, readiness,
   exact ownership and recovery remain test-harness responsibilities.
2. Reuse the existing health/admin/Git/SQL semantics and canonical fixtures/pins.
   Preserve the full two-service graph, excluded peer, actual HTTP and SSH Git
   behavior, exact database row, retained-volume recreation, active bounded peer
   isolation controls, loopback ingress and protected-value checks. Native
   execution still needs a separately reviewed transport and independent guard.
3. Register only `bash scripts/test-docker-forgejo-application.sh` as canonical
   hermetic source contracts: once in the complete local gate and as the separate
   parallel `docker-forgejo-application` CI job with a ten-minute host backstop.
   Add its schema-2 policy job key without changing shared planner semantics or
   the existing prose/executable-documentation subsets. Full, unknown/code/mixed
   or failed-comparison PR plans, main pushes, dispatch and reusable Release CI
   select it; focused documentation plans retain an explicit planned skip.
4. Require the aggregate's exact selected-job and needs contracts, including both
   full/all-jobs first-rollout lists. Failed, cancelled, missing or unexpectedly
   skipped selected work remains a hard failure. Release already reuses complete
   CI through `workflow_call`; it needs no separate duplicate registration.
5. Keep the real Docker application dispatcher and admission scope unchanged.
   Preserve `core-reacquire-expectations.json` as historical evidence and the
   empty current runtime map. Small-core, current exact-pair/native prerequisites,
   compatible published dependencies and actual route-specific application
   acceptance remain required before any migration claim.

## Ownership and dependency review

### Exact trusted-base transition

The reviewed main cohort at `b571269393e105e3938302df39dea13f76bc836b` already
has the schema-2 classifier and ten-job policy. It cannot verify an eleven-job
aggregate with its older exact-needs set. In both selection and aggregation, the
existing full/all-jobs bootstrap additionally accepts only this exact pair of
trusted-base file hashes:

| Trusted base file                | SHA-256                                                            |
| -------------------------------- | ------------------------------------------------------------------ |
| `scripts/validation-policy.json` | `2c2b99cc7e5e1baf3ad00a6f1ab27a2f9e3036e0fa8a19a90d4cf7271a7806b7` |
| `scripts/validation-plan.py`     | `df2f7fa2adfc0002bd3984c91e72ba8e1412ced36e7f3acd7003960242412417` |

Both files must match. An unknown or modified base does not activate this
exception; the normal trusted path retains its exact dependency checks and
fails closed on incompatible needs. Candidate policy contents, schema number or
job count are not authority. The bootstrap requires all old ten jobs, the new
job and validation-plan to succeed, with exact plan and needs sets.

These constants are immutable historical transition evidence, not software
versions or Renovate-managed pins. The existing legitimate absent-base bootstrap
is preserved. Once main carries the eleven-job policy, its changed policy hash
naturally leaves this branch and uses the ordinary trusted classifier/verifier.
Retire the narrow exception when old-base transition support is no longer
needed; future classifier changes must review or retire its fixed-byte tests
rather than copying a divergent historical classifier implementation.

### Canonical owners

This is a BoxFerry-owned application contract, not a shared planner change or a
Lens dependency. ComposeLens, DockerLens, PodmanLens, QuadletLens and the website
need no consumer edit. The six-suite integration goal and actual all-route
acceptance remain open; preparing Forgejo does not reduce them.

The new job reuses the existing checkout alias, its full-SHA/exact-tag action pin
and disabled persisted credentials. Its unchanged `ubuntu-24.04` runner is covered
by the existing workflow-path `github-runners` regex owner. Actions retain their
native manager; no duplicate manager, software/image/provider pin, digest,
checksum, approval or grouping change is introduced. The profile reads existing
canonical image references rather than declaring new versions. No container
engine, credentials, secrets or software installer is added to the hermetic job.

## Evidence and consequences

The suite verifies literal phase order, exact argv/emitted-operation binding,
ownership and name/ID drift, partial creation, active-isolation classifications,
privacy, deadlines, cancellation and cleanup with inert callbacks. Equivalent
transport error classes run once; this is not exhaustive prefix replay. The
measured local duration is source-test cost, not a Docker workload budget.

Full-plan CI runs the job independently of file quality, adding a parallel task
whose duration may affect the critical path. The complete local gate remains
sequential and complete; neither focused checks nor formatting replace it. The
primary owns final integration review and the complete gate before Git writes.
Future changes invalidate their affected evidence and require renewed review.

No native runtime result, application acceptance, deployment, release or
publication follows. Native probes observe a bounded reachability window; only
the isolation probe containers have read-only roots and dropped capabilities.

## Alternatives

Deferring all reusable source until native admission couples independent work.
Registering this suite in file quality would slow focused documentation plans.
Treating mocked transport success as migration acceptance, replacing emitted
operations with an independent fixture, weakening the application graph, or
expanding real runtime admission from offline results is rejected.
