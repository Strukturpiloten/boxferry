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

`conformance/podman-live/matrix.tsv` is the reviewed digest-pinned inventory;
`scenarios.tsv` defines routes and `limitations.tsv` records justified gaps.
The runner validates all three before pulling. Of 48 installed-build cells,
43 exercise live resources; five UBI/openSUSE rootless cells prove only a
`newuidmap` failure. The nine-cell smoke spans Podman 3.0.1–6.1 and both root
modes. See the [live-catalogue guide](conformance/podman-live/) for admission,
replacement, and `smoke`/`full` evidence rules.

External apply/reacquire executes generated commands only inside a disposable
6.1 target with a no-firewall drop-in; it does not prove host NAT. BoxFerry
itself never executes output. The Nextcloud, Forgejo, Paperless-ngx, Immich,
and Supabase suites under `conformance/` each document their own topology,
application checks, privacy, and cleanup. Captured sanitizer-v3 cassettes
supplement authored semantics; they never replace them. Human-review retained
live logs and artifacts before sharing; never commit raw output.

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

Route artifact authorization is explicit. `environment-values = "include"` on an
`[[evidence]]` route permits the CLI to retain literal environment values only for a
privacy-reviewed, repository-authored input and a Compose or Quadlet output.
An authored Podman cassette also needs its own `include-environment-values = true`
and portable effective-setting promotion before its routes may authorize inclusion.
Omission means the CLI default, `withhold`; neither expected neutral values nor a
manifest-wide privacy review grants artifact inclusion by itself. Podman output
remains withheld while its protected-value renderer contract is unavailable in the
published dependency. The scenario runner always checks that declared protected
values stay out of reports and console output, including when artifact inclusion
is authorized.

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

Every positive adapter or conversion case declares `extensions.scenarios`
with input, sources, and an expectation for each `boxferry capabilities`
exporter: loss policy, diagnostics, artifacts, and target bounds. Compose may
define interpolation; Quadlet may select application grouping or Podman bounds.
Protected values are report-redacted and appear in artifacts only when
authorized. `normalize-project-root = true` replaces only the fixture root
with `<project>`.

The runner checks stricter policies, artifact/diagnostic sequences, redaction,
and reimport or deterministic output; expected artifacts never become native
input. `real-world/corpus.toml` is the opt-in pinned upstream Compose contract.
See [testing](../docs/testing.md) and the
[application test map](../crates/boxferry/tests/README.md).
