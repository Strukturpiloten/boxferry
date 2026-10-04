# Offline Docker application contract prerequisite

[`expected-applications.json`](expected-applications.json) independently declares the
application-owned service, network, volume, mount, ingress, alias, dependency,
excluded-peer, and success-category inventories for Forgejo, Nextcloud,
Paperless-ngx, Immich, observability, and the full eleven-service Supabase graph.
The source kind is explicitly `authored-compose`: these expectations describe the
reviewed BoxFerry Compose application fixtures, not acquired runtime defaults.
Source files and reusable probes are bound by SHA-256 of their actual bytes.
This includes `scripts/lib/observability-application.sh`, which independently
defines the observability boundary peer and behavior/persistence probes outside
the Compose fixture. Its binding is checked without executing that harness.
`images.tsv` and `providers.tsv` in each original fixture directory remain the
canonical software/version/integrity/provenance/license inventories. This contract
duplicates no software pins, changes no dependency definition or Renovate owner,
and leaves admitted historical native evidence unchanged. Canonical fixture
updates require deliberate review of the affected contract and source hashes;
do not silently refresh hashes to make a changed graph pass.

The app-owned service counts are 2, 6, 5, 4, 6, and 11, respectively. Nextcloud's
shared proxy is declared separately from its six application services; the
second application stays excluded. Forgejo, observability, and Supabase also
exclude separately owned edge peers. Shared edges are external network
prerequisites, while application backends are internal and application-owned.
All six canonical Compose sources explicitly author application/run labels on
their created networks and named volumes. The topology validator compares those
authored labels without injecting or rewriting them; an acquired source with
different label intent needs its own independently reviewed contract. A future
harness cleanup annotation must have a separate receipt bound to the original
plan bytes and cannot masquerade as rendered intent.

[`docker-application-expectations.py`](../../../scripts/lib/docker-application-expectations.py)
validates the actual inert DockerLens version-1 complete artifact, rather than
trusting a projected inventory. It checks the exact independently supplied target
profile, private image-role aliases, resource creates, created-volume-before-mount
ordering, network attachments, loopback-only publications, and complete external
prerequisites. Network prerequisite `reference` fields remain unsigned decimal
strings through the full `u64` range. Missing, swapped, extra, duplicated,
misowned, partial, or malformed topology fails closed. Request-only streams,
unreviewed operations, and host-runtime socket mounts are rejected. JSON inputs
are capped at 1 MiB and depth 32, with duplicate keys and nonfinite numbers refused.
The container-native schema is closed: only the reviewed topology fields and
known command, environment, healthcheck, user, working-directory, and stop-setting
fields are accepted. The latter receive shape checks only and still establish no
semantic fidelity. `HostConfig` permits only `NetworkMode`, `Mounts`, and
`PortBindings`; anonymous `Volumes`, dynamic publication, DNS/host overrides,
links, and unknown native fields cannot bypass the inventory checks. Empty or
short-form source network attachments author no aliases. Immich's server mounts
only its library volume; the writable fixture bind belongs to the ML service.

The pure `validate_application` function receives plan and admission bytes plus
independently selected application, lane, profile, image aliases, prefix, run ID,
fixture root, and expectation bytes. `check_sources` separately verifies the
canonical source bytes; the CLI always runs that check first. The closed version-1
admission has these fields:

- Identity: `schema_version`, `kind`, `evidence_kind`, `source_kind`,
  `native_execution`, `application`, `lane`, `prefix`, `run_id`, and `fixture_root`.
- Byte bindings: `docker_plan_sha256`, `expectations_sha256`, and `source_sha256`.
  The last hashes sorted source records, each containing the eight-byte big-endian
  UTF-8 path length, path bytes, and 32 decoded checksum bytes.
- Declared requirements: `required_checks`, `dependencies`, `excluded_peers`, and
  `shared_services`, each exactly equal to the selected authored contract.
- Explicit non-evidence: `runtime_evidence` must be `unmeasured` and
  `budget_measurements` must be `null`.

Admission `kind` is `boxferry-docker-application-offline-admission`,
`evidence_kind` is `offline-contract-prerequisite`, `source_kind` is
`authored-compose`, and `native_execution` is `false`. A passing result proves
only this offline topology prerequisite. Declared dependencies and application,
persistence, and safety checks have not executed. In particular, this validator
does not authorize command/environment/default-field behavior, losses,
dependency-sidecar choreography, external-resource existence, native capability
compatibility, labelled-volume admission, budget ceilings, or runtime replay.
It supplies neither a live success result nor runtime replay authority. Native
labels and attachment branches still need their own evidence. This checkout
contains only the offline prerequisite; the unfinished live scaffold remains
separately preserved and is not included in this change.

The finite future live scope remains current upstream rootless for all six
applications and representative Forgejo/Nextcloud upstream rootful. This helper
does not admit Debian application lanes, Swarm, macOS, a Cartesian application
matrix, or additional native profiles. The approved four small native lanes
remain a separate prerequisite and are not implemented or validated by these
application contracts.

Run the focused offline checks without building Rust or starting a runtime:

```console
PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-docker-application-expectations.py
PYTHONDONTWRITEBYTECODE=1 python3 scripts/lib/docker-application-expectations.py check-sources
```

The tests use separately authored native artifacts for all six applications and
negative mutations for missing services, ownership, isolation, source and plan
binding, aliases, ingress, mount access, ordering, schema versions, and malformed
admission data. They never derive native test requests from generated output or
from the expectation catalogue. Direct source-correspondence assertions also
protect null/short-form aliases and the Immich server/ML mount distinction.
The existing [`test-application-probes.sh`](../../../scripts/test-application-probes.sh)
wrapper runs both read-only commands with Python bytecode writing disabled.
Its regression test verifies exact invocation, environment, and failure
propagation for each command before any later probe suites run. The wrapper is
already consumed by `check-all.sh` and hosted CI, including main and Release
validation; no separate workflow or gate definition is introduced. These checks
complement the complete gate and
do not replace any core, native-library, diagnostic/loss, or live acceptance
check. No runtime runner is wired to this helper, and this prerequisite does not
complete BoxFerry #366.

## Forgejo topology and dependency-sidecar review

[`docker-application-schedule.py`](../../../scripts/lib/docker-application-schedule.py) adds one
Forgejo-only offline check of the actual native plan, existing topology admission, and unpublished
BoxFerry dependency sidecar as an inseparable review set. It reuses `validate_application` above;
there is no second native decoder, request renderer, or runtime client. The canonical CLI calls
`check_sources` before reading artifacts. Its pure `validate_schedule` takes the three raw byte
documents and the independently supplied catalogue, application, lane, profile, image aliases,
prefix, run ID, and fixture root; embedded callers must separately check the source bindings.
The catalogue and selected profile are caller trust inputs, not authority inferred from output.
All JSON uses the existing 1-MiB/depth-32 parser, rejecting duplicate keys and nonfinite numbers.
CLI file reads reject nonregular files and final-component symlinks before a bounded read.

The sidecar contract follows the repository-authored #343 candidate's
`render_docker_dependency_sidecar` and the separately preserved #366 scaffold's pure dependency
validator. Neither unpublished worktree becomes a dependency of this check. Its closed schema-1
envelope contains `kind: "boxferry-docker-dependency-decisions"`, `native_execution: false`,
`docker_plan_sha256`, and `decisions`. Each decision has `service`, `service_runtime_name`,
`dependency`, `dependency_runtime_name`, `condition`, `condition_explicit`, `required`,
`required_explicit`, `restart`, `restart_explicit`, `fidelity`, `native_engine_field`, and
`provenance`. Conditions are `started`, `healthy`, or `completed_successfully`; implicit values
must resolve to started/required/no-restart. Fidelity must be `approximate` and
`native_engine_field` must be false. Provenance contains only `reference`, `condition`, `required`,
and `restart` arrays of the finite source-document, runtime-observation, user-override,
implementation-default, and conversion-decision category spellings. Paths and protected values
are not provenance categories. This is a BoxFerry choreography decision, not an Engine field.

The independently authored Forgejo contract requires exactly `db` and `forgejo`, with the sole
`forgejo` → `db` edge `healthy`, required, and no restart propagation. Logical keys and runtime
identities must agree with the authored inventory, not just with each other. The admission and
sidecar each bind SHA-256 of the exact native plan bytes. The closed result additionally hashes
the exact admission and sidecar bytes before reserialization; whitespace changes in plan bytes
invalidate either old binding, and whitespace changes in the other documents change their result
hashes. This does not invent an admission-to-sidecar binding absent from the existing schemas.
Missing, swapped, extra, unknown, duplicate, self-referential, reversed, or cyclic decisions fail.
The merged topology validator retains its network, volume, alias, ingress, mount, excluded-peer,
ownership, and operation-order assertions unchanged.

The result is `boxferry-docker-forgejo-offline-schedule` schema 1. It preserves the external edge
prerequisite's identity, bridge constraint, and exact decimal `u64` reference, including its
maximum value. `native_request_order` retains original request indices. Separate
`review_operations` reference those same indices in deterministic network, volume,
dependency-ordered container, then attachment order; they neither alter nor re-render the plan.
External prerequisites must be reviewed before those operations. `service_review_layers` is
`[["db"], ["forgejo"]]`, not a generated native start request or a claim that database health
was observed. Validated dependency choices are retained separately. Result identities are private
artifact-review data; raw request bodies, environment values, and commands are never copied into
the result or rejection diagnostics.

Every result says `offline-contract-prerequisite`, `native_execution: false`,
`replay_authority: false`, `native_admission: false`, `runtime_evidence: "unmeasured"`, and null
budget measurements. Command/environment/health semantics, actual external-resource existence,
image loading, startup/readiness/restart behavior, Git/SSH/persistence/isolation acceptance,
apply/reacquire, native capability admission, and measured budgets remain separate requirements.
No runtime runner or new application lane is enabled; #343 and #366 remain incomplete.

Run the focused regression without building Rust or launching a runtime:

```console
PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-docker-application-schedule.py
```

The existing `test-application-probes.sh` entry point runs this suite after the topology tests and
source-binding check, before privacy and existing Podman probe suites. Its runner regression
checks the exact Python command, disabled bytecode writes, order, and stop-on-failure behavior,
preserving every prior case. Canonical consumers remain `scripts/check-all.sh`, hosted CI/main,
and Release's complete validation through the existing shared wrapper; none requires new wiring.
BoxFerry alone owns this application contract, so Lens/native conformance and other repositories
need no consumer change or new dependency. All manifests, lockfiles, workflows, application image
and provider catalogues, and `.github/renovate.json` are unchanged. No operational pin, extraction
path, manager, grouping, approval, or historical evidence is added, moved, or changed: existing
Renovate ownership stays intact. Test Engine/profile strings and synthetic aliases are authored
offline evidence, not downloaded software pins or native admission.

## Forgejo explicitly authored fields

[`docker-forgejo-authored-fields.py`](../../../scripts/lib/docker-forgejo-authored-fields.py)
adds a separate Forgejo-only offline prerequisite for the explicitly authored environment,
database healthcheck, and application user. It reuses the complete topology and dependency-sidecar
review above, including exact plan/admission/sidecar bytes, source bindings and independently
selected context. The CLI checks canonical source bytes before opening artifact or protected
expectation files. The pure `validate_authored_fields` API performs no I/O; its caller must
separately call `topology.check_sources` before relying on source bindings, as with the other
pure helpers. No additional native decoder, renderer, runtime client, or interpolation engine
is introduced.

The separate `--interpolation-expectations` regular JSON file has exactly these fields:

- `schema_version`: integer `1` (not a boolean).
- `kind`: `boxferry-docker-forgejo-interpolation-expectations`.
- `context`: exactly `application`, `lane`, `profile`, `image_aliases`, `prefix`, `run_id`, and
  `fixture_root`, equal to the independently supplied review context. Application is `forgejo`;
  profile and aliases are full objects, not identities inferred from the native plan.
- `interpolation`: exactly `BF_DB_PASSWORD` and `BF_FORGEJO_SECRET_KEY`, supplied independently
  from the reviewed source inputs, never extracted from the artifact being checked. Each value
  is a nonempty, valid UTF-8 string of at most 4096 bytes, without NUL. Whitespace, Unicode, and
  embedded `=` remain significant.

All JSON documents retain the shared 1-MiB/depth-32, duplicate-key and nonfinite-number rejection.
Expectation input rejects missing, extra, malformed or cross-context fields. The CLI retains
bounded, nonblocking regular-file reads and rejects final-component symlinks. Keep private
expectation files owner-readable only and outside shared evidence; supplying this file is not
authorization to publish credentials, render them into artifacts, or execute output. No implicit
process environment or `.env` lookup occurs. Caller-supplied catalogue, source, profile, image
aliases and interpolation are trust inputs, not authority established by the result.

The independent authored comparisons require all three PostgreSQL and fifteen Forgejo environment
assignments, the same independent database password in both containers, and the independent Forgejo
secret key. Missing or wrong values/types and duplicate environment names fail, including duplicate
unauthored names. PostgreSQL's health test is exactly the authored `CMD-SHELL` command, with interval
2 seconds, timeout 5 seconds (native nanoseconds), and 60 retries. Forgejo's user is exactly
`1000:1000`. No health test is executed or observed.

Additional well-formed native environment assignments, including unauthored overrides, are
**unassessed**: neither their presence nor their absence establishes image-default or complete
environment fidelity. Likewise this helper does not assert unauthored command, entrypoint,
database user, application healthcheck, or health default fields. The existing native shape
boundary still applies to every field. Passing these authored-subset assertions does not establish
that an application will work.

The result kind is `boxferry-docker-forgejo-authored-fields`, schema 1, and records only the four
authored-check categories alongside the existing topology/schedule review metadata and exact
artifact hashes. It never copies protected expectation values, their input digest, raw native
environment assignments or health commands into results or rejection diagnostics. It remains
`offline-contract-prerequisite`, with `native_execution`, `native_admission` and `replay_authority`
all false, `runtime_evidence: "unmeasured"` and null budget measurements. Real external resources,
image defaults, native capability admission, startup/readiness/restart, Git/SSH/persistence/isolation,
apply/reacquire and budgets still require separate reviewed acceptance. No runtime runner is wired;
The parent issues #343 and #366 remain incomplete.

Run the independent synthetic-artifact regression without Rust builds or a runtime:

```console
PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-docker-forgejo-authored-fields.py
```

The canonical `test-application-probes.sh` wrapper runs it after the existing topology/source and
schedule checks, before privacy and Podman probes. The runner regression checks exact command,
disabled bytecode writes, ordering and stop-on-failure while retaining every prior suite and case.
Consumers remain `scripts/check-all.sh`, CI/main and Release validation through that same wrapper;
there is no new workflow or gate. BoxFerry owns this application assertion; no Lens or other
repository consumer changes. All manifests, lockfiles, workflows, image/provider catalogues and
Renovate definitions are unchanged. The new helper/tests contain no operational software pin,
download or package declaration and add no Renovate extraction match: existing native/custom
managers, grouping and approval rules retain their canonical paths, with synthetic profile strings
remaining offline test data rather than managed dependencies.
