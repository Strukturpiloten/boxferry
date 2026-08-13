# Platform support

## Command-line application

The BoxFerry CLI is supported on Linux. Windows users must install and run the Linux CLI inside
WSL2. Native Windows binaries, Windows containers, PowerShell path semantics, drive-letter paths,
and UNC paths are outside the supported execution contract.

BoxFerry targets Linux container definitions and Linux facilities such as Podman Quadlet and
systemd. Docker Desktop and Podman on Windows already use a Linux environment through WSL2,
Hyper-V, or a managed virtual machine for Linux containers. Running BoxFerry in WSL2 keeps its
filesystem paths, shell behavior, and generated definitions in the same Linux environment.

Install WSL2 from an elevated Windows terminal:

```console
wsl --install
```

After opening the installed Linux distribution, install Rust and BoxFerry there:

```console
cargo install boxferry
boxferry version
```

Keep inputs and outputs in the WSL filesystem when possible. Translate a Windows path explicitly
with `wslpath` before passing it to BoxFerry; do not pass `C:\...` or UNC spellings to the CLI.

macOS remains a deterministic portability test platform because it provides POSIX filesystem and
process behavior. Container execution on macOS still uses a Linux virtual machine, and BoxFerry
does not claim native macOS container-runtime or systemd availability.

## Rust libraries

BoxFerry does not intentionally prevent its side-effect-free component libraries from compiling on
other targets. Such compilation is incidental unless that platform appears in the supported CI
matrix. The native `boxferry` executable fails to compile on Windows with a message directing the
user to WSL2.

## Input data

Rejecting native Windows execution does not remove diagnostics for Windows-authored path strings.
Adapters continue to treat drive-letter, UNC, tilde, and other host-specific paths as explicit
losses unless the caller supplies a reviewed target-path mapping.
