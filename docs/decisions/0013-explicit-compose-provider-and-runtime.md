# ADR 0013: explicit Compose provider, runtime, and generated-document boundary

- Status: accepted
- Date: 2026-08-04

## Context

A Compose document is interpreted by a provider and ultimately applied to a container runtime.
Those are separate compatibility dimensions. Docker Compose is a provider, while `podman compose`
is a wrapper that delegates to an external provider such as `podman-compose`. A provider version
does not identify its Docker Engine or Podman backend version.

ComposeLens 0.1.7 supplies deterministic, parse-back-validated construction plus compatibility
rules for exact provider versions and optional exact runtime versions. BoxFerry must expose those
decisions without reading installed tools, inventing a current version, or pretending an entire
version range has the evidence of one endpoint.

## Decision

`ComposeExporter` accepts only these `TargetProfile::implementation` values:

- `docker-compose`; and
- `podman-compose` for the independent `containers/podman-compose` provider.

The target minimum and maximum must be present and identical. The exact target version identifies
the provider. `podman compose` is rejected because it does not identify the delegated provider.

`ComposeExporter::with_runtime` optionally attaches an exact `ComposeRuntime::DockerEngine` or
`ComposeRuntime::Podman` version. Omitting it means that runtime-dependent claims remain subject to
the finite ComposeLens evidence available without a backend version; it never means “use the local
runtime.”

The exporter maps the neutral model into ComposeLens 0.1.7 generated values and returns its
`GeneratedComposeDocument` directly. ComposeLens owns syntax selection, deterministic YAML,
sensitive-output redaction, and parse-back validation. BoxFerry owns semantic mapping, provenance,
target compatibility classification, and policy-controlled outcomes.

Runtime-observed network and volume names receive explicit top-level `name` values so Compose
project scoping cannot silently rename them. Uncertain or implicit lifecycle is represented
conservatively as external reuse and remains an unsupported outcome requiring `AllowPartial`.
Exact lifecycle resolutions supplied to the runtime importer remain the preferred path.

## Consequences

- Embedded callers can reproduce compatibility decisions without installing Docker, Podman, or a
  Compose provider.
- Provider and backend upgrades are deliberate inputs and can be tested independently.
- The first exporter slice supports images, commands, environment, identity/context, host
  mappings, ports, mounts, networks, and network/volume lifecycle.
- Compose constructs whose selected provider/runtime behavior is implementation-specific or
  unknown receive stable `BFC0009` outcomes before output authorization.
- Health checks, dependencies, configs, secrets, service groups, and newer neutral variants remain
  explicit `BFC0007` partial losses until ComposeLens generation exposes reviewed equivalents.

## Alternatives considered

- **Use `podman compose` as the target implementation.** Rejected because the wrapper does not
  identify which external provider parses the document.
- **Treat `TargetProfile` as a provider-version range.** Rejected for this slice because
  ComposeLens compatibility evidence is exact-versioned and selecting one bound would overclaim
  the rest of the range.
- **Infer the runtime from the provider.** Rejected because Docker Compose can target non-Docker
  backends and provider identity does not establish a runtime release.
- **Render YAML inside BoxFerry.** Rejected because syntax ownership, short/long form selection,
  parse-back validation, and native-model evolution belong in ComposeLens.
