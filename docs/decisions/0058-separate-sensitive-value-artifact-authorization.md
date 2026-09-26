# ADR 0058: Separate sensitive-value artifact authorization

- Status: accepted
- Date: 2026-09-26
- Amends: [ADR 0018](0018-generic-cli-and-diagnostic-support-bundle.md) and
  [ADR 0042](0042-bounded-podman-creation-evidence-and-intent-promotion.md)

## Context

The Podman portable-effective-settings flag previously enabled environment-value acquisition.
Ports, restart policy, and ordinary health settings do not authorize copying credentials into a
deployment artifact. Compose and Quadlet sources also contain literal environment values whose
names do not reliably reveal whether they are private. Reports and support bundles have a separate
redaction promise from generated deployment files.

## Decision

The CLI uses `--environment-values withhold|include`, defaulting to `withhold`. This policy is
independent of `--loss-policy` and Podman effective-setting promotion. It applies to conversion and
validation alike. All imported literal service environment assignments are protected regardless
of variable name. Withholding replaces the value with a neutral `Required` state while retaining
the name and source origins. Each exporter omits that assignment and records a named unsupported
target prerequisite. Exact and approximate policies block output; partial output is review
material that needs the value supplied before use.

Podman input acquires environment values only when inclusion and effective-setting promotion are
both selected. Otherwise PodmanLens retains a redacted state; the importer represents its name as
`Required` when the field is promoted. This does not infer secret delivery from inspection.
Explicit Compose `--env` and `--env-file` remain interpolation sources. No mode reads all process
environment values or an implicit `.env` file.

`include` authorizes literal environment values in Compose and Quadlet output. Generated files are
created with owner-only permissions on Unix; newly created directories use owner-only permissions.
The current PodmanLens renderer deliberately rejects protected inline environment values. Until
the native library supplies a safe protected-value artifact channel, Podman output with `include`
and a protected literal fails before writing under every loss policy. Casting protected values to
the native public type is prohibited because its debug representation exposes the value.

Terminal messages, JSON console output, report files, and support ZIPs remain redacted in both
modes. No artifact bytes are printed to the terminal. Environment-file contents, bind-path contents,
external resources, and application data migration remain separate decisions.
Generated files can contain authored command, health-check, image, and path text; this narrowly
named policy does not claim to classify or suppress those fields. Operators must review output.

## Consequences

Operators have an explicit, noninteractive choice when a generated artifact must contain values.
Default and partial artifacts can be incomplete; the diagnostic names each required assignment.
An authorized Podman artifact cannot include protected inline values until the reviewed native
renderer contract is available. Embedded importers and exporters retain their explicit Rust API
boundaries; the CLI applies its artifact policy to the imported neutral model before export.
