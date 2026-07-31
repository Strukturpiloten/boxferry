# BoxFerry documentation

This directory is the architectural source of truth for BoxFerry. The root README explains the project to users; these documents guide implementation and maintenance.

## Start here

- [Architecture](architecture.md) — components, dependency direction, and data flow
- [Project structure](project-structure.md) — intended Cargo workspace and ownership boundaries
- [Conversion model](conversion-model.md) — application model, conversion outcomes, and diagnostics
- [Testing strategy](testing.md) — unit, fixture, compatibility, and runtime testing
- [Dependency and license policy](dependency-policy.md) — dependency selection, allowed sources, and license checks
- [Roadmap](roadmap.md) — ordered milestones without calendar promises
- [Architecture decisions](decisions/README.md) — durable decisions and their rationale

## Documentation rules

- Describe current behavior in the present tense and planned behavior explicitly as planned.
- Keep architectural ownership in one document and link to it instead of duplicating it.
- Update affected documents in the same pull request as architectural or user-visible changes.
- Use ADRs for decisions that constrain multiple crates or repositories.
- Include source links and tested versions for external platform behavior.

Coding agents must also follow the repository-root `AGENTS.md`.
