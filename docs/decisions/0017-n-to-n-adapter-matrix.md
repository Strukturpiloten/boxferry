# ADR 0017: N-to-N adapter matrix and explicit runtime deployment plans

- Status: accepted
- Date: 2026-08-05

## Context

BoxFerry's first implementation route was Docker Compose to Podman Quadlet. Runtime inspection and
Compose generation were added afterward, but project language and CLI structure could still be
read as a collection of privileged pairwise migrations. That interpretation does not match the
product goal.

Docker runtime resources, Docker Compose, Podman runtime resources, Podman Quadlet, and later
Kubernetes are all native views of overlapping application intent. Users must be able to select
any supported view as the source and any supported view as the target. Some native semantics have
no equivalent in another target, so route completeness cannot mean silent one-to-one parity.

Runtime targets also differ from document targets. Producing Compose YAML or Quadlet files is a
pure output operation. Creating Docker or Podman resources changes external state and requires a
review and authorization boundary.

## Decision

1. Every supported integration is designed as an independent importer and exporter around the
   neutral application model.
2. BoxFerry composes importers and exporters through the shared conversion engine. It does not
   implement or maintain a separate converter for every ordered source/target pair.
3. The first major milestone requires Docker runtime resources, Docker Compose, Podman runtime
   resources, and Podman Quadlet as both sources and targets. Kubernetes later joins the same
   matrix rather than creating a separate conversion subsystem.
4. A route supports only the intersection of semantics represented by its importer, the neutral
   model, and its exporter. Every approximation, unsupported value, invalid value, and manual
   action participates in the same structured loss policy.
5. Docker and Podman runtime exporters produce deterministic, reviewable deployment plans. An
   executor applies an authorized plan as a separate explicit operation; conversion alone never
   mutates an ambient runtime.
6. Runtime acquisition and application require explicit endpoints and resource selections. Empty
   selection never means enumerate or modify every resource.
7. The CLI evolves toward one generic conversion route with explicit source and target kinds.
   Source- and target-specific options remain typed, but conversion rules stay in public adapters.
8. Offline contract tests cover every source/target combination over one shared supported subset.
   Native conformance tests remain evidence for the individual importer and exporter boundaries.

## Consequences

- Adding one importer makes that source available to every existing exporter.
- Adding one exporter makes that target available to every existing importer.
- Route coverage can be explained as adapter and semantic coverage instead of an unmaintainable
  list of special-case converters.
- Docker-to-Docker or Quadlet-to-Quadlet remains useful for normalization, version migration, and
  diagnostics even when no format change occurs.
- Runtime writes require additional planning, conflict, idempotency, rollback, and executor
  contracts beyond the existing read-only inspection adapters.
- The current `compose-to-quadlet` command remains a valid first route but is not the final CLI
  architecture.

## Alternatives considered

### One command and implementation for every pair

Rejected because four boundaries already produce sixteen ordered routes, and Kubernetes would
increase that count further. Pair-specific logic would drift and report the same incompatibility
differently depending on the chosen route.

### Treat Docker and Podman only as input sources

Rejected because the product goal includes converting definitions back into deployed runtime
resources. Runtime targets need stronger safety boundaries, not exclusion from the matrix.

### Apply runtime changes directly during conversion

Rejected because conversion and external mutation have different authorization, recovery, and
testing requirements. A reviewable deployment plan keeps the conversion engine deterministic and
embeddable.
