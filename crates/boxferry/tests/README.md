# BoxFerry application integration tests

Cargo-discovered integration-test entry points for application-level and cross-repository policy checks live here. Shared test-only helpers live in `support/` and must not become part of the application or a public library API.

The root [`tests/`](../../../tests/README.md) directory documents cross-crate scenario ownership, and the root [`fixtures/`](../../../fixtures/README.md) directory contains their inputs.
