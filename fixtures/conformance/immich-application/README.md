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
fixture is automatically admitted by this harness.

## Admitted captured-native evidence

[`immich-application-6.1.0-rootless.cassette.json`](immich-application-6.1.0-rootless.cassette.json)
is supplementary, non-synthetic production-acquisition evidence. Workflow-dispatch run
[`34369395375`](https://github.com/Strukturpiloten/boxferry/actions/runs/34369395375) captured it at
BoxFerry revision `6ef0b9d6c4c8c2bc5708b7be9de19215d151721c`. Artifact
`immich-native-capture-candidate` (`10111477551`) had archive SHA-256
`4c1ed0d1ba76dd21a2140b7ff1178e2cbcd9be587b5d165965ea60ddaac744e2`.

The admitted cassette SHA-256 is
`743f7983e64578e6c82068307e1dcbeb7ab64ee3ba0baf7b82aa89d789673a78`. Its embedded provenance
links capture-manifest SHA-256
`f04e364c5417fad7f024e9261ca2df110066dd1f094856b350dadc0c975ee6ae`; the reviewed candidate
`SHA256SUMS` SHA-256 was
`bdeed597b3dbb54e7533558ad35467e3acedc6acbdee1f7de42256b151ca9274`. The manifest and checksum
file remain outside the repository.

The evidence binds Podman/API `6.1.0` rootless, Podman revision
`cade97a52ebdf9dbf9e81de8009015776837a074`, matrix SHA-256
`1ed306f4b368c229bca927697156e2314b922c2ec728c55c2820c69a712bad25`, and runtime image
`ghcr.io/strukturpiloten/podman-6.1-rootless:v6.1.0@sha256:dd00fadfff6e732728643df565a5db50f6d36dc3ec2d7f23a1fe87e905e08b5e`.
All four source images exactly match `images.tsv`.

The independently verified source SHA-256 values are:

- `53688d266de9e6c0e763dad5c9971a2ab126b788b24ab56fc0bf886d768809cb` —
  `scripts/lib/immich-application.sh`
- `37335416bca14d60e713e247844e765dce3ea0f96c7a0d1cd1c0909feccd8f16` —
  `scripts/podman-live-conformance.sh`
- `a1ca57ad8d5342aafa2e947c9ed657b6006e89288c97d1d12166ddc86b55755d` —
  `fixtures/conformance/immich-application/compose.yaml`
- `a5ad20d591524a0014319ca4a90f08f85fc60d5e0c7268964e48ca83a872b6f5` —
  `fixtures/conformance/immich-application/media-probe.py`
- `77eabf3a9ae489d44a825d55f64d769c7a9ae4437b48a14e9e6cc9f3081bb011` —
  `fixtures/conformance/immich-application/images.tsv`
- `1ed306f4b368c229bca927697156e2314b922c2ec728c55c2820c69a712bad25` —
  `fixtures/conformance/podman-live/matrix.tsv`
- `dc65572fcd81d5b617b094b07ca5adbefed7a52944718a7a1c9cffbed3d58a3a` —
  `fixtures/conformance/podman-live/capture_proxy.py`

Independent review verified all 23 bodyless `GET` interactions, sanitizer version 3, strict
checksums, image and source provenance, and complete production replay. All 2,238 value strings and
2,377 key occurrences were traversed: 149 environment assignments are `NAME=redacted`; native
identifiers in values and object keys, compact run IDs, paths, addresses, timestamps, request IDs,
and hostnames are normalized; protected values and forbidden headers or URLs are absent.

Redaction prevents this capture from carrying semantic environment intent. It must not replace
`fixtures/scenarios/immich-application/input-podman.cassette.json`, which remains the only Podman
scenario input. The captured cassette exists only to replay the exact production acquisition route.
