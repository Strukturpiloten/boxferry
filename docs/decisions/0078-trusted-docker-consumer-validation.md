# ADR 0078: Trusted exact-pair Docker consumer diagnostics

- Status: proposed; production authorization and trusted-main bootstrap pending
- Date: 2026-10-18
- Updated: 2026-10-25
- Builds on: [ADR 0066](0066-metadata-only-reviewed-pr-catalogue-admission.md)

## Context

The existing reviewed-application dispatcher selects only Nextcloud, Paperless and
Immich application tasks. It does not authorize a candidate's changed Docker
harness, unconditional Python imports, producer examples or validator merely
because those filenames exist on main. Generated build/source receipts establish
identity, not authority. The Docker consumer needs a separate bounded diagnostic;
importing its entire multiprofile implementation into main would also import
unrelated Forgejo/core execution policy.

## Decision

Prepare a manual trusted-main-only Docker consumer workflow, separate from the
existing application dispatcher and complete PR/main/Release gates. Its only
scope is six actual authored fixtures and 23 volume requests on each fixed Debian/
upstream rootful/rootless lane, plus the two explicitly named authored bind and
external-constraint tests. Those tests remain inert: no host bind-path inspection,
SELinux effect, runtime isolation, application readiness or full migration claim.
No arbitrary task, core journey or Forgejo application is selectable.

Before activation, trusted main must contain one explicit authorization in
`.github/reviewed-docker-consumer-authorizations.json` for the actual published,
open BoxFerry and DockerLens PRs and their full current heads. There is no
placeholder authorization, self-authorizing receipt or fallback to filenames.
The closed version-1 manifest supplies literal repository names and numeric IDs,
PR numbers, full revisions, source/lock identities and reviewed per-file SHA-256
authorization for the complete script/import/data closure. Its policy must equal
the fixed trusted scope, four lanes, fixture/volume counts, tests and budgets.
Changed heads, closure bytes or policy require renewed review and authorization.

The Box workflow's real repository, ID, event, ref and original/rerun actors are
never fabricated. Normal Box admission uses the independently pinned shared
DockerLens helper. A narrow explicit dependency adapter uses that same helper's
bounded fetch/path/permission contracts only for literal DockerLens pull and
collaborator-permission paths. It independently checks the manifest's Lens ID/name,
open same-repository PR, base main, exact number/head and both real actors' current
write/maintain/admin roles. Literal Box requests use only the Box workflow token;
literal Lens requests require a separate explicit Lens installation token. Missing
tokens or unknown repositories fail closed, with no fallback or permission inferred
from Box membership. Reuse the existing `RELEASE_PLZ_APP_CLIENT_ID` variable and
`RELEASE_PLZ_APP_PRIVATE_KEY` secret references with the already pinned App-token
action. Fix its owner to `Strukturpiloten`, repository list to `docker-lens`, and
permissions to Metadata read and Pull requests read only. App installation coverage
and reference availability must be established by the hosted admission; host `gh`
access is not evidence. This change creates no stored secret or installation policy.

There are ten complete admissions: initial, before and after each of four builds,
and aggregate. Every admission runs on a fresh trusted hosted runner which never
executes candidate code or transferred binaries. Each requires successful explicit
`DELETE https://api.github.com/installation/token` revocation before dependent work.
An always-run cleanup step attempts revocation after failures and cancellation;
the pinned action's warning-only job-end cleanup remains a backup, never the
success barrier. Unknown revocation blocks all dependent work. Runner loss can
prevent cleanup requests; such a job cannot authorize work, and normal token
expiry is the residual bound. The App private key remains on trusted runners.
Checkout never persists credentials.

Admission failures report only a fixed phase and an explicitly allowlisted, value-free
pinned-helper reason; unknown exceptions become `rejected`. Authorization, workflow pin,
helper load, Box context/admission, and each repository's permission/pull checks remain
distinct. These labels identify a failed boundary, not an HTTP status or proven root cause:
the canonical fetch helper discards non-success status details. No exception text, API
payload, request path, actor or token is emitted; no alternate credential is selected.

## Isolated jobs and closed build transfer

The dispatcher calls a trusted same-commit lane orchestration workflow. Its
pre-build admission gates a fresh reusable candidate worker; a fresh data-only
sealing job then validates that build, re-admits both actors and heads, and revokes
the token before a second fresh worker can perform native execution. Four lanes
therefore require 19 hosted runner jobs including a context-only smoke, initial
admission and aggregate.
Worker calls declare and forward no stored secrets and select no environment.
GitHub still supplies its ordinary read-only workflow/artifact credentials;
candidate subprocess environments exclude those credentials. A composite action
shares the trusted admission lifecycle but is not an isolation boundary.

Nested calls must retain the real caller's repository/ID, event, ref, original and
triggering actors, workflow ref/SHA, run and attempt. The worker's actual job ID
remains `lanes`. A context check before candidate checkout/build refuses any
different hosted behavior; do not fabricate environment values to accommodate it.
The dispatcher's first dependency runs that check through both reusable workflow
levels without forwarding stored secrets. Normal admission cannot begin until it
succeeds. Dispatch with `context_only: true` to run only this diagnostic, without
App authentication, candidate checkout, build or native work. The existing four
pair inputs remain required, but this route establishes caller context only;
it does not admit or qualify those candidate heads. The actual hosted run is a
required bootstrap checkpoint; syntax and synthetic context tests are insufficient.

The one-day build artifact contains exactly canonical `manifest.json` and three
regular binaries named `volume-fixtures`, `host-metadata`, and `inert-tests`.
Files are bounded at 512 MiB each and 1 GiB total; JSON is bounded at 128 KiB.
No links, special files, duplicate keys, extra names, logs, source trees, Git
metadata, Cargo caches, private receipts, Actions state or caller-selected paths
are accepted. The manifest binds the complete authorized pair/closure/source/lock
identity, trusted revision and authorization digest, actual run/attempt/lane/job,
fixed build recipes/layout, byte hashes/sizes, and the two successful inert tests.
The successful build job remains compilation evidence; artifact hashing is not
compiler attestation against a malicious builder.

Build and native runners assert the fixed hosted workspace layout
`/home/runner/work/boxferry/boxferry`; source paths are independently derived there.
The importer validates all bytes before installing executable bits at fixed
destinations, copies the authorized execution closure, and reconstructs canonical
private receipts without claiming compilation happened in the importing job.
Layout mismatch fails closed rather than rewriting uploaded command paths.

Post-build sealing downloads only the immutable numeric artifact identity from
its direct successful build dependency. It reads uploaded files strictly as data,
re-derives source/recipe/context facts, and issues a trusted binding only after
final admission and successful revocation. The native worker receives that binding
through its successful direct dependency and rechecks actual downloaded bytes.
Each download uses current-run scope, with no cross-run credential/ref override,
merge, overwrite or latest-run fallback. The pinned download action uses explicit
`digest-mismatch: error`; independent byte checks are mandatory as well. The
binding is also saved for one day under a separate per-lane artifact namespace.

Admission time is recorded after the last canonical GET and before revocation;
the binding expires 120 seconds later. Native import and the final execution
handoff both reject stale, future or other-attempt bindings. Source checkout and
artifact download consume that same budget. Queue delays require a new complete
attempt, not an automatic expiry extension. This bounds the existing check/use
race; it does not claim atomic permission validity during execution.

All authorized Box execution/import files are copied into a separate canonical
`scripts/lib` tree before import/source. This includes the harness, contract,
core-artifact, application-schedule, application-expectations and bounded-native-read
modules, relevant fixture/expectation files, producer examples and inert test source.
Repository-relative paths and `__file__` layout remain intact; no candidate
validator is selected by an uploaded path. A separate real clean exact Lens checkout
retains existing Git guards. The volume producer's imported
`tests/support/docker_volume_fixtures.rs` is also an authorized Box source input.
Its native Unix-HTTP helper, canonical native image
assignments, reviewed catalogue and all four referenced records are explicitly
authorized before use. Never fabricate Git metadata for a partial Lens tree.

Build only the existing locked two-job volume producer and host-metadata recipes,
using an isolated test-only Lens Cargo patch. Published manifests and main are not
overridden. Discover exactly one selected test and require one actual successful,
nonignored execution for each of:

- `full_nextcloud_and_explicit_supabase_bind_mount_only_projection_retain_binds_for_all_profiles`
- `generated_external_constraint_controls_retain_actual_fixture_binds_on_all_four_profiles`

Cargo may hardlink each executable's normal name to its hashed compiler-cache
alias. Before strict hashing, the build driver captures the two known example
outputs and the compiler-selected inert test executable into independent inodes
at their existing owned target paths. Only those build outputs are eligible:
canonical paths, current ownership, regular executable ELF marker/minimum size,
the existing 512 MiB limit, and the original deadline are checked. A held no-follow
descriptor supplies the bytes; inode/link/size/mode/timestamp checks reject changes
or path replacement during capture. A create-new same-directory snapshot is
flushed, atomically installed and checked by the unchanged strict digest reader.
Failed or cancelled captures remove their owned partial snapshot. Other compiler
aliases cannot mutate the captured bytes afterward. This capture is never applied
to downloaded artifacts; runtime, restore and transfer readers still reject
multiple links. It grants no repository or native authority on its own.

Build failures identify a fixed value-free phase from argument/context/source
checks through compilation, capture, receipts, inert tests and export. Raw compiler
output, paths, arguments and exception text remain private; a phase label is not
proof of a compiler error or native result. Both exact inert tests remain mandatory.

The live launcher has no profile override: it invokes only `volume-fixtures`.
Existing capacity preflight (6 GiB available RAM/8 GiB free disk), 512 PID/4 GiB/
2 CPU outer limits, owned socket/storage, 780-second work/120-second cleanup
watchdogs, lane-specific nesting exceptions and exact ownership/absence checks
remain unchanged. Only the Docker-backed `upstream-rootless` volume lane uses the
canonical DockerLens `github_runner_userns_v1` prerequisite, under the actual
BoxFerry workflow/job/run/attempt identity. It prepares and loads a unique
run-owned AppArmor profile, binds its nonce to creation, verifies the live label,
and removes the profile after exact outer teardown, before storage cleanup.
The existing deadlines also bound these phases; no timer is renewed. Unknown
load/create/unload completion fails closed and retains private custody and the
run directory. Debian-rootless, rootful, local and self-hosted execution gain no
profile authority. Do not relax the unconfined/global host-policy guard or contact ambient
host Docker. Capture build/runtime output privately; never upload raw captures/logs.
Build and inert-test subprocesses have owned process-group cleanup before the
leader is reaped. The native backstop first signals its owner and preserves its
cleanup reserve rather than prematurely terminating the existing watchdog.

Upload only bounded, closed sanitized `volume-proof.json` and `consumer.json`.
The volume proof retains the existing 16 KiB bound; its exact canonical uploaded
bytes are SHA-256-bound into the current-run consumer envelope, preventing a
separately completed same-head proof from being substituted beside that envelope.
Schema 2 also requires `runner_prerequisite`: exactly `required=false` on the
other three lanes, or the closed Box consumer receipt on `upstream-rootless`,
bound to the exact Lens source and current run/attempt, with verified label/outcome
and removed cleanup. Native receipts, prepared-only receipts and earlier-attempt
receipts cannot substitute. The bounded canonical prerequisite and
outer-observation records must immediately precede the successful final log
line; no raw receipt file is uploaded. This prerequisite is not native
product/application evidence.
The new trusted read-only validator is not the existing runtime evidence writer.
A fresh aggregate rechecks admission, exact source and authorized closure, constructs
its own verifier tree, and requires all four complete current-run/current-attempt
artifacts. It rejects failed/skipped/cancelled/missing lanes, replays, wrong heads/
lanes, partial fixtures or 23-volume inventories, incomplete cleanup, and changed
source/lock/build/binary bindings. It imports no candidate validation module.
It also requires exactly four separate trusted bindings, distinct immutable build
artifact identities, and each result's exact build/inert hashes from that binding.
Candidate-authored result digests cannot authorize themselves.
Sanitized opaque cleanup/source assertions are not independent raw host inventory.

## Consumers and dependency ownership

The existing application dispatcher, PR/main/full Release gates and native suites
are unchanged. BoxFerry owns this consumer/application diagnostic; other Lens
products and the website do not consume it and gain no BoxFerry dependency.
Reuse the existing full-SHA/exact-tag checkout/upload/download Actions and the
independently pinned admission revision. Extend the existing Renovate admission
manager and approval/security-review rule to this workflow; add no duplicate
manager or copied Docker image pin. Canonical image refs remain DockerLens-owned.
The new reusable workflows and `.github/actions/reviewed-docker-admission/action.yml`
use the native `github-actions` manager's existing workflow/composite patterns.
The App action reuses the exact release-plz SHA/tag; existing Action pinning,
grouping and reviewed update policy apply once. The admission revision remains
canonical in the dispatcher with its existing separate approval rule. No new
Renovate manager, extraction path override or software version is needed.
The compiler-capture repair uses only the standard library and owning driver/tests:
no dependency, Action/tool/image pin, workflow consumer or extraction path moves.
Existing single-manager ownership, grouping and approval rules therefore remain
unchanged; the catalogue's extraction and consumer regressions still apply.
Ephemeral outer-runner prerequisites intentionally follow Ubuntu 24.04's signed
distribution repositories rather than pinning package revisions that security
updates can retire. APT verifies repository metadata and package digests; there
is no invented standalone checksum or claim of a fully reproducible runner image.
This is the existing diagnostic bootstrap exception, not Docker version evidence:
the inner Engine images still require their exact Lens-owned versions/digests.
The new workflow reuses that existing installation pattern; no APT-version pin
or Renovate extraction location is introduced. Other dispatchers retain their
existing prerequisite sets and do not consume this Docker-only execution policy.

## Consequences and alternatives

The manifest/copy mechanism is not a sandbox against reviewed code running as root;
exact source and complete transitive execution review remain necessary. This bridge
cannot override a failed DockerLens native campaign or replace complete gates,
source publication, actual consumers, package resolution, application qualification,
release or deployment authorization.

Production authorization is intentionally absent during preparation until both
reviewed exact candidates are remotely checkoutable and the primary approves their
closure. Bootstrap then needs its own complete gate, independent review and trusted-
main merge before a real dispatch can claim any hosted result. ADR numbering 0068–
0077 belongs to the separately integrated source candidate; preserve those records
when combining the index, rather than renumbering or hiding their history.

The alternative is a reusable long-term Docker dispatcher framework. This narrow
explicitly reviewed bridge avoids importing unrelated multiprofile execution now.

## Maintained prerequisite and offline controls

The trusted-main bootstrap must include the Box consumer's fixed prerequisite
policy, six-file canonical Lens closure and schema-2 validator before admitting
the corresponding product/helper source pair. Renew exact source identities
after infrastructure integration, including the volume producer's imported
fixture-support source. The upstream-rootless lane additionally needs a fresh
Box-specific verified-and-removed prerequisite receipt for the same Lens SHA,
workflow run and attempt; prior native or prepared-only receipts are ineligible.
The remaining lanes explicitly report that no profile is required. No new image,
Action, tool or admission pin is introduced by this seam: existing DockerLens
image ownership, Action managers and the single admission-revision manager are
unchanged. This diagnostic does not alter PR/main/Release gates or qualify other
repositories' consumers.

The inert consumer controls cover the complete authorized source closure, actual
workflow/job/run/attempt identity, upstream-rootless-only profile selection,
prepare/load/create/verify/cleanup ordering, and private custody retention after
unknown cleanup or invalid receipts. They reject native or cross-source receipts,
attempt replay, incomplete cleanup, extra/missing fields, wrong types/digests,
and duplicate, noncanonical or misplaced final log projections. These controls
do not load AppArmor profiles or run a container runtime. Fresh hosted native
lanes and the complete gate remain separate requirements.
