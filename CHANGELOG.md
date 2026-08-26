# Changelog

All notable BoxFerry changes are recorded here. The project follows the pre-1.0 compatibility
policy in [`docs/api-stability.md`](docs/api-stability.md).

## [Unreleased]

### Changed

- [**breaking**] Upgrade PodmanLens to 0.2.1, separate finite Podman 3.0.1–6.1 input capability
  evidence from the 5.4.0–6.1.0 output catalogue, add rootless-first local socket discovery,
  optional application-name inference, literal prefix selectors, causal diagnostics, and
  opt-in always-redacted Podman support snapshots. Empty user overrides and ID-derived hostnames no
  longer become authored intent, while Podman planning/rendering failures retain their exact
  resource and field context ([#65](https://github.com/Strukturpiloten/boxferry/issues/65)).
- Add digest-pinned live Podman conformance for all 48 rootful/rootless container images, covering
  real workloads, selectors, all exporters, re-imports, fault responses, and redaction without a
  nightly schedule. Pull requests build BoxFerry once, then run a deadline-bounded nine-cell
  compatibility matrix with timestamped operation progress and one representative live socket
  discovery check. External apply targets are isolated per source cell. Five published
  UBI/openSUSE rootless images report a verified helper-privilege limitation instead of overstating
  live-resource coverage ([#65](https://github.com/Strukturpiloten/boxferry/issues/65)).

### Fixed

- Restrict newly created support-report directories and archives to the invoking user.

## [0.8.0](https://github.com/Strukturpiloten/boxferry/compare/boxferry-v0.7.1...boxferry-v0.8.0) - 2026-08-24

### Added

- [**breaking**] rename Podman command artifact ([#62](https://github.com/Strukturpiloten/boxferry/pull/62))

### Changed

- [**breaking**] Rename the generated Podman command artifact from `review.sh` to
  `podman-commands.sh` and the public Rust accessor from `review_shell` to `commands_shell`, while
  clarifying that BoxFerry never executes the runnable script
  ([#61](https://github.com/Strukturpiloten/boxferry/issues/61)).
- Reorganize current documentation around task-oriented entry points and canonical sources, move
  the Podman adapter policy into BoxFerry, separate active decisions from superseded history, and
  prevent duplicate planning and API ledgers from returning
  ([#57](https://github.com/Strukturpiloten/boxferry/pull/57)).
- Use Cargo's conventional repository-local `target` directory for Dev Container builds instead
  of a hidden volume-backed target directory
  ([#63](https://github.com/Strukturpiloten/boxferry/issues/63)).

## [0.7.1](https://github.com/Strukturpiloten/boxferry/compare/boxferry-v0.7.0...boxferry-v0.7.1) - 2026-08-22

### Changed

- Make every public Podman conversion route executable and corpus-backed, including all-exporter
  re-import and deterministic fixed-point coverage ([#51](https://github.com/Strukturpiloten/boxferry/pull/51)).

## [0.7.0](https://github.com/Strukturpiloten/boxferry/compare/boxferry-v0.6.0...boxferry-v0.7.0) - 2026-08-22

### Added

- [**breaking**] integrate PodmanLens-backed Podman routes ([#48](https://github.com/Strukturpiloten/boxferry/pull/48))

### Changed

- [**breaking**] complete the Compose and Quadlet document milestone ([#21](https://github.com/Strukturpiloten/boxferry/pull/21))

## [0.6.0](https://github.com/Strukturpiloten/boxferry/compare/boxferry-v0.5.1...boxferry-v0.6.0) - 2026-08-22

### Changed

- [**breaking**] complete the neutral route scenario matrix ([#46](https://github.com/Strukturpiloten/boxferry/pull/46))
- [**breaking**] enforce the neutral Compose conversion pipeline ([#43](https://github.com/Strukturpiloten/boxferry/pull/43))

## [0.5.1](https://github.com/Strukturpiloten/boxferry/compare/boxferry-v0.5.0...boxferry-v0.5.1) - 2026-08-21

### Changed

- Add PodmanLens to the shared five-repository development workspace and verify its Dev Container
  mount ([#37](https://github.com/Strukturpiloten/boxferry/pull/37)).

## [0.5.0](https://github.com/Strukturpiloten/boxferry/compare/boxferry-v0.4.0...boxferry-v0.5.0) - 2026-08-19

### Added

- [**breaking**] publish executable documentation and canonical YAML ([#34](https://github.com/Strukturpiloten/boxferry/pull/34))

## [0.4.0](https://github.com/Strukturpiloten/boxferry/compare/boxferry-v0.3.0...boxferry-v0.4.0) - 2026-08-18

### Changed

- [**breaking**] complete the Compose and Quadlet document milestone ([#21](https://github.com/Strukturpiloten/boxferry/pull/21))

### Fixed

- *(ci)* [**breaking**] accept release-plz changelog layout ([#28](https://github.com/Strukturpiloten/boxferry/pull/28))
- *(ci)* [**breaking**] avoid duplicate pull-request validation ([#26](https://github.com/Strukturpiloten/boxferry/pull/26))
- *(release)* [**breaking**] preserve lockstep release preparation ([#23](https://github.com/Strukturpiloten/boxferry/pull/23))

## [0.3.0] - 2026-08-17

### Changed

- Automates future version and changelog preparation with release-plz, makes this changelog the
  sole release-history source, and retains the protected trusted-publishing workflow as publisher.
- Updates the native adapters to ComposeLens 0.2.0 and QuadletLens 0.2.0; all supported
  BoxFerry crates move together to 0.3.0 for their intentional pre-1.0 API changes.
- Rejects obsolete Compose `external: {name: ...}` resource mappings. Use `external: true` with a
  top-level `name`; BoxFerry no longer preserves the obsolete runtime-name form.
- Generates application-owned file-backed top-level Compose configs and secrets through
  ComposeLens 0.2.0, with parse-back validation and protected-path redaction.
- Uses QuadletLens's `model::SystemdUnitKey` ownership path and retains only reviewed
  `Requires`/`Wants` plus `After` neutral dependency semantics.
- Quadlet semantic `Environment=` decoding maps protected literal assignments and reports remaining
  forms. Every entry in `.kube` and `.artifact` documents is reported individually until it has a
  neutral mapping; a systemd-version selector is deferred until it affects a supported capability.
- Replaces `NetworkAttachment::new(Identifier, Vec<String>)` with provenance-bearing
  `NetworkAttachment::new(Identifier, Vec<Sourced<ProtectedString>>)`. Each alias now retains
  its source and sensitivity; no compatibility constructor remains.
- Extends the source-reviewed Podman inspect decoder ceiling from 6.0.2 to 6.1.0; reproducible
  live-runtime evidence remains capped at the available 5.8.2 image.

## [0.2.0] - 2026-08-13

### Added

- Adds nested Compose/Quadlet `convert` and `validate` routes, same-format canonicalization,
  ordered input discovery, and explicit interpolation inputs.
- Adds typed diagnostic rules, grouped remediation, JSON reports, and privacy-safe local error
  report archives.
- Expands loss-aware mappings for lifecycle, dependency, environment-file, identity, security,
  topology, image-build, network, volume, configuration, and secret intent.

### Changed

- Updates the published ComposeLens dependency to 0.1.17 and QuadletLens to 0.1.13.
- Replaces the aggregate
  `QuadletSource::parse -> Result<QuadletSource, QuadletSourceError>` contract. The sole parser
  returns `QuadletParseResult` or `QuadletParseError` and retains recoverable native diagnostics.
- Replaces the pair-specific and flat conversion syntax with
  `boxferry convert <INPUT_TYPE> <OUTPUT_TYPE>`; removed CLI forms have no compatibility aliases.
- Supports the CLI on Linux; Windows users run it in WSL2. Native Windows binaries and Windows
  containers are outside the supported platform contract.

## [0.1.1] - 2026-08-10

### Added

- Publishes the facade and seven component crates as the first crates.io release.
- Provides loss-aware Compose, Quadlet, Docker, and Podman import/export foundations through one
  neutral model and public conversion engine.
- Adds ordered trusted publishing, package attestations, checksums, and per-crate SemVer checks.

## [0.1.0] - 2026-08-10

### Added

- Established the initial repository implementation and documentation before crates.io
  publication.
