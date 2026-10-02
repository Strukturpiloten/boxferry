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
