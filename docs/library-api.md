# Library API and publication policy

## Purpose

BoxFerry is both an embeddable Rust conversion system and a command-line application. External
projects must be able to parse through native adapters, build and inspect conversion plans, apply
an explicit loss policy, render supported targets, and consume structured diagnostics without
starting the `boxferry` executable.

This boundary is recorded in [ADR 0002](decisions/0002-public-library-facade.md).

## Consumption levels

### `boxferry` facade

The `boxferry` crate is the recommended dependency for applications that want a supported,
high-level API. It exposes the core model and engine and will expose stable format adapters through
additive Cargo features. The package also contains the `boxferry` executable.

The facade owns orchestration convenience APIs only when their inputs keep file access,
environment access, runtime inspection, target profiles, and loss policy explicit. It does not add
a single “magic conversion” function that reads ambient state.

### Component crates

- `boxferry-model` provides the format-independent application graph and provenance types.
- `boxferry-engine` provides adapter contracts, target profiles, planning, loss policy, outcomes,
  and diagnostics.
- `boxferry-compose`, `boxferry-quadlet`, and later format adapters provide native mappings and
  depend on their corresponding native libraries.
- `boxferry-runtime` provides pure runtime-neutral observation and reconstruction contracts.
- Runtime-specific adapter crates provide replaceable inspection interfaces and implementations.

Direct component dependencies are supported for applications building custom adapters, services,
editor integrations, language bindings, or their own user interface. Component APIs receive the
same pre-1.0 compatibility policy as the facade once published.

## CLI parity

The CLI may own:

- argument and configuration-file parsing;
- source discovery and authorized file access;
- terminal and machine-readable presentation;
- process exit status; and
- explicit invocation of optional verification tools.

It may not own conversion rules, private application-model mutations, target capability decisions,
or diagnostics that cannot be obtained from the public libraries. Public integration tests will
exercise the same orchestration path used by the executable.

## Feature policy

Format and runtime features are additive. The implemented `runtime` feature exposes the pure
shared reconstruction layer. The implemented non-default `podman-runtime` and `docker-runtime`
features add their independent native inspect decoders and enable `runtime` transitively. Enabling
one feature must not disable or change another adapter's public behavior.

The first release candidate enables `cli`, `compose`, and `quadlet` by default so
`cargo install boxferry` builds a useful command. Embedded callers can select a smaller dependency
surface with `default-features = false` and explicit format features. CI tests the default set,
no-default core, every supported individual feature, and all features.

The implemented adapter features are `compose`, `quadlet`, `runtime`, `podman-runtime`, and
`docker-runtime`; `cli` enables the argument parser and requires Compose and Quadlet for the
current executable. The facade re-exports `ComposeImporter`, `ComposeSource`, `ComposeExporter`,
`ComposeRuntime`, Compose target constants, `QuadletExporter`, `QuadletGroupingPolicy`, and
`QuadletOutput`. It also exposes each adapter and matching native
dependency through `boxferry::compose`, `boxferry::quadlet`, `boxferry::podman`, and
`boxferry::docker`, so embedded callers do not need to guess a second crate version.

The `runtime` feature re-exports `RuntimeSnapshot`, its effective-state observation types including
`RuntimeHealthcheck` and `RuntimeMetadataLabel`, plus the neutral `RestartPolicy`, `MetadataLabel`, `OverrideReconstruction`,
`RuntimeResolutions`, and `RuntimeImporter`. It does
not enable Docker or Podman clients and does not read a daemon, command, filesystem, or process
environment.

The `podman-runtime` feature re-exports `PodmanInspectDocuments`, `PodmanInspectSource`,
`PodmanSnapshotResult`, `PodmanImporter`, explicit resource-selection and command types,
`PodmanInspector`, the replaceable `PodmanCommandExecutor`, and
`ProcessPodmanCommandExecutor`. Decoding performs no I/O. The inspector runs only fixed read-only
inspect commands for caller-selected resources and a caller-supplied producing version.
`PodmanExpansionPolicy` lets callers opt into finite container-resource discovery or selected-pod
member discovery; explicit-only acquisition remains the default and the original `inspect`
contract.

The `docker-runtime` feature re-exports `DockerApiVersion`, `DockerInspectDocuments`,
`DockerInspectSource`, `DockerSnapshotResult`, `DockerImporter`, explicit resource-selection and
closed command types, `DockerInspector`, the replaceable `DockerCommandExecutor`, and
`ProcessDockerCommandExecutor`. Decoding performs no I/O and accepts only the finite reviewed
Engine API 1.40-through-1.55 range. The process inspector requires a caller-selected executable,
protected daemon endpoint, and exact API version. It uses an isolated empty client configuration,
removes ambient Docker daemon/context/custom-header/TLS selection variables, sets
`DOCKER_API_VERSION`, and never enumerates empty resource families.
`DockerExpansionPolicy::ContainerResources` follows only selected containers' image, network, and
named-volume references; explicit-only acquisition remains the default.

## Versioning and publication

Supported BoxFerry crates use a lockstep version. Workspace path dependencies also declare that
version so crates.io packages resolve without the repository checkout. Intentional public breaks
require a new pre-1.0 minor version, migration notes, and an ADR when the architecture changes.

Crates stay `publish = false` until their first useful API and compatibility contract are tested.
The [release policy](releasing.md) defines publication order; unpublished repository tools and test
utilities are not part of the public dependency surface.

## Side-effect contract

Core model and planning operations are pure. File access, environment access, runtime inspection,
native command execution, and output writes enter through explicit caller-selected adapters. An
external project can substitute in-memory implementations and can create a plan without applying
or deploying its output.

## Implemented core surface

T4 provides the first tested public surface:

- an ordered multi-service `Application` with images, commands, container restart policies, health checks, service dependencies,
  environment, protected metadata labels, explicit host mappings, ports, mounts, networks, volumes, config and secret
  declarations, ordered service grants, lifecycle ownership, and source provenance;
- tolerant `ImageReference` parsing that retains `name:tag@digest` forms;
- `ProtectedString` and structured diagnostics whose sensitive fields redact debug and display
  output;
- inclusive `PlatformVersion` and `TargetProfile` minimum/optional-maximum ranges;
- exact, approximate, unsupported, and invalid `ConversionOutcome` values;
- `LossPolicy`, validated `ConversionPlan`, and policy-authorized `ConversionResult` values;
- public import/export adapter traits, `boxferry::convert`, and an `InMemoryAdapter` for tests;
- import-side conversion outcomes that participate in the same `LossPolicy` authorization as
  target-side mapping decisions;
- an optional `compose` facade feature backed by `boxferry-compose` and ComposeLens 0.1.11;
- an optional `quadlet` facade feature backed by `boxferry-quadlet` and QuadletLens 0.1.9;
- an optional `runtime` facade feature backed by the pure `boxferry-runtime` component;
- an optional `podman-runtime` facade feature backed by `boxferry-podman`; and
- an optional `docker-runtime` facade feature backed by `boxferry-docker`.

The crates are still `publish = false`; this is a usable development API, not a crates.io release
promise. Broader native value encoders and the final default feature set remain T5/T6 work.

`HostMapping` retains an ordered hostname and raw-preserving `HostAddress`. Its conservative
classification distinguishes IPv4, bracketed or unbracketed IPv6, the runtime-specific
`host-gateway` token, and other deferred or implementation-specific values. The neutral model does
not assume that every address is an IP or that a runtime-generated alias replaces explicit source
intent.

`Service` also exposes format-independent execution identity and context: provenance-bearing
primary user/group values, a user-namespace mode, ordered supplementary groups, a working
directory, and an explicit read-only-root-filesystem choice. Text values use `ProtectedString`, so
sensitive interpolation remains redacted from debug output while authorized adapters can expose
it for native encoding.

`Service::runtime_name` is an optional provenance-bearing container name distinct from the
neutral service identifier. Source adapters preserve explicit names; runtime reconstruction uses
the inspected name; Compose and Quadlet exporters validate their respective target grammars before
emitting `container_name` or `ContainerName=`.

`RestartPolicy` is the container-level automatic restart contract. It keeps `Never`, `Always`,
unlimited or non-zero retry-limited `OnFailure`, and `UnlessStopped` distinct from service-
dependency restart propagation and deployment-orchestrator policy. Compose service `restart` and
runtime observations map into this contract; Compose generation preserves every variant exactly.
Quadlet generation keeps the separate systemd fidelity rules documented below.

`MetadataLabel` retains an opaque non-empty name and a `ProtectedString` value. Runtime adapters
construct sensitive `RuntimeMetadataLabel` values. The runtime importer can preserve them or
compare container values with linked image metadata; reserved `com.docker.compose.*` provider
labels remain reviewable but are explicitly unsafe to re-author. Compose mapping and sequence
forms import into the same neutral type with name/value provenance. The Compose exporter emits
deterministic label mappings, while the Quadlet exporter emits capability-checked repeatable
`Label=` entries with systemd quoting and literal-specifier escaping. These APIs cover service and
container metadata only; resource labels, image-build labels, annotations, and label files need
separate ownership contracts.

`Application` exposes separate config and secret resource collections with application/external
ownership, optional provider/runtime names, and optional material origins. `Service` exposes
separate ordered grant collections whose shared `ResourceGrant` retains authored short/long
syntax and separately sourced target, UID, GID, and mode values. File and environment access stay
outside the model; material and sensitive grant values retain `ProtectedString` redaction.

`Application::service_groups` exposes ordered `ServiceGroup` values. Each group has a neutral
name, lifecycle ownership, and ordered provenance-bearing service identifiers. Membership does
not imply a particular source pod type, shared namespace set, infra container, or target workload.

`ProvenanceKind` distinguishes source documents, runtime observations, user overrides,
implementation defaults, and conversion decisions. Embedded runtime adapters should attach
`RuntimeObservation` only to effective inspected state and use `ConversionDecision` for inferred
author intent.

`RuntimeImporter` requires an explicit `OverrideReconstruction` policy. `PreserveObservedState`
materializes supported effective command, environment, metadata-label, `user[:group]`, working-directory, and
regular-health-check values. `InferImageOverrides` omits values equal to linked image defaults and
retains differing values with both observation and decision provenance. Health command, disable,
interval, timeout, retries, start-period, and supported start-interval fields are compared
independently. Retained combined identities split into the neutral primary-user and primary-group
fields. Explicit read-only-root and container restart-policy state is preserved directly. Restart
policy is not compared with an image because it is a container host setting. Matching image labels
are omitted, while changed and runtime-added labels receive decision provenance. Podman startup-health checks,
on-failure actions, and log policies are not represented as regular health checks.
Both policies emit an application-level approximate outcome because neither
can recover complete author intent. Runtime network and volume ownership is `Uncertain`; pod
membership becomes an ordered `ServiceGroup` when pod and container observations agree. Group
lifecycle and target semantics remain explicit non-exact decisions. Callers can supply exact-name
`RuntimeResolutions` selecting application-owned or external lifecycle only when each choice has
`UserOverride` provenance. Resolved values retain observation and override origins and receive
`BFR0009`. `PodmanImporter::with_resolutions` and `DockerImporter::with_resolutions` forward this
same configuration into shared reconstruction. See
[Runtime reconstruction](runtime-reconstruction.md).

## Minimal embedded flow

```rust
use boxferry::{
    Application, Identifier, InMemoryAdapter, LossPolicy, PlatformVersion, TargetProfile, convert,
};

let application = Application::new(Identifier::new("example")?);
let adapter = InMemoryAdapter::exact("target document".to_owned());
let target = TargetProfile::new("podman", PlatformVersion::new(5, 4, 0), None)?;
let result = convert(
    &adapter,
    &application,
    &adapter,
    &target,
    LossPolicy::ExactOnly,
)?;

assert_eq!(result.output().map(String::as_str), Some("target document"));
# Ok::<(), Box<dyn std::error::Error>>(())
```

Real format adapters replace the in-memory adapter. They receive native parsed models explicitly;
the facade does not discover files, read environment variables, or invoke runtimes on their behalf.

The Compose importer accepts a caller-processed ComposeLens `MergedProject`. A caller-created
`ProfileSelection` is required when the project contains profiled services and must belong to that
same merged project. Each Compose source ID has a deterministic fallback identity and can be
replaced with a caller-owned path or URI through `ComposeSource::with_source_id`.

The importer consumes ComposeLens 0.1.11's native `build_project_view` boundary directly. Effective
multi-file values, including service label names and scalar-normalized values, retain every contributing source origin in BoxFerry's neutral model and
conversion outcomes; no canonical YAML render-and-reparse bridge or private BoxFerry YAML
interpretation is used.

The Compose exporter accepts a neutral `Application`, an exact `docker-compose` or
`podman-compose` provider `TargetProfile`, and an optional exact Docker Engine or Podman backend
through `ComposeRuntime`. It returns a policy-controlled
`ConversionPlan<compose_lens::render::GeneratedComposeDocument>`. ComposeLens owns deterministic
syntax selection and parse-back validation; BoxFerry owns semantic mapping, provider/runtime
compatibility outcomes, provenance, and authorization. Runtime-observed resource names are emitted
explicitly so Compose project scoping cannot rename them. See the
[Compose exporter contract](compose-adapter.md).

The Quadlet exporter accepts the neutral `Application` and an explicit Podman `TargetProfile`. It
returns a policy-controlled `ConversionPlan<QuadletOutput>` whose files and native dependency graph
have passed QuadletLens construction and parse-back validation. It reads no installed Podman
version, environment, or filesystem state. See the [Quadlet exporter contract](quadlet-adapter.md).
Relative Compose bind paths remain unsupported by default; callers opt into exact lexical
resolution with `QuadletExporter::with_relative_bind_root` and the real Compose project directory.
Host-specific forms such as tilde, Windows, or environment-derived source spellings remain losses
unless the caller supplies an exact absolute or systemd-specifier target through
`QuadletExporter::with_bind_source_mapping`. Neither API reads the host environment or filesystem.
Separate containers remain the exact grouping default. Embedded callers may select
`QuadletGroupingPolicy::SinglePod`; compatible declarations produce an approximate plan requiring
`LossPolicy::AllowApproximate`, while incompatible declarations produce no candidate. Explicit
`QuadletGroupingPolicy::PreserveSingleGroup` preserves one complete application-owned neutral
group using the group name and rejects missing, multiple, unresolved, external, or partial groups.
It remains approximate because structural membership does not itself assert shared namespaces.
Explicit host mappings, health checks, dependency/readiness directives, execution-context values,
container restart policies, explicit container names, external secret grants, and service
metadata labels convert through QuadletLens 0.1.9. `Never`
maps exactly to `Restart=no`; unbounded policies are explicit approximations and finite retry
limits remain manual actions. A single-pod request requires identical
ordered mappings and compatible user-namespace intent on every service. Common mappings and an
identical explicit namespace emit once at pod scope; separate containers retain their own values.
Compose config lifecycle and application-owned secret materialization remain explicit target-side
manual actions.
