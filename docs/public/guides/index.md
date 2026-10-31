# Conversion guides

Choose the input you have and the output you need. Every route imports through the same neutral
application model before exporting, including same-format routes. The Unreleased development
source exposes 16 routes; installed releases may expose fewer. Docker qualification boundaries
are in the [Docker guide](docker-input/).

| Input   | Compose output                                    | Docker output                                   | Podman output                                   | Quadlet output                                    |
| ------- | ------------------------------------------------- | ----------------------------------------------- | ----------------------------------------------- | ------------------------------------------------- |
| Compose | [Compose to Compose](convert/compose-to-compose/) | [Compose to Docker](convert/compose-to-docker/) | [Compose to Podman](convert/compose-to-podman/) | [Compose to Quadlet](convert/compose-to-quadlet/) |
| Docker  | [Docker to Compose](convert/docker-to-compose/)   | [Docker to Docker](convert/docker-to-docker/)   | [Docker to Podman](convert/docker-to-podman/)   | [Docker to Quadlet](convert/docker-to-quadlet/)   |
| Podman  | [Podman to Compose](convert/podman-to-compose/)   | [Podman to Docker](convert/podman-to-docker/)   | [Podman to Podman](convert/podman-to-podman/)   | [Podman to Quadlet](convert/podman-to-quadlet/)   |
| Quadlet | [Quadlet to Compose](convert/quadlet-to-compose/) | [Quadlet to Docker](convert/quadlet-to-docker/) | [Quadlet to Podman](convert/quadlet-to-podman/) | [Quadlet to Quadlet](convert/quadlet-to-quadlet/) |

- [Compose input](compose-input/) covers interpolation and file discovery.
- [Docker input and artifacts](docker-input/) covers bounded acquisition, source reconstruction,
  exact target profiles, separate consent and inert plans.
- [Podman input](podman-input/) covers discovery, read-only boundaries and promotion decisions.
- [Quadlet input](quadlet-input/) covers document sets and target-version ranges.

[Quadlet bundles](quadlet-bundles/) covers sharing complete native file sets and extracting them
without semantic conversion.

Keep the default exact loss policy for a first run. If BoxFerry blocks output, review the reported
rule before choosing `approximate` or `partial`.
