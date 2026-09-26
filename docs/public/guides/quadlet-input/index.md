# Quadlet input

Use Quadlet input for explicitly selected files. Repeat `--input-file`, or use `--input-directory`.
For one resolved regular unit file, BoxFerry uses its filename without the extension as the neutral
application name: `web.container` becomes `web`. For two or more files, supply
`--application-name NAME`; native relationships do not prove application ownership. An explicit
name also overrides the single-file default.

For a sole `web.container`, the shortest validation command is:

<!-- boxferry-example: quadlet-inferred-name-validate -->

```console
boxferry validate quadlet compose --input-file web.container
```

If separately selected paths have the same unit basename, input resolution rejects the collision
even with an explicit application name. Select one unit or rename a colliding unit before combining
the files; a shared basename is not evidence that two files form one application.

- [Compose output](../convert/quadlet-to-compose/) reconstructs one canonical Compose document.
- [Podman output](../convert/quadlet-to-podman/) writes a reviewable plan and command script.
- [Quadlet output](../convert/quadlet-to-quadlet/) validates and writes canonical Quadlet files.

Compose interpolation options do not apply to Quadlet input. `Environment=` values are workload
environment and are withheld from generated artifacts by default. Use
`--environment-values include` explicitly for Compose or Quadlet artifacts after review.
`EnvironmentFile=` paths remain target-host dependencies and can require an
approximation when another output format cannot preserve systemd or Podman loading behavior.

Before moving files between hosts, review bind-mount paths, secret references, network unit
relationships, and the selected output version. Source Podman or systemd versions never silently
become target choices.

QuadletLens decodes quoted container/build environment assignments, command and entrypoint arguments, bracketed IPv6 `PublishPort=` values, and short/long mount syntax. BoxFerry maps only portable neutral meaning; ranges, deferred systemd values, opaque mount options, and unsupported mount types remain source-spanned review outcomes. Command and environment values stay protected in diagnostics even when authorized artifacts contain them.
