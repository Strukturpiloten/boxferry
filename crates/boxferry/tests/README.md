# BoxFerry application integration tests

Cargo-discovered integration-test entry points for application-level and cross-repository policy checks live here. Shared test-only helpers live in `support/` and must not become part of the application or a public library API.

The root [`tests/`](../../../tests/README.md) directory documents cross-crate scenario ownership, and the root [`fixtures/`](../../../fixtures/README.md) directory contains their inputs.

`public_api.rs` protects the core and additive facade surfaces. `compose_to_quadlet.rs` owns the
first public end-to-end golden conversion when both native adapter features are enabled.
`runtime_to_quadlet.rs` and `runtime_to_compose.rs` prove that caller-built observations use the
same public import, planning, policy, provenance, and secret-redaction path as native files.
