# Architecture

Each native format has an importer and exporter around one provenance-aware application model.

```text
ComposeLens ── Compose adapter ──┐
DockerLens ─── Docker adapter ───┤
PodmanLens ─── Podman adapter ───┼── model ── planner ── loss policy
QuadletLens ── Quadlet adapter ──┘
```

## Ownership

- `boxferry-model` owns format-neutral application intent and protected values.
- `boxferry-engine` owns adapters, planning, diagnostics, target profiles, and loss policy.
- `boxferry-compose`, `boxferry-docker`, `boxferry-podman`, and `boxferry-quadlet` own semantic mappings.
- `boxferry` exposes the Rust facade and CLI presentation.
- ComposeLens, DockerLens, PodmanLens, and QuadletLens own their native parsing or protocol
  handling, rendering and native diagnostics.

The CLI calls the same public orchestration API available to Rust callers. Podman acquisition uses
an explicit read-only transport and discovery request; it never discovers an ambient connection or
shells out to `podman`. Every output is planned before a file is created.

PodmanLens owns Podman protocols, read-only acquisition, native types, version evidence,
diagnostics, deployment planning, and deterministic rendering. BoxFerry writes reviewable
`podman.json` plus runnable `podman-commands.sh`, but never executes, applies, or deploys either
artifact. This deployment output is not runtime inventory and cannot be used as Podman input.

The Unreleased development CLI exposes all 16 Compose, Docker, Quadlet and Podman routes through
the neutral model, including Docker-to-Docker. DockerLens owns bounded read-only native acquisition
and complete inert serialization. The facade requires explicit transport/limits/selection;
CLI conventional local discovery adds no runtime mutation. Embedded Docker sources remain
conservative by default; CLI portable reconstruction, exact-default loss, resource decisions and
protected-value artifact consent are separate boundaries.

Docker output is `docker-plan.json` and a conditional SHA-256-bound dependency sidecar with
`native_execution=false`. Native requests are data, not transmitted operations. Exact reviewed
profiles are selected independently of source mode. See the [Docker guide](../../guides/docker-input/).
Installed releases may expose fewer routes. Native Lens results, actual BoxFerry consumers,
application migrations and ordinary published-dependency/API/package qualification remain
separate. No Lens product depends on BoxFerry. Kubernetes remains deferred.

Accepted decisions live in the repository's `docs/decisions/` directory.
