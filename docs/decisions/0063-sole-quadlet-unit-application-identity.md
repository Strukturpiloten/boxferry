# ADR 0063: Sole Quadlet unit application identity

- Status: accepted
- Date: 2026-09-26
- Supersedes: [ADR 0029](0029-nested-input-output-cli-routes.md) decision 4, only for the
  unconditional `--application-name` requirement

## Context

Quadlet unit files have no Compose project identity. Requiring an application name even for one
ordinary unit makes a simple conversion harder than necessary. A unit filename is a stable,
user-visible source of a neutral name when exactly one regular supported file is resolved. Native
references among two or more units do not establish that they belong to one application.

## Decision

1. Resolve Quadlet inputs first. If exactly one supported regular unit file is selected and no
   `--application-name` is supplied, use its extensionless filename as the neutral application
   name after validating it as a model `Identifier`.
2. For two or more files, require `--application-name` and report an actionable input-discovery
   failure when it is absent. An explicit name overrides inference for any document set. Quadlet
   stdin remains unsupported. Distinct selected paths with the same unit basename remain a
   collision even with an explicit application name; select one or rename a unit before combining
   them.
3. Apply the decision through the shared `convert` and `validate` path for every Quadlet output
   route. It changes only application identity selection. Native parsing, import through the
   neutral model, export, provenance, privacy, loss policy, and create-new output safety remain
   in force.
4. A source filename and source host do not select a target Podman version, target context, or
   target-host paths and secrets. Callers review those prerequisites separately.

## Consequences

- A sole `web.container` can be validated or converted without a naming option and yields neutral
  application `web`.
- A document set needs an explicit application identity even if its native references connect its
  units. No ownership is inferred from that connectivity.
- Callers can keep existing explicit names; they take precedence over the filename.
