# Compose to Quadlet

Use this route to create canonical Quadlet files for an explicit Podman version range.

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

## Interpolate before converting

Given `compose-interpolation.yaml` with `image: ${IMAGE}`, and `variables.env` containing
`IMAGE=example.invalid/web:2`:

<!-- boxferry-example: compose-to-quadlet-interpolate -->

```console
boxferry convert compose quadlet --input-file compose-interpolation.yaml --interpolate --env-file variables.env --env RESTART=no --output-directory quadlet-interpolated-output
```

`--env-file` values are applied in order. Later `--env` values win. Sensitive values should use
`--env NAME`, which reads only that authorized process variable and redacts it from reports.

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
