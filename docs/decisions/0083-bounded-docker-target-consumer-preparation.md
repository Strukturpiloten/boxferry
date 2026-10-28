# ADR 0083: Bounded Docker target-only consumer preparation

- Status: accepted bounded prerelease preparation; complete-gate, exact-pair,
  native, actual consumer and published-dependency qualification remain pending
- Date: 2026-10-24
- Supersedes only: [ADR 0032](0032-future-native-lens-boundaries.md) decisions
  5, 6, 8 and 9 insofar as they defer a real Docker target adapter and its target
  diagnostic namespace; [ADR 0033](0033-universal-neutral-model-pipeline.md)
  decision 8 insofar as it defers this target-only library checkpoint
- Preserves: [ADR 0078](0078-trusted-docker-consumer-validation.md) exact-pair
  admission and [ADR 0079](0079-native-test-runtime-selection.md) staged bootstrap

## Context and history

Trusted consumer infrastructure and a qualified native source do not supply
BoxFerry product integration. The preserved issue-343 research checkout contains
251 changed paths spanning adapter, CLI, application, capture and infrastructure
work. It is not an authenticated candidate and must not be copied wholesale.
A minimal real target adapter can prepare the six-fixture volume rehearsal and
two authored bind/external-expectation consumers without adopting that work.

Numbers 0064 and 0068-0076 belong to preserved integration history and are not
silently adopted or renumbered here. The separately preserved source-daemon-context proposal remains unadopted. This decision
neither claims those implementations are delivered nor erases them.

## Decision

1. Add a real `boxferry-docker` target adapter and nondefault facade `docker`
   feature. The public slice is `DockerExporter`, `DockerOutput`,
   `DockerEnvironmentPolicy`, `DockerDependencyDecision`, `DOCKER_TARGET` and the
   native Lens reexport. No importer, source type, acquisition function, input stub,
   CLI route or default-feature change is introduced. Existing nine CLI routes,
   neutral types and the importer → neutral → exporter pipeline remain unchanged.
   Docker input/CLI and Kubernetes integration remain deferred.
2. BoxFerry maps cross-format meaning only. DockerLens owns typed native intent,
   validation, capability admission, planning and serialization. Every plan resolves
   an explicitly chosen exact reviewed profile; a numeric version range cannot
   substitute for native context/evidence. Unmapped intent has subject-level
   outcomes. Invalid references, protected non-environment values and withheld
   required environment cannot produce apparently complete service artifacts.
   Literal environment output has a separate explicit authorization.
3. Expose unchanged native complete bytes with conditional bind-source, external
   resource and destination obligations; requests alone are incomplete review
   material. Absence, false and true external isolation expectations stay distinct.
   Configured relabels or destination metadata do not prove suitable host paths,
   SELinux effects, enforcement, readiness or satisfied obligations. Typed service
   dependency decisions retain provenance and approximate outcomes separately;
   nothing executes them. Publish only BFD0005, BFD0006 and BFD0007 target rules,
   preserving existing discriminants/metadata and the sorted catalogue. Source BFD
   rules and the BFR runtime namespace remain deferred.
4. Prepare only the two validation examples. `docker-volume-fixture-rehearsal`
   imports unchanged actual authored Compose fixtures and explicitly projects
   volume declarations: Forgejo 2, Nextcloud 3, Paperless-ngx 6, Immich 4,
   Observability 5 and Supabase 3, totaling 23 inert requests per profile.
   Independently authored identities/labels/counts, embedded source hashes,
   private held-directory/create-new writes and bounded arguments/outputs remain.
   `docker-host-metadata` is an explicit-socket, bounded read-only Lens probe that
   discovers/selects no workload and introduces no facade acquisition API.
5. Retain both exact inert test names required by ADR 0078. Full Nextcloud crosses
   the production Compose importer, neutral model and exporter. Supabase is
   explicitly bind/mount-only after full import; full Supabase's protected-health
   refusal stays a separate negative control. Independently authored None/false/
   true external-network controls retain actual fixture binds without rewriting
   canonical topology. Closed schemas, identity/mount-index/provenance association,
   source obligations, redaction and omitted/substituted/malformed evidence controls
   remain. No host path is inspected and no generated request is applied.
6. The intended Lens dependency is the separately reviewed guarded CommandClear
   successor. Its historical source cohort is tested source
   `a36e42fbaeee6585c954948c9fe3a8686d86018b`, source run 40119803977 attempt 1,
   not its future admission head. Independent record-key literals identify that
   cohort in the bind tests. Only guarded CommandClear is added relative to the
   earlier StopSignal cohort; EntrypointClear remains denied. Neither these literals
   nor the older 5fef54d API audit qualify the mutable candidate's final head.
7. Register the real seventh publishable lockstep 0.9.0 crate in ordinary package,
   release preparation/publication, semver and Renovate inventories. Preserve all
   complete checks and thresholds; add only the standalone locked `ci-docker`
   check to local/CI feature boundaries. Do not change API baseline selection,
   comparison/release-type behavior, admission policy, trusted runner or harness.
   Registering publication metadata authorizes no publication or release.

## Source-pair and dependency contract

The clean candidate is prepared separately from preserved research. The primary
freezes real remotely checkoutable open BoxFerry/Lens PR heads, source/lock/config
identities and the reviewed execution/build closure after the required gates.
The intended successor is not interchangeable with source S or an older
API-compatible tree. Changed heads, bytes, records or policy require renewed
review/authorization. Trusted admission and four fixed lanes remain authoritative;
this change installs no authorization and supplies no runtime result.

Retain `core-reacquire-expectations.json` unchanged solely as historical data
required by the strict authorized closure; no current core journey, source
adapter, runtime acceptance or capability follows.

Manifests retain ordinary registry-shaped dependencies. A separately authorized
test-only exact Lens override and reconciled lock can support an isolated clean
prerelease source branch; they must not enter published manifests or main and are
not normal registry/package verification. The primary verifies source/configuration
identities and the unweakened complete gate, including its separate semver Cargo
home. No worker-provisioned override is part of this change.

`boxferry-docker` already has releases 0.3.0, 0.2.0 and 0.1.1. Normal registry
baseline comparison and existing pre-1.0/release-type rules are mandatory: no
first-publication exemption, API waiver, self-baseline, package exclusion or
release-type workaround. This target slice does not recreate historical source
APIs or promise compatibility without real comparison. Any failure blocks the
corresponding qualification. Actual compatible registry resolution and fresh normal
package/integration gates are required before normal main integration.

## Ownership, verification and consequences

Native runtime source, canonical image pins, four-lane launcher, cleanup,
privilege/admission guards and exact-pair control plane remain unchanged. No #366
application/listener/capture module, core journey, Forgejo outcome producer,
application dispatcher or expanded profile is adopted. No Lens or website gains
a BoxFerry dependency. No upstream native implementation is copied.

The existing Cargo manager owns new declarations, lock integrity and grouping.
The existing exact-name first-party release-age rule gains only boxferry-docker.
Release registration repeats the same full-SHA/exact-tag crates.io authentication
action under the Actions manager. Approval rules and other pins remain unchanged;
no duplicate manager, image/tool pin or integrity exception is introduced.
RustCrypto sha2 0.11.0 (MIT OR Apache-2.0) is dev-only hashing for the producer,
not product serialization or a runtime dependency of the docker feature.

Focused mapping, invalid/unsupported/protected/version-boundary, standalone facade,
producer, independent fixture and policy checks are development feedback. The
primary owns the complete gate, independent check-only verification, normal API
comparison, fresh final-Lens native campaign, trusted exact-pair authorization and
actual four-lane consumer rehearsal. All remain required. Inert test discovery
must find exactly one successful nonignored execution of each required named test;
missing, skipped and failed evidence is not success.

This checkpoint does not complete #343, implement all 16 routes, qualify six
applications or full migrations, or permit release, deployment, generated command
execution or application data transfer.

## Alternatives

Copying all 251 paths, adding an input stub, widening CLI/defaults, adopting
deferred modules, weakening API/package checks or presenting source receipts as
application acceptance are rejected. Waiting for full CLI integration would couple
bounded preparation to unrelated work; claiming this smaller checkpoint completes
that work would also be incorrect.

## Independent inert consumer contracts

The [target-only Docker checkpoint](#decision)
adds an optional library exporter, not new CLI routes. Its `docker_fixture_volume_labels` suite
independently checks six real authored fixtures and 23 volume requests per reviewed profile.
The `docker-volume-fixture-rehearsal` example produces only inert volume artifacts; the
`docker-host-metadata` example is a bounded explicit-socket read-only validation probe.
Neither is a product executor or evidence of application acceptance.

The two exact `docker_authored_bind_rehearsal` checks required by the trusted consumer are:

- `full_nextcloud_and_explicit_supabase_bind_mount_only_projection_retain_binds_for_all_profiles`
- `generated_external_constraint_controls_retain_actual_fixture_binds_on_all_four_profiles`

They preserve full Nextcloud and explicitly projected Supabase bind intent, configured relabels,
identity/provenance and None/false/true external-network obligations. Negative controls reject
protected, omitted, substituted and malformed evidence. They do not inspect host paths, prove
SELinux effects, execute readiness, qualify full Supabase, or satisfy the four-lane live rehearsal.
Ordinary registry semver, complete gates and fresh exact-source-pair admission remain separate.
