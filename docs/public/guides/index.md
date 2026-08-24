# Conversion guides

Choose the input you have and the output you need. Every route imports through the same neutral
application model before exporting, including same-format routes.

| Input   | Compose output                                    | Podman output                                   | Quadlet output                                    |
| ------- | ------------------------------------------------- | ----------------------------------------------- | ------------------------------------------------- |
| Compose | [Compose to Compose](convert/compose-to-compose/) | [Compose to Podman](convert/compose-to-podman/) | [Compose to Quadlet](convert/compose-to-quadlet/) |
| Podman  | [Podman to Compose](convert/podman-to-compose/)   | [Podman to Podman](convert/podman-to-podman/)   | [Podman to Quadlet](convert/podman-to-quadlet/)   |
| Quadlet | [Quadlet to Compose](convert/quadlet-to-compose/) | [Quadlet to Podman](convert/quadlet-to-podman/) | [Quadlet to Quadlet](convert/quadlet-to-quadlet/) |

## Start from your input

- [Compose input](compose-input/) covers explicit interpolation, environment files, and overrides.
- [Podman input](podman-input/) covers read-only sockets, selectors, network boundaries, and
  promotion decisions.
- [Quadlet input](quadlet-input/) covers document sets and target-version ranges.

Keep the default exact loss policy for a first run. If BoxFerry blocks output, review the reported
rule before choosing `approximate` or `partial`.
