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
