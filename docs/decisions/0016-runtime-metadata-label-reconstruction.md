# ADR 0016: protected runtime metadata-label reconstruction

- Status: superseded
- Date: 2026-08-05
- Superseded by: [ADR 0032](0032-future-native-lens-boundaries.md)

## Context

Docker and Podman inspection expose container labels as an effective map. That map can combine
labels inherited from image metadata, labels supplied when the container was created, and labels
added by an orchestration provider. Inspection cannot recover list-versus-map source syntax,
duplicate source entries, label files, or the original owner of one effective value.

Docker Compose adds canonical `com.docker.compose.*` labels to its resources and reserves that
namespace for the provider. Copying those observed labels into an authored Compose definition
would not reconstruct user intent and would produce an invalid Compose input. Other labels may
still contain private deployment metadata even though labels are not conventionally secret
storage.

At the time of this decision, ComposeLens 0.1.7 did not expose service labels through its typed
project or generation boundary, and QuadletLens 0.1.6 did not expose the native repeatable
container `Label=` key. Bypassing those libraries with generated YAML text or `PodmanArgs=` would
weaken the typed adapter boundaries.

## Decision

1. `MetadataLabel` is the neutral application value. Its name remains an opaque non-empty string;
   its value uses `ProtectedString` so runtime-derived values redact by default.
2. `RuntimeMetadataLabel` represents one effective inspected key/value pair. Docker and Podman
   decode their private `Config.Labels` maps independently, sort them deterministically through the
   native map representation, accept empty maps, and fail closed on empty/NUL names or non-string
   values.
3. `PreserveObservedState` retains every supplied effective label. `InferImageOverrides` compares
   labels by name and protected value, omits matching image defaults, and retains new or changed
   values with container, image, and conversion-decision provenance.
4. A missing image default in the effective container map remains unsupported rather than being
   treated as a portable deletion. Runtime labels in the reserved `com.docker.compose.*` namespace
   remain visible in the neutral model but receive `BFR0010`; they are not claimed as safely
   re-authorable application metadata.
5. Compose and Quadlet exporters report every retained label as unsupported until the respective
   Lens release provides a typed source/generation key. BoxFerry does not use raw YAML generation
   or a `PodmanArgs=` fallback for a native Quadlet feature.
6. This slice covers service/container metadata only. Image-build labels, annotations, label files,
   and network/volume/pod labels require separate lifecycle and source-ownership decisions.

## Consequences

- Runtime migration no longer hides container and image labels inside generic unmodeled native
  configuration diagnostics.
- Image metadata is not frozen into every reconstructed service merely because inspection returns
  the merged effective map.
- Compose-managed metadata is preserved as evidence without becoming silently generated input.
- Values can be reviewed by authorized callers, while debug output and diagnostics remain
  redacted.
- End-to-end label emission is intentionally release-gated on focused ComposeLens and QuadletLens
  APIs.

## Implementation amendment (2026-08-05)

ComposeLens 0.1.8 and QuadletLens 0.1.7 fulfilled the release gate in decision 5. BoxFerry now:

- imports Compose service-label mapping and sequence forms through the typed effective-project
  view, normalizes key-only/null, boolean, number, and string values without losing contributing
  name/value provenance, and refuses sensitive interpolated names that the neutral identifier
  type cannot redact;
- generates protected service-label mappings through ComposeLens, including empty values;
- emits native repeatable Quadlet `Label=` entries through the capability catalogue, quotes
  systemd-sensitive text, and doubles literal `%` so metadata cannot become a systemd specifier;
- keeps `com.docker.compose.*` labels as reviewable unsupported evidence and omits them from both
  generated targets; and
- protects the full Compose-to-Quadlet path with adapter, redaction, parse-back, golden-output,
  and local Podman 6.0.2 generator validation.

This amendment completes service/container label emission without changing decision 6. Network,
volume, pod, build/image, annotation, and label-file ownership still require separate decisions.

## Evidence

- [Docker object labels](https://docs.docker.com/engine/manage-resources/labels/)
- [Dockerfile inherited labels](https://docs.docker.com/reference/dockerfile/#label)
- [Compose service labels and reserved canonical labels](https://docs.docker.com/reference/compose-file/services/#labels)
- [Podman container labels](https://docs.podman.io/en/latest/markdown/podman-run.1.html#--label-lkeyvalue)
- [Podman Quadlet manual](https://docs.podman.io/en/latest/markdown/podman-systemd.unit.5.html)

## Alternatives considered

Treating every container label as authored intent was rejected because image and provider metadata
are merged into the inspected map. Dropping `com.docker.compose.*` values was rejected because
runtime evidence should remain reviewable even when it is unsafe to re-author. Treating label
values as always public was rejected because arbitrary organizational metadata can contain
sensitive deployment details. Emitting `--label` through `PodmanArgs=` was rejected because
Quadlet already has a native repeatable key that belongs in QuadletLens.
