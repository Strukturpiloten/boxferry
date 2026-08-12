# Changelog

All notable BoxFerry changes are recorded here. The project follows the pre-1.0 compatibility
policy in [`docs/api-stability.md`](docs/api-stability.md).

## [Unreleased]

### Changed

- The next lockstep version, 0.2.0, replaces the aggregate
  `QuadletSource::parse -> Result<QuadletSource, QuadletSourceError>` contract. The sole parser
  returns `QuadletParseResult` or `QuadletParseError` and retains recoverable native diagnostics.

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
