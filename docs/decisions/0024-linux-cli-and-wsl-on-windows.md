# ADR 0024: Linux CLI and WSL on Windows

- Status: accepted
- Date: 2026-08-12
- Supersedes: [ADR 0023](0023-windows-local-time-zone-database.md)
- Amends: [ADR 0022](0022-sole-quadlet-parser-and-deterministic-test-contract.md)

## Context

BoxFerry converts definitions for Linux container environments and emits Linux-native Quadlet and
systemd configuration. Native Windows CI introduced separate path, filesystem, linking, and local
time-zone behavior without evidence of a Windows-container use case. Docker Desktop and Podman on
Windows already provide Linux execution through WSL2 or a managed virtual machine.

## Decision

1. The BoxFerry CLI supports Linux. Windows users run the Linux CLI inside WSL2.
2. Native Windows CLI compilation fails with a direct WSL2 instruction. Windows containers and
   Windows host-path semantics are outside the product contract.
3. Remove native Windows CI and retain the macOS deterministic portability lane.
4. Component library portability outside tested platforms is incidental, not a support promise.
5. Windows-authored path strings remain fallible input. Adapters keep rejecting or explicitly
   mapping them rather than interpreting them as native BoxFerry execution paths.
6. Remove the Windows-only bundled Jiff time-zone database introduced by ADR 0023.

## Consequences

The project no longer carries native Windows CLI workarounds or claims. Windows users have one
documented Linux path matching the generated artifacts and container runtimes. macOS testing still
catches useful POSIX portability defects, but it does not imply native systemd or container-engine
support.

## Alternatives considered

Maintaining a native Windows CLI was rejected because its ongoing filesystem and process behavior
cost is disproportionate to the current product scope. Removing all defensive handling of Windows
path strings was rejected because those strings can still occur in imported definitions and must
never be silently reinterpreted.
