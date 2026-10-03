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
Failure diagnostics contain only fixed roles and running/exit/OOM state, or a fixed Grafana
endpoint, HTTP status, and allowlisted error category. They never include container names, runtime
identifiers, paths, environment, protected values, raw native replies, or raw logs.

Grafana's independently authored Compose, Podman CLI, and offline Quadlet definitions disable
default plugin preinstallation and automatic updates. The pinned full image supplies the bundled
Prometheus and Loki plugins; no download fallback is permitted. Basic `/api/health` must report
`database: "ok"`, and both provisioned datasource-health endpoints must report `status: "OK"`,
within the existing single 240-second Grafana readiness deadline. A missing plugin therefore fails
closed. The later strict telemetry/dashboard assertions and all 35 live checkpoints are unchanged.

Each readiness GET receives at most five seconds, capped to the remaining shared deadline. GNU
`timeout --foreground --signal=KILL` owns and reaps the direct host runtime client. Inside the pinned
producer image, explicit BusyBox `timeout -s KILL` independently bounds `wget` from guest startup;
the socket timeout alone is not a total deadline. Killing the host client does not prove remote
guest termination, and a guest may start after the host deadline; the guest timer bounds its own
lifetime, not the delay before startup. These probes require no new software or image pin.
The classifier reads at most 16 KiB plus one overflow-detection byte, accepts only an initial
HTTP/header block and a complete JSON object, and emits closed fields. Unknown native errors remain
unknown; neither malformed success nor failed transport can satisfy readiness.

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

[`expected-compose-provisioned-aliases.yaml`](expected-compose-provisioned-aliases.yaml) is an
independently authored, privacy-safe regression contract for the exact stable identity shape
created by the reviewed Compose provider. It distinguishes the authored Compose service-key and
`container_name` identities from repeated spellings and runtime container IDs. The latter two are
never accepted as additional output aliases.

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

Plugin-setting and loader research used Grafana v13.2.1's immutable source commit
`56cd3e9288d8255fecebe5d05b48d191f50674b5` (AGPL-3.0-only), read-only HTTP inspection, and the
exact digest-pinned full image's bundled plugin manifests. A reproducible source lookup is
`curl --fail --location --max-time 30 https://raw.githubusercontent.com/grafana/grafana/56cd3e9288d8255fecebe5d05b48d191f50674b5/pkg/setting/setting_plugins.go`.
That setting implementation skips preinstallation when disabled; `pkg/setting/setting.go` and
`pkg/services/pluginsintegration/pluginsources/pluginsources.go` retain bundled plugin scanning
independently. The reviewed full image contains bundled Prometheus 13.1.7 and Loki 13.1.0; those
are owned by its existing digest, not separate downloaded dependencies. No upstream source was
copied or translated, and none is redistributed. These settings are not a claim that all outbound
metadata checks are prohibited. The earlier Loki-health HTTP 404 remains an unproved native cause:
its response body and Grafana logs were not retained, so a possible asynchronous plugin race is
not a retrospective diagnosis.

Canonical consumers are `scripts/podman-live-conformance.sh` (the live helper),
`scripts/check-all.sh` (the focused offline regression), and the Observability task in
`fixtures/conformance/migration-readiness/tiers.toml`, dispatched by
`scripts/migration-readiness.py` through the reusable `migration-readiness.yml` workflow, including
Release's fresh pre-release validation. No Lens product depends on this BoxFerry application suite.
The current-authored offline Docker prerequisite in
`fixtures/conformance/docker-application/expected-applications.json` also binds this Compose source
and helper by SHA-256. Their two bindings are updated only after reviewing this readiness change:
the six-service graph, excluded boundary peer, network isolation, mounts, loopback ingress,
dependencies, and application/persistence/safety success categories remain unchanged. The
`scripts/test-application-probes.sh` consumer verifies those source bytes without executing the
harness; this binding update admits no native Docker evidence and alters no historical evidence.
This change leaves `images.tsv`, `providers.tsv`, `scripts/lib/compose-provider.sh`, all operational
pins, and `.github/renovate.json` unchanged. The existing live-image regex manager still extracts
the image catalogue at its unchanged path; the checksum-pinned provider manager still owns the
canonical installer. Their grouping, Dashboard approvals, and no-automerge rules remain intact;
fixture Compose/Quadlet managers remain intentionally disabled, so no duplicate manager is added.

The entry point sources `scripts/lib/observability-application.sh`, exposes the bounded profile,
selects only `podman-6.1-rootless`, and calls `run_observability_application_cell` through the same
deadline, cleanup, and evidence boundary as the established application profiles. The pre-release
migration-readiness tier owns this live task; ordinary pull requests retain the offline scenario.

## Live diagnostic contracts

Expectation scope is explicit. `live-native-export` selects the native Podman importer/exporter
contract; `live-reimport` selects independently authored Compose/Quadlet reimport contracts;
`offline-scenario` selects the scenario fixture contract. Scope is never inferred from source
kind. The eight `reimport-*.tsv` templates cover all 36 live reimport combinations: two
provisioners, three selectors, two inputs, and three outputs. Compose-to-Compose and
Quadlet-to-Quadlet have empty multisets; Compose-to-Quadlet retains the reviewed network-alias
omission, and remaining routes retain exact duplicate-sensitive omissions. Their sole
interpolation is `{{resource_prefix}}`, which is the complete
`<run-prefix>-observability-` resource stem; templates must not append another application
segment. Static reviewed templates never derive expectations from reports or artifacts. Exact and
label selectors agree, while all uses separately reviewed boundary-peer deltas.

The plugin flags are literal environment strings, not lossy boolean coercions. Explicitly included
authored and live routes preserve them without new Quadlet diagnostics. The offline matcher also
checks the distinct default-withheld Podman-to-Compose-to-Quadlet route: its required environment
values cannot be emitted by Quadlet, so
`fixtures/scenarios/observability-application/expected.compose-quadlet.withheld.tsv` independently
requires one `BFQ0003` row for each plugin flag and the admin-password field. Each plugin-field
omission is a separate negative mutation; the unchanged exact matcher rejects missing or extra
rows. Podman-output routes likewise require both independent `BFP0007` environment-omission rows;
the live-template row/code counts include those two rows without changing loss policy or privacy.

[`diagnostics/`](diagnostics/) holds independently authored factored normalized multisets for the
reviewed rootless Podman 6.1.0 nested image:
`ghcr.io/strukturpiloten/podman-6.1-rootless:v6.1.0@sha256:dd00fadfff6e732728643df565a5db50f6d36dc3ec2d7f23a1fe87e905e08b5e`.
They apply the `podman-lens` 0.2.4 acquisition and promotion evidence to both independently provisioned
topologies: direct Podman CLI and Docker Compose 5.5.0. These tuples retain their original provenance.
The active lane comes from [`../podman-live/matrix.tsv`](../podman-live/matrix.tsv); changing that lane
requires rerunning both provisioners against these independent expectations, not relabelling history.
The templates retain native image, network,
creation-evidence, runtime, mount, and volume findings; no conversion report is read to construct an
expectation. The failed focused pre-release run at
`ff840133a1d8ddef9bdf3c532179b6751e464064` is only a cross-check.

Each row has five fields: code, subject, severity, decision, and required loss policy. The visible
`-` marker represents absent fields and is normalized independently for decision and policy before
exact comparison. The 214-row shared importer base includes five portable
DNS-alias promotions. Compose mode adds two service-identity promotions, while CLI mode adds six
creation-evidence tuples.
Environment diagnostics are output-specific: explicitly included Compose and Quadlet routes add
six aggregate `BFP0003` approximation warnings; default-withheld Podman routes instead add 49 named
`BFP0002` omissions. The names are independently reviewed against the existing authored Podman
exporter omission vector, including both Grafana plugin flags. Each withheld name requires exactly
one importer omission and one exporter omission; it never also receives an aggregate promotion.
Compose output adds four network tuples in either mode plus seven CLI-only dependency tuples;
Quadlet output adds the reviewed multi-network Grafana alias omission; Podman output adds 61
output-omission tuples. Exact and label selectors are equal. The all selector
adds 18 shared importer tuples (the boundary peer and default Podman network), plus one included
boundary-peer environment approximation or nine withheld named omissions. It then adds one Compose
network tuple or eleven Podman omission tuples as appropriate. This factoring preserves duplicate
tuples and represents each observed mode/selection/output multiset without copying six full routes.

Under [ADR 0060](../../../docs/decisions/0060-portable-podman-cli-import-policy.md), the base separately
requires 18 `BFP0009` reconstruction notes: six named-volume mount relationships, six aggregate
network-attachment subjects, and six restart policies. These notes have decision `reconstructed`
and no required loss policy. Grafana retains both backend and edge attachments; the CLI deduplicates
their identical complete aggregate diagnostics into one network note. Six independent `BFP0003`
warnings retain inferred application ownership for backend and the five named volumes. Edge remains
external and must not acquire an ownership warning. All-resource selection adds two boundary-peer
reconstruction notes and an inferred-ownership warning for the default Podman network. Environment,
aliases, network definition/IPAM, bind mounts, and child-field warnings remain non-exact.
The historical tuple review used PodmanLens 0.2.4; the current candidate consumes released
PodmanLens 0.2.5 and the active Podman 6.1.2 rootless lane. Exact-head acceptance must reprove both
provisioners, every selector, and every exporter/reimport without relabelling the historical review.
The six application logging observations and the all-selector peer logging observation remain
non-promoted, not neutral logging intent.
All 36 document-reimport combinations retain their separately authored contracts: Podman outputs
require named exporter omissions, never native-importer omissions or observation-only logging rows.

`{{resource_prefix}}` is the only supported template marker. The harness validates its generated
prefix and every row before substitution; malformed templates and reports fail closed. Reports and
diagnostic TSVs must not contain the protected-value canary. Deployable Compose and Quadlet artifacts
may retain it as explicitly authorized protected configuration; Podman plans must not.
