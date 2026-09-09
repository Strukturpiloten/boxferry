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
provenance review before admission. No captured Paperless fixture is checked in yet.

The manual `paperless-capture` workflow-dispatch profile runs that same bounded command and uploads
the three sanitized candidate files for one day; ordinary pull requests never enable capture.
