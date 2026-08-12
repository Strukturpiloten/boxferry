# Software architecture

## Purpose

BoxFerry translates container application intent between formats and environments with explicit
compatibility reporting. Its conversion system is a reusable Rust library; the CLI is one
application of that library. BoxFerry is not a text-to-text YAML transformer and does not promise
lossless conversion between models with different operational semantics.

## N-to-N conversion contract

Every supported format or runtime is both a potential source and a potential target. BoxFerry does
not implement a separate converter for every pair. Instead, each integration owns two independent
boundaries:

1. an importer from the native definition or observed runtime state into the neutral application
   model; and
2. an exporter from the neutral model into a native definition or reviewable runtime deployment
   plan.

Once both boundaries exist, the conversion engine can compose that source with every target. A
route is usable only for the intersection of semantics supported by its importer, the neutral
model, and its exporter. Incompatible intent remains a structured outcome and never disappears
because another route happens to support it.

The first major product milestone covers Docker runtime state, Docker Compose, Podman runtime
state, and Podman Quadlet in every source/target combination. Kubernetes then joins the same
matrix. Applying a runtime target is a separate, explicit side effect: the pure exporter first
creates a deterministic deployment plan, and an executor may apply that plan only after caller
authorization.

This contract is fixed by [ADR 0017](decisions/0017-n-to-n-adapter-matrix.md).

## System context

```text
native source             importer                 neutral model

Docker runtime ─────────▶ Docker importer ───────┐
Compose documents ──────▶ Compose importer ──────┤
Podman runtime ─────────▶ Podman importer ───────┼──▶ application model ──▶ planner + diagnostics
Quadlet files ──────────▶ Quadlet importer ──────┤
Kubernetes resources ──▶ Kubernetes importer ───┘

neutral model             exporter                 native target

application model ──────▶ Docker exporter ───────▶ Docker deployment plan ──▶ explicit executor
                  ├─────▶ Compose exporter ──────▶ Compose documents
                  ├─────▶ Podman exporter ───────▶ Podman deployment plan ──▶ explicit executor
                  ├─────▶ Quadlet exporter ──────▶ Quadlet files
                  └─────▶ Kubernetes exporter ──▶ Kubernetes resources
```

Each native format is parsed into its own native model before mapping to BoxFerry's application model. Rendering reverses this direction: the target adapter maps application intent into a native target model, and the native library renders it.

## Major components

### Application model

The application model represents the concepts BoxFerry needs to reason about: workloads, images,
commands, environment values, protected metadata labels, explicit hostname mappings, ports,
storage, networks, health checks, container restart policies, service dependencies, execution identity and container context,
resources, secrets, configuration, replicas, and grouping.

`ServiceGroup` retains ordered structural co-membership and lifecycle ownership without implying
which Linux namespaces are shared or which target workload kind should implement it. Runtime
group members can therefore carry both group- and container-observation provenance while target
policy remains explicit.

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

In the reverse direction, `ComposeExporter` accepts a rolling provider-neutral Compose
Specification target or an exact Docker Compose or `podman-compose` provider target plus an
optional exact Docker Engine or Podman backend. It maps the neutral graph into ComposeLens's
generated values and returns deterministic, parse-back-validated YAML. Provider and runtime
identity remain separate for embedded provider-aware targets; the generic CLI uses only the
rolling specification target and does not inspect installed tools. See
[ADR 0013](decisions/0013-explicit-compose-provider-and-runtime.md) and
[ADR 0020](decisions/0020-rolling-compose-specification-cli-target.md).

The Quadlet importer consumes explicit in-memory unit inputs through `QuadletSource::parse`, or a
caller-validated named `QuadletDocumentSet`; it does not search systemd or Quadlet directories.
The sole parse boundary is `QuadletSource::parse`. Its `QuadletParseResult` preserves BoxFerry-owned,
value-free native syntax, model, and document-set diagnostic DTOs in collection order; recoverable
document-set diagnostics retain an incomplete graph. Syntax/model errors and non-recoverable
construction failures return `QuadletParseError`. Its current exact slice maps direct container images, explicit container names,
safe unquoted exec arguments, single explicit environment assignments, scalar port publications,
named or absolute-bind mounts, named network attachments, and application-owned or explicitly
external network and volume resources. Metadata labels, IPv4/bracketed-IPv6/`host-gateway` host
mappings, user and numeric group identity, supplementary groups, user namespaces, absolute
container working directories, and explicit read-only-root state use the same boundary. It rejects
duplicate singleton keys and invalid document graphs and reports all unmodeled or ambiguous native
entries. Repeated absolute-literal `EnvironmentFile=` declarations retain their order, protected
paths, and provenance without filesystem access. Their provider-parser equivalence is explicit
approximation evidence; relative and specifier-bearing paths require unavailable unit-directory
context and do not enter the neutral model. The regular native health-check subset retains protected JSON/plain commands, explicit
disable intent, validated Podman durations, and decimal retry counts with field provenance. Native
`Secret=` mount grants become references to application-level external secret resources. The
importer preserves grant order and reviewed target/UID/GID/mode options but never obtains secret
material; environment exposure and unknown native options stay unsupported. Native
`.pod` identities and sibling container `Pod=` references become application-owned structural
service groups when an explicit `PodName=` matches the unit stem. This resolution is independent
of document order and retains pod/member provenance. The group runtime separately retains a
divergent explicit runtime pod name, pod host mappings, ports, networks, user namespace, mounts,
shared memory, exit policy, stop timeout, and an unsuffixed service name; it is never assigned to
an arbitrary service. Omitted runtime pod names remain omitted. Native
section identity is retained while mapping: `[Service] Restart=no` is exact,
unbounded systemd restart policies are explicit approximations, and only complete sibling
`Requires`/`Wants` plus `After` relationships become neutral started-service dependencies.
Arbitrary host-unit references, activation without ordering, and ordering without activation stay
explicitly unsupported. Systemd quoting, shell interpretation, unit-relative and
specifier-bearing bind paths, special network modes, port ranges, and target-specific options are
never guessed.

The Quadlet exporter consumes the neutral application explicitly, evaluates the caller's Podman
version range through QuadletLens, and constructs typed native documents. It keeps services as
separate container units for the first slice, resolves generated network and volume references as
a document set, and returns structured losses for value forms it cannot encode safely. It never
reads an installed Podman version or writes unit files.

Execution identity stays container-scoped for separate units. An explicitly grouped pod moves one
identical user-namespace choice declared by every service to the capability-checked pod-level
`UserNS=` key; mixed or conflicting choices invalidate grouping.

### Runtime inspectors

Runtime inspectors read deployed Docker or Podman resources into observations and then map those
observations to application intent. Inspection is lossy by nature: a running container does not
retain every choice from its original source definition.

The implemented `boxferry-runtime` crate owns runtime-neutral observation DTOs and a pure
reconstructor. Its caller-selected policy either preserves supported effective state or compares
linked container and image observations to infer command, environment, protected metadata labels,
combined user/group, and working-directory overrides plus field-level regular-health-check differences. Retained identities
split into the neutral fields with shared provenance. Explicit read-only-root state is preserved
directly. Reserved Compose provider labels remain explicit unsafe-to-reauthor evidence. Podman startup health remains outside the shared regular-health contract. The
implemented `boxferry-podman` and `boxferry-docker` crates decode explicit native response
documents and acquire caller-selected resources through closed, replaceable command boundaries.
Finite policies may expand only directly evidenced relationships without ambient enumeration.
Docker acquisition additionally requires an explicit daemon endpoint and forced Engine API
version. Separate opt-in conformance harnesses run digest-pinned nested runtimes without host
runtime sockets. Native response types never cross into the shared runtime crate or application
model.

Neutral provenance distinguishes effective runtime observations from authored documents,
implementation defaults, caller overrides, and BoxFerry conversion decisions. Optional creation
commands may support an inference, but effective inspection data remains the primary observation.

Optional creation-command arguments are sensitive by default and can add provenance to the broad
reconstruction decision, but cannot change an effective value. Resource inspection establishes
network and volume existence and relationships, not lifecycle ownership; the neutral model records
that ownership as uncertain until an exact-name `RuntimeResolutions` entry chooses
application-owned or external behavior with user-override provenance.
Consistent Podman pod membership enters the neutral graph as a structural service group; response
disagreement is invalid and target grouping/lifecycle resolution remains caller-owned. The
Quadlet adapter can preserve one resolved application-owned group that covers the complete
application; it does not infer multi-group or partial-group topology.

External commands and APIs are behind runtime-specific replaceable interfaces so tests can supply
deterministic implementations.

### Target profiles and capability providers

A target profile describes the environment for which output must work. Examples include Podman
and systemd version ranges, the rolling Compose Specification compatibility profile, an exact
Compose provider release, Kubernetes versions and API resources, rootless mode, and allowed
fallbacks. Compose backend runtime identity is an additional explicit exporter input only for
provider-aware targets because it is distinct from the provider.

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
compose-lens ───▶ boxferry-compose ──────────────────┐
quadlet-lens ───▶ boxferry-quadlet ──────────────────┤
k8s libraries ──▶ boxferry-kubernetes ───────────────┤──▶ boxferry facade ──▶ boxferry CLI
runtime APIs ───┐                                    │
                ├──▶ Docker/Podman adapters ─────────┤
boxferry-runtime┘                                    │
model and engine crates ─────────────────────────────┘
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
