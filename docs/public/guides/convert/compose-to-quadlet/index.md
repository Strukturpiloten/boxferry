# Compose to Quadlet

Use this route to create canonical Quadlet files for an explicit Podman version range.

## Prerequisites

- One or more Compose files with explicit interpolation values.
- The complete Podman version range that must accept the generated units.
- An absent or empty output directory.

## Convert literal input

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

<!-- boxferry-example: compose-to-quadlet -->

```console
boxferry convert compose quadlet --input-file compose.yaml --output-directory quadlet-output
```

The result is:

```text
quadlet-output/
└── web.container
```

```ini
[Container]
Image=example.invalid/web:1
ContainerName=web-runtime

[Service]
Restart=no
```

## Interpolate production values

Use this service in `compose-interpolation.yaml`:

```yaml
---
name: docs-interpolation
services:
  web:
    image: example.invalid/web:${IMAGE_TAG}
    environment:
      LOG_LEVEL: ${LOG_LEVEL:-info}
    restart: ${RESTART_POLICY:-unless-stopped}
```

Save `IMAGE_TAG=2026.08.24` and `RESTART_POLICY=always` in `variables.env`, then override the log
level for this deployment:

<!-- boxferry-example: compose-to-quadlet-interpolate -->

```console
boxferry convert compose quadlet --input-file compose-interpolation.yaml --interpolate --env-file variables.env --env LOG_LEVEL=warning --loss-policy approximate --output-directory quadlet-interpolated-output
```

`--env-file` values are applied in order. Later `--env` values win. Sensitive values should use
`--env NAME`, which reads only that authorized process variable and redacts it from reports.
This example explicitly accepts `BFQ0009`: systemd approximates the Compose runtime restart policy.

The generated unit contains:

```ini
[Container]
Image=example.invalid/web:2026.08.24
Environment=LOG_LEVEL=warning

[Service]
Restart=always
```

## Unresolved variable

Quadlet does not evaluate Compose expressions. This command fails if the image is `${IMAGE}`:

<!-- boxferry-example: compose-to-quadlet-unresolved-variable -->

```console
boxferry convert compose quadlet --input-file compose-unresolved.yaml --output-directory quadlet-error-output
```

[`BFC0105`](../../../reference/diagnostics/) identifies the Compose variable.
[`BFQ0014`](../../../reference/diagnostics/) blocks the unusable Quadlet value and
appears in `fix first`. Add `--interpolate` and provide the missing value.

## Output policy

- The default target covers Podman 5.4.0 through 6.0.2.
- Change the range with `--podman-minimum-version` and `--podman-maximum-version`.
- Separate container units are exact by default. `--quadlet-grouping pod` is an approximation.
- `--loss-policy approximate` accepts documented approximations. `partial` also accepts documented
  omissions. Invalid input always fails.
- Relative bind mounts and `env_file:` paths still refer to target-host files. Conversion cannot
  copy their content or prove that a production host has the same paths.

---

[← Compose to Podman](../compose-to-podman/) · [Next: Podman to Compose →](../podman-to-compose/)
