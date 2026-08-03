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
- Runtime adapter crates provide replaceable inspection interfaces and implementations.

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

Format and runtime features are additive and named for the integration they enable, such as
`compose`, `quadlet`, `kubernetes`, `docker-runtime`, and `podman-runtime`. Enabling one feature
must not disable or change another adapter's public behavior.

The exact default feature set will be frozen before the first BoxFerry release. It must keep
`cargo install boxferry` useful while documenting how embedded callers select a smaller dependency
surface with `default-features = false`. CI will test the default set, no-default core, every
supported individual feature, and all features once adapters exist.

The implemented adapter features are `compose` and `quadlet`. They are additive and disabled by
default while the facade remains unpublished. The facade re-exports `ComposeImporter`,
`ComposeSource`, `QuadletExporter`, `QuadletGroupingPolicy`, and `QuadletOutput`. It also exposes
each adapter and matching Lens dependency through `boxferry::compose` and `boxferry::quadlet`, so
embedded callers do not need to guess a second native-crate version.

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

- an ordered multi-service `Application` with images, commands, environment, ports, mounts,
  networks, volumes, lifecycle ownership, and source provenance;
- tolerant `ImageReference` parsing that retains `name:tag@digest` forms;
- `ProtectedString` and structured diagnostics whose sensitive fields redact debug and display
  output;
- inclusive `PlatformVersion` and `TargetProfile` minimum/optional-maximum ranges;
- exact, approximate, unsupported, and invalid `ConversionOutcome` values;
- `LossPolicy`, validated `ConversionPlan`, and policy-authorized `ConversionResult` values; and
- public import/export adapter traits, `boxferry::convert`, and an `InMemoryAdapter` for tests.
- import-side conversion outcomes that participate in the same `LossPolicy` authorization as
  target-side mapping decisions; and
- an optional `compose` facade feature backed by `boxferry-compose` and ComposeLens 0.1.1; and
- an optional `quadlet` facade feature backed by `boxferry-quadlet` and QuadletLens 0.1.1.

The crates are still `publish = false`; this is a usable development API, not a crates.io release
promise. Broader native value encoders and the final default feature set remain T5/T6 work.

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

The adapter consumes ComposeLens 0.1.1's native `build_project_view` boundary directly. Effective
multi-file values retain every contributing source origin in BoxFerry's neutral model and
conversion outcomes; no canonical YAML render-and-reparse bridge or private BoxFerry YAML
interpretation is used.

The Quadlet exporter accepts the neutral `Application` and an explicit Podman `TargetProfile`. It
returns a policy-controlled `ConversionPlan<QuadletOutput>` whose files and native dependency graph
have passed QuadletLens construction and parse-back validation. It reads no installed Podman
version, environment, or filesystem state. See the [Quadlet exporter contract](quadlet-adapter.md).
Relative Compose bind paths remain unsupported by default; callers opt into exact lexical
resolution with `QuadletExporter::with_relative_bind_root` and the real Compose project directory.
Separate containers remain the exact grouping default. Embedded callers may select
`QuadletGroupingPolicy::SinglePod`; compatible declarations produce an approximate plan requiring
`LossPolicy::AllowApproximate`, while incompatible declarations produce no candidate.
