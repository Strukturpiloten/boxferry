# Library API and stability

Use the `boxferry` facade for normal embedding. Direct component crates are supported when an
application needs a smaller or lower-level boundary.

## Features

| Feature   | Adds                                            |
| --------- | ----------------------------------------------- |
| `compose` | ComposeLens types and Compose semantic adapters |
| `quadlet` | QuadletLens types and Quadlet semantic adapters |
| `cli`     | Clap, reports, ZIP support, and the executable  |

Default features enable the useful CLI. Embedded applications can disable defaults.

## Public flow

1. Build or parse a native source explicitly.
2. Import it through `ImportAdapter`.
3. Select `TargetProfile` and `LossPolicy`.
4. Export through `ExportAdapter` or call `boxferry::convert`.
5. Inspect `ConversionResult` and diagnostics before consuming the inert output.

Core planning is pure. File, environment, and read-only native acquisition stay in explicit
caller-selected boundaries. Applying or deploying output is outside BoxFerry.

## Supported crates

`boxferry`, `boxferry-model`, `boxferry-engine`, `boxferry-compose`, and `boxferry-quadlet` publish
in lockstep.

## Pre-1.0 stability

The current release is pre-1.0:

- patch releases preserve documented source compatibility;
- minor releases may replace or remove APIs with concise migration notes;
- compatibility shims are not retained by default;
- MSRV changes require a minor release and release notes;
- native diagnostic codes remain provenance, while BoxFerry rule codes remain the policy contract.

The facade is the preferred compatibility boundary. CLI-only conversion behavior that an embedded
caller cannot obtain is an architecture defect.

Build the public API documentation with:

```console
RUSTDOCFLAGS="-D warnings" cargo ci-doc
```
