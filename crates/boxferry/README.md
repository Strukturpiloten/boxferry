# BoxFerry

`boxferry` is the supported library facade and Linux CLI for loss-aware conversion among Compose,
Quadlet, and Podman. Windows users run it inside WSL2.

All nine routes use an importer → neutral `Application` → exporter pipeline. Podman input is
explicit read-only acquisition; Podman output is reviewable material. BoxFerry never executes
generated commands or mutates a runtime.

```console
cargo install boxferry --locked
```

Default features provide the CLI and all three adapters. Embedded callers may disable defaults and
select only the features they need. See the
[BoxFerry documentation](https://github.com/Strukturpiloten/boxferry#choose-the-next-document) for
commands, compatibility, diagnostics, and API links.
