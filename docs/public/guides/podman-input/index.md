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
- [Podman output](../convert/podman-to-podman/) writes a reviewable plan and command script.
- [Quadlet output](../convert/podman-to-quadlet/) writes canonical Quadlet files.

## Production boundaries to review

- **Network borders:** discovery does not cross a named network unless
  `--podman-network-boundary NAME_OR_ID` explicitly allows it.
- **Shared volumes:** effective named volumes remain runtime evidence unless
  `--promote-podman-effective-named-volumes` authorizes portable desired state.
- **Shared networks:** use `--promote-podman-effective-named-networks` only after confirming the
  target should recreate that network intent.
- **Bind mounts:** host paths are machine-local. Review every local-resolution diagnostic before
  accepting output for a different host.
- **Secrets:** inspection cannot reconstruct secret delivery intent. BoxFerry reports incomplete
  grants instead of inventing them.

Podman deployment-v1 JSON is output intent, not an acquired inventory snapshot, and cannot be used
as Podman input.
