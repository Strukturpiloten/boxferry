# Testing

For quick formatting and linting without tests, run:

```console
./scripts/format-lint.sh --fix
```

Use `--check` for a read-only pass. Clippy defaults to two jobs; set `BOXFERRY_LINT_JOBS=1` on a
smaller computer. This command is a pre-push cleanliness aid, not proof that tests passed. Run the
complete deterministic gate locally when resources permit; required GitHub checks run it for every
pull request:

```console
./scripts/check-all.sh
```

It formats and lints repository files, tests all Rust targets and feature boundaries, checks the
MSRV, audits dependencies, builds documentation, verifies coverage floors, and checks local links.

## Fast offline CLI feedback

Run `./scripts/check-cli-usability.sh` or the VS Code task **BoxFerry: Fast offline CLI usability**.
It uses at most two Cargo build jobs by default and runs the CLI integration tests on one test
thread.

The preserved three-format authored route matrix exercises both `validate` and `convert` for all
nine Compose/Podman/Quadlet source-target pairs, checks that their semantic decisions agree, and
inspects planned and written artifacts against independent fixture intent. Authored runtime
names and restart behavior must survive every target. Podman output checks container
create/start resource, CLI and API identities independently. The Podman cassette proves
named network attachment and reports that
host-local bind sources need explicit same-host promotion; an additional case exercises
that opt-in for document targets. A separate reviewed scenario checks ports, mounts,
environment, runtime identity, restart policy, and selected subject-level diagnostics. The
same focused command also runs privacy tests for withheld and explicitly included values.
Podman input uses a local read-only cassette; no container runtime is required.

Focused usability cases also check the displayed portable/exact/withhold defaults, refusal of
ambiguous Quadlet inputs and unsupported Podman targets, exact-policy refusal versus reviewed
approximation, and Compose interpolation implied by `--env-file`/`--env` with later-file and
explicit-assignment precedence. Explicit same-host bind promotion checks each source, target,
and read-only mode as one association rather than independent substring presence.

The minimal Compose journeys also exercise current-directory discovery on `validate` and `convert`
for Compose, Quadlet, and Podman output. They compare planned and written decisions, verify the
selected input is reported without leaking its absolute path or an implicit `.env` value, and
reject no-match and ambiguous modern/legacy filenames without writing output. Explicit
`--input-file` remains the deterministic escape hatch. Podman output still needs an explicit
target context until its separate default-alignment work lands.

The original usability/privacy subset was measured on 2026-09-26 with Rust 1.98.1
on a 16-thread Linux host (62 GiB RAM),
two Cargo jobs and one test thread: a fresh worktree-local target completed in
35.58 s, with 683,696 KiB maximum single-process RSS and 1.1 GiB build output;
an unchanged warm rerun completed in 3.57 s with 51,380 KiB maximum
single-process RSS. These are observed bounds for this machine, not a small-PC
guarantee, measurement of the expanded Docker suite, or aggregate memory measurement.
The cassette tests require permission
to bind local Unix sockets; a restricted sandbox can deny that operation.

This command gives fast feedback while changing CLI behavior. It does not replace
`./scripts/check-all.sh`, native conformance, or release validation. The suite checks the default
Quadlet range requested as 5.4 through 6.1 and resolved as 5.4.0 through 6.1.2 across every input
format and both commands, while preserving the separate Podman deployment default of 6.1.0.
Route help exposes the Quadlet maximum default, and unsupported or reversed Quadlet ranges fail
closed. Existing sole-unit identity coverage checks filename inference and explicit overrides;
multi-unit inputs still require an explicit application name.

## Docker development checks

The development CLI registers 16 routes; synthetic Docker-row journeys complement the preserved
nine-route usability subset. The `cli_docker_journeys` integration test uses authored bounded
local Unix-socket responses. The 33 authored scenarios and 62 Docker rows are offline corpus
evidence; real native application migrations require separate results.

The source library, CLI and complete route corpus are separate development layers. Source tests
retain configured/effective/runtime-assigned distinctions, conservative defaults, provenance,
resource identity, finite topology, unsupported/version boundaries and source privacy. Authored
bounded Unix-socket facade/CLI journeys check GET-only acquisition, selector conflicts/no-match,
ambiguous discovery, budgets/cancellation, target independence, separate consents, validate/convert
parity and blocked/no-output behavior. They require no real host daemon and are not native tests.

Independent document fixtures define complete native Docker plans and conditional dependency
sidecars; use their reviewed expectations, not product-generated golden refreshes. Docker output
is terminal inert review material, never reacquisition evidence. Check exact profile identity,
complete prerequisites, typed dependency conditions/provenance, `native_execution=false`, exact
plan SHA-256, redaction, create-new/owner-only writes, collisions and refusal of required protected
or incomplete intent. Existing document roundtrips and nine-route subset checks remain intact;
full 16-route capability/help and Docker-row checks complement them.

Every displayed BoxFerry command is registered in
[documentation-examples.toml](../../../documentation-examples.toml). New document-to-Docker examples cover
validate/convert, independent artifacts and refusal without output; Docker-source help is offline.
The documentation harness has no Docker socket-fixture hook. Help proves option availability,
not a successful conversion. Execution of the integrated examples and complete gate remains
required; authored expectations alone are not validation evidence.

[ADR 0085](../../../decisions/0085-docker-cli-neutral-routes.md) preserves the independent checkpoints:
reviewed source, native Lens four-lane results, trusted exact-pair admission, actual BoxFerry
consumers, application migrations and normal registry/API/package checks. None can replace another.
The development source does not establish BoxFerry native/application acceptance or compatibility
with the published DockerLens 0.1.0 graph. Fixture profile/image pins remain historical evidence,
not active runtime image declarations. Complete validation and compatible publication remain
required before normal main/package integration; no runtime-policy gate or threshold is relaxed.

The focused script also selects this Docker journey suite, with the same two-build-job default
and single test thread. The expanded suite's cold/warm duration still requires measurement;
the older timings above do not qualify it. Neither a help command nor source presence proves a
completed conversion or a passing test.

Read the [Docker guide](../../guides/docker-input/) for acquisition limits and artifact obligations.

### Docker fixture expectations

Docker expectations select `--docker-target upstream-29.8.1-rootful` explicitly.
Authored `docker-plan.json` expectations describe inert Engine requests, never
executed commands or observed inventories. Dependency choreography is retained
separately in `docker-dependencies.json`, with `native_execution = false` and
SHA-256 binding to the exact plan bytes; it is not an Engine create field or a
runtime-readiness claim. Literal environment values require separate explicit
artifact consent even when interpolation was authorized. Blocked/invalid Docker
cases retain exact diagnostic subjects and emit no apparently complete plan.

Environment consent does not authorize private command, entrypoint, or health
text. Docker expectations retain those refusals for native observations and
Quadlet inputs even when the corresponding authored Compose text is public.
The simplified authored `supabase-application` inputs contain no process or
health overrides; they do not qualify the full vendored upstream Supabase
source, whose relative binds and protected fields have separate refusal rows.

## Podman withheld-value fixture contracts

For reviewed authored Podman-to-Podman routes, the native library expectation
retains the included-value acquisition contract while the CLI exercises its
default withheld-value contract. `cli-withheld-diagnostics-delta-file` names the
small exact diagnostic difference: `-BFP0003|services.<service>.environment`
removes the richer observation summary and
`+BFP0002|services.<service>.environment.<name>` asserts a required value by
name. The validator requires one addition for every independently required
environment assignment and one removal per affected service; all other
diagnostics remain exactly as reviewed in `diagnostics-file`. This delta does
not authorize protected values in a Podman output artifact.

`cli-withheld-losses-delta-file` records the corresponding independently
reviewed loss changes for those Podman routes. Each tab-separated row prefixes
the rule with `+` or `-`, followed by subject, decision, version scope, and
count; additions and removals are checked against `allowed-losses-file`.
The runner replays redacted acquisition to verify the exact loss tuples and
checks the CLI report's aggregate fidelity counts and policy. CLI JSON does not
expose per-subject loss tuples, so the replay and exact diagnostics provide
that evidence without claiming direct tuple visibility in the report.

## Test layers

| Layer         | Protects                                                                                      |
| ------------- | --------------------------------------------------------------------------------------------- |
| Unit          | Model invariants, mappings, diagnostics, redaction                                            |
| Adapter       | Native-to-neutral and neutral-to-native contracts                                             |
| Golden        | Exact generated bytes and diagnostic sets                                                     |
| CLI           | Arguments, exit status, output safety, reports                                                |
| Documentation | Every displayed BoxFerry command and expected artifact                                        |
| Conformance   | Reviewed external tool behavior; manual or pull-request evidence when privileged or networked |

Every behavior change needs a positive case and a relevant failure case. Tests must never use live
credentials, private runtime output, or ambient application resources.

Offline Podman evidence includes finite legacy input anchors from 3.0.1 through 4.9.4 and fourteen
immutable request-bound 5.4–6.1 rootful/rootless cassettes. Podman input scenarios run through every
exporter. Compose and Quadlet outputs are re-imported for chained and fixed-point assertions;
Podman deployment output is not observed inventory, so tests assert deterministic bytes and
semantic operation equivalence instead.

The opt-in live runner uses all 48 digest-pinned rootful/rootless images as container cells. Forty-three
currently exercise complete live resources; five published UBI/openSUSE rootless images have an
explicit helper-privilege limitation and must not be counted as resource coverage. A default-branch
manual workflow revalidates their pending immutable replacements serially and uploads only bounded,
privacy-checked JSON evidence. It has no nightly schedule and does not expand platform support.

Coverage is a regression ratchet, not evidence that semantics are correct.

## Podman application profiles

The `forgejo-application` profile runs reviewed `podman-arch-rootful` and
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
