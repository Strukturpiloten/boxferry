# Runtime reconstruction

## Current boundary

`boxferry-runtime` is the pure common layer between future Docker/Podman inspectors and the
format-independent application graph. It performs no daemon access, command execution, file
reading, environment lookup, or native JSON parsing.

```text
Docker API/CLI ─▶ boxferry-docker ─┐
                                  ├─▶ RuntimeSnapshot ─▶ RuntimeImporter ─▶ Application
Podman API/CLI ─▶ boxferry-podman ─┘                         │
                                                            └─▶ outcomes and diagnostics
```

`boxferry-podman` and `boxferry-docker` implement independent native boundaries. Both decode
explicit caller-supplied inspect arrays and can acquire an explicit resource selection plus a
finite caller-authorized relationship closure through replaceable read-only command executors.
Podman additionally retains pod inspection and membership. Docker instead keys compatibility to
the exact Engine API response version and requires an explicit daemon endpoint.

Podman live conformance covers the official immutable 5.4-through-5.8 minor images; an opt-in
installed-current lane covers exact 6.1.0 without claiming reproducible scheduled-image evidence.
A scheduled Podman 6.1.0 image lane remains open. Docker fixtures and an isolated digest-pinned
Docker Engine 29.7.1 lane cover forced Engine API 1.40 and 1.55 responses. The common crate never
guesses a runtime from the development machine.

## Podman native decoder

`PodmanInspectDocuments` requires all five JSON arrays; use `[]` for an intentionally empty
resource family. `PodmanInspectSource` adds the caller-selected application name and exact Podman
version. The additive, non-default `podman-runtime` facade feature exposes `PodmanImporter` and
enables the shared `runtime` feature transitively.

The decoder supports the finite inclusive range 5.4.0 through 6.1.0. `BFP0003` rejects versions
outside that range. Within it:

- `BFP0001` marks malformed or invalid native data and prevents partial reconstruction;
- `BFP0002` names meaningful reusable configuration fields outside the current mapped subset;
- `BFP0004` reports relationships whose referenced resource was absent from the supplied set; and
- mapped fields become protected runtime observations before the shared reconstruction policy is
  applied.

Inspect payloads are redacted from `Debug`. Raw container, image, pod, network, and volume IDs are
used only in private lookup tables. Public `SourceId` values use stable resource kinds and names.
Effective command/environment/metadata-label/user/working-directory values, health commands, and
creation commands remain protected. Container restart policy is non-sensitive structured state.

### Read-only acquisition

`PodmanResourceSelection` contains caller-supplied names or IDs for each resource family.
`PodmanInspector` requires an explicit executor, executable path/name, and Podman version. Empty
families become `[]` without starting a process. `inspect` uses the default explicit-only policy.
`inspect_with_policy` additionally accepts one of these finite acquisition policies:

- `ExplicitOnly` follows no relationships;
- `ContainerResources` adds images, networks, and named volumes referenced by selected containers;
  and
- `PodMembersAndContainerResources` first adds selected pods' member containers, then adds those
  containers' images, networks, and named volumes.

Expansion never enumerates a resource family. It does not inspect bind-mount sources, reverse-map
a container to all pod members, or follow Podman's mixed namespace/generic container dependency
list. That list can include Podman infrastructure containers used to share IPC, mount, network,
PID, user, UTS, or cgroup namespaces, so it remains an unsupported diagnostic rather than an
acquisition instruction. Exact selectors and expanded responses are deduplicated in first-observed
order, including the case where a caller-supplied alias and a discovered runtime ID identify the
same object. Malformed JSON needed for expansion fails acquisition without exposing the response.
Selected families execute only these documented forms, with selectors after `--`:

```text
podman container inspect -- <container>...
podman image inspect -- <image>...
podman network inspect -- <network>...
podman volume inspect -- <volume>...
podman pod inspect -- <pod>...
```

`ProcessPodmanCommandExecutor` uses argument arrays, null stdin, and captured stdout/stderr; it
never invokes a shell. `PodmanCommandExecutor` lets embedded users and tests replace process
execution. Selectors, JSON stdout, and stderr are protected in debug output. The library does not
run a version command: the caller supplies the producing version so decoder policy remains
explicit. Calling the process executor requires a compatible Podman executable; building and
testing BoxFerry does not.

## Docker native decoder

`DockerInspectDocuments` requires container, image, network, and volume JSON arrays; use `[]` for
an intentionally empty resource family. `DockerInspectSource` adds a caller-selected application
name and exact two-component `DockerApiVersion`. The additive, non-default `docker-runtime` facade
feature exposes `DockerImporter` and enables the shared `runtime` feature transitively.

The decoder supports the finite inclusive Engine API range 1.40 through 1.55. It does not infer
the response version from a Docker CLI or daemon release number. Within that range:

- `BFD0001` marks malformed or invalid native data and prevents partial reconstruction;
- `BFD0002` names meaningful reusable configuration outside the current mapped subset, including
  the semantically important entrypoint/command boundary;
- `BFD0003` rejects API versions outside the reviewed fixture range;
- `BFD0004` reports referenced images, networks, or named volumes absent from the supplied set;
  and
- mapped fields become protected runtime observations before shared reconstruction policy is
  applied.

Docker's runtime-added leading slash is removed from container names. Effective container command
arguments come from top-level `Path` plus `Args`; image defaults come from `Config.Entrypoint` plus
`Config.Cmd`. Combining those values preserves observed behavior but cannot recover the original
entrypoint/command authorship boundary, so that limitation receives an explicit loss outcome. Raw
opaque IDs remain private lookup keys. For API 1.40 through 1.44, the decoder removes Docker's
generated short container ID from endpoint `Aliases`; beginning with API 1.45, `Aliases` is retained
exactly and the separate generated `DNSNames` response field is not treated as authored intent.
`HostConfig.RestartPolicy` is decoded as the reviewed container-level policy object. It is not
inferred from top-level restart history.

### Explicit endpoint acquisition

`DockerResourceSelection` holds caller-supplied names or IDs for the four supported resource
families. `DockerInspector` requires an explicit executor, Docker executable, daemon endpoint, and
Engine API version. Empty families become `[]` without execution. `inspect_with_policy` accepts:

- `ExplicitOnly`, which follows no relationships; or
- `ContainerResources`, which adds selected containers' images, attached networks, and named
  volumes.

Expansion never enumerates a family or follows bind paths. Responses and selectors are
deduplicated in first-observed order. The inspector rejects versions outside the reviewed range
before execution. `ProcessDockerCommandExecutor` uses only these fixed forms:

```text
docker --host ENDPOINT container inspect -- <container>...
docker --host ENDPOINT image inspect -- <image>...
docker --host ENDPOINT network inspect -- <network>...
docker --host ENDPOINT volume inspect -- <volume>...
```

Each process receives the exact `DOCKER_API_VERSION` and a unique empty `--config` directory.
Ambient `DOCKER_HOST`, `DOCKER_CONTEXT`, `DOCKER_CUSTOM_HEADERS`, `DOCKER_TLS`,
`DOCKER_TLS_VERIFY`, and `DOCKER_CERT_PATH` values are removed. The endpoint, selectors, JSON
stdout, and stderr are protected in debug output. The temporary client directory is removed after
the command. TLS-specific process configuration is outside this first closed executor; embedded
callers can supply a replaceable executor or direct API implementation. Building and testing
BoxFerry requires no Docker CLI or daemon.

## Observation contract

`RuntimeSnapshot` contains explicitly selected containers, images, networks, volumes, and pods.
Top-level source identities and resource names are duplicate-checked. The caller or native adapter
chooses stable `SourceId` values that are safe to display; raw opaque runtime IDs are not required.

The first reconstructed subset includes:

- effective container image references;
- effective command vectors, environment values, and metadata-label maps;
- non-empty effective `user[:group]` and working-directory values;
- effective container-level `Never`, `Always`, `OnFailure[:maximum-retries]`, or `UnlessStopped`
  restart policy;
- regular health commands, explicit disable state, interval, timeout, retries, start period, and
  a native-equivalent start interval where supported;
- explicit read-only-root-filesystem state;
- published ports and storage mounts;
- network attachments with every alias in observed order;
- inspected network and volume existence;
- container-to-image, network, volume, mount, and pod relationships; and
- optional creation-command evidence.

Effective command arguments, environment values, metadata-label values, health command arguments,
and creation arguments use `ProtectedString` and are sensitive by default. Native adapters must not place unsanitized
production inspect responses in repository fixtures.

An absent command, environment, or label collection means the native adapter did not supply that field.
An explicitly empty command uses `EffectiveCommand::Empty`; an observed empty environment uses
`set_environment(Vec::new())`, and an observed empty label map uses `set_labels(Vec::new())`. This distinction prevents missing inspection data from being
mistaken for an authored clear operation.

## Reconstruction policies

The caller must choose one policy:

- `PreserveObservedState` materializes each supported effective command, environment,
  metadata-label, `user[:group]`, working-directory, and regular-health-check value. Retained combined identities
  split into the neutral primary-user and primary-group fields with the same provenance. This is
  useful for a behavioral snapshot but can freeze image defaults into the generated definition.
- `InferImageOverrides` requires an explicit container-to-image observation link. Values equal to
  image defaults are omitted; differing command, environment, metadata-label, `user[:group]`, working-directory,
  and regular-health-check values are retained. Health command, disable, timing, and retry fields
  are classified independently. Each classification receives both runtime-observation and
  conversion-decision provenance plus an approximate outcome. Read-only-root state is a container
  setting rather than an image-default comparison and is preserved directly when supplied.

Container restart policy is also a host setting rather than an image default. Both policies retain
a supplied value directly with runtime-observation provenance.

Container and image label maps are compared by opaque name and protected value. A matching image
label is omitted; a new or changed container value is retained. A missing inherited image label is
unsupported because inspection cannot prove a portable deletion operation. Values using Compose's
reserved `com.docker.compose.*` provider namespace remain in the reviewable neutral model but
receive `BFR0010` rather than being claimed as authored application metadata. Compose and Quadlet
exporters emit retained application labels through typed Lens boundaries.
Reserved `com.docker.compose.*` labels remain report-only and are omitted from generated output.
See [ADR 0016](decisions/0016-runtime-metadata-label-reconstruction.md).

`RuntimeHealthcheck::new()` records an inspected absence of regular health configuration when it is
attached to an observation; a missing observation means the adapter did not supply the comparison
field. Docker `StartInterval` is decoded only for Engine API 1.44 and newer. Podman
`StartupHealthCheck`, health-on-failure actions, and health-log controls remain named native losses
because they are not equivalent to the shared regular-health fields.

Docker and Podman adapters accept the reviewed restart-policy inspect object and fail closed on
unknown names, negative counters, malformed objects, or retry counters attached to a
non-`on-failure` policy. Podman's empty default name and current `never` synonym with a zero
counter mean `Never`.

Quadlet uses systemd rather than the source runtime's internal restart manager. `Never` maps
exactly to `[Service] Restart=no`. `Always`, unlimited `OnFailure`, and `UnlessStopped` map only as
explicit approximations because activation gates, signal/timeout failure classification, manual
stop state, and daemon-restart behavior differ. Finite `on-failure:N` emits no restart directive:
systemd `StartLimitBurst=` is a time-window rate limit, not an equivalent retry counter. See
[ADR 0015](decisions/0015-runtime-container-restart-policy.md).

If image defaults are missing, inference preserves available effective values and reports that it
could not classify true overrides. Optional creation evidence does not change this decision.

Every runtime import produces `BFR0001` because neither policy can recover the complete authored
definition. `BFR0002` records image comparisons, `BFR0003` incomplete comparison evidence,
`BFR0004` uncertain resource ownership, `BFR0005` pod/grouping limitations, `BFR0006` a missing
reconstructable image, `BFR0007` an invalid neutral-model mapping, `BFR0008` contradictory group
membership evidence, and `BFR0009` an explicit caller lifecycle resolution.
`BFR0010` identifies retained runtime metadata in Compose's reserved provider namespace.

## Resource lifecycle and grouping

Inspection proves that a network or volume exists, but not whether a reusable definition should
create it or refer to it as external. Reconstructed resources therefore use
`ResourceOwnership::Uncertain` and block target adapters that require a lifecycle choice unless
the caller supplies a matching `RuntimeResolutions` entry. A resolution selects only
application-owned or external behavior, must carry `UserOverride` provenance, and is applied by
exact resource name. It retains the observation and override origins and receives `BFR0009`; there
are no ambient or blanket lifecycle defaults. `PodmanImporter` and `DockerImporter` forward the
same resolution set after native decoding, so callers do not need to bypass the native adapters.

Multiple network aliases and network/volume/mount relationships enter the application model.
Consistent Podman pod membership becomes an ordered neutral `ServiceGroup`. Each member retains
both pod- and container-observation provenance. The group asserts only structural co-membership:
it does not infer shared namespaces, an infra container, lifecycle ownership, or a target workload
kind. Reconstructed groups therefore use `ResourceOwnership::Uncertain` unless their exact
lifecycle is resolved by the caller, while missing or contradictory pod/member fields remain
explicit unsupported or invalid outcomes.

Quadlet output reports unresolved groups as unsupported rather than silently flattening them.
`QuadletGroupingPolicy::PreserveSingleGroup` can preserve exactly one application-owned group that
covers every application service. The group name becomes the `.pod` name, and existing topology
compatibility validation still applies. This explicitly selected structural-to-shared-namespace
mapping is approximate; zero, multiple, external, uncertain, or partial groups fail closed.
See [ADR 0011](decisions/0011-neutral-service-group-relationships.md) for the structural contract
and [ADR 0012](decisions/0012-explicit-runtime-lifecycle-resolution.md) for its resolution boundary.

The tested public vertical slices pass caller-built observations and pure Docker inspect documents
through the appropriate importer, normal conversion engine, explicit loss authorization, and a
native exporter. Quadlet slices cover ordinary container output plus explicit lifecycle resolution
and one preserved observed pod. The Compose slice generates deterministic parse-back-validated
YAML through `ComposeExporter`, retains sensitive-output redaction, and uses an exact provider plus
optional backend runtime. Runtime-observed network and volume names receive explicit Compose
resource names; unresolved lifecycle remains a visible partial loss.

## Native adapter requirements

Before a Docker or Podman inspector is marked supported, it must provide:

1. an explicit API/CLI version policy and response-version fixture;
2. replaceable I/O interfaces with no daemon dependency in unit tests;
3. sanitized container, image, network, volume, and Podman pod fixtures;
4. a structured outcome for every native field outside the shared supported subset;
5. stable redacted resource identities and sensitive-by-default values;
6. opt-in isolated integration tests for the supported runtime versions; and
7. evidence that inspection is read-only and cleanup is limited to test-owned resources.

The Podman adapter satisfies items 1 through 5 at its reviewed 5.4.0 floor and 6.1.0 ceiling. Its
digest-pinned nested-runtime harness satisfies items 6 and 7 for the available exact 5.4.0,
5.5.2, 5.6.2, 5.7.1, and 5.8.2 official images. A separately selected installed 6.1.0 executable
can run the same capture with a unique resource prefix and exact cleanup. The missing reproducible
scheduled 6.1.0 image remains an explicit evidence gap; the normal Dev Container is not privileged
and receives no host runtime socket.

The Docker adapter satisfies items 1 through 7 at its reviewed Engine API 1.40 floor and 1.55
ceiling through a digest-pinned nested Docker Engine 29.7.1 harness. The 1.40 lane verifies the
current daemon's forced compatibility response; it does not reproduce every historical Docker
19.03 implementation detail. Local `podman-docker` output does not count as Docker Engine evidence.

### Live conformance boundary

[`../tools/podman-runtime-matrix.toml`](../tools/podman-runtime-matrix.toml) separates three facts:
the 5.4.0 support floor, the 6.1.0 source-reviewed decoder ceiling, and the 5.8.2 official runtime-
image ceiling. A non-ignored contract test keeps those values aligned with the crate constants and
requires a full digest for each executable lane. The ignored live test starts only when
`BOXFERRY_CONTAINER_ENGINE` names an outer engine. An optional
`BOXFERRY_PODMAN_RUNTIME_VERSION` selects one exact lane.

`BOXFERRY_CURRENT_PODMAN` enables a second ignored test for the exact reviewed ceiling. Unlike the
nested-image tier, this test temporarily changes the selected installed runtime. It verifies the
version before mutation, uses a process-unique resource prefix, never enumerates ambient resources,
and cleans only its own image, pod, container, network, volume, and temporary directory. It is
useful current-patch evidence but does not replace a reproducible scheduled image.

The outer image creates only `boxferry-conformance-*` resources in its private nested runtime,
captures the five inspect document families, and removes itself. No production runtime response is
retained or uploaded. Weekly and manual GitHub jobs run on disposable hosted runners. See
[ADR 0008](decisions/0008-isolated-podman-runtime-conformance.md) for the privilege and supply-chain
decision.

### Docker live conformance boundary

[`../tools/docker-runtime-matrix.toml`](../tools/docker-runtime-matrix.toml) binds the reviewed API
floor and ceiling to the exact official Docker Engine 29.7.1 `dind` image and immutable digest. Its
non-ignored contract test keeps the crate constants, API list, release signal, image, and provenance
aligned. The ignored live test starts only when `BOXFERRY_CONTAINER_ENGINE` explicitly selects an
outer Docker or Podman executable.

The outer container starts a private nested daemon, imports an authored empty image, creates one
container, network, named volume, and bind mount, then captures only those resources at both API
versions. It mounts no host runtime socket, repository write path, home directory, or credential
store. A weekly/manual workflow runs the same harness on disposable hosted runners. See
[ADR 0010](decisions/0010-isolated-docker-runtime-conformance.md) for the privilege, isolation, and
historical-evidence limits.

## Official response evidence

The initial contract is based on the official
[Podman 5.4 container inspect](https://docs.podman.io/en/v5.4.2/markdown/podman-container-inspect.1.html),
[image inspect](https://docs.podman.io/en/v5.4.2/markdown/podman-image-inspect.1.html),
[pod inspect](https://docs.podman.io/en/v5.4.2/markdown/podman-pod-inspect.1.html),
[network inspect](https://docs.podman.io/en/v5.4.2/markdown/podman-network-inspect.1.html), and
[volume inspect](https://docs.podman.io/en/v5.4.2/markdown/podman-volume-inspect.1.html) contracts,
the exact
[Podman 5.4.2 container response source](https://github.com/podman-container-tools/podman/blob/v5.4.2/libpod/define/container_inspect.go),
and the exact
[Podman 6.1.0 container response source](https://github.com/podman-container-tools/podman/blob/v6.1.0/libpod/define/container_inspect.go),
plus Docker's official [container inspection](https://docs.docker.com/reference/cli/docker/container/inspect/),
[image inspection](https://docs.docker.com/reference/cli/docker/image/inspect/),
[versioned Engine API](https://docs.docker.com/reference/api/engine/),
[API version history](https://docs.docker.com/reference/api/engine/version-history/), and
[Engine 29.7.1 release notes](https://docs.docker.com/engine/release-notes/29/#2971). These pages
establish the reviewed response and command categories; reviewed fixtures and recorded live lanes
support BoxFerry's concrete compatibility claims.
