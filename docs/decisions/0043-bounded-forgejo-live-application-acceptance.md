# ADR 0043: Bounded Forgejo live application acceptance

- Status: accepted
- Date: 2026-09-08
- Builds on: [ADR 0037](0037-finite-podman-input-and-live-conformance.md) and
  [ADR 0039](0039-independent-migration-scenarios.md)

## Context

The production-shaped Nextcloud lane proves a large application in one rootless context. Routine
migration checks also need a small workload that proves a real stateful operation, both Podman root
modes, two independent provisioners, and HTTP plus SSH publication. Container health alone cannot
prove repository data and database metadata survived recreation.

The nested rootless target disables unavailable firewall integration. Applying the same
`firewall_driver = "none"` drop-in to rootful Podman would prevent Netavark from installing the
publication rules that real HTTP and SSH probes must cross.

## Decision

Add one `forgejo-application` profile to the existing live runner. It selects only reviewed
`podman-arch-rootful` and `podman-6.1-rootless` images and runs them sequentially so one verified
application archive is reused. Each cell independently provisions Forgejo 16.0.3-rootless,
PostgreSQL 17.6, and an `alpine/git` client/boundary peer first with native Podman commands and then
with standalone Docker Compose 5.5.0 against the disposable API.

PostgreSQL stays on an internal unpublished network. Forgejo publishes loopback high ports 13000
and 12222 for container ports 3000 and 2222 and joins an externally owned shared edge network. The
peer has a different application label and only the edge network. Exact and label selectors must
exclude it; all-resource selection includes it.

The harness creates a private repository, pushes deterministic content over HTTP, clones and pushes
over SSH, then verifies both transports after recreating Forgejo and PostgreSQL while retaining
their named volumes. Credentials are public test canaries. Reports must redact them. An ephemeral
SSH private key stays below the outer container's `/tmp/boxferry-fixture` tree, never enters retained
artifacts, and is removed by scoped cleanup.

Only the rootless target receives the no-firewall drop-in. The reviewed Arch rootful target
retains stock Netavark firewall configuration and includes `nft`. The upstream-source
`podman-6.1-rootful` target is excluded because it omits `nft`, making real HTTP and SSH DNAT
impossible without a mutable package installation. The nested target pulls nothing: the host verifies immutable
digests, loads one archive before API activation, and every nested operation uses `--pull=never`.
BoxFerry remains read-only and never executes generated commands. Quadlet units receive offline
native validation; these nested images do not boot systemd.

The same-repository pull-request job receives no secrets, persists no checkout credentials, rejects
fork-authored code, and has a 30-minute hard limit. Normal duration should remain below 12 minutes.

## Consequences

- Rootful and rootless claims require actual HTTP and SSH publication success, not matrix names.
- Three application images and two Podman cells bound runtime and supply-chain review.
- Fixed high ports cannot collide across cells because each runs in its own outer network namespace.
- Process-unique names, collision refusal, exact cleanup, and the outer `--rm` trap protect ambient
  resources.
- Retained live artifacts still require human privacy review before sharing.

## Alternatives considered

SQLite was rejected because it cannot prove a private database boundary or credential retention.
Running Quadlet was rejected because the reviewed nested images do not provide a booted systemd.
A distribution matrix was rejected because application semantics, not package coverage, is the
claim. Reusing the rootless firewall drop-in for rootful was rejected because it would turn port
inspection into false publication evidence.
