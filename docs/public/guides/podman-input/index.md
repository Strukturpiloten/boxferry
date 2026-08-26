# Podman input

Podman input acquires an inventory through one local Unix socket. It is read-only: BoxFerry never
invokes the `podman` command. Unless `--podman-socket PATH` is supplied, the CLI checks only these
local service sockets, in this order: `/run/user/<current-uid>/podman/podman.sock`, then
`/run/podman/podman.sock`. It never reads Podman connection configuration, contacts a remote host,
or scans arbitrary paths.

Every Podman input route requires at least one root selector:

- `--podman-all`;
- repeatable `--podman-resource KIND=REFERENCE`; or
- repeatable `--podman-resource-prefix KIND=PREFIX`; or
- repeatable `--podman-label NAME[=VALUE]`.

`KIND` is one of `container`, `image`, `network`, `pod`, `secret`, or `volume`. An exact selector
accepts one native name, complete ID, or image alias. A prefix selector accepts one literal name
prefix. Selector forms may be combined, and repeatable forms may be supplied more than once to add
roots. Globs, regular expressions, and partial IDs are rejected.

<!-- boxferry-example: podman-input-prefix -->

```console
boxferry validate podman compose --podman-socket /run/user/1000/podman/podman.sock --podman-resource-prefix container=obs --loss-policy partial
```

`--application-name NAME` is optional. Without it, BoxFerry uses the only non-ID exact resource
name, the only prefix, or the value from one exact `NAME=VALUE` label selector. Ambiguous selectors,
full IDs, name-only labels, and `--podman-all` use `podman-import`. Set an explicit name when output
names must remain stable.

For a conventional local service, omit both `--podman-socket` and `--application-name`. Keep the
socket override for nonstandard paths and remote integrations supplied by an embedding caller.

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
- **Local images:** `localhost/...`, image IDs, unqualified names, and tagless repositories are not
  portable Podman acquisition sources. Podman output fails with `PLN0048` and names the affected
  image and `source.portability` field. Push the image and recreate the source container with a
  registry-qualified tag, or convert to Compose/Quadlet and replace the reference there. BoxFerry
  does not invent a remote image source.

Podman deployment-v1 JSON is output intent, not an acquired inventory snapshot, and cannot be used
as Podman input.
