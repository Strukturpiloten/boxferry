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

Parsing and planning have no runtime side effects. BoxFerry does not infer target versions from the
development machine.

## Same-format behavior

Compose-to-Compose uses ComposeLens native canonicalization so unresolved expressions and extension
data remain native. The other three routes use the neutral model.

## Future native formats

DockerLens, PodmanLens, and KubernetesLens will own their native protocols, version evidence,
deployment plans, and execution safety. BoxFerry will add only semantic mappings after those
libraries publish reviewed contracts. See [ADR 0032](decisions/0032-future-native-lens-boundaries.md).
