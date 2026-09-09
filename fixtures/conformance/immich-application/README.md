# Immich application conformance

This harness-owned fixture drives the opt-in `immich-application` live profile. It is independent
of the authored offline migration scenario. The profile provisions the same four-service,
CPU-only Immich 3.1.0 topology with native Podman commands and the standalone Docker Compose 5.5.0
provider against an isolated Podman 6.1 rootless API.

The standard-library `media-probe.py` generates the same small PNG twice and rejects byte drift. It
creates or signs in to the disposable administrator account, disables ML inference before upload,
uploads the image through the public API, waits for metadata, preview, and thumbnail completion,
then downloads a byte-identical original and distinct nonempty derivatives. The harness separately
proves PostgreSQL asset and job-status rows, VectorChord 0.4.3 and its supporting extensions, Valkey
activity, the ML `/ping` endpoint without `/predict` or model downloads, private backend membership,
loopback-only publication, four named-volume mounts, selector isolation, protected-value redaction,
and persistence after volume-preserving recreation.

The pinned Valkey 9.1.0 Debian image intentionally owns `/data` as sticky mode `1777`; the harness
requires that exact mode, proves no other application container mounts `redisdata`, and continues to
reject other-write access on the database, machine-learning cache, and Immich library mounts.

Probe state and synthetic media are repository-authored MPL-2.0 material and remain below the
disposable run prefix. All images are digest pinned. The host verifies each digest, builds a bounded
OCI archive below 2.5 GiB, and loads it before the nested API is activated. Nested provisioning uses
only `registry.invalid` aliases with `--pull=never`. `images.tsv` and `providers.tsv` record the exact
versions, upstream provenance, licenses, and redistribution status; BoxFerry redistributes none of
the runtime images or tools.

This lane does not execute BoxFerry-generated artifacts and does not claim GPU or hardware
acceleration, ML model inference, Quadlet systemd execution, ComposeLens provider conformance,
rootful or non-amd64 operation, other Podman or Immich versions, arbitrary-media behavior, or
production-secret safety.

An optional one-off capture is enabled only when
`BOXFERRY_IMMICH_CAPTURE_DIRECTORY` names a nonexistent child of a caller-owned, non-symlink `0700`
directory outside the repository. The privacy-safe proxy holds that directory through a no-follow
descriptor, accepts only bounded bodyless `GET` requests, and writes only a sanitized candidate,
manifest, and `SHA256SUMS`. Raw request and response bytes are never written. Ordinary pull-request
CI never enables capture; the manual `immich-capture` workflow-dispatch profile retains a candidate
for one day. Every candidate needs separate privacy and provenance review before admission; no
captured-native Immich fixture is admitted by this harness.
