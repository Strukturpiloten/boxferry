# API stability

Use the `boxferry` facade for normal embedding. Select component crates only when a smaller model,
engine, or adapter boundary is useful.

## Public contract

- `boxferry`, `boxferry-model`, `boxferry-engine`, `boxferry-compose`, `boxferry-docker`,
  `boxferry-podman`, and `boxferry-quadlet` publish one version.
- Additive `compose`, `podman`, `quadlet`, and `cli` features select integrations.
- Nondefault `docker` selects only the bounded inert target API. Defaults and the nine CLI routes
  are unchanged; there is no Docker source/import or acquisition API in this checkpoint.
- Embedded callers can disable default features.
- The facade and CLI use the same importer → neutral model → exporter APIs.
- File access, environment access, Podman acquisition, and output writes remain explicit
  caller-selected boundaries.

Rustdoc is the exhaustive API reference. Build it with:

```console
RUSTDOCFLAGS="-D warnings" cargo ci-doc
```

The target-only adapter already has a published 0.3.0 historical baseline. Normal published API
comparison and the existing pre-1.0 release-type rules remain mandatory. The bounded checkpoint
does not recreate legacy source APIs or assert compatibility without that comparison. Isolated
exact-source dependency rehearsals are not normal registry/package proof; see
[ADR 0081](decisions/0081-bounded-docker-target-consumer-preparation.md).

## Pre-1.0 changes

- Patch releases preserve documented source compatibility.
- Minor releases may replace or remove APIs and include concise migration notes.
- Compatibility shims require a demonstrated user need.
- Public enums are non-exhaustive when new variants are expected.
- Rust 1.98.1 is the minimum supported compiler, aligned with the Lens repositories.
  Raising that minimum requires a minor release, release notes, and CI evidence.
- Native catalogue updates are evidence changes and do not by themselves break the Rust API.

CLI-only conversion behavior that an embedded caller cannot obtain is an architecture defect.
