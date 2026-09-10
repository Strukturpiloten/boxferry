# ADR 0048: Bounded observability application acceptance

- Status: accepted
- Date: 2026-09-10
- Builds on: [ADR 0037](0037-finite-podman-input-and-live-conformance.md),
  [ADR 0039](0039-independent-migration-scenarios.md),
  [ADR 0042](0042-bounded-podman-creation-evidence-and-intent-promotion.md), and
  [ADR 0047](0047-bounded-migration-readiness-tiers.md)

## Context

Offline conversion of observability definitions can prove native parsing, neutral intent, every
importer/exporter route, explicit loss, and deterministic artifacts. It cannot prove that a real
collector can scrape and remote-write a metric, tail and push a log, that Grafana can use the
resulting datasources and dashboard, or that retention and storage survive recreation. Merely
starting Prometheus, Loki, Grafana, and Alloy would not prove those application semantics.

The required evidence must stay independent of production telemetry. Mounting a host Podman or
Docker socket, production log directory, or secret would broaden the privacy and trust boundary.
Repeating this large application across every historical Podman version and root mode would add
cost without strengthening the current application claim already separated from acquisition
compatibility by ADR 0037.

## Decision

Add one `observability-application` pre-release task and live-runner profile. It is bounded to the
reviewed linux/amd64 `podman-6.1-rootless` cell, a 60-minute task deadline, two CPUs, 4 GiB available
memory, 8 GiB available disk, a 2 GiB image-archive cap, and sequential readiness execution. The
profile is not part of the trusted-live tier and does not form a version or root-mode Cartesian
product.

The harness independently provisions the same six-service topology with native Podman CLI and
standalone Docker Compose 5.5.0. Controlled Nginx producers expose one known Prometheus sample and
write one known log line to a test-owned named volume. Alloy receives that volume read-only,
remote-writes the metric to Prometheus, and pushes the log to Loki. Only Grafana is published, on a
loopback port; Prometheus, Loki, Alloy, and both producers remain private. A separately owned peer
shares Grafana's edge network so exact-resource, application-label, and all-resource selection can
prove ownership boundaries and external prerequisites.

Acceptance executes the exact PromQL and LogQL expressions carried by the provisioned Grafana
dashboard, checks both provisioned datasource UIDs and dashboard content through Grafana's API,
and verifies the configured 24-hour Prometheus and Loki retention. Before container recreation it
records a Prometheus historical value and a Grafana-volume marker. After recreating containers
without deleting named volumes it requires the historical sample, current metric, known log, and
marker to remain queryable. Collision refusal, loopback ingress, private collector reachability,
volume ownership, read-only log handoff, protected-value redaction, generated-artifact privacy,
prefix-scoped cleanup, and all nine importer/exporter routes are also required.

The reviewed transient inputs are pinned exactly:

- Prometheus 3.14.0 —
  `sha256:5ce7540c3c00ef4ab0c9d2c995c6a5b9c421f44b4a115d97a2c7af3b1c21cbb0`
- Loki 3.7.7 —
  `sha256:d70e4659623f3e109af669cae76fe2a5dd5be54e2298fe8aed380d982fbc2500`
- Grafana 13.2.1 —
  `sha256:f772d434e8fab0049deb2b1b30abd43342bcfca1537614aa8d36080232cf4283`
- Alloy 1.19.2 —
  `sha256:b8ec653c44235fbe910879145dac3597d66b0aaecf60bcbbe82580767771a839`
- Nginx 1.29.1-alpine —
  `sha256:42a516af16b852e33b7682d5ef8acbd5d13fe08fecadc7ed98605ba5e3b26ab8`
- Docker Compose 5.5.0 linux/amd64 asset — SHA-256
  `c57ab918abd5b05ca7e7d0f275875dd1330a695074f309dc9eab1b49efafcd4b`

Images and the provider remain transient test inputs and are not redistributed by BoxFerry.
BoxFerry itself continues to perform read-only acquisition and never executes generated Podman
plans or starts generated Quadlet units.

This acceptance does not claim rootful behavior, other Podman versions, non-amd64 architectures,
production socket or log collection, production secrets, systemd boot behavior, or execution of
generated Quadlet units. Those boundaries remain explicit rather than being inferred from offline
generation.

Because the independent offline scenario and the bounded live application now provide the required
observability migration evidence, `observability-stack` is removed from the readiness gap set. The
remaining gap set stays explicit and non-successful; the pre-release tier deadline remains 19,800
seconds because the new task fits the tier's existing sequential budget and outer timeout margin.

## Consequences

- Observability readiness now depends on real metric, log, Grafana, retention, and persistence
  behavior rather than container health alone.
- Collector privacy and shared-resource ownership are executable acceptance boundaries.
- Image, provider, query, retention, topology, budget, or approved-loss changes require deliberate
  review of the scenario, live helper, and readiness catalogue together.
- Wider platform and service-manager claims remain excluded until separate evidence is admitted.

## Alternatives considered

### Treat offline artifacts as runtime evidence

Rejected because parsers and exporters cannot prove Prometheus, Loki, Grafana, or Alloy behavior.

### Mount the host runtime socket or production logs

Rejected because it would expose unrelated resources and private operational data to the test.

### Run every Podman version and root mode

Rejected because the existing live matrix owns acquisition compatibility; this decision adds one
current, application-semantic acceptance cell.
