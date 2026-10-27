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
boxferry convert compose quadlet --output-directory quadlet-output
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
boxferry convert compose quadlet --input-file compose-interpolation.yaml --env-file variables.env --env LOG_LEVEL=warning --environment-values include --loss-policy approximate --output-directory quadlet-interpolated-output
```

`--env-file` values are applied in order. Later `--env` values win. The explicit
`--environment-values include` authorizes `LOG_LEVEL` in the generated unit; without it the
value is withheld and reported as a target prerequisite. Sensitive values should use
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
appears in `fix first`. Provide the value with `--env` or `--env-file`, or
use `--interpolate` to select a process-environment fallback.

## Defer explicit values to the target service

Use `compose-deferred.yaml` when the target service must supply deployment values:

```yaml
---
name: deferred-example
services:
  web:
    image: example.invalid/web:1
    restart: "no"
    environment:
      APP_MODE: "${RUN_MODE}"
    volumes:
      - "${DATA_ROOT}/config:/config:ro"
```

Declare each reference independently of its provider. The external file need not exist on the
conversion machine; BoxFerry never reads, copies or creates it.

<!-- boxferry-example: compose-to-quadlet-deferred-files -->

```console
boxferry convert compose quadlet --input-file compose-deferred.yaml --quadlet-defer web:RUN_MODE --quadlet-defer web:DATA_ROOT --quadlet-service-env-file web=/etc/boxferry/example.env --output-directory quadlet-deferred-output
```

The container retains `APP_MODE=${RUN_MODE}` and `${DATA_ROOT}/config:/config:ro`. The provider
is a separate `[Service] EnvironmentFile` declaration. Provider variables are not automatically
forwarded into the container. `BFQ0015` identifies these requirements as **external-unverified**;
exact conversion does not prove that files exist or that their values work on the target.

Alternatively, authorize protected inline service providers explicitly:

<!-- boxferry-example: compose-to-quadlet-deferred-inline -->

```console
boxferry convert compose quadlet --input-file compose-deferred.yaml --quadlet-defer web:RUN_MODE --quadlet-defer web:DATA_ROOT --quadlet-service-env web:RUN_MODE=production --quadlet-service-env web:DATA_ROOT=/srv/example --environment-values include --output-directory quadlet-deferred-inline-output
```

Inline assignments use `[Service] Environment`. All external files override inline assignments;
later files override earlier files, including empty overrides. Assignment values are literal:
`$OTHER` inside a provider value is not recursively expanded. Dollar and percent characters in
provider paths are also literal. Files must have absolute paths without glob activation.

Pure reference environment expressions work with the default `--environment-values withhold`.
Any nonempty literal part, such as `prefix-${RUN_MODE}`, and every inline provider require
`include`, including an empty inline value. Generated output remains private on Unix; diagnostics,
reports and support ZIPs withhold provider values. Bind suffixes describe paths and need no
separate environment-value acquisition consent.

Only direct `$NAME`/`${NAME}` references in plain or double-quoted scalars are supported.
Defaults, alternatives, required operators, malformed/nested expressions, and dollar-bearing
single-quoted/block scalars require conversion-time resolution. Bind sources allow one leading
reference and an optional safe slash suffix; native `.volume`/`.artifact` suffix rewrites are
rejected. Targets/options, named volumes and other deferred fields remain unsupported. Effective
bind roots must be nonempty absolute paths without colons or control characters. Inline-only
known-invalid roots fail during conversion; file overrides remain external requirements. Missing
or empty container environment references contribute empty strings.

Unused references, orphan providers, duplicate entries, unknown or inactive services and unbound
references fail. These flags cannot coexist with conversion-time `--env`, `--env-file`, or
`--interpolate`. `validate compose quadlet` checks the same plan without writing output. Bundles
preserve these declarations, but extraction does not migrate provider files or host data. Reverse
conversion reports unresolved execution references and providers as unsupported intent.

## Single-file transport

Use `--output-layout quadlet-bundle` to transport the complete generated unit set in one file.
[Extract the bundle](../../quadlet-bundles/) before native installation. Ordinary native files
remain the default, and grouping/loss policy are unchanged.

## Vendor extensions

Application and direct-service `x-*` metadata is retained as inert typed data in the original vendor's
`X-` section. Application metadata uses the lexicographically smallest generated unit filename as
its carrier; other units contain references. Service metadata stays on its `.container` unit.
Keep these files together for conversion back to Compose. Folded description strings retain the
folded presentation hint required by some catalogue validators. Metadata does not execute generators
or supply runtime settings. See [compatibility](../../../reference/compatibility/#vendor-metadata)
for supported shapes, limits and privacy behavior.

## Output policy

- The default target covers Podman 5.4.0 through 6.1.2.
- Change the range with `--podman-minimum-version` and `--podman-maximum-version`.
- Separate container units are exact by default. `--quadlet-grouping pod` is an approximation.
- `--loss-policy approximate` accepts documented approximations. `partial` also accepts documented
  omissions. Invalid input always fails.
- Relative bind mounts and `env_file:` paths still refer to target-host files. Conversion cannot
  copy their content or prove that a production host has the same paths.

---

[← Compose to Podman](../compose-to-podman/) · [Next: Podman to Compose →](../podman-to-compose/)
