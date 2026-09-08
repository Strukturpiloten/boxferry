# BoxFerry fixtures

Fixtures are executable evidence, not sample configuration. Store each case in
`fixtures/<suite>/<id>/` with a `fixture.toml` manifest and only the files named by that
manifest.

## Manifest

```toml
schema = 1
id = "minimal-conversion"
suite = "conversion"
description = "Protects a minimal lossless conversion."
secrets_reviewed = true
files = ["compose.yaml", "expected.container"]

[provenance]
source = "authored"
license = "MPL-2.0"
redistribution = "allowed"
modifications = "none"

[environment]
description = "No runtime or process environment is provided."

[expectations]
summary = "The workload converts without loss."
```

IDs and suite names use lowercase ASCII letters, digits, and hyphens. The ID matches the directory;
the suite matches its parent. Valid suites are `model`, `adapter-contract`, `conversion`,
`roundtrip`, `differential`, and `real-world`.

## Evidence and safety

- `authored`, `external`, and `generated` are the supported provenance sources.
- External evidence includes an immutable URL and revision.
- Generated evidence records the oracle implementation, exact version, and command.
- Every case records license, redistribution, modifications, environment, and expected behavior.
- Every listed path is relative, inside the fixture directory, present, and unique.
- Set `secrets_reviewed = true` only after inspecting every listed file.
- Never store credentials, private runtime output, or unredistributable upstream data.

The repository-policy suite validates these rules.

## Live conformance matrix

`conformance/podman-live/matrix.tsv` is a reviewed inventory for the opt-in live
runner, not a deterministic parser fixture. Each row pins one trusted nested-Podman image
and records its version, distribution, root mode, and lane. `scenarios.tsv` inventories
routes; `limitations.tsv` records reviewed image-level coverage exceptions. The runner
validates all three catalogues before pulling anything.

All 48 installed-build cells are digest-pinned. Forty-three execute the complete live-resource
suite. Five UBI/openSUSE rootless images prove a specific `newuidmap` helper failure and make
no resource-coverage claim. Delete those limitation rows when corrected images initialize
nested rootless Podman. The nine-cell smoke profile spans the finite 3.0.1 through 6.1 parser
boundaries and both root modes; version-independent policy checks run once on 6.1 rootful.
Evidence is labelled `smoke` or `full`.

The external apply/reacquire case executes generated commands only inside a fresh disposable
6.1 target. A checked-in target drop-in disables unavailable nested firewall rules, so the case
proves planning, isolated network membership, and reacquisition rather than host NAT.
BoxFerry remains read-only and non-executing.

`conformance/nextcloud-application/` is a harness-owned live fixture, not an authored
scenario contract. Four application images and the standalone Docker Compose provider have
immutable reviewed metadata. The host verifies every digest and provider checksum, loads an
archive into the isolated rootless 6.1 target, and gives Compose only that disposable socket.
Direct CLI and Compose provisioning share a reviewed topology but remain independent. Redis
uses a named `/data` volume so selector and cleanup assertions cover every resource.

`conformance/forgejo-application/` is a separate harness-owned live fixture. Three immutable
images cover Forgejo 16.0.3-rootless, PostgreSQL, and the Git client/boundary peer. The same archive
is loaded into reviewed `podman-arch-rootful` and `podman-6.1-rootless` targets before API activation. Native CLI and Docker Compose provisioning independently prove real HTTP and SSH Git operations, private database networking; Compose keeps the peer in a separate project so shared-edge exclusion, collision refusal, and volume persistence after
container recreation. Only public test canaries are used; the generated SSH private key remains in
the disposable outer fixture directory and scoped cleanup removes it. Rootless receives the
no-firewall drop-in while the reviewed Arch rootful target retains stock Netavark publication with
its included `nft`; the upstream-source rootful target is excluded because it lacks `nft`.

Retained live logs and artifacts require human privacy review before sharing. Never commit raw
live output.

## Migration scenario contracts

`fixtures/scenarios/<id>/scenario.toml` is a versioned authored acceptance
contract independent of golden artifacts. It records application/component
versions and image digests, native inputs, provenance/license/privacy review,
deployment origin/root mode, and source/target capabilities. Neutral
expectations state exact included/excluded resources, ownership/shared
boundaries, required mounts/environment/ports, prerequisites, approved loss
tuples (`rule`, `subject`, `decision`, `version-scope`), and exact diagnostics.
Podman scenarios declare cassette/promotion metadata, bind/network expectations, reimport
evidence, and loss counts.

Every capability-derived importer/exporter pair has an independent outcome
(`migration-success`, `expected-rejection`, `unsupported-environment`, or
`known-migration-gap`) plus separate native-validation, runtime-probe, and
reimport evidence. Only `migration-success` counts as success; every
non-passing dimension has a reason. The shared validator rejects unknown schema
or fields, unsafe paths, unpinned digests, duplicate tuples, missing exporters,
and meaningful drift. It does not normalize semantic values; volatile IDs and
timestamps require explicit future review.
Dependency expectations compare edge options by default.
`required-dependencies[].assert-options = false` asserts only the named service-to-dependency
edge; condition, required, and restart remain unconstrained. Omitted optional network settings
are also unconstrained. Supplied `internal`, `ipv6`, and `ipam` values compare exactly.

`required-bind-mounts` compares service, fake absolute source, target, mode, and SELinux
relabel. A route that cannot retain the bind records
`bind-mount:<service>:<target>` in `semantic-gaps`; the gap key intentionally omits the host
source.

Manifest evidence entries are expectations, not attestations: the runner builds
separate observations after each check. Diagnostics match code plus subject,
and route losses match all four tuple fields. Route capability names must agree
with the executable registry. Exact numeric versions are required in schema 1.
The authored-core fixture uses explicitly synthetic digest-shaped image
identities under example.invalid and no external prerequisites; it is offline
proof only. Native validation means parsing or native-JSON structure, not an
external provider, generator or running application. See
[ADR 0039](../docs/decisions/0039-independent-migration-scenarios.md) for prerequisite grammar and observations.

## Route scenarios

Every positive adapter or conversion case declares one or more
`extensions.scenarios`. A scenario names its input, source files, and one expectation for every
exporter reported by `boxferry capabilities`. Export expectations define loss policy,
diagnostics, exact artifacts, and relevant target bounds.

Compose scenarios may provide explicit interpolation values. Quadlet scenarios name an
application and may select grouping or Podman bounds. Protected values never appear in reports but
may remain in explicitly authorized artifacts. `normalize-project-root = true` replaces only the
fixture checkout root with `<project>`.

The corpus runner tests stricter blocking policies, complete artifact and diagnostic sequences,
redaction, and applicable re-import or deterministic-output contracts. Expected artifacts are
never inferred as native input.

`real-world/corpus.toml` is a separate pinned-remote contract. Its opt-in test retrieves upstream
Compose files without vendoring them. See [testing](../docs/testing.md) and the
[application test map](../crates/boxferry/tests/README.md).
