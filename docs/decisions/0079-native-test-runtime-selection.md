# ADR 0079: Native test runtime selection

- Status: accepted policy; implementation and native qualification pending
- Date: 2026-10-21

## Decision

GitHub Docker native tests use outer Docker. Podman and Quadlet tests use outer
Podman. Local development remains Podman-only, including isolated Docker guests.
This changes test infrastructure, never product execution or conversion boundaries.

Select the backend explicitly with no fallback. Runner Ubuntu, outer engine and
inner daemon/version/root mode are separate facts. Verify the actual outer engine;
a runner label or Docker-compatible API is not exact Docker version evidence.
Pinned inner images, candidate/helper identities, assertions, original deadlines,
resource limits and exact-owned cleanup remain mandatory. A launcher switch does
not repair or qualify earlier failed OOM evidence by itself.

DockerLens owns the canonical Docker test setup/launcher contract; BoxFerry consumes
its reviewed source without copying native product behavior. No Lens product
depends on BoxFerry. Keep engine-specific inspect, mount, storage and cleanup
semantics explicit rather than translating command prefixes.

Preserve each existing isolation boundary: BoxFerry/DockerLens own their inner
daemon, socket and data volume; their outer runtime may be the explicitly verified
runner daemon. Never mount its socket into a guest. The containers producer has a
stronger private outer-store contract and must preserve it separately.

## Rollout and consumers

The bounded BoxFerry volume-consumer prerequisite explicitly binds the canonical
DockerLens Unix-HTTP helper by held bytes, clean reviewed revision and SHA-256,
before runtime setup. It does not adopt the unfinished #366 application/capture
modules or migrate historical curl readiness. Catalogue extraction remains
data-only. Live consumer profiles separately bind the canonical fixed outer
command bridge and its complete six-file runtime/observation/presence/profile
source closure (including `native-runner-apparmor.py` and
`native_outer_diagnostic_capture.py`) into
a private owned snapshot. Caller deadlines, watchdog and held resource identity
remain authoritative; the historical readiness comparison remains Podman-only.
New consumer lane schema 2 independently binds reviewed helper source identity
and whitelisted fresh outer preflight observations. Neither a helper hash nor
outer-version observations qualify the inner Engine or applications. Historical
volume proofs and evidence are unchanged; native qualification is still pending.

The reviewed hosted Docker volume lane uses the same canonical narrowly scoped
user-namespace prerequisite for `upstream-rootless`, with a distinct BoxFerry
workflow/job identity, owner label, guest socket, profile prefix and custody
directory. Native Lens modes and receipts remain unchanged. The Box receipt
validator is read-only and rejects native, cross-source and cross-attempt
receipts; the native validator rejects Box receipts. Other lanes and local or
self-hosted execution do not acquire this exception. See ADR 0078 for lifecycle,
custody and schema-2 acceptance requirements.

[BoxFerry #469](https://github.com/Strukturpiloten/boxferry/issues/469) coordinates
DockerLens #127, QuadletLens #157 and containers #442. Update local, admitted PR,
main and Release consumers together; retain current gates until replacement is
verified. Native evidence and complete gates remain separate requirements.

PodmanLens already uses Podman. Compose provider configuration checks and website
checks need no daemon switch; planned Compose runtime lanes remain unimplemented.
Review pins, integrity and Renovate ownership in each affected change. Preserve
historical evidence. Exact-pair authorization and all ten admissions under ADR
0078 remain mandatory. Its credential-isolation split uses fresh trusted
App-bearing jobs and reusable candidate workers with no forwarded stored secrets.
Require the real nested caller context and fixed `lanes` job identity before any
candidate build; the native handoff also requires the current-attempt trusted
binding within its 120-second admission budget. Queue delay or context mismatch
fails closed and needs a new attempt, never a fabricated runtime context.
Use the dispatcher's `context_only: true` route to establish the real hosted
nested-call behavior before native qualification; it receives no stored App
secret and executes no candidate code. Every normal run repeats that same smoke
as a prerequisite to its first admission.

## Safe staged bootstrap and acceptance checkpoints

These are rollout requirements, not delivered native acceptance. The manual
consumer executes `trusted/scripts/reviewed-docker-consumer-run.py` from the
workflow's main revision, not the candidate's runner. A #469 candidate cannot
exercise its new runner, closure policy or schema-2 validator through an older
trusted-main workflow. Changing the candidate SHA does not update that trust
anchor. Never run a candidate runner as trusted code or weaken exact-source
admission to bypass this bootstrap boundary.

1. Independently review and completely gate the bounded trusted infrastructure
   changes and the canonical DockerLens producer closure. Land the necessary
   trusted-main infrastructure through the normal workflow without claiming
   new native qualification. Keep existing PR/main/Release gates intact.
2. Resolve product prerequisites separately. BoxFerry main lacks the #343 product
   integration; any consumer requiring those changes remains unqualified until
   a separately scoped, approved and verified minimal product change exists.
   This runtime-policy change does not transplant the large #343 integration
   or adopt the deferred #366 application/listener/capture modules.
3. From updated trusted main, review and explicitly authorize the exact current
   open BoxFerry/Lens PR pair, source/lock identities, complete helper closure
   and policy. Infrastructure merges or changed product/helper bytes invalidate
   earlier candidate identities. Renew authorization instead of reusing an old
   receipt or treating a merged/closed PR as an eligible open candidate.
4. Dispatch the admitted pair on the verified hosted Docker prerequisite. Require
   all four fresh native volume lanes and their fresh aggregate, including actual
   outer Docker observations, independently bound source identity, inner lane
   evidence, cleanup and the unchanged complete gates. Missing, failed or skipped
   results leave this acceptance checkpoint pending. Do not reseal historical
   evidence or promote outer preflight facts to inner/application qualification.

## Runner prerequisites and Renovate

The Docker-only consumer checks hosted Docker/jq command availability without
apt installation, daemon startup or package replacement. Its canonical admitted
preflight, not that presence check, owns actual runtime verification. The narrow
hosted-runner prerequisite exception and Renovate no-change review follow below;
[dependency policy](../dependency-policy.md) retains canonical pin ownership. Other repositories and runtime
consumers require their own verified rollout; one consumer is not workspace proof.

This Docker-only consumer uses the GitHub-hosted Ubuntu 24.04 runner's existing
Docker and jq as bounded runner-tool prerequisites. This is an externally
provisioned, non-reproducible prerequisite exception, not a pinned Docker
installation: missing commands fail closed, and only the admitted canonical Lens
preflight verifies the local root daemon and records actual Docker client/server
and runner-image versions. Do not reinstall Docker/Moby, start a replacement
daemon or install unused Podman, skopeo or host libcap tooling here. Guest images
and their digests remain independently pinned.

Renovate no-change review: removing the former unpinned apt prerequisites changes
no Action SHA, image declaration or admission revision. The existing Actions
manager and single `ADMISSION_REVISION` custom manager retain their paths,
approval rules and ownership. No new pin, integrity claim or duplicate manager
is introduced for the hosted runner's preinstalled tools.

Containers image publication and package releases remain manual. Do not merge
a publication-triggering change without separate authorization or weaken publishing
rules to evade this boundary.
