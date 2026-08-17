# Changelog

All notable BoxFerry changes are recorded here. The project follows the pre-1.0 compatibility
policy in [`docs/api-stability.md`](docs/api-stability.md).

## [Unreleased]

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
