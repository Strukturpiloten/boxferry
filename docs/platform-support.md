# Platform support

| Platform | Contract                                                                                                                                                    |
| -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Linux    | The BoxFerry CLI is supported on Linux.                                                                                                                     |
| Windows  | Windows users must install and run the Linux CLI inside WSL2. Native Windows binaries, Windows containers, and Windows host-path semantics are unsupported. |
| macOS    | CI checks deterministic POSIX behavior; native systemd, Quadlet, and container-runtime behavior is not claimed.                                             |

Component crates may compile elsewhere.
Such compilation is incidental unless that platform appears in the supported CI matrix.

Podman limitation revalidation runs nested distribution userspace on an Ubuntu GitHub-hosted amd64
runner. It establishes exact Podman/API/package behavior for the candidate image, not an independent
kernel or cgroup-delegation environment. Evidence records those as shared-host boundaries;
SELinux-enforcing behavior and systemd/Quadlet generator execution are explicitly unperformed.
