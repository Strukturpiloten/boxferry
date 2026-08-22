# Quadlet input

Read a Quadlet document set, then choose the output you need:

- [Compose output](../convert/quadlet-to-compose/) reconstructs one canonical Compose document.
- [Quadlet output](../convert/quadlet-to-quadlet/) validates and writes canonical Quadlet files.
- [Podman output](../convert/quadlet-to-podman/) writes an inert Podman deployment plan.

Repeat `--input-file` to select documents explicitly, or use `--input-directory` to discover the
set. Compose interpolation options do not apply to Quadlet input.
