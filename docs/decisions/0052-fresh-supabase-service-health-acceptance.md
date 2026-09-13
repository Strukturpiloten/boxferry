# ADR 0052: Fresh Supabase service-health acceptance

- Status: accepted
- Date: 2026-09-12
- Builds on: [ADR 0049](0049-bounded-supabase-application-acceptance.md), [ADR 0051](0051-peer-network-supabase-readiness.md)

## Context

The nested rootless Podman target can retain configured container health metadata without
advancing its scheduled status. Polling `.State.Health.Status` then waits on stale lifecycle
metadata even when a service's configured health command is able to provide a current result.
That makes the acceptance lane depend on runtime scheduling behavior rather than the configured
service contract.

ADR 0051 already makes the PostgreSQL peer-network SQL predicate the database authority. The
remaining health-checked services need a similarly fresh, bounded acceptance proof without
changing Compose dependency and lifecycle semantics.

## Decision

After the PostgreSQL peer SQL contract succeeds, native and Compose provisioning both execute
`podman healthcheck run` through the target socket for Auth, PostgREST, Realtime, imgproxy,
Storage, Edge Runtime, Studio, and Kong. Each command is retried only within its existing
600-second deadline; a non-zero result fails closed and prevents later readiness checks.

The pinned Studio image has no embedded healthcheck. Native provisioning therefore installs the
independently defined Compose-equivalent profile-endpoint probe at container creation: `node -e`
requests `http://127.0.0.1:3000/api/platform/profile`, with a 3-second interval, 5-second timeout,
150 retries, and 10-second start period. The native command is passed as a scalar Podman
`--health-cmd`, which Podman records as `CMD-SHELL`; explicit JSON `CMD` or `CMD-SHELL` marker
arrays are avoided because the reviewed remote client does not serialize them safely. Studio
remains a mandatory fresh-execution contract.

Compose first runs its normal full `up --detach --remove-orphans` provider command in the
background. While that bounded provider command is waiting on its retained
`condition: service_healthy` dependencies, the runner explicitly executes each configured
healthcheck as its container appears: PostgreSQL plus the eight services above. PostgreSQL's
execution only drives Compose lifecycle progress; it never replaces ADR 0051's peer-network SQL
acceptance proof. The runner waits for the provider process before proceeding, so a provider
failure fails closed and cannot leave it running in the background.

The acceptance path never reads `.State.Health.Status`. Compose retains its configured service
healthchecks and `depends_on` rules as local lifecycle and startup ordering signals, not as the
authoritative acceptance proof. PostgreSQL remains governed by ADR 0051's peer-network SQL
contract. Meta and Supavisor remain process plus HTTP checks because they do not supply the
same configured container-health contract.

## Consequences

- Readiness observes a fresh execution of each service's native health command rather than
  cached scheduling metadata.
- The existing deadlines, retry behavior, redacted failure handling, and database-first ordering
  remain intact.
- A target that cannot execute a configured service healthcheck cannot claim Supabase acceptance.

## Alternatives considered

Increasing the health-status polling deadline was rejected because it cannot make stale metadata
fresh. Treating process state or an HTTP subset as a replacement was rejected because it weakens
the reviewed configured-health coverage for the eight services.
