# Compose to Compose

Use this route to merge and canonicalize Compose input. It is not a byte-for-byte copy.

## Canonicalize without interpolation

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

<!-- boxferry-example: compose-to-compose -->

```console
boxferry convert compose compose --input-file compose.yaml --output-directory compose-output
```

The route writes one file:

```text
compose-output/
└── compose.yaml
```

Compose expressions and extension fields remain native Compose data when interpolation is off.

## Interpolate explicitly

Save the following input as `compose-interpolation.yaml`:

```yaml
---
name: docs-interpolation
services:
  web:
    image: ${IMAGE}
    restart: ${RESTART:-no}
```

Save `IMAGE=example.invalid/web:2` as `variables.env`, then run:

<!-- boxferry-example: compose-to-compose-interpolate -->

```console
boxferry convert compose compose --input-file compose-interpolation.yaml --interpolate --env-file variables.env --env RESTART=no --output-directory compose-interpolated-output
```

The rendered service contains the resolved image and `restart: "no"`. BoxFerry reads no implicit
`.env` file and no process variable unless `--env NAME` authorizes that exact name.

## Missing required value

For `image: ${IMAGE:?set IMAGE before converting}`, this command fails before writing output:

<!-- boxferry-example: compose-to-compose-required-variable -->

```console
boxferry convert compose compose --input-file compose-required.yaml --interpolate --output-directory compose-error-output
```

[`BFC0102`](../../../reference/diagnostics/) tells you to provide the value with
`--env-file FILE` or `--env NAME=VALUE`.

## Useful options

- Repeat `--input-file` to set merge order.
- Use `--profile NAME` or `--all-profiles` for profiled services.
- Add `--project-directory DIR` when relative source paths use another project root.
- Keep the default `--loss-policy exact` unless a diagnostic explains an acceptable loss.
