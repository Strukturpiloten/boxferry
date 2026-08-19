# Getting started

This guide installs the Linux CLI and converts one Compose service into a Quadlet container unit.

## Install

BoxFerry requires Linux. On Windows, use WSL2.

```console
cargo install boxferry --locked
```

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
boxferry convert compose quadlet --input-file compose.yaml --output-directory quadlet-output
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

## Next

- [Use interpolation or inspect conversion failures](../guides/convert/compose-to-quadlet/)
- [Choose another route](../guides/)
- [Understand exact, approximate, and partial output](../concepts/)
