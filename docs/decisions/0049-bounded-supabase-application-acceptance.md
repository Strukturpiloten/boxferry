# ADR 0049: Bounded Supabase application acceptance

- Status: accepted
- Date: 2026-09-10
- Builds on: [ADR 0037](0037-finite-podman-input-and-live-conformance.md),
  [ADR 0039](0039-independent-migration-scenarios.md),
  [ADR 0047](0047-bounded-migration-readiness-tiers.md), and
  [ADR 0048](0048-bounded-observability-application-acceptance.md)

## Context

The authored Supabase scenario proves neutral intent, exact loss and diagnostic
contracts, and all nine importer/exporter routes offline. It cannot prove that
eleven mutually dependent services can authenticate a user, expose database
changes through PostgREST and Realtime, persist Storage data, or execute an
Edge Runtime function. A partial-stack or health-only probe could conceal an
essential unsupported prerequisite and therefore cannot establish migration
readiness.

The application also has a larger resource and supply-chain footprint than
the earlier live profiles. Repeating it across historical Podman versions or
on ordinary pull requests would combine the current application-semantic claim
with acquisition compatibility and impose unbounded cost.

## Decision

Add one manual `supabase-application` profile to the shared live runner and one
privileged `pre-release` migration-readiness task immediately after
`observability-application`. Both select only the reviewed linux/amd64
`podman-6.1-rootless` cell. The readiness task invokes exactly:

```console
bash scripts/podman-live-conformance.sh --profile supabase-application --matrix-cell podman-6.1-rootless --engine podman
```

The task requires Podman, `BOXFERRY_BIN`, and a checksum-verified Docker
Compose 5.5.0 binary in `BOXFERRY_COMPOSE_BIN`. Its sources are independent
native Podman provisioning, Docker Compose 5.5.0 provisioning, and read-only
acquisition through the Podman 6.1.0 rootless API.

The pre-release tier retains concurrency one, its 19,800-second wall deadline,
and the enclosing 360-minute workflow limit. The Supabase task has its own
5,400-second deadline. Before pulling images, the profile requires four CPUs,
12 GiB available memory, and 24 GiB free space on the temporary/archive and
Podman graph-root filesystems. The eleven-image cold archive is capped at
5 GiB. Each catalogue row records both the immutable registry index used for acquisition and the
independently resolved `linux/amd64` child manifest used by the OCI archive and live runtime.
Archive creation and target loading fail unless that child digest and immutable runtime reference
survive exactly; a single-platform archive never claims to retain the multi-architecture index
digest.

The readiness task uses a conservative 14 GiB (14,336 MiB) maximum RSS ceiling:
the 12 GiB application admission requirement plus 2 GiB for the outer runner,
provisioner, and measurement headroom. Its 20 GiB (20,480 MiB) maximum
disk-growth ceiling allows four simultaneous cap-sized representations during
a cold run: the host image store, packed workload archive, extracted OCI
members in the disposable target, and nested image store. The 24 GiB preflight
leaves 4 GiB beyond the measured ceiling for filesystem and mutable application
overhead. These ceilings are fail-closed admission limits, not a claim that a
live run has already measured them; exceeding either requires new reviewed
evidence and a deliberate catalogue change.

The profile independently provisions Studio, Kong, Auth, PostgREST, Realtime,
Storage, imgproxy, Postgres Meta, Edge Runtime, PostgreSQL, and Supavisor using
both native Podman and Docker Compose. It must prove authentication,
database/API and Storage round trips, Realtime insert notification, exact Edge
Runtime responses, private-service boundaries, loopback-only Kong ingress,
shared-resource ownership, partial application selection, report redaction,
collision refusal, persistence after recreation, and prefix-scoped cleanup.

Every source selection exercises every exporter. Generated Compose and Quadlet
artifacts are reimported, but no generated deployment artifact is executed.
Immutable-image, source-review, provider, PostgreSQL component-license, graph,
route, exact diagnostic, and approved-loss contracts remain in
`fixtures/conformance/supabase-application/`. Images and Docker Compose are
transient test inputs and are not redistributed. Public test canaries may occur
only in explicitly authorized deployment artifacts and must never appear in
BoxFerry JSON reports.

The readiness catalogue removes `supabase-runtime` from its gap set because
offline semantics and a bounded live gate now define the required evidence. A
checkout alone still makes no live-success claim: only a fresh successful
pre-release task for the exact BoxFerry revision does so. GPU behavior, virtual
machines, SELinux-enforcing runtime effects, and booted systemd remain explicit
non-success gaps.

This acceptance does not claim rootful behavior, other Podman releases,
non-amd64 architectures, production credentials, SMTP/SMS/OAuth/SAML, S3
storage, arbitrary Edge functions, GPU behavior, Logflare or Vector analytics,
ComposeLens provider conformance, booted Quadlet units, or execution of a
BoxFerry-generated plan.

## Consequences

- Supabase readiness requires actual application behavior across all essential
  services rather than a reduced topology or container-health proxy.
- The expensive eleven-image lane remains manual, exact-revision, single-cell,
  and sequential.
- Image, provider, topology, diagnostic, resource-budget, and approved-loss
  changes require explicit review of the fixture, live module, readiness
  catalogue, and this decision.
- The four remaining platform gaps stay visible and cannot be converted into
  successful evidence.

## Alternatives considered

Running only the gateway, database, Auth, and Storage services was rejected
because it would hide required Realtime, Edge Runtime, Studio, Meta, imgproxy,
and Supavisor compatibility.

Treating the offline scenario as runtime proof was rejected because
deterministic conversion cannot establish user-facing behavior.

Running the graph across the full Podman matrix on pull requests was rejected
because existing matrix evidence owns acquisition coverage and the large live
claim belongs in the manual pre-release gate.
