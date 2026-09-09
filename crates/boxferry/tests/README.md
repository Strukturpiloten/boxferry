# BoxFerry application integration tests

The scenario-contract suite discovers versioned migration scenarios, validates
independent neutral expectations and exact subject-level losses/diagnostics,
and executes every exporter and applicable semantic reimport. Mutation tests
must fail for their own reason; unperformed checks cannot count as passed.

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

`repository_policy.rs` also owns static and focused helper contracts for the bounded Nextcloud,
Forgejo, Paperless, and Immich live application profiles. The Immich checks keep the authored
offline replay distinct from runtime evidence, pin the sole rootless cell and every external digest,
and protect CPU-only upload/derivative/persistence, capture privacy, selector, redaction, resource,
and cleanup boundaries without executing the live profile during the deterministic test suite.
Reviewed Paperless and Immich sanitizer-v3 cassettes additionally replay complete production
acquisition while repository policy keeps their redacted evidence separate from authored semantics.

## Scenario acceptance ownership

`scenario_contract.rs` owns catalogue-only application acceptance. It validates both supported manifest spellings, all exporters for every input, independent neutral intent, exact artifact/diagnostic/loss sidecars, protected-value counterfactuals, and applicable reimports. Real-world Compose blobs are vendored with pinned Git provenance and reviewed legal files so this suite remains offline. Remote corpus fetching is a refresh check, not a test dependency.

Podman JSON and command artifacts are validated as deployment plans—including creates, external preconditions, image operations, start order, and CLI/API parity—and are never treated as observed inventory. `native-validation`, `runtime-probe`, and `reimport` remain separate dimensions; a reviewed gap in one cannot be renamed migration success by updating a golden file.
