# Architecture

Each native format has an importer and exporter around one provenance-aware application model.

```text
ComposeLens ── Compose adapter ──┐
PodmanLens ─── Podman adapter ───┼── model ── planner ── loss policy
QuadletLens ── Quadlet adapter ──┘
```

## Ownership

- `boxferry-model` owns format-neutral application intent and protected values.
- `boxferry-engine` owns adapters, planning, diagnostics, target profiles, and loss policy.
- `boxferry-compose`, `boxferry-podman`, and `boxferry-quadlet` own semantic mappings.
- `boxferry` exposes the Rust facade and CLI presentation.
- ComposeLens, PodmanLens, and QuadletLens own their native parsing or protocol handling, rendering,
  and native diagnostics.

The CLI calls the same public orchestration API available to Rust callers. Podman acquisition uses
an explicit read-only transport and discovery request; it never discovers an ambient connection or
shells out to `podman`. Every output is planned before a file is created.

PodmanLens owns Podman protocols, read-only acquisition, native types, version evidence,
diagnostics, deployment planning, and deterministic rendering. BoxFerry writes reviewable
`podman.json` plus runnable `podman-commands.sh`, but never executes, applies, or deploys either
artifact. This deployment output is not runtime inventory and cannot be used as Podman input.

All nine Compose, Quadlet, and Podman routes cross the neutral model. Docker and Kubernetes
integrations remain deferred.

Accepted decisions live in the repository's `docs/decisions/` directory.
