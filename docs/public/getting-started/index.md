# Getting started

This guide installs the CLI and converts one Compose service into a Quadlet container unit.

## Install

BoxFerry runs on Linux and macOS. On Windows, use the Linux CLI in WSL2. macOS Podman input
requires an explicit socket path; native systemd and runtime behavior are not claimed.

```console
cargo install boxferry --locked
```

Alternatively, when a release includes binary assets, download the archive for your platform from
the [GitHub release](https://github.com/Strukturpiloten/boxferry/releases), together with its
`SHA256SUMS`. For version `0.10.0`, the archive names are:

- `boxferry-0.10.0-x86_64-unknown-linux-musl.tar.gz`
- `boxferry-0.10.0-aarch64-unknown-linux-musl.tar.gz`
- `boxferry-0.10.0-i686-unknown-linux-musl.tar.gz`
- `boxferry-0.10.0-armv7-unknown-linux-musleabihf.tar.gz`
- `boxferry-0.10.0-x86_64-apple-darwin.tar.gz`
- `boxferry-0.10.0-aarch64-apple-darwin.tar.gz`

Check the selected archive's SHA-256 against its entry in `SHA256SUMS` before extracting it;
Linux users can use `sha256sum`, and macOS users can use `shasum -a 256`. The archive contains
`boxferry`, `LICENSE`, `README.md`, and `BUILD-INFO.json` in a versioned root directory. Place
the extracted executable on your `PATH`. Select the version shown on the release page when
choosing a filename.

<!-- boxferry-example: version -->

```console
boxferry --version
```

## Create the input

Save this as `compose.yaml`:

```yaml
---
name: route-matrix
services:
  web:
    container_name: web-runtime
    image: example.invalid/web:1
    restart: "no"
```

The example image name is intentionally non-routable. BoxFerry converts definitions; it does not
pull or start images.

## Convert

<!-- boxferry-example: compose-to-quadlet -->

```console
boxferry convert compose quadlet --output-directory quadlet-output
```

Success ends with:

```text
boxferry: command succeeded; wrote 1 file(s) to output directory
```

The new directory contains:

```text
quadlet-output/
└── web.container
```

BoxFerry never starts the unit. Review the generated file before installing it.
With no input selector, BoxFerry reads the sole conventional Compose file in the current directory
and reports which file it selected. This imports an authored definition, not a deployed service.

The Docker guides describe Unreleased development functionality and may expose routes absent
from the installed release. They do not change this existing Compose-to-Quadlet walkthrough.

## Next

- [Use interpolation or inspect conversion failures](../guides/convert/compose-to-quadlet/)
- [Choose another route](../guides/)
- [Understand exact, approximate, and partial output](../concepts/)
