# Roadmap

This roadmap describes dependency order, not delivery dates. A later phase may be explored early, but stable behavior must be built on completed lower layers.

Cross-repository delivery uses the stable task numbers in the [implementation plan](implementation-plan.md). This roadmap remains the detailed internal phase order for BoxFerry.

## Status key

- [x] Completed and validated
- [ ] Open

## Phase 0: foundation — completed

- [x] Accept repository and licensing decisions.
- [x] Establish documentation and ADR practice.
- [x] Scaffold the Cargo workspace and crate boundaries.
- [x] Define the public library facade, reusable component-crate, CLI-parity, and publication boundaries.
- [x] Define Rust version, lint, dependency, CI, and release policies.
- [x] Define fixture provenance requirements.
- [x] Define the product diagnostic schema.

## Phase 1: application model and engine — completed

- [x] Implement the minimal application graph for one multi-service application.
- [x] Represent resource ownership, lifecycle, service attachment, and declared-source provenance.
- [x] Implement provenance and structured diagnostics.
- [x] Implement target profiles and conversion outcome policies.
- [x] Support strict planning plus explicitly authorized partial output with a diagnostic for every loss.
- [x] Provide adapter contracts and an in-memory test adapter.

## Phase 2: Compose to Quadlet vertical slice — completed

- [x] Consume released ComposeLens 0.1 as an independent crates.io dependency.
- [x] Consume QuadletLens 0.1.1 as an independent crates.io dependency.
- [x] Import Compose images, commands, environment, single ports, named volumes, bind mounts,
  networks, explicit profiles, source provenance, and SELinux relabel intent.
- [x] Consume a loss-aware typed ComposeLens merged-project view for multi-file projects without
  reparsing canonical YAML or losing source provenance.
- [x] Export the first neutral subset through QuadletLens with explicit outcomes for deferred value forms.
- [x] Preserve declaring-service ownership by keeping the first slice in separate container units.
- [x] Select pod grouping only when service networking and port semantics remain compatible.
- [x] Distinguish application-owned, external, and implicit default networks and volumes.
- [x] Translate absolute and systemd-specifier paths and resolve relative paths only with an explicit caller-provided project root.
- [x] Generate a complete compatibility report for every first-slice target decision.
- [x] Validate Podman 5.4.0 through the finite current QuadletLens catalogue ceiling.

Current dependency gates:

- ComposeLens 0.1.1 is published and consumed through its native merged-project view. The adapter
  regression imports complete unquoted short-volume scalars such as `./data:/data:Z,ro` and retains
  all contributing multi-file source origins.
- QuadletLens 0.1.1 is published and consumed through its validated document builder and finite
  capability catalogue. The exporter generates container, optional explicitly selected pod,
  application-owned network, and application-owned volume units; retains external resources as
  direct references; validates its native dependency graph; and redacts generated contents from
  `Debug` output. Single-pod grouping requires compatible declarations and explicit approximation
  authorization; incompatible requests fail without fallback.
- Public-facade golden scenarios prove multi-file Compose processing, explicit profile selection,
  strict/partial/approximate authorization, exact separate-container and pod-grouped file bytes,
  stable diagnostics, dependency graphs, and provenance.
- Relative bind paths resolve lexically when the caller supplies their absolute Compose project
  root; otherwise they remain explicit losses. Tilde/home and non-POSIX forms, per-network aliases,
  and quoting-dependent values remain documented target-side losses.

## Phase 3: runtime migration — open

- [ ] Inspect existing Podman containers, pods, networks, volumes, and images.
- [ ] Inspect equivalent Docker resources.
- [ ] Treat runtime `CreateCommand` data as optional provenance rather than a required source of truth.
- [ ] Reconstruct overrides by comparing effective container and image inspection data.
- [ ] Preserve multiple network aliases and resource relationships from observations.
- [ ] Reconstruct application intent with explicit uncertainty.
- [ ] Generate Compose and Quadlet definitions from observations.

## Phase 4: Kubernetes — open

- [ ] Read and write core Kubernetes resources.
- [ ] Define workload-controller selection and service/storage mappings.
- [ ] Add Kubernetes version and API capability checks.
- [ ] Validate on disposable clusters.

## Phase 5: Helm and Kustomize — open

- [ ] Consume rendered Helm and Kustomize input.
- [ ] Generate maintainable Kustomize bases and overlays where possible.
- [ ] Investigate Helm chart generation as a separate policy-driven backend.

## Phase 6: ecosystem hardening — open

- [ ] Expand the real-world corpus.
- [ ] Stabilize selected library APIs.
- [ ] Add packaging and signed releases.
- [ ] Publish compatibility matrices and migration guides.
- [ ] Establish contributor governance and long-term maintenance policy.

## Issue-derived evidence

The dated [Podlet and `compose_spec_rs` issue-corpus review](research/podlet-compose-spec-rs-issues-2026-08-01.md)
maps real user reports to these phases and to the owning Lens repositories. Issue state does not
complete a task or establish compatibility; specifications and exact-version tests remain required.
