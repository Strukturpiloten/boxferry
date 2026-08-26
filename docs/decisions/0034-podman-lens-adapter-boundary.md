# ADR 0034: PodmanLens adapter and inert deployment boundary

- Status: accepted
- Date: 2026-08-22
- Amends: [ADR 0026](0026-typed-diagnostic-rule-catalogue.md),
  [ADR 0027](0027-format-neutral-native-findings.md),
  [ADR 0032](0032-future-native-lens-boundaries.md), and
  [ADR 0033](0033-universal-neutral-model-pipeline.md)
- Amended by: [ADR 0035](0035-explicit-podman-command-artifact.md),
  [ADR 0036](0036-local-podman-cli-discovery-and-selectors.md), and
  [ADR 0037](0037-finite-podman-input-and-live-conformance.md)

## Context

ADR 0033 made Podman the next native integration after PodmanLens established a
reviewed input and output contract. BoxFerry now needs a precise dependency,
acquisition, promotion, target-selection, diagnostic, and artifact boundary.
Without that boundary, runtime-effective or locally resolved observations could
silently become portable intent, target behavior could depend on the development
machine, and a review artifact could be mistaken for either an observed Podman
inventory or an executable deployment facility.

PodmanLens 0.2.1 exposes explicit read-only acquisition, resource discovery,
typed observation origins and states, semantic deployment planning, and
deterministic rendering. It does not choose BoxFerry mappings, execute rendered
operations, or provide a supported deserializer for an inventory snapshot.

## Decision

1. `boxferry-podman` depends on the crates.io release `podman-lens` 0.2.1.
   PodmanLens owns Libpod protocol decoding, read-only transport, resource
   inventory and graph types, native version evidence, deployment planning, and
   deterministic Podman rendering. BoxFerry owns application selection,
   observation-promotion policy, neutral-model mapping, loss authorization,
   orchestration, reports, CLI presentation, and output-directory safety.
   PodmanLens does not depend on BoxFerry.
2. Podman input acquisition is always explicit and read-only. The public
   BoxFerry library API used by the CLI accepts a caller-selected transport and
   discovery request, calls `acquire_inventory`, then `discover`, and passes
   the resulting `ResourceInventory` and `ResourceGraph` to
   `PodmanImporter`. BoxFerry does not discover ambient Podman connections,
   read Podman connection configuration or environment variables, shell out to
   `podman`, deserialize a PodmanLens snapshot as live input, or send a
   mutating Libpod request. The first CLI transport is an explicit local Unix
   socket; remote SSH or mutual-TLS transports remain caller-owned library
   integrations.
3. `PodmanImporter` handles every selected resource observation and every
   modeled `ObservationField` state. `Observed(Configured)` values are
   promoted only for documented exact mappings. `Observed(Effective)` values
   require an explicit promotion policy and retain conversion-decision
   provenance. `Observed(RuntimeAssigned)` values never become desired state
   automatically. `Observed(LocalResolution)` values require an explicit
   target-local decision. Absent values remain absent; unavailable, malformed,
   version-inapplicable, not-applicable, and unmodelled values produce
   structured outcomes or diagnostics appropriate to their state. Native
   stable IDs remain correlation evidence. Neutral-name collisions and
   ambiguous references are invalid.
4. Runtime resources begin with uncertain ownership. Inspected secrets remain
   external or uncertain unless separately authored material exists. Inspection
   cannot reconstruct a secret grant because it does not contain the delivery
   form and target; the adapter therefore never invents either detail.
5. Podman output maps neutral `Application` intent to PodmanLens
   `DeploymentIntent`, calls `plan_deployment`, retains every planning and
   rendering finding, and renders only a complete plan. The result contains the
   deterministic, versioned `podman.json` deployment-v1 artifact and
   `review.sh` review script. Both are inert. BoxFerry exposes no execute,
   apply, deploy, or mutating Podman operation.
6. Podman output selects one exact reviewed engine/API target from this finite
   catalogue: 5.4.0, 5.5.0, 5.6.0, 5.7.0, 5.8.6, 6.0.0, and 6.1.0. The default
   is 6.1.0. `--podman-max-version` resolves to the newest reviewed exact
   target not greater than the requested ceiling and fails closed below the
   catalogue. The target execution context is an explicit rootful, rootless, or
   unknown choice. It is never inferred from the inspected source or
   development host.
7. BoxFerry owns `BFP` rule identities for Podman semantic mapping and caller
   action: malformed or incomplete source, unmodelled observations, required
   promotion or local-resolution decisions, identity/reference conflicts,
   incomplete secret grants, invalid target selection, unsupported neutral
   intent, and planning or rendering findings. PodmanLens native diagnostic
   identifiers remain `NativeFinding::source_code` provenance and never become
   BoxFerry rule codes. Existing privacy and redaction rules apply to both.
8. Podman output is not Podman input. `podman.json` describes a desired
   deployment plan, while Podman input is an acquired runtime inventory and
   graph. BoxFerry therefore does not re-import Podman output or claim a Podman
   fixed point. Chained re-import and fixed-point assertions apply only to
   generated Compose and Quadlet artifacts. Podman output tests assert
   deterministic bytes, semantic operation equivalence, findings, and
   redaction.

## Consequences

- All nine Compose, Quadlet, and Podman routes retain the same
  importer-to-neutral-`Application`-to-exporter pipeline.
- Podman conversion is reproducible without a local Podman installation for
  output and without live version containers for the offline fixture matrix.
- Runtime inspection remains evidence rather than proof of portable authored
  intent. Some useful effective or local values require explicit policy or
  remain reported losses.
- Podman deployment artifacts can be reviewed and handed to an external system,
  but BoxFerry never executes them.
- A future Podman output importer would need a new native contract and decision;
  it cannot treat deployment-v1 as an inspected inventory.

## Alternatives considered

### Read ambient Podman state or invoke the CLI

Rejected because it makes acquisition host-dependent, bypasses PodmanLens's
bounded read-only protocol contract, and prevents deterministic library use.

### Promote every effective runtime value

Rejected because inspected defaults, runtime assignments, and local resolution
do not prove portable authored intent.

### Treat the deployment artifact as a Podman snapshot

Rejected because desired operations and observed inventory have different
semantics, completeness, provenance, and safety boundaries.

### Execute rendered operations from BoxFerry

Rejected because conversion planning and infrastructure mutation require
different authorization, recovery, and operational ownership.
