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

### Captured-evidence amendment (2026-09-09)

After independent privacy, provenance, checksum, and request-coverage review, the sanitized
cassette from workflow-dispatch run
[`34369395375`](https://github.com/Strukturpiloten/boxferry/actions/runs/34369395375) at revision
`6ef0b9d6c4c8c2bc5708b7be9de19215d151721c` is admitted under
`fixtures/conformance/immich-application/` as supplementary, non-synthetic acquisition evidence.
Its SHA-256 is `743f7983e64578e6c82068307e1dcbeb7ab64ee3ba0baf7b82aa89d789673a78`;
its embedded provenance retains capture-manifest SHA-256
`f04e364c5417fad7f024e9261ca2df110066dd1f094856b350dadc0c975ee6ae`. The reviewed candidate
`SHA256SUMS` SHA-256 was
`bdeed597b3dbb54e7533558ad35467e3acedc6acbdee1f7de42256b151ca9274`; those two admission files
remain outside the repository.

The evidence binds sanitizer version 3, Podman/API 6.1.0 rootless, Podman revision
`cade97a52ebdf9dbf9e81de8009015776837a074`, the reviewed runtime and application image digests,
and exact source hashes. All 23 interactions are bounded bodyless `GET` requests. Review found no
native identifiers in values or object keys, compact run IDs, private paths or addresses,
noncanonical timestamps or request IDs, protected values, authorization/cookie headers, or
unreviewed URLs. Production-acquisition replay consumes every interaction exactly once.

All 149 captured environment assignments are intentionally redacted, so this evidence cannot
establish semantic environment intent. The repository-authored
`fixtures/scenarios/immich-application/input-podman.cassette.json` remains the sole Podman scenario
input; executable repository policy enforces that separation.

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
