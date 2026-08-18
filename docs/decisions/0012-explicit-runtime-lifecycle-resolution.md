# ADR 0012: explicit runtime lifecycle and Quadlet group resolution

- Status: superseded
- Date: 2026-08-04
- Superseded by: [ADR 0032](0032-future-native-lens-boundaries.md)

## Context

Runtime inspection establishes that networks, volumes, and Podman pods exist. It cannot prove
whether a reusable definition should create those resources, refer to externally managed
resources, or reproduce a pod as a shared-namespace target. Treating every observed resource as
application-owned would turn deployment-machine state into unreviewed author intent. Treating
everything as external would make generated definitions incomplete by default.

The neutral `ServiceGroup` deliberately records only structural membership. It does not assert
that members should share Linux namespaces or that a target should create a Podman pod.

## Decision

`RuntimeResolutions` is the only shared-reconstructor input that changes uncertain runtime
network, volume, or service-group lifecycle ownership. Each resolution:

- is keyed by one exact neutral resource name and kind;
- selects only `Application` or `External` ownership;
- carries at least one explicit `UserOverride` provenance origin; and
- cannot silently replace another resolution for the same resource.

`RuntimeImporter::with_resolutions` applies those finite decisions. The reconstructed resource
retains both runtime-observation and caller-override provenance. `BFR0009` reports the choice as
approximate because the caller resolved a fact that inspection itself could not prove. Missing
resolutions remain `Uncertain`; there is no blanket ownership default.

`QuadletGroupingPolicy::PreserveSingleGroup` consumes exactly one application-owned neutral group
only when it contains every application service exactly once. The existing network, alias, host
mapping, port, and user-namespace compatibility checks still apply. The generated `.pod` uses the
group name and every member container references that unit. Mapping structural membership to a
Podman shared namespace remains a `BFQ0007` approximation requiring explicit authorization.

Zero, multiple, external, uncertain, or partial groups are invalid for this policy. The exporter
does not guess whether to split, merge, flatten, or reference such groups.

## Consequences

- Runtime migration can generate application-owned or external resource intent without erasing
  the distinction between observation and decision.
- Embedded users can persist their own policy source identity and audit who selected lifecycle.
- The first preserved Quadlet group intentionally supports one complete application pod; richer
  multi-pod layouts require a separately designed neutral and target contract.
- `AllowApproximate` remains necessary for reviewed runtime reconstruction and preserved pod
  topology. Invalid evidence cannot be authorized through a more permissive loss policy.

## Alternatives considered

- **Default every observed resource to application-owned.** Rejected because it may recreate or
  take ownership of infrastructure the source application never managed.
- **Default every observed resource to external.** Rejected because it produces definitions that
  depend on undocumented pre-existing state.
- **Accept unproven `Application`/`External` values without provenance.** Rejected because the
  resulting model could not distinguish caller policy from runtime fact.
- **Automatically create one Quadlet pod for every observed group.** Rejected because partial and
  multiple groups require additional network, lifecycle, and ungrouped-service policy.
