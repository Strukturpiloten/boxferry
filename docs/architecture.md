# Architecture

Use this guide to decide where behavior belongs and which boundaries a change must preserve.

## Conversion pipeline

```text
native source → Lens → importer → Application → exporter → native candidate
                                      │
                         outcomes + target profile
                                      │
                                loss policy
                                      │
                           authorized inert output
```

Every route follows this pipeline, including Compose-to-Compose, Quadlet-to-Quadlet, and
Podman-to-Podman. A native parser or renderer cannot bypass the neutral model.

1. Parse a document or explicitly acquire a read-only inventory.
2. Import native intent and findings into the neutral `Application`.
3. Resolve the caller-selected target profile.
4. Export a typed candidate and collect target findings.
5. Apply loss policy to the combined conversion plan.
6. Write create-new files only when policy authorizes the result.

Parsing, acquisition, import, export, and planning do not mutate a runtime. The CLI calls the same
public orchestration APIs available to embedded callers.

## Ownership

| Owner                    | Responsibility                                                       |
| ------------------------ | -------------------------------------------------------------------- |
| `boxferry-model`         | Format-neutral intent, provenance, and protected values              |
| `boxferry-engine`        | Adapter traits, planning, target profiles, outcomes, and loss policy |
| `boxferry-compose`       | Compose ↔ neutral semantic mapping                                   |
| `boxferry-podman`        | PodmanLens ↔ neutral semantic mapping                                |
| `boxferry-quadlet`       | QuadletLens ↔ neutral semantic mapping                               |
| `boxferry`               | Facade, CLI, reports, presentation, and create-new file boundary     |
| Native Lens repositories | Native parsing/acquisition, rendering, evidence, and native findings |

The neutral model contains no Lens types. Format-specific behavior stays in the owning Lens;
cross-format meaning stays in the corresponding BoxFerry adapter; orchestration stays out of the
CLI layer.

## Podman adapter contract

PodmanLens reports whether a field is absent, configured, effective, runtime-assigned, locally
resolved, unavailable, malformed, version-inapplicable, not applicable, or unmodelled.
`boxferry-podman` must handle that state before reading a value:

- configured values may become neutral intent when the semantics match;
- effective values require an explicit promotion policy;
- runtime-assigned and locally resolved values remain evidence unless the caller authorizes them;
- unavailable, malformed, ambiguous, and unmodelled data produce structured outcomes;
- redacted data never becomes an empty value or reconstructed secret.

The adapter preserves Podman resource identity for correlation, rejects normalized-name
collisions, and starts inspected ownership as uncertain. Pod networking belongs to the neutral
service group; unpodded container networking belongs to the service. Exact field mappings and
regressions live with `boxferry-podman` source, Rustdoc, and tests rather than in a prose ledger.

For output, the adapter constructs typed PodmanLens image, network, volume, secret, pod, container,
and dependency intent. It returns every planning and rendering finding. The exact Podman version
and rootful, rootless, or unknown context come from the target profile, never from the source or
development host.

## Output boundary

Compose and Quadlet output are documents that can be imported again, so tests cover chained
conversion and fixed points. Podman output is different: `podman.json` and `review.sh` describe
desired operations for review and cannot be used as observed Podman input. Tests therefore protect
deterministic bytes, semantic operations, findings, and redaction.

BoxFerry exposes no executor, invokes no generated command, starts no unit, and sends no mutating
runtime request. Docker and Kubernetes remain deferred until independent native libraries and
reviewed adapter contracts exist.

## Change checklist

- Put the behavior in its owning layer.
- Preserve source evidence long enough to explain every decision.
- Add positive, failure, unsupported, and target-boundary tests where relevant.
- Update machine evidence rather than copying its rows into prose.
- Add or supersede an [ADR](decisions/) when an architectural constraint changes.
