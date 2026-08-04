# Software architecture

## Purpose

BoxFerry translates container application intent between formats and environments with explicit
compatibility reporting. Its conversion system is a reusable Rust library; the CLI is one
application of that library. BoxFerry is not a text-to-text YAML transformer and does not promise
lossless conversion between models with different operational semantics.

## System context

```text
native source                BoxFerry                              native target

Compose document ──▶ Compose adapter ──┐                    ┌──▶ Compose document
Quadlet files ─────▶ Quadlet adapter ──┤                    ├──▶ Quadlet files
Kubernetes YAML ───▶ Kubernetes adapter├─▶ application ────┼──▶ Kubernetes YAML
Docker runtime ────▶ Docker inspector ─┤      model         └──▶ conversion report
Podman runtime ────▶ Podman inspector ─┘           │
                                                    ▼
                                          planner and diagnostics
```

Each native format is parsed into its own native model before mapping to BoxFerry's application model. Rendering reverses this direction: the target adapter maps application intent into a native target model, and the native library renders it.

## Major components

### Application model

The application model represents the concepts BoxFerry needs to reason about: workloads, images,
commands, environment values, explicit hostname mappings, ports, storage, networks, health checks,
service dependencies, execution identity and container context,
resources, secrets, configuration, replicas, and grouping.

Configuration and secret declarations retain lifecycle ownership, an optional provider/runtime
name, and an optional caller-supplied material origin. Services hold separate ordered config and
secret grant collections. A grant retains its authored short/long syntax and separately sourced
target, UID, GID, and mode values; native Lens types do not cross this boundary.

It is deliberately not a new container specification. It may carry source provenance and opaque source-specific data, but it must not expose native library types in its public fields.

### Conversion engine

The engine coordinates adapters, capability resolution, planning, and diagnostics. Source adapters
record source-to-neutral-model outcomes, and target adapters add neutral-to-target outcomes. The
combined plan therefore applies one explicit loss policy to both halves before rendering. A plan
records what is exact, adjusted, omitted by request, unsupported, or requires manual work.

### Format adapters

Adapters map between a native model and the application model. They own semantic conversion rules, not parsing syntax.

- The Compose adapter depends on ComposeLens.
- The Quadlet adapter depends on QuadletLens.
- The Kubernetes adapter depends on maintained Kubernetes API types.
- Helm and Kustomize adapters initially invoke native renderers and pass the result to the Kubernetes adapter.

The Compose adapter consumes a `MergedProject`, an optional matching ComposeLens
`ProfileSelection`, and caller-owned identities for every contributing source document. It builds
ComposeLens's native project view directly, without canonical rendering or reparsing. BoxFerry does
not infer active profiles or read the process environment.

The Quadlet exporter consumes the neutral application explicitly, evaluates the caller's Podman
version range through QuadletLens, and constructs typed native documents. It keeps services as
separate container units for the first slice, resolves generated network and volume references as
a document set, and returns structured losses for value forms it cannot encode safely. It never
reads an installed Podman version or writes unit files.

Execution identity stays container-scoped for separate units. An explicitly grouped pod cannot
retain container-level user-namespace selection because Podman uses the pod namespace; until the
typed Quadlet adapter can generate pod-level `UserNS=`, that field is a reported partial loss.

### Runtime inspectors

Runtime inspectors read deployed Docker or Podman resources into observations and then map those observations to application intent. Inspection is lossy by nature: a running container does not retain every choice from its original source definition.

Neutral provenance distinguishes effective runtime observations from authored documents,
implementation defaults, caller overrides, and BoxFerry conversion decisions. Optional creation
commands may support an inference, but effective inspection data remains the primary observation.

External commands and APIs are behind traits so tests can supply deterministic implementations.

### Target profiles and capability providers

A target profile describes the environment for which output must work. Examples include Podman and systemd version ranges, Kubernetes versions and API resources, rootless mode, and allowed fallbacks.

Each adapter owns its relevant capability provider. The engine combines capability results but does not contain a global list of platform-specific keys.

### Public facade and CLI

The `boxferry` package contains both a library facade and the `boxferry` executable. The facade
provides the supported high-level entry point and re-exports deliberately supported component
surfaces. Format and runtime adapters remain separate crates so applications can select only the
integrations they need.

The executable owns argument parsing, terminal presentation, configuration-file discovery, and
process exit codes. It calls public library orchestration APIs for every conversion. A behavior
that can only be reached through private CLI code is an architectural defect.

## Dependency rules

```text
compose-lens ───▶ boxferry-compose ────┐
quadlet-lens ───▶ boxferry-quadlet ────┤
k8s libraries ──▶ boxferry-kubernetes ─┤──▶ boxferry facade ──▶ boxferry CLI
runtime APIs ───▶ runtime adapters ────┘            ▲
                                                    │
                                       model and engine crates
```

- Lens libraries never depend on BoxFerry.
- `boxferry-model` never depends on native-format libraries.
- Adapters may depend on a native library, the model, and engine interfaces.
- The facade may expose core crates and optional adapters but does not hide their side effects.
- The CLI consumes the facade and does not implement conversion rules.

## Execution phases

1. Detect or accept the source kind.
2. Parse into a native source model.
3. Validate according to the selected source implementation profile.
4. Map into the application model while recording provenance.
5. Resolve target capabilities.
6. Map into the native target model and build a candidate conversion plan.
7. Validate and render the candidate through the owning native library.
8. Apply the explicit loss policy before releasing candidate output to the caller.
9. Optionally write the authorized files through a caller-selected interface.
10. Optionally verify output with an installed native tool.

## Safety and predictability

- Conversion is read-only unless an explicit output path is supplied.
- Inspecting a runtime must not modify it.
- Applying generated output is outside the default conversion command.
- Secret values are represented separately from ordinary text and redacted by default.
- Output is deterministic for identical input, configuration, dependency versions, and target profiles.
