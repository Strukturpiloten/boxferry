# Architecture

Each native format has an importer and exporter around one provenance-aware application model.

```text
ComposeLens ── Compose adapter ──┐
                                ├── model ── planner ── loss policy
QuadletLens ── Quadlet adapter ──┘
```

## Ownership

- `boxferry-model` owns format-neutral application intent and protected values.
- `boxferry-engine` owns adapters, planning, diagnostics, target profiles, and loss policy.
- `boxferry-compose` and `boxferry-quadlet` own semantic mappings.
- `boxferry` exposes the Rust facade and CLI presentation.
- ComposeLens and QuadletLens own native parsing, rendering, and native diagnostics.

The CLI calls the same public orchestration API available to Rust callers. Parsing never inspects a
runtime, and output is planned before any file is created.

PodmanLens owns Podman protocols, read-only acquisition, native types, version evidence,
diagnostics, and deterministic rendering. Podman semantic integration is next and does not wait for
DockerLens. Docker and Kubernetes integrations remain deferred. BoxFerry adds semantic mappings and
never applies or deploys output.

Accepted decisions live in the repository's `docs/decisions/` directory.
