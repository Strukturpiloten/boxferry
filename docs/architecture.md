# Software architecture

## Purpose

BoxFerry translates container application intent between formats and environments with explicit compatibility reporting. It is not a text-to-text YAML transformer and does not promise lossless conversion between models with different operational semantics.

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

The application model represents the concepts BoxFerry needs to reason about: workloads, images, commands, environment values, ports, storage, networks, health checks, resources, secrets, configuration, replicas, and grouping.

It is deliberately not a new container specification. It may carry source provenance and opaque source-specific data, but it must not expose native library types in its public fields.

### Conversion engine

The engine coordinates adapters, capability resolution, planning, and diagnostics. It produces a conversion plan before rendering. A plan records what is exact, adjusted, omitted by request, unsupported, or requires manual work.

### Format adapters

Adapters map between a native model and the application model. They own semantic conversion rules, not parsing syntax.

- The Compose adapter depends on ComposeLens.
- The Quadlet adapter depends on QuadletLens.
- The Kubernetes adapter depends on maintained Kubernetes API types.
- Helm and Kustomize adapters initially invoke native renderers and pass the result to the Kubernetes adapter.

### Runtime inspectors

Runtime inspectors read deployed Docker or Podman resources into observations and then map those observations to application intent. Inspection is lossy by nature: a running container does not retain every choice from its original source definition.

External commands and APIs are behind traits so tests can supply deterministic implementations.

### Target profiles and capability providers

A target profile describes the environment for which output must work. Examples include Podman and systemd version ranges, Kubernetes versions and API resources, rootless mode, and allowed fallbacks.

Each adapter owns its relevant capability provider. The engine combines capability results but does not contain a global list of platform-specific keys.

## Dependency rules

```text
compose-lens ───▶ boxferry-compose ───┐
quadlet-lens ───▶ boxferry-quadlet ───┤
k8s libraries ──▶ boxferry-kubernetes─┤──▶ boxferry CLI
runtime APIs ───▶ runtime adapters ───┘
                         ▲
              model and engine crates
```

- Lens libraries never depend on BoxFerry.
- `boxferry-model` never depends on native-format libraries.
- Adapters may depend on a native library, the model, and engine interfaces.
- The CLI composes adapters but does not implement conversion rules.

## Execution phases

1. Detect or accept the source kind.
2. Parse into a native source model.
3. Validate according to the selected source implementation profile.
4. Map into the application model while recording provenance.
5. Resolve target capabilities.
6. Build a conversion plan and diagnostics.
7. Stop before rendering when policy forbids unresolved losses.
8. Map into the native target model.
9. Validate and render the target.
10. Optionally verify output with an installed native tool.

## Safety and predictability

- Conversion is read-only unless an explicit output path is supplied.
- Inspecting a runtime must not modify it.
- Applying generated output is outside the default conversion command.
- Secret values are represented separately from ordinary text and redacted by default.
- Output is deterministic for identical input, configuration, dependency versions, and target profiles.
