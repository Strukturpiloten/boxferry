# Paperless-ngx application conformance

This harness-owned fixture drives the opt-in `paperless-application` live profile. It is separate
from the authored offline migration scenarios. The profile independently provisions the same
five-service Paperless-ngx topology with native Podman commands and the standalone Docker Compose
provider against an isolated Podman 6.1 rootless API.

The standard-library `document-probe.py` generates byte-stable PDF, DOCX, and ODT inputs twice,
then uploads them through the Paperless API. It waits for asynchronous processing, searches unique
body text, downloads byte-identical originals, and checks that DOCX and ODT have PDF archives. The
harness separately proves PostgreSQL rows, Valkey activity, private backend membership, loopback-only
web publication, storage access and ownership, and volume-preserving recreation before a second
ingestion.

The probe and its synthetic document text are repository-authored MPL-2.0 material, use no external
document source or oracle, and fix archive metadata and PDF object ordering. Generated documents and
API state are transient evidence below the disposable run prefix and are not redistributed.

All images remain digest pinned. The host verifies each digest, creates five compressed OCI archives,
bundles them under the declared 2.5 GiB cap, and loads them before activating the nested API. Nested
provisioning uses only local `registry.invalid` aliases and `--pull=never`.

Paperless-ngx 3.1.3 is GPL-3.0-only, PostgreSQL uses the PostgreSQL license, Valkey is BSD-3-Clause,
Gotenberg is MIT, Apache Tika is Apache-2.0, and Docker Compose 5.5.0 is Apache-2.0. The exact source
repositories and reviewed versions are recorded in `images.tsv` and `providers.tsv`. BoxFerry
redistributes none of these runtime images or tools.

This lane does not execute any BoxFerry-generated artifact and does not claim Quadlet generator,
ComposeLens provider, other Podman version/root-mode/architecture, arbitrary-document, or
production-secret conformance.

## One-off native capture candidate

Native capture is disabled by default and in pull-request CI. A maintainer may explicitly record the first
CLI-provisioned, converter-enabled topology after API readiness by supplying an absolute, nonexistent
candidate below an existing caller-owned, non-symlink `0700` parent outside the repository:

```console
install -d -m 0700 /tmp/boxferry-paperless-capture-parent
test ! -e /tmp/boxferry-paperless-capture-parent/candidate
cargo build --locked --package boxferry --bin boxferry --features podman
sudo env \
  BOXFERRY_BIN="$PWD/target/debug/boxferry" \
  BOXFERRY_COMPOSE_BIN="$PWD/target/tools/docker-compose" \
  BOXFERRY_PAPERLESS_CAPTURE_DIRECTORY=/tmp/boxferry-paperless-capture-parent/candidate \
  bash scripts/podman-live-conformance.sh \
    --profile paperless-application \
    --matrix-cell podman-6.1-rootless \
    --engine podman
```

The privileged proxy creates and holds the private candidate by no-follow directory descriptor,
then permits only bounded, bodyless `GET` requests and runs a fixed
BoxFerry validation command. Raw request and response bytes remain in memory. It writes only a
sanitized cassette candidate, capture manifest, and `SHA256SUMS`; it refuses overwrites and does
not broaden BoxFerry's production read-only boundary. Every candidate requires independent privacy and
provenance review before admission.

The manual `paperless-capture` workflow-dispatch profile runs that same bounded command and uploads
the three sanitized candidate files for one day; ordinary pull requests never enable capture.

## Admitted captured-native evidence

[`paperless-ngx-6.1.0-rootless.cassette.json`](paperless-ngx-6.1.0-rootless.cassette.json) is
supplementary, non-synthetic production-acquisition evidence. It was captured by workflow-dispatch
run [`34343718035`](https://github.com/Strukturpiloten/boxferry/actions/runs/34343718035) from BoxFerry
revision `1aaafaab5fb60b55ac300e77531a958bb3a3f4b2`. The candidate artifact was
`paperless-native-capture-candidate` (`10101095627`), with archive digest
`sha256:ec4331655b8b4360432117b68cce621f81f485dbdb8184330963f25221b6c239`.

The admitted cassette SHA-256 is
`0c56e684908b673344e0beda4f7609da0fd6a351c314b97e339e13f90f595d14`. Its embedded provenance
links the candidate manifest SHA-256
`10dbd1cfcf32e74a5cb0b62ffa52ae9dc8ecf05961b302a14345c84a752d0fad`; the reviewed candidate
`SHA256SUMS` file had SHA-256
`61d0d23995b0b95c73bc4fd129228cb580370223137f637175054104f809dba5`. The manifest and checksum
file remain outside the repository because the cassette preserves the required manifest linkage
without making capture-admission metadata a semantic scenario input.

The evidence binds Podman/API `6.1.0`, rootless execution, Podman source revision
`cade97a52ebdf9dbf9e81de8009015776837a074`, matrix cell `podman-6.1-rootless`, matrix SHA-256
`1ed306f4b368c229bca927697156e2314b922c2ec728c55c2820c69a712bad25`, and runtime image
`ghcr.io/strukturpiloten/podman-6.1-rootless:v6.1.0@sha256:dd00fadfff6e732728643df565a5db50f6d36dc3ec2d7f23a1fe87e905e08b5e`.
The five source images are the exact digest-pinned references in `images.tsv`.

The independently verified source SHA-256 values at the capture revision are:

- `ee53d51a32aaab5573f289e2ae52fe6be6c9bcf263e19aff890078f0410a9df1` —
  `scripts/lib/paperless-application.sh`
- `f4ea33cb16ec041bfaa1210041cee32c0091b2343393db6f5f970de4b400fa0c` —
  `scripts/podman-live-conformance.sh`
- `f22f0e4194db3b907cdb0845045339f8561ad830407acf66f6f1063fcca6f762` —
  `fixtures/conformance/paperless-ngx-application/compose.yaml`
- `22b8606dfeae19eeb2d7c0ac81b27e5583d088283a27b0d4e40a99dd5371cd87` —
  `fixtures/conformance/paperless-ngx-application/images.tsv`
- `1ed306f4b368c229bca927697156e2314b922c2ec728c55c2820c69a712bad25` —
  `fixtures/conformance/podman-live/matrix.tsv`
- `f5a0afc2f910cf9f5135b1aabb0e914150478abce9472a8ea2842b6a0b3cfc50` —
  `fixtures/conformance/podman-live/capture_proxy.py`

Independent review verified all 27 interactions as bounded bodyless `GET` requests, strict
candidate checksums, source and image provenance, and complete request consumption through the
production acquisition path. All 2,489 cassette strings and 2,763 key occurrences were inspected:
143 environment assignments are intentionally `NAME=redacted`; native identifiers, host paths,
addresses, timestamps, request identifiers, and hostnames are normalized; protected values,
authorization/cookie headers, private markers, and unreviewed URLs are absent.

Because the capture cannot carry semantic environment values, it must never replace
`fixtures/scenarios/paperless-ngx-application/input-podman.cassette.json`. The authored cassette
remains the only Podman scenario input; repository-policy tests enforce this separation. The
captured cassette exists only to replay the exact 27-request production acquisition route.
