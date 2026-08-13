# Testing strategy

Tests are part of the product contract. A conversion feature is incomplete until its fidelity, failure modes, and target-version behavior are tested.

## Test layers

### Unit tests

Cover model invariants, individual mappings, target-profile resolution, diagnostics, redaction, and deterministic rendering choices.

### Adapter contract tests

Every adapter must demonstrate:

- native model to application model
- application model to native model
- unsupported-feature reporting
- preservation of provenance
- handling of unknown or extension data
- target capability boundaries

Source-side approximate, unsupported, and invalid decisions are part of `ImportResult`. The engine
prepends them to the target plan and validates their diagnostic links before applying `LossPolicy`.
This prevents an importer warning from silently bypassing partial-output authorization.

The Quadlet adapter contract suite covers exact typed generation, application-owned and external
resource lifecycles, native dependency resolution, absolute and `%h` paths, sensitive-output debug
redaction, caller-owned relative and host-specific path resolution, explicit compatible pod
grouping, network/port conflict rejection, partial-candidate authorization, target coverage
boundaries, omitted-maximum reporting, container/pod `AddHost` generation, incompatible grouped
host mappings, deferred-address losses, required/optional dependency directives, health readiness,
dependency restart/completion losses, container restart-policy exact/approximate/unsupported boundaries,
protected repeatable labels with empty/quoted/literal-specifier values, reserved-provider omission,
missing services, and cycle rejection. Neutral-model tests separately preserve ordered host
mappings, the literal `host-gateway` token, and bracketed IPv6 spelling.

Image-artifact coverage exhaustively exercises the 12 `.image` and 28 `.build` typed settings,
their capability floors, required `Image=`/`ImageTag=` plus context validation, duplicate and
explicit-reset handling, and protected debug redaction. A public-facade Compose-to-Quadlet scenario
checks exact `.build` and `.container` bytes, source provenance, strict-versus-partial policy, and
source-only loss reporting.

Public volume tests cover all 16 typed keys, distinct logical/runtime/service names, 5.4.0 and
6.0.0 identity floors, the 6.0.2 ceiling, protected raw lists, resets/duplicates, local
type/device/options rules, `Copy=` plus `Image=` loss, and literal/typed/missing/cyclic artifacts.

Iteration-one container-setting coverage exercises all 24 typed keys through the model, relevant
native adapters, and public Quadlet round trips. It verifies the 5.4.0 floor for 21 keys, the
5.5.0 floor for memory and reload keys, the finite 6.0.2 ceiling, strict-versus-partial policy,
redaction, and rejection of ambiguous static-address/alias attachment. Unsafe, deferred, empty,
or mutually exclusive values retain explicit outcomes instead of being discarded.

Public topology tests cover container `Rootfs=`, `Notify=`, and authored `PodmanArgs=` redaction,
plus preserved pod settings, omitted `PodName=`, explicit resets, unsuffixed `ServiceName=`,
host-network/port rejection, the 5.4.0/5.6.0/5.7.0 capability floors, and the 6.0.2 ceiling.
Quadlet-to-Compose coverage proves group-runtime values remain group-scoped.

Public network tests cover all ten typed Quadlet network keys at the 5.4.0 floor and 6.0.2
ceiling, logical/runtime identity, ordered safe IPAM association, and protected debug output.
They also require an explicit Compose IPAM loss and reject resets, duplicates, and positional
multi-row IPAM inference.

Security-option coverage spans the public Compose-to-Quadlet and Quadlet-to-Compose routes for
`AppArmor`, `NoNewPrivileges`, `SeccompProfile`, `SecurityLabelDisable`,
`SecurityLabelFileType`, `SecurityLabelLevel`, `SecurityLabelNested`, `SecurityLabelType`, and
repeatable `Mask`/`Unmask`. It checks the Podman 5.8.0 AppArmor floor and 5.4.0 floor for the
other keys, canonical true forms, false SELinux options withheld from Compose, duplicate
`Mask`/`Unmask` retention, strict-versus-partial authorization, and redaction. Explicit-empty,
unsafe, and conflicting values stay non-exact; grouped output keeps the values on containers, and
the suite makes no host LSM, profile, file, or runtime-enforcement claim.

The Compose exporter contract suite covers exact provider selection, optional backend runtime
selection, deterministic parse-back-validated YAML, every field in the first generated subset,
application/external and unresolved runtime resource lifecycle, runtime-name preservation,
explicit service runtime-name generation and validation, provider/runtime-sensitive outcomes,
strict/approximate/partial authorization, generation
failures, protected service-label mappings, reserved-provider omission, and complete
sensitive-document debug redaction. They also generate every neutral container restart policy
exactly. Compose import tests cover mapping and sequence labels, key-only and scalar values,
name/value provenance across merged files, every authored restart policy, and field-specific
invalid outcomes for unresolved, explicit-zero, and out-of-range retry limits.

### Golden conversion scenarios

Each scenario contains source input, BoxFerry configuration, expected native output, and expected diagnostics. Golden updates require review of both file changes and semantic outcomes.

### CLI option and value contracts

The CLI suite table-checks every finite value spelling for input type, output type, console format,
output layout, Quadlet grouping, and loss policy, plus both accepted Podman version forms. Each
finite parser also rejects an unknown or malformed value. Black-box tests separately prove the
behavioral boundaries: `exact` succeeds for exact output and blocks approximations;
`approximate` authorizes approximations and blocks partial output; and `partial` authorizes partial
output but never invalid input. Path, identifier, interpolation, profile, report, presentation, and
route options have successful cases plus representative missing, conflicting, inapplicable,
malformed, duplicate, unsafe, or already-existing cases as appropriate.

Output-directory tests cover absent and existing-empty success for both routes, visible-file and
dotfile rejection, non-directory rejection, create-new output files, and preservation of existing
content. An approximate-output collision test preserves its warning while identifying `BFO2001`
as the actual error. Combined-stream black-box tests verify progress, common diagnostic context is
printed once, varying finding evidence remains complete, native codes stay in JSON, attached help
remains paired, `fix first` follows every diagnostic group, and the final success or failure line
stays in human reading order. JSON, report-file, and support-bundle tests assert the same structured
remediation. Paired source/target tests retain an unresolved Compose image, expose its variable
through `BFC0105`, and keep the source-neutral Quadlet failure under `BFQ0014`.

Adapter and CLI tests also prove that successful Compose and Quadlet sources retain the same
native producer, stage, code, protected fields, label roles, aliased spans, notes, and help seen by
embedded callers. Seeded-canary tests cover both the top-level BoxFerry diagnostic and the nested
native-finding report object.

The first public-facade golden scenario processes a two-file Compose project with an explicit empty
profile selection and converts it to a `.network`, `.volume`, and `.container` file for Podman
5.4.0 through 6.0.2. It verifies exact generated bytes, stable unsupported subjects, retained
provenance, native graph completeness, Compose sequence/mapping extra hosts, container-level
`AddHost`, explicit `container_name` to `ContainerName=` mapping, protected service labels,
systemd quoting and literal `%` escaping, exact `restart: "no"` to `[Service] Restart=no`, and
strict-versus-partial policy behavior. The checked-in output is also accepted by the installed
Podman 6.0.2 Quadlet generator.

The second public-facade scenario maps two compatible Compose services into one caller-selected
`.pod`. It verifies reviewed bytes for the pod and both containers, pod-owned user namespace,
ports, networking, and host mapping, the complete native dependency graph, retained source
provenance, and the required `AllowApproximate` authorization.

The config/secret scenario imports application and external resources plus short and long grants.
It verifies exact repeatable `Secret=` bytes, custom-name default-target preservation, UID/GID/mode
options, stable manual-action diagnostics for config and application-owned secret material, and
strict-versus-partial authorization. Focused tests add multi-file grant provenance and sensitive
runtime-name redaction.

The dependency scenarios prove long-form required, optional, and health-gated edges in exact
separate-container output and short-form ordering inside an explicitly selected pod. Adapter tests
separately cover partial restart/completion behavior and invalid missing-target/cycle behavior.

The CLI interpolation scenario proves that processing is opt-in and per-file-before-merge, direct
variables are explicit, a named process variable is marked sensitive, an unauthorized ambient
value cannot override a Compose default, and the reviewed output remains exact. Separate
black-box failures cover missing authorized variables and duplicate sources before output
creation. Another test supplies ambient values without `--interpolate` and proves they neither
enter output nor diagnostic text.

Neutral-model, adapter, and public-facade tests protect execution user/group values, user
namespaces, ordered supplementary groups, working directories, explicit true/false read-only-root
intent, field provenance, and sensitive debug redaction. Golden output protects the exact
separate-container mapping. Focused regressions prove that named primary groups remain explicit
losses while named supplementary groups are retained, identical grouped `UserNS` moves to pod
scope, and mixed or conflicting namespace intent invalidates grouping.

Runtime-migration tests must assert provenance category as well as value. Effective inspection
fields use runtime-observation origins; inferred author intent uses conversion-decision origins and
an explicit non-exact outcome.

The pure `boxferry-runtime` suite currently covers duplicate snapshot identities, sensitive-by-
default effective commands/environment/creation evidence, caller-selected preservation versus
image comparison, retained and omitted command/environment/user-group/working-directory overrides,
field-level regular-health-check and protected metadata-label differences, reserved provider-
metadata diagnostics, direct read-only-root and container restart-policy preservation, incomplete image evidence, ordered network
aliases, volume relationships, uncertain lifecycle ownership, optional creation evidence, and
ordered provenance-aware service groups. It also proves that contradictory or missing pod/member
observations remain invalid or unsupported instead of being guessed. Explicit lifecycle tests
require application/external ownership plus user-override provenance, reject duplicates, retain
observation and override origins, and cover a complete observed-group-to-Quadlet public flow.
Podman and Docker native-import tests prove that both wrapper importers forward the resolutions
instead of exposing the feature only to caller-built snapshots. Native JSON and daemon
conformance fixtures begin with the
Docker and Podman adapter crates; the shared crate deliberately does not invent a native JSON
schema. `boxferry-podman` adds authored, secrets-reviewed 5.4.0 and 6.0.2 fixture sets. Its tests
cover native casing, malformed JSON, finite version rejection, image links, pod membership,
creation evidence, commands, environment, ports, network aliases, named/bind mounts, SELinux
relabeling, regular health checks, protected metadata labels, container restart policies, explicit startup-health separation,
unmodeled configuration, and raw-ID/debug redaction without an installed runtime.
Fake-executor acquisition tests additionally prove fixed resource-family ordering, no execution
for empty families, selector validation, finite pod-member and container-resource expansion,
selector/response deduplication, bind-mount exclusion, malformed-response redaction, and
selector/stdout/stderr redaction. They never invoke the process executor.

`boxferry-docker` adds authored, secrets-reviewed Engine API 1.40 and 1.55 fixture sets. Its pure
tests cover Docker-specific casing and leading-slash names, tolerant additive fields, malformed
JSON, finite API rejection, tag-plus-digest references, effective `Path`/`Args`, image defaults,
user/group-identity and working-directory overrides, read-only-root state, container restart
policies, protected metadata labels, ports, network aliases, named/bind mounts, regular health checks with API-aware start
intervals, SELinux relabeling, missing relationships, unmodeled configuration, and raw-ID/debug
redaction. Fake-executor tests prove the explicit protected daemon
endpoint and forced API version are present on every request, empty families run no command, and
container expansion follows only image, network, and named-volume references. One Unix-only test
invokes the standard process executor against a temporary assertion script—not Docker—to verify
the exact argument array, forced API version, isolated empty client configuration, and removed
ambient selection variables.

All-feature public-facade integration tests convert complete resource-free Podman observations and
Docker inspect documents into reviewed Quadlet bytes and convert a Docker observation into
reviewed Compose YAML. They verify that the broad `BFR0001` uncertainty outcome must be authorized
through the same engine path used by every other importer.
The Podman observation slice additionally resolves one group explicitly and verifies its group-
named `.pod`, container reference, `BFR0009`, and `BFQ0007` outcomes.

### Property and round-trip tests

Use generated inputs where useful to verify parsing never panics, deterministic output, native round trips, and application-model round trips.

### Differential tests

Docker, Podman, Compose implementations, Helm, Kustomize, and Kubernetes tools may be used as behavior oracles. Store the exact tool version, command, environment, and expected result. A difference is not automatically a BoxFerry bug; it must be classified.

### Runtime integration tests

Opt-in tests exercise real Docker, Podman, systemd/Quadlet, and Kubernetes environments. The
initial supported Podman floor is 5.4. Test each supported minor version and the newest available
version where CI infrastructure permits. Docker's reviewed range is Engine API 1.40 through 1.55.
Its isolated live harness uses Podman or Docker only as an outer container engine and verifies the
implementation inside the digest-pinned official Docker Engine 29.7.1 image. No Docker evidence is
inferred from the local `podman-docker` command itself.

The first live tier uses the exact digest-pinned images in
[`../tools/podman-runtime-matrix.toml`](../tools/podman-runtime-matrix.toml). Pull requests validate
that contract without starting a container. The weekly/manual `Podman runtime conformance`
workflow runs one disposable job for each available 5.x minor lane. It gives an ephemeral outer
container the privileges required for nested Podman, but does not mount a host runtime socket or
repository write path. Podman 6.0.2 is the reviewed decoder ceiling and an explicit live-evidence
gap for the reproducible scheduled-image tier because the official stable registry did not provide
that exact local-runtime image when the matrix was reviewed. A separate installed-current test can
exercise exactly 6.0.2 when a caller explicitly supplies that executable. It creates a unique
resource prefix, inspects only those resources, and removes them before returning.

Local live execution is optional and requires an explicitly selected outer engine:

```shell
BOXFERRY_CONTAINER_ENGINE=docker \
BOXFERRY_PODMAN_RUNTIME_VERSION=5.4.0 \
cargo ci-podman-conformance
```

Omit `BOXFERRY_PODMAN_RUNTIME_VERSION` to run every executable lane. Docker or Podman must already
be able to start privileged Linux containers. The normal Dev Container deliberately does not
mount an engine socket or request privileges; run the live command from an explicitly prepared
disposable host/runner boundary.

To verify the exact current patch through an already installed Podman:

```shell
BOXFERRY_CURRENT_PODMAN=/usr/bin/podman \
cargo ci-podman-current-conformance
```

This command intentionally changes the selected runtime for the duration of the test: it imports
an empty test image and creates one uniquely named pod, container, network, and volume with a
protected metadata label, regular health check, and finite restart policy. It checks
the Podman version before creating them and its cleanup trap removes only that unique prefix. Do
not point it at a production runtime.

Docker live conformance uses the exact image and API bounds in
[`../tools/docker-runtime-matrix.toml`](../tools/docker-runtime-matrix.toml). The weekly/manual
`Docker runtime conformance` workflow starts a private nested daemon in an ephemeral privileged
container, creates only its own prefixed resources with protected metadata, regular health, and a
finite restart policy, forces API 1.40 and 1.55 inspect responses, and
removes the outer container. It mounts a read-only script and unique temporary evidence directory,
not a host socket, home directory, credential store, or repository write path. API 1.40 here proves
current-daemon downgrade behavior; it is not historical Docker 19.03 implementation evidence.

To run it locally with the installed Podman outer engine:

```shell
BOXFERRY_CONTAINER_ENGINE=/usr/bin/podman \
cargo ci-docker-conformance
```

The first run pulls the digest-pinned official Docker image. It requires permission to run a
privileged container but does not require a local Docker Engine installation.

## Real-world corpus

The reviewed project catalogue, current application-level compatibility reading, immutable
upstream links, and opt-in command live in
[`real-world-compose-corpus.md`](real-world-compose-corpus.md). The machine-readable pins live in
[`../fixtures/real-world/corpus.toml`](../fixtures/real-world/corpus.toml).

Every imported fixture requires:

- source URL and immutable revision
- license and redistribution decision
- local modifications
- secrets review
- expected behavior or issue being tested

If redistribution is not permitted, store a generation script or minimal original reproduction instead of the source file.

The pinned-remote corpus is not a replacement for these vendored-fixture rules. It is a
network-dependent discovery and ingestion tier. A promoted regression receives a minimal authored
offline fixture with an exact expected outcome. The ComposeLens 0.1.11 restart-policy promotion
has both: focused offline boundary tests plus a corpus run covering 73 exact literal policies and
two intentionally unresolved Mattermost expressions.

Environment-file declarations have layered offline coverage: neutral model/facade tests prove
order, options, provenance, and redaction; Compose adapter tests prove short/long import without
file reads; Quadlet adapter tests prove path resolution, repeatable output, parser-parity
approximation, and explicit optional/unsafe-path losses; and a public end-to-end test proves the
Compose-to-Quadlet boundary. File-content loading is intentionally absent from these tests because
it is not authorized by declaration conversion.

## Test organization

Unit tests live beside the implemented model and engine modules. The facade's
`crates/boxferry/tests/public_api.rs` test compiles the same public orchestration path available to
external crates. Cross-crate scenarios are organized in [`../tests/`](../tests/README.md), and
Cargo-discovered repository-policy tests live in `crates/boxferry/tests/`. Fixtures live in
[`../fixtures/`](../fixtures/README.md) and are validated against the versioned
[fixture manifest contract](fixture-format.md). Further product suites are added only with
implemented behavior and meaningful assertions.

The model and facade suites exercise config/secret resource ordering, duplicate rejection,
application/external ownership, runtime names, material origins, short/long grants, nested
provenance, and redaction independently of any native adapter. Adapter and golden scenarios are
added only after the corresponding Lens releases can be consumed from crates.io.

Runtime reconstruction tests additionally prove that an inspected container name becomes an
explicit neutral service runtime name and is re-emitted by both Compose and Quadlet adapters.

## Security rules

- Never commit live credentials, tokens, private keys, or production inspect output.
- Redact secret values before snapshots.
- Give runtime tests isolated names and cleanup procedures.
- Do not make destructive cleanup broader than resources created by the test.

## Canonical commands

The workspace uses Rust 2024 with an MSRV of 1.85.0. `rust-toolchain.toml` pins the normal
development toolchain; the explicit MSRV command prevents that pin from hiding accidental use of
newer language or library features.

For the complete local deterministic suite, including Rust, Markdown, JSON, YAML, TOML, shell, and
Dockerfile formatting or linting, coverage, MSRV, dependencies, local links, and published-API
compatibility, run:

```shell
./scripts/check-all.sh
```

The commands below remain the individually runnable validation boundaries. Privileged conformance
and remote-corpus commands are opt-in and are not called by `check-all.sh`.

```shell
cargo fmt --all -- --check
./scripts/check-files.sh --check
cargo ci-check
cargo ci-core
cargo ci-compose
cargo ci-docker
cargo ci-docker-conformance # opt-in; starts an isolated privileged nested Docker daemon
cargo ci-quadlet
cargo ci-podman
cargo ci-podman-conformance # opt-in; requires the explicit environment documented above
cargo ci-podman-current-conformance # opt-in; creates temporary resources in installed Podman
cargo ci-runtime
cargo ci-policy
cargo ci-clippy
cargo ci-test
cargo ci-doctest
RUSTDOCFLAGS="-D warnings" cargo ci-doc
cargo llvm-cov --locked --workspace --all-features --all-targets --summary-only \
  --fail-under-regions 82 --fail-under-functions 87 --fail-under-lines 82
cargo +1.85.0 ci-check
cargo +1.85.0 ci-policy
cargo deny --all-features check
```

The main `ci-*` aliases use `--locked`, all workspace features, and all targets where the Cargo
command supports them. The core, Compose-only, Docker-runtime-only, Podman-runtime-only, and
Quadlet-only facade aliases protect additive feature boundaries independently. All-feature tests
include black-box CLI checks for reviewed
output, loss-policy blocking before writes, and refusal to overwrite an existing directory. CI
also runs the shared non-Rust file checker in non-mutating mode and checks local documentation
links with Lychee's network-disabled mode. The weekly/manual `External documentation link health`
workflow owns HTTP(S) validation. It reuses successful responses for up to fourteen days, does not
cache HTTP failures, and rate-limits each host. External availability therefore remains visible
without making a third-party outage or developer network condition a pull-request failure.
The Docker and Podman runtime matrices are isolated, opt-in locally, and scheduled separately from
pull-request CI. The deterministic PR contract runs the pure-library and black-box CLI suite on
Ubuntu and macOS; privileged runtime conformance, external link health, and the network-dependent
remote corpus remain scheduled/manual evidence. Native Windows CLI execution is outside the supported platform
contract; Windows users run the Linux CLI in WSL2. Small unit cases, medium
component/public-facade cases, and a repository-owned large offline CLI scenario cover positive
and negative paths. The Quadlet parser also has a bounded fixed-seed 0..=256 corpus that contains
no fuzzing dependency, catches panics, checks repeatability, diagnostic/failure span bounds,
ordering, and secret-canary redaction.

Ubuntu runs `cargo-llvm-cov` 0.8.7 with Rust 1.97.1 over the locked workspace, all features, and
all targets, without source exclusions. Its integer coarse-ratchet floors are 82% regions, 87%
functions, and 82% lines; coverage is a regression signal, not correctness evidence. The
always-running `PR gate` requires successful Rust, MSRV, dependency, documentation, SemVer,
coverage, and macOS portability jobs. Repository branch protection must require that
`PR gate` check. Every promoted issue receives a deterministic offline regression before it is
considered covered.

The full copy/paste local sequence, including workflow, Markdown, link, and SemVer checks, is in
[Development environment](development-environment.md).
