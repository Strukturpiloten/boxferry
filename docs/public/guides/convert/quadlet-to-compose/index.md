# Quadlet to Compose

Use this route to reconstruct one canonical Compose document from a Quadlet document set.

## Prerequisites

- One or more related Quadlet files.
- One regular Quadlet unit, or an explicit `--application-name` for a document set.
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

The command uses an explicit name to override the default. With only `web.container`, omitting
`--application-name` names the neutral application `web`; two or more files require an explicit
name. The output targets the rolling Compose Specification, not an installed provider. BoxFerry
does not run that provider or infer target-host paths, secrets, or environment files.

## Production checks

- `Environment=` values become protected service environment values. `EnvironmentFile=` paths are
  target-host dependencies and may require approximation.
- Bind mounts remain host-specific even when their syntax converts exactly.
- Secret references do not include secret material; provision it separately on the target.
- Pod and systemd relationships without a Compose equivalent remain visible as diagnostics.

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

---

[← Podman to Quadlet](../podman-to-quadlet/) · [Next: Quadlet to Podman →](../quadlet-to-podman/)
