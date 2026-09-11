# Observability application conformance

This harness-owned fixture drives the bounded `observability-application` live profile. It is an
independently authored Prometheus 3.14.0, Loki 3.7.7, Grafana 13.2.1, and Alloy 1.19.2 application,
not a copy of an upstream example deployment. A Nginx 1.29.1-alpine service exposes one deterministic
Prometheus metric and a second service writes one deterministic log line. Alloy scrapes and
remote-writes that metric to Prometheus, tails the test-owned log volume, and pushes the line to
Loki. It never mounts a host runtime socket, host log directory, or production collector path.

The authored Alloy scrape interval is two seconds and its explicit timeout is one second. The
timeout remains strictly below the interval so the pinned Alloy release can load the configuration
and the live acceptance can distinguish configuration failure from missing metric ingestion.
Before querying telemetry, the harness also checks every fixed application role is still running.
Failure output contains only the role, running state, exit code, and OOM state; it never includes
container names, runtime identifiers, paths, environment, protected values, or raw logs.

The profile is deliberately one reviewed Podman 6.1 rootless cell rather than an application by
version Cartesian product. It independently provisions the same topology through native Podman
CLI and standalone Docker Compose 5.5.0. Only Grafana is published, on loopback port `13000`.
Prometheus, Loki, Alloy, and both producers remain on an internal network. Grafana also joins an
explicitly shared edge network containing a differently owned boundary peer; exact and label
selection must exclude that peer while retaining the shared-network prerequisite, and all-resource
selection must include it.

Runtime acceptance requires substantially more than container or dashboard reachability:

- PromQL returns
  `boxferry_fixture_temperature_celsius{source="controlled"} = 42` after Alloy remote-write.
- LogQL returns exactly the line containing `boxferry-observability-known-log` after Alloy tailing.
- Grafana reports both provisioned datasource UIDs healthy, and its provisioned dashboard retains
  those exact PromQL and LogQL expressions.
- Prometheus reports the required 24-hour TSDB retention and enabled remote-write receiver; Loki
  starts with its reviewed 24-hour filesystem/TSDB retention configuration.
- Before recreation the harness briefly emits metric value `84`, restores current value `42`, and
  records a Grafana-volume marker. After removing and recreating every application container while
  retaining volumes, `max_over_time(...[15m])` must still return `84`, Loki must still return exactly
  one known line without Alloy replay, and the Grafana marker must remain.
- Inspect evidence proves loopback-only ingress, internal collector membership, volume ownership,
  Alloy's read-only log handoff, absence of runtime-socket mounts, collision refusal, protected-value
  report redaction, prefix-scoped cleanup, and exact/label/all selection behavior.

The live module exports every selection to Compose, Quadlet, and Podman plans without executing any
generated artifact. It then reimports generated Compose and Quadlet through all three exporters,
covering every reimportable route contract. Podman output remains a deployment plan and is never
treated as observed inventory.

## Reviewed provenance

[`images.tsv`](images.tsv) is the machine-readable source of truth. Every reference combines the
reviewed release tag with the observed linux/amd64 manifest digest; the harness checks the pulled
digest, OS, and architecture before archiving it for the isolated target. Sources and licenses were
reviewed from each upstream repository and release image metadata:

- Prometheus 3.14.0 — Apache-2.0 — <https://github.com/prometheus/prometheus>
- Loki 3.7.7 — AGPL-3.0-only — <https://github.com/grafana/loki>
- Grafana 13.2.1 — AGPL-3.0-only — <https://github.com/grafana/grafana>
- Alloy 1.19.2 — Apache-2.0 — <https://github.com/grafana/alloy>
- Nginx 1.29.1-alpine official image — BSD-2-Clause — <https://github.com/nginx/docker-nginx>

[`providers.tsv`](providers.tsv) pins Docker Compose 5.5.0's linux/amd64 release asset and SHA-256.
The provider is Apache-2.0 from <https://github.com/docker/compose>. Images and the provider are
pulled or downloaded only as transient test inputs; BoxFerry redistributes none of them. The config,
dashboard, and producer under this directory are repository-authored MPL-2.0 test material.

The entry point sources `scripts/lib/observability-application.sh`, exposes the bounded profile,
selects only `podman-6.1-rootless`, and calls `run_observability_application_cell` through the same
deadline, cleanup, and evidence boundary as the established application profiles. The pre-release
migration-readiness tier owns this live task; ordinary pull requests retain the offline scenario.

## Live diagnostic contracts

Expectation scope is explicit. `live-native-export` selects the native Podman importer/exporter
contract; `live-reimport` selects independently authored Compose/Quadlet reimport contracts;
`offline-scenario` selects the scenario fixture contract. Scope is never inferred from source
kind. The eight `reimport-*.tsv` templates cover all 36 live reimport combinations: two
provisioners, three selectors, two inputs, and three outputs. Compose-to-Compose/Quadlet and
Quadlet-to-Quadlet have empty multisets; remaining routes retain exact duplicate-sensitive
omissions. Their sole interpolation is `{{resource_prefix}}`; static reviewed templates never
derive expectations from reports or artifacts. Exact and label selectors agree, while all uses
separately reviewed boundary-peer deltas.

[`diagnostics/`](diagnostics/) holds independently authored factored normalized multisets for the
reviewed rootless Podman 6.1.0 nested image:
`ghcr.io/strukturpiloten/podman-6.1-rootless:v6.1.0@sha256:dd00fadfff6e732728643df565a5db50f6d36dc3ec2d7f23a1fe87e905e08b5e`.
They apply the `podman-lens` 0.2.3 acquisition and promotion evidence to both independently provisioned
topologies: direct Podman CLI and Docker Compose 5.5.0. The templates retain native image, network,
creation-evidence, runtime, mount, and volume findings; no conversion report is read to construct an
expectation. The failed focused pre-release run at
`ff840133a1d8ddef9bdf3c532179b6751e464064` is only a cross-check.

Each row has five fields: code, subject, severity, decision, and required loss policy. The visible
`-` marker represents the two fields absent from a native Compose diagnostic and is normalized to
empty fields before exact comparison. The 209-row Compose-provisioned importer base is augmented by six CLI creation-evidence
tuples. Compose output adds four network tuples in either mode plus seven CLI-only dependency tuples;
Podman output adds 59 output-omission tuples. Exact and label selectors are equal. The all selector
adds 18 shared importer tuples (the boundary peer and default Podman network), then one Compose
network tuple or eleven Podman omission tuples as appropriate. This factoring preserves duplicate
tuples and represents each observed mode/selection/output multiset without copying six full routes.

`{{resource_prefix}}` is the only supported template marker. The harness validates its generated
prefix and every row before substitution; malformed templates and reports fail closed. Reports and
diagnostic TSVs must not contain the protected-value canary. Deployable Compose and Quadlet artifacts
may retain it as explicitly authorized protected configuration; Podman plans must not.
