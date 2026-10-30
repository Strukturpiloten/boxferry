# Prepared Paperless application source

`scripts/lib/docker-paperless-application.sh` prepares the Paperless portion of
BoxFerry #366. It is inert when sourced. Its only native-operation boundary is a
caller-supplied Bash function. The hermetic test registers through
`scripts/test-application-probes.sh`; no dispatcher, admission, native job, workflow,
production adapter, or Podman application runner selects this module.

This is **source preparation, unqualified for native application execution**.
Passing fakes establishes orchestration and rejection behavior. It does not prove
Docker compatibility, application behavior, migration fidelity, cancellation of
real process trees, observed image provenance, or resource cleanup. Native
prerequisites, independent review, and actual isolated application execution remain
mandatory. Podman Compose execution cannot be relabeled Docker evidence.

## Ownership and invocation

After sourcing the module, call `docker_paperless_application_run` with four
arguments: repository root, transport function name, opaque transport context,
and a private configuration JSON path. No default transport exists. The caller
must supply a process-tree watchdog covering this entire invocation and every
transport descendant. The module does not implement a second runtime executor.

The transport function receives exactly three arguments: context, request JSON
path, and reply JSON path. It must finish within the request's absolute deadline,
write one bounded regular JSON reply file, and return its actual exit status.
No arbitrary shell string is accepted or evaluated. Transport stdout and stderr
are withheld. Return statuses, including cancellation statuses 130 and 143, survive
cleanup; a cleanup failure turns an otherwise successful invocation into failure.

The configuration has exactly these fields:

| Field                                     | Contract                                                                   |
| ----------------------------------------- | -------------------------------------------------------------------------- |
| `schema_version`                          | Integer `1`                                                                |
| `stage`                                   | `source-fixture` or `generated-target`                                     |
| `prefix`, `run_id`, `fixture_root`        | Independently supplied exact topology identities                           |
| `endpoint`                                | Exactly `id`, `uri`, `scope`, `mode`, `ownership_sha256`                   |
| `profile`                                 | Exact upstream target profile accepted by the topology prerequisite        |
| `image_aliases`                           | Exact canonical image keys, unique private `registry.invalid` aliases      |
| `image_config_ids`                        | Same keys, independently observed image configuration SHA-256 IDs          |
| `plan`, `admission`                       | Paths to exact topology plan and offline prerequisite admission            |
| `work_deadline_ms`, `cleanup_deadline_ms` | Original absolute Unix millisecond deadlines; cleanup must follow work     |
| `target`                                  | `null` for source fixture; held target custody object for generated target |

Endpoint scope is exactly `owned-inner`, mode exactly `rootless`, and URI an
explicit absolute Unix socket inside the caller-owned isolated harness. The two
conventional host Docker socket paths are rejected. A different pathname alone
does not establish ownership: the caller must independently bind the endpoint ID,
ownership hash, daemon profile, inner placement, and watchdog. Ambient runtime
discovery, backend fallback, host sockets, implicit pulls, host execution, and
rootful Paperless expansion are outside this profile.

The module validates and privately holds the canonical catalogue, its Paperless
sources, the exact plan/admission, literal public test credentials, and target
custody bytes before provisioning. It rechecks held hashes before and after each
transport call. The plan receives both the existing topology validation and
additional closed Paperless environment, command, healthcheck, and image-default
override checks. The small fixture reader accepts only the hash-bound canonical
environment/command forms; it is not a general Compose importer.

## Two distinct stages

For `source-fixture`, the transport provisions the held canonical authored
fixture. The held plan binds expected topology and is not authority to replay
generated requests. The provider and image catalogues remain canonical.

For `generated-target`, the caller's isolated test harness may apply the exact
allowlisted held emitted target plan. The target object contains
`artifact`, `artifact_sha256`, `neutral_expectations`,
`neutral_expectations_sha256`, `authorization`, and `authorization_sha256`.
It additionally requires `source_handoff` and `source_handoff_sha256`. All four
paths are copied privately and all four hashes must match. Artifact
and neutral expectation formats remain caller-owned; their bytes are opaque to
this app module and must be independently validated by that caller.

The authorization JSON is an exact object with integer `schema_version: 1`,
`kind: caller-owned-allowlisted-paperless-target`, `application: paperless-ngx`,
the exact `prefix`, `run_id`, `endpoint`, and `profile`, and SHA-256 bindings
`plan_sha256`, `artifact_sha256`, `neutral_expectations_sha256`, and
`source_handoff_sha256`. It also requires `authored_fields_reviewed`, `loss_policy_reviewed`, and
`isolated_test_harness_only` to be true. These are explicit caller custody and
review assertions, not a signature, native admission, or application proof.

This stage distinction supports a future admitted migration harness without
granting arbitrary generated-plan replay. A canonical source fixture receipt
cannot stand in for target execution. Acquisition, artifact export, migration
state transfer, independent neutral expectation interpretation, and comparison
of source and target results remain the caller's responsibility. The app module
does not claim an end-to-end migration has run.

The source stage seeds and verifies exactly three documents, then requests a
`handoff`. Its payload is an exact `paperless-source-handoff` object containing
`schema_version: 1`, `application: paperless-ngx`, source `prefix` and `run_id`,
`plan_sha256`, canonical `probe_sha256`, and the actual `source_state` read from
`/fixture/probe-state.json`. The module validates that state's three baseline
PDF/DOCX/ODT records against the canonical deterministic generator, including
titles, body search markers, original byte hashes, distinct positive document IDs,
and DOCX/ODT archive hashes. The caller retains this exact handoff outside the
module's temporary directory. The module separately holds the last successful
source verification receipt's document state and retains its exact byte hash in
the orchestration scope. Later handoff state must equal that verified observation,
including document IDs and archive hashes; syntactically valid replacement
identities are rejected. The held verification state is rechecked across all
subsequent callbacks. On success the source invocation prints the SHA-256
of its canonical compact JSON encoding with a final newline; that retained byte
encoding must match the target's handoff hash.

The target must install that held source state before its first verification.
It verifies the same three records and source SQL count before any ingest,
recreates all five containers, verifies those three records again, and only then
adds the `second` batch and verifies six records. Missing prepopulated state,
changed source document IDs, original bytes, or archive evidence fail before any
new ingestion. No reset, deletion, or target-empty invariant is allowed.

## Request and receipt contract

Every request carries integer `schema_version: 1`, `action`, string-array
`arguments`, `stage`, the private `manifest` path and `manifest_sha256`, exact
`endpoint`, `deadline_ms`, and unchanged `work_deadline_ms` and
`cleanup_deadline_ms`. Work requests use the work deadline; cleanup uses the
reserved cleanup deadline. The manifest's `native_execution: false` describes
source evidence, not a runtime transport observation.

Every reply has exactly `schema_version`, `request_sha256`, `manifest_sha256`,
`action`, `stage`, `endpoint`, and `payload`. Reply hashes bind the request bytes
held before the transport call; rewriting the request cannot authorize a new
operation. Replies larger than one MiB, symlinks, duplicate JSON keys, malformed
JSON, changed identities, nonzero callback statuses, and late results fail.

The caller must cap its own writes, commands, and processes. Post-return deadline
checks do not interrupt a hung callback. The required caller watchdog must stop
and reap descendants, reserve cleanup time, and propagate uncertainty. A receipt
asserting that a watchdog exists does not prove that it works.

| Action      | Payload and caller responsibility                                                                                                                                                             |
| ----------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `preflight` | Empty `collisions`; true `process_tree_watchdog`, `absolute_deadlines`, `cleanup_reserved`, `no_fallback`, `no_implicit_pulls`; actual application `budget`; inspect before creating anything |
| `provision` | Complete observed resource graph, pinned image/config bindings, dependency barriers, settings, and application-user storage access                                                            |
| `inspect`   | Repeat the complete observed payload; must equal the held current graph                                                                                                                       |
| `probe`     | Run the held canonical `document-probe.py` with the exact shared callback arguments; return integer `exit_code: 0`, its exact `probe_sha256`, and actual `state` (null for readiness)         |
| `broker`    | Integer `lpush_calls` from actual authenticated Valkey `INFO commandstats` LPUSH activity, never a fabricated increment                                                                       |
| `query`     | String `rows` from the exact supplied PostgreSQL count query; use private password transport                                                                                                  |
| `privacy`   | Actual default export/report bytes in `report` and nonempty string-array `artifacts`, with successful `BFP0002` environment omission evidence and no protected canaries                       |
| `handoff`   | Source stage only: retain and return the exact origin-bound three-document handoff                                                                                                            |
| `recreate`  | Replace all five containers, then return complete observations with retained network IDs and named-volume custody identities                                                                  |
| `cleanup`   | Exact reconciled created/removed ledger, no remaining resources, no unowned resources touched, and no uncertainty                                                                             |

Probe placement belongs to the transport: use the held pinned Paperless image,
no pulls, the owned inner endpoint, the private fixture mount, and public test
credentials. Run the actual held probe rather than substituting a boolean receipt.
The shared probe checks PDF/DOCX/ODT ingestion, asynchronous completion, searchable
body text, byte-exact original retrieval, and PDF archives for DOCX and ODT.
Broker and SQL adapters must report actual observations; fakes cannot establish
those application properties.

The preflight budget has integer `cpus`, `memory_kib`, `disk_kib`, and
`image_archive_bytes` measurements. Capacity minima and compressed image archive
maximum are read from the canonical Paperless runner's literal declarations,
held with the source, and enforced without copying operational values. The caller
must measure its owned application cell and preserve those observations; a fake
budget receipt is only a source-contract control.

Graph payload fields are exactly `resources`, `dependencies`, `storage`, and
`image_bindings`. Resource keys are the complete thirteen expanded canonical names
(five containers, six volumes, two networks). Each observation has `kind`,
`identity`, exact ownership `labels`, and `specification` matching its canonical
application node. The transport must derive this normalized specification from
observed native fields, including actual mounts, private service ingress,
loopback-only web ingress, aliases, network membership, internal backend isolation,
and absence of extra peers. Echoing authored expectations is not native evidence.

Containers additionally have `image_config_id`, `running: true`, `healthy` (true
for db/broker, false for services without a healthcheck), and exact `settings`.
Settings contain observed `environment`, `command`, and `pull_policy: never`.
The converter disables JavaScript and permits only the reviewed local file path;
worker counts and user mapping match the canonical fixture. Image bindings map
each image key to exact pinned `reference`, private `alias`, and observed
`config_id`. Manifest digests and configuration IDs are distinct identities.

Network/container identities are observed 64-hex native IDs. Volume identities
are caller-custody incarnation tokens, not fabricated Docker volume IDs. The
caller must detect removal/recreation of a same-named volume independently.
Storage observations cover data, media, consume, and export: UID/GID 1000,
non-world-writable, actual application UID/GID 1000 read/write/create/remove
success. Dependency receipts bind all four canonical web dependency barriers;
the caller must observe their satisfaction before starting the web application.

The source sequence is readiness, baseline ingest, broker increase, document
verification, exactly three SQL rows, privacy export, handoff, and cleanup. The
target sequence is readiness, verification of pre-existing source documents,
exactly three SQL rows, privacy export, replacement of all five containers,
readiness and verification of prior documents, exactly three rows, second ingest
and broker increase, verification and exactly six rows, repeat graph/privacy
inspection, and cleanup. SQL uses the existing shared Paperless count callback.
Resource/image identity drift fails at every graph inspection. Probe receipts
carry actual parsed state and an independently supplied expected count in the
request, so verification cannot silently accept a subset. The target's baseline
records must equal the held source records throughout.

Cleanup liability starts before provisioning, including failure without a reply.
A pre-held cleanup request includes exact label and resource-name/kind scope, so
the caller can reconcile its own ledger even when held work inputs were damaged.
Cleanup cannot depend on successful work replies. The payload has exactly
`created`, `removed`, `remaining`, `unowned_touched`, `reconciled`, and `uncertain`.
Created/removed entries are exact `name`, `kind`, `identity` objects. Every
observed incarnation must be removed. Partial creation requires caller ledger
reconciliation; missing ownership, unknown status, or uncertain cleanup fails.

## Dependency and architecture review

No versions, image pins, downloads, providers, dependencies, or workflows change.
The module consumes `paperless-ngx-application/images.tsv` and `providers.tsv`
through the existing source catalogue; it neither downloads nor invokes a
provider. `.github/renovate.json` already extracts the five Paperless image
tag/digest declarations through its live-application image regex manager.
The application/workload image rule groups those changes, requires dashboard
approval, and disables automerge. The canonical checksum-pinned Compose provider
manager remains on `scripts/lib/compose-provider.sh`, with explicit checksum and
native revalidation review. This new consumer holds those existing files without
introducing a parallel manager, version declaration, or copied integrity pin.

ADRs 0043, 0044, and 0079 remain unchanged: exports do not execute product output,
test-only caller transport is explicit, and native backend selection and
qualification stay outside this app-owned module. No architecture decision is
superseded by this source-only preparation.

Focused validation is `bash scripts/test-docker-paperless-application.sh`, plus
shell syntax/format/lint and Markdown checks on the changed files. The hermetic
test uses independently authored input plans, literal callback/phase expectations,
and counterfactual receipts. The complete repository gate remains required
before publication; focused source tests are not that gate.
