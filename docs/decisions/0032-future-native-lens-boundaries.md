# ADR 0032: Future native Docker, Podman, and Kubernetes Lens boundaries

- Status: accepted
- Date: 2026-08-18
- Supersedes: [ADR 0003](0003-explicit-runtime-observation-provenance.md),
  [ADR 0005](0005-shared-runtime-observation-layer.md),
  [ADR 0006](0006-finite-podman-inspect-decoder.md),
  [ADR 0007](0007-finite-podman-relationship-expansion.md),
  [ADR 0008](0008-isolated-podman-runtime-conformance.md),
  [ADR 0009](0009-versioned-docker-inspection.md),
  [ADR 0010](0010-isolated-docker-runtime-conformance.md),
  [ADR 0012](0012-explicit-runtime-lifecycle-resolution.md),
  [ADR 0014](0014-runtime-regular-health-observations.md),
  [ADR 0015](0015-runtime-container-restart-policy.md),
  [ADR 0016](0016-runtime-metadata-label-reconstruction.md), and
  [ADR 0017](0017-n-to-n-adapter-matrix.md)
- Amends: [ADR 0001](0001-project-boundaries-and-origin.md) and
  [ADR 0026](0026-typed-diagnostic-rule-catalogue.md)

## Context

BoxFerry's product direction remains an N-to-N converter. Docker runtime resources, Compose,
Podman runtime resources, Quadlet, and Kubernetes are overlapping native views of application
intent. The original first-major-milestone decision assigned native Docker and Podman inspection,
planning, and execution directly to BoxFerry adapter crates.

Implementation experience with ComposeLens and QuadletLens shows that a native boundary contains
substantial independent work: tolerant parsing or protocol decoding, native types, version and
capability evidence, deterministic generation, diagnostics, explicit acquisition, execution
safety, conformance fixtures, and release maintenance. Embedding those concerns in BoxFerry would
make its runtime adapters qualitatively different from its document adapters and would couple the
orchestrator to three independently evolving native platforms.

Docker, Podman, and Kubernetes also differ too much to share one generic native runtime library.
Their version models, APIs, lifecycle semantics, resource graphs, and safety boundaries require
separate evidence and public contracts.

The DockerLens, PodmanLens, and KubernetesLens projects do not exist yet. They are future,
separately planned projects and are not part of the current BoxFerry implementation milestone.

## Decision

1. `docker-lens`, `podman-lens`, and `kubernetes-lens` will be independent future libraries and
   repositories. None of them will depend on BoxFerry.
2. A native Lens owns its platform's syntax or protocol decoding, native source and target types,
   version/capability evidence, native diagnostics, deterministic native deployment-plan contract,
   explicit acquisition, and explicitly authorized application/execution boundary.
3. BoxFerry owns the format-neutral application model, conversion engine, fidelity policy,
   orchestration, CLI, reports, and thin semantic mappings between a Lens-native model and the
   neutral model. Future mapping adapters may use the `boxferry-docker`, `boxferry-podman`, and
   `boxferry-kubernetes` names after the corresponding Lens contracts exist.
4. The previous `boxferry-runtime`, `boxferry-docker`, and `boxferry-podman` crates are removed
   now. They were published experimental pre-1.0 surfaces and have no compatibility or migration
   shim. Their removal is an intentional breaking change for the next pre-1.0 minor release and is
   recorded through the release-plz changelog workflow. Native response types, endpoints,
   commands, platform versions, acquisition, conformance harnesses, deployment plans, and
   executors do not remain in this repository.
5. The current BoxFerry document product must not reintroduce a private runtime abstraction while
   the future Lens projects are deferred.
6. The current diagnostic catalogue assigns only Compose (`BFC`), Quadlet (`BFQ`), and
   orchestration/file/report (`BFO`) prefixes. The removed `BFD`, `BFP`, and `BFR` assignments do
   not remain part of the public catalogue. Future Lens integrations choose reviewed namespaces
   through a new decision after their native diagnostic contracts exist.
7. The current BoxFerry milestone is the complete Compose/Quadlet definition boundary and the
   shared conversion core. The four document routes remain `compose -> compose`,
   `compose -> quadlet`, `quadlet -> compose`, and `quadlet -> quadlet`.
8. The broader N-to-N runtime milestone is blocked until DockerLens and PodmanLens have released
   reviewed importer, deployment-plan, and execution contracts. Kubernetes joins later through
   KubernetesLens; it does not create a second conversion engine.
9. No placeholder crate, compatibility shim, speculative native DTO, or generic runtime Lens is
   added while those future projects are deferred.
10. After the initial BoxFerry implementation scope is complete, replacing the current project
    documentation is the next high-priority milestone. That rewrite is planned separately and is
    not designed by this decision.

The future dependency direction is:

```text
docker-lens ──> future BoxFerry Docker mapping ──┐
compose-lens ─> boxferry-compose ────────────────┤
podman-lens ──> future BoxFerry Podman mapping ───┼─> BoxFerry engine and CLI
quadlet-lens ─> boxferry-quadlet ────────────────┤
kubernetes-lens -> future BoxFerry Kubernetes mapping ─┘
```

The arrows indicate dependency direction. Lens libraries never depend on BoxFerry.

## Consequences

- BoxFerry can finish and stabilize its current definition-conversion scope without inventing
  incomplete runtime target contracts.
- Native Docker, Podman, and Kubernetes behavior can be tested, versioned, and released
  independently by projects whose APIs are useful outside BoxFerry.
- BoxFerry retains one N-to-N neutral conversion architecture, but runtime and Kubernetes cells are
  not advertised as available before their native libraries exist.
- Completing the full runtime matrix takes longer because three native library projects must first
  establish their own contracts.
- The T8 milestone remains visible but blocked rather than being reported as partially complete or
  silently narrowed.

## Alternatives considered

### Keep all native runtime behavior in BoxFerry

Rejected because acquisition, versioned native decoding, plan generation, execution, and
conformance are independently complex and would make BoxFerry own platform libraries in addition
to conversion orchestration.

### Create one generic runtime Lens

Rejected because Docker, Podman, and Kubernetes do not share one native protocol, resource model,
version policy, or execution contract. Common conversion semantics belong in BoxFerry's neutral
model, not in a lowest-common-denominator native library.

### Create the three repositories immediately

Rejected for the current milestone. Their boundaries are recorded now, but implementation and
repository setup will be planned separately when those projects are started.
