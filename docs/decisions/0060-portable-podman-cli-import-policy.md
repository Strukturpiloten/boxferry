# ADR 0060: Portable Podman CLI import with separate reconstruction and loss decisions

- Status: accepted
- Date: 2026-09-26
- Amends: [ADR 0034](0034-podman-lens-adapter-boundary.md),
  [ADR 0042](0042-bounded-podman-creation-evidence-and-intent-promotion.md), and
  [ADR 0054](0054-typed-podman-network-alias-promotion.md)
- Builds on: [ADR 0058](0058-separate-sensitive-value-artifact-authorization.md) and
  [ADR 0059](0059-podman-cli-evidence-based-application-selection.md)

## Context

The CLI already discovers a bounded local application, but a normal migration still required three
separate effective-setting promotion switches and then a loss-policy override for the same
reviewed reconstruction. The source snapshot and the target conversion are different decisions.
Podman inspection does not reveal whether an effective value was explicitly authored or supplied
by a default. Nor does permission to reconstruct one portable field authorize a target exporter to
change its behavior or omit another field.

## Decision

1. The Podman-input CLI selects `--podman-import-policy portable` by default. It enables the
   reviewed effective settings, named-volume mounts, and named-network relationships together.
   `--podman-import-policy conservative` leaves those effective fields as evidence unless an
   expert enables individual promotion switches. The library's default policy remains
   conservative. Explicitly selecting the same granular library policy has the same semantic
   outcome as the CLI preset.
2. Reconstruction of a reviewed effective snapshot without a known behavior change has an exact
   source-fidelity outcome and an independent `BFP0009` note. The note names the subject, effective
   observation origin, reconstruction decision, and unresolved authored-versus-default distinction.
   Neutral values and outcomes retain both runtime-observation and conversion-decision provenance.
   This treatment is restricted to complete reviewed published-port bindings, supported restart
   behavior, normal non-shell health behavior, named-volume mount relationships, and named-network
   attachment relationships. A partially decoded field, shell health normalization, or an
   unverified retry limit remains non-exact. This source decision does not change target-exporter
   fidelity; an exporter approximation still blocks under exact loss policy.
3. The shared `--loss-policy exact|approximate|partial` default remains `exact` for every route.
   Missing or unsupported intent needs an explicit loss-policy decision and remains visible.
   Invalid source data never becomes valid through loss policy. Effective environment, DNS, network
   definition/IPAM, aliases, and inferred resource ownership retain their existing non-exact
   decisions; they are not covered by the narrow reconstruction rule.
4. Shared prerequisites and stopped shared boundaries remain external. Runtime-assigned ports and
   addresses, arbitrary host bind paths, and guessed image/build provenance never become portable
   intent through the default. Host bind promotion remains a separate reviewed same-path choice.
   Environment values remain withheld unless separately authorized with `--environment-values
include`; the Podman renderer's protected-value limitation remains fail-closed.
5. The CLI records the selected import policy and effective promotion choices in reports. Human
   and JSON output preserve diagnostics; BoxFerry still only reads the runtime and never executes
   generated output. Reviewable output does not copy volume data or application state.

## Consequences

A single unambiguous local application can be validated or converted with fewer options when its
reviewed fields are portable. An actual target difference or unsupported field still requires an
explicit loss-policy choice. Operators can choose conservative source handling without changing
the loss-policy vocabulary used by Compose and Quadlet routes.
