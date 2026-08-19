# Platform support

The BoxFerry CLI is supported on Linux.

Windows users must install and run the Linux CLI inside WSL2. Native Windows binaries, Windows
containers, and Windows host-path semantics are outside the product contract.

macOS CI checks deterministic POSIX behavior. It does not claim native systemd, Quadlet, or
container-runtime support.

Component crates may compile elsewhere.

Such compilation is incidental unless that platform appears in the supported CI matrix.
