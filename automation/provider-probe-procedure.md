# Future manual provider experiment procedure

**NO EXECUTION AUTHORIZATION.** This is a reviewable procedure for a later, separately
explicitly authorized series. Source admission, this document, and offline tests do not permit
initialization of live custody or any experiment API request, dispatch, cancellation or rerun.
Keep #629, #655 and #453 outstanding. Production recovery, committed reservations, 500-request /
480-second limits, ages, integrity rules, recovery fences and allowances remain unchanged.

## Required review before a new series

The primary first freezes the exact tracked package and procedure, obtains independent source
review, formatting, complete primary and check-only gates, protected PR/queue/main admission,
and required shared-source distribution. A separate authorization must identify one new series,
operator, exact admitted main source S, successful complete source-CI run, durable-custody
location/storage limits and the reviewed failure/continuation policy. Default on an unmet case
prerequisite is STOP; there is no improvised diagnostic continuation into D/E or resumed control.

Use only the four canonical `boxferry_ci.provider_probe*` modules and executed package `__init__.py` at S. Never import historical
`target/` or `/tmp` helpers. Prepare an already-held exact S checkout and all Git objects, and an
already-installed locked Python environment before the series. During the series use its
explicit interpreter with `UV_OFFLINE=1`; if invoking uv, use `uv run --offline --no-sync --locked
--project automation python`. No dependency acquisition, Git fetch, LFS, smudge, textconv, pager,
credential helper or lazy fetch may accompany source checks. The local reader disables Git
replacement refs, lazy fetching, protocols, optional locks, hooks and global/system configuration;
unavailable objects, symlinks, executable modes or differing on-disk bytes refuse locally.

From the held admitted checkout, locally generate a source descriptor with:

```console
python -m boxferry_ci.provider_probe describe --source-sha S --main-ci-run MAIN_CI_RUN
```

Here and below uppercase identifiers are review-time values, not literal command arguments.
Retain the exact descriptor bytes and SHA-256. Independently verify S, all five implementation
blob IDs/modes/SHA-256, all three probe workflow paths/blob IDs/modes/SHA-256, the uploader's full
SHA/exact release tag, caller workflow ID/path/name and same-commit local reusable calls. Verify
main-CI repository/source/event/default branch/status and the operator's exact public account
IDs. The descriptor is a reviewed closure, not a new floating source pin or a future self-commit
literal. Select S/MAIN_CI_RUN from already-retained admission evidence; every fresh remote
verification belongs to the charged preflight. The code verifies held package/workflow bytes
before HTTP, then authenticates
remote source/catalog/CI through the charged preflight. No pre-series API helper is permitted.

## Fresh zero-start custody and accounting

Choose one fresh 32-character lowercase hexadecimal series. Use new owned local and durable
paths, not either historical 94-file custody copy. Before any experiment HTTP:

```console
python -m boxferry_ci.provider_probe init --custody CUSTODY --series SERIES --source-sha S --source-manifest DESCRIPTOR --descriptor-sha256 DESCRIPTOR_SHA256 --workflow-id WORKFLOW_ID --actor ACTOR --actor-id ACTOR_ID --triggering-actor ACTOR --triggering-actor-id ACTOR_ID
python -m boxferry_ci.provider_probe control --custody CUSTODY preflight --main-ci-run MAIN_CI_RUN
```

Initialization is local: exclusive series registration, fresh custody, immutable start/boot clock,
requests 0 and an empty transport journal. It rejects old custody, seeded counters and manual
charge commands. Local `gh auth token` retrieves credentials; it is not an HTTP observation and
its token is never printed or stored. A conservative reservation remains spent if credential
retrieval fails before send. The successful finite preflight performs 14 charged GETs: user,
repository, main head, probe workflow, CI workflow, exact CI run, source commit, three trees,
three workflow blobs, and probe inventory. It verifies no active probe. Refusal aborts normal
controls; do not repeat preflight, reset its clock or seed another counter.

All HTTP uses one custody ledger: preflight, ordinary/diagnostic inventory, exact run/job/check
reads, every page, each redirect/download hop, terminal rereads and POSTs are durably reserved
before transport. Reservations are distinct from observed responses; failed or ambiguous sends
are spent. Keep 400 reservations / 1800 seconds total, five-second endpoint cadence and at most
one spaced terminal reread per identity. Normal dispatch needs elapsed <1200 and spent +2 <=373;
normal D/E cancellation needs elapsed <1200 and spent +3 <=373. The 27 remaining requests are
headroom within the same 400, not another pool. No retry, refund, redispatch or deadline extension.

Every case has three shared inventory reservations, including explicitly diagnostic reads and
failed reads. Each discovery emits/persists global and per-case before/after/delta/remaining,
fixed phase/refusal code, safe expected/observed identity fields and bounded evidence references.
Malformed list costs one global reservation; successful list plus exact read costs two. A fourth
inventory read makes no HTTP request. General observer inventory routes cannot bypass this cap.
Diagnostic correlation is evidence-only: it cannot bind a run or resume normal control.

## Exact A–E command sequence

Commands are one invocation at a time; there is no automatic polling or retry loop. Observe
cadence without estimating hold time. Use `observe snapshot --custody CUSTODY --case CASE --run
RUN --label LABEL` for explicitly needed active inventories (for example B-pending and
E-inventory-before). For terminal waits, use the bounded capture path below. Use `observe job` with those arguments plus exact
`--job JOB_ID --check CHECK_ID` for authenticated job/check/step observations. Inspect preserved
complete inventories to select IDs by closed role shape, never by prefix. Record observed
wrappers and expanded leaves separately. Any unknown/parallel/replaced identity refuses.

For each accepted dispatch, run `control --custody CUSTODY dispatch --case CASE`, then at most
three `control --custody CUSTODY discover --case CASE` attempts with proper cadence. Bind only its
unique first-attempt run from the recorded dispatch window. A NOT_YET result permits only a
remaining discovery read, never another dispatch. After refusal, the optional explicitly
reviewed `diagnose-discovery` command uses the same cap, sets aborted state and grants no binding
or continuation. Do not fetch a hidden diagnostic inventory with gh/curl or a manual charge.

1. **A:** Dispatch/bind A. Authenticate preparation's expanded leaf job/check and the exact fixed
   hold step in progress as `A-before-B`. No root pending status, elapsed duration or witness alone
   authorizes B. The observation records an interval, not remaining hold duration.
2. **B:** While the same A hold is freshly authenticated, dispatch/bind B once. Save `B-pending`
   with complete inventories and positively pending root state. Never manually cancel B.
3. **C:** After B is pending, read the same A hold as `A-before-C`, then dispatch/bind C once.
   Capture B first terminal cancelled inventory as `B-terminal` using capture-terminal; immediately authenticate A's
   same run/job/check/hold-step still in progress as `A-after-B`. Preserve ordered snapshots and
   run `observe guard-burst --custody CUSTODY --before A-before-C --after A-after-B --pending B-pending --victim
B-terminal`. An ended/unobservable hold makes the burst inconclusive. Let A/C finish naturally.
   Use capture-terminal to bind complete `A-terminal`, `C-terminal` and their sole >=5-second
   `-terminal-stable` inventories. Authenticate every exposed job/check after each inventory, and retain source/
   uploader/actual ZIP evidence with inherited preparation linkage. Generic cancelled B without
   a positively authenticated failed preparation role stays UNSUPPORTED, not no-effect proof.
4. **D:** Only after successful A/C and intact burst prerequisites, dispatch/bind D. Authenticate
   preparation hold as `D-hold`, then invoke `control --custody CUSTODY cancel --case D --run D_RUN`
   once. Use capture-terminal for first/stable terminal inventories (`D-terminal`, `D-terminal-stable`) and exact
   failed/cancelled preparation job/check as `D-preparation-terminal`; dependent must be absent
   or skipped without contradictory entry. Missing artifacts alone prove nothing. E cannot
   compensate for unsupported B/D or proceed after a hard abort.
5. **E:** Only after D prerequisites, dispatch/bind E. Follow the mandatory phase split below.

For every terminal wait use the manual capture command, with a new unique sample label on
each invocation:

```console
python -m boxferry_ci.provider_probe observe capture-terminal --custody CUSTODY --case CASE --run RUN --label CASE-sample-01
```

While active it makes only the necessary exact run GET, retains that immutable raw sample and
returns NOT_YET_OBSERVED. With remaining original budget, wait >=5 seconds and invoke once with
a new label. This is a new bounded observation, not a retry of failed transport or control.
On the FIRST completed run response it reuses that same authenticated response (no second run
GET) to collect complete jobs/artifacts and bind `CASE-terminal` to the retained snapshot. After at least five seconds, the next unique-label capture performs the sole permitted stable
reread and binds `CASE-terminal-stable`. A third terminal capture refuses before HTTP. Existing
raw samples and canonical snapshots are never overwritten. Active→first complete→stable costs
1+3+3 requests for one-page empty inventories; every additional page/hop remains charged in the
original 400.

Do not poll-run into completion and then attempt two snapshots, spend the canonical label on an
active sample, predict provider completion timing, or backfill missing first-terminal evidence.
An earlier terminal observation without a complete captured first snapshot, incomplete pages,
unknown role, changed identity or refused request aborts; no additional terminal GET can repair it.

## E pre-cancel proof and separate post-cancel corroboration

First collect `E-inventory-before` containing both unique leaf roles and both enumerated artifacts.
Authenticate every exposed wrapper separately. Preparation must be completed-success; obtain its
completed log with `observe log` using exact run/job/check and `--e-phase E_PRE_CANCEL_PREPARATION
--label E-prepare-log`. Review the exact uploader step number and actual line span containing its
source uploader SHA, artifact ID and sole archive digest. Bind it with:

```console
python -m boxferry_ci.provider_probe observe uploader-provenance --custody CUSTODY --log-label E-prepare-log --artifact PREP_ARTIFACT --uploader-digest PREP_SHA256 --step-number PREP_STEP --line-start PREP_FIRST_LINE --line-end PREP_LAST_LINE
python -m boxferry_ci.provider_probe observe artifact --custody CUSTODY --case E --run E_RUN --label E-prepare-archive --artifact PREP_ARTIFACT --uploader-digest PREP_SHA256
```

Verify actual bounded archive bytes and witness/source/series/run/attempt/preparation bindings.
Do this before the final effect-hold observation. Pre-cancel effect-entry metadata is a unique
ID/digest only; do NOT request an active effect log or manufacture its uploader provenance.
Authenticate effect's validation-success, entry-upload-success and fixed hold in progress in
strictly ordered distinct steps as `E-effect-hold`. Then authenticate the same preparation success
as `E-prepare-pre-cancel`, finishing in that order after the inventory. Authenticate wrappers
exposed by the inventory after it too. An exposed preparation wrapper must succeed; an exposed
effect wrapper must still be active without conclusion. Missing wrappers remain unobserved.

Run `observe guard-e --custody CUSTODY --prepare E-prepare-pre-cancel --effect E-effect-hold` to
freeze the single `pre_cancel_cut`. Preserve actual preparation provenance/ZIP and the unique
effect-entry metadata in it. Copy state and every indexed pre-boundary file to separately
reviewed durable custody, fsync files and directory, and verify byte lengths/hashes/readback.
Provide `storage-attestation.json` matching the controller's closed schema
`boxferry.provider632.storage-attestation.v5`: current series/S,
observer/controller hashes, cut SHA, `root_reviewed: true`, storage class
`operator-reviewed-durable-filesystem`, and the literal limit
`fsync-and-readback-only; no WORM or power-loss guarantee`. Storage review must be true;
ordinary runner disposal or a hash alone cannot establish durability.

Invoke the single `control --custody CUSTODY cancel --case E --run E_RUN --durable-custody DURABLE`.
The controller rechecks source, full copied proof, file/directory fsync/readback, active owned
run and <=5-second hold freshness. A slow copy or API/fsync sequence refuses without extending
freshness. Do not replace a cut, refresh its missing facts after cancel, or retry ambiguous control.

After actual cancellation, use capture-terminal to bind `E-terminal` and `E-terminal-stable`, with exact same cancelled
effect job/check and retained successful preparation, authenticating every exposed role after
each inventory. Use `E-effect-terminal`, `E-effect-stable`, `E-prepare-terminal`, and
`E-prepare-after` for leaf observations. Only now collect the completed cancelled effect log
with `--e-phase E_POST_CANCEL_EFFECT`; bind its exact uploader span and verify the SAME entry
artifact enumerated before cancellation, including inherited preparation ID/digest/marker and
scheduler-success field. No final record can backfill a missing pre-cancel fact.

Run `observe compare-e --custody CUSTODY --prepare-before E-prepare-pre-cancel --prepare-after
E-prepare-after --inventory-before E-inventory-before --inventory-after E-terminal-stable --artifact
PREP_ARTIFACT`. Missing final corroboration leaves the cut/cancellation spent and E incomplete.
Report separate effect cancellation, leaf retention, verified effect provenance and caller-wrapper
retention. Without a distinct authenticated retained preparation wrapper, the maximum result is
OBSERVED_LEAF_RETENTION_ONLY / CALLER_WRAPPER_UNOBSERVED; never full E/fixture/production success.

## Stop, custody and independent acceptance

Source movement, custody/identity mismatch, ambiguous correlation, incomplete pages, contrary
wrapper, refused evidence or spent control aborts normal work. Natural completion is default.
Only before hard exhaustion may `control ... cleanup --case CASE --run RUN --reason REASON`
ordinarily cancel an already-dispatched/bound exact active owned run, with unused cancellation
key, fresh authentication and original remaining budget. Reasons are source/identity/budget/
evidence/deadline. At 400 spent, 1800 seconds, invalid clock or inability to reserve durably:
**NO HTTP, including authentication or cleanup.** Record active/unknown runs locally; no force
cancel, rerun, compensating run, unrelated control or external cleanup budget.

Retain <=100 jobs/run with total-count/unique-ID agreement, <=2 artifacts/run/10 series,
<=256 KiB ZIP exactly one regular unencrypted witness.json <=4 KiB, <=1 MiB/job log,
<=5 MiB series logs, <=1 MiB JSON response and <=64 MiB aggregate custody. Clipped, missing,
expired or contradictory evidence stays INCONCLUSIVE/UNSUPPORTED. Freeze source/implementation/
workflow/uploader/input->run map, complete reservation/response counts and times, diagnostics,
raw/normalized own-case metadata, logs/ZIP bytes, actual server/uploader digests, local SHA-256/
lengths and independent durable readback. Exclude credentials, private headers, signed URLs,
environment dumps and unrelated bodies. Independently audit all five cases before promoting facts.

Keep historical series `9f5732bf0d63400381f463243184af6d` ABORTED/INCONCLUSIVE: 74/400 requests,
17m10s, B's three actual inventory reads versus the old internal two, only A42505913795 and
B42506013197, natural completion, no cancellation/C/D/E. Its first B refusal remains UNKNOWN.
The later B fixture is not that missing body. Never reuse/rebind/backfill either immutable
94-file copy, restore discovery/effect allowances, clear fences or infer generic cancellation
proved required roles. New observations cannot establish universal REST guarantees, exactly-once
effects, production committed reservations or later automatic recovery/queue acceptance.
