# Testing

Native terminal wire goldens retain their renderer-owned bytes, including the final newline.
The reviewed Prettier exclusions cover Podman artifacts and `expected-docker-plan.json` only;
fixture and documentation contracts still parse and compare the Docker plans exactly. Reformatting
these files is not a substitute for reviewing the independent native intent.

The [Docker consumer bridge](decisions/0078-trusted-docker-consumer-validation.md)
requires offline source-custody/current-attempt/cleanup controls, not native
acceptance. Complete gates remain mandatory.

The historical
[target-only checkpoint](decisions/0083-bounded-docker-target-consumer-preparation.md#independent-inert-consumer-contracts)
defines separate inert consumer contracts.

Before every PR:

```console
./scripts/check-all.sh
```

## Test layers and independent scenarios

Unit tests protect model invariants, mappings, diagnostics and redaction; adapter tests protect
native/neutral contracts. Golden, CLI, documentation and opt-in conformance tests cover artifacts,
commands, safety and reviewed external tools. Test positive, failure, unsupported, privacy and
version-boundary cases. See [fast offline CLI testing](public/development/testing/index.md#fast-offline-cli-feedback).

The [scenario contract](../fixtures/README.md#migration-scenario-contracts) independently checks
expected neutral intent, every exporter, losses, diagnostics and reimports. [ADR 0039](decisions/0039-independent-migration-scenarios.md)
defines evidence boundaries and synthetic image identities. The [live harness](../scripts/podman-live-conformance.sh)
uses [scenario](../scripts/lib/scenario-contract.sh) and [validator](../scripts/lib/scenario-validators.sh)
modules; [`migration-readiness.py`](../scripts/migration-readiness.py) owns outer selection,
budgets and evidence.

## Migration-readiness tiers

The [tier catalogue](../fixtures/conformance/migration-readiness/) defines `offline`, manual
`trusted-live`, and exact-SHA `pre-release` gates. Evidence records wall/RSS/disk/concurrency budgets,
sources, targets, approved losses, and gaps. Pre-release evidence v2 isolates workers under a
four-runner cap. Four shards cover 48 Podman cells and five limitations. Its 1,200-second aggregate
deadline preserves task safety budgets; failure is never success, including Release validation.
Local `--task` records outcomes, not release approval.

Disk growth counts shared filesystem views once and fails closed on unavailable or changing identity;
see the [measurement contract](../fixtures/conformance/migration-readiness/README.md#filesystem-space-measurement).

## Fixture route corpus

Fixtures under `fixtures/<suite>/<id>/` follow [their contract](../fixtures/). Positive scenarios
specify input and expectations for every exporter in `boxferry capabilities`. They check minimum
and stricter loss policies, diagnostics, bytes, redaction, reimports, chains and fixed points;
importer and exporter coverage are independent for every registered fixture input. Synthetic
Docker-source journeys complement the document/Podman corpus for the Unreleased 16-route contract.

Compose and Quadlet output can be reimported. Podman output is a deployment plan, not observed
inventory; assertions cover deterministic artifacts, semantic operations, findings and redaction.
BoxFerry never executes `podman-commands.sh`.

## Docker development evidence

Source-library tests, CLI socket journeys, documentation examples and the authored corpus are
separate development evidence, not native acceptance. The
[Docker checks](public/development/testing/index.md#docker-development-checks) retain bounded
GET-only acquisition, independent intent, privacy/refusal, exact plan/sidecar contracts and
validation requirements. All 16 routes supplement the preserved nine-route subset.
[ADR 0085](decisions/0085-docker-cli-neutral-routes.md) retains distinct native, consumer,
application, admission and publication checkpoints; source overrides never qualify normal
registry/package integration.

### Prepared Forgejo Docker source contracts

Run `bash scripts/test-docker-forgejo-application.sh` for 76 inert-transport
controls (275.571 seconds locally). It starts no runtime and proves no native
acceptance.

The complete gate runs it once; full CI plans select its separate ten-minute
`docker-forgejo-application` job. Documentation plans skip it explicitly; the
aggregate rejects missing or failed selected work.

[Transport details](../fixtures/conformance/forgejo-application/docker-profile-contract.md)
and [ADR 0087](decisions/0087-prepared-docker-forgejo-application-profile.md)
retain choreography, cleanup, privacy and native-admission prerequisites.

## Podman evidence

Historical input anchors cover 3.0.1, 3.4.4, 4.3.1, 4.9.3, 4.9.4 and reviewed 5.4–6.1
rootful/rootless cassettes. Active upstream runtime lanes use 5.8.7 and 6.1.2 without relabelling
captures. Exact Podman rendering still ends at 6.1.0; Quadlet generator evidence reaches 6.1.2.
These are separate contracts. Use `boxferry capabilities --verbose` for finite bounds.

Fixtures include all resource kinds, pods and standalone containers, isolated and shared networks,
volumes and bind mounts, dependencies, protected metadata, incomplete resources, ambiguous
references, malformed observations, and redaction.

The normal deterministic gate needs no live Podman service or historical Podman container image.

### Live Podman conformance

[`scripts/podman-live-conformance.sh`](../scripts/podman-live-conformance.sh) is the
single inner live harness. It starts only digest-pinned nested images from
[`fixtures/conformance/podman-live/matrix.tsv`](../fixtures/conformance/podman-live/matrix.tsv).
It never mounts the host Podman socket or repository checkout into a target. Each verified
cell reports reviewed and observed versions, API and package revisions, distribution,
architecture, root mode, lane, transport, and resource-coverage level. Artifacts are removed
after success unless `--retain-artifacts` is selected.

See [ADR 0057](decisions/0057-run-owned-podman-live-storage.md) for run-owned storage cleanup.

The runner creates production-shaped test applications with stopped and running containers,
health states, pods, standalone services, aliases, volumes, bind and tmpfs mounts, labels,
environment evidence, runtime policy, and conditional secrets. It exercises every selector
and exporter plus glob rejection, socket discovery, reimports, deterministic output, strict
loss policy, and redaction. Exact run labels and identities bound cleanup. Generated commands
run only in the disposable apply/reacquire target; BoxFerry itself remains non-executing.

The 48-cell complete matrix currently has 43 full cells and five reviewed UBI/openSUSE
rootless limitations. Those five images combine file capabilities and setuid bits on
`newuidmap` and `newgidmap`, causing second-namespace `uid_map` setup to fail. The runner
verifies `helper-privilege-collision` and claims no resource coverage for those cells. The
[live-catalogue contract](../fixtures/conformance/podman-live/) keeps revalidation separate. Remove
the limitation rows after corrected images and digests are published.

Run the nine-cell smoke profile:

```console
cargo build --locked --package boxferry --bin boxferry --features podman
BOXFERRY_BIN="$PWD/target/debug/boxferry" sudo env BOXFERRY_BIN="$BOXFERRY_BIN" bash scripts/podman-live-conformance.sh --profile smoke --engine podman
```

Smoke covers Podman 3.0.1 through 6.1 across both root modes. GitHub runs the cells in
parallel with a 10-minute limit per cell. The matrix has 97 checks; version-independent checks
run once on 6.1 rootful.

The manual pre-release tier runs the 18-stage, 60-minute `application` profile on
`podman-6.1-rootless`; ordinary pull requests do not repeat the expensive live applications. It
independently provisions the same digest-pinned Nextcloud,
PostgreSQL, Redis, and Nginx topology with direct Podman commands and the verified
standalone Docker Compose provider. It verifies WebDAV upload and retrieval, database and
cache use, private and shared network boundaries, the published status and `/second/` routes,
every source selector and exporter, recreation persistence, collision refusal, cleanup, and
omission of public protected-value canaries from BoxFerry JSON conversion reports.

```console
cargo build --locked --package boxferry --bin boxferry --features podman
BOXFERRY_COMPOSE_BIN="$PWD/target/tools/docker-compose"
sudo env BOXFERRY_BIN="$PWD/target/debug/boxferry" BOXFERRY_COMPOSE_BIN="$BOXFERRY_COMPOSE_BIN" bash scripts/podman-live-conformance.sh --profile application --matrix-cell podman-6.1-rootless --engine podman
```

Detailed [application profiles](public/development/testing/index.md#podman-application-profiles)
retain Forgejo, Paperless, Immich, observability and Supabase topologies, native-provider checks,
resource/deadline budgets, privacy, persistence, cleanup and manual-capture requirements.
These bounded pre-release profiles remain separate from ordinary pull requests.

The complete profile runs all reviewed rootful and rootless cells:

```console
cargo build --locked --package boxferry --bin boxferry --features podman
BOXFERRY_BIN="$PWD/target/debug/boxferry" sudo env BOXFERRY_BIN="$BOXFERRY_BIN" bash scripts/podman-live-conformance.sh --profile full-container --engine podman
```

Resume a transiently interrupted complete run at the first unproved cell:

```console
BOXFERRY_BIN="$PWD/target/debug/boxferry" sudo env BOXFERRY_BIN="$BOXFERRY_BIN" bash scripts/podman-live-conformance.sh --profile full-container --matrix-start-at podman-alpine-3.24-rootful --engine podman
```

Every profile prints its plan and timestamped start/pass/fail events with elapsed time.
Named deadlines bound setup, runtime/BoxFerry calls and cleanup. Host-verified image
digests and transferred archives avoid nested registry access.

Manual `workflow_dispatch` runs the same smoke profile or all 48 container cells. The current
complete profile totals 1,322 checks; there is deliberately no nightly schedule.

## Gate contents

`./scripts/format-lint.sh --fix` formats and lints without tests. `./scripts/check-all.sh` also tests
every Cargo target and feature boundary, checks Rust 1.98.1, audits dependencies, builds Rustdoc,
checks coverage floors and local links, and validates publishable packages.
Changelog validation is a dedicated job required by the aggregate gate.

Coverage is a regression ratchet, not proof of semantic correctness.

## Release binary evidence

The binary gate checks archive structure, architecture, static Linux linkage, metadata and
checksums for the exact six-target revision and run. Before packaging, each binary runs version,
capability and Compose → Quadlet → Compose smoke tests. Four 64-bit targets run natively; two
32-bit Linux targets use direct QEMU user-mode emulation. This does not prove native kernel,
systemd or runtime behavior. Hosted success establishes candidate evidence; see [release guidance](releasing.md).
