# ADR 0086: Downloadable CLI binaries

- Status: accepted
- Date: 2026-10-28
- Amends: [ADR 0024](0024-linux-cli-and-wsl-on-windows.md)

## Context

Installing the CLI should not require a Rust toolchain. Distribution needs exact-candidate
evidence without implying that client conversion establishes container-runtime support.

## Decision

Provide gated versioned archives for Linux musl x86_64, aarch64, i686 and armv7, and macOS
x86_64 and aarch64. Run the 64-bit candidates on native runners and the 32-bit Linux candidates
under explicit QEMU user-mode emulation. Package only after bounded CLI and conversion smoke
checks; verify the complete set against its revision and run before publication. Archives contain
the executable, license, README and build metadata. A unified checksum file and attestations
accompany the release assets.

macOS support is client-side: no native systemd, Quadlet generator or runtime support follows from
the smoke test. Podman input on macOS requires an explicit socket path. Windows remains supported
through WSL2, with no native Windows artifact. Hosted execution must still pass for each candidate;
this decision alone is not evidence of a successful release.

## Consequences

Users can select a portable Linux musl or macOS executable without compiling it. The release gate
adds platform-specific execution and archive integrity evidence; emulation does not prove native
32-bit runtime behavior. Developer ID signing and notarization are not implemented. QEMU is a
third-party verification tool and is not included in the archives.

## Alternatives considered

Source-only installation requires a toolchain from every CLI user. Cross-compilation without
executing the binaries would leave basic client behavior unverified. Native Windows distribution
would expand the platform contract beyond the WSL2 decision.
