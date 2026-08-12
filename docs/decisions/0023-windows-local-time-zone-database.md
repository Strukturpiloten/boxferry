# ADR 0023: Windows local time-zone database

- Status: superseded
- Superseded by: [ADR 0024](0024-linux-cli-and-wsl-on-windows.md)
- Date: 2026-08-12
- Amends: [ADR 0021](0021-automatic-local-error-report-names.md)

## Context

ADR 0021 required a fallible local wall clock for automatic diagnostic-report names. Its direct
Jiff dependency enabled `tz-system` but not the target-specific IANA database Jiff needs to map a
Windows time-zone name. As a result, a supported Windows CLI could fail before publishing a
report even when the local clock was otherwise available.

## Decision

Enable Jiff's `tzdb-bundle-platform` feature alongside `std` and `tz-system` at the CLI feature
boundary. It activates `jiff-tzdb-platform` and `jiff-tzdb` only for targets without a usable
system IANA database, including Windows. This supersedes ADR 0021 decision item 6; its exact Jiff
pin, fail-closed local-time behavior, and privacy boundary remain unchanged.

## Consequences

Windows CLI binaries carry the reviewed IANA time-zone data needed for system-zone mapping.
Linux builds do not gain that target-only database dependency. The `jiff-tzdb-platform` proxy and
`jiff-tzdb` use the workspace-allowed MIT license and support the Rust 1.85.0 MSRV.

## Alternatives considered

Using UTC would violate the local-name contract. Requiring `TZDIR` or a user-provided time zone
would make normal Windows report creation fail or depend on hidden local setup.
