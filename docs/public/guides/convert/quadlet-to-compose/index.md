# Quadlet to Compose

Use this route to reconstruct one canonical Compose document from a Quadlet document set.

## Convert

Save this as `web.container`:

```ini
[Container]
Image=example.invalid/web:1
ContainerName=web-runtime

[Service]
Restart=no
```

<!-- boxferry-example: quadlet-to-compose -->

```console
boxferry convert quadlet compose --input-file web.container --application-name route-matrix --output-directory compose-output
```

The route writes:

```text
compose-output/
└── compose.yaml
```

```yaml
---
name: route-matrix
services:
  web:
    container_name: web-runtime
    image: example.invalid/web:1
    restart: "no"
```

`--application-name` is required because a Quadlet file set has no Compose project name. The
output targets the rolling Compose Specification, not an installed provider.

## Invalid Quadlet value

An empty `Image=` entry fails before output is created:

<!-- boxferry-example: quadlet-to-compose-invalid -->

```console
boxferry convert quadlet compose --input-file invalid.container --application-name docs-error --output-directory compose-error-output
```

[`BFQ1102`](../../../reference/diagnostics/) points to the native value that must be
corrected.

## Useful options

- Repeat `--input-file` or add `--input-directory DIR` for a document set.
- Use `--loss-policy approximate` or `partial` only after reviewing the reported rule.
- Compose interpolation options do not apply to Quadlet input.
