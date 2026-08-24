# API stability

Use the `boxferry` facade for normal embedding. Select component crates only when a smaller model,
engine, or adapter boundary is useful.

## Public contract

- `boxferry`, `boxferry-model`, `boxferry-engine`, `boxferry-compose`,
  `boxferry-podman`, and `boxferry-quadlet` publish one version.
- Additive `compose`, `podman`, `quadlet`, and `cli` features select integrations.
- Embedded callers can disable default features.
- The facade and CLI use the same importer → neutral model → exporter APIs.
- File access, environment access, Podman acquisition, and output writes remain explicit
  caller-selected boundaries.

Rustdoc is the exhaustive API reference. Build it with:

```console
RUSTDOCFLAGS="-D warnings" cargo ci-doc
```

## Pre-1.0 changes

- Patch releases preserve documented source compatibility.
- Minor releases may replace or remove APIs and include concise migration notes.
- Compatibility shims require a demonstrated user need.
- Public enums are non-exhaustive when new variants are expected.
- Raising Rust 1.85.0 requires a minor release, release notes, and CI evidence.
- Native catalogue updates are evidence changes and do not by themselves break the Rust API.

CLI-only conversion behavior that an embedded caller cannot obtain is an architecture defect.
