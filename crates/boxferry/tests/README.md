# BoxFerry application integration tests

Cargo-discovered integration-test entry points for application-level and cross-repository policy checks live here. Shared test-only helpers live in `support/` and must not become part of the application or a public library API.

The root [`tests/`](../../../tests/README.md) directory documents cross-crate scenario ownership, and the root [`fixtures/`](../../../fixtures/README.md) directory contains their inputs.

`public_api.rs` protects the core and additive facade surfaces. `compose_to_quadlet.rs` owns the
first public end-to-end golden conversion when both document-adapter features are enabled.
`document_route_matrix.rs` owns deterministic all-four-pair public facade, CLI,
validate-without-writing, chained conversion, discovery, fixed-point, path-consistency, policy,
diagnostic, and redaction boundaries using `fixtures/conversion/document-route-matrix/`.

`fixture_route_corpus.rs` discovers every positive adapter-contract and conversion manifest,
requires each input scenario to define every exporter reported by live capabilities, validates
reviewed artifacts and diagnostic sequences across the policy lattice, and re-imports every
generated result to a same-format fixed point. The remaining route, CLI, report, and
repository-policy tests exercise focused Compose/Quadlet behaviors and their public orchestration
boundary.
