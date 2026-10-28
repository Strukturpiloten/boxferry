# Prepared Forgejo inner-Docker profile

`scripts/lib/docker-forgejo-application.sh` is a prepared test-only source profile
for issue #366, unregistered with the real application dispatcher and admission.
Its hermetic contracts are registered in the complete local gate and the
full-plan-only parallel CI job under [ADR 0086](../../../docs/decisions/0086-prepared-docker-forgejo-application-profile.md).
Sourcing it defines `docker_forgejo_application_run` and
performs no I/O. It is not a product execution API, runtime admission, application
acceptance receipt, migration result, or replacement for the historical core
oracle. The empty current runtime oracle and existing admission scope remain
unchanged. No real runtime was launched to validate this source slice.

The profile validates canonical source hashes, the independent authored graph,
interpolation expectations, native target context, admission bindings, and the
SHA-bound dependency sidecar through the existing offline helpers. Extra
environment assignments and native command/default overrides are refused before
native access: the offline helpers intentionally do not assess those fields.
It applies the emitted request method/path/body in original order; explicit
image loading, external edge/peer provisioning, dependency waits, starts,
recreation and cleanup belong to this test harness. Recreation reuses emitted
container and attachment requests while retaining both declared volumes.
Image inspection refuses default anonymous volumes outside the declared mounts
before any container is created, so cleanup never inherits an untracked volume.

The existing shared Forgejo semantic callbacks execute health, administrator,
Git and exact repository-count probes. The catalogue-bound Git fixture performs
private repository creation, commits, HTTP push/clone, SSH clone/commit/push,
independent proof-file comparison, and head verification over both transports.
Both transports and the exact database row are checked again after recreation.
Runtime observations check the internal backend, external edge, peer exclusion,
loopback publications, exact named writable mounts, running state and ownership.
A separate active check uses the existing PostgreSQL image's `pg_isready`, with
its version checked against the canonical image catalogue. Successful backend
connections bracket a denied connection from a probe sharing the exact excluded
peer's network namespace. All connections target the canonical numeric IPv4 from
the owner-bound database endpoint, avoiding DNS ambiguity. Each attempt retains
the original execution deadline and uses a two-second client timeout. Both
backend checks must return exit 0 and the exact accepting-connections message;
the peer check must return exit 2 and the exact no-response message. A rejecting
or down database, wrong output/version, missing utility, invalid parameters,
transport failure, or expiration of the original deadline causes failure.
The denied observation describes only that bracketed reachability window.

Isolation probe containers have exact run/application labels, a unique tracked name,
read-only roots, dropped capabilities, and an explicit bounded tmpfs over the
PostgreSQL image's declared data volume. Other undeclared image volume layouts
are refused before container creation. Backend probes use the owner-checked
network ID; peer probes use `--network container:ID` with the owner-checked peer
ID. Every probe records liability before execution and verifies absence after
`--rm`; cancellation/failure leaves exact ID/label-checked cleanup responsibility.

The probe contract follows the official
[PostgreSQL 17 `pg_isready` options and exit statuses](https://www.postgresql.org/docs/17/app-pg-isready.html),
[Docker container network sharing](https://docs.docker.com/engine/network/#container-networks),
and [bounded tmpfs mount options](https://docs.docker.com/engine/storage/tmpfs/#options-for---mount).
The existing authored PostgreSQL healthcheck already requires this utility;
runtime version and positive controls independently verify its availability for
this profile. These references establish command semantics, not live evidence.

A duplicate emitted container request must return a conflict without identity
change. A successful hermetic transport response proves only source choreography.

## Caller contract

The function takes exactly sixteen positional arguments:

1. Absolute executable path to the reviewed inner-runtime transport.
2. Opaque transport context identifier (letters, digits, `_`, `.`, `-`).
3. Repository root with the canonical reviewed fixture catalogue and sources.
4. Generated Docker plan JSON file.
5. Independently bound offline admission JSON file.
6. Dependency sidecar JSON file.
7. Independently selected target profile JSON file.
8. Independently selected Forgejo/PostgreSQL private image-alias JSON file.
9. Independent private interpolation-expectations JSON file.
10. Selected `upstream-rootless` or `upstream-rootful` lane.
11. Unique caller-owned run identifier.
12. Unique caller-owned prefix.
13. Absolute private fixture directory inside the isolated runtime boundary;
    only letters, digits, `_`, `.`, `-` and `/` are accepted, without dot segments.
14. Original absolute integer `CLOCK_BOOTTIME` execution deadline.
15. Original absolute integer `CLOCK_BOOTTIME` cleanup deadline.
16. Disposable private administrator password.

Execution must expire within one hour of entry; cleanup must expire after
execution and at most ten minutes later. Every call retains those original
deadlines. The runner bounds each subprocess's combined output to one MiB,
suppresses native diagnostics and protected values, interrupts input/output
stalls, kills and reaps its transport process group, and handles INT, TERM, HUP
and QUIT. Cleanup failure overrides success. Further cancellation during cleanup
does not interrupt its bounded attempts. SIGKILL, host loss and unknown resource
identity require the caller's independently bounded recovery guard; the profile
cannot guarantee cleanup after those events.

The caller must establish the reviewed outer/inner runtime, independent
candidate/source evidence, actual daemon selection and identity, finite resource
budgets and storage observations, private transport configuration, an original
deadline recovery guard and an exact-resource recovery ledger **before entry**.
An attestation is a boundary prerequisite, never native evidence by itself. The
profile independently reads the daemon version/API and rootless security option.
It does not install or discover Docker, select credentials, choose fallback
transports, launch an outer runtime, use a host socket, or prune resources.

## Transport protocol

Each invocation is `TRANSPORT CONTEXT ABSOLUTE_DEADLINE OPERATION [ARGUMENTS...]`.
The deadline uses the caller's BOOTTIME clock domain. The transport is a reviewed
test executable, not a user command or arbitrary shell expression. It must
enforce deadlines and resource/output bounds independently, route exclusively to
the attested inner daemon, and return nonzero on transport/native failure.
It must not log credentials, raw CLI argv, API bodies or probe stdout/stderr to
retained artifacts. API errors may contain secrets and are discarded by the
profile. The public result is only an exit status with a fixed failure message.

Operations are closed:

| Operation         | Arguments and stdin                                                  | Successful output / obligation                                                                                                                                                                                                                                                                                                                                                                           |
| ----------------- | -------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `attest`          | Closed JSON context on stdin                                         | Exact JSON object after independently verifying it against caller-owned context. Fields: `protocol` = `boxferry-docker-forgejo-transport-v1`, context/run/prefix/owner/lane/profile/fixture root, `docker_plan_sha256`, `dependency_sidecar_sha256`, `expectations_sha256`, both deadlines, and true `isolated_inner_runtime`, `caller_owns_cleanup_guard`, `no_host_socket`, `no_credential_discovery`. |
| `api`             | Method, exact versioned path; optional JSON body on stdin            | Exactly `{"status": INTEGER, "body": JSON_VALUE}`. Do not follow redirects or retry mutations. Preserve HTTP errors as status observations.                                                                                                                                                                                                                                                              |
| `cli`             | Exact inner-client argv, no runtime executable name                  | Raw bounded stdout, original native exit status. No ambient socket, context, registry credentials or fallback.                                                                                                                                                                                                                                                                                           |
| `image-provision` | Existing catalogue's immutable reference, private alias              | Refuse an existing alias; record liability before loading; verify immutable digest and alias binding. No new image pins.                                                                                                                                                                                                                                                                                 |
| `image-release`   | Exact private alias                                                  | Idempotently close only that context's previously recorded image liability, preserving unrelated images. Verify closure.                                                                                                                                                                                                                                                                                 |
| `fixture-create`  | Exact private inner fixture directory                                | Refuse preexisting/unowned directory; private permissions; durable liability before creation.                                                                                                                                                                                                                                                                                                            |
| `fixture-install` | Directory, closed fixture basename, expected SHA-256; bytes on stdin | Verify digest, use create-new regular files without symlink traversal. Only `git-probe.sh` and `repository-proof.txt`.                                                                                                                                                                                                                                                                                   |
| `fixture-remove`  | Exact private directory                                              | Remove only the context's recorded fixture/probe/key state; verify absence. Never recursively remove a caller-selected unowned path.                                                                                                                                                                                                                                                                     |

Every transport operation must preserve an independently recoverable owner ledger
even when it fails after a side effect. Image aliases and fixture liability are
recorded locally before their calls; exact API resource liability precedes each
create. Native resources must be absent before creation. Cleanup reads exact
name/ID plus run/application labels, refuses identity drift, removes only recorded
resources, and verifies absence after each deletion. It never removes volumes
during recreation and never force-deletes volumes. Unknown ownership is retained
for explicit recovery and causes failure.

Resource lookups always include kind: the database container and volume share a
name but retain distinct ownership records. Both retained volumes are checked
before either initial or recreated container is created. Every secret-bearing
exec uses a recorded full container ID after checking that object's labels and
the current name-to-ID binding. Git refreshes the live running state, exact
loopback publications and topology before each credential-bearing client starts.
Image aliases and emitted network/volume/container
name references are checked against their recorded bindings immediately before
use; emitted requests themselves are not rewritten.

## Integration and evidence prerequisites

The primary-owned integration must review and provide a real transport,
independently review the final source and run the complete repository gate after
all edits. Only the dedicated offline contracts are registered: once in
`scripts/check-all.sh` and in the parallel `docker-forgejo-application` CI job.
Full plans, main/dispatch and reusable Release CI select it; focused documentation
plans retain an explicit planned skip. The measured local cost is 275.571 seconds,
with a ten-minute CI host backstop, not a measured native application budget.
The hermetic suite `scripts/test-docker-forgejo-application.sh` launches
only inert local scripts and tests explicit argv/order, emitted-operation binding,
refused fields, ownership, collisions, isolation, privacy, injected failures,
cancellation and cleanup.
The fault matrix selects one representative per independently required boundary
and error class from the complete successful choreography, including the actual
first SQL call and cleanup-only delete/fixture/image operations. Separate hostile
controls cover partial creation, ownership drift, stopped applications, isolation
and deadline/cancellation states; it does not replay every transport prefix.
It cannot establish native behavior, resource budgets,
Docker-target migration, provider comparison, reacquisition/loss expectations or
current runtime qualification. Those need separately authorized exact-candidate
runtime admission and full application evidence before any acceptance claim.

Renovate review: the profile files declare no software version, download,
workflow/action pin or independent image catalogue. Immutable image
references are read only from the existing Forgejo `images.tsv`, whose canonical
catalogue/source SHA binding is checked before execution. The CI job reuses the
existing SHA-pinned checkout alias and unchanged hosted runner under their
existing Actions and runner managers; no new extraction owner, approval/grouping
rule, provider pin, historical oracle or manifest change is needed. Release
already calls reusable full CI. No Lens product or website consumer needs an edit
for this BoxFerry-owned application contract; shared planner semantics are unchanged.
