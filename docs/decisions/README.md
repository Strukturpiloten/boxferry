# Architecture decision records

Architecture decision records capture choices that constrain future development.

## Status values

- `proposed` — under discussion
- `accepted` — current project direction
- `superseded` — replaced by another ADR
- `rejected` — considered but not adopted

## Index

| ADR | Status | Decision |
| --- | --- | --- |
| [0001](0001-project-boundaries-and-origin.md) | accepted | Independent repositories, dependency direction, and from-scratch implementation |
| [0002](0002-public-library-facade.md) | accepted | Public library facade, reusable component crates, and CLI parity |
| [0003](0003-explicit-runtime-observation-provenance.md) | accepted | Runtime observations remain distinct from authored intent and conversion decisions |
| [0004](0004-first-cli-feature-and-write-safety.md) | accepted | Useful default CLI features with fail-closed, non-overwriting output |
| [0005](0005-shared-runtime-observation-layer.md) | accepted | Pure shared runtime observations with separate Docker and Podman native adapters |
| [0006](0006-finite-podman-inspect-decoder.md) | accepted | Finite Podman inspect decoding with separately replaceable resource acquisition |
| [0007](0007-finite-podman-relationship-expansion.md) | accepted | Finite policy-controlled Podman relationship expansion without ambient enumeration |
| [0008](0008-isolated-podman-runtime-conformance.md) | accepted | Digest-pinned nested Podman conformance without a host runtime socket |
| [0009](0009-versioned-docker-inspection.md) | accepted | Docker inspection follows an explicit Engine API version and daemon endpoint |
| [0010](0010-isolated-docker-runtime-conformance.md) | accepted | Digest-pinned nested Docker conformance without a host runtime socket |
| [0011](0011-neutral-service-group-relationships.md) | accepted | Structural service grouping without inferred namespace or lifecycle semantics |
| [0012](0012-explicit-runtime-lifecycle-resolution.md) | accepted | Provenance-bearing runtime lifecycle choices and one preserved Quadlet group |
| [0013](0013-explicit-compose-provider-and-runtime.md) | accepted | Provider-aware embedded Compose targets and generated output |
| [0014](0014-runtime-regular-health-observations.md) | accepted | Regular runtime health inference without conflating Podman startup health |
| [0015](0015-runtime-container-restart-policy.md) | accepted | Container restart observations with conservative systemd approximation |
| [0016](0016-runtime-metadata-label-reconstruction.md) | accepted | Protected runtime metadata labels with image-default and provider-metadata boundaries |
| [0017](0017-n-to-n-adapter-matrix.md) | accepted | N-to-N import/export adapters with reviewable runtime deployment plans |
| [0018](0018-generic-cli-and-diagnostic-support-bundle.md) | accepted | Generic CLI migration and privacy-safe local diagnostic support bundle |
| [0019](0019-generic-cli-route-registry.md) | accepted | Extensible CLI route registry with Compose and Quadlet document routes |
| [0020](0020-rolling-compose-specification-cli-target.md) | accepted | Rolling Compose Specification target and detailed Quadlet CLI diagnostics |
| [0021](0021-automatic-local-error-report-names.md) | accepted | Automatic local diagnostic report names and publication |

## Adding an ADR

Use the next four-digit number. Include context, decision, consequences, and alternatives. Do not rewrite an accepted ADR to hide a changed decision; add a new ADR that supersedes it.
