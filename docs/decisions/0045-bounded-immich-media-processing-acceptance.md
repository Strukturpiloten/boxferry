# ADR 0045: Bounded Immich media-processing acceptance

- Status: accepted
- Date: 2026-09-09
- Builds on: [ADR 0037](0037-finite-podman-input-and-live-conformance.md),
  [ADR 0039](0039-independent-migration-scenarios.md), and
  [ADR 0044](0044-bounded-paperless-document-processing-acceptance.md)

## Context

An offline Immich migration scenario can prove native parsing, neutral expectations, structured
loss, and every exporter route. It cannot prove that the database extensions initialize, the API
accepts media, asynchronous metadata and derivative jobs complete, or the library remains usable
after container recreation. Health checks alone also do not establish those application semantics.

Immich adds a large machine-learning image and can download models or select GPU-specific behavior.
Those operations are unnecessary for the media-migration claim and would make runtime cost and
external inputs unbounded. Running the application across the complete historical Podman matrix
would conflate one current application acceptance claim with acquisition compatibility evidence.

## Decision

Add a separate `immich-application` profile to the existing live runner, bounded to the reviewed
`podman-6.1-rootless` amd64 cell on trusted same-repository pull requests. The job has a 60-minute
limit and preflights at least two CPUs, 8 GiB available memory, and 12 GiB free space on both the
archive and Podman graph-root filesystems. Four immutable images are verified and bundled below a
2.5 GiB archive cap. Docker Compose 5.5.0 is verified with its recorded SHA-256. Nested
provisioning uses `--pull=never`.

The harness independently provisions an Immich server, CPU-only Immich machine-learning service,
Valkey, and PostgreSQL 14 with VectorChord 0.4.3 using native Podman commands and Docker Compose.
It proves the ML `/ping` endpoint and disables ML inference before upload. A standard-library probe
generates a deterministic PNG twice, creates or signs in to a disposable administrator, uploads the
image, waits for metadata, preview, and thumbnail processing, and downloads the byte-identical
original plus distinct nonempty derivatives. Independent checks cover exact asset and job-status
rows, supporting database extensions, Valkey activity, internal backend and loopback publication,
named-volume ownership, selector exports, protected-value redaction, collision refusal, cleanup,
and successful retrieval after volume-preserving recreation.

The pinned Valkey 9.1.0 Debian image deliberately creates `/data` with sticky mode `1777`; the live
storage check requires that exact upstream mode and proves that only the Valkey container mounts
`redisdata`. PostgreSQL, machine learning, and Immich library mounts remain writable by their
application container but not other-writable. This service-specific rule preserves the upstream
image contract without treating it as a general shared-write policy.

The harness asserts that no `/predict` request or model-cache file appears. It does not claim GPU or
hardware acceleration, ML model inference or download, rootful operation, other Podman versions,
other architectures, arbitrary media, Quadlet systemd execution, ComposeLens provider conformance,
or execution of a BoxFerry-generated deployment plan.

An optional one-off capture hook may run after native-CLI readiness only when
`BOXFERRY_IMMICH_CAPTURE_DIRECTORY` names a nonexistent child of a caller-owned private parent
outside the repository. The existing no-follow privacy proxy retains no raw traffic and writes only
a sanitized candidate, manifest, and checksums. Pull-request CI never enables capture. Explicit
`immich-capture` workflow dispatch may retain the candidate for one day. Capture output is never
automatically admitted or described as captured-native evidence; independent privacy and
provenance review remains mandatory. No Immich captured-native cassette is admitted by this
decision.

## Consequences

- Immich migration readiness gains bounded upload, processing, retrieval, and persistence evidence.
- The expensive lane remains one trusted, current, CPU-only application cell.
- Image or provider changes require explicit version, digest, license, provenance, footprint, and
  behavior review.
- Authored offline replay, live runtime proof, and any future captured-native evidence remain
  separate dimensions.

## Alternatives considered

Using only server and ML health endpoints was rejected because they do not prove media processing.
Calling the ML prediction API was rejected because it downloads models and broadens the claim.
Executing exported artifacts was rejected because BoxFerry is non-executing and the harness must
not turn migration candidates into deployments. Expanding the distribution matrix was rejected
because one reviewed rootless application cell supplies the bounded semantic claim.
