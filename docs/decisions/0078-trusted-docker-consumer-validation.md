# ADR 0078: Trusted exact-pair Docker consumer diagnostics

- Status: proposed; production authorization and trusted-main bootstrap pending
- Date: 2026-10-18
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
write/maintain/admin roles. A Box token denied dependency API access fails closed:
there is no token fallback or permission inferred from Box membership. Both heads
and actors are re-admitted before every privileged job, immediately before runtime,
and in the fresh aggregate. Tokens exist only in admission steps; checkout never
persists credentials.

All authorized Box execution/import files are copied into a separate canonical
`scripts/lib` tree before import/source. This includes the harness, contract,
core-artifact, application-schedule, application-expectations and bounded-native-read
modules, relevant fixture/expectation files, producer examples and inert test source.
Repository-relative paths and `__file__` layout remain intact; no candidate
validator is selected by an uploaded path. A separate real clean exact Lens checkout
retains existing Git guards. Its native Unix-HTTP helper, canonical native image
assignments, reviewed catalogue and all four referenced records are explicitly
authorized before use. Never fabricate Git metadata for a partial Lens tree.

Build only the existing locked two-job volume producer and host-metadata recipes,
using an isolated test-only Lens Cargo patch. Published manifests and main are not
overridden. Discover exactly one selected test and require one actual successful,
nonignored execution for each of:

- `full_nextcloud_and_explicit_supabase_bind_mount_only_projection_retain_binds_for_all_profiles`
- `generated_external_constraint_controls_retain_actual_fixture_binds_on_all_four_profiles`

The live launcher has no profile override: it invokes only `volume-fixtures`.
Existing capacity preflight (6 GiB available RAM/8 GiB free disk), 512 PID/4 GiB/
2 CPU outer limits, owned socket/storage, 780-second work/120-second cleanup
watchdogs, lane-specific nesting exceptions and exact ownership/absence checks
remain unchanged. Do not relax the unconfined/host-policy guard or contact ambient
host Docker. Capture build/runtime output privately; never upload raw captures/logs.
Build and inert-test subprocesses have owned process-group cleanup before the
leader is reaped. The native backstop first signals its owner and preserves its
cleanup reserve rather than prematurely terminating the existing watchdog.

Upload only bounded, closed sanitized `volume-proof.json` and `consumer.json`.
The volume proof retains the existing 16 KiB bound; its exact canonical uploaded
bytes are SHA-256-bound into the current-run consumer envelope, preventing a
separately completed same-head proof from being substituted beside that envelope.
The new trusted read-only validator is not the existing runtime evidence writer.
A fresh aggregate rechecks admission, exact source and authorized closure, constructs
its own verifier tree, and requires all four complete current-run/current-attempt
artifacts. It rejects failed/skipped/cancelled/missing lanes, replays, wrong heads/
lanes, partial fixtures or 23-volume inventories, incomplete cleanup, and changed
source/lock/build/binary bindings. It imports no candidate validation module.
Sanitized opaque cleanup/source assertions are not independent raw host inventory.

## Consumers and dependency ownership

The existing application dispatcher, PR/main/full Release gates and native suites
are unchanged. BoxFerry owns this consumer/application diagnostic; other Lens
products and the website do not consume it and gain no BoxFerry dependency.
Reuse the existing full-SHA/exact-tag checkout/upload/download Actions and the
independently pinned admission revision. Extend the existing Renovate admission
manager and approval/security-review rule to this workflow; add no duplicate
manager or copied Docker image pin. Canonical image refs remain DockerLens-owned.
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
