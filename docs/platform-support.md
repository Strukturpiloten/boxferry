# Platform support

| Platform | Client contract                                                                                                                                                                                         |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Linux    | The BoxFerry CLI is supported on Linux. Release archives target musl on x86_64, aarch64, i686 and armv7.                                                                                                |
| macOS    | Release archives target x86_64 and aarch64 for client-side conversion. Native systemd, Quadlet generator and container-runtime behavior are not claimed. Podman input requires an explicit socket path. |
| Windows  | Windows users must install and run the Linux CLI inside WSL2. Native Windows binaries, Windows containers, and Windows host-path semantics are unsupported.                                             |

Each archive is conditional on its exact-revision release gates passing. The 64-bit Linux and
macOS binaries are exercised on their respective native runners; the 32-bit Linux binaries use
explicit QEMU user-mode execution. This client smoke evidence does not establish native runtime
or kernel behavior on those targets. Component crate compilation elsewhere is incidental.

Podman limitation revalidation runs nested distribution userspace on an Ubuntu GitHub-hosted amd64
runner. It establishes exact Podman/API/package behavior for the candidate image, not an independent
kernel or cgroup-delegation environment. Evidence records those as shared-host boundaries;
SELinux-enforcing behavior and systemd/Quadlet generator execution are explicitly unperformed.
