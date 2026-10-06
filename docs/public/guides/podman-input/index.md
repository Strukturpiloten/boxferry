# Podman input

Podman input reads one local Unix socket, never invoking `podman`. Without `--podman-socket PATH`,
the CLI checks `/run/user/<current-uid>/podman/podman.sock`, then `/run/podman/podman.sock`.
It never reads connection configuration, contacts remote hosts, or scans arbitrary paths.

Without selectors, native pod membership and container dependencies identify applications.
One application is selected automatically; multiple applications require an interactive choice
(at most 20), or an explicit selector. Complete Compose-project groups require explicit consent,
never automatic joining. Noninteractive ambiguity and empty inventories fail without selecting
anything. Custom sockets require an override; BoxFerry never starts services or escalates privileges.

For exact control, choose a selector:

- `--podman-all`;
- repeatable `--podman-resource REFERENCE` (container) or `KIND=REFERENCE`; or
- repeatable `--podman-resource-prefix KIND=PREFIX`; or
- repeatable `--podman-label NAME[=VALUE]`.

`KIND` is `container`, `image`, `network`, `pod`, `secret`, or `volume`. Exact selectors accept
native names, complete IDs, or image aliases; prefixes are literal names. Combine or repeat
selectors to add roots. Globs, regular expressions, and partial IDs are rejected.
Only `--podman-all` requests every eligible root; ambiguity and naming never imply it.

For a noninteractive Compose project migration,
`--podman-label com.docker.compose.project=PROJECT` explicitly authorizes that label selection.
Review the printed members: consistent labels remain advisory evidence.

<!-- boxferry-example: podman-input-prefix -->

```console
boxferry validate podman compose --podman-socket /run/user/1000/podman/podman.sock --podman-resource-prefix container=obs --loss-policy partial
```

`--application-name NAME` controls naming, never selection. Otherwise, BoxFerry uses an available
selected native pod/container name or explicit container/prefix; the fallback is `podman-import`.
Labels never supply output names. Set the option when names must remain stable.

Human output identifies the selected rootless/rootful endpoint, graph and stopped shared boundaries
before writing artifacts.

The default `--podman-import-policy portable` reconstructs reviewed effective settings and named
volume/network relationships. `BFP0009` identifies each reconstruction: inspection cannot distinguish
authored choices from runtime defaults. This never authorizes exporter approximations or omissions;
`--loss-policy exact` remains the default. Choose `conservative` to retain effective evidence and
enable reviewed promotion families individually. Those switches are redundant under `portable`.
JSON reports retain policy/promotion choices; human output retains field diagnostics.

Choose the output:

- [Compose output](../convert/podman-to-compose/) writes one canonical Compose document.
- [Podman output](../convert/podman-to-podman/) writes a reviewable plan and command script.
- [Quadlet output](../convert/podman-to-quadlet/) writes canonical Quadlet files.

## Production boundaries to review

- **Network borders:** discovery does not cross a named network unless
  `--podman-network-boundary NAME_OR_ID` explicitly allows it.
- **Shared volumes and networks:** the portable preset retains reviewed named relationships.
  Shared prerequisites and stopped boundaries remain external; new volumes contain no source data.
  Conservative mode requires individual named-resource promotion.
- **Bind mounts:** `--promote-podman-effective-bind-mounts` preserves absolute host sources,
  destinations, and read-only state, assuming those paths are valid on the target.
  Non-default options, propagation, and subpaths remain diagnostic findings.
- **Portable effective settings:** the portable preset enables the reviewed environment names,
  published-port, restart, normal-health, DNS, and standalone-container network-alias subset.
  `--promote-podman-portable-effective-settings` enables the same family in conservative mode.
  Values require separate `--environment-values include` authorization for acquisition and
  Compose/Quadlet artifacts. Aliases also require named-network promotion (included in `portable`);
  runtime container-ID aliases and pod-member networking remain evidence, the latter pod-scoped.
  Reports and snapshots stay redacted.
- **Network definition settings:** the portable-effective flag also promotes typed network-internal,
  subnet, gateway, and lease-range observations. Inclusive lease endpoints become the supported
  `<start-IP>-<end-IP>` `IPRange` form. An IPv6 subnet enables IPv6 output. Driver, DNS, IPAM-driver,
  interface, and standalone IPv6 fields remain visible losses without safe PodmanLens typed values.
  IPAM is network address allocation, not a separate resource.
- **SELinux relabeling:** typed configured `z` and `Z` evidence maps to shared and private
  neutral intent. Only PodmanLens decodes native mount evidence; bounded creation hints corroborate
  typed mounts. Unusable optional evidence retains the mount and reports relabel omission.
- **Secrets:** inspection cannot reconstruct secret delivery intent. BoxFerry reports incomplete
  grants instead of inventing them.
- **Creation evidence:** a matching authored-image hint corroborates configured `$.ImageName`.
  Local-ID matches/contradictions are value-free evidence, never permission to recover builds,
  pull policies, commands, environments or mounts from raw `CreateCommand`.
- **Image reference origin:** BoxFerry copies configured `$.ImageName` unchanged. Podman may already
  have normalized it; spelling never proves registry or pull provenance.
- **Local images:** `localhost/...`, IDs, unqualified names, and tagless repositories do not prove a
  portable acquisition source. Configured intent remains; `image_builds` stays empty. No Containerfile,
  build context or remote source is invented. Podman may report `PLN0048` at `source.portability`
  until the operator supplies a portable source.

Configured values are mapped directly when the neutral meaning is exact. Effective values outside
the reviewed portable preset remain evidence or a non-exact decision. Runtime-assigned ports and addresses,
static IP/MAC observations, bind paths, opaque network options, startup healthchecks, logging,
security, namespaces, and resource controls are never included by the portable-settings flag.

Start a production migration with validation and a local support bundle when the selected graph is
unexpected:

<!-- boxferry-example: podman-snapshot-error-report -->

```console
boxferry validate podman compose --podman-socket /run/user/1000/podman/podman.sock --podman-resource container=c-observer --loss-policy partial --generate-error-report --include-podman-snapshot --error-report-directory reports
```

After reviewing the diagnostics and redacted snapshots, authorize same-host bind paths only when
the target deliberately reuses them. The other portable families are already enabled:

<!-- boxferry-example: podman-portable-effective-settings -->

```console
boxferry convert podman quadlet --podman-socket /run/user/1000/podman/podman.sock --podman-resource container=c-observer --promote-podman-effective-bind-mounts --loss-policy partial --output-directory quadlet-portable-output
```

Podman deployment-v1 JSON is output intent, not an acquired inventory snapshot, and cannot be used
as Podman input.
