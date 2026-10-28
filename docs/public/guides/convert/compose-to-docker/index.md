# Compose to Docker

This route is Unreleased development functionality. Read the [Docker acquisition, consent and
artifact contract](../../docker-input/) and [exact profile list](../../../reference/compatibility/#docker-development-boundaries).
Installed releases may expose fewer routes; source support does not establish native consumer,
application migration or release qualification.

## Author the input

Save this as `compose.yaml`. The non-routable image is intentional; no image is pulled or started.

```yaml
---
name: route-matrix
services:
  web:
    container_name: web-runtime
    image: example.invalid/web:1
    restart: "no"
```

## Validate and convert

Choose the exact upstream rootful profile for this review. The source document does not choose it.
Validation runs the same import/export/loss decisions and writes no artifacts:

<!-- boxferry-example: compose-to-docker-validate -->

```console
boxferry validate compose docker --input-file compose.yaml --docker-target upstream-29.8.1-rootful
```

<!-- boxferry-example: compose-to-docker -->

```console
boxferry convert compose docker --input-file compose.yaml --docker-target upstream-29.8.1-rootful --output-directory docker-output
```

The absent or empty destination receives only `docker-plan.json` for this service, preserving its
image, runtime name and explicit no-restart policy. It contains inert native create intent.
No POST is sent. This plan has no dependency decisions, so no dependency sidecar is needed.

## Refusal without output

An unresolved image in `compose-unresolved.yaml` is invalid intent even with partial loss:

```yaml
---
name: docs-error
services:
  web:
    image: ${IMAGE}
```

<!-- boxferry-example: compose-to-docker-unresolved -->

```console
boxferry convert compose docker --input-file compose-unresolved.yaml --docker-target upstream-29.8.1-rootful --loss-policy partial --output-directory docker-error-output
```

The diagnostic identifies the unresolved image; the command leaves no output directory.

Required environment intent is also separate from loss policy. Use the interpolation input and
`variables.env` from the [Compose to Quadlet example](../compose-to-quadlet/#interpolate-production-values).
Without independent artifact-value inclusion, validation and conversion both refuse this Docker
candidate under partial loss:

<!-- boxferry-example: compose-to-docker-withheld-validate -->

```console
boxferry validate compose docker --input-file compose-interpolation.yaml --env-file variables.env --docker-target upstream-29.8.1-rootful --loss-policy partial
```

<!-- boxferry-example: compose-to-docker-withheld -->

```console
boxferry convert compose docker --input-file compose-interpolation.yaml --env-file variables.env --docker-target upstream-29.8.1-rootful --loss-policy partial --output-directory docker-private-output
```

Review the named findings and [independent consent controls](../../docker-input/#independent-decisions-and-privacy).
Artifact inclusion does not authorize protected process or health values. Diagnostics and reports
remain redacted. More permissive loss never guarantees that a secret can be dropped while a
complete service request is emitted.
