# BoxFerry

`boxferry` is the supported Rust library facade and Linux CLI for source-aware conversion between
Docker Compose, Docker Engine resources, Podman resources, and Podman Quadlet. Windows users can
run the CLI inside WSL2.

[Website](https://boxferry.dev/) · [User documentation](https://boxferry.dev/docs/) ·
[Rust API](https://docs.rs/boxferry) ·
[Source code](https://github.com/Strukturpiloten/boxferry)

## Install

```console
cargo install boxferry --locked
```

For library use:

```console
cargo add boxferry
```

## Conversion model

The Unreleased development CLI exposes all 16 importer → neutral `Application` → exporter routes,
including Docker-to-Docker. Docker and Podman input use explicit read-only acquisition. Docker
output is inert `docker-plan.json` plus a conditional SHA-256-bound dependency sidecar; Podman
output is reviewable material. BoxFerry never executes generated requests or commands, starts
units, or mutates a container runtime.

Default features provide the CLI and all four adapters. The `docker` feature itself remains a
library integration; it does not enable the CLI. Docker targets require an exact reviewed profile,
independently of source mode. Source reconstruction, loss policy and sensitive-value artifact
consent are separate. See the [Docker guide](../../docs/public/guides/docker-input/) and
[development contract](../../docs/decisions/0085-docker-cli-neutral-routes.md).

This is development functionality, not a claim that an installed release contains it. Compatible
published dependencies, ordinary API/package checks, native consumers and application migrations
need their own qualification.

Embedded callers may disable defaults and
select only the features they need. See the [BoxFerry documentation](https://boxferry.dev/docs/)
for conversion guides, compatibility boundaries, diagnostics, and CLI reference.

BoxFerry is open source under the
[Mozilla Public License 2.0](https://github.com/Strukturpiloten/boxferry/blob/main/LICENSE).
