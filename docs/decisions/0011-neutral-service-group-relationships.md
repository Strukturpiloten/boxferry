# ADR 0011: Neutral service-group relationships

- Status: accepted
- Date: 2026-08-04

## Context

Podman inspection can prove that selected containers belong to one pod. The shared runtime layer
already retains that relationship, but the neutral application graph cannot represent it. Runtime
reconstruction therefore reports every observed pod as unsupported even when all selected members
are present.

A Podman pod does not by itself prove portable namespace, lifecycle, or target-management intent.
Treating every observed pod as an application-owned Quadlet or Kubernetes pod would silently make
those decisions for the caller. Naming the neutral type after Podman would also couple the model to
one source implementation.

## Decision

1. The neutral model gains an ordered `ServiceGroup`. It records a name, lifecycle ownership, and
   ordered service references with provenance.
2. A service group records only structural co-membership. It does not imply which Linux namespaces
   are shared, whether an infra container is required, or which target workload kind must be used.
3. A service must exist before a group referencing it is added, may appear only once in a group,
   and may belong to at most one group in an application.
4. Runtime reconstruction maps a consistent Podman pod/member relationship into a service group
   with `ResourceOwnership::Uncertain`. Membership is retained exactly; lifecycle and target
   interpretation remain an approximate decision requiring review.
5. Missing or contradictory pod/member observations remain structured unsupported or invalid
   outcomes. The importer does not repair them by guessing or by taking the union of conflicting
   response fields.
6. Target adapters must report every unresolved service group. They may preserve, flatten, split,
   or reject it only through a later explicit caller-selected resolution policy.

## Consequences

- Runtime-to-application conversion no longer discards valid pod membership.
- The model can later represent equivalent grouping discovered from Kubernetes without carrying
  Podman response types.
- Existing authored Compose applications remain unchanged because they contain no implicit group.
- Quadlet and future Compose output stay fail-visible until their grouping and lifecycle policies
  are implemented.
- Additional shared-context details can be added later without retroactively claiming that bare
  membership proved them.

## Alternatives considered

Adding a neutral `Pod` was rejected because the name suggests semantics not established by the
current observations. Inferring `SharedNetworkNamespace` was rejected because Podman pod namespace
sharing is configurable and the initial decoder does not model that configuration. Storing only a
group identifier on each service was rejected because it would lose group-level provenance,
ordering, lifecycle, and future target policy.
