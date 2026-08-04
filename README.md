# BoxFerry

BoxFerry is a loss-aware Rust library and command-line application for migrating and converting
container application definitions.

The project is intended to help people move applications between Docker Compose, Podman Quadlet, and Kubernetes without pretending that these environments are perfectly equivalent. BoxFerry will preserve intent where possible and report every approximation, unsupported feature, and required manual action.

## Goals

- Convert supported application definitions in both directions where the semantics allow it.
- Import existing Docker and Podman resources through runtime inspection.
- Produce actionable compatibility and loss reports instead of silently dropping configuration.
- Account for target versions, including Podman and Kubernetes feature differences.
- Keep format parsing in focused libraries rather than embedding every format in the application.
- Let Rust applications embed conversion planning and adapters without invoking the CLI.
- Remain useful for real-world files that contain extensions and implementation-specific behavior.

## Initial scope

Planned inputs include:

- Docker Compose files
- Podman Quadlet files
- Kubernetes resources
- Rendered Helm and Kustomize resources
- Docker and Podman runtime inspection data
- Selected Docker and Podman commands

Planned outputs include:

- Docker Compose files
- Podman Quadlet files
- Kubernetes resources
- Compatibility reports and manual migration guidance

Helm chart and Kustomize overlay generation are later capabilities. Their rendered Kubernetes resources can be consumed earlier.

## Related repositories

- [ComposeLens](https://github.com/Strukturpiloten/compose-lens) parses, models, validates, resolves, and renders Compose documents.
- [QuadletLens](https://github.com/Strukturpiloten/quadlet-lens) parses, models, validates, and renders version-aware Quadlet documents.

BoxFerry owns the application model, conversion planning, runtime adapters, and mappings between native formats. The Lens libraries do not depend on BoxFerry.

## Command-line use

The first command converts explicitly ordered Compose files into a new directory of validated
Quadlet files:

```shell
cargo run -p boxferry -- compose-to-quadlet \
  --file compose.yaml \
  --file compose.production.yaml \
  --project-name example \
  --profile production \
  --podman-minimum-version 5.4.0 \
  --podman-maximum-version 6.0.2 \
  --loss-policy exact \
  --output-directory ./quadlet-output
```

The output directory must not already exist. `exact` is the default policy; `approximate` and
`partial` authorize their corresponding documented losses. The command does not read the process
environment, so Compose variable expressions remain unresolved and may block output instead of
silently capturing workstation values. See the [CLI contract](docs/cli.md).

## Library use

The `boxferry` crate is both the high-level library facade and the package that provides the
`boxferry` executable. External Rust projects use the same model, planning, diagnostic, and adapter
APIs as the CLI. Applications with narrower requirements may depend on component crates such as
`boxferry-model`, `boxferry-engine`, `boxferry-compose`, or `boxferry-quadlet`.

The crates remain unpublished while their pre-1.0 contract is exercised. The current facade can
already be used from a repository checkout to implement an [`ImportAdapter`](docs/library-api.md)
and [`ExportAdapter`](docs/library-api.md), call `boxferry::convert`, and receive a typed
`ConversionResult` instead of parsing CLI output.

The additive `compose` feature exposes `ComposeImporter` and `ComposeSource` through the facade.
The importer consumes an explicit ComposeLens merged project, maps its native project view without
rendering and reparsing YAML, preserves every contributing source origin, requires a matching
ComposeLens profile selection whenever profiles are present, retains SELinux relabel intent, and
reports unsupported source features as policy-controlled conversion outcomes.

The additive `quadlet` feature exposes `QuadletExporter` and its validated file-set output. The
exporter uses QuadletLens 0.1.5 for typed native construction and capability evidence, supports
Podman 5.4.0 through the finite current catalogue ceiling, keeps each service in its own container
unit by default, distinguishes application-owned and external resources, preserves absolute and
systemd-specifier bind paths, and reports every omitted target feature through the same loss policy.
Callers can also provide an explicit absolute Compose project root to resolve `./` and `../` bind
sources lexically without filesystem access.
Tilde, Windows, and other host-specific bind spellings are never guessed; embedded callers can map
an exact source spelling to an absolute or systemd-specifier target explicitly.
Explicit single-pod grouping is available only for compatible declared networks, ports, and
ordered host mappings. It is
reported as an approximation because sharing a network namespace changes Compose service
isolation, and incompatible requests fail without an automatic fallback.

The neutral model preserves explicit service host mappings, including the `host-gateway` runtime
token and IPv4/IPv6 addresses. Compose `extra_hosts` convert to capability-checked Quadlet
`AddHost` entries. Separate services retain container-level mappings; explicitly grouped services
must declare identical ordered mappings, which move to the generated pod.

The neutral model also preserves Compose health-check command form, explicit disable intent,
durations, retries, startup grace period, and `start_interval` with field-level provenance. The
Quadlet adapter emits the capability-checked regular health-check subset and reports
`start_interval` as an explicit loss because Quadlet has no equivalent key.

ComposeLens 0.1.5 service dependencies retain source order, short/long defaults, and field-level
provenance in the neutral graph. Required and optional startup dependencies become capability-
checked systemd `Requires`/`Wants` plus `After` directives. A healthy dependency additionally
enables `Notify=healthy` only when BoxFerry can establish an explicit target health command.
Compose-controlled restart propagation and successful-completion conditions remain explicit
partial losses; missing required services and ordering cycles are invalid.

The neutral model and adapters retain provenance-aware primary user/group, user namespace, ordered
supplementary groups, working directory, and read-only-root intent through ComposeLens and
QuadletLens 0.1.5. Separate-container output maps numeric primary GIDs and named or numeric
supplementary groups; named primary groups remain explicit losses because Quadlet's native
`Group=` contract is numeric. Output is capability-checked across the supported Podman range.
Explicit pod grouping reports container-level user namespaces as a loss because Podman uses the
pod's namespace and BoxFerry cannot yet generate Quadlet's pod-level `UserNS=` key.

The format-independent graph now also represents application-owned or external configuration and
secret resources, optional runtime names and material origins, and ordered short/long service
grants with per-option provenance. Sensitive material and grant values use the same redacting
`ProtectedString` boundary as environment values. Compose and Quadlet adapter integration remains
gated on the independent ComposeLens and QuadletLens 0.1.6 releases.

The CLI remains a thin consumer of the public facade. It must not gain private conversion behavior
that embedded users cannot call. See the [library API and publication policy](docs/library-api.md).

## Documentation

Start with the [documentation index](docs/README.md). Important design documents include:

- [Software architecture](docs/architecture.md)
- [Target project structure](docs/project-structure.md)
- [Library API and publication policy](docs/library-api.md)
- [API stability](docs/api-stability.md)
- [Conversion model and diagnostics](docs/conversion-model.md)
- [Format coverage](docs/format-coverage.md)
- [Quadlet exporter](docs/quadlet-adapter.md)
- [Testing strategy](docs/testing.md)
- [Development environment](docs/development-environment.md)
- [Release policy](docs/releasing.md)
- [Podlet and compose_spec_rs issue-corpus review](docs/research/podlet-compose-spec-rs-issues-2026-08-01.md)
- [Cross-repository implementation plan](docs/implementation-plan.md)
- [Roadmap](docs/roadmap.md)
- [Architecture decisions](docs/decisions/README.md)

Repository-specific guidance for coding agents is in [AGENTS.md](AGENTS.md).

## Origin

BoxFerry is a new implementation. It is not a fork or continuation of Podlet, and source code is not imported from Podlet. Existing tools may be used for behavioral comparison and interoperability testing, with their provenance recorded in test metadata.

## Stewardship

BoxFerry is created and maintained by [Martin “Becks” Beckert](https://github.com/TheRealBecks) through [Strukturpiloten OHG](https://www.strukturpiloten.de/). The project is part of Strukturpiloten's work on open, maintainable, and portable container infrastructure.

## License

BoxFerry is licensed under the [Mozilla Public License 2.0](LICENSE).
