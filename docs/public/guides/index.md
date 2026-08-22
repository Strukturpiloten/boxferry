# Conversion guides

Choose the input row and output column.

| Input   | Compose output                                    | Quadlet output                                    | Podman output                                   |
| ------- | ------------------------------------------------- | ------------------------------------------------- | ----------------------------------------------- |
| Compose | [Compose to Compose](convert/compose-to-compose/) | [Compose to Quadlet](convert/compose-to-quadlet/) | [Compose to Podman](convert/compose-to-podman/) |
| Quadlet | [Quadlet to Compose](convert/quadlet-to-compose/) | [Quadlet to Quadlet](convert/quadlet-to-quadlet/) | [Quadlet to Podman](convert/quadlet-to-podman/) |
| Podman  | [Podman to Compose](convert/podman-to-compose/)   | [Podman to Quadlet](convert/podman-to-quadlet/)   | [Podman to Podman](convert/podman-to-podman/)   |

Compose input can be interpolated explicitly. Quadlet input is a document set. Podman input uses
explicit read-only acquisition and selectors.
