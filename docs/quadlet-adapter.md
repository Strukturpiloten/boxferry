# Quadlet exporter

The `boxferry-quadlet` crate maps BoxFerry's neutral application model into a deterministic set of
Quadlet files through `quadlet-lens` 0.1.7. The `boxferry` facade exposes it through the additive
`quadlet` feature.

## Planning boundary

`QuadletExporter` implements `ExportAdapter`. It accepts an explicit `TargetProfile` whose
implementation is `podman`, evaluates every used native capability through QuadletLens, builds
typed documents, and returns a `ConversionPlan<QuadletOutput>`.

The adapter reads no files, environment variables, or installed Podman version. It writes no
files and does not invoke Podman or systemd. `QuadletOutput` exposes the generated files and the
parse-back-validated `QuadletDocumentSet`; callers decide where authorized output is written.

Generated output can contain sensitive environment values. `Debug` output for `QuadletFile` and
`QuadletOutput` never prints file contents. Calling `QuadletFile::text` is the explicit access
boundary for deployable text.

## First supported target subset

The exporter generates one `.container` file per service by default, so published ports and
network attachments remain owned by their declaring service. It never infers pod grouping.

Callers can explicitly select `QuadletGroupingPolicy::SinglePod`. The adapter accepts that request
only when every service has the same ordered network attachments, host mappings, and user-
namespace intent, no per-service aliases, and no overlapping declared container or published host
port/protocol pair. It then generates one application-named `.pod`, moves a shared explicit user
namespace, common host mappings, networks, and published ports to `[Pod]`, and links every
container through `Pod=application.pod`. Mixed implicit/explicit or conflicting user namespaces
invalidate grouping. The document-set graph validates all resulting native references.

Even a compatible pod request is an approximation because the target services share a network
namespace that Compose normally keeps separate. It emits `BFQ0007`, retains every contributing
service origin, and requires `LossPolicy::AllowApproximate`. An incompatible explicit request is
invalid and produces no candidate; the adapter does not silently fall back to separate containers.

Callers reconstructing an existing pod can instead select
`QuadletGroupingPolicy::PreserveSingleGroup`. This requires exactly one application-owned neutral
group containing every application service. It generates the group-named `.pod` and links each
member to it after applying the same topology checks. Missing, multiple, uncertain, external, or
partial groups are invalid. Preservation still emits `BFQ0007` as an approximation because a
neutral structural group alone does not assert Podman shared-namespace behavior.

It currently maps:

- image references, including `name:tag@digest` spellings;
- exec-form commands whose individual arguments need no systemd quoting;
- literal environment assignments whose names and values need no systemd quoting or specifier
  escaping;
- protected service metadata through repeatable `Label=` entries, including empty values,
  systemd-quoted whitespace/control/quote characters, and doubled literal `%` specifiers;
- health checks using JSON-preserved `CMD` or `CMD-SHELL` commands, explicit disable intent,
  regular interval, timeout, retries, and start period;
- required and optional service-start dependencies through `[Unit]` `Requires`/`Wants` plus
  `After`, including health-gated activation through `Notify=healthy` when target readiness can be
  established from an explicit encodable health command;
- primary user and numeric primary GID, user namespace, ordered named or numeric supplementary
  groups, working directory, and explicit read-only-root choice through capability-checked
  container keys;
- explicit host mappings with IPv4, bracketed or unbracketed IPv6, or the `host-gateway` token;
- single published TCP, UDP, and SCTP ports, with an optional IPv4 host address;
- application-owned named volumes through generated `.volume` files;
- external named volumes through direct runtime-name references;
- anonymous mounts, absolute bind sources, and systemd-specifier sources such as `%h/data`;
- `./` and `../` bind sources when the caller configures the exact absolute Compose project root;
- source-machine-specific bind forms such as `~/data` or `C:\data` when the caller provides an
  exact target mapping;
- read-only and `z`/`Z` SELinux mount options;
- application-owned networks through generated `.network` files;
- external networks through direct runtime-name references; and
- pre-existing external Podman secrets through repeatable `Secret=` entries, preserving Compose's
  default target filename when a custom runtime name differs and mapping safe target, numeric UID,
  numeric GID, and read-only octal mode options.

Application-owned resource references are resolved through QuadletLens's document-set graph.
External resources deliberately produce no lifecycle file.

## Explicit losses and limits

The adapter currently reports rather than guesses:

- host-resolved or explicitly unset environment variables;
- deferred or implementation-specific host-mapping addresses;
- shell-form, empty, or quoting-dependent commands;
- quoting-dependent environment values;
- relative bind sources when the caller does not provide their Compose project root;
- tilde, non-POSIX, and other host-specific bind sources without an exact caller-provided mapping;
- IPv6 or otherwise non-simple host-address port spellings;
- container-only exposed ports without host publication;
- implicit resource lifecycles;
- per-network aliases;
- identifiers requiring systemd unit-name escaping;
- Compose `start_interval`, which has no native Quadlet key and is not silently mapped to Podman's
  semantically different startup-healthcheck feature family;
- health commands containing unresolved systemd percent specifiers;
- Compose-controlled dependency restart propagation and successful-completion or provider-specific
  dependency conditions;
- optional dependencies whose service is absent from the converted application;
- named primary groups, because Quadlet's native `Group=` contract requires a numeric GID;
- execution-context values that require systemd quoting or target-specific validation;
- Compose config resources and grants, because Quadlet has no managed config-resource lifecycle;
- application-owned secret creation and file/environment materialization, because `Secret=` only
  consumes a Podman secret that already exists; and
- secret names or options outside the reviewed unambiguous one-line Podman secret grammar;
- reserved `com.docker.compose.*` metadata plus resource, build/image, annotation, and label-file
  label ownership that is not equivalent to service/container metadata; and
- neutral service groups without a supported caller-selected target grouping and lifecycle
  resolution.

Missing required dependency services, dependency-ordering cycles, missing secret declarations,
and incompatible explicit grouping requests are invalid. Optional absent services and unsupported
conditions or resource materialization retain the remaining candidate but require
`LossPolicy::AllowPartial`. Dependency directives are generated between container service units
in both separate-container and explicitly selected single-pod layouts; pod membership does not
erase service startup order.

Unsupported fields remain in the conversion report and require `LossPolicy::AllowPartial` before
the remaining candidate can be released. Invalid required values and target profiles always block
output.

`QuadletExporter::with_relative_bind_root` supplies Compose project context explicitly. Resolution
is lexical, does not require the path to exist, and never reads the filesystem. It produces an
absolute Quadlet bind source and rejects roots or traversals that would escape the filesystem root.

`QuadletExporter::with_bind_source_mapping` is the separate policy boundary for host-specific
forms. It matches the authored source spelling exactly and requires a safely encodable absolute
POSIX or systemd-specifier target. For example, a caller may assert `~/data` maps to `%h/data` or a
Windows source maps to a deployment-specific absolute Linux path. Mappings take precedence over
project-root resolution. Empty, unsafe, or conflicting mappings fail when the exporter is
configured. BoxFerry never assumes that `~`, `%h`, Windows paths, and target-host paths are
interchangeable.

## Version evidence

The exporter supports explicit Podman ranges inside QuadletLens's finite verified catalogue,
currently 5.4.0 through 6.0.2. A range below or above that evidence fails closed. When
`podmanMaximumVersion` is omitted, planning evaluates through the catalogue ceiling and emits a
note that later releases remain an assumption.

## Diagnostics

| Code      | Severity      | Meaning                                                                  |
| --------- | ------------- | ------------------------------------------------------------------------ |
| `BFQ0001` | error         | Target implementation or Podman range cannot be planned safely.          |
| `BFQ0002` | note          | Maximum is omitted; the report names the finite verified ceiling.        |
| `BFQ0003` | warning       | Neutral intent is unsupported by the current target mapping.             |
| `BFQ0004` | error         | A required value cannot be emitted safely as native Quadlet.             |
| `BFQ0005` | error         | QuadletLens rejected a generated document or document set.               |
| `BFQ0006` | warning/note  | A required capability is unavailable or deprecated for the target range. |
| `BFQ0007` | warning/error | Explicit pod grouping is approximate or incompatible with source intent. |
| `BFQ0008` | warning/error | Dependency semantics are partial, missing, cyclic, or otherwise unsafe.  |

Every warning/error fidelity decision carries the contributing neutral-model provenance. Sensitive
values are not copied into diagnostic fields.
