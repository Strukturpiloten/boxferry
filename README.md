# BoxFerry

BoxFerry is an open-source CLI and Rust library for source-aware conversion of container
application definitions between Docker Compose, Podman resources, and Podman Quadlet. Every
conversion passes through a format-neutral model, so incompatible intent remains visible as
structured diagnostics instead of being silently discarded.

[Website](https://boxferry.dev/) · [Getting started](https://boxferry.dev/docs/getting-started/) ·
[crates.io](https://crates.io/crates/boxferry) · [Rust API](https://docs.rs/boxferry)

## What it converts

| Input   | Compose           | Podman                      | Quadlet              |
| ------- | ----------------- | --------------------------- | -------------------- |
| Compose | Generated Compose | Reviewable Podman artifacts | Generated unit files |
| Podman  | Generated Compose | Reviewable Podman artifacts | Generated unit files |
| Quadlet | Generated Compose | Reviewable Podman artifacts | Generated unit files |

All nine routes use the same importer → neutral model → exporter pipeline. Same-format conversion
is never a native passthrough. Docker Engine resources and Kubernetes are not yet implemented.

## Quick start

BoxFerry supports Linux. On Windows, run the Linux CLI inside WSL2.

```console
cargo install boxferry --locked
boxferry convert compose quadlet --input-file compose.yaml --output-directory quadlet-output
```

The output directory may be absent or empty. BoxFerry refuses to overwrite an existing entry.

## Safety model

- Podman input is read-only. The CLI checks only conventional local sockets unless the user
  supplies an explicit socket; embedded callers retain a caller-selected transport boundary.
- Podman output is review material, not an observed inventory or an apply operation.
- BoxFerry never executes generated commands or units. A user who runs the command script performs
  real Podman operations.
- Runtime mutation and infrastructure deployment are outside the product.
- Environment interpolation is opt-in; BoxFerry reads no implicit `.env` file.
- Secrets are redacted from diagnostics and reports by default.

## Choose the next document

- [Install and run a first conversion](docs/public/getting-started/)
- [Follow a conversion guide](docs/public/guides/)
- [Use the CLI reference](docs/public/reference/cli/)
- [Understand the model and loss policy](docs/public/concepts/)
- [Develop BoxFerry](docs/README.md)

The same pages are published at [boxferry.dev/docs](https://boxferry.dev/docs/). Every displayed
BoxFerry command is executed by repository tests.

## Rust libraries

The `boxferry` crate is the supported facade. Component crates expose the neutral model, engine,
and individual adapters. [ComposeLens](https://github.com/Strukturpiloten/compose-lens),
[PodmanLens](https://github.com/Strukturpiloten/podman-lens), and
[QuadletLens](https://github.com/Strukturpiloten/quadlet-lens) own their native formats.

## Open source

BoxFerry is maintained by [Martin “Becks” Beckert](https://github.com/TheRealBecks) through
[Strukturpiloten OHG](https://www.strukturpiloten.de/) and released under the
[Mozilla Public License 2.0](LICENSE). Contributions, issue reports, and practical migration
feedback are welcome.

Run the complete development gate with `./scripts/check-all.sh` before submitting a change.
