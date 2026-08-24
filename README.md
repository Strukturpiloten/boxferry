# BoxFerry

BoxFerry converts container application definitions through one neutral model. It keeps
incompatible intent visible as structured diagnostics and writes output only when the selected
loss policy allows it.

## What it converts

| Input   | Compose           | Quadlet              | Podman                |
| ------- | ----------------- | -------------------- | --------------------- |
| Compose | Generated Compose | Generated unit files | Inert deployment plan |
| Quadlet | Generated Compose | Generated unit files | Inert deployment plan |
| Podman  | Generated Compose | Generated unit files | Inert deployment plan |

All nine routes use the same importer → neutral model → exporter pipeline. Same-format conversion
is never a native passthrough. Docker and Kubernetes are not implemented.

## Quick start

BoxFerry supports Linux. On Windows, run the Linux CLI inside WSL2.

```console
cargo install boxferry --locked
boxferry convert compose quadlet --input-file compose.yaml --output-directory quadlet-output
```

The output directory may be absent or empty. BoxFerry refuses to overwrite an existing entry.

## Safety model

- Podman input uses an explicit, caller-selected, read-only connection.
- Podman output is review material, not an observed inventory or an apply operation.
- Generated commands and units are never executed.
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

Run the complete development gate with `./scripts/check-all.sh`. BoxFerry is maintained by
[Martin “Becks” Beckert](https://github.com/TheRealBecks) through
[Strukturpiloten OHG](https://www.strukturpiloten.de/) under the
[Mozilla Public License 2.0](LICENSE).
