# Testing strategy

Tests are part of the product contract. A conversion feature is incomplete until its fidelity, failure modes, and target-version behavior are tested.

## Test layers

### Unit tests

Cover model invariants, individual mappings, target-profile resolution, diagnostics, redaction, and deterministic rendering choices.

### Adapter contract tests

Every adapter must demonstrate:

- native model to application model
- application model to native model
- unsupported-feature reporting
- preservation of provenance
- handling of unknown or extension data
- target capability boundaries

### Golden conversion scenarios

Each scenario contains source input, BoxFerry configuration, expected native output, and expected diagnostics. Golden updates require review of both file changes and semantic outcomes.

### Property and round-trip tests

Use generated inputs where useful to verify parsing never panics, deterministic output, native round trips, and application-model round trips.

### Differential tests

Docker, Podman, Compose implementations, Helm, Kustomize, and Kubernetes tools may be used as behavior oracles. Store the exact tool version, command, environment, and expected result. A difference is not automatically a BoxFerry bug; it must be classified.

### Runtime integration tests

Opt-in tests exercise real Docker, Podman, systemd/Quadlet, and Kubernetes environments. The initial supported Podman floor is 5.4. Test each supported minor version and the newest available version where CI infrastructure permits.

## Real-world corpus

Every imported fixture requires:

- source URL and immutable revision
- license and redistribution decision
- local modifications
- secrets review
- expected behavior or issue being tested

If redistribution is not permitted, store a generation script or minimal original reproduction instead of the source file.

## Security rules

- Never commit live credentials, tokens, private keys, or production inspect output.
- Redact secret values before snapshots.
- Give runtime tests isolated names and cleanup procedures.
- Do not make destructive cleanup broader than resources created by the test.

## Canonical commands

The workspace has not been scaffolded yet. Once it exists, this document must list the exact commands used by CI for formatting, linting, unit tests, integration tests, documentation, dependency policy, and minimum-supported-Rust-version checks.
