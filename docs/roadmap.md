# Roadmap

This roadmap describes dependency order, not delivery dates. A later phase may be explored early, but stable behavior must be built on completed lower layers.

Cross-repository delivery uses the stable task numbers in the [implementation plan](implementation-plan.md). This roadmap remains the detailed internal phase order for BoxFerry.

## Status key

- [x] Completed and validated
- [ ] Open

## Phase 0: foundation — in progress

- [x] Accept repository and licensing decisions.
- [x] Establish documentation and ADR practice.
- [x] Scaffold the Cargo workspace and crate boundaries.
- [x] Define Rust version, lint, dependency, CI, and release policies.
- [x] Define fixture provenance requirements.
- [ ] Define the product diagnostic schema.

## Phase 1: application model and engine — open

- [ ] Implement the minimal application graph for one multi-service application.
- [ ] Implement provenance and structured diagnostics.
- [ ] Implement target profiles and conversion outcome policies.
- [ ] Provide adapter contracts and an in-memory test adapter.

## Phase 2: Compose to Quadlet vertical slice — open

- [ ] Consume ComposeLens and QuadletLens as independent dependencies.
- [ ] Support images, commands, environment, ports, named volumes, bind mounts, and networks.
- [ ] Generate a complete compatibility report.
- [ ] Validate Podman 5.4 and newer capability selection.

## Phase 3: runtime migration — open

- [ ] Inspect existing Podman containers, pods, networks, volumes, and images.
- [ ] Inspect equivalent Docker resources.
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
