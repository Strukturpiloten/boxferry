# ADR 0041: Separate portable intent from retained native evidence

- Status: accepted
- Date: 2026-09-07
- Amends: [ADR 0027](0027-format-neutral-native-findings.md), [ADR 0031](0031-quadlet-systemd-environment-and-native-only-reporting.md), and [ADR 0033](0033-universal-neutral-model-pipeline.md)
- Builds on: [ADR 0040](0040-quadlet-native-value-decoder-boundary.md)

## Context

The neutral `Service` and `Volume` types exposed Quadlet
`PodmanArgs=`, `GlobalArgs=`, and `ContainersConfModule=` values as
ordinary desired configuration. Exporters already treated most of those fields
as unsupported, but a same-format exporter could still render them. That mixed
portable intent with source evidence and left an opaque execution channel in
the public model.

The audit separated three categories:

1. Portable typed intent remains on neutral resources: images and builds,
   commands and entrypoints, environment, ports, mounts, dependencies,
   resource limits, networks, volumes, configs, and secrets. Raw-preserving
   scalar spellings such as durations, user namespaces, host mappings,
   sysctls, and limits remain typed by their semantic field and are validated
   by adapters; they are not free-form native argument channels.
2. Reviewed target-specific semantics remain explicit typed capabilities:
   root filesystems, startup notification, service-group runtime settings,
   volume identity/copy/ownership settings, and typed image-volume sources.
   Exporters can map or diagnose each capability independently.
3. Quadlet container `PodmanArgs=` and volume
   `ContainersConfModule=`, `GlobalArgs=`, and `PodmanArgs=` have no
   reviewed portable meaning. They are retained only as opaque source
   evidence.

## Decision

BoxFerry stores category-three facts in an application-level
`RetainedNativeEvidence` collection. Model-owned subject variants qualify
the owning neutral resource and stable conversion subject without embedding a
Lens type. Each authored event preserves value versus reset, global order,
protected physical segments, and provenance for both the event envelope and
every physical continuation segment.

Public construction rejects empty events, missing event provenance,
unprovenanced physical segments, and any segment not marked sensitive. This
keeps derived debug output private even for evidence constructed through the
public model API. Adding evidence rejects an absent owning resource. Importers
report that invalid boundary explicitly; they do not claim an exact retained
event that was not stored.

Every exporter projects one unsupported decision and one value-free diagnostic
per stable evidence subject. All event and physical-segment origins are
deduplicated in discovery order. No exporter renders, executes, forwards, or
same-format-passes-through the protected values. Exact-only policy therefore
blocks output; partial policy may authorize the separately mapped portable
candidate.

Compose and Podman importers cannot manufacture Quadlet evidence. Quadlet
import detects reset semantics from the complete logical value across physical
continuations, rather than from only the first segment.

## Consequences

The pre-1.0 Rust API removes the former raw argument getters and setters from
`Service` and `Volume`. Callers inspecting imported evidence use
`Application::retained_native_evidence`; there is deliberately no equivalent
desired-state setter that causes opaque arguments to be regenerated.

The same Quadlet source fact remains explainable on Compose, Podman, and
Quadlet routes while portable image, volume, rootfs, notification, and other
typed intent continues through normal exporters.

## Alternatives considered

Keeping the fields but documenting them as evidence was rejected because their
location and setters still represented desired state and enabled same-format
passthrough. Reducing all target-specific fields to opaque evidence was
rejected because reviewed typed semantics are safe capabilities, not a
smallest-common-subset leak. Discarding native-only values after diagnostics
was rejected because authored ordering, reset state, protection, and source
references are required to explain conversion loss.
