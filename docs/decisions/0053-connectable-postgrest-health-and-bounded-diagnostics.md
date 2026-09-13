# ADR 0053: Connectable PostgREST health and bounded diagnostics

- Status: accepted
- Date: 2026-09-12
- Builds on: [ADR 0049](0049-bounded-supabase-application-acceptance.md),
  [ADR 0051](0051-peer-network-supabase-readiness.md), and
  [ADR 0052](0052-fresh-supabase-service-health-acceptance.md)

## Context

PostgREST's `postgrest --ready` health command connects to its configured administrative host.
A wildcard administrative host is a listener address, not a reviewed concrete loopback endpoint,
and therefore cannot reliably serve that native health command. Native Podman and Compose must
express the same PostgREST contract.

Fresh healthcheck execution is authoritative for non-database service acceptance under ADR 0052.
When it times out, a stale health state cannot make readiness pass, but it is useful diagnostic
evidence if it is bounded and redacted with the final configured-healthcheck result.

## Decision

Both native Podman and Compose configure `PGRST_ADMIN_SERVER_HOST=127.0.0.1` and retain
`PGRST_ADMIN_SERVER_PORT=3001` plus `postgrest --ready`. Native Podman supplies
`["postgrest","--ready"]` to `--health-cmd`; Podman prepends the exec-form `CMD` marker and stores
`["CMD","postgrest","--ready"]`. This marker-free CLI form remains correctly encoded when the
Podman 4.9.3 remote client provisions the nested Podman 6.1 service. Supplying the marker in the
CLI argument makes the 4.9.3 client serialize the complete JSON array as one command string. Compose
retains its equivalent YAML CMD list.
The pinned PostgREST image has no `/bin/sh`, so shell-form health commands are not valid. PostgREST
remains private to the backend network; this setting does not publish a port or alter the public
Kong boundary.

On timeout of any non-database configured service healthcheck, the runner fails closed after one
final `podman healthcheck run`. It emits that bounded, redacted result together with bounded
container state, the diagnostic health-log, and the bounded container-log tail. Cached health
state is diagnostic evidence only and never satisfies readiness. PostgreSQL remains governed by
ADR 0051's peer-network SQL proof.
For a final PostgREST failure only, the diagnostic phase additionally runs exactly one bounded
`podman exec <rest> postgrest --ready` probe through the runner's standard 90-second
per-operation `engine_operation` bound. Its exit and bounded, redacted output,
the stored health-test shape, and only `PGRST_ADMIN_SERVER_HOST` and
`PGRST_ADMIN_SERVER_PORT` are evidence; a successful probe never overrides the failed fresh
healthcheck. No other PostgREST environment is emitted.
Raw final-healthcheck output remains only in bounded process memory. Redaction conservatively masks
protected values split at the raw capture boundary before the diagnostic output is truncated.

## Consequences

- The PostgREST configured readiness command has a concrete loopback administrative target in
  both provisioners.
- Service-health failures provide actionable but privacy-safe evidence without weakening ordering;
  the additional PostgREST diagnostic uses the runner's standard 90-second operation cap.
- The extra PostgREST command probe cannot promote a failed
  healthcheck to readiness.
- A configured healthcheck that cannot succeed remains a migration-readiness failure regardless
  of retained lifecycle metadata.

## Alternatives considered

Keeping the wildcard host was rejected because a bind address is not a dependable readiness target.
Using a shell-form native health command was rejected because the pinned image does not provide a
shell. Using cached health status as a fallback was rejected because it repeats the stale-metadata
failure addressed by ADR 0052. Emitting unbounded inspect or container logs was rejected because
live failure evidence can contain protected values.
