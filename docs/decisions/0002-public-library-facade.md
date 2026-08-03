# ADR 0002: public library facade and CLI parity

- Status: accepted
- Date: 2026-08-02

## Context

BoxFerry was initially described primarily as a command-line migration tool, but its neutral model,
conversion planning, structured diagnostics, and native adapters are also useful to editors,
services, graphical applications, language bindings, CI systems, and other container tooling.

Making these capabilities private to the executable would force external projects to spawn a
process and parse presentation output. Extracting a library after the CLI accumulates conversion
rules would create avoidable migration work. Conversely, publishing every workspace crate without
a supported facade would expose repository topology as the primary user experience and make later
reorganization expensive.

## Decision

1. The `boxferry` package contains both a public Rust library facade and the `boxferry` executable.
2. The facade is the recommended high-level dependency. It exposes supported model and engine
   surfaces and later exposes native adapters through documented additive features.
3. `boxferry-model` and `boxferry-engine` are independently reusable public component crates once
   T4 establishes their APIs. Format adapters may also be published when their mappings are useful
   independently.
4. Supported BoxFerry crates use one lockstep pre-1.0 version. Local workspace dependencies also
   declare that version so published packages resolve through crates.io.
5. The CLI calls the same public orchestration path available to embedded consumers. Argument
   parsing, presentation, and process exit codes remain CLI concerns; conversion rules do not.
6. Core operations make side effects explicit through interfaces. A public convenience API may
   compose stages but may not silently read files or environment variables, inspect a runtime,
   invoke a native tool, write output, or weaken loss policy.
7. Format and runtime integrations use additive Cargo features. Exact defaults are fixed before
   the first release and must balance useful `cargo install` behavior with a documented minimal
   embedded dependency set.
8. All crates remain unpublished until their useful public boundary, documentation, package
   contents, tests, and pre-1.0 compatibility policy are ready. Repository test utilities remain
   private.

## Consequences

- External Rust projects can use structured conversion results without executing or scraping the
  CLI.
- The CLI continuously exercises the public library and cannot become a separate implementation.
- Consumers can choose the facade or narrower component and adapter crates.
- Multiple published workspace crates add release ordering and semver coordination; lockstep
  versions and automated package tests make that cost explicit.
- Optional adapters require a documented feature matrix and CI coverage for supported feature
  combinations.
- The first facade API must avoid leaking native format types into the neutral model while still
  allowing callers to inspect native adapter results deliberately.

## Alternatives considered

### CLI-only application

Rejected because process invocation, presentation parsing, and CLI-global configuration are poor
integration boundaries for Rust applications and services.

### Only publish low-level component crates

Rejected because external users should not need to understand workspace topology or manually
assemble common orchestration to use BoxFerry safely.

### Separate `boxferry-cli` package

Not selected initially. Cargo supports a library and binary in one package, which keeps the
installable command and recommended dependency under the same project name. A later split remains
possible if binary-only dependencies or release cadence create a demonstrated need.

### Put all code in the `boxferry` package

Rejected because the neutral model, engine, adapters, and runtime integrations have different
dependency and testing boundaries. A facade provides one entry point without collapsing those
boundaries.
