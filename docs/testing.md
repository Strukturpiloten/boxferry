# Testing

The [Docker consumer bridge](decisions/0078-trusted-docker-consumer-validation.md)
requires offline source-custody/current-attempt/cleanup controls, not native
acceptance. Complete gates remain mandatory.

Run the complete deterministic gate before every pull request:

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
importer and exporter coverage are independent across all nine routes.

Compose and Quadlet output can be reimported. Podman output is a deployment plan, not observed
inventory; assertions cover deterministic artifacts, semantic operations, findings and redaction.
BoxFerry never executes `podman-commands.sh`.

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

The separate `forgejo-application` profile runs reviewed `podman-arch-rootful` and
`podman-6.1-rootless` cells sequentially under a 30-minute job limit. One three-image archive is reused. Each cell independently provisions Forgejo and PostgreSQL through native Podman and Docker
Compose. Compose provisions the boundary peer as a separate project on the external edge, keeping
selector ownership independent. It proves private-repository creation, HTTP push/clone, SSH clone/push, database
non-publication, selector isolation, report redaction, and both named volumes after container
recreation. Rootless alone receives the no-firewall drop-in; the reviewed Arch rootful target
retains stock Netavark configuration and includes `nft`, so both published ports are exercised. The reviewed upstream-source rootful target is excluded because its missing `nft` makes real
DNAT impossible. Expected cold duration is below 12 minutes.

```console
cargo build --locked --package boxferry --bin boxferry --features podman
BOXFERRY_COMPOSE_BIN="$PWD/target/tools/docker-compose"
sudo env BOXFERRY_BIN="$PWD/target/debug/boxferry" BOXFERRY_COMPOSE_BIN="$BOXFERRY_COMPOSE_BIN" scripts/podman-live-conformance.sh --profile forgejo-application --engine podman
```

The `paperless-application` profile uses `podman-6.1-rootless` with a 60-minute deadline. Native
Podman and Docker Compose 5.5.0 provision its five-service converter topology. PDF, DOCX, and ODT
inputs must pass ingestion, search, retrieval, and office-to-PDF conversion. Checks cover private
networking, storage permissions, collision refusal, and persistent PostgreSQL/Valkey volumes. It
requires two CPUs, 6 GiB RAM, 12 GiB disk, and at most 2.5 GiB compressed images.

```console
cargo build --locked --package boxferry --bin boxferry --features podman
BOXFERRY_COMPOSE_BIN="$PWD/target/tools/docker-compose"
sudo env BOXFERRY_BIN="$PWD/target/debug/boxferry" BOXFERRY_COMPOSE_BIN="$BOXFERRY_COMPOSE_BIN" bash scripts/podman-live-conformance.sh --profile paperless-application --matrix-cell podman-6.1-rootless --engine podman
```

Paperless capture is manual. Sanitizer-v2 evidence is revoked; reviewed v3 replay consumes 27 GETs.

The 60-minute `immich-application` profile runs CPU-only Immich 3.1.0 on 6.1 rootless with two CPUs,
8 GiB RAM, 12 GiB disk, and a 2.5 GiB archive cap. Its reviewed v3 replay consumes 23 GETs. Both
captured fixtures remain supplementary to authored semantics.

`observability-application` and `supabase-application` add bounded pre-release application evidence.

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
