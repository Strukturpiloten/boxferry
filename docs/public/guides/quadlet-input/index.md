# Quadlet input

Use Quadlet input for an explicitly selected document set. Repeat `--input-file`, or use
`--input-directory`; always provide `--application-name` because the files do not define a Compose
project identity.

- [Compose output](../convert/quadlet-to-compose/) reconstructs one canonical Compose document.
- [Podman output](../convert/quadlet-to-podman/) writes a reviewable plan and command script.
- [Quadlet output](../convert/quadlet-to-quadlet/) validates and writes canonical Quadlet files.

Compose interpolation options do not apply to Quadlet input. `Environment=` values are workload
environment. `EnvironmentFile=` paths remain target-host dependencies and can require an
approximation when another output format cannot preserve systemd or Podman loading behavior.

Before moving files between hosts, review bind-mount paths, secret references, network unit
relationships, and the selected output version. Source Podman or systemd versions never silently
become target choices.
