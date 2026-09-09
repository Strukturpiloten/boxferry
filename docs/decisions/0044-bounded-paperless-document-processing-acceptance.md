# ADR 0044: Bounded Paperless document-processing acceptance

- Status: accepted
- Date: 2026-09-09
- Builds on: [ADR 0037](0037-finite-podman-input-and-live-conformance.md),
  [ADR 0039](0039-independent-migration-scenarios.md), and
  [ADR 0043](0043-bounded-forgejo-live-application-acceptance.md)

## Context

Offline Paperless-ngx migration scenarios can prove native parsing, neutral expectations, losses,
and exporter behavior, but they cannot prove asynchronous document processing, full-text search,
office conversion, or persistent application state. A live claim also needs boundaries: accepting a
health endpoint or deriving expectations from generated output would not prove a migration-ready
document workflow.

Paperless-ngx has a comparatively large five-image footprint. Running it across the complete Podman
matrix would be expensive and would conflate application semantics with historical acquisition
coverage. QuadletLens generator evidence and ComposeLens provider-conformance evidence remain owned
by their native libraries and are not available through BoxFerry's currently released pins.

## Decision

Add one `paperless-application` profile to the existing live runner, bounded to the reviewed
`podman-6.1-rootless` cell and same-repository pull requests. The job has a 60-minute hard limit and
preflights at least two CPUs, 6 GiB available memory, and 12 GiB free space on both the temporary
archive and Podman graph-root filesystems. It validates five immutable image digests, bundles five
compressed OCI archives under a 2.5 GiB cap, and validates Docker Compose 5.5.0 by its recorded
SHA-256 before starting the isolated target. All nested pulls are disabled.

The harness independently provisions one converter-enabled Paperless-ngx, PostgreSQL, Valkey,
Gotenberg, and Tika topology using native Podman commands and Docker Compose. A standard-library
probe generates each PDF, DOCX, and ODT twice and rejects byte drift. For both provisioners it uploads
all three documents through the API, waits for asynchronous completion, proves searchable body text,
downloads byte-identical originals, and requires PDF archives for DOCX and ODT. Independent checks
prove exact PostgreSQL document rows, increasing Valkey command activity, required bounded worker and
converter settings, loopback-only web publication, unpublished private services, internal backend
membership, named mounts, and application-user access without world-writable storage.

The harness recreates every container without deleting volumes, repeats search/download/converter
checks, ingests a second document set, and then proves six database rows. Exact, label, and all-source
BoxFerry conversions exercise every exporter; generated artifacts are inspected but never executed.
Reports must omit all public protected-value canaries. Collision refusal happens before provisioning,
cleanup names only the process prefix, and retained artifacts still require human privacy review.

An optional one-off capture hook may run immediately after the first native-CLI readiness check,
only when `BOXFERRY_PAPERLESS_CAPTURE_DIRECTORY` names a nonexistent child of a caller-owned private
parent outside the repository. The privileged proxy creates and retains that child through a
no-follow directory descriptor, admits bounded bodyless `GET` requests, and runs a
fixed BoxFerry validation command. Raw traffic is never written; only a sanitized candidate,
manifest, and `SHA256SUMS` may be emitted. Pull-request CI leaves capture disabled; an explicit
`paperless-capture` workflow dispatch may upload the sanitized candidate for one day. Candidates require
independent privacy review before admission, and this test-only hook does not extend BoxFerry's production read-only
acquisition authority. At initial acceptance, no Paperless captured-native cassette was admitted.

### Captured-evidence amendment (2026-09-09)

After independent privacy, provenance, checksum, and request-coverage review, the sanitized cassette
from workflow-dispatch run
[`34368397233`](https://github.com/Strukturpiloten/boxferry/actions/runs/34368397233) at revision
`6ef0b9d6c4c8c2bc5708b7be9de19215d151721c` is admitted under
`fixtures/conformance/paperless-ngx-application/` as supplementary, non-synthetic acquisition
evidence. Its SHA-256 is
`427a86d9e8d798ae8a99e8aeeffb5bfc0ef7c94ce366fd268fbe30eb4a4acaea`; its embedded provenance
retains the capture-manifest SHA-256
`4a135307f745905f50de1522ecf4d971b78456b30f88adbd0a5f65f710dc01ea`. The reviewed candidate
`SHA256SUMS` SHA-256 was
`0f130cde39bf9952a97906a9f36d78146016e5ddca99bba201fddef9663a33c9`; those two admission files
remain outside the repository.

The cassette binds Podman/API 6.1.0 rootless, Podman revision
`cade97a52ebdf9dbf9e81de8009015776837a074`, sanitizer version 3, the reviewed
`podman-6.1-rootless` runtime image and
matrix hash, all five reviewed application images, and exact hashes for the setup, runner, Compose,
image inventory, matrix, and capture proxy sources. Review found all 27 requests to be bounded,
bodyless `GET` operations and found no native identifiers in values or object keys, compact run IDs,
private paths or addresses,
noncanonical timestamps or request IDs, protected values, authorization/cookie headers, or
unreviewed URLs. Production-acquisition replay must consume all 27 requests exactly once; independent
resource inspections may arrive in a different order when the acquisition future schedules them concurrently.

All 143 captured environment assignments are intentionally redacted. Therefore the admitted
cassette cannot establish semantic environment intent and must never replace the repository-authored
`fixtures/scenarios/paperless-ngx-application/input-podman.cassette.json`. The authored cassette
remains the only Podman scenario input; executable repository policy enforces that separation.

### Privacy correction (2026-09-09)

The cassette from workflow run `34343718035`, SHA-256
`0c56e684908b673344e0beda4f7609da0fd6a351c314b97e339e13f90f595d14`, was briefly admitted and
is now revoked. Independent follow-up review found that sanitizer version 2 did not transform JSON
object keys and did not recognize the compact live-run identifier. Native container identifiers and
the timestamped run ID therefore remained in that candidate. Sanitizer version 3 transforms and
verifies object keys, normalizes live-run identifiers, and has counterfactual regression tests for
both paths. The revoked digest must not be re-admitted; any replacement requires a fresh capture and
independent privacy, provenance, checksum, and request-coverage review.

This evidence does not claim Quadlet generator execution, ComposeLens provider conformance, other
Podman versions, rootful operation, non-amd64 architectures, arbitrary documents, production-secret
safety, or execution of a BoxFerry deployment plan.

## Consequences

- Migration readiness now includes real ingestion, conversion, search, retrieval, and persistence.
- The expensive lane remains finite, opt-in locally, and restricted to trusted pull-request code.
- Image or provider updates require explicit digest, version, license, footprint, and behavior review.
- Offline scenario contracts and live runtime evidence remain independent proof dimensions.

## Alternatives considered

Using the consume directory alone was rejected because it would not prove the public API workflow.
Using only PDF was rejected because it would not exercise Tika and Gotenberg. Running exported
artifacts was rejected because BoxFerry is non-executing and the harness must not turn migration
candidates into deployments. Expanding the distribution matrix was rejected because one reviewed
rootless application cell supplies the bounded semantic claim.
