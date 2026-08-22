# Podman input

Podman input acquires an inventory through one explicit local Unix socket. It is read-only:
BoxFerry neither discovers an ambient Podman connection nor invokes the `podman` command.

Every Podman input route requires:

- `--podman-socket PATH`;
- `--application-name NAME`;
- at least one root selector: `--podman-all`, repeatable
  `--podman-resource KIND=REFERENCE`, or repeatable `--podman-label NAME[=VALUE]`.

Add repeatable `--podman-network-boundary NAME_OR_ID` only when discovery may cross that named
network boundary. The selected inventory and discovered resource graph pass through
`PodmanImporter` into the same neutral application model used by every route.

Choose the output:

- [Compose output](../convert/podman-to-compose/) writes one canonical Compose document.
- [Quadlet output](../convert/podman-to-quadlet/) writes canonical Quadlet files.
- [Podman output](../convert/podman-to-podman/) writes an inert Podman deployment plan.

Podman deployment-v1 JSON is output intent, not an acquired inventory snapshot, and cannot be used
as Podman input.
