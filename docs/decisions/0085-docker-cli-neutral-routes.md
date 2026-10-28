# ADR 0085: Docker CLI neutral routes and bounded artifact publication

- Status: accepted development contract; integration validation and release qualification pending
- Date: 2026-10-25
- Supersedes only: [ADR 0032](0032-future-native-lens-boundaries.md) decisions 5, 6, 8 and 9 and
  [ADR 0033](0033-universal-neutral-model-pipeline.md) decision 8 insofar as they defer real Docker
  CLI integration; [ADR 0083](0083-bounded-docker-target-consumer-preparation.md) decision 1's
  CLI/default-feature deferrals and [ADR 0084](0084-bounded-docker-source-adapter.md) decision 1's
  CLI/default-feature deferrals
- Preserves: accepted target/source adapter contracts and checkpoint history under ADR 0083/0084;
  [ADR 0078](0078-trusted-docker-consumer-validation.md) exact-pair admission and
  [ADR 0079](0079-native-test-runtime-selection.md) runtime selection/staged bootstrap

## Context

The target checkpoint and optional source library do not themselves provide usable CLI routes.
The next bounded product slice composes those real adapters through the existing neutral pipeline.
It must preserve the native Lens boundary, separate reconstruction from loss authorization, and
publish complete review material without introducing an executor or claiming application success.
The separately preserved source-daemon-context proposal and unrelated research
application/capture/infrastructure work remain unadopted.

## Decision

1. Register all four inputs and outputs in the development CLI: 16 unique routes, including
   `DockerImporter → Application → DockerExporter`. CLI defaults and binary requirements include
   `cli`, `compose`, `docker`, `podman` and `quadlet`. The `docker` feature itself remains adapter-only;
   embedded callers retain `default-features = false` and explicit integration selection.
2. Keep DockerLens responsible for native protocol/acquisition, decoding, types, capability evidence,
   validation, planning and serialization. BoxFerry owns semantic mapping, orchestration, fidelity,
   diagnostics and file publication. No native parser or runtime abstraction is recreated in
   BoxFerry and no Lens product depends on it. Kubernetes remains deferred.
3. Require one exact reviewed `--docker-target` preset: Debian 11 Engine `20.10.5+dfsg1`, package
   `20.10.5+dfsg1-1+deb11u2`, or upstream Engine `29.8.1`, each rootful or rootless. Native APIs and
   evidence resolve from the selected profile. A numeric range, observed source mode or host facts
   cannot choose or establish the target. Preserve unknown observed source mode as evidence.
4. Acquire Docker input read-only through Lens using an explicit Unix socket or finite Linux lookup
   of `/run/user/<uid>/docker.sock` then `/var/run/docker.sock`. Do not infer remote environment
   endpoints, execute Docker, perform a connection-only preflight or fall back after a selected
   endpoint fails. Only one selector family is allowed. Without a selector, bounded metadata must
   identify a sole container or complete consistent application group; empty, one-off, mixed,
   conflicting and ambiguous discovery fails closed. The caller/default `docker-import` names the
   neutral application, not a native label. Each acquisition call has 128 requests, 32 selected
   resources, 64 expansions, 2 MiB/response, 16 MiB total and 15 seconds. Default discovery and
   inspection are separately bounded calls, not a 15-second whole-invocation guarantee.
5. Default CLI reconstruction to the reviewed portable current-state subset; keep conservative,
   unspecified library defaults. Reconstruction carries evidence and decision provenance and
   proves neither original authorship, target fidelity, ownership nor artifact consent. Exact loss
   remains the default. Approximate/partial policies authorize only their existing outcome classes;
   invalid input always blocks. Protected environment source promotion, literal environment output,
   same-host binds, exact public-label declarations and checked network/volume create/external
   decisions are independent controls. Environment consent never authorizes protected process or
   health values. Redacted/unavailable values are not empty configured values.
6. Emit complete inert `docker-plan.json`; emit `docker-dependencies.json` only for typed dependency
   decisions. Preserve conditions, provenance, `native_execution=false` and SHA-256 binding to the
   exact native plan bytes. Dependency conditions and external/bind/destination prerequisites are
   manual obligations, not Engine fields, readiness execution or proof of host suitability/SELinux
   enforcement. POSTs in artifacts are inert; actual acquisition is GET-only. Docker output is not
   observed input. Compose/Quadlet remain reimportable and existing Podman artifact names stay intact.
7. Publish owner-only/create-new Docker files only after policy and complete-candidate preflight.
   Required protected/incomplete intent may refuse output even under partial loss; no general
   secret-dropping promise is made. Protected dependency identities refuse the sidecar. A required
   sidecar must publish before the complete plan. Diagnostics, JSON reports and support ZIPs remain
   redacted. Validation uses the same plan without artifact writes; existing output is preserved.
8. Reuse the facade's existing exact RustCrypto `sha2 =0.11.0` requirement for optional normal
   CLI-side hashing of dependency sidecars; retain its dev declaration. The `cli` feature enables
   the product dependency, not the embedded `docker` feature. This changes the feature/dependency
   graph without adding a version or download pin. The native Cargo Renovate manager owns both
   declarations and lock integrity; verify extraction, grouping, license, actual locked feature
   graph, package and no-default boundaries. No duplicate manager or image pin is introduced.
9. Reconcile with the neutral contracts in ADRs 0080–0082 without inventing Docker support.
   Docker output reports one value-free unsupported outcome per inert vendor-metadata item,
   retaining its origins without converting metadata to runtime labels. Deferred expressions
   and service execution provider plans remain unsupported; an incomplete service never leaves
   a complete-looking Docker artifact, even under partial policy or literal environment consent.
   Every acquired Docker mount's container target and any bind source are explicitly literal
   paths. Dollar and percent bytes cannot acquire target execution meaning from provenance.
   Existing authored mounts retain their default target-context interpretation.

## Dependency review obligations

Reconciliation with current main retains the 0.10.0 workspace train and its locked
`serde_json` 1.0.154 resolution. The Docker checkpoint adds `docker-lens` 0.1.0 and the
reviewed exact `sha2` 0.11.0 requirement plus its locked transitive closure to that main
graph; reuse across the Docker checkpoints does not make the main dependency graph
unchanged. No existing locked external version is downgraded. Registry/package evidence
remains separate from the private exact-source rehearsal described below.

The reused hashing requirement has license `MIT OR Apache-2.0`. Its existing dev declaration
remains; `cli` enables the optional normal dependency and embedded `docker` alone does not.
The native Cargo Renovate manager retains both declarations, existing grouping/approvals and
lock integrity. Actual extraction, locked feature graph, license, no-default and package checks
are required; source inspection is not unchanged-graph evidence. No version, image, Action or
downloaded-tool pin is added. Historical fixture profile/image pins remain evidence, not new
operational images.

The application dispatcher's canonical `ADMISSION_REVISION` pins DockerLens #42's reviewed merge
commit: unreleased infrastructure, not a package release. No helper tag or downloadable checksum
exists; invent neither. One `github-digest` manager owns this pin; a later rule requires Dashboard
approval and disables automerge. Review producer/consumer together: both actors' permissions,
exact-head admission, privacy, and fail-closed cases.

The [consumer runtime](0079-native-test-runtime-selection.md#runner-prerequisites-and-renovate)
reuses this pin, manager and approval rule. Its
`.github/workflows/reviewed-docker-consumer-validation.yml` adds no image pins. Exact-pair execution
authorization needs renewed source/closure review, never automatic dependency updates or generated
receipts. Other Lens products and the website are nonconsumers.

## Evidence and consequences

Offline authored documents, independent native-artifact expectations, synthetic read-only socket
journeys, privacy controls, validate/convert parity, four-profile boundaries and refusal/no-output
cases protect this contract. Registered documentation examples must execute offline; route help
may document live-input options without contacting a host daemon. Existing nine-route subset
regressions remain valuable alongside the new full 16-route checks. Test source presence is not
an execution result. Focused scripts are not the complete gate.

Historical fixture profile/image pins remain historical evidence, not new operational image
selections. Existing integrity, Renovate approvals and canonical owners are unchanged. This ADR
neither changes the runtime-policy rollout nor adopts deferred application/capture modules.

Native Lens four-lane evidence is separate from actual BoxFerry consumers, full application
migrations and fresh exact-pair admission. The published DockerLens 0.1.0 dependency graph is not
compatible with this development integration. A separately authorized exact-source prerelease
override does not qualify ordinary registry resolution and must not enter published manifests or
main. Compatible publication, real API-baseline comparison, packaging and fresh complete gates
remain required. The existing published Docker-adapter baseline has no first-release exemption.
Linux support remains unchanged; macOS code does not establish platform qualification. This
change grants no release, publication, deployment, host modification or generated-request execution
authority and does not declare issue 343 complete.

## Alternatives

Native passthrough would bypass neutral intent and fidelity. Implicit target selection would
confuse source facts with destination requirements. A single broad consent flag would mix source
reconstruction, privacy, lifecycle and artifact publication. Applying requests or treating native
Lens results as application acceptance would cross product and evidence boundaries. These options
are rejected.
