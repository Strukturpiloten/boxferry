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

## Podman input

Input compatibility and output targets are separate contracts. The finite input catalogue is:

| Podman line | Accepted versions                                                         |
| ----------- | ------------------------------------------------------------------------- |
| 3.0         | 3.0.1                                                                     |
| 3.4         | 3.4.4                                                                     |
| 4.3         | 4.3.1                                                                     |
| 4.9         | 4.9.3 and 4.9.4                                                           |
| 5.4–5.8     | The reviewed minor line, starting at 5.4.0, 5.5.0, 5.6.0, 5.7.0, or 5.8.0 |
| 6.0–6.1     | The reviewed minor line, starting at 6.0.0 or 6.1.0                       |

The selected read-only Libpod service reports its Podman and API versions. BoxFerry fails closed
outside this catalogue. Use `boxferry capabilities --verbose` for precise inclusive and exclusive
bounds in the installed build. Legacy input is migration evidence, not a claim that BoxFerry can
generate output for that old version.

## Output target versions

Quadlet output must be valid across the complete selected Podman range. BoxFerry does not inspect
the installed Podman or systemd version. A feature introduced after the selected minimum is used
only when a reviewed fallback exists across the range.

Podman deployment output selects one exact reviewed target: 5.4.0, 5.5.0, 5.6.0, 5.7.0, 5.8.6,
6.0.0, or 6.1.0. The default is 6.1.0. A Podman 3.x or 4.x source can therefore migrate to
Compose, Quadlet, or a reviewable modern Podman plan.

`--podman-max-version` chooses the newest exact target no greater than its ceiling.
`--podman-target-context` explicitly chooses `rootful`, `rootless`, or `unknown`. BoxFerry never
infers the output context from the source or development machine.

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
