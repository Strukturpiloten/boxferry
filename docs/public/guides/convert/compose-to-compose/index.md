# Compose to Compose

Merge and normalize Compose files through BoxFerry's neutral application model. The result is one
canonical Compose document, not a byte-for-byte copy.

## Prerequisites

- One or more Compose files.
- An absent or empty output directory.
- Explicit interpolation input when the source contains `${NAME}` expressions.

## Convert a literal document

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

Every value crosses the neutral application model. Unresolved typed expressions are invalid at that
boundary, and native-only extension fields require `--loss-policy partial` before BoxFerry can omit
them. No same-format shortcut preserves unsupported native data.

## Supply production values explicitly

Save the following input as `compose-interpolation.yaml`:

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

Save non-secret deployment defaults as `variables.env`:

```dotenv
IMAGE_TAG=2026.08.24
RESTART_POLICY=always
```

Override one value on the command line:

<!-- boxferry-example: compose-to-compose-interpolate -->

```console
boxferry convert compose compose --input-file compose-interpolation.yaml --interpolate --env-file variables.env --env LOG_LEVEL=warning --output-directory compose-interpolated-output
```

The rendered service contains image tag `2026.08.24`, `LOG_LEVEL=warning`, and `restart: always`.
Later `--env-file` inputs win over earlier files; `--env NAME=VALUE` wins over all files.

For a sensitive process value, prefer `--env NAME`. It authorizes only that named variable and
keeps its value out of diagnostics. BoxFerry reads no implicit `.env` file.

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

---

[← Conversion guides](../../) · [Next: Compose to Podman →](../compose-to-podman/)
