# BoxFerry

`boxferry` is the supported Rust library facade and Linux CLI for loss-aware conversion between
Docker Compose, Podman resources, and Podman Quadlet. Windows users can run the CLI inside WSL2.

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

All nine routes use an importer → neutral `Application` → exporter pipeline. Podman input is
explicit read-only acquisition; Podman output is reviewable material. BoxFerry never executes
generated commands, starts units, or mutates a container runtime.

Default features provide the CLI and all three adapters. Embedded callers may disable defaults and
select only the features they need. See the [BoxFerry documentation](https://boxferry.dev/docs/)
for conversion guides, compatibility boundaries, diagnostics, and CLI reference.

BoxFerry is open source under the
[Mozilla Public License 2.0](https://github.com/Strukturpiloten/boxferry/blob/main/LICENSE).
