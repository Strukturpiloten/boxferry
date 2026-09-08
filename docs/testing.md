# Testing

Run the complete deterministic gate before every pull request:

```console
./scripts/check-all.sh
```

## Choose a test layer

| Layer         | Protects                                                |
| ------------- | ------------------------------------------------------- |
| Unit          | Model invariants, mappings, diagnostics, and redaction  |
| Adapter       | Native-to-neutral and neutral-to-native contracts       |
| Golden        | Exact artifacts and diagnostic sequences                |
| CLI           | Arguments, exit status, output safety, and reports      |
| Documentation | Every displayed command and its expected result         |
| Conformance   | Reviewed external tools in explicit opt-in environments |

Every behavior change needs a positive case and a relevant failure case. A mapping is incomplete
until unsupported values and target-version boundaries are tested.

## Independent migration scenarios

The [scenario contract](../fixtures/README.md#migration-scenario-contracts) checks
independent neutral intent, every exporter, exact losses/diagnostics and actual
reimports.
[ADR 0039](decisions/0039-independent-migration-scenarios.md) defines evidence
boundaries and synthetic image identities.

The [live entry point](../scripts/podman-live-conformance.sh) sources reusable
[scenario](../scripts/lib/scenario-contract.sh) and
[validator](../scripts/lib/scenario-validators.sh) modules; it retains ownership
of deadlines, numbered progress, isolation and cleanup.

## Fixture route corpus

Fixtures live under `fixtures/<suite>/<id>/`; their complete contract is in
[`fixtures/README.md`](../fixtures/). Each positive adapter or conversion scenario declares its
input and one expectation for every exporter reported by `boxferry capabilities`.

The corpus verifies:

- the minimum authorized loss policy and all stricter blocking policies;
- exact diagnostics, artifact sets, bytes, redaction, and protected-value handling;
- every applicable re-import, chained-conversion, and fixed-point contract;
- importer and exporter coverage as independent dimensions across all nine routes.

Compose and Quadlet artifacts can be imported again. Podman output cannot: it is a deployment plan,
not an observed inventory. Podman assertions instead compare deterministic `podman.json` and
runnable `podman-commands.sh`, semantic operations, complete findings, and redaction. BoxFerry never
executes that script.

## Podman evidence

Offline evidence separates input compatibility from output compatibility. Legacy input anchors
cover 3.0.1, 3.4.4, 4.3.1, 4.9.3, and 4.9.4; modern request-bound cassettes cover reviewed 5.4
through 6.1 observations in rootful and rootless contexts. The output target catalogue remains
5.4.0 through 6.1.0. Use `boxferry capabilities --verbose` for exact finite bounds.

Fixtures include all resource kinds, pods and standalone containers, isolated and shared networks,
volumes and bind mounts, dependencies, protected metadata, incomplete resources, ambiguous
references, malformed observations, and redaction.

The normal deterministic gate needs no live Podman service or historical Podman container image.

### Live Podman conformance

[`scripts/podman-live-conformance.sh`](../scripts/podman-live-conformance.sh) is the
single local and GitHub Actions runner. It starts only digest-pinned nested images from
[`fixtures/conformance/podman-live/matrix.tsv`](../fixtures/conformance/podman-live/matrix.tsv).
It never mounts the host Podman socket or repository checkout into a target. Each verified
cell reports reviewed and observed versions, API and package revisions, distribution,
architecture, root mode, lane, transport, and resource-coverage level. Artifacts are removed
after success unless `--retain-artifacts` is selected.

The runner creates production-shaped test applications with stopped and running containers,
health states, pods, standalone services, aliases, volumes, bind and tmpfs mounts, labels,
environment evidence, runtime policy, and conditional secrets. It exercises every selector
and exporter plus glob rejection, socket discovery, reimports, deterministic output, strict
loss policy, and redaction. A process-unique `bf65-` prefix bounds cleanup. Generated commands
run only in the disposable apply/reacquire target; BoxFerry itself remains non-executing.

The 48-cell complete matrix currently has 43 full cells and five reviewed UBI/openSUSE
rootless limitations. Those five images combine file capabilities and setuid bits on
`newuidmap` and `newgidmap`, causing second-namespace `uid_map` setup to fail. The runner
verifies `helper-privilege-collision` and claims no resource coverage for those cells. Remove
the limitation rows after corrected images and digests are published.

Run the nine-cell smoke profile:

```console
cargo build --locked --package boxferry --bin boxferry --features podman
BOXFERRY_BIN="$PWD/target/debug/boxferry" sudo env BOXFERRY_BIN="$BOXFERRY_BIN" bash scripts/podman-live-conformance.sh --profile smoke --engine podman
```

Smoke covers Podman 3.0.1 through 6.1 across both root modes. GitHub runs the cells in
parallel with a 10-minute limit per cell. The matrix contains 97 numbered checks; checks that
do not vary by Podman version run once on 6.1 rootful.

Same-repository pull requests also run the 18-stage, 60-minute `application` profile on
`podman-6.1-rootless`. It independently provisions the same digest-pinned Nextcloud,
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

The complete profile runs all reviewed rootful and rootless cells:

```console
cargo build --locked --package boxferry --bin boxferry --features podman
BOXFERRY_BIN="$PWD/target/debug/boxferry" sudo env BOXFERRY_BIN="$BOXFERRY_BIN" bash scripts/podman-live-conformance.sh --profile full-container --engine podman
```

Resume a transiently interrupted complete run at the first unproved cell:

```console
BOXFERRY_BIN="$PWD/target/debug/boxferry" sudo env BOXFERRY_BIN="$BOXFERRY_BIN" bash scripts/podman-live-conformance.sh --profile full-container --matrix-start-at podman-alpine-3.24-rootful --engine podman
```

Every profile prints a plan and timestamped start, pass, or fail event with elapsed time.
Nested resource setup, runtime calls, BoxFerry calls, and cleanup have named deadlines. The
host verifies image digests and passes archives to nested engines, so nested resource creation
does not need registry access.

Manual `workflow_dispatch` runs the same smoke profile or all 48 container cells. The current
complete profile totals 1,322 checks; there is deliberately no nightly schedule.

## Gate contents

`./scripts/check-all.sh` formats and lints owned files, tests every Cargo target and feature
boundary, checks Rust 1.85.0, audits dependencies, builds Rustdoc, checks coverage floors and local
links, and validates publishable packages.
Changelog validation is a dedicated job required by the aggregate gate.

Coverage is a regression ratchet, not proof of semantic correctness.
