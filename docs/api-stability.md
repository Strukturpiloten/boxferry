# API stability

Use the `boxferry` facade for normal embedding. Select component crates only when a smaller model,
engine, or adapter boundary is useful.

## Public contract

- `boxferry`, `boxferry-model`, `boxferry-engine`, `boxferry-compose`, `boxferry-docker`,
  `boxferry-podman`, and `boxferry-quadlet` publish one version.
- Additive `compose`, `docker`, `podman`, `quadlet`, and `cli` features select integrations.
- The Unreleased CLI defaults and binary requirements include all four adapters and 16 routes.
  The `docker` feature itself remains adapter-only; embedded callers can disable defaults.
- Docker source/import and explicit bounded read-only acquisition extend the existing inert target
  API. Source policies, resource/public-label decisions and output-value consent remain separate.
- Embedded callers can disable default features.
- The facade and CLI use the same importer → neutral model → exporter APIs.
- File access, environment access, Docker/Podman acquisition, and output writes remain explicit
  caller-selected boundaries.

Rustdoc is the exhaustive API reference. Build it with:

```console
RUSTDOCFLAGS="-D warnings" cargo ci-doc
```

The Docker adapter has a published 0.3.0 historical baseline. Normal published API comparison
and existing pre-1.0 release-type rules remain mandatory; this development integration offers no
legacy API compatibility promise or first-publication exemption. Defaults expand for the CLI,
while `default-features = false` preserves explicit embedded selection. See
[ADR 0085](decisions/0085-docker-cli-neutral-routes.md) and owning Rustdoc for migration details.

The published DockerLens 0.1.0 graph is incompatible with this development integration's needs.
An isolated exact-source rehearsal is not registry/package proof and its test-only override must
not enter published manifests or main. Compatible publication, real locked resolution, API and
package checks and fresh complete gates remain prerequisites to normal integration/release.

## Pre-1.0 changes

- Patch releases preserve documented source compatibility.
- Minor releases may replace or remove APIs and include concise migration notes.
- Compatibility shims require a demonstrated user need.
- Public enums are non-exhaustive when new variants are expected.
- Rust 1.98.1 is the minimum supported compiler, aligned with the Lens repositories.
  Raising that minimum requires a minor release, release notes, and CI evidence.
- Native catalogue updates are evidence changes and do not by themselves break the Rust API.

CLI-only conversion behavior that an embedded caller cannot obtain is an architecture defect.
