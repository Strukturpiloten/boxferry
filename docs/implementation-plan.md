# Cross-repository implementation plan

This plan gives BoxFerry, ComposeLens, and QuadletLens one stable task numbering scheme. Repository roadmaps describe internal phases; this document describes delivery order across repositories.

Last synchronized: 2026-08-03.

## Status convention

- `planned` — scoped but not started
- `in progress` — implementation is currently active
- `completed` — exit criteria are met and validation is documented
- `blocked` — progress requires a named external decision or capability

The repository that owns a task is authoritative for its detailed status. Update the summary copies in the other two repositories whenever a task changes state.

## Program status

| Task | Owner | Status | Deliverable |
| --- | --- | --- | --- |
| T1 | All repositories | completed | Executable testing and fixture foundations |
| T2 | ComposeLens | completed | Loss-aware YAML syntax and diagnostic kernel |
| T3 | QuadletLens | completed | Ordered Quadlet syntax and rendering kernel |
| T4 | BoxFerry | completed | Independent neutral model and conversion engine |
| T5 | All repositories | in progress | Minimum native typed subsets for the first conversion |
| T6 | BoxFerry, integrating both Lens libraries | in progress | First Compose-to-Quadlet vertical slice |
| T7 | All repositories | in progress | Expanded conformance, runtime, and release testing tiers |

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

- ComposeLens (completed): services, images, commands, health checks, environment, extra hosts,
  ports, volumes, networks, profiles, configs, and secrets.
- QuadletLens (completed): `.container`, `.pod`, `.volume`, `.network`, required generic systemd
  sections, repeatable container/pod host mappings, regular health-check keys, and exact
  document-set relationships.
- BoxFerry (in progress): Compose-to-neutral and first neutral-to-Quadlet mappings are implemented;
  explicit compatibility-checked pod grouping, caller-owned mappings for host-specific bind
  sources, end-to-end host mappings, regular health checks, and dependency/readiness semantics are
  implemented; broader value encoders remain.

ComposeLens has completed its T5 native subset with source-aware typed resources, tolerant image
references, deferred values, and representation-preserving command, environment, port, volume,
network, profile, config, secret, and label forms. QuadletLens has completed its first native subset
with ordered source-aware `.container`, `.pod`, `.network`, and `.volume` documents, generic systemd
and unknown entry preservation, native key enums, conservative path/reference forms,
separate syntax/model diagnostics, and exact document-set dependency resolution. BoxFerry now
consumes ComposeLens 0.1.5 from crates.io through its independent `boxferry-compose` crate. The
adapter maps images, commands, execution identity/context, health checks, environment, extra hosts, single ports, named
volumes, bind mounts, networks, explicit profiles, provenance, and short/long SELinux relabel
intent into the neutral model. Source omissions are structured outcomes governed by `LossPolicy`,
not warning-only side effects. The neutral model retains ordered explicit hostname mappings,
distinguishes `host-gateway` from ordinary IP address spellings, and keeps unsupported
`start_interval` intent for target-specific reporting.

The implemented input boundary is ComposeLens 0.1.5's native `build_project_view` over a
`MergedProject` and optional matching `ProfileSelection`. BoxFerry maps it without canonical
rendering or reparsing and retains all contributing source origins in neutral values and outcomes.

ComposeLens 0.1.5 is published on crates.io with a documented pre-1.0 compatibility contract.
BoxFerry consumes ComposeLens 0.1.5 through a compatible crates.io requirement and commits its
application lockfile. Commit-pinned Git dependencies remain an emergency-only fallback.

The Compose adapter fixture also exposed a ComposeLens 0.1 YAML-backend defect: an unquoted short
volume scalar with comma-separated options can truncate the document without a syntax diagnostic.
The ComposeLens 0.1.1 parser correction accepts the complete valid scalar through its
byte-preserving private parser adapter and retains `compose.yaml.unparsed-input` as a fail-safe for
future backend omissions; that behavior remains present in the consumed 0.1.5 release.
BoxFerry can now use the unquoted real-world spelling and the released source-aware merged-project
view without a canonical render-and-reparse bridge.

QuadletLens 0.1.5 is published on crates.io and BoxFerry consumes it through the independent
`boxferry-quadlet` crate. The exporter uses typed native construction and capability evaluation,
generates exact separate container units or an explicitly authorized compatible pod plus
application-owned network and volume units, references external resources directly, validates the
native document graph, and reports every deferred value form through the shared loss policy.
Relative bind paths resolve lexically only with a caller-provided absolute Compose project root;
tilde, Windows, and other host-specific paths require an exact caller-provided target mapping.
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

ComposeLens and QuadletLens 0.1.5 are published and consumed from crates.io. BoxFerry retains and
maps primary user/group, user namespace, ordered supplementary groups, working directory, and
explicit read-only-root intent with field provenance and sensitivity. Separate containers use
capability-checked `User`, `Group`, `UserNS`, repeated `GroupAdd`, `WorkingDir`, and `ReadOnly`
entries across the verified Podman range. Unresolved booleans and unsafe encodings remain explicit
losses. Pod-grouped container `UserNS` is intentionally omitted only with partial authorization
because Podman uses the pod namespace. QuadletLens 0.1.6 is the tested release candidate for the
pod-level key. After publication, BoxFerry will emit it only when every grouped service has the
same explicit namespace intent; mixed absent/explicit or conflicting values will invalidate the
grouping request.

### Config and secret release gate

ComposeLens and QuadletLens 0.1.6 are coordinated release candidates for the next integration
gate. ComposeLens exposes effective service config/secret grants with short/long syntax and nested
multi-file provenance. QuadletLens exposes repeatable container `Secret` plus pod `UserNS`, with
real-generator evidence across every recorded Podman patch from 5.4.0 through 6.0.2. BoxFerry will
not use sibling path dependencies: publication precedes adapter integration. The independent
BoxFerry neutral slice is complete: resource ownership, runtime names, material origins, ordered
short/long grants, per-option provenance, and redaction are represented without Lens types.

## T6: First end-to-end milestone

Status: in progress. BoxFerry coordinates this task. Compose import, the first Quadlet export
subset, their combined policy-controlled report, and explicit compatibility-checked pod grouping
are implemented. Explicit project-root and host-specific bind path policies are also implemented.
Explicit Compose-to-Quadlet host mappings, regular health checks, and dependency/readiness mapping
are implemented. A first black-box-tested CLI exposes this same public conversion path with
explicit file order, profile selection, target range, grouping, loss policy, and non-overwriting
output. Broader value encoders and the TYPO3 showcase remain.

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
Podman 5.4-to-current generator matrix, and BoxFerry now has its first provenance-reviewed Compose
adapter fixture; broader BoxFerry tiers remain.

- Per pull request: unit, integration, golden, round-trip, and property tests.
- Scheduled: Docker Compose, Podman Compose, and real Quadlet generator conformance.
- Release validation: supported Podman matrices, rootless/rootful contexts, real-world projects, and eventually disposable Kubernetes clusters.

Each harness becomes required only after its command, isolation model, version source, fixture provenance, and failure policy are documented.
