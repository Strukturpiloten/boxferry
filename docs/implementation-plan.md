# Cross-repository implementation plan

This plan gives BoxFerry, ComposeLens, and QuadletLens one stable task numbering scheme. Repository roadmaps describe internal phases; this document describes delivery order across repositories.

Last synchronized: 2026-08-18.

## Status convention

- `planned` — scoped but not started
- `in progress` — implementation is currently active
- `completed` — exit criteria are met and validation is documented
- `blocked` — progress requires a named external decision or capability

The repository that owns a task is authoritative for its detailed status. Update the summary copies in the other two repositories whenever a task changes state.

## Program status

| Task | Owner                                     | Status      | Deliverable                                              |
| ---- | ----------------------------------------- | ----------- | -------------------------------------------------------- |
| T1   | All repositories                          | completed   | Executable testing and fixture foundations               |
| T2   | ComposeLens                               | completed   | Loss-aware YAML syntax and diagnostic kernel             |
| T3   | QuadletLens                               | completed   | Ordered Quadlet syntax and rendering kernel              |
| T4   | BoxFerry                                  | completed   | Independent neutral model and conversion engine          |
| T5   | All repositories                          | in progress | Minimum native typed subsets for the first conversion    |
| T6   | BoxFerry, integrating both Lens libraries | in progress | First Compose-to-Quadlet vertical slice                  |
| T7   | All repositories                          | in progress | Expanded conformance, runtime, and release testing tiers |
| T8   | BoxFerry and future native Lens projects  | blocked     | First N-to-N Docker/Compose/Podman/Quadlet milestone     |
| T9   | BoxFerry                                  | completed   | Initial Compose/Quadlet product completion               |
| T10  | BoxFerry                                  | planned     | Complete project documentation replacement               |

## T1: Testing foundations

Status: completed.

The repositories have Cargo-discovered policy tests, versioned fixture manifests, provenance and secret-review rules, immutable GitHub Action checks, stable/MSRV CI execution, and documented suite ownership. Product suites are created only with meaningful behavior.

## T2: ComposeLens YAML syntax kernel

Status: completed. ComposeLens owns this task.

ComposeLens evaluated loss-aware YAML representations, accepted ADR 0002, implemented source and diagnostic primitives, and proved exact preservation and malformed-input recovery on stable Rust and Rust 1.85.0. Its repository copy contains the detailed evidence.

## T3: QuadletLens ordered syntax kernel

Status: completed. QuadletLens owns this task.

QuadletLens implements ordered and repeated entries, comments, continuations, unknown keys,
generic systemd sections, systemd specifiers, structured diagnostics, preservation and canonical
rendering, and Podman 5.4.0 as its support floor. Its digest-pinned generator harness verifies the
first-conversion subset on all 20 patch releases through current 6.0.2, using official images
through 5.8.2 and exact source builds thereafter. Untested capabilities remain explicit
fail-closed evidence gaps. Its repository copy is authoritative for detailed evidence.

## T4: BoxFerry independent conversion core

Status: completed. BoxFerry owns this task.

Work:

1. Implement neutral application, service, volume, network, port, environment, and image-reference models.
2. Accept tolerant image references such as `name:tag@sha256:...` without unrelated OCI normalization.
3. Attach provenance to source-derived values and decisions.
4. Implement structured, secret-redacted diagnostics.
5. Represent exact, approximate, unsupported, and invalid conversion outcomes.
6. Represent target profiles with explicit minimum and optional maximum versions.
7. Define adapter contracts and an in-memory test adapter.

The accepted [public library-facade decision](decisions/0002-public-library-facade.md) makes the
facade and component crates first-class T4 consumers. The CLI must exercise the same public engine
path that external Rust projects use.

Exit criteria:

- The neutral model has no native Compose, Quadlet, Docker, Podman, or Kubernetes types.
- Model invariants, outcome aggregation, version boundaries, provenance, and redaction have unit tests.
- An in-memory adapter proves both import and export contracts without external commands.
- The `boxferry` facade exposes the supported model and engine contracts, and the CLI contains no
  private conversion behavior.
- No BoxFerry crate depends on unfinished Lens APIs.
- Public core types compile on Rust 1.85.0.

Completion evidence: the component crates and facade expose an ordered neutral application graph,
tolerant image references, source and decision provenance, redacted protected values and diagnostic
fields, inclusive target version ranges, validated fidelity outcomes, explicit loss authorization,
import/export traits, and an in-memory adapter. Unit and external-style facade tests cover model
invariants, `name:tag@digest`, provenance ordering, redaction, version boundaries, missing
diagnostics, strict/partial authorization, invalid output, and orchestration. Stable and Rust 1.85.0
workspace gates pass without Lens dependencies.

## T5: Minimum native typed subsets

Status: in progress. Each repository owns its native types; BoxFerry owns mappings.

- ComposeLens (completed): services, images, commands, health checks, restart policies,
  environment, environment-file declarations, extra hosts, ports, volumes, networks, profiles,
  configs, and secrets.
- QuadletLens (completed): `.container`, `.pod`, `.volume`, `.network`, required generic systemd
  sections, repeatable container/pod host mappings, regular health-check keys, and exact
  document-set relationships.
- BoxFerry (in progress): Compose-to-neutral and first neutral-to-Quadlet mappings are implemented;
  explicit compatibility-checked pod grouping, caller-owned mappings for host-specific bind
  sources, end-to-end host mappings, regular health checks, and dependency/readiness semantics are
  implemented. Quadlet import also retains reviewed external mount-secret grants and exact owned
  pod membership plus ordered absolute environment-file declarations; pod-scoped settings,
  context-dependent paths, and broader value encoders remain.

ComposeLens has completed its T5 native subset with source-aware typed resources, tolerant image
references, deferred values, and representation-preserving command, environment, port, volume,
network, profile, config, secret, and label forms. QuadletLens has completed its first native subset
with ordered source-aware `.container`, `.pod`, `.network`, and `.volume` documents, generic systemd
and unknown entry preservation, native key enums, conservative path/reference forms,
separate syntax/model diagnostics, and exact document-set dependency resolution. BoxFerry now
consumes ComposeLens 0.2.0 from crates.io through its independent `boxferry-compose` crate. The
adapter maps images, commands, execution identity/context, health checks, environment, extra
hosts, single ports, named volumes, bind mounts, networks, explicit profiles, config/secret
resources and grants, service labels, container restart policies, provenance, and short/long
SELinux relabel intent into the neutral model.
The reverse Quadlet adapter imports the documented conservative native subset, including external
Podman mount secrets and their target/UID/GID/mode grant options, without reading secret material.
It also reconstructs owned service groups from matching `.pod` identities and sibling `Pod=`
references without depending on document order.
Source omissions are structured outcomes governed by `LossPolicy`, not warning-only side effects.
The neutral model retains ordered explicit hostname mappings, distinguishes `host-gateway` from
ordinary IP address spellings, and keeps unsupported `start_interval` intent for target-specific
reporting.

The implemented input boundary is ComposeLens 0.2.0's native `build_project_view` over a
`MergedProject` and optional matching `ProfileSelection`. BoxFerry maps it without canonical
rendering or reparsing and retains all contributing source origins in neutral values and outcomes.

Compose `include` remains deliberately unintegrated. A safe cross-format mapping requires a
ComposeLens processed per-occurrence traversal that retains composed provenance and sensitivity,
reconciles profiles, and supplies child-relative path context. The released 0.2.0 view does not
provide that contract, so BoxFerry reports the field rather than guessing composition semantics.

ComposeLens 0.2.0 is published on crates.io with a documented pre-1.0 compatibility contract.
BoxFerry consumes ComposeLens 0.2.0 through a compatible crates.io requirement and commits its
application lockfile. Commit-pinned Git dependencies remain an emergency-only fallback.

The Compose adapter fixture also exposed a ComposeLens 0.1 YAML-backend defect: an unquoted short
volume scalar with comma-separated options can truncate the document without a syntax diagnostic.
The ComposeLens 0.1.1 parser correction accepts the complete valid scalar through its
byte-preserving private parser adapter and retains `compose.yaml.unparsed-input` as a fail-safe for
future backend omissions; that behavior remains present in the consumed 0.2.0 release.
BoxFerry can now use the unquoted real-world spelling and the released source-aware merged-project
view without a canonical render-and-reparse bridge.

QuadletLens 0.2.0 is published on crates.io and BoxFerry consumes it through the independent
`boxferry-quadlet` crate. The exporter uses typed native construction and capability evaluation,
generates exact separate container units or an explicitly authorized compatible pod plus
application-owned network and volume units, references external resources directly, validates the
native document graph, and reports every deferred value form through the shared loss policy.
Relative bind paths resolve lexically only with a caller-provided absolute Compose project root;
tilde, Windows, and other host-specific paths require an exact caller-provided target mapping.
No systemd-version selector is exposed until a reviewed emitted capability depends on it, and
BoxFerry never probes the host. The released semantic `Environment=` view maps protected literal
assignments while resets, bare names, deferred specifiers, and unmodeled forms remain explicit;
every entry in `.kube` and experimental `.artifact` documents has a value-free individual
native-only outcome.
Broader native value encoders remain T5 work. Explicit host mappings are complete across the
source-aware merged Compose `extra_hosts` view, neutral model, and repeatable capability-evidenced
Quadlet `AddHost` entries. Separate containers retain service scope. Single-pod grouping requires
identical ordered mappings and emits them at pod scope; conflicting mappings invalidate that
explicit grouping request.

### Coverage guardrail and completed health slice

The three repositories now document syntax preservation, native typing, effective project views,
neutral representation, target capabilities, and end-to-end conversion as separate coverage
stages. The authoritative cross-format matrix lives in the
[BoxFerry repository](https://github.com/Strukturpiloten/boxferry), with native details in the
ComposeLens and QuadletLens coverage documents. A field is not complete
merely because one Lens recognizes it.

ComposeLens 0.1.3 and QuadletLens 0.1.3 are published and consumed from crates.io. BoxFerry's
neutral model and adapters preserve health command form, explicit disable intent, regular timing,
retries, startup grace period, field-level provenance, and strict/partial policy behavior. The
Quadlet output is capability-checked against real-generator evidence across all 20 recorded Podman
patches from 5.4.0 through 6.0.2. Compose `start_interval` is retained but explicitly unsupported;
it is not treated as equivalent to Podman's separate startup-healthcheck mechanism.

### Completed dependency ordering and readiness slice

ComposeLens 0.1.4 and QuadletLens 0.1.4 are published and consumed from crates.io. Ordered neutral
dependency edges retain condition, requirement, restart, and merge provenance. Required and
optional startup edges become separately capability-checked `Requires`/`Wants` plus `After`.
Healthy edges additionally select `Notify=healthy` only for explicit encodable target health
commands. Restart propagation, successful completion, provider conditions, and absent optional
services are policy-controlled partial losses; missing required services and cycles are invalid.
Adapter and public-facade golden tests cover separate containers and explicitly grouped pods.

### Completed execution identity and context slice

ComposeLens 0.1.9 and QuadletLens 0.1.7 are published and consumed from crates.io. BoxFerry retains and
maps primary user/group, user namespace, ordered supplementary groups, working directory, and
explicit read-only-root intent with field provenance and sensitivity. Separate containers use
capability-checked `User`, `Group`, `UserNS`, repeated `GroupAdd`, `WorkingDir`, and `ReadOnly`
entries across the verified Podman range. Unresolved booleans and unsafe encodings remain explicit
losses. For pod grouping, BoxFerry emits pod-level `UserNS` only when every service has the same
explicit namespace intent. Mixed absent/explicit or conflicting values invalidate the grouping
request.

### Completed config and secret slice

ComposeLens 0.1.9 and QuadletLens 0.1.7 are published and consumed without sibling path dependencies.
ComposeLens exposes effective service config/secret grants with short/long syntax and nested
multi-file provenance. QuadletLens exposes repeatable container `Secret` plus pod `UserNS`, with
real-generator evidence across every recorded Podman patch from 5.4.0 through 6.0.2. BoxFerry
imports resource ownership, runtime names, material origins, and ordered grants without Lens types.
It emits exact references to pre-existing external Podman secrets when target defaults and options
are safe and equivalent. Application-owned secret materialization and Compose config lifecycle are
explicit manual actions. Adapter and public-facade tests cover short/long, multi-file, custom-name,
sensitive-value, strict/partial, and golden behavior.

### Completed explicit container-name slice

ComposeLens 0.1.10 and QuadletLens 0.1.9 are published and consumed from crates.io. BoxFerry keeps
an optional provenance-bearing declared service runtime name separate from its neutral service
identifier. Effective multi-file Compose `container_name` values map into that field and emit as
capability-checked Quadlet `ContainerName=`. Both output adapters validate their target grammar
and produce an invalid outcome rather than omitting an unsafe name. Model, adapter, invalid-value,
and public-facade golden tests cover the complete boundary.

## T6: First end-to-end milestone

Status: in progress. BoxFerry coordinates this task. Compose import, the first Quadlet export
subset, their combined policy-controlled report, and explicit compatibility-checked pod grouping
are implemented. Explicit project-root and host-specific bind path policies are also implemented.
Explicit Compose-to-Quadlet host mappings, regular health checks, and dependency/readiness mapping
are implemented. Pod-level user namespaces and external-secret grants are also implemented with
explicit compatibility and manual-action reporting. A first black-box-tested CLI exposes this same public conversion path with
explicit file order, caller-controlled per-file Compose interpolation, profile selection, target
range, grouping, loss policy, and non-overwriting output. Interpolation starts from an empty map;
plain literals and individually authorized sensitive process variables are the only inputs. No
implicit `.env` or ambient environment lookup occurs. ComposeLens 0.2.0's native service
`env_file` declaration boundary is consumed from crates.io. BoxFerry retains declarations without
file I/O and maps required safe paths to Quadlet `EnvironmentFile=` as an explicit parser-parity
approximation. Optional files, unsafe paths, authorized file-content processing, broader value
encoders, and the TYPO3 showcase remain.

The exporter consumes ComposeLens 0.2.0's parse-back-validated generated-document API. Embedded
callers may require one exact Docker Compose or `podman-compose` provider release and an optional
separate exact Docker Engine or Podman backend; the generic Quadlet-to-Compose CLI route instead
uses the rolling provider-neutral Compose Specification target. It emits ordered short/long
environment-file declarations with their explicit options and sensitivity. It reports every
compatibility or unsupported field decision before
authorization. Health checks, dependencies, config/secret service grants, non-file material, and
structural groups remain explicit partial losses. Application-owned file-backed top-level configs
and secrets now use ComposeLens's parse-back-validated generated subset.

Deliver tested Compose-to-Quadlet conversion for images, commands, execution identity/context,
health checks, dependencies, environment, extra hosts, ports, named volumes, bind mounts,
networks, and explicit Compose profile selection. Every
conversion emits compatibility and manual-action reports. After synthetic scenarios are stable,
use `Strukturpiloten/typo3-container` as the first public real-world showcase and regression
corpus.

Exit criteria:

- Supported features produce complete Quadlet file sets for Podman 5.4 and a selected current target.
- Every non-exact mapping produces a structured compatibility outcome.
- Profile selection is explicit; BoxFerry never guesses active Compose profiles.
- Golden scenarios cover exact, approximate, unsupported, and invalid results.
- The TYPO3 showcase has immutable provenance, licensing review, and documented manual actions.

## T7: Expanded testing tiers

Status: in progress. ComposeLens has delivered its repository tier. QuadletLens has an exact
Podman 5.4-to-current generator matrix. BoxFerry's remaining work is limited to the document
product until future native Lens projects supply their own reviewed runtime contracts.

- Per pull request: unit, integration, golden, round-trip, and property tests.
- Scheduled: Docker Compose, Podman Compose, and real Quadlet generator conformance.
- Release validation: supported Podman matrices, rootless/rootful contexts, real-world projects, and eventually disposable Kubernetes clusters.

Each harness becomes required only after its command, isolation model, version source, fixture provenance, and failure policy are documented.

The earlier BoxFerry-native runtime implementation has been removed. Docker, Podman, and
Kubernetes native boundaries are future independent Lens projects; BoxFerry must not recreate
their protocol, acquisition, conformance, deployment, or execution responsibilities while T9 and
T10 are in progress.

## T8: First N-to-N runtime and definition milestone

Status: blocked by the deferred DockerLens and PodmanLens projects. BoxFerry coordinates the future
integration task but does not own their native contracts. The Quadlet-to-neutral importer covers
the exact shared image, container-name, command, environment, scalar-port, named/absolute-mount,
named-network, metadata-label, host-mapping, and execution-context slice. It retains native section
identity for systemd restart policy and complete sibling activation-plus-ordering dependencies;
regular native health commands and scalars also enter the protected, source-aware health model.
Host-unit references and incomplete relationships remain explicit. Docker and Podman native
responsibilities move behind future independent Lens libraries; no native runtime import API
remains in BoxFerry. The nested CLI exposes the four Compose/Quadlet document routes.
Compose-to-Compose uses public native canonicalization to retain expressions and extension data;
cross-format and Quadlet-to-Quadlet routes use the neutral model.

Docker runtime resources, Docker Compose, Podman runtime resources, and Podman Quadlet must each
be available as a source and a target. Routes compose through the neutral model rather than using
pair-specific conversion logic. Runtime targets first produce deterministic, reviewable deployment
plans; applying a plan remains a separate explicit side effect.

Exit criteria:

- All four boundaries have importers and exporters for one documented shared semantic subset.
- The CLI can select each input and output explicitly without owning conversion rules.
- Every one of the sixteen source/target combinations has an offline golden contract test.
- Runtime acquisition and application require explicit endpoints and resources and never enumerate
  or mutate ambient state implicitly.
- Incompatible source intent produces structured outcomes governed by the same loss policy on
  every route.

No T8 runtime target, executor, CLI route, or 16-cell completion claim is implemented until the
future DockerLens and PodmanLens projects release the required native contracts. See
[ADR 0032](decisions/0032-future-native-lens-boundaries.md).

## T9: Initial Compose/Quadlet product completion

Status: completed. Implementation, focused review, and the complete 21-step repository validation
gate passed on 2026-08-18. BoxFerry uses only released ComposeLens and QuadletLens public APIs.

Work:

1. Complete the deterministic Compose-to-neutral, Quadlet-to-neutral, neutral-to-Compose, and
   neutral-to-Quadlet document matrix using the current Lens APIs without native-library changes.
2. Record project `.env` parsing/materialization, service `env_file` content
   parsing/materialization, Compose `include` processing, and generated config/secret service
   grants as named released-Lens API gaps. BoxFerry does not privately implement these native
   behaviors; `--env-file` remains limited to explicit Compose interpolation assignments.
3. Complete positive, negative, loss-policy, same-format, JSON-report, fix-first, error-report, and
   deterministic golden coverage for all four Compose/Quadlet document routes.
4. Promote corpus-derived regressions only when the required native behavior is already available
   through released Lens APIs; record other cases as named dependency gaps.
5. Review and stabilize the public core, Compose, Quadlet, CLI, diagnostic, and report contracts
   intended for the initial BoxFerry product. Remove inaccurate capability claims and unreleased
   transitional APIs rather than preserving compatibility shims.
6. Run the complete repository validation and prepare the next BoxFerry release through the
   existing release-plz workflow.

Exit criteria:

- The deterministic four document routes expose truthful route-specific help and use public facade APIs.
- Every supported mapping has positive and negative boundary coverage and every loss is structured.
- Same-format Compose preserves native expressions/extensions; all other document routes use the
  neutral model and deterministic native rendering.
- Authorized environment content never becomes an ambient default or leaks into diagnostics,
  reports, fixtures, or debug output.
- No available capability depends on DockerLens, PodmanLens, or KubernetesLens.
- The complete local validation gate passes from a clean checkout.

The following are explicitly outside T9: Docker/Podman runtime targets or executors, runtime CLI
routes, Kubernetes resources, `.kube` semantic conversion, artifact execution, native resource
label/security behavior without Lens evidence, and Compose/Quadlet features that require an
unreleased Lens API change.

## T10: Complete project documentation replacement

Status: planned and high priority immediately after T9.

Replace the current development-era documentation with a coherent user, operator, library,
architecture, compatibility, troubleshooting, and contributor documentation set that describes
only implemented behavior. Detailed information architecture and migration work are intentionally
deferred until T9 fixes the product contract that the new documentation must explain.
