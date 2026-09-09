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
acquisition authority. No Paperless captured-native cassette is admitted by this decision.

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
