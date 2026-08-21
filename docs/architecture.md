# Architecture

BoxFerry composes independent native adapters around one provenance-aware application model.

```text
ComposeLens ── boxferry-compose ──┐
                                 ├── boxferry-model ── boxferry-engine
QuadletLens ── boxferry-quadlet ──┘                         │
                                                           └── boxferry facade and CLI
```

## Ownership

- `boxferry-model` owns format-neutral application intent and protected values.
- `boxferry-engine` owns adapter traits, planning, target profiles, diagnostics, and loss policy.
- `boxferry-compose` and `boxferry-quadlet` own semantic mappings.
- `boxferry` owns the public facade, CLI parsing, presentation, reports, and file-write boundary.
- ComposeLens and QuadletLens own native parsing, rendering, source locations, and native findings.

The neutral model never exposes Lens types. The CLI calls the same public orchestration APIs used
by embedded Rust callers.

## Conversion phases

1. Parse with the native Lens.
2. Import into the neutral model and retain source outcomes.
3. Resolve the explicit target profile.
4. Export a typed candidate and add target outcomes.
5. Apply the selected loss policy to the combined plan.
6. Write create-new files only when authorized.

Parsing and planning have no mutating runtime side effects. Explicit read-only acquisition can feed
a native importer, but BoxFerry never applies output or sends mutating runtime requests. BoxFerry
does not infer target versions from the development machine.

## Same-format behavior

Every route uses the same importer-to-neutral-model-to-exporter pipeline. Same-format conversion is
not passthrough or native canonicalization. Native-only intent remains visible through normal
outcomes and loss policy.

## Future native formats

PodmanLens owns Podman protocols, read-only acquisition, native types, version evidence,
diagnostics, and deterministic rendering. Podman semantic integration is Phase 2 and does not wait
for DockerLens. Docker and Kubernetes integrations remain deferred. BoxFerry adds semantic mappings
only and never becomes an infrastructure deployment tool. See
[ADR 0033](decisions/0033-universal-neutral-model-pipeline.md).
