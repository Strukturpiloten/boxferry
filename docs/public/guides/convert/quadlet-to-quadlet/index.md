# Quadlet to Quadlet

Use this route to parse, validate, and rebuild Quadlet files through BoxFerry's neutral model. It
is not a byte-preserving formatter.

## Prerequisites

- One or more related Quadlet files and an explicit application name.
- The complete target Podman version range.
- An absent or empty output directory.

## Convert

Save this as `web.container`:

```ini
[Container]
Image=example.invalid/web:1
ContainerName=web-runtime

[Service]
Restart=no
```

<!-- boxferry-example: quadlet-to-quadlet -->

```console
boxferry convert quadlet quadlet --input-file web.container --application-name route-matrix --output-directory quadlet-output
```

The output tree contains one canonical file:

```text
quadlet-output/
└── web.container
```

The default target covers Podman 5.4.0 through 6.1.0. Every emitted key must work across the whole
selected range.

Same-format conversion does not preserve native-only values silently. `EnvironmentFile=` paths,
bind mounts, secret references, pod grouping, and systemd relationships still pass through the
neutral model and normal loss policy.

## Invalid Quadlet value

An empty `Image=` entry is invalid for both parsing and reconstruction:

<!-- boxferry-example: quadlet-to-quadlet-invalid -->

```console
boxferry convert quadlet quadlet --input-file invalid.container --application-name docs-error --output-directory quadlet-error-output
```

[`BFQ1102`](../../../reference/diagnostics/) is the first rule to fix. Output is not
created.

## Useful options

- `--application-name` assigns the neutral application identity.
- `--podman-minimum-version` and `--podman-maximum-version` select the complete target range.
- `--quadlet-grouping pod` requests one compatible pod and requires approximation authorization.
- Compose interpolation options do not apply to Quadlet input.

BoxFerry never installs, enables, or starts generated units. Copy the reviewed files through your
normal configuration-management process and verify target-host paths separately.

---

[← Quadlet to Podman](../quadlet-to-podman/) · [Conversion guides](../../)
