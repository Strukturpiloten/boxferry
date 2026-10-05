# Offline Docker application contract prerequisite

## Closed volume-only fixture rehearsal

`scripts/docker-application-conformance.sh --profile volume-fixtures` is an
explicitly opt-in, test-only isolated-daemon profile. It does not run the six
applications, exercise persistence or container mounts, or establish fresh native
admission. It retains the existing four-lane clean DockerLens catalogue selection,
outer daemon/bootstrap/socket, two-CPU/4-GiB/512-PID limits, storage watchdog,
execution/cleanup deadlines, ownership checks and outer cleanup. Unlike the core
journey it never pulls/saves/loads the Busybox fixture, creates/starts a core
container, runs the core marker, or reacquires Docker-to-Compose output. The
existing core/replay validators and fail-closed core reacquisition oracle remain
separate and unchanged.

The producer is the facade's build-only example
`docker-volume-fixture-rehearsal`, not the CLI or a public runtime executor.
Candidate receipt schema 2 keeps its existing closed shape. The trusted profile
selects the exact build recipe, never a recipe proposed by the receipt:

```text
cargo build --locked --package boxferry --example docker-volume-fixture-rehearsal --no-default-features --features compose,docker --jobs 2 --config 'patch.crates-io.docker-lens.path="CLEAN_ABS_LENS"'
```

`core-journey` remains the default candidate-verification profile with its
existing CLI recipe. Cross-profile receipts, arbitrary commands, dirty/changed
Lens inputs, changed candidate/source/lock/binary bytes and noncanonical paths
are refused. The runner executes only its bounded owner-private binary snapshot,
passing exactly `--lane`, `--run`, `--prefix`, `--receipt-sha256` and
`--output-directory`. Run and prefix are lowercase tokens matching
`^[a-z][a-z0-9-]{0,63}$`, derived separately from the unchanged outer ownership ID.
The producer receives an empty mode-0700 directory. Its bounded console remains
private and is not echoed into diagnostics.

The output directory must contain exactly six `<id>-volumes.json` complete
schema-1 artifacts plus `manifest.json`, all mode 0600. The manifest's closed
fields are `schema: 1`, `scope: "volume-only"`, `lane`, `run`, `prefix`,
`candidate_receipt_sha256` and `fixtures`. Its six ordered fixture records contain
only `id`, `source_sha256`, `artifact`, `artifact_sha256` and `volume_count`:
Forgejo 2, Nextcloud 3, Paperless-ngx 6, Immich 4, observability 5 and Supabase 3.
The validator independently binds the original canonical Compose source bytes in
the selected BoxFerry checkout, literal volume name/owner suffixes and both full
labels (`io.boxferry.live-run` and `io.boxferry.application`). It does not learn
authority from manifest hashes or rewrite authored labels.

All six artifacts and all 23 requests are validated before any POST. Every
artifact must have the complete independently selected clean-Lens catalogue
context, empty prerequisites and only exact versioned volume-create POSTs with
closed `Name`/`Labels` bodies. Duplicate names, wrong context/evidence, additional
fields/files/requests, partial schemas and source/receipt/artifact drift fail
closed. Native JSON responses are bounded to 16 KiB, parsed privately and never
printed; diagnostics do not include raw bodies or exception text.

Each create requires successful native absence first; Engine's idempotent create
cannot reuse an existing volume. An atomic owner-private ledger records the exact
name and two labels before each POST, including a POST whose response fails or
times out. Fresh successful inspection must confirm the exact name and full
labels. Cleanup reads only this independently constrained ledger, requires a
successful fresh ownership inspection before DELETE, and verifies native absence.
Failed inspection stdout cannot authorize deletion. Wrong/unavailable ownership,
timeouts, cancellation, partial creation and failed absence never become success;
outer owned-resource teardown and residual reporting remain mandatory. A volume
apply attempt or existing ledger requires explicit successful inner cleanup;
an already absent outer daemon, missing ledger or successful outer/storage
teardown cannot replace inner absence proof. Unverified inner cleanup fails
closure and retains private ledger evidence with an uncertainty diagnostic.
Receipt, source and binary bindings are rechecked before apply and during closure,
including after cleanup. No retries, ambient resources or broader cleanup scope
are introduced.

The focused Python/fake regressions cover recipe confusion, source and manifest
binding, context and field injection, all-request-before-mutation refusal,
existing volumes, registered partial POSTs, deadlines, failed inspection,
ownership and cleanup. They do not establish real live volume compatibility,
six-application acceptance or the complete repository gate. No operational image,
tool, dependency pin or Renovate definition is added or moved: the canonical
DockerLens image assignments and their existing managers remain authoritative.

## Private volume-only closure proof

`volume-fixtures` requires `--evidence-directory`: an existing, empty, canonical,
root-owned mode-0700 directory outside both source checkouts and the disposable run
directory. Other profiles reject this option. The helper records the destination's
device/inode and the original disposable directory's device/inode before runtime
mutation. It rechecks directory identity through a no-follow directory descriptor;
replacement, symlink ancestry, stale files or wrong ownership/mode fail admission.
Core/replay behavior, runtime requests, limits and cleanup ownership remain unchanged.

The only final file is `volume-proof.json`, schema 1, scope `volume-only`, result
`checks-passed`, bounded to 16 KiB and created exclusively mode 0600. Its closed fields
retain the exact case-preserved outer run token, separate volume run/prefix, selected
lane/API, schema-2 candidate receipt digest and source/lock/binary/revision hashes,
the selected native-script/catalogue digests and reviewed target context, validated
observed versions, manifest digest, and six ordered fixture source/artifact digests
with counts 2/3/6/4/5/3. All 23 unique approved volume identities are SHA-256 hashes of
canonical JSON (`sort_keys=True`, compact separators) containing the exact native
`Name` and complete expected `Labels`; no raw labels or runtime response bodies survive.
Each has a distinct `removed` (identity-gated GET, DELETE 204, GET 404) or
`already-absent` (initial GET 404) cleanup outcome and verified absence.

Outer/storage `ownership_sha256` hashes bind canonical `{kind,name,run}` objects for
the exact derived container/storage names and run label. They are name/ownership
bindings plus positive exact-name absence, **not immutable-ID deletion proof**. The
disposable identity hash binds its original device/inode and run token. The approved
outer token permits an independent auditor to derive both exact native names without
retaining private paths. Concurrent replacement/transient state between observations
is not ruled out; none of these hashes expands deletion authority.

Sanitized input and inner-cleanup records remain bounded shell-held values while the
disposable files are removed. No passing proof is written until all 23 inner absences,
outer/container storage absence and disposable-directory removal have succeeded,
no catchable interruption was observed, and final candidate/receipt/native bindings
have been revalidated. Writing, fsync or readback failure fails the run after safe
teardown and revokes only the exact newly created proof inode. An interrupted shell
handoff uses the same private inode token. After the finalizer returns, the caller
first resets interruption traps to end the catchable interval, then checks the now
stable flag and revokes/fails before acknowledging success if it was set. This is
not immunity from signals after reset; replaced files are never deleted. The
proof survives disposable-directory deletion. SIGKILL, host failure and power loss
cannot guarantee trap cleanup or an acknowledged handoff; retained private evidence
requires review rather than automatic acceptance. This is volume-only development
evidence, not a full repository/publication gate, native capability qualification or
six-application acceptance.

Canonical consumers remain the opt-in `scripts/docker-application-conformance.sh`
volume profile and its `scripts/lib/docker-application-contract.py` helper. Offline
controls remain in `scripts/test-docker-application-contract.py`, registered once by
`scripts/test-application-probes.sh` and consumed by local `check-all.sh`, PR/main/
dispatch CI and Release's reused CI. No workflow/native consumer is silently opted in.
Renovate no-change review: none of the current custom-manager file patterns matches
these four changed paths; application pins remain in the six `images.tsv` catalogues,
Podman pins in its matrix, and Docker pins in the selected clean DockerLens script.
No package/tool/image/Action pin, extraction path, grouping, approval, historical
artifact or manager changes. This is static ownership evidence, not a Renovate run.

## Closed outer-resource presence checks

Container and storage-volume name preflight, initial cleanup selection and post-removal
readback use the same bounded `podman-presence` helper. It reuses the readiness subprocess
reader with both output streams combined under its existing 16-KiB cap, three-second read
deadline and owned-process-group teardown. Only a completed read with empty stdout and
stderr and native status 0 or 1 establishes `present` or `absent`, respectively. Warnings,
configuration errors (including native exit 1 with diagnostics), other statuses, output
overflow, timeout, cancellation and unverified termination remain `unknown`.
The reader now lives once in `scripts/lib/bounded-native-read.py`; this helper imports
it while preserving its existing diagnostic/presence call and mock boundary. The Podman
live harness reuses that primitive for separately typed local/Unix-socket queries.
Presence queries additionally require read-only process-group disappearance after
leader reaping, bounded to 250 milliseconds within the helper deadline. A surviving
group or lookup error leaves termination unverified. No group signal is sent after
reaping; conservative group-identity reuse cannot establish presence or absence.

The helper emits exactly one closed marker with matching status: `present`/0, `absent`/1
or `unknown`/2. The shell requires the exact marker, newline and status together; launch
errors, wrapper failures and malformed replies cannot impersonate absence. Existing
runtime and aggregate cleanup budget wrappers enclose the helper. Native output and
exception text never appear in the marker or harness diagnostics.

Unknown presence fails preflight or cleanup and retains the run-private evidence directory,
including storage-volume-only uncertainty. Cleanup still attempts later registered resources,
checks each exact run label before removal and positively proves absence afterward. It removes
only the run-owned temporary image archive when evidence must remain. Successful cleanup
preserves the original failing exit status; no unknown outcome establishes acceptance.
Offline regressions exercise actual fake-native diagnostics through the helper and shell,
strict marker parsing, preflight and both cleanup phases, bounds and cancellation. They do
not invoke a native runtime or supply live compatibility evidence.

Consumer/Renovate no-change evidence: the existing application-probe wrapper registers
`test-docker-application-contract.py` once and remains consumed by local `check-all.sh`,
PR/main/dispatch CI and Release's reused CI. None of `.github/renovate.json`'s custom-manager
file patterns matches the changed harness, helper, regression or this README. Application
images remain extracted from the six `images.tsv` catalogues, Podman matrix images from
`podman-live/matrix.tsv`, and Docker pins from the independently selected clean DockerLens
catalogue. No operational pin, manager, manifest, lockfile, workflow or historical evidence
changes. DockerLens's independently owned native harness coordinates the same closed
presence protocol under [DockerLens #76](https://github.com/Strukturpiloten/docker-lens/issues/76).

## Readiness failure observations

The unchanged 180-second private-socket readiness timeout and early outer-daemon
exit path collect read-only observations before mandatory teardown. Diagnostics
never turn that original failure into success, retry a lane, repair the host,
change launcher privileges or establish a startup cause. Core and volume profiles
share this same boundary; their native/application acceptance remains unchanged.

Only an explicitly registered outer name/run can be observed. Socket metadata is
limited to the exact run-private `/tmp/boxferry-docker-core.<run>/socket/docker.sock`
boundary with an owner-private, nonsymlink root and nonsymlink socket directory.
The helper authenticates a successful bounded narrow Podman inspection: exact
outer name, complete immutable container ID and exact run label. Wrong ownership,
malformed inspection or failed status with matching stdout never authorizes log
reading. Logs use only that authenticated ID, never ambient names or prefix scans.
Only after this ownership check does the helper observe socket metadata or attempt
a connection. The canonical harness has already checked its exact mounts; these
diagnostics do not independently authenticate a mount, daemon, or Engine peer.

On Linux, no-follow descriptors hold the private root, socket directory and socket
node. A bounded `AF_UNIX` connection addresses that exact held node through
`/proc/self/fd`, never a replacement pathname, and sends no bytes: no HTTP or
mutating request is added. `socket-connect` distinguishes `connected`, `refused`,
`missing`, `not-socket`, `permission-denied`, `timed-out`, `unknown` and
`not-checked`. Connected is transport observation only, not Engine authentication,
readiness, a startup cause, compatibility or application proof. Unsupported
platforms or unavailable held-node references have no ordinary-path fallback.

`socket-owner` reports only `self`, `other` or `unknown`; `socket-mode` reports
`owner-only`, `shared` or `unknown`, not access permission. No UID, GID, inode,
mode number or private path is printed. Root/directory/node identity and metadata
are rechecked after the connection interval. `socket-lifetime` is `stable`,
`changed` or `unknown`; replacement, unlink or metadata drift invalidates even a
successful connection. A final narrow outer inspection targets only the original
immutable ID and rechecks name/run ownership. `outer-recheck` distinguishes stable
identity from changed, malformed, failed, expired or cancelled observations; any
unverified recheck invalidates socket metadata and transport. These are bounded,
non-atomic snapshots, not protection against every transient external change.

Each read subprocess has a three-second limit and a 16-KiB output cap. Inspection
stderr is discarded; the last 80 log lines combine both streams under the same
cap, and any failed log status discards all bytes. Inspection, socket metadata,
the at-most-one-second connection, logs, identity recheck and process teardown
share one eight-second budget, enclosed by the existing runtime-budget
wrapper at 12 seconds with its unchanged kill-after and cleanup reserve. Timeout
or cancellation kills only diagnostic process groups, waits boundedly and closes
output handles. Kill/reap uncertainty is explicitly `termination-unverified`, not
successful observation; diagnostic failure never suppresses exact-owned cleanup.

Only finite socket, ownership, native-state and log-read categories are printed.
Recognized permission/storage/network/socket/startup error phrases are reported
as log observations, not inferred causes. Empty, unrecognized, oversized, failed,
timed-out and cancelled reads remain distinct. No raw log/inspect body, native
error, absolute private path, protected value or exception text is emitted.
`startup-cause=unestablished` remains explicit even when a phrase is recognized.

Independent offline fakes cover wrong owner/status, immutable-ID selection,
closed parsing, protected/oversized output, both log streams, deadline/cancellation
and kill/reap uncertainty. Real disposable local Unix sockets independently prove
positive/refused/held-node replacement behavior and that no bytes are sent; they
are not Docker or Podman runtime probes. Negative controls cover permission,
timeout, invalid boundaries, missing/non-socket nodes, ownership and lifetime
drift, descriptor closure, unsupported platforms and private CLI output.
Both core and volume readiness paths test observations
before removal and preserve failure even if diagnostics fail. These source checks
do not diagnose the historical failed run or provide fresh native evidence.

The last actual existing `/_ping` poll supplies one atomic record: `ping-curl-exit` (0–99,
`not-run`, or `unknown`), `ping-http-status` (exactly three digits, 000–599, or `unknown`),
`ping-curl-error`, `ping-collector`, and `ping-teardown-signal`. Readiness uses `--show-error` for
bounded error observations; executable selection and inherited environment stay unchanged.
Curl still discards the response
body and uses fixed `%{http_code}` write-out. Separate private stdout/stderr buffers retain at most
16 KiB each. Overflow clears only the affected buffer and continues bounded draining rather than
inducing SIGPIPE; an independently observed native exit is retained even when payload collection
fails. Stderr-only overflow retains valid HTTP stdout but leaves the text observation unknown;
stdout overflow or malformed HTTP output cannot promote readiness.
Raw stderr, response data, addresses, paths, protected values and exceptions are never emitted.

All five existing local Docker curl call sites (readiness, version, info, and the two exact-owned
cleanup GETs) deliberately harden the private-socket transport boundary with first-option `-q`
and explicit `--noproxy '*'`. `-q` disables default curl configuration; proxy bypass is explicit
because `-q` alone does not suppress environment-selected proxies. No environment/configuration
values are read or cleared. Selected executables, inherited environment, URLs, default GET methods,
conditional invocation counts, limits, readiness cadence and cleanup ownership remain unchanged;
no request, probe, retry, privilege change or deadline extension is added. Independently authored
offline expectations cover the complete readiness argv and all four shell call sites, exact
per-site invocation counts and unchanged limits, including literal wildcard quoting. This is
reviewed trust-boundary hardening, not evidence that ambient configuration or a proxy caused any
historical native exit 7. Historical failed receipts remain failed and immutable.

`ping-curl-error` recognizes only whole, narrow English curl envelopes matching the actual native
exit: connect, timeout, HTTP, proxy-resolution or host-resolution error observations. Empty,
localized, malformed, ambiguous, conflicting, oversized or unavailable text remains `unknown`.
`ping-collector` separately reports completion, overflow, timeout, cancellation, launch/read
failure, invalid boundary/output, wrapper failure or unverified teardown. A collector-induced kill is
never reported as native curl exit 28. Shell and helper independently validate closed fields,
including the diagnostic-unavailable fallback. A missing socket makes no request and preserves
the last actual record; before any poll it remains `not-run`/`unknown`.

`ping-teardown-signal` retains the original owned-group signal observation: `not-run`, `sent`,
`absent`, `denied`, `failed`, `cancelled`, or `unknown`. Before any poll it is `not-run`; mocked or
malformed observations are `unknown`, independently of native status. A denied or failed pre-reap
signal does not suppress a bounded read-only signal-0 absence lookup after successful leader reap
and both stream closes. Only ESRCH/`ProcessLookupError` before the lookup deadline proves absence;
present, denied, failed, cancelled or late lookup remains unverified. No nonzero signal or second
reap occurs after successful reap. Positive absence clears only group-teardown uncertainty and
never erases the original signal observation, collection failures, timeout/overflow, teardown
cancellation or uncertain reap/close. It changes neither curl privileges nor requests and does
not establish a daemon startup cause or native admission. Failure fallbacks preserve the validated
signal field; no exception, process identity or raw native text is emitted.

The five-second curl request limit, 180-second readiness budget, two-second cadence, private
socket URL, failure-before-apply and diagnostics-before-teardown order remain. The collector's
separate five-second cap also respects an absolute readiness deadline anchored before helper
startup, with 250 ms reserved for teardown. Absolute BOOTTIME is checked again after helper
collection and shell handoff, including BOOTTIME-only advancement; late or clock-uncertain
completion retains actual native fields but cannot promote readiness. A nonzero wrapper status
likewise preserves any validated native fields and forces collector uncertainty rather than
replacing the complete record. The existing wrapper's five-second KILL fallback is
reserved within that absolute budget; conservative rounding clips late polls, and exhausted
reserve skips a new poll without replacing the last actual record. Wrapper startup, scheduling
and teardown uncertainty can prevent complete collection of a native five-second timeout:
unknown observations are honest, not evidence that curl returned 28. Deadline-expired or
unverified teardown cannot promote readiness; normally completed native-zero polls retain
their existing acceptance when HTTP stdout is valid, including stderr-only overflow. The shared
presence/default reader keeps its separate three-second
contract unchanged.

These are text observations, not causes: `startup-cause=unestablished` remains mandatory. No
readiness poll, retry, environment/configuration inspection or host repair is added. Independent
offline classifier, stream overflow, native-status, deadline, cancellation, descendant-pipe,
missing-socket and privacy regressions establish only this diagnostic contract. The preserved
historical receipt remains untouched; these tests supply no Engine authentication, fresh native
readiness/admission, volume compatibility, application acceptance, Release evidence or explanation
of that earlier failure.

An isolated stopped-container probe on host Podman 6.0.2 confirmed that plain
`{{.Id}}` is a compatibility alias, while JSON template arguments require the
native Go field `{{json .ID}}`. The combined inspection uses that native field
and keeps its controlled JSON key `id`. Exact label/ID ownership was verified
before all probe reads; the container was never started and its ID/name absence
was verified after removal. This is template evidence only, not Docker startup,
volume behavior, daemon-mode compatibility or application acceptance.

Consumer/Renovate no-change evidence: `scripts/test-application-probes.sh` already
registers this Python contract once. Local `scripts/check-all.sh`, CI's application
contract job for PR/main/dispatch, and Release's reused CI consume that wrapper.
No gate wiring, workflow, operational image assignment, package/lockfile or tool
pin changes. `.github/renovate.json` retains its existing application-image,
Podman-matrix/provider and Lens-revision extraction paths; Docker image pins remain
only in the selected clean DockerLens catalogue, with no duplicate manager added.
For #413, the three changed helper/test/documentation paths match none of the
Renovate custom-manager file patterns; no software assignment, dependency, pin,
extraction path, grouping or approval rule changes. The existing Python standard
library supplies the Linux socket/descriptor calls. Independently published Lens
products retain their native suites and do not consume this BoxFerry-owned
application diagnostic helper, so no Lens consumer or manager change is needed.

For #420, the four owned helper/harness/test/documentation paths likewise match none of the
Renovate custom-manager file patterns. No dependency, software/image/tool pin, canonical pin
location, extraction rule, manager, manifest, lockfile, workflow, grouping or approval definition
changes. The unchanged wrapper registrations cover the new argv regressions in local,
PR/main/dispatch CI and Release; the shared bounded-native reader remains byte-for-byte unchanged.
Equivalent native-harness hardening is independently tracked in
[DockerLens #78](https://github.com/Strukturpiloten/docker-lens/issues/78). DockerLens remains the
independent native catalogue owner; that follow-up is not a completed cross-repository rollout,
and the immutable producer/candidate and historical receipts are not modified by this change.

## Authored core artifact prerequisite

[`core.compose.yaml`](core.compose.yaml) and [`core-expectations.json`](core-expectations.json)
are repository-authored, privacy-reviewed MPL-2.0 fixtures; redistribution is allowed.
No external oracle, capture, downloaded image, or operational software pin is used. The
literal placeholder is not a pullable image. The machine contract independently records the
container identity, command, exact source-byte SHA-256, and four intended lane identities:
Debian 11 and upstream, each rootful/rootless. These identities are authored expectations,
not native catalogue admission, historical-engine evidence, or four-lane runtime test passes.

[`docker-core-artifact.py`](../../../scripts/lib/docker-core-artifact.py) accepts exactly one
complete version-1 artifact with zero prerequisites and one literal `POST` container-create
request. Its `Image` must match the independently caller-selected private
`registry.invalid/boxferry-core/busybox:<literal-tag>` alias, its `Cmd` must match the authored
command, and `HostConfig` must be empty. Only that image substitution and the explicitly
selected rendering API prefix vary. Extra requests, fields, methods, paths, defaults, identities,
or prerequisites fail closed. The full context must exactly match the independently selected
target profile, including build/package revision, engine release, all three API versions,
root mode, and evidence digest. Lane checks constrain build kind/root mode only; no version
string admits a native engine or claims supported-version coverage.

The pure `validate_core` function requires raw plan, expectation, and source bytes, plus
`expected_plan_sha256`, lane, profile, and image alias. The raw-plan SHA-256 must be supplied
independently of the artifact under review; calculating it from an untrusted artifact alone
does not establish review authority. Whitespace changes invalidate an old plan binding.
Canonical source checking is mandatory in both the pure function and CLI. Expectations and
caller selections are trust inputs, never inferred from the plan. JSON parsing and regular-file
reads reuse the existing bounded helpers (1 MiB, depth 32, no duplicate keys/nonfinite numbers,
nonblocking reads, final-component no-follow). Unknown schemas and source drift fail closed.

The closed result contains byte hashes, lane, and offline metadata only, not raw request bodies,
commands, private aliases, or context evidence. It always reports `native_execution: false`,
`replay_authority: false`, `native_admission: false`, unmeasured runtime evidence, and null
budget measurements. There is no executor import, Rust catalogue-source parsing, daemon/API
access, image loading, apply/reacquire, migration, or dependency override. This is the core
preparatory slice of BoxFerry #366, not completion of that parent or native/runtime acceptance.

Run the independent literal positive/negative tests and exact source check:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-docker-core-artifact.py
PYTHONDONTWRITEBYTECODE=1 python3 scripts/lib/docker-core-artifact.py check-sources
```

The canonical `scripts/test-application-probes.sh` wrapper appends those two commands in that
order after every existing suite, with bytecode writing disabled. Its runner regression checks
exact argv, environment, order, and stop-on-failure for both, preserving every existing case.
Local `scripts/check-all.sh`, PR/main/dispatch CI (`.github/workflows/ci.yml`), and Release's
reused CI (`.github/workflows/release.yml`) consume that same wrapper; no workflow edit or
Lens/native conformance consumer is needed. Focused tests do not replace the complete gate.

Renovate no-change evidence: `.github/renovate.json` disables native Compose/Quadlet managers;
none of its custom manager file patterns matches these seven changed paths. Its active image
manager still owns only the six application `images.tsv` catalogues, not this authored core
source/expectation. Existing software/version/integrity definitions, extraction, grouping,
approval rules, historical evidence, and all their consumers remain unchanged. No new manager,
pin, dependency, or download is introduced.

## Six-application topology prerequisite

[`expected-applications.json`](expected-applications.json) independently declares the
application-owned service, network, volume, mount, ingress, alias, dependency,
excluded-peer, and success-category inventories for Forgejo, Nextcloud,
Paperless-ngx, Immich, observability, and the full eleven-service Supabase graph.
The source kind is explicitly `authored-compose`: these expectations describe the
reviewed BoxFerry Compose application fixtures, not acquired runtime defaults.
Source files and reusable probes are bound by SHA-256 of their actual bytes.
This includes `scripts/lib/observability-application.sh`, which independently
defines the observability boundary peer and behavior/persistence probes outside
the Compose fixture, and `scripts/lib/observability-application-probes.sh`, which
owns the shared application-semantic HTTP assertions. These bindings are checked
without executing either the harness or any HTTP callback.
`images.tsv` and `providers.tsv` in each original fixture directory remain the
canonical software/version/integrity/provenance/license inventories. This contract
duplicates no software pins, changes no dependency definition or Renovate owner,
and leaves admitted historical native evidence unchanged. Canonical fixture
updates require deliberate review of the affected contract and source hashes;
do not silently refresh hashes to make a changed graph pass.

The app-owned service counts are 2, 6, 5, 4, 6, and 11, respectively. Nextcloud's
shared proxy is declared separately from its six application services; the
second application stays excluded. Forgejo, observability, and Supabase also
exclude separately owned edge peers. Shared edges are external network
prerequisites, while application backends are internal and application-owned.
All six canonical Compose sources explicitly author application/run labels on
their created networks and named volumes. The topology validator compares those
authored labels without injecting or rewriting them; an acquired source with
different label intent needs its own independently reviewed contract. A future
harness cleanup annotation must have a separate receipt bound to the original
plan bytes and cannot masquerade as rendered intent.

[`docker-application-expectations.py`](../../../scripts/lib/docker-application-expectations.py)
validates the actual inert DockerLens version-1 complete artifact, rather than
trusting a projected inventory. It checks the exact independently supplied target
profile, private image-role aliases, resource creates, created-volume-before-mount
ordering, network attachments, loopback-only publications, and complete external
prerequisites. Network prerequisite `reference` fields remain unsigned decimal
strings through the full `u64` range. Missing, swapped, extra, duplicated,
misowned, partial, or malformed topology fails closed. Request-only streams,
unreviewed operations, and host-runtime socket mounts are rejected. JSON inputs
are capped at 1 MiB and depth 32, with duplicate keys and nonfinite numbers refused.
The container-native schema is closed: only the reviewed topology fields and
known command, environment, healthcheck, user, working-directory, and stop-setting
fields are accepted. The latter receive shape checks only and still establish no
semantic fidelity. `HostConfig` permits only `NetworkMode`, `Mounts`, and
`PortBindings`; anonymous `Volumes`, dynamic publication, DNS/host overrides,
links, and unknown native fields cannot bypass the inventory checks. Empty or
short-form source network attachments author no aliases. Immich's server mounts
only its library volume; the writable fixture bind belongs to the ML service.

The pure `validate_application` function receives plan and admission bytes plus
independently selected application, lane, profile, image aliases, prefix, run ID,
fixture root, and expectation bytes. `check_sources` separately verifies the
canonical source bytes; the CLI always runs that check first. The closed version-1
admission has these fields:

- Identity: `schema_version`, `kind`, `evidence_kind`, `source_kind`,
  `native_execution`, `application`, `lane`, `prefix`, `run_id`, and `fixture_root`.
- Byte bindings: `docker_plan_sha256`, `expectations_sha256`, and `source_sha256`.
  The last hashes sorted source records, each containing the eight-byte big-endian
  UTF-8 path length, path bytes, and 32 decoded checksum bytes.
- Declared requirements: `required_checks`, `dependencies`, `excluded_peers`, and
  `shared_services`, each exactly equal to the selected authored contract.
- Explicit non-evidence: `runtime_evidence` must be `unmeasured` and
  `budget_measurements` must be `null`.

Admission `kind` is `boxferry-docker-application-offline-admission`,
`evidence_kind` is `offline-contract-prerequisite`, `source_kind` is
`authored-compose`, and `native_execution` is `false`. A passing result proves
only this offline topology prerequisite. Declared dependencies and application,
persistence, and safety checks have not executed. In particular, this validator
does not authorize command/environment/default-field behavior, losses,
dependency-sidecar choreography, external-resource existence, native capability
compatibility, labelled-volume admission, budget ceilings, or runtime replay.
It supplies neither a live success result nor runtime replay authority. Native
labels and attachment branches still need their own evidence. This checkout
contains only the offline prerequisite; the unfinished live scaffold remains
separately preserved and is not included in this change.

The finite future live scope remains current upstream rootless for all six
applications and representative Forgejo/Nextcloud upstream rootful. This helper
does not admit Debian application lanes, Swarm, macOS, a Cartesian application
matrix, or additional native profiles. The approved four small native lanes
remain a separate prerequisite and are not implemented or validated by these
application contracts.

Run the focused offline checks without building Rust or starting a runtime:

```console
PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-docker-application-expectations.py
PYTHONDONTWRITEBYTECODE=1 python3 scripts/lib/docker-application-expectations.py check-sources
```

The tests use separately authored native artifacts for all six applications and
negative mutations for missing services, ownership, isolation, source and plan
binding, aliases, ingress, mount access, ordering, schema versions, and malformed
admission data. They never derive native test requests from generated output or
from the expectation catalogue. Direct source-correspondence assertions also
protect null/short-form aliases and the Immich server/ML mount distinction.
The existing [`test-application-probes.sh`](../../../scripts/test-application-probes.sh)
wrapper runs both read-only commands with Python bytecode writing disabled.
Its regression test verifies exact invocation, environment, and failure
propagation for each command before any later probe suites run. The wrapper is
already consumed by `check-all.sh` and hosted CI, including main and Release
validation; no separate workflow or gate definition is introduced. These checks
complement the complete gate and
do not replace any core, native-library, diagnostic/loss, or live acceptance
check. No runtime runner is wired to this helper, and this prerequisite does not
complete BoxFerry #366.

## Forgejo topology and dependency-sidecar review

[`docker-application-schedule.py`](../../../scripts/lib/docker-application-schedule.py) adds one
reviewed Forgejo/Nextcloud offline check of the actual native plan, existing topology admission, and unpublished
BoxFerry dependency sidecar as an inseparable review set. It reuses `validate_application` above;
there is no second native decoder, request renderer, or runtime client. The canonical CLI calls
`check_sources` before reading artifacts. Its pure `validate_schedule` takes the three raw byte
documents and the independently supplied catalogue, application, lane, profile, image aliases,
prefix, run ID, and fixture root; embedded callers must separately check the source bindings.
The catalogue and selected profile are caller trust inputs, not authority inferred from output.
All JSON uses the existing 1-MiB/depth-32 parser, rejecting duplicate keys and nonfinite numbers.
CLI file reads reject nonregular files and final-component symlinks before a bounded read.

The sidecar contract follows the repository-authored #343 candidate's
`render_docker_dependency_sidecar` and the separately preserved #366 scaffold's pure dependency
validator. Neither unpublished worktree becomes a dependency of this check. Its closed schema-1
envelope contains `kind: "boxferry-docker-dependency-decisions"`, `native_execution: false`,
`docker_plan_sha256`, and `decisions`. Each decision has `service`, `service_runtime_name`,
`dependency`, `dependency_runtime_name`, `condition`, `condition_explicit`, `required`,
`required_explicit`, `restart`, `restart_explicit`, `fidelity`, `native_engine_field`, and
`provenance`. Conditions are `started`, `healthy`, or `completed_successfully`; implicit values
must resolve to started/required/no-restart. Fidelity must be `approximate` and
`native_engine_field` must be false. Provenance contains only `reference`, `condition`, `required`,
and `restart` arrays of the finite source-document, runtime-observation, user-override,
implementation-default, and conversion-decision category spellings. Paths and protected values
are not provenance categories. This is a BoxFerry choreography decision, not an Engine field.

The independently authored Forgejo contract requires exactly `db` and `forgejo`, with the sole
`forgejo` → `db` edge `healthy`, required, and no restart propagation. Logical keys and runtime
identities must agree with the authored inventory, not just with each other. The admission and
sidecar each bind SHA-256 of the exact native plan bytes. The closed result additionally hashes
the exact admission and sidecar bytes before reserialization; whitespace changes in plan bytes
invalidate either old binding, and whitespace changes in the other documents change their result
hashes. This does not invent an admission-to-sidecar binding absent from the existing schemas.
Missing, swapped, extra, unknown, duplicate, self-referential, reversed, or cyclic decisions fail.
The merged topology validator retains its network, volume, alias, ingress, mount, excluded-peer,
ownership, and operation-order assertions unchanged.

The result is `boxferry-docker-forgejo-offline-schedule` schema 1. It preserves the external edge
prerequisite's identity, bridge constraint, and exact decimal `u64` reference, including its
maximum value. `native_request_order` retains original request indices. Separate
`review_operations` reference those same indices in deterministic network, volume,
dependency-ordered container, then attachment order; they neither alter nor re-render the plan.
External prerequisites must be reviewed before those operations. `service_review_layers` is
`[["db"], ["forgejo"]]`, not a generated native start request or a claim that database health
was observed. Validated dependency choices are retained separately. Result identities are private
artifact-review data; raw request bodies, environment values, and commands are never copied into
the result or rejection diagnostics.

Every result says `offline-contract-prerequisite`, `native_execution: false`,
`replay_authority: false`, `native_admission: false`, `runtime_evidence: "unmeasured"`, and null
budget measurements. Command/environment/health semantics, actual external-resource existence,
image loading, startup/readiness/restart behavior, Git/SSH/persistence/isolation acceptance,
apply/reacquire, native capability admission, and measured budgets remain separate requirements.
No runtime runner or new application lane is enabled; #343 and #366 remain incomplete.

Run the focused regression without building Rust or launching a runtime:

```console
PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-docker-application-schedule.py
```

The existing `test-application-probes.sh` entry point runs this suite after the topology tests and
source-binding check, before privacy and existing Podman probe suites. Its runner regression
checks the exact Python command, disabled bytecode writes, order, and stop-on-failure behavior,
preserving every prior case. Canonical consumers remain `scripts/check-all.sh`, hosted CI/main,
and Release's complete validation through the existing shared wrapper; none requires new wiring.
BoxFerry alone owns this application contract, so Lens/native conformance and other repositories
need no consumer change or new dependency. All manifests, lockfiles, workflows, application image
and provider catalogues, and `.github/renovate.json` are unchanged. No operational pin, extraction
path, manager, grouping, approval, or historical evidence is added, moved, or changed: existing
Renovate ownership stays intact. Test Engine/profile strings and synthetic aliases are authored
offline evidence, not downloaded software pins or native admission.

## Nextcloud dependency schedule and authored init/cron intent

The same canonical schedule validator accepts `--application nextcloud`; omitting that option keeps
the existing Forgejo CLI and result unchanged. Other applications remain unreviewed. Nextcloud uses
the same bounded parser, source-first CLI check, caller-selected authority, and three exact raw-byte
bindings described above, with no separate decoder, renderer, interpolation engine, or runtime client.
The independently literal contract contains exactly six app-owned services and five required,
non-restarting dependency decisions:

- `app` depends on `db` and `cache`, both `healthy`.
- `init` depends on `app`, `healthy`.
- `cron` depends on `init`, `completed_successfully`.
- `frontend` depends on `app`, `healthy`.

Conditions are explicitly authored; `required: true` and `restart: false` are implicit defaults in
this fixture. The sidecar must retain those values and explicitness flags. Omitted-option provenance
arrays may be empty; provenance retains the existing finite-category boundary rather than inventing
origins. Missing, extra, repeated, self-referencing, reversed, or cyclic edges, altered conditions,
runtime identities, flags, and unreviewed provenance categories are rejected.

The validator separately requires init's native `Cmd` to be exactly
`["php", "/var/www/html/occ", "status"]` and `User` to be `"www-data"`, and cron's native `Cmd` to be
exactly `["/cron.sh"]`. These checks bind authored field intent only: they do not inspect or infer
image-default `Entrypoint`, protected environment fidelity, health, successful initialization,
successful exit status, actual cron execution, or runtime compatibility. Missing, null, wrong-type,
changed, or exchanged authored fields fail without returning their values.

The schema-1 result kind is `boxferry-docker-nextcloud-offline-schedule`. Its service review layers
are `[["cache", "db"], ["app"], ["frontend", "init"], ["cron"]]`, never native start requests or
observed readiness. `authored_checks` contains only `init.command`, `init.user`, and `cron.command`,
not command/user bodies. `shared_service_expectations` retains only the shared proxy's prefixed
identity, `ownership: "shared"`, and `runtime_evidence: "unmeasured"`; it proves neither existence nor
ownership. The proxy remains a separately owned external declaration, not a seventh admitted
container or attachment. Attempted proxy and excluded-peer create/connect requests are rejected
by the unchanged topology boundary, never filtered or projected away. Every admitted native request
index appears exactly once in `review_operations`, even when container requests or sidecar decisions
are reordered; `native_request_order` retains the original indices.

Rootful and rootless upstream profiles remain independently selected review inputs. Mismatched
versions, API context, lanes, source digests, and raw plan/admission/sidecar bytes are rejected; these
offline profile strings do not establish an Engine-version support catalogue or native admission.
All result non-executing, non-replayable, non-admitting, unmeasured-runtime and null-budget markers
remain unchanged. Provisioning, startup/readiness, one-shot completion, persistence, isolation,
apply/reacquire, native capabilities, and measured budgets remain outstanding under #343/#366.

The existing focused schedule suite independently authors the Nextcloud requests, five edges,
commands, and review order, with positive rootful/rootless and mutation tests. The existing shared
probe entry point and runner regression already execute this same suite with bytecode disabled and
stop on failure; local complete-gate, hosted CI/main, and Release consumers therefore need no wiring
change. No other repository, manifest, lockfile, provider/image catalogue, software pin, Renovate
manager/extraction path/group/approval, or historical evidence changes for this extension. Existing
Renovate ownership and Lens independence remain intact. This is an offline integration prerequisite,
not a Docker adapter, new native lane, compatibility claim, or library release.

## Forgejo explicitly authored fields

[`docker-forgejo-authored-fields.py`](../../../scripts/lib/docker-forgejo-authored-fields.py)
adds a separate Forgejo-only offline prerequisite for the explicitly authored environment,
database healthcheck, and application user. It reuses the complete topology and dependency-sidecar
review above, including exact plan/admission/sidecar bytes, source bindings and independently
selected context. The CLI checks canonical source bytes before opening artifact or protected
expectation files. The pure `validate_authored_fields` API performs no I/O; its caller must
separately call `topology.check_sources` before relying on source bindings, as with the other
pure helpers. No additional native decoder, renderer, runtime client, or interpolation engine
is introduced.

The separate `--interpolation-expectations` regular JSON file has exactly these fields:

- `schema_version`: integer `1` (not a boolean).
- `kind`: `boxferry-docker-forgejo-interpolation-expectations`.
- `context`: exactly `application`, `lane`, `profile`, `image_aliases`, `prefix`, `run_id`, and
  `fixture_root`, equal to the independently supplied review context. Application is `forgejo`;
  profile and aliases are full objects, not identities inferred from the native plan.
- `interpolation`: exactly `BF_DB_PASSWORD` and `BF_FORGEJO_SECRET_KEY`, supplied independently
  from the reviewed source inputs, never extracted from the artifact being checked. Each value
  is a nonempty, valid UTF-8 string of at most 4096 bytes, without NUL. Whitespace, Unicode, and
  embedded `=` remain significant.

All JSON documents retain the shared 1-MiB/depth-32, duplicate-key and nonfinite-number rejection.
Expectation input rejects missing, extra, malformed or cross-context fields. The CLI retains
bounded, nonblocking regular-file reads and rejects final-component symlinks. Keep private
expectation files owner-readable only and outside shared evidence; supplying this file is not
authorization to publish credentials, render them into artifacts, or execute output. No implicit
process environment or `.env` lookup occurs. Caller-supplied catalogue, source, profile, image
aliases and interpolation are trust inputs, not authority established by the result.

The independent authored comparisons require all three PostgreSQL and fifteen Forgejo environment
assignments, the same independent database password in both containers, and the independent Forgejo
secret key. Missing or wrong values/types and duplicate environment names fail, including duplicate
unauthored names. PostgreSQL's health test is exactly the authored `CMD-SHELL` command, with interval
2 seconds, timeout 5 seconds (native nanoseconds), and 60 retries. Forgejo's user is exactly
`1000:1000`. No health test is executed or observed.

Additional well-formed native environment assignments, including unauthored overrides, are
**unassessed**: neither their presence nor their absence establishes image-default or complete
environment fidelity. Likewise this helper does not assert unauthored command, entrypoint,
database user, application healthcheck, or health default fields. The existing native shape
boundary still applies to every field. Passing these authored-subset assertions does not establish
that an application will work.

The result kind is `boxferry-docker-forgejo-authored-fields`, schema 1, and records only the four
authored-check categories alongside the existing topology/schedule review metadata and exact
artifact hashes. It never copies protected expectation values, their input digest, raw native
environment assignments or health commands into results or rejection diagnostics. It remains
`offline-contract-prerequisite`, with `native_execution`, `native_admission` and `replay_authority`
all false, `runtime_evidence: "unmeasured"` and null budget measurements. Real external resources,
image defaults, native capability admission, startup/readiness/restart, Git/SSH/persistence/isolation,
apply/reacquire and budgets still require separate reviewed acceptance. No runtime runner is wired;
The parent issues #343 and #366 remain incomplete.

Run the independent synthetic-artifact regression without Rust builds or a runtime:

```console
PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-docker-forgejo-authored-fields.py
```

The canonical `test-application-probes.sh` wrapper runs it after the existing topology/source and
schedule checks, before privacy and Podman probes. The runner regression checks exact command,
disabled bytecode writes, ordering and stop-on-failure while retaining every prior suite and case.
Consumers remain `scripts/check-all.sh`, CI/main and Release validation through that same wrapper;
there is no new workflow or gate. BoxFerry owns this application assertion; no Lens or other
repository consumer changes. All manifests, lockfiles, workflows, image/provider catalogues and
Renovate definitions are unchanged. The new helper/tests contain no operational software pin,
download or package declaration and add no Renovate extraction match: existing native/custom
managers, grouping and approval rules retain their canonical paths, with synthetic profile strings
remaining offline test data rather than managed dependencies.

## Shared observability HTTP assertions

[`observability-application-probes.sh`](../../../scripts/lib/observability-application-probes.sh)
is inert when sourced. Its caller supplies a semantic HTTP callback with exactly
`context`, `prefix`, and `url` arguments. Context is opaque: the helper neither interprets
sockets nor supplies native flags, probe placement, authentication, or runtime selection.
The existing Podman wrapper entrypoints delegate through their existing backend HTTP transport.
Provisioning, bounded readiness and wait deadlines, published-port/native-inspect checks,
persistence operations, resource budgets and cleanup remain runtime-owned and unchanged.

Shared assertions retain the exact authored PromQL value `42`, exactly one known LogQL line,
Prometheus retention/remote-write flags, Grafana datasource identities and health, and dashboard
UID/title/query expressions. Generic metric queries still take a caller-selected expected value
for the existing historical-persistence checks; extraction does not observe persistence. HTTP
callbacks and every parse/query/assertion explicitly return failures, including conditional
shell contexts where `errexit` does not apply. Each HTTP reply must contain one JSON document;
empty, malformed and multiple-document replies fail. Raw replies and parser errors are not
printed by the assertions; callbacks own redacted transport diagnostics.

The canonical `test-application-probes.sh` wrapper runs the new independently authored
`test-observability-application-probes.sh` suite exactly once after the existing application
probe suites. Its runner regression preserves all previous commands/failure cases and checks
the added suite's order and failure propagation. Local `check-all.sh`, CI/PR/main/dispatch and
Release's reusable CI consumer receive the suite through that existing wrapper. Existing
observability offline/native-wrapper regressions and pre-release live task remain separate
consumers; no live runner, workflow definition or cross-repository consumer is added.

The changed observability wrapper and new shared helper are bound to their exact reviewed bytes
in `expected-applications.json`; all other source records and topology/success-category values
remain unchanged. These source-only assertions are not native Docker capability, compatibility,
application execution, replay or admission evidence. #343 and #366 remain incomplete.
Image/provider catalogues, manifests, lockfiles, operational pins, workflow definitions and
Renovate configuration remain unchanged. Custom-manager file patterns match none of the changed
paths; canonical image/provider extraction, grouping and approval rules stay intact. No native
package-manager input or additional software dependency is introduced.

## Pending bounded core harness (#366)

The isolated core harness reuses `scripts/lib/docker-core-artifact.py` for its
independently authored source, artifact and profile checks. It selects the bounded
private artifact's SHA-256 once and reuses that identity before daemon startup,
after observed identity checks and at test-only replay. This digest binds bytes;
it is not semantic authority and cannot approve changed output. Image pins remain
owned by the explicitly selected clean DockerLens checkout rather than copied here.

The per-lane reacquisition diagnostic oracle remains empty and deliberately
fail-closed. Offline contracts, source receipt checks and harness definitions do
not establish a passing Docker journey or six-application acceptance. Candidate
Docker CLI/library integration, native evidence and independent live expectations
remain #343/#366 prerequisites. No product code applies output; only this explicitly
isolated, opt-in test boundary can replay its single allowlisted synthetic request.

The canonical `scripts/test-application-probes.sh` wrapper also runs
`scripts/test-docker-application-contract.py` once after the core artifact tests
and source check. Its independent runner regression checks exact Python argv,
disabled bytecode writing, ordering and failure propagation. The existing local
complete gate, PR/main/dispatch CI and Release validation all consume this wrapper;
none launches the opt-in runtime harness. Existing Podman application and Lens
native requirements are unchanged. No other repository consumes this BoxFerry-only
test boundary.

No operational pin, package declaration, lockfile, workflow definition or canonical
software location changes in this checkpoint. The harness reads image pins only
from the caller-selected exact clean DockerLens checkout; it does not copy or
manage them. The BusyBox constant is a fixed independent historical image-config
expectation, not a pull source. Existing Renovate custom-manager paths do not
extract these new scripts or this fixture README; native package inputs are
unchanged. Existing image/provider owners, grouping and approval rules therefore
need no configuration change. Updating that historical expectation in the future
requires independent native source review, not an automated silent refresh.

## Opt-in startup-only readiness comparison (#425)

`scripts/docker-application-conformance.sh --profile readiness-comparison` is a
development diagnostic on one owned, pinned `debian11-rootful` daemon. It is not
a BoxFerry executor, native admission, migration receipt, compatibility result,
or six-application acceptance. No other lane is admitted. Supply an existing
fresh, canonical root-owned mode-0700 diagnostic directory outside both source
checkouts, and the independently established rootful host PID namespace's
device/inode identity; the namespace is not inferred from the current caller.

```text
sudo scripts/docker-application-conformance.sh \
  --profile readiness-comparison --lane debian11-rootful \
  --docker-lens-root CLEAN_ABSOLUTE_NATIVE_CHECKOUT \
  --docker-lens-revision EXACT_NATIVE_COMMIT \
  --native-script-sha256 EXACT_NATIVE_SCRIPT_SHA256 \
  --diagnostic-directory FRESH_PRIVATE_ABSOLUTE_DIRECTORY \
  --expected-pid-namespace EXPECTED_HOST_DEVICE:EXPECTED_HOST_INODE
```

The profile rejects artifact, API-version, candidate/binary/receipt and volume
evidence arguments; other profiles reject its two diagnostic arguments. Before
any runtime mutation it verifies UID 0, the explicit PID namespace, initial UID
mapping, effective NET_ADMIN/SYS_ADMIN, rootful Podman, the clean exact native
checkout/script and read-only native bridge prerequisites. The native helper is
imported with an empty policy environment, so inherited hosted-job settings
cannot authorize module loading. Missing prerequisites produce a closed refusal,
not a workaround. There is no module loading, host configuration change, cgroup
disablement or custom native network/egress sidecar creation.

The canonical pinned image catalogue, single daemon launcher, owned storage and
socket, two-CPU/4-GiB/512-PID envelope, independent deadline guard and storage
watchdog are reused. Exact creation CID, run label/name, running privileged
process/PID/start/namespace and physical cgroup-v2 limits are admitted before
probing. Only the known `machine.slice/libpod-CID.scope[/container]` and
`libpod_parent/libpod-CID[/container]` layouts are accepted. Delegated leaves
must inherit effective finite limits from their physical ancestors; unknown
layouts or missing controllers fail closed. No fixture image pull/save/import,
core workload, fixture POST, version/info acceptance or candidate verification
is performed.

Both deadlines start before the daemon launcher: 180 seconds for the actual
BoxFerry helper/wrapper/five-field parser and 360 seconds for the native literal
curl route. Launcher/admission time, serial observers and supervision reserve
consume those original budgets. BOOTTIME elapsed measurements include suspend;
completion exactly at a budget boundary is not readiness. Late polls are skipped
when the wrapper's KILL reserve cannot fit, without replacing prior observations.
BoxFerry elapsed time comes from the literal canonical wrapper/parser handoff
sample, not the later recording helper's clock. Recorder startup or suspend
cannot reclassify an already completed pre-budget poll as late. The handoff is
bound to the original anchor and checked against the recorder clock and prior
observations; malformed, future, pre-anchor or impossible out-of-order samples
are refused. Cross-route same-centisecond ordering is not invented because the
shell clock sample is coarser than the native observer's nanosecond clock.
The first actual BoxFerry poll is recorded before changing socket permissions.
After authenticating the same owned socket inode, an explicit mode-0666
transition precedes native-before, harmonized BoxFerry and native-after polls.
The original failure and first harmonized failure remain separate from later
readiness. If no original poll fits before the consumer closes, its baseline is
null (not a fabricated curl failure); a late socket may still be observed by the
native route. No original-permission causality is inferred from the harmonized
bracket. Native custom-network/sidecar omissions and shared bounded supervision
of the otherwise literal exit-only native command are recorded differences.

Oracle provenance is the caller-selected exact clean
`Strukturpiloten/docker-lens` revision's `scripts/native-conformance.sh` and its
canonical Debian 11 rootful image tag/digest, not a copied fixture or captured
response. Its observed command is `curl -q --noproxy '*' -fs --max-time 5
--unix-socket "$socket" http://localhost/_ping >/dev/null`; the diagnostic
independently supplies the argument and discards output under its bounded
supervisor. DockerLens's source license is MPL-2.0. No native source or image is
redistributed by this change; the bound bridge helper is read from that external
checkout at runtime. Exact source version and command binding reside in the
private report's revision/script/helper hashes.

The only retained file is private mode-0600 schema-2 `readiness-comparison.json`, bounded
to 64 KiB, exclusively created to refuse historical-file overwrite. It contains
closed phases/statuses, source/native revisions and hashes, selected image
digest, namespace/daemon/socket identities, elapsed times and actual numeric
errno only when available. Endpoint paths are hashed; no arbitrary native
stderr, exception text, environment, credentials or request/response body is
retained. Native HTTP is always null: curl exit 7 is not errno 111 or an inferred
HTTP status. Invalid/truncated readiness collector records, unverified child teardown and
uncertain resource closure withhold the final result. Source hashes and the
clean native binding are rechecked after teardown.

After route sampling ends and before teardown, one optional source-owned operation
reads `podman logs --tail 80` using only the exact creation-bound immutable CID.
Both streams share the canonical reader's private 16-KiB combined cap; nonzero,
unknown, truncated, late or cancelled reads provide no category. CID/name/run,
running privileged PID/start/namespace, host namespace, exact cgroup membership,
unchanged effective limits, held root/socket/cgroup directories and any previously
bound socket node are checked before and after collection. Directory/node checks
send no bytes and add no curl, connection or guest execution. These checks are
bounded snapshots, not atomic protection against every transient replacement.

The shared internal work deadline is seven seconds across both brackets and the
log read, with one second reserved for handoff. A whole-operation SIGALRM guard
interrupts blocking metadata reads as well as subprocess collection through the
canonical reader's cancellation/owned-child teardown path. The short
fork/PID/pipe publication masks ALRM/TERM/INT/HUP briefly, with no bootstrap or exec
wait masked. The child sets up its own session, sends a fixed READY marker and waits
for ACK; the parent authenticates and publishes session ownership before ACK can
authorize exact exec. Bootstrap waits remain interruptible within the same work
deadline. Until ACK, only the held direct child can be terminated. After ACK, group
signals require current parent WNOWAIT ownership and original PID/start/session/group
identity; ECHILD or identity drift forbids numeric rescue. Nonblocking reaping and
state publication are cancellation/SIGCHLD-masked; no group signal follows reaping.
Bounded shutdown defers catchable cancellation until exact state and owned FD closure
are published, then restores handlers and propagates uncertainty. All registry
children share at most one second of teardown, not one second per child.
Every outer shutdown scope charges entry through exit, including handler setup and
restoration, all owned FD/pidfd closes, cached reaped-child returns and registry
retries. Nested registry loops share one fixed scope end and are not double-charged;
legitimate work gaps between the sequential readers do not consume teardown time.
Both remaining cumulative allowance and the absolute work-plus-handoff deadline
must hold at final handoff. Late finite closure latches timing uncertainty and
forbids subsequent acquisition or category promotion, even when the child was
positively reaped and its FDs physically closed. Physical closure and verified
timeliness are separate facts; a kernel-D-state close can return late but cannot
be reported as a timely diagnostic. There is no ordinary userspace blocking
exception to this admission rule.
The previous alarm handler, inactive timer and signal mask are restored. An already
active timer or blocked alarm facility is refused without stealing caller state.
BOOTTIME expiry is checked through setup, collection and restoration, including
suspend; remaining time is translated to the reader's MONOTONIC deadline without
resetting the shared budget. Kernel uninterruptible I/O cannot be synchronously
bounded or cleaned beyond the caller's recovery reserve. Interruptible blocked
bootstrap is not an exception to the work deadline.
The existing outer wrapper supplies
a separate 12-second TERM bound and five-second KILL reserve. This diagnostic
reserve never extends the original readiness budgets or changes their timestamps.
Catchable-cancellation cleanup handlers are armed before the diagnostic; refusal
or timeout cannot skip mandatory cleanup or clear an earlier failure.

Only `startup_logs.status` (`not-run`, `observed`, `withheld`) and its nullable
category are retained. The shared classifier's closed categories are `empty`,
`content-present`, `permission-error-observed`, `storage-error-observed`,
`network-error-observed`, `socket-error-observed`, `startup-error-observed` and
`multiple-errors-observed`. A category is text observation, not a daemon cause.
Raw logs, exception text and extra native fields are never written. Diagnostic
withholding affects only this category, not comparison uncertainty or readiness
classification; it cannot grant native/application acceptance, and qualification
remains `none`. Schema-1 and unknown-version historical reports are refused without
rewriting or upgrading their bytes.
Duplicate log operations, whether a category was observed or withheld, are refused
before any write or native call and leave the existing report bytes unchanged.

Classifications distinguish early native-only readiness, native readiness after
the consumer budget, both routes ready, consumer-only readiness and
`shared-no-ready-observed`. The last is an observation, not a shared-cause
diagnosis; late native success cannot negate earlier consumer success. All
classifications retain `qualification: none`. A nonzero harness exit or
`status: withheld` is not a completed diagnostic.

Canonical cleanup runs after success, failure and catchable cancellation. It
requires the exact created CID before removal, positive owned outer/storage
absence, watchdog closure and private-run-directory closure; final PID and
physical cgroup absence are separately required. Unknown identity/absence
retains task-owned resources and withholds completion. No pruning or historical
resource deletion is authorized. SIGKILL, host loss or interruption beyond the
independent guard's recovery cannot guarantee cleanup; a pending/withheld report
requires independently reviewed exact-resource recovery, never broader cleanup.

Offline `scripts/test-readiness-comparison.py` controls cover profile isolation,
literal observer ordering, exact deadlines/suspend, expected classification,
rootful and bridge refusal, source binding, cgroup admission, permission/inode
transition, privacy, historical-report refusal and resource-closure uncertainty.
Log controls independently assert categories, exact-CID invocation, shared deadline,
before/after binding and effective-limit drift, cancellation, cleanup ordering and
privacy. Real bounded-reader subprocess controls cover merged streams, native
nonzero status, overflow and TERM without starting Podman or a daemon.
Real blocking-pipe metadata and aggregate-alarm controls check interruption,
owned-reader reaping, acquisition-time cancellation and alarm-state restoration;
separate clock controls cover BOOTTIME-only suspend and exact expiry.
The explicit `readiness_read(..., launcher=...)` seam is opt-in only for the log
operation's inspect/log/inspect chain; the default Popen collector, presence reader
and native curl collector retain their existing launch/collection behavior. The
small `scripts/lib/owned-native-launch.py` helper is Linux/main-thread/single-thread
only, owns at most three children and is not a product execution API. Schema 2's
closed `sources` object now explicitly includes `launcher_sha256` alongside the
other nine source fingerprints; old shapes are refused, never silently upgraded.
Real fork controls cover all four cancellation signals at publication/bootstrap,
READY/ACK isolation, blocked-bootstrap expiry, external reap/ECHILD, shutdown wait
and reap-state races, thread refusal and pipe/fork failure closure.
It is registered once in `scripts/test-application-probes.sh`; the runner-order
regression preserves all existing suites and failure propagation. Canonical
consumers remain local `scripts/check-all.sh`, PR/main/dispatch
`.github/workflows/ci.yml`, and Release's reusable deterministic CI gate. None
automatically launches this opt-in diagnostic. Lens native suites remain
independent; no cross-repository runtime consumer is changed.
Canonical collector consumers in this repository are the Docker contract and
`native-presence.py` (including Podman/application harness calls), plus this
comparison. Targeted source inventory in the approved ComposeLens, PodmanLens,
QuadletLens, DockerLens and website checkouts found no `bounded-native-read.py` or
`readiness_read` consumer. No other consumer opts into the new launcher.

Renovate no-change review: `.github/renovate.json` custom-manager patterns do not
match these changed scripts or this README. No software, operational image pin,
package declaration/lock, workflow or provider definition is added, moved or
changed. The exact caller-selected DockerLens catalogue remains the sole image
owner; existing extraction, grouping and approval rules therefore remain intact.
