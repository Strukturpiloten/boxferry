# ADR 0009: Versioned Docker inspection with an explicit daemon endpoint

- Status: superseded
- Date: 2026-08-04
- Superseded by: [ADR 0032](0032-future-native-lens-boundaries.md)

## Context

Docker CLI and daemon release numbers do not by themselves define the shape returned by an
inspection. The versioned Docker Engine API does. Client and daemon versions may differ, the CLI
normally negotiates an API version, and `DOCKER_API_VERSION` can force an exact supported response
contract. The API uses an open response schema, so additive fields must be tolerated while
meaningful unmapped configuration remains visible.

The Docker CLI also otherwise selects a daemon through ambient `DOCKER_HOST`, contexts, or local
defaults. That behavior is convenient interactively but unsuitable for a reusable migration
library whose source and provenance must be explicit.

## Decision

Create a separate unpublished `boxferry-docker` crate and additive `docker-runtime` facade feature.
The public decoder accepts explicit container, image, network, and volume inspect arrays plus an
exact two-component `DockerApiVersion`. Its finite reviewed range begins at API 1.40 and ends at
API 1.55. Versions outside that range fail closed.

The process acquisition boundary requires an explicit Docker executable, daemon endpoint, and API
version. Every fixed inspect command uses a unique empty `--config` directory, includes
`--host ENDPOINT`, sets `DOCKER_API_VERSION`, removes ambient Docker host/context/custom-header/TLS
selection variables, and places selectors after `--`. Empty families run no command. TLS-specific
CLI configuration is outside the first process executor; callers can provide a replaceable
executor or API implementation.

Explicit-only acquisition remains the default. A separate finite policy may add the image,
attached networks, and named volumes referenced directly by selected containers. It never lists a
resource family, follows bind paths, or discovers unrelated containers.

Container effective commands come from Docker's observed `Path` plus `Args`. Image defaults come
from `Config.Entrypoint` plus `Config.Cmd`. The boundary between entrypoint and command remains a
named unsupported reconstruction concern until the neutral model represents both concepts.
Docker container names lose the one runtime-added leading slash when used as neutral service
names. Raw IDs stay private relationship keys. API versions before 1.45 injected a short
container ID into endpoint `Aliases`, so the decoder removes that one generated value there. API
1.45 and newer retain `Aliases` exactly and ignore the separate generated `DNSNames` field as
runtime lookup state rather than authored alias intent.

## Consequences

- Compatibility claims follow the response protocol actually decoded.
- A caller cannot accidentally label a negotiated current response as an older API response when
  using the standard process executor.
- The explicit endpoint makes daemon selection reviewable and keeps it out of debug output.
- API 1.40 covers the current supported floor and Docker 19.03-era engines; API 1.55 covers the
  reviewed current Docker Engine 29.7.1.
- Open-schema additions do not break parsing, but reusable non-empty fields outside the mapped
  subset produce structured diagnostics.
- Docker remains independent from Podman's superficially similar JSON and version policy.

## Evidence

Docker documents API negotiation, `DOCKER_API_VERSION`, the supported 1.40 floor, and the current
1.55 API in the [Engine API reference](https://docs.docker.com/reference/api/engine/). The
[version history](https://docs.docker.com/reference/api/engine/version-history/) records response
changes and explicitly describes the API as versioned. The
[container-inspect CLI](https://docs.docker.com/reference/cli/docker/container/inspect/) documents
the fixed read-only command used by the process boundary. The general
[Docker CLI reference](https://docs.docker.com/reference/cli/docker/) documents ambient client
configuration, custom headers, and daemon-selection variables. Docker Engine
[29.7.1 release notes](https://docs.docker.com/engine/release-notes/29/#2971) establish the current
reviewed release.

## Alternatives

Using only the CLI release number was rejected because the daemon and negotiated API may differ.
Allowing ambient context selection was rejected because identical library inputs could inspect a
different daemon. Reusing Podman's native decoder was rejected because field meaning, pod support,
API versioning, command behavior, and response evolution differ. Calling the daemon socket
directly remains a future replaceable executor option; it is not required for the first pure
decoder and closed CLI boundary.
