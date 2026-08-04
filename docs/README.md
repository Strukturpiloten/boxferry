# BoxFerry documentation

This directory is the architectural source of truth for BoxFerry. The root README explains the project to users; these documents guide implementation and maintenance.

## Start here

- [Architecture](architecture.md) — components, dependency direction, and data flow
- [Project structure](project-structure.md) — intended Cargo workspace and ownership boundaries
- [Library API and publication policy](library-api.md) — facade, component crates, features, and CLI parity
- [Command-line interface](cli.md) — supported commands, safety behavior, and exit status
- [API stability](api-stability.md) — unpublished and planned pre-1.0 compatibility contracts
- [Conversion model](conversion-model.md) — application model, conversion outcomes, and diagnostics
- [Format coverage](format-coverage.md) — field-by-field pipeline coverage and promotion rules
- [Compose exporter](compose-adapter.md) — generated fields, provider/runtime selection, and explicit limits
- [Quadlet exporter](quadlet-adapter.md) — supported mappings, version evidence, and explicit limits
- [Runtime reconstruction](runtime-reconstruction.md) — observation contract, inference policy, Docker/Podman decoding, finite acquisition, and live evidence
- [Testing strategy](testing.md) — unit, fixture, compatibility, and runtime testing
- [Development environment](development-environment.md) — reproducible VS Code tooling and update policy
- [Release policy](releasing.md) — library publication order and future binary automation boundary
- [Fixture format](fixture-format.md) — shared metadata, provenance, and secrets contract
- [Podlet and compose_spec_rs issue-corpus review](research/podlet-compose-spec-rs-issues-2026-08-01.md) — user scenarios, regressions, and repository ownership
- [Dependency and license policy](dependency-policy.md) — dependency selection, allowed sources, and license checks
- [Implementation plan](implementation-plan.md) — synchronized cross-repository tasks T1–T7
- [Roadmap](roadmap.md) — ordered milestones without calendar promises
- [Architecture decisions](decisions/README.md) — durable decisions and their rationale

## Documentation rules

- Describe current behavior in the present tense and planned behavior explicitly as planned.
- Keep architectural ownership in one document and link to it instead of duplicating it.
- Update affected documents in the same pull request as architectural or user-visible changes.
- Use ADRs for decisions that constrain multiple crates or repositories.
- Include source links and tested versions for external platform behavior.

Coding agents must also follow the repository-root `AGENTS.md`.
