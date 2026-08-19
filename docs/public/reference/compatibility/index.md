# Compatibility

The current CLI supports Linux and four document routes.

| Input   | Output  | Target contract                                |
| ------- | ------- | ---------------------------------------------- |
| Compose | Compose | Rolling Compose Specification canonicalization |
| Compose | Quadlet | Podman 5.4.0 through 6.0.2 by default          |
| Quadlet | Compose | Rolling Compose Specification generation       |
| Quadlet | Quadlet | Podman 5.4.0 through 6.0.2 by default          |

Run `boxferry capabilities` for the installed build's exact route and version data.

## Target versions

Quadlet output must be valid across the complete selected Podman range. BoxFerry does not inspect
the installed Podman or systemd version. A feature introduced after the selected minimum is used
only when a reviewed fallback exists across the range.

Compose output is provider-neutral. The CLI does not ask for a Docker Compose or podman-compose
version and does not claim every historical provider accepts the document. Embedded Rust callers
can select exact provider-aware targets through the library API.

## Platform support

- Linux is supported.
- Windows users run the Linux CLI in WSL2.
- Native Windows containers and Windows path semantics are not supported.
- macOS CI checks deterministic POSIX behavior; it is not a native Quadlet runtime claim.

## Scope

Docker runtime, Podman runtime, and Kubernetes routes are not available. They require future
independent DockerLens, PodmanLens, and KubernetesLens projects. BoxFerry does not ship speculative
runtime adapters.

Recognizing a native key does not guarantee every value converts exactly. When source and target
semantics differ, the diagnostic and loss policy are the compatibility contract.
