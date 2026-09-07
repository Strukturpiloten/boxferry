# ADR 0039: Independent migration scenarios

Status: accepted

Date: 2026-09-07

## Context

Golden artifacts, counts and self-consistent round trips cannot establish that
required application intent survived. Large-application runtime acceptance needs
one reusable contract without making native libraries depend on BoxFerry.

## Decision

BoxFerry owns test-only schema-1 scenario manifests and shared validators.
Manifests record application/component versions, image identities, native input
cases, deployment origin/root mode, route capabilities, test-data provenance,
license/privacy review, independent neutral expectations, prerequisites, and
exact approved loss tuples scoped to an explicit target. Every input case must
exercise every exporter reported by the executable registry.

Route expectations and actual observations are different objects. Native
validation, runtime probes and reimport are separate evidence dimensions.
An unperformed dimension requires an explicit not-applicable reason. Expected
rejection, unsupported environment and a known migration gap never count as
migration success. An offline scenario proves only its declared offline claims.
Diagnostics are matched by code and subject; losses by rule, subject, decision
and version scope. Unknown losses cannot hide behind an approved gap.

The initial authored fixture is deliberately synthetic and never pulled or
provisioned. Native parsing, independent neutral comparisons and six independent
mutations establish the foundation. Later application work must add real native
definitions and runtime operations; it cannot relabel this fixture as live proof.

The existing live runner remains the single local/CI entry point. Sourced
scenario and validator modules retain its existing assertion behavior while the
entry point owns deadlines, progress, isolation and cleanup. Native libraries
retain their independent fixture formats and own native conformance evidence.

## Consequences

Scenario schema changes require review. Unsupported assertion forms fail closed;
optional empty categories are legitimate. The schema currently asserts service,
volume, network, config and secret selection, volume ownership/consumers,
required volume mounts, literal environment and published ports, and unpublished
services. Additional graph kinds need explicit assertion support before use.
The engine and neutral public APIs are unchanged.

Schema 1 external prerequisites use `file:<fixture-relative-path>`, with no
absolute paths or traversal. Preflight checks file availability without reading
contents or provisioning resources. Each route's `unavailable-prerequisites`
lists the exact expected missing subset of `semantics.external-prerequisites`.
A nonempty subset requires `unsupported-environment`; execution independently
compares the observed missing files and records unperformed evidence dimensions.

## Alternatives

Deriving expectations from exported artifacts or relying only on round trips was
rejected because importer/exporter defects can agree. Duplicating the live matrix
in each Lens repository was rejected because ownership and evidence would drift.
