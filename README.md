# BoxFerry

BoxFerry is an open-source CLI and Rust library for source-aware conversion of container
application definitions between Docker Compose, Docker Engine resources, Podman resources, and
Podman Quadlet. Every conversion passes through a format-neutral model, so incompatible intent
remains visible as structured diagnostics instead of being silently discarded.

[Website](https://boxferry.dev/) · [Getting started](https://boxferry.dev/docs/getting-started/) ·
[crates.io](https://crates.io/crates/boxferry) · [Rust API](https://docs.rs/boxferry)

## What it converts

The Unreleased development CLI exposes these 16 routes; installed releases may expose fewer.
Docker availability still requires compatible published Lens dependencies and the normal package,
API and complete validation gates. This source integration does not qualify native consumers or
application migrations.

| Input   | Compose           | Docker            | Podman                      | Quadlet              |
| ------- | ----------------- | ----------------- | --------------------------- | -------------------- |
| Compose | Generated Compose | Inert Docker plan | Reviewable Podman artifacts | Generated unit files |
| Docker  | Generated Compose | Inert Docker plan | Reviewable Podman artifacts | Generated unit files |
| Podman  | Generated Compose | Inert Docker plan | Reviewable Podman artifacts | Generated unit files |
| Quadlet | Generated Compose | Inert Docker plan | Reviewable Podman artifacts | Generated unit files |

Every route uses importer → neutral model → exporter, including Docker-to-Docker. Docker output
requires one of four exact reviewed target presets, independently of source daemon mode. See
[Docker input and output boundaries](docs/public/guides/docker-input/) and
[ADR 0085](docs/decisions/0085-docker-cli-neutral-routes.md). Kubernetes remains deferred.

## Quick start

BoxFerry supports Linux and client-side conversion on macOS. On Windows, run the Linux CLI inside
WSL2. See [installation](docs/public/getting-started/) for release archives and platform limits.

```console
cargo install boxferry --locked
```

<!-- boxferry-example: compose-to-quadlet-explicit -->

```console
boxferry convert compose quadlet --input-file compose.yaml --output-directory quadlet-output
```

The output directory may be absent or empty. BoxFerry refuses to overwrite an existing entry.

## Safety model

- Docker and Podman input are read-only. The CLI checks only conventional local sockets unless
  the user supplies an explicit socket; embedded callers retain a caller-selected transport boundary.
- Podman output is review material, not an observed inventory or an apply operation.
- BoxFerry never executes generated commands or units. A user who runs the command script performs
  real Podman operations.
- Runtime mutation and infrastructure deployment are outside the product.
- Compose interpolation starts with `--env`, `--env-file`, or `--interpolate`; BoxFerry reads no
  implicit `.env` file.
- Secrets are redacted from diagnostics and reports by default.

## Choose the next document

- [Install and run a first conversion](docs/public/getting-started/)
- [Follow a conversion guide](docs/public/guides/)
- [Use the CLI reference](docs/public/reference/cli/)
- [Understand the model and loss policy](docs/public/concepts/)
- [Develop BoxFerry](docs/README.md)

The same pages are published at [boxferry.dev/docs](https://boxferry.dev/docs/). Displayed BoxFerry
commands are registered for offline repository tests. Development examples require execution
against the integrated source before their results can be claimed.

## Rust libraries

The `boxferry` crate is the supported facade. Component crates expose the neutral model, engine,
and individual adapters. [ComposeLens](https://github.com/Strukturpiloten/compose-lens),
[DockerLens](https://github.com/Strukturpiloten/docker-lens),
[PodmanLens](https://github.com/Strukturpiloten/podman-lens), and
[QuadletLens](https://github.com/Strukturpiloten/quadlet-lens) own their native formats.

## Open source

BoxFerry is maintained by [Martin “Becks” Beckert](https://github.com/TheRealBecks) through
[Strukturpiloten OHG](https://www.strukturpiloten.de/) and released under the
[Mozilla Public License 2.0](LICENSE). Contributions, issue reports, and practical migration
feedback are welcome.

Use `./scripts/format-lint.sh --fix` (or the VS Code task **BoxFerry: Format and lint only
(no tests)**) for quick pre-push cleanup on a smaller development computer. It deliberately
executes no tests and is not validation evidence. Run the complete development gate with
`./scripts/check-all.sh` when resources permit; protected GitHub checks remain authoritative for
every pull request.
