# Fixture format

Every test fixture uses the directory form `fixtures/<suite>/<id>/` and contains a `fixture.toml` manifest. Schema version 1 provides common provenance and expectation fields while allowing suite-specific data under `extensions`.

## Required manifest

```toml
schema = 1
id = "minimal-conversion"
suite = "conversion"
description = "Protects a minimal lossless workload conversion."
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
summary = "The workload converts without loss or compatibility diagnostics."
```

`id` and suite names use lowercase ASCII letters, digits, and hyphens. The manifest ID must match its directory name, and its suite must match the parent directory.

Allowed suites are `model`, `adapter-contract`, `conversion`, `roundtrip`, `differential`, and `real-world`. The `repository-policy` test suite validates every discovered manifest.

## Provenance

`provenance.source` is one of `authored`, `external`, or `generated`.

- External fixtures also require an immutable `url` and `revision`.
- Generated fixtures require an `oracle` table containing `implementation`, exact `version`, and `command`.
- `license`, `redistribution`, and `modifications` are always required.
- If redistribution is forbidden, do not store the external input. Store a minimal authored reproduction or a retrieval/generation procedure instead.

## Files and secrets

Every `files` entry is a relative path inside its fixture directory. Absolute paths, parent traversal, missing files, and duplicate entries are rejected. Set `secrets_reviewed = true` only after checking every listed file for credentials and sensitive runtime data.

## Environment and expectations

`environment.description` states whether interpolation values, working-directory assumptions, target versions, runtime context, or external tools affect the case. Additional structured fields may be added inside the table.

`expectations.summary` explains the behavior protected by the fixture. Expected native outputs, diagnostics, conversion outcomes, exit states, and normalization rules belong in suite-specific fields or under `extensions`.
