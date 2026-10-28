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
                        authorized reviewable output
```

Every route follows this pipeline, including Compose-to-Compose, Docker-to-Docker,
Quadlet-to-Quadlet, and Podman-to-Podman. A native parser or renderer cannot bypass the neutral model.

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
| `boxferry-docker`        | DockerLens observations ↔ neutral intent ↔ inert Docker plans        |
| `boxferry-podman`        | PodmanLens ↔ neutral semantic mapping                                |
| `boxferry-quadlet`       | QuadletLens ↔ neutral semantic mapping                               |
| `boxferry`               | Facade, CLI, reports, presentation, and create-new file boundary     |
| Native Lens repositories | Native parsing/acquisition, rendering, evidence, and native findings |

Published format-adapter crates do not depend on sibling adapters in normal, build, development,
or target-specific dependency sections. Adapter tests stop at their neutral boundary; N:N route
composition tests belong to the `boxferry` facade.

The neutral model contains no Lens types. Format-specific behavior stays in the owning Lens;
cross-format meaning stays in the corresponding BoxFerry adapter; orchestration stays out of the
CLI layer.

## Podman adapter contract

PodmanLens reports whether a field is absent, configured, effective, runtime-assigned, locally
resolved, unavailable, malformed, version-inapplicable, not applicable, or unmodelled.
`boxferry-podman` must handle that state before reading a value:

- configured values may become neutral intent when the semantics match;
- effective values require a reviewed promotion policy; the CLI's portable preset covers only
  the bounded application subset, while embedded callers remain conservative by default;
- runtime-assigned and locally resolved values remain evidence unless the caller authorizes them;
- unavailable, malformed, ambiguous, and unmodelled data produce structured outcomes;
- redacted data never becomes an empty value or reconstructed secret.

The CLI's reviewed source-snapshot reconstruction and target-exporter fidelity are separate
decisions. Exact source reconstruction carries an informational per-field diagnostic and both
observation and conversion-decision provenance; semantic adjustments and inferred named-resource
ownership remain non-exact. See [ADR 0060](decisions/0060-portable-podman-cli-import-policy.md).

The adapter preserves Podman resource identity for correlation, rejects normalized-name
collisions, and starts inspected ownership as uncertain. Pod networking belongs to the neutral
service group; unpodded container networking belongs to the service. Exact field mappings and
regressions live with `boxferry-podman` source, Rustdoc, and tests rather than in a prose ledger.

For output, the adapter constructs typed PodmanLens image, network, volume, secret, pod, container,
and dependency intent. It returns every planning and rendering finding. The exact Podman version
and rootful, rootless, or unknown context come from the target profile, never from the source or
development host.

PodmanLens also owns bounded decoding of optional creation evidence and native mount relabel values.
BoxFerry uses those hints only to corroborate typed configured inspect fields. They never create
neutral image builds, mounts, commands, or environment. Effective named resources and portable
settings need reviewed promotion: the CLI portable preset enables their bounded application subset,
while embedded callers and the conservative CLI mode require independent choices. Absolute bind
sources remain reviewed same-host evidence. See
[ADR 0042](decisions/0042-bounded-podman-creation-evidence-and-intent-promotion.md).

## Output boundary

Compose and Quadlet output are documents that can be imported again, so tests cover chained
conversion and fixed points. Podman output is different: `podman.json` and `podman-commands.sh`
describe desired operations and cannot be used as observed Podman input. BoxFerry never executes
the command script, but a user who runs it performs real Podman operations. Tests therefore protect
deterministic bytes, semantic operations, findings, and redaction.

Docker output is also terminal review material: `docker-plan.json` contains native inert requests,
not observed inventory. Conditional `docker-dependencies.json` carries typed dependency decisions,
provenance and `native_execution=false`, bound by SHA-256 to the exact plan bytes. Dependency
conditions are manual obligations, not Engine fields or readiness execution. Review external
resource, bind-source and destination obligations separately. POST entries inside a plan are data;
actual acquisition uses only read-only GET requests.

BoxFerry exposes no executor, invokes no generated command, starts no unit, and sends no mutating
runtime request. Kubernetes remains deferred.

## Docker development contract

The Unreleased CLI registers four inputs and four outputs, all 16 neutral routes. Default CLI
features include all four adapters; embedded callers can disable defaults and select adapters.
The `docker` feature itself exposes library integration without requiring the CLI. DockerLens
owns native acquisition/decoding, types, version/context evidence, validation, planning and
serialization; no Lens product depends on BoxFerry.

`acquire_docker_source` requires an explicit application name, Unix endpoint, selector, finite
limits and cancellation flag. The importer consumes already decoded observations. Library sources
start with unspecified selection and conservative promotion. The CLI adds bounded local selection
and a portable reconstruction preset, independently of the exact-default loss policy. Source
reconstruction does not prove authorship, grant lifecycle ownership, choose a target or authorize
sensitive artifact values. Environment, binds, public labels and resource decisions have separate
controls described in the [Docker guide](public/guides/docker-input/).

Docker targets require one exact reviewed Debian 11 or upstream profile in the chosen daemon mode;
source/host facts never select output profiles. Required intent that is incomplete or protected can
block the complete Docker candidate under every loss policy. A permissive policy is not a general
secret-dropping mechanism. Docker files use owner-only/create-new publication; diagnostics and
reports retain default redaction.

[ADR 0085](decisions/0085-docker-cli-neutral-routes.md) extends the historical
[target checkpoint](decisions/0083-bounded-docker-target-consumer-preparation.md) and
[source library checkpoint](decisions/0084-bounded-docker-source-adapter.md). Source tests,
native Lens evidence, exact-pair admission, actual BoxFerry consumer/application acceptance and
normal registry/package/API checks remain distinct. Source integration alone establishes none of
the latter qualifications.

## Change checklist

- Put the behavior in its owning layer.
- Preserve source evidence long enough to explain every decision.
- Add positive, failure, unsupported, and target-boundary tests where relevant.
- Update machine evidence rather than copying its rows into prose.
- Add or supersede an [ADR](decisions/) when an architectural constraint changes.
