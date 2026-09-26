# Podman input

Podman input acquires an inventory through one local Unix socket. It is read-only: BoxFerry never
invokes the `podman` command. Unless `--podman-socket PATH` is supplied, the CLI checks only these
local service sockets, in this order: `/run/user/<current-uid>/podman/podman.sock`, then
`/run/podman/podman.sock`. It never reads Podman connection configuration, contacts a remote host,
or scans arbitrary paths.

With no selector, BoxFerry reads the local inventory and identifies applications from native pod
membership and container dependencies. One application is selected automatically. If there are
several, an interactive terminal offers at most 20 numbered choices, including a complete
Compose-project group only with explicit consent; noninteractive use fails with an actionable error
and selects nothing. An empty inventory also fails. Complete Compose ownership labels are advisory
and never join applications for an automatic choice. A custom or remote socket
must be supplied explicitly; BoxFerry never starts a service or escalates privileges.

For exact control, choose a selector:

- `--podman-all`;
- repeatable `--podman-resource REFERENCE` (container) or `KIND=REFERENCE`; or
- repeatable `--podman-resource-prefix KIND=PREFIX`; or
- repeatable `--podman-label NAME[=VALUE]`.

`KIND` is one of `container`, `image`, `network`, `pod`, `secret`, or `volume`. An exact selector
accepts one native name, complete ID, or image alias. A prefix selector accepts one literal name
prefix. Selector forms may be combined, and repeatable forms may be supplied more than once to add
roots. Globs, regular expressions, and partial IDs are rejected. `--podman-all` is the only way to
request every eligible root; neither an ambiguous choice nor `--application-name` implies it.

For a noninteractive Compose project migration,
`--podman-label com.docker.compose.project=PROJECT` explicitly authorizes that label selection.
Review the printed members: consistent labels remain advisory evidence.

<!-- boxferry-example: podman-input-prefix -->

```console
boxferry validate podman compose --podman-socket /run/user/1000/podman/podman.sock --podman-resource-prefix container=obs --loss-policy partial
```

`--application-name NAME` controls output naming only, never application selection. Without it,
BoxFerry uses the selected native pod or container name when available. An explicitly selected
container or prefix may also provide a neutral name. Label values never supply output names;
otherwise the name is `podman-import`. Set `--application-name` when output names must remain stable.

For a conventional local service, omit both `--podman-socket` and `--application-name`. Human output
shows the selected rootless or rootful endpoint, selected graph, and stopped shared boundaries before
artifacts are written. Keep the socket override for nonstandard paths; the CLI never probes remote
connection configuration.

Add repeatable `--podman-network-boundary NAME_OR_ID` only when discovery may cross that named
network boundary. The selected inventory and discovered resource graph pass through
`PodmanImporter` into the same neutral application model used by every route.

Podman input uses `--podman-import-policy portable` by default. It reconstructs reviewed
effective published ports, restart and normal health behavior, named-volume mounts, and
named-network relationships without three separate promotion switches. A `BFP0009` note
identifies each reviewed reconstruction and states that inspection cannot distinguish an
original authored choice from a runtime default. It does not authorize a target exporter to
approximate behavior or omit unsupported intent: `--loss-policy exact` remains the default.
Select `--podman-import-policy conservative` to keep effective values as evidence, then enable
only the particular promotion families you have reviewed. Expert promotion switches are
redundant under the portable preset. JSON reports record the selected policy and effective
promotion choices; human output retains the field diagnostics.

Choose the output:

- [Compose output](../convert/podman-to-compose/) writes one canonical Compose document.
- [Podman output](../convert/podman-to-podman/) writes a reviewable plan and command script.
- [Quadlet output](../convert/podman-to-quadlet/) writes canonical Quadlet files.

## Production boundaries to review

- **Network borders:** discovery does not cross a named network unless
  `--podman-network-boundary NAME_OR_ID` explicitly allows it.
- **Shared volumes and networks:** the portable preset retains reviewed named relationships.
  Shared prerequisites and stopped shared boundaries remain external. A new target volume does
  not contain the source data. In conservative mode, the named-resource promotion switches
  authorize their families individually.
- **Bind mounts:** `--promote-podman-effective-bind-mounts` preserves absolute host sources,
  container destinations, and read-only state. It explicitly assumes those paths are valid on
  the target. Non-default native options, propagation, and subpaths remain diagnostic findings.
- **Portable effective settings:** the portable preset enables the reviewed environment names,
  published-port, restart, normal-health, DNS, and standalone-container network-alias subset.
  `--promote-podman-portable-effective-settings` enables the same family in conservative mode.
  It does not acquire environment values. Values stay withheld
  unless `--environment-values include` separately authorizes their acquisition and inclusion in
  Compose or Quadlet artifacts. Alias promotion also requires named-network promotion (already
  included by the portable preset); runtime container-ID aliases remain evidence only,
  and pod-member networking remains pod-scoped evidence. Reports and snapshots stay redacted.
- **Network definition settings:** the portable-effective flag also promotes typed network-internal,
  subnet, gateway, and lease-range observations. Inclusive lease endpoints become the supported
  `<start-IP>-<end-IP>` `IPRange` form. An IPv6 subnet enables IPv6 output. Driver, DNS, IPAM-driver,
  interface, and standalone IPv6 fields remain visible losses because PodmanLens does not yet expose
  safe typed values for them. Here, IPAM means the address-allocation subsection of a network
  definition; it is not a separate network resource.
- **SELinux relabeling:** typed configured `z` and `Z` evidence maps to shared and private
  neutral relabel intent. BoxFerry does not parse `HostConfig.Binds` or `Mounts[].Mode`; bounded
  PodmanLens creation evidence may only corroborate the typed mount. Missing or unusable optional
  evidence retains the parent mount and reports relabel omission.
- **Secrets:** inspection cannot reconstruct secret delivery intent. BoxFerry reports incomplete
  grants instead of inventing them.
- **Creation evidence:** a matching authored-image hint corroborates configured `$.ImageName`.
  A local-ID match or contradiction is value-free actionable evidence, never permission to recover
  a build, pull policy, command, environment, or mount from raw `CreateCommand`.
- **Image reference origin:** BoxFerry copies configured `$.ImageName` unchanged. Podman may already
  have normalized an initially unqualified name. That spelling does not prove registry or pull
  provenance.
- **Local images:** `localhost/...`, IDs, unqualified names, and tagless repositories do not prove a
  portable Podman acquisition source. BoxFerry keeps configured image intent and leaves
  `image_builds` empty; it never invents a Containerfile, build context, or remote source. Podman
  output may report `PLN0048` against `source.portability` until the operator supplies a portable
  source.

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
