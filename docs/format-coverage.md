# Format coverage

This document is the end-to-end coverage source of truth for BoxFerry. Native parsing coverage
belongs to ComposeLens and QuadletLens; this matrix records whether a value can travel through the
complete conversion pipeline.

The [real-world Compose corpus](real-world-compose-corpus.md) turns source-side field boundaries
into pinned application-level tests and an ordered compatibility backlog. The separate
[QuadletLens real-world corpus](https://github.com/Strukturpiloten/quadlet-lens/blob/main/docs/real-world-quadlet-corpus.md)
provides target-format pressure from public Quadlet deployments. Passing that corpus proves
loss-aware Quadlet ingestion; it does not mean BoxFerry can yet import Quadlet or generate every
typed and untyped native key found there.

Coverage was audited against the current official
[Compose service reference](https://docs.docker.com/reference/compose-file/services/) and the
[Podman Quadlet manual](https://docs.podman.io/en/latest/markdown/podman-systemd.unit.5.html) on
2026-08-05. A documentation entry is not compatibility evidence by itself. Version claims still
require the repository tests described in [Testing strategy](testing.md).

## Route availability

The product contract is N-to-N. This table records adapter availability; it is not a list of
independent pairwise converters. Any implemented importer can feed any implemented exporter, but
the resulting route supports only their shared semantic subset.

| Boundary | Import to neutral model | Export from neutral model | CLI orchestration |
| --- | --- | --- | --- |
| Docker runtime | Inspect decoder and reconstruction library implemented for the reviewed API range | Deployment planning and execution open | Open |
| Docker Compose | Implemented for the documented subset | Implemented for the documented subset | Compose-to-Quadlet only |
| Podman runtime | Inspect decoder and reconstruction library implemented for Podman 5.4.0–6.0.2 | Deployment planning and execution open | Open |
| Podman Quadlet | Shared service/resource subset implemented, including labels, host mappings, and execution context; broader native surface open | Implemented for the documented subset | Compose-to-Quadlet only |
| Kubernetes | Open | Open | Open |

Runtime export means generating and validating a reviewable create/update plan before an optional
explicit apply operation. It never means mutating an ambient daemon merely because conversion was
requested.

## Status vocabulary

| Status | Meaning |
| --- | --- |
| `end to end` | ComposeLens exposes the effective merged value, BoxFerry represents it, and QuadletLens can emit the supported subset. |
| `partial` | The pipeline exists, but some documented value forms or semantics produce explicit non-exact outcomes. |
| `native only` | A Lens has a typed native value, but the value is not yet available to BoxFerry through the complete pipeline. |
| `preserved` | Source syntax and provenance survive, but no typed conversion contract exists yet. |
| `not applicable` | The concept is source processing or target-specific and is not emitted as a corresponding target field. |

`Preserved` never means ignored: BoxFerry's Compose importer reports every unmodeled effective
field as a policy-controlled unsupported outcome.

## Compose to Quadlet pipeline

| Intent | Compose fields | Current status | Current boundary |
| --- | --- | --- | --- |
| Image | `image` | `end to end` | Tolerant tags plus digests are retained; unsafe one-line target spellings fail explicitly. |
| Runtime container name | `container_name` | `end to end` | The name remains distinct from the service key and emits as capability-checked `ContainerName=`. Compose and Podman target grammars are validated independently; invalid values block output. |
| Command | `command` | `partial` | Safe exec-form arguments emit `Exec=`; shell form, clearing, and values needing target quoting are reported. |
| Environment | `environment` | `partial` | Literal safe values emit `Environment=`; host lookup, unset intent, and values needing target encoding are reported. |
| Environment files | `env_file` | `partial` | ComposeLens 0.1.13 imports and exports ordered short/long declarations with `required`, `format`, sensitivity, and provenance without reading files. Required safe absolute paths and relative paths resolved from an explicit project root emit repeatable capability-checked Quadlet `EnvironmentFile=` entries. Quadlet output and Quadlet-to-Compose reconstruction are approximate until Compose-default and `raw` parser parity with Podman is proven. Optional files, systemd-specifier ambiguity, and unsafe paths are reported instead of guessed. |
| Host mappings | `extra_hosts` | `end to end` | Sequence/mapping syntax, IPv4, IPv6, and `host-gateway` reach container or compatible pod `AddHost=` entries. |
| Published ports | `ports` | `partial` | Single numeric publications are supported; ranges, deferred values, unsupported host-address forms, and target-only options are reported. |
| Storage | `volumes` plus top-level `volumes` | `partial` | Named, bind, and anonymous mounts cover the first slice, including short-syntax SELinux relabel intent; other mount types/options are reported. |
| Networking | `networks` plus top-level `networks` | `partial` | Named attachments and ownership are represented; aliases and advanced attachment/definition options are reported. |
| Project interpolation | `${NAME}` and supported operators | `not applicable` | The CLI leaves interpolation disabled by default. `--interpolate` evaluates per-file overlays from an empty environment plus repeatable plain `--variable NAME=VALUE` and individually authorized sensitive `--variable-from-environment NAME` inputs. No implicit process or `.env` lookup occurs. Embedded callers use the same explicit ComposeLens overlay before constructing `ComposeSource`. |
| Profile selection | `profiles` | `not applicable` | The caller must explicitly select profiles before import; profiles do not become Quadlet fields. |
| Health checks | `healthcheck` | `partial` | `CMD`, `CMD-SHELL`, `NONE`/`disable`, interval, timeout, retries, and start period are source-aware and version-checked end to end. Compose `start_interval`, deferred/invalid scalars, conflicting disable/command intent, and systemd-percent-bearing commands produce explicit non-exact outcomes. |
| Dependencies | `depends_on` | `partial` | Ordered required/optional `service_started` edges map to `Requires`/`Wants` plus `After`. `service_healthy` also maps through `Notify=healthy` when the target has an explicit encodable health command. Restart propagation, successful completion, provider-specific conditions, absent optional services, missing required services, and cycles produce explicit unsupported or invalid outcomes. |
| Identity | `user`, `userns_mode`, `group_add` | `partial` | User, numeric primary group, user namespace, and ordered named or numeric supplementary groups retain provenance and sensitivity and emit capability-checked keys. Identical explicit namespaces on every grouped service move to pod `UserNS`; mixed or conflicting intent invalidates grouping. Named primary groups and unsafe values are explicit losses. |
| Released container settings | `hostname`, `pids_limit`, `shm_size`, `cap_add`, `cap_drop`, `tmpfs`, `sysctls`, `ulimits`, `devices`, `stop_signal` | `partial` | The ten released fields reach raw/protected neutral values with ordered entries and source provenance, then capability-checked Quadlet keys. Only the documented safe resolved forms emit; deferred, malformed, provider-specific, CDI/opaque, incomplete, empty-reset, or namespace-sensitive forms remain explicit losses. Other exporters retain a partial outcome rather than discarding them. |
| Build | `build`, `pull_policy` | `native only` / `preserved` | ComposeLens retains field-level build intent; `.build` generation and image/build policy remain open. |
| Config and secret grants | `configs`, `secrets` | `partial` / `end to end` | ComposeLens 0.1.11 imports ownership, runtime names, file/environment/inline material origins, and ordered short/long grants with per-option provenance and redaction. Pre-existing external Podman secrets emit repeatable Quadlet `Secret=` entries with preserved target defaults and validated target/UID/GID/read-only-mode options. Application-owned secret materialization and every Compose config lifecycle/grant remain explicit manual actions because Quadlet has no equivalent managed config resource. |
| Lifecycle | `restart`, `stop_grace_period`, `stop_signal`, `init`, lifecycle hooks | `partial` | Safe resolved `stop_signal` values map end to end. Authored Compose and runtime inspection both retain container `Never`, `Always`, `OnFailure`, finite positive retry limits, and `UnlessStopped`; Compose output is exact for every neutral variant. Quadlet emits exact `Restart=no`, explicit approximations for unbounded policies, and no unsafe substitute for finite retry limits. Unresolved, zero, and out-of-range authored retry limits are invalid. Other lifecycle fields remain preserved only. |
| Process | `entrypoint`, `working_dir`, `tty`, `stdin_open`, `read_only` | `partial` / `preserved` | Safely encodable working directories and explicit true/false read-only-root intent convert end to end. Values needing systemd encoding are reported. Entrypoint, TTY, and standard-input behavior remain preserved only. |
| Hostname and DNS | `hostname`, `domainname`, `dns`, `dns_opt`, `dns_search` | `partial` / `preserved` | Safe resolved `hostname` values reach Quadlet `HostName=` when UTS mode is not retained. Ordered non-empty DNS servers, options, and search domains map through the neutral model and Compose/Quadlet native keys; explicit empties, resolver-special `none`/`.`, deferred or multiline values, and generated Pod resolver sharing remain structured non-exact outcomes. Runtime container names do not imply container hostnames. |
| Security and devices | capabilities, devices, CDI/GPU, namespaces, privileged, security options, sysctls | `partial` / `preserved` | Reviewed safe capabilities, host devices, and sysctls reach typed Quadlet keys. CDI/GPU, namespaces, privileged mode, security options, and unsafe or context-dependent forms remain explicit losses. |
| Metadata and logging | annotations, labels, label files, logging | `end to end` for service labels / `partial` otherwise | Compose mapping and sequence labels import with value-scalar normalization and multi-file provenance. Protected neutral service labels generate deterministic Compose mappings and native repeatable Quadlet `Label=` entries; empty, quoted, control, and literal systemd-specifier values are encoded. Runtime container/image maps support image-default comparison. Reserved Compose-managed labels remain reviewable but are never re-authored. Annotations, label files, image-build labels, resource labels, and logging remain open. |
| Compose orchestration extensions | `extends`, `develop`, `provider`, `models`, links, scaling | `preserved` | These have no direct first-slice Quadlet equivalent and require separate processing or explicit diagnostics. |

The detailed native boundaries are maintained in the
[ComposeLens typed-model documentation](https://github.com/Strukturpiloten/compose-lens/blob/main/docs/typed-model.md)
and
[QuadletLens typed-model documentation](https://github.com/Strukturpiloten/quadlet-lens/blob/main/docs/typed-model.md).

## Quadlet-only surface

BoxFerry currently emits `.container`, `.pod`, `.network`, and `.volume` files. QuadletLens keeps
unknown native entries and generic systemd sections loss-aware, but that does not authorize
BoxFerry to synthesize them. `.image`, `.build`, `.kube`, and experimental `.artifact` generation
are open work. Native Podman-only keys are added when they support a defined migration scenario,
not merely because the key exists.

The released `HostName=`, `PidsLimit=`, `ShmSize=`, `DropCapability=`, `AddCapability=`,
`Tmpfs=`, `Sysctl=`, `Ulimit=`, `AddDevice=`, and `StopSignal=` keys are export-only in this
slice. The Quadlet importer names each as an unmapped `BFQ1003` outcome rather than silently
dropping it until its inverse source contract is defined.

The Quadlet importer reads typed `.container`, `.network`, and `.volume` documents. Its exact
subset covers direct `Image=`, `ContainerName=`, safe unquoted `Exec=`, one safe explicit
`NAME=VALUE` per `Environment=`, scalar `PublishPort=` declarations, anonymous/named/absolute-bind
`Volume=` declarations with `ro`, `rw`, `z`, and `Z`, and named `Network=` attachments. Referenced
`.network` and `.volume` units become application-owned resources; safe literal names become
explicit external resources. Environment values are protected in memory and diagnostics never
contain their authored contents. Single safe `Label=` assignments, repeatable `AddHost=` entries
with IPv4, bracketed IPv6, or `host-gateway`, user and numeric group identity, supplementary groups,
user namespace mode, absolute container working directory, and explicit read-only-root state are
also represented with provenance. One `AddHost=` entry containing semicolon-separated hostnames is
expanded into equivalent ordered neutral mappings. Section-aware generic-systemd handling maps
`Restart=no` exactly and retains `always` or `on-failure` only with an explicit approximation
outcome. Complete `Requires=`/`Wants=` plus `After=` pairs referencing sibling `.container` or
generated `.service` units become ordered required/optional `service_started` edges. Multiple unit
names per directive and repeated equivalent directives retain their contributing provenance.
Regular `HealthCmd=`, `HealthInterval=`, `HealthTimeout=`, `HealthRetries=`, and
`HealthStartPeriod=` fields enter the neutral health model. `none` disables a check; JSON arrays
and conservative plain commands are protected so diagnostics and debug output do not expose them.
Repeated absolute-literal `EnvironmentFile=` paths enter the ordered neutral declaration list
without reading the referenced files. They carry explicit approximation outcomes because Podman
and Compose parser parity is not proven. Relative, unit-relative, systemd-specifier-bearing, and
native-quoted paths stay unsupported until the caller supplies safe source-location context.
Repeatable `Secret=` values using the default or explicit `type=mount` form become ordered grants
to external neutral secret resources. Source, target, UID, GID, mode, syntax form, and provenance
are retained; no secret material is read or synthesized. `type=env`, unknown options, duplicate
options, unsafe names/targets, invalid decimal IDs, and invalid modes remain explicit outcomes.
Owned `.pod` documents with explicit `PodName=` equal to the unit stem and container
`Pod=<name>.pod` references become ordered, provenance-aware neutral service groups. Resolution is
independent of source document order. Duplicate container `Pod=` declarations are invalid.

Systemd quoting, shell interpretation, continued values, relative or specifier-bearing bind
sources, IPv6 publications, port ranges, special network modes, attachment options, and unmodeled
mount options produce explicit unsupported outcomes rather than guessed neutral values. Arbitrary
host-unit dependencies and incomplete activation/ordering pairs are unsupported; conflicting
`Requires=` and `Wants=` declarations for one sibling are invalid. Duplicate singleton keys,
invalid booleans, invalid health durations/retry counts, and `Group=` without a valid `User=`
produce invalid outcomes. `HealthInterval=disable` remains unsupported because disabling the
automatic timer is not equivalent to disabling the health check.
Omitted `PodName=` values use Podman's `systemd-`-prefixed runtime default, which cannot coexist
with the unit identity in the current single-name service-group model. Divergent explicit pod
names and pod-scoped `AddHost=`, `Network=`, `PublishPort=`, `UserNS=`, and `Volume=` settings also
remain explicit outcomes; BoxFerry does not assign shared pod state to an arbitrary service.
Invalid or unresolved document-set references block import. Typed `.image`, `.build`, `.kube`, and
`.artifact` input also remains open in QuadletLens and BoxFerry.

## Runtime reconstruction foundation

The additive `runtime` feature accepts caller-constructed, runtime-neutral observations. The
non-default `podman-runtime` feature additionally decodes explicit container, image, network,
volume, and pod inspect arrays for Podman 5.4.0 through the reviewed 6.0.2 ceiling. A replaceable
executor can acquire explicitly selected resources through fixed read-only Podman commands and a
finite policy may add selected pod members plus referenced container resources. The non-default
`docker-runtime` feature independently decodes Engine API 1.40-through-1.55 container, image,
network, and volume inspect arrays. Its replaceable command boundary requires an explicit daemon
endpoint and API version; a finite policy may add selected containers' referenced resources. No
policy enumerates an ambient resource family. The supported effective subset is image reference,
command, environment, non-empty `user[:group]`, non-empty working directory, explicit read-only-root
state, explicit runtime container name, container restart policy, protected effective metadata labels, regular health command/disable/timing/retry configuration, ports, mounts, network
relationships and ordered aliases, inspected
network/volume existence, optional creation-command evidence, and Podman pod membership evidence.

Command, environment, metadata-label, `user[:group]`, working-directory, and regular-health-check values can be
preserved or compared with a linked image observation. Health fields are compared independently;
commands remain protected. A retained identity is split into neutral primary user and group fields
with shared provenance. Read-only-root state is preserved directly.
Matching image label defaults are omitted; changed, added, and reserved Compose-provider values
remain explicit reviewable outcomes. Restart policy is also preserved directly because it is a container host setting, not an image
default. Runtime-to-Quadlet maps `Never` exactly and retains explicit approximate or unsupported
outcomes for the other policy forms.
Every comparison is approximate and receives conversion-decision provenance. Network and volume
lifecycle ownership remains uncertain unless an exact-name caller resolution selects application-
owned or external behavior with user-override provenance. Consistent Podman pod membership becomes
an ordered neutral `ServiceGroup` with pod and container provenance, but no inferred namespace or
lifecycle semantics. Quadlet output reports unresolved groups rather than flattening them and can
preserve one resolved complete application group as a named pod after explicit approximation
authorization. Entrypoint, remaining runtime policy and security settings, annotations and
resource labels, Podman startup-
health configuration, and other inspect fields remain for later native adapter milestones. The
Podman decoder reports each meaningful unmodeled native field by
name as an unsupported outcome instead of discarding it. The Docker decoder applies the same rule
with independent `BFD` diagnostics and additionally reports the lost entrypoint/command boundary.

The Compose export path covers the same supported effective subset for images, commands,
environment, explicit runtime names, service labels, identity/context, container restart policies,
ports, mounts, networks, volumes, and ordered environment-file declarations. It uses ComposeLens 0.1.13's
deterministic generated-document boundary and requires an exact Docker Compose or
`podman-compose` provider version; the optional exact Docker Engine or Podman backend remains a
separate input. Short/long `env_file` syntax, order, explicit `required`/`raw` options, and path
sensitivity survive generated output and parse-back validation.
Runtime-observed resource names are explicit. Provider/runtime-sensitive behavior,
unresolved lifecycle, structural pod grouping, and fields outside the generated subset remain
policy-controlled non-exact outcomes rather than silent output changes.

## Promotion rule

A field moves to `end to end` only when all of these are present:

1. a source-aware native type;
2. an effective merged-project view where the source format supports merging;
3. a format-independent BoxFerry representation;
4. explicit source and target mapping outcomes;
5. target-version capability evidence;
6. unit, adapter, and golden end-to-end tests; and
7. documentation of unsupported forms and semantic differences.

This rule is the guardrail added after `extra_hosts`: a single parser type or target key is not a
completed conversion feature.
