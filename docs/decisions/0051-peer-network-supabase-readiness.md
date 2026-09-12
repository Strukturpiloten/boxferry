# ADR 0051: Peer-network Supabase readiness

- Status: accepted
- Date: 2026-09-12
- Builds on: [ADR 0049](0049-bounded-supabase-application-acceptance.md) and
  [ADR 0050](0050-executable-post-migration-supabase-bootstrap.md)
- Supersedes: ADR 0049 and ADR 0050's database self-address readiness clauses

## Context

An in-database `exec` probe can prove only that PostgreSQL accepts a connection
from itself. Choosing a non-loopback address inside that same container avoids
the trusted socket but still cannot prove backend-network DNS, routing, or a
peer's password-authenticated connection. Compose's database healthcheck is a
useful local lifecycle signal, but has the same boundary limitation.

## Decision

The authoritative SQL readiness proof is a disposable, prefix-named Podman
peer. It runs the pinned PostgreSQL image with `--rm --pull=never`, attaches
only to the private `${prefix}-supabase-backend` network, uses `psql` as its
entrypoint with the protected password environment, and connects to `db` as
`postgres`. It retains the existing complete SQL predicate. No authoritative
probe uses database `exec`, a socket, a self-address, hostname discovery, or
loopback.

Native provisioning runs this proof before creating dependents. Every Compose
graph start, including persistence recreation, starts only `db`, retries the
peer proof with bounded, redacted SQL, state, and log-tail evidence on failure,
and only then starts the complete graph. Initial Compose provisioning first
creates its edge network and starts its separate edge-only boundary peer after
the complete graph. The Compose database healthcheck uses the same
password-authenticated SQL predicate through `127.0.0.1`; it remains a local
Compose signal rather than readiness evidence.

## Consequences

- Readiness proves the private backend network contract from a real peer.
- A failed peer probe cannot start application dependents, the Compose full
  graph, or the edge boundary peer.
- The disposable peer has a visible prefix-scoped name for bounded cleanup and
  failure investigation while `--rm` prevents retained container state.
