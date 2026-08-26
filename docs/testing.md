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

[`scripts/podman-live-conformance.sh`](../scripts/podman-live-conformance.sh) is the single runner
for local and GitHub Actions evidence. It starts only the trusted nested images listed in
[`fixtures/conformance/podman-live/matrix.tsv`](../fixtures/conformance/podman-live/matrix.tsv),
never mounts a host Podman socket or a repository checkout into them. Every verified cell writes
and prints a tab-separated evidence row containing its reviewed image digest, declared and
observed Podman versions, API version, package revision, distribution, architecture, root mode,
lane, transport, and resource-coverage level. Successful diagnostic artifacts are removed unless
`--retain-artifacts` is selected; verified evidence remains visible in the GitHub Actions log. A
matrix image reference must already carry an exact `@sha256:` digest before the runner is allowed
to pull it.

The runner creates small and multi-service applications with stopped/running containers, supported
health states, pods and standalone services, network aliases, shared volumes, bind/tmpfs mounts,
environment evidence, labels, runtime policy, and conditional secrets. It exercises every
selector and exporter, literal-glob rejection, explicit/discovered sockets, reimports,
deterministic artifacts, and strict-policy blocking. A process-unique `bf65-` prefix limits cleanup
to that run. BoxFerry never executes its generated script as product behavior.

The harness-owned apply/reacquire scenario exports one portable container, validates its operation
and precondition plan, provisions its prefix-scoped network and volume, and executes the generated
script inside a fresh Podman 6.1 rootful target. Reacquisition must produce byte-identical Podman
and Compose projections before cleanup. BoxFerry itself never invokes the script.
The disposable Podman 6.1 target uses the checked-in Netavark `firewall_driver = "none"` drop-in.
The harness preloads the digest-pinned archive under the configured portable reference before the
generated container starts. This tests isolated network membership, not registry access,
masquerading, or host firewall integration.

Podman 6.1 rootful temporarily creates `/run/user/0/podman` to prove conventional discovery against
a real service. This host-side CLI behavior is version-independent. The runner refuses to replace
an existing path and removes what it creates.

All 48 matrix entries are container cells. Forty-three run the complete resource suite. The five
current UBI/openSUSE rootless image digests are listed in `podman-live/limitations.tsv`: their
packaged `newuidmap` and `newgidmap` files carry capabilities, while the image recipe also adds the
setuid bit. That combined helper state fails while opening the second namespace's `uid_map`. The
runner verifies the exact failure and records `helper-privilege-collision`; it does not claim live
BoxFerry resource coverage for those cells. Removing the extra setuid change in the image recipes,
publishing corrected images, updating the reviewed digests, and deleting the corresponding
limitation rows promotes them to the complete suite.

Run a fast, representative check locally:

```console
cargo build --locked --package boxferry --bin boxferry --features podman
BOXFERRY_BIN="$PWD/target/debug/boxferry" sudo env BOXFERRY_BIN="$BOXFERRY_BIN" bash scripts/podman-live-conformance.sh --profile smoke --engine podman
```

Locally, the nine smoke cells run sequentially. GitHub runs the same cells in isolated jobs, four at
a time. They cover the finite live-input boundaries: Podman 3.0.1 rootful/rootless, 3.4.4 rootless,
4.3.1 rootful, 4.9.3 rootless, 4.9.4 rootful, 5.4 rootless, and 6.1 rootful/rootless.

Every cell runs ten checks: setup, evidence, a minimal real workload, runtime assertions, all three
exports, and cleanup. Podman 6.1 rootful adds six version-independent checks: glob rejection,
determinism, strict loss policy, support redaction, malformed input, and conventional socket
discovery. That is 96 checks; repeating those checks on every version would add no compatibility
evidence.

The complete runtime matrix, representative live socket discovery, every selector, re-imports,
partial/disappeared-resource failures, and apply/reacquire remain in the complete profile. Pull
requests have a 10-minute hard limit per smoke cell.

The runner prints a timestamped plan and a start/pass/fail line for every numbered test, including
elapsed time. Setup operations are numbered tests too, and nested resource groups, runtime calls,
BoxFerry calls, and cleanup have named deadlines. A slow image pull, workload operation, or API call
is therefore visible before the enclosing job limit. The outer engine verifies and archives the
digest-pinned workload image; nested engines load that archive instead of reaching a registry.

Run all 48 rootful and rootless container cells. The evidence distinguishes the 43 complete cells
from the five reviewed helper limitations:

```console
cargo build --locked --package boxferry --bin boxferry --features podman
BOXFERRY_BIN="$PWD/target/debug/boxferry" sudo env BOXFERRY_BIN="$BOXFERRY_BIN" bash scripts/podman-live-conformance.sh --profile full-container --engine podman
```

If a transient registry or runner failure interrupts this long local profile, resume at the first
unproved reviewed cell instead of repeating earlier evidence:

```console
BOXFERRY_BIN="$PWD/target/debug/boxferry" sudo env BOXFERRY_BIN="$BOXFERRY_BIN" bash scripts/podman-live-conformance.sh --profile full-container --matrix-start-at podman-alpine-3.24-rootful --engine podman
```

GitHub workflows invoke the same runner. Pull requests from this repository run the nine-cell smoke
profile. Manual `workflow_dispatch` selects the same smoke profile or all 48 container cells. The
full profile plans 30 checks for 33 complete cells, 31 for nine external-apply cells, 32 for the
Podman 6.1 rootful external-apply and discovery cell, and four limitation checks for each of five
published rootless images: 1,321 checks in total. There is deliberately no nightly schedule.

## Gate contents

`./scripts/check-all.sh` formats and lints owned files, tests every Cargo target and feature
boundary, checks Rust 1.85.0, audits dependencies, builds Rustdoc, checks coverage floors and local
links, and validates publishable packages.
Changelog validation is a dedicated job required by the aggregate gate.

Coverage is a regression ratchet, not proof of semantic correctness.
