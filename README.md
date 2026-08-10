# BoxFerry

BoxFerry is a loss-aware Rust library and command-line application for migrating and converting
container application definitions.

The project is intended to help people move applications between Docker, Docker Compose, Podman,
Podman Quadlet, and Kubernetes without pretending that these environments are perfectly
equivalent. Every supported source can be converted into every supported target through the same
neutral application model. BoxFerry preserves intent where possible and reports every
approximation, unsupported feature, and required manual action.

## Goals

- Import every supported source into one format-independent, provenance-aware application model.
- Export that model to every supported target where the semantics allow it.
- Import existing Docker and Podman resources through runtime inspection.
- Produce actionable compatibility and loss reports instead of silently dropping configuration.
- Account for target versions, including Podman and Kubernetes feature differences.
- Keep format parsing in focused libraries rather than embedding every format in the application.
- Let Rust applications embed conversion planning and adapters without invoking the CLI.
- Remain useful for real-world files that contain extensions and implementation-specific behavior.

## Initial scope

The first major milestone is the complete source/target matrix for Docker runtime resources,
Docker Compose, Podman runtime resources, and Podman Quadlet. Kubernetes follows as another input
and output adapter; it is not a separate conversion engine. Runtime targets produce a reviewable
deployment plan before any optional, explicitly authorized mutation of Docker or Podman.

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
`partial` authorize their corresponding documented losses. Compose interpolation is disabled by
default, so expressions remain unresolved rather than silently capturing workstation values.
Callers can opt in with `--interpolate`, add repeatable plain `--variable NAME=VALUE` inputs, and
authorize individual sensitive process values with repeatable
`--variable-from-environment NAME`. No other ambient variable or implicit `.env` file is read.
See the [CLI contract](docs/cli.md).

## Library use

The `boxferry` crate is both the high-level library facade and the package that provides the
`boxferry` executable. External Rust projects use the same model, planning, diagnostic, and adapter
APIs as the CLI. Applications with narrower requirements may depend on component crates such as
`boxferry-model`, `boxferry-engine`, `boxferry-compose`, or `boxferry-quadlet`.

The additive `runtime` feature exposes a pure runtime-neutral observation and reconstruction
boundary. Embedded callers explicitly choose whether to preserve supported effective state or
infer command, environment, protected metadata-label, user/group-identity, working-directory, and
regular-health-check overrides by comparing linked container and image observations. Compose-
managed `com.docker.compose.*` labels remain visible but receive a dedicated unsafe-to-reauthor
diagnostic. Effective read-only-root state is preserved directly, as is container-level restart policy. Quadlet restart output distinguishes
exact `Never`, approximate unbounded policies, and unsupported finite retry limits. Podman's
separate startup-healthcheck family remains native-specific and is
never substituted for Docker/Compose start-interval semantics.
Every runtime reconstruction reports that original author intent is uncertain, inspected command,
environment, and metadata-label contents are sensitive by default, and optional creation commands contribute
provenance without becoming a required source of truth. Consistent Podman pod membership becomes
an ordered neutral `ServiceGroup`; it records structural membership without inventing shared-
namespace or lifecycle semantics. Embedded callers can resolve exact observed network, volume,
and group names as application-owned or external only through provenance-bearing
`RuntimeResolutions`; there is no implicit lifecycle default. Podman response parsing is available
behind the non-default `podman-runtime` feature for explicit 5.4.0-through-6.0.2 inspect arrays.
Embedded callers can acquire explicitly selected resources through a replaceable read-only Podman
executor. An explicit finite policy can add selected pods' member containers and selected
containers' images, networks, and named volumes without enumerating ambient resources. A
runtime-migration CLI command remains open.

Docker response parsing is available behind the non-default `docker-runtime` feature for the
finite Engine API 1.40-through-1.55 range. Its pure decoder accepts explicit container, image,
network, and volume inspect arrays. Its replaceable inspector requires an explicit executable,
protected daemon endpoint, and exact API version; it never relies on Docker's ambient context or
enumerates a resource family. Callers may explicitly authorize a bounded expansion from selected
containers to referenced images, networks, and named volumes.

A separate weekly/manual conformance workflow runs the digest-pinned official Docker Engine 29.7.1
daemon inside an ephemeral privileged container and forces both reviewed API bounds. It mounts no
host runtime socket or repository write path. This verifies current-daemon API 1.40 compatibility
responses and API 1.55 responses; it does not claim to reproduce every historical Docker 19.03
implementation detail.

An opt-in, separately scheduled conformance workflow decodes real inspect output from exact,
digest-pinned official Podman images for the available supported 5.4-through-5.8 minor lines. It
runs nested Podman in an ephemeral privileged container without mounting a host runtime socket.
The source-reviewed 6.0.2 decoder ceiling remains an explicit reproducible scheduled-image
evidence gap until an exact immutable local-runtime image or reviewed build lane is available.

Developers who already have the exact reviewed 6.0.2 ceiling may run a separate opt-in current-
runtime test. It verifies the version first, creates only uniquely named temporary resources, and
removes those resources after inspection; it is not enabled by normal tests or pull-request CI.

The crates remain unpublished while their pre-1.0 contract is exercised. The current facade can
already be used from a repository checkout to implement an [`ImportAdapter`](docs/library-api.md)
and [`ExportAdapter`](docs/library-api.md), call `boxferry::convert`, and receive a typed
`ConversionResult` instead of parsing CLI output.

The additive `compose` feature exposes `ComposeImporter`, `ComposeSource`, `ComposeExporter`,
`ComposeRuntime`, and provider target constants through the facade.
The importer consumes an explicit ComposeLens merged project, maps its native project view without
rendering and reparsing YAML, preserves every contributing source origin, requires a matching
ComposeLens profile selection whenever profiles are present, retains SELinux relabel intent, and
reports unsupported source features as policy-controlled conversion outcomes.

The exporter consumes the neutral application and an exact Docker Compose or `podman-compose`
provider target, with an optional exact Docker Engine or Podman backend. ComposeLens 0.1.13 owns
deterministic short/long syntax choices, sensitive-output redaction, and parse-back validation,
including ordered short/long service `env_file` output.
BoxFerry reports compatibility-sensitive tag-plus-digest images, `host-gateway`, Podman user
namespaces, SELinux relabeling, and SCTP before the caller authorizes output. Runtime-observed
network and volume names are emitted explicitly so Compose project scoping cannot rename them.
See the [Compose exporter contract](docs/compose-adapter.md).

The additive `quadlet` feature exposes `QuadletImporter`, `QuadletSource`, `QuadletExporter`, and
the validated file-set output. The recommended `QuadletSource::parse` boundary accepts explicit
in-memory unit files, retains native parse validity, and maps direct container images, explicit
container names, safe unquoted exec arguments, single explicit environment assignments, scalar
published ports, named and absolute-bind mounts, named network attachments, and application-owned
or explicitly external network and volume resources. The same exact slice includes single
metadata-label assignments, host mappings (including `host-gateway` and bracketed IPv6), user and
numeric group identity, supplementary groups, user namespaces, absolute container working
directories, and explicit read-only-root state. Environment values enter the neutral model as
protected values. Repeated absolute-literal `EnvironmentFile=` declarations retain order and
protected paths without file I/O; parser parity remains an explicit approximation, while relative
and systemd-specifier paths require caller context and stay unsupported. Regular health checks
import from `HealthCmd=`, `HealthInterval=`,
`HealthTimeout=`, `HealthRetries=`, and `HealthStartPeriod=`; JSON command arrays and conservative
plain command strings are protected, while `HealthCmd=none` retains explicit disable intent.
Repeatable `Secret=` entries import as grants to explicitly external Podman secret resources.
The default and explicit `type=mount` forms retain source, target, UID, GID, mode, option order,
and source provenance without reading or inventing secret material. Environment exposure and
unreviewed options remain explicit unsupported outcomes.
Application-owned `.pod` documents with an explicit `PodName=` matching the unit stem and sibling
container `Pod=<name>.pod` references become provenance-aware neutral service groups independent
of source document order. Implicit or divergent runtime pod names and pod-scoped settings remain
explicit losses until the neutral model can retain those values without assigning them to an
arbitrary member service.
Section-aware import additionally maps `Restart=no` exactly, records
`Restart=always` and `Restart=on-failure` as explicit systemd-to-container approximations, and
turns complete sibling `Requires=`/`Wants=` plus `After=` pairs into ordered neutral startup
dependencies. References to arbitrary host units and incomplete dependency pairs remain visible
instead of becoming invented application services. Native forms that require systemd quoting,
shell parsing, path resolution,
network-mode interpretation, or unsupported mount/port options remain policy-controlled
unsupported outcomes; duplicate singleton keys and invalid resource graphs fail explicitly. The
exporter uses QuadletLens 0.1.9 for typed native construction and capability evidence, supports
Podman 5.4.0 through the finite current catalogue ceiling, keeps each service in its own container
unit by default, distinguishes application-owned and external resources, preserves absolute and
systemd-specifier bind paths, and reports every omitted target feature through the same loss policy.
Callers can also provide an explicit absolute Compose project root to resolve relative bind and
environment-file paths lexically without filesystem access.
Tilde, Windows, and other host-specific bind spellings are never guessed; embedded callers can map
an exact source spelling to an absolute or systemd-specifier target explicitly.
Explicit single-pod grouping is available only for compatible declared networks, ports, and
ordered host mappings. It is
reported as an approximation because sharing a network namespace changes Compose service
isolation, and incompatible requests fail without an automatic fallback.
`QuadletGroupingPolicy::PreserveSingleGroup` separately preserves one complete application-owned
neutral group under its own name. This also requires approximation authorization because neutral
membership alone does not prove shared Podman namespace semantics.

The neutral model preserves explicit service host mappings, including the `host-gateway` runtime
token and IPv4/IPv6 addresses. Compose `extra_hosts` convert to capability-checked Quadlet
`AddHost` entries. Separate services retain container-level mappings; explicitly grouped services
must declare identical ordered mappings, which move to the generated pod.

ComposeLens 0.1.13 service `env_file` declarations enter the neutral graph without opening the
referenced files. Short/long syntax, order, protected paths, explicit `required` and `format`
options, and nested provenance remain available to embedded callers and generate back to Compose
through the same syntax family. Required safe paths emit
repeatable capability-checked Quadlet `EnvironmentFile=` entries after lexical project-root
resolution. They require approximation authorization until Podman parser parity with Compose's
default and `raw` formats is proven; optional files and unsafe or ambiguous paths remain explicit
partial losses. File-content loading is a separate future authorization boundary.

An optional neutral service runtime name remains distinct from the service key. Compose
`container_name` imports with complete merge provenance and emits as capability-checked Quadlet
`ContainerName=`. Runtime reconstruction sets the inspected container name explicitly so generated
Compose or Quadlet definitions preserve the reviewed identity instead of relying on provider name
generation.

The neutral model also preserves Compose health-check command form, explicit disable intent,
durations, retries, startup grace period, and `start_interval` with field-level provenance. The
Quadlet adapter emits the capability-checked regular health-check subset and reports
`start_interval` as an explicit loss because Quadlet has no equivalent key.

Compose mapping and sequence service labels now enter the protected neutral model with complete
multi-file provenance. BoxFerry generates deterministic Compose label mappings and native
repeatable Quadlet `Label=` entries through ComposeLens 0.1.13 and QuadletLens 0.1.9. Empty and
quoted values are preserved, and literal `%` is escaped so systemd cannot turn label metadata into
a specifier expansion. Reserved `com.docker.compose.*` labels stay visible in diagnostics but are
never re-authored. Resource, annotation, and label-file ownership remain separate follow-up work;
image-build labels use the distinct image-artifact conversion contract.

ComposeLens 0.1.13 service dependencies retain source order, short/long defaults, and field-level
provenance in the neutral graph. Required and optional startup dependencies become capability-
checked systemd `Requires`/`Wants` plus `After` directives. A healthy dependency additionally
enables `Notify=healthy` only when BoxFerry can establish an explicit target health command.
Compose-controlled restart propagation and successful-completion conditions remain explicit
partial losses; missing required services and ordering cycles are invalid.

ComposeLens 0.1.13 service `restart` values now map into the distinct neutral container restart
policy and generate back to Compose without changing their meaning. Compose-to-Quadlet emits
`restart: "no"` exactly as systemd `Restart=no`; `always`, unbounded `on-failure`, and
`unless-stopped` remain explicit systemd approximations, while finite retry limits have no safe
systemd equivalent. Unresolved expressions, explicit zero retry limits, and counters outside the
neutral `u64` range are invalid rather than silently normalized.

The neutral model and adapters retain provenance-aware primary user/group, user namespace, ordered
supplementary groups, working directory, and read-only-root intent through ComposeLens and
QuadletLens 0.1.9. Separate-container output maps numeric primary GIDs and named or numeric
supplementary groups; named primary groups remain explicit losses because Quadlet's native
`Group=` contract is numeric. Output is capability-checked across the supported Podman range.
Explicit pod grouping moves one identical namespace choice declared by every service to the pod's
capability-checked `UserNS=` key. Mixed implicit/explicit or conflicting namespace choices
invalidate the requested grouping rather than selecting a value by service order.

The format-independent graph now also represents application-owned or external configuration and
secret resources, optional runtime names and material origins, and ordered short/long service
grants with per-option provenance. Sensitive material and grant values use the same redacting
`ProtectedString` boundary as environment values. ComposeLens 0.1.13 imports these definitions and
grants. QuadletLens 0.1.9 emits exact mounted-file `Secret=` references for pre-existing external
Podman secrets, including custom-name default preservation and validated target, UID, GID, and
read-only mode options. Application-owned secret materialization and Compose config lifecycle are
explicit manual actions because Quadlet container units cannot create those resources.

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
- [Runtime reconstruction](docs/runtime-reconstruction.md)
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
