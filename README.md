# BoxFerry

BoxFerry is a loss-aware migration and conversion tool for container application definitions.

The project is intended to help people move applications between Docker Compose, Podman Quadlet, and Kubernetes without pretending that these environments are perfectly equivalent. BoxFerry will preserve intent where possible and report every approximation, unsupported feature, and required manual action.

> [!IMPORTANT]
> BoxFerry is in its initial design phase. It does not yet provide a usable command-line application or a stable API.

## Goals

- Convert supported application definitions in both directions where the semantics allow it.
- Import existing Docker and Podman resources through runtime inspection.
- Produce actionable compatibility and loss reports instead of silently dropping configuration.
- Account for target versions, including Podman and Kubernetes feature differences.
- Keep format parsing in focused libraries rather than embedding every format in the application.
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

## Documentation

Start with the [documentation index](docs/README.md). Important design documents include:

- [Software architecture](docs/architecture.md)
- [Target project structure](docs/project-structure.md)
- [Conversion model and diagnostics](docs/conversion-model.md)
- [Testing strategy](docs/testing.md)
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
