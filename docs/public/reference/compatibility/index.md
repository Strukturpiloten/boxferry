# Compatibility

The current CLI supports Linux and all nine Compose, Quadlet, and Podman routes.

| Input   | Output  | Target contract                                |
| ------- | ------- | ---------------------------------------------- |
| Compose | Compose | Rolling Compose Specification generation       |
| Compose | Quadlet | Podman 5.4.0 through 6.0.2 by default          |
| Compose | Podman  | Reviewed exact Podman target, 6.1.0 by default |
| Quadlet | Compose | Rolling Compose Specification generation       |
| Quadlet | Quadlet | Podman 5.4.0 through 6.0.2 by default          |
| Quadlet | Podman  | Reviewed exact Podman target, 6.1.0 by default |
| Podman  | Compose | Rolling Compose Specification generation       |
| Podman  | Quadlet | Podman 5.4.0 through 6.0.2 by default          |
| Podman  | Podman  | Reviewed exact Podman target, 6.1.0 by default |

Run `boxferry capabilities` for the installed build's exact route and version data.

## Target versions

Quadlet output must be valid across the complete selected Podman range. BoxFerry does not inspect
the installed Podman or systemd version. A feature introduced after the selected minimum is used
only when a reviewed fallback exists across the range.

Podman deployment output selects one exact reviewed target: 5.4.0, 5.5.0, 5.6.0, 5.7.0, 5.8.6,
6.0.0, or 6.1.0. `--podman-max-version` chooses the newest exact target no greater than its
ceiling; the default ceiling selects 6.1.0. `--podman-target-context` explicitly chooses
`rootful`, `rootless`, or `unknown`. BoxFerry never infers either choice from the development
machine.

Podman input uses the version reported by its explicitly selected read-only Libpod service.

Compose output is provider-neutral. The CLI does not ask for a Docker Compose or podman-compose
version and does not claim every historical provider accepts the document. Embedded Rust callers
can select exact provider-aware targets through the library API.

## Platform support

- Linux is supported.
- Windows users run the Linux CLI in WSL2.
- Native Windows containers and Windows path semantics are not supported.
- macOS CI checks deterministic POSIX behavior; it is not a native Quadlet runtime claim.

## Scope

Podman input is an acquired runtime inventory and resource graph. Podman output is a desired,
review-only deployment artifact and cannot be re-imported as runtime evidence. Docker and Kubernetes
remain deferred, and BoxFerry does not ship speculative adapters or any apply/deploy command.

Recognizing a native key does not guarantee every value converts exactly. When source and target
semantics differ, the diagnostic and loss policy are the compatibility contract.
