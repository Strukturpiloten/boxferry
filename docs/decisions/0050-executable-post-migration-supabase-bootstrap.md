# ADR 0050: Executable post-migration Supabase bootstrap

- Status: accepted
- Date: 2026-09-12
- Builds on: [ADR 0049](0049-bounded-supabase-application-acceptance.md)
- Supersedes: ADR 0049's authored SQL placement and ordering clause

## Context

Protected pre-release evidence for exact BoxFerry revision
`1766ef60850371342df02aaa4d53bcf2449ae099` showed the PostgreSQL container running while every
password-authenticated non-loopback readiness probe failed. The reviewed Supabase image inherits
the docker-library PostgreSQL entrypoint, which expands `/docker-entrypoint-initdb.d/*` only at the
top level. It does not recurse into the image's `init-scripts/` directory.

The ADR 0049 implementation mounted authored SQL below that directory, so the entrypoint ignored
the file. Moving the existing numeric filename to the top level would not preserve the intended
ordering: digit-prefixed names sort before the image-owned `migrate.sh` driver. In either case the
authored PostgreSQL password, table, grants, and terminal completion marker do not prove that they
were applied after the image migrations.

## Decision

Both native Podman and Docker Compose provisioners mount the authored SQL as the top-level file
`/docker-entrypoint-initdb.d/zzzzzzzzzzzz-boxferry.sql`. The basename sorts after the image-owned
`migrate.sh` driver and is included by the entrypoint's non-recursive glob. The fixture graph and
structural regressions use the same path and reject both the ignored nested path and an early
top-level numeric path.

The database readiness contract is unchanged. It uses password-authenticated non-loopback TCP as
the application-facing `postgres` role and requires the exact PostgreSQL configuration file, the
image-owned `supabase_read_only_user` migration role, the authored `public.boxferry_items` table,
and the terminal `public.boxferry_bootstrap_complete` marker. Successful migration-readiness still
requires fresh protected `pre-release` evidence for the exact merged BoxFerry revision.

Failure evidence includes the bounded, redacted final SQL probe before the existing bounded
container-state and log-tail context. This distinguishes connection or contract failures from
unrelated tolerated image migration messages without exposing protected fixture values.

## Consequences

- The authored extension actually executes and remains ordered after the reviewed image migration
  driver.
- A healthy temporary initialization server or a running container cannot satisfy readiness before
  both image-owned and authored database contracts exist.
- Changing the image entrypoint layout or sort order requires new reviewed evidence and an explicit
  contract update.
