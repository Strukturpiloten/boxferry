# Architecture decision records

ADRs explain durable constraints and why they were chosen. Current work should start with the
active table; superseded records are history and are read only when that history matters.

## Active decisions

| ADR                                                                   | Decision                                                                     |
| --------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| [0001](0001-project-boundaries-and-origin.md)                         | Repository boundaries, dependency direction, and from-scratch implementation |
| [0002](0002-public-library-facade.md)                                 | Public facade, reusable component crates, and CLI parity                     |
| [0004](0004-first-cli-feature-and-write-safety.md)                    | Useful default CLI with fail-closed, non-overwriting output                  |
| [0011](0011-neutral-service-group-relationships.md)                   | Structural service grouping without inferred runtime semantics               |
| [0013](0013-explicit-compose-provider-and-runtime.md)                 | Provider-aware embedded Compose targets                                      |
| [0018](0018-generic-cli-and-diagnostic-support-bundle.md)             | Generic CLI and privacy-safe local support bundle                            |
| [0019](0019-generic-cli-route-registry.md)                            | Extensible route registry                                                    |
| [0020](0020-rolling-compose-specification-cli-target.md)              | Rolling Compose target and detailed Quadlet diagnostics                      |
| [0021](0021-automatic-local-error-report-names.md)                    | Automatic local error-report names                                           |
| [0022](0022-sole-quadlet-parser-and-deterministic-test-contract.md)   | Quadlet parser and deterministic test ownership                              |
| [0024](0024-linux-cli-and-wsl-on-windows.md)                          | Linux CLI and WSL2 on Windows                                                |
| [0025](0025-empty-output-directories-and-final-human-status.md)       | Empty output directories and final status                                    |
| [0026](0026-typed-diagnostic-rule-catalogue.md)                       | Typed diagnostic rules and causal final status                               |
| [0027](0027-format-neutral-native-findings.md)                        | Format-neutral native findings                                               |
| [0028](0028-primary-remediation-guidance.md)                          | Structured primary remediation guidance                                      |
| [0029](0029-nested-input-output-cli-routes.md)                        | Nested input/output CLI routes                                               |
| [0031](0031-quadlet-systemd-environment-and-native-only-reporting.md) | Quadlet environment boundaries and native findings                           |
| [0032](0032-future-native-lens-boundaries.md)                         | Independent native Lens ownership                                            |
| [0033](0033-universal-neutral-model-pipeline.md)                      | Universal neutral pipeline and nonexecuting Podman-first direction           |
| [0034](0034-podman-lens-adapter-boundary.md)                          | PodmanLens adapter and inert output boundary                                 |
| [0035](0035-explicit-podman-command-artifact.md)                      | Explicit Podman command artifact and safety contract                         |
| [0036](0036-local-podman-cli-discovery-and-selectors.md)              | Deterministic local Podman CLI discovery and exact selectors                 |
| [0037](0037-finite-podman-input-and-live-conformance.md)              | Finite legacy Podman input, diagnostic snapshots, and live conformance       |
| [0038](0038-bounded-human-diagnostic-presentation.md)                 | Bounded human diagnostics with complete structured evidence                  |

## Superseded history

| ADR                                                         | Replaced decision                 |
| ----------------------------------------------------------- | --------------------------------- |
| [0003](0003-explicit-runtime-observation-provenance.md)     | Runtime observation provenance    |
| [0005](0005-shared-runtime-observation-layer.md)            | Shared runtime observation layer  |
| [0006](0006-finite-podman-inspect-decoder.md)               | In-repository Podman inspection   |
| [0007](0007-finite-podman-relationship-expansion.md)        | In-repository Podman discovery    |
| [0008](0008-isolated-podman-runtime-conformance.md)         | In-repository Podman conformance  |
| [0009](0009-versioned-docker-inspection.md)                 | Docker inspection                 |
| [0010](0010-isolated-docker-runtime-conformance.md)         | Docker conformance                |
| [0012](0012-explicit-runtime-lifecycle-resolution.md)       | Runtime lifecycle resolution      |
| [0014](0014-runtime-regular-health-observations.md)         | Runtime health observations       |
| [0015](0015-runtime-container-restart-policy.md)            | Runtime restart observations      |
| [0016](0016-runtime-metadata-label-reconstruction.md)       | Runtime label reconstruction      |
| [0017](0017-n-to-n-adapter-matrix.md)                       | Earlier adapter matrix            |
| [0023](0023-windows-local-time-zone-database.md)            | Native Windows time-zone database |
| [0030](0030-native-compose-same-format-canonicalization.md) | Same-format Compose shortcut      |

## Status and changes

Records use `proposed`, `accepted`, `superseded`, or `rejected`. Add the next four-digit
record with context, decision, consequences, and alternatives. Never rewrite an accepted record to
hide a changed decision; add a record that explicitly supersedes it.
