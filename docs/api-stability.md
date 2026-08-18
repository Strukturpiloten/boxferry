# API stability

## Current status

BoxFerry 0.3.0 is the current lockstep pre-1.0 release. It intentionally adopts the released
ComposeLens and QuadletLens 0.2 native API changes. The facade and five supported
component crates use one lockstep version and the pre-1.0 contract below.

The 0.3.0 migration note records the native public reexport paths for
`compose_lens::model::ResourceExternal` and `quadlet_lens::model::SystemdUnitKey`.

The additive `compose` and `quadlet` facade features are exercised as external callers use them,
and are part of this pre-1.0 contract. Generated Quadlet file contents require
an explicit `QuadletFile::text` call and are redacted from adapter `Debug` output.

## Planned pre-1.0 contract

After publication, the `boxferry` facade and every documented public component crate follow Cargo
SemVer conventions for `0.x` releases:

- patch releases preserve supported source compatibility and behavior;
- a minor release may make an intentional breaking change and includes migration notes;
- supported crates use one lockstep version while their boundaries are evolving; and
- yanked releases are reserved for defects that make a published package unsafe or unusable.

Public enums are non-exhaustive where future format and policy variants are expected. Callers must
retain a fallback match arm. New trait methods, replacements, and removals land directly in the
next minor release with concise migration notes; BoxFerry does not retain compatibility shims by
default. The facade is the preferred compatibility boundary; component crates are supported for
callers that deliberately need lower-level model, engine, or adapter APIs.

The initial `ServiceGroup` contract guarantees ordered structural membership and provenance only.
Adding portable namespace or target-workload semantics requires new additive fields or types; it
must not silently reinterpret existing groups.

`NativeFinding` is the format-neutral provenance boundary for producer diagnostics. Its
non-exhaustive enums and private fields may grow additively; native codes do not become BoxFerry
rule codes, and source adapters must not drop retained findings when consumed or imported.

## Features and dependencies

Adapter Cargo features are additive: enabling one must not disable or reinterpret another. Default
features will be fixed before the first BoxFerry release and documented for both
`cargo install boxferry` and embedded use with `default-features = false`.

The MSRV is part of the public contract. Raising it requires a pre-1.0 minor release, release notes,
and CI evidence for the new floor. Normal development and the explicit MSRV job must both remain
green.

## Platform support

The CLI supports Linux. Windows users run the Linux executable inside WSL2; native Windows CLI
behavior is not part of the compatibility contract. macOS remains a deterministic POSIX
portability lane, not a claim of native systemd or container-runtime availability. Component
library compilation on other targets is incidental unless the platform is listed in the supported
CI matrix. See [platform support](platform-support.md).

## Intentional API changes

This is new, evolving pre-1.0 software. An intentional public API replacement or removal lands in
the next minor release without a deprecated compatibility path. Its changelog and release notes
state the direct migration. Native compatibility catalogue changes are evidence updates, not
automatically Rust API breaks.
