# Cross-repository implementation plan

This plan gives BoxFerry, ComposeLens, and QuadletLens one stable task numbering scheme. Repository roadmaps describe internal phases; this document describes delivery order across repositories.

Last synchronized: 2026-07-31.

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
| T3 | QuadletLens | planned | Ordered Quadlet syntax and rendering kernel |
| T4 | BoxFerry | planned | Independent neutral model and conversion engine |
| T5 | All repositories | planned | Minimum native typed subsets for the first conversion |
| T6 | BoxFerry, integrating both Lens libraries | planned | First Compose-to-Quadlet vertical slice |
| T7 | All repositories | planned | Expanded conformance, runtime, and release testing tiers |

## T1: Testing foundations

Status: completed.

The repositories have Cargo-discovered policy tests, versioned fixture manifests, provenance and secret-review rules, immutable GitHub Action checks, stable/MSRV CI execution, and documented suite ownership. Product suites are created only with meaningful behavior.

## T2: ComposeLens YAML syntax kernel

Status: completed. ComposeLens owns this task.

ComposeLens evaluated loss-aware YAML representations, accepted ADR 0002, implemented source and diagnostic primitives, and proved exact preservation and malformed-input recovery on stable Rust and Rust 1.85.0. Its repository copy contains the detailed evidence.

## T3: QuadletLens ordered syntax kernel

Status: planned. QuadletLens owns this task.

QuadletLens implements ordered and repeated entries, comments, continuations, unknown keys, generic systemd sections, systemd specifiers, structured diagnostics, rendering, and the Podman 5.4 capability baseline. Its repository copy is authoritative for detailed exit criteria.

## T4: BoxFerry independent conversion core

Status: planned. BoxFerry owns this task.

Work:

1. Implement neutral application, service, volume, network, port, environment, and image-reference models.
2. Accept tolerant image references such as `name:tag@sha256:...` without unrelated OCI normalization.
3. Attach provenance to source-derived values and decisions.
4. Implement structured, secret-redacted diagnostics.
5. Represent exact, approximate, unsupported, and invalid conversion outcomes.
6. Represent target profiles with explicit minimum and optional maximum versions.
7. Define adapter contracts and an in-memory test adapter.

Exit criteria:

- The neutral model has no native Compose, Quadlet, Docker, Podman, or Kubernetes types.
- Model invariants, outcome aggregation, version boundaries, provenance, and redaction have unit tests.
- An in-memory adapter proves both import and export contracts without external commands.
- No BoxFerry crate depends on unfinished Lens APIs.
- Public core types compile on Rust 1.85.0.

## T5: Minimum native typed subsets

Status: planned. Each repository owns its native types; BoxFerry owns mappings.

- ComposeLens: services, images, commands, environment, ports, volumes, networks, and profiles.
- QuadletLens: `.container`, `.volume`, `.network`, and required generic systemd sections.
- BoxFerry: mappings, path-policy differences, and Podman 5.4 fallback decisions.

Before integration, document dependency and release mechanics. Prefer early pre-1.0 Lens releases; use commit-pinned Git dependencies only as a temporary fallback.

## T6: First end-to-end milestone

Status: planned. BoxFerry coordinates this task.

Deliver tested Compose-to-Quadlet conversion for images, commands, environment, ports, named volumes, bind mounts, networks, and explicit Compose profile selection. Every conversion emits compatibility and manual-action reports. After synthetic scenarios are stable, use `Strukturpiloten/typo3-container` as the first public real-world showcase and regression corpus.

Exit criteria:

- Supported features produce complete Quadlet file sets for Podman 5.4 and a selected current target.
- Every non-exact mapping produces a structured compatibility outcome.
- Profile selection is explicit; BoxFerry never guesses active Compose profiles.
- Golden scenarios cover exact, approximate, unsupported, and invalid results.
- The TYPO3 showcase has immutable provenance, licensing review, and documented manual actions.

## T7: Expanded testing tiers

Status: planned.

- Per pull request: unit, integration, golden, round-trip, and property tests.
- Scheduled: Docker Compose, Podman Compose, and real Quadlet generator conformance.
- Release validation: supported Podman matrices, rootless/rootful contexts, real-world projects, and eventually disposable Kubernetes clusters.

Each harness becomes required only after its command, isolation model, version source, fixture provenance, and failure policy are documented.
