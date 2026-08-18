# BoxFerry application integration tests

Cargo-discovered integration-test entry points for application-level and cross-repository policy checks live here. Shared test-only helpers live in `support/` and must not become part of the application or a public library API.

The root [`tests/`](../../../tests/README.md) directory documents cross-crate scenario ownership, and the root [`fixtures/`](../../../fixtures/README.md) directory contains their inputs.

`public_api.rs` protects the core and additive facade surfaces. `compose_to_quadlet.rs` owns the
first public end-to-end golden conversion when both document-adapter features are enabled.
`document_route_matrix.rs` owns the deterministic all-four-pair public facade, CLI, and
validate-without-writing boundary using `fixtures/conversion/document-route-matrix/`. The remaining
route, CLI, report, and repository-policy tests exercise focused Compose/Quadlet behaviors and
their public orchestration boundary.
