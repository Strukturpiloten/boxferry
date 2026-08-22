# Architecture

BoxFerry composes independent native adapters around one provenance-aware application model.

```text
ComposeLens ── boxferry-compose ──┐
PodmanLens ─── boxferry-podman ───┼── boxferry-model ── boxferry-engine
QuadletLens ── boxferry-quadlet ──┘                         │
                                                           └── boxferry facade and CLI
```

## Ownership

- `boxferry-model` owns format-neutral application intent and protected values.
- `boxferry-engine` owns adapter traits, planning, target profiles, diagnostics, and loss policy.
- `boxferry-compose`, `boxferry-podman`, and `boxferry-quadlet` own semantic mappings.
- `boxferry` owns the public facade, CLI parsing, presentation, reports, and file-write boundary.
- ComposeLens, PodmanLens, and QuadletLens own their native parsing or protocol handling, rendering,
  source evidence, and native findings.

The neutral model never exposes Lens types. The CLI calls the same public orchestration APIs used
by embedded Rust callers.

## Conversion phases

1. Parse with the native Lens or explicitly acquire a read-only native inventory.
2. Import into the neutral model and retain source outcomes.
3. Resolve the explicit target profile.
4. Export a typed candidate and add target outcomes.
5. Apply the selected loss policy to the combined plan.
6. Write create-new files only when authorized.

Parsing, acquisition, and planning have no mutating runtime side effects. Podman acquisition accepts
an explicit caller-owned transport and discovery request; it does not discover an ambient connection
or shell out to `podman`. BoxFerry never applies output or sends mutating runtime requests. Target
versions and rootful/rootless context are explicit and never inferred from the development machine.

## Same-format behavior

Every route uses the same importer-to-neutral-model-to-exporter pipeline. Same-format conversion is
not passthrough or native canonicalization. Native-only intent remains visible through normal
outcomes and loss policy.

## Podman output boundary

PodmanLens owns Podman protocols, read-only acquisition, native types, version evidence,
diagnostics, deployment planning, and deterministic rendering. BoxFerry maps a neutral
`Application` to PodmanLens deployment intent and writes only complete, inert `podman.json` and
`review.sh` artifacts. It exposes no executor.

Podman output is not re-importable Podman input: the output represents desired operations, while
input is an acquired runtime inventory and resource graph. Compose and Quadlet outputs receive
chained re-import and fixed-point tests; Podman output receives deterministic-byte and
semantic-operation tests.

## Deferred native formats

Docker and Kubernetes integrations remain deferred. BoxFerry adds no placeholder adapter and never
becomes an infrastructure deployment tool. See
[ADR 0034](decisions/0034-podman-lens-adapter-boundary.md).
