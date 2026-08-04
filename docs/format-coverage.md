# Format coverage

This document is the end-to-end coverage source of truth for BoxFerry. Native parsing coverage
belongs to ComposeLens and QuadletLens; this matrix records whether a value can travel through the
complete conversion pipeline.

Coverage was audited against the current official
[Compose service reference](https://docs.docker.com/reference/compose-file/services/) and the
[Podman Quadlet manual](https://docs.podman.io/en/latest/markdown/podman-systemd.unit.5.html) on
2026-08-03. A documentation entry is not compatibility evidence by itself. Version claims still
require the repository tests described in [Testing strategy](testing.md).

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
| Command | `command` | `partial` | Safe exec-form arguments emit `Exec=`; shell form, clearing, and values needing target quoting are reported. |
| Environment | `environment` | `partial` | Literal safe values emit `Environment=`; host lookup, unset intent, and values needing target encoding are reported. |
| Host mappings | `extra_hosts` | `end to end` | Sequence/mapping syntax, IPv4, IPv6, and `host-gateway` reach container or compatible pod `AddHost=` entries. |
| Published ports | `ports` | `partial` | Single numeric publications are supported; ranges, deferred values, unsupported host-address forms, and target-only options are reported. |
| Storage | `volumes` plus top-level `volumes` | `partial` | Named, bind, and anonymous mounts cover the first slice, including short-syntax SELinux relabel intent; other mount types/options are reported. |
| Networking | `networks` plus top-level `networks` | `partial` | Named attachments and ownership are represented; aliases and advanced attachment/definition options are reported. |
| Profile selection | `profiles` | `not applicable` | The caller must explicitly select profiles before import; profiles do not become Quadlet fields. |
| Health checks | `healthcheck` | `partial` | `CMD`, `CMD-SHELL`, `NONE`/`disable`, interval, timeout, retries, and start period are source-aware and version-checked end to end. Compose `start_interval`, deferred/invalid scalars, conflicting disable/command intent, and systemd-percent-bearing commands produce explicit non-exact outcomes. |
| Dependencies | `depends_on` | `partial` | Ordered required/optional `service_started` edges map to `Requires`/`Wants` plus `After`. `service_healthy` also maps through `Notify=healthy` when the target has an explicit encodable health command. Restart propagation, successful completion, provider-specific conditions, absent optional services, missing required services, and cycles produce explicit unsupported or invalid outcomes. |
| Identity | `user`, `userns_mode`, `group_add` | `partial` | User, numeric primary group, user namespace, and ordered named or numeric supplementary groups retain provenance and sensitivity and emit capability-checked keys. Identical explicit namespaces on every grouped service move to pod `UserNS`; mixed or conflicting intent invalidates grouping. Named primary groups and unsafe values are explicit losses. |
| Limits and deployment | `ulimits`, CPU/memory/PID fields, `deploy` | `native only` / `preserved` | ComposeLens types `ulimits` and field-level `deploy`; BoxFerry does not yet map resource policy. |
| Build | `build`, `pull_policy` | `native only` / `preserved` | ComposeLens retains field-level build intent; `.build` generation and image/build policy remain open. |
| Config and secret grants | `configs`, `secrets` | `partial` / `end to end` | ComposeLens 0.1.6 imports ownership, runtime names, file/environment/inline material origins, and ordered short/long grants with per-option provenance and redaction. Pre-existing external Podman secrets emit repeatable Quadlet `Secret=` entries with preserved target defaults and validated target/UID/GID/read-only-mode options. Application-owned secret materialization and every Compose config lifecycle/grant remain explicit manual actions because Quadlet has no equivalent managed config resource. |
| Lifecycle | `restart`, `stop_grace_period`, `stop_signal`, `init`, lifecycle hooks | `preserved` | No typed end-to-end lifecycle contract exists. Generic systemd directives alone are not a semantic mapping. |
| Process | `entrypoint`, `working_dir`, `tty`, `stdin_open`, `read_only` | `partial` / `preserved` | Safely encodable working directories and explicit true/false read-only-root intent convert end to end. Values needing systemd encoding are reported. Entrypoint, TTY, and standard-input behavior remain preserved only. |
| Names and DNS | `container_name`, `hostname`, `domainname`, `dns`, `dns_opt`, `dns_search` | `preserved` | No end-to-end mapping exists beyond `extra_hosts`. |
| Security and devices | capabilities, devices, CDI/GPU, namespaces, privileged, security options, sysctls | `preserved` | These require target-, rootless-, platform-, and pod-aware compatibility decisions. |
| Metadata and logging | annotations, labels, label files, logging | `preserved` | No neutral metadata/logging policy has been selected. |
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
state, ports, mounts, network relationships and ordered aliases, inspected
network/volume existence, optional creation-command evidence, and Podman pod membership evidence.

Command, environment, `user[:group]`, and working-directory values can be preserved or compared
with a linked image observation. A retained identity is split into neutral primary user and group
fields with shared provenance. Read-only-root state is preserved directly.
Every comparison is approximate and receives conversion-decision provenance. Network and volume
lifecycle ownership remains uncertain. Consistent Podman pod membership becomes an ordered
neutral `ServiceGroup` with pod and container provenance, but no inferred namespace or lifecycle
semantics. Quadlet output reports unresolved groups rather than flattening them. Entrypoint, broader
runtime policy and security settings, labels, health configuration, and other inspect fields remain for the
native adapter milestones. The Podman decoder reports each meaningful unmodeled native field by
name as an unsupported outcome instead of discarding it. The Docker decoder applies the same rule
with independent `BFD` diagnostics and additionally reports the lost entrypoint/command boundary.

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
