# Inert cancellation provider probe (#632 / #629)

This fixture observes GitHub scheduling. It sends no recovery, status, retry,
label, queue, publication, or runtime request. Source admission does not authorize
execution or establish production recovery safety. The three workflows stay
manual/call-only. A separately authorized operator controls the bounded series;
the paired manual package is `python -m boxferry_ci.provider_probe`. It has no automatic
poll loop, retry or production classifier. The [future procedure](provider-probe-procedure.md)
requires separate explicit authorization after source admission; do not execute it now.

## Source and identity custody

Preflight must confirm no other probe series is active and leave current consumer
gates and writers alone. Stop as inconclusive if main moves or run correlation is
ambiguous; do not dispatch compensating runs.

Record immutable default-main source S, all three workflow Git blob IDs/modes,
workflow ID/path/name, and the uploader SHA/tag before execution. All five runs
must use that same S and one operator-owned 32-character lowercase hexadecimal
series. `expected_source_sha` is S, not a moving ref. Bind A–E to unique positive
run IDs, attempt 1, exact repository and workflow, event, actors, ref and head SHA.
Verify actual caller and callee source from S. Local reusable calls resolve from
the caller's commit; record absent referenced-workflow metadata as absent.

The sole mutex is the caller's fixed probe group; series values cannot create
parallel locks. Preparation holds for 240 seconds only in A/D. The inert dependent
uploads an entry witness before holding for 240 seconds only in E. Runner jobs
have six-minute limits. Artifacts expire after seven days; copy exact archive
bytes and verify readback hashes into durable #549/#629 evidence custody first.
The witness is source-produced correlation, not job-attribution authority or a
production checkpoint. Independently authenticate its provider provenance.

## Exactly five first-attempt cases

1. **A:** Dispatch A. Positively observe the preparation job and its fixed hold
   step in progress; bind exact job/check/step identities. Observation does not
   start the hold or establish its remaining duration.
2. **B:** While A is positively holding, dispatch B. Capture B positively pending
   and its complete current-attempt job/check inventories. Never manually cancel B.
3. **C:** After B is pending, freshly reread A's bound hold immediately before
   dispatching C. Observe B becoming terminal cancelled, then freshly reread A's
   **same** hold step still in progress **after B's terminal observation**. Preserve
   both A snapshots. An ended or unobservable hold makes the burst inconclusive;
   do not estimate remaining time or add replacement runs. Let A/C finish naturally;
   both must succeed with preparation before dependent entry and verified witnesses.
   Without a provider cancellation-reason field, report a controlled sequence,
   not an independently authenticated cancellation cause.
4. **D:** After A/C terminate, dispatch D. Positively observe its preparation hold,
   then issue one ordinary cancellation. Capture an authenticated terminal
   unsuccessful preparation role and absent/skipped dependent with no contradictory
   entry. Missing artifacts alone prove nothing.
5. **E:** After D terminates, dispatch E. Persist preparation success in exact
   job/check records plus verified artifact. Observe dependent entry/upload success
   and its hold in progress. Snapshot preparation success again before issuing one
   ordinary cancellation. Reread the same preparation IDs and artifact afterwards,
   preserving full inventories to detect new, replaced, or parallel role records.

Normal execution is exactly five dispatches and two ordinary cancellations D/E,
plus automatic B cancellation. No rerun, force cancellation, bulk control, or
retry-until-preferred-result loop. Stop on safety/identity failure. Before hard exhaustion, only freshly authenticated active runs belonging to this exact
probe series may use its existing one-shot cleanup, recording the series as aborted.
At 400 spent or 1800 seconds, no HTTP is permitted, including authentication or cleanup.
Never affect other work.

## Bounded observations and refusal rules

Use fixed-origin read-only Actions/checks/content access. Limit the entire series
to 30 minutes and 400 transport requests, including authentication, provenance,
pagination, redirects/downloads and terminal rereads. Poll at five-second cadence;
at transitions capture full inventories, during waits only necessary run/bound-job
records. Allow one bounded terminal reread per identity after at least five seconds.
No automatic retries or opaque download loops. Budget exhaustion is inconclusive.

Use exact-attempt job endpoints and enumerate all pages with total-count agreement,
unique positive job IDs, and a hard cap of 100 jobs/run. Keep actual placeholder,
null, and missing fields. Verify check API URLs at api.github.com for the exact
repository/check ID, details URL run/job linkage, S, and GitHub Actions app
15368 / github-actions. Preserve full raw dispatch/cancel responses, timestamps,
run/job/check/artifact snapshots, referenced-workflow metadata and omissions.

Classify **outer caller wrapper**, **expanded inner leaf**, and simultaneous,
new, or replaced records separately. Preserve IDs, names and external identities;
never join records only by display-name prefix. Missing authenticated wrapper/leaf
mapping stays unobserved. E has two distinct results: expanded successful
prerequisite retained; caller-wrapper result retention supported. A retained
successful leaf cannot pass the latter. Its fixed `caller_prerequisite_result`
records scheduler admission only. A new cancelled preparation wrapper alongside
an old successful leaf is contradictory/unsupported. Rewriting the same successful
prerequisite identity to failure/cancellation rejects the retention inference.

Download only exact enumerated probe artifact IDs. Verify run/name/expiry/size,
server and uploader archive digests, and file SHA256. Each ZIP must be at most
256 KiB, containing exactly one regular, unencrypted `witness.json` at most 4 KiB;
reject directories, links and extra members. Verify exact schema/source/series/
case/run/attempt/role bindings and inherited preparation identities. At most two
artifacts/run, ten/series. B/D need not upload artifacts. Cap job logs at 1 MiB and
series logs at 5 MiB; clipping means incomplete evidence. Exclude credentials,
private headers, environment dumps and unrelated API data.

Freeze a manifest of S, all three blob IDs/modes, uploader SHA/tag, input/run map,
every snapshot/archive/log hash and byte length, timestamps, case results and
omissions/refusals. Preserve raw and normalized comparisons together. Use separate
OBSERVED / SUPPORTED_NARROW_SHAPE / UNSUPPORTED / INCONCLUSIVE outcomes. Empty or
generic cancelled B metadata remains unsupported: zero runners, empty steps,
missing artifacts and elapsed time cannot establish a failed preparation role.
These observations establish no universal API guarantee, exactly-once effects,
reservation safety, recovery acceptance, or permission to clear historical fences.
Independent review precedes any promotion of provider evidence into a regression.

## Pin ownership and local verification

The external uploader SHA/exact release tag and hosted runner follow the reviewed
canonical declarations in `.github/workflows/shared-rust-ci.yml`; tests read that
metadata without importing or executing its workflow. Coherent reviewed upgrades
of the canonical declaration and both probes remain supported; isolated or
malformed pin/tag/runner drift refuses. Explicit `archive: true` preserves the ZIP
contract. Native `github-actions`
extraction and common preset GitHub Actions grouping/pin/approval rules own both
occurrences; the existing hosted-runner manager owns both literal runner labels.
Local same-commit calls add no remote revision pin or new manager. Tests and this
historical evidence guide are not update candidates. No Renovate definition or
consumer change is needed for this isolated BoxFerry fixture; other repositories
consume none of these probe workflows.

`automation/tests/test_provider_probe.py` exercises hostile identities and inputs,
closed execution operations, positive witnesses and fixed holds without waiting.
Focused checks are development evidence only; the primary must still run formatting,
complete local gates and independent review before normal protected admission.

## Canonical manual package and derivation

The tracked controller/observer derive from the repository-owned, authored v5 drafts
(controller SHA-256 `40d5343a79aadba9ead4ff00f094bbd53fc98d20c94e8d2f7296fc7a61d8619a`;
observer `c5b81b9fefa2b685cf3ec2d925bf60573c6a092aa14fae73ae9d965407694006`).
They use fixed package imports and an independently reviewed exact admitted-S descriptor,
not archive imports, an arbitrary helper path or a future self-commit literal. The descriptor
binds held package/workflow Git bytes and modes, uploader/tag and reviewed source-CI identity;
local Git operations cannot lazily fetch. Old custody schemas refuse rather than migrate.
Normal commands cannot operate on copied custody; E's durable readback copy is local read-only.

Initialize before any experiment HTTP at zero requests and a fresh immutable clock/registration.
The same charged transport performs the finite 14-GET authentication/source/catalog/CI preflight.
Local credential retrieval is not an HTTP observation; a conservative precharge remains spent
if it fails before send. Seeded preflight counts and manual charge commands are absent.
One persisted three-read reservation per case covers normal and diagnostic discovery. General
inventory reads cannot bypass it. Refusal aborts normal control; diagnostic correlation is
non-authorizing and cannot bind or resume a case. Every discovery result persists bounded
static phase/code, before/after/delta global and per-case counts, safe captured identity facts
and evidence references, including failures before list success or after exact binding.
The nine literal-site structural metadata guards and both actor-loop sites have fixed codes;
unknown exceptions remain static.

The v5 role/phase/custody guards remain: positive A holds, authenticated B pending/replacement
sequence, successful A/C provenance, positive failed D prerequisite, full pre-cancel E preparation
attribution and durable cut, then post-cancel effect corroboration. Successful leaf retention
never proves a separate caller wrapper. Typed JSON-boundary annotations and new-schema/source
binding replace draft constants; they grant no new effect or recovery authority.

Offline tests exercise actual package calls with transport disabled/mocked, the privacy-reviewed
[later B subset](tests/fixtures/provider-probe-discovery/), request/page/hop reservations,
closed identities, abort/quota/deadline failures and inherited v5 archive/role/privacy guards.
The original workflow fixture tests remain unchanged. The later metadata is not the missing
original B refusal body. Historical series `9f5732bf0d63400381f463243184af6d` remains
ABORTED/INCONCLUSIVE (74/400, 17m10s, actual three B inventory reads versus internal two,
only A/B, natural completion, no cancellation/C/D/E); its first refusal cause is UNKNOWN.
The package and procedure neither reproduce that refusal nor authorize another experiment,
production recovery, allowance/fence resets, publication or closure of #629/#655/#453.

A non-mutating schema/helper/source/directory precheck precedes lock creation, with revalidation
under lock. No-follow directory descriptors protect custody registration, lock and file writes;
old or symlinked custody is refused without mutation. A durable incomplete-invocation marker
precedes observations/controls. Any metadata, IO or interruption failure aborts normal control;
if abort persistence fails, the marker forces abort on later load. Diagnostic/cleanup reads
never clear the permanent abort flag or reset spent counts.

For manual terminal waits, `observe capture-terminal` saves unique active run samples without
job/artifact inventory reads. The first completed run response is reused for its complete first
snapshot, then exactly one spaced stable snapshot is allowed. Fixed canonical terminal labels
bind only completed captures; no predictive timing, duplicate run GET, overwritten raw sample,
third terminal read or new allowance is needed. See the future procedure for exact commands.
