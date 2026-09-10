# Migration-readiness catalogue

[`tiers.toml`](tiers.toml) is the source of truth for BoxFerry's `offline`, `trusted-live`, and
`pre-release` evidence. Run the same selector used by GitHub Actions:

```console
python3 scripts/migration-readiness.py plan --tier offline --format json
python3 scripts/migration-readiness.py run --tier offline
```

Live tiers need the prerequisites printed by `plan`, a freshly built `BOXFERRY_BIN`, and the
checksum-verified Docker Compose 5.5.0 binary recorded by each application `providers.tsv`:

```console
sudo env BOXFERRY_BIN="$PWD/target/debug/boxferry" BOXFERRY_COMPOSE_BIN="$PWD/target/tools/docker-compose" PATH="$PATH" python3 scripts/migration-readiness.py run --tier trusted-live
sudo env BOXFERRY_BIN="$PWD/target/debug/boxferry" BOXFERRY_COMPOSE_BIN="$PWD/target/tools/docker-compose" PATH="$PATH" python3 scripts/migration-readiness.py run --tier pre-release
```

Each catalogue tier has an independent wall deadline below its enclosing GitHub job timeout. The
runner caps every task to the smaller of its own deadline and the tier time remaining, then writes
failed and `not-run` evidence when the tier is exhausted. Workflow setup time and evidence upload
remain outside that catalogue budget with a deliberate job-timeout margin.

Reproduce one failed task without broadening its claim:

```console
sudo env BOXFERRY_BIN="$PWD/target/debug/boxferry" BOXFERRY_COMPOSE_BIN="$PWD/target/tools/docker-compose" PATH="$PATH" python3 scripts/migration-readiness.py run --tier trusted-live --task forgejo-root-modes
```

The default output is `target/migration-readiness/evidence-v1.json`. Validate it against an exact
revision with `validate-evidence --tier TIER --revision FULL_SHA --require-success`. The helper
records missing tools, privilege, memory, or disk as `unavailable` and exits nonzero. It never turns
a gap or an unfinished later task into success.

Lens revisions come only from the reviewed catalogue. Changing a ComposeLens or QuadletLens pin
requires changing `tiers.toml`; the runner accepts no command-line revision override.

Live tasks reuse `scripts/podman-live-conformance.sh`: Docker Compose output is consumed by the
reviewed standalone provider, Quadlet output is checked by the pinned candidate generator, and
Podman plans/scripts are structurally checked and executed only inside disposable test targets.
ComposeLens and QuadletLens candidates are fetched into temporary exact-revision checkouts and are
never Cargo dependencies.

The pre-release tier includes bounded `observability-application` and `supabase-application` Podman
6.1 rootless tasks. Supabase runs immediately after observability with a 5,400-second deadline and
requires four CPUs, 12 GiB available memory, and 24 GiB free temporary and Podman graph-root space.
Its 5 GiB cold-archive cap supports a reviewed 14,336 MiB RSS ceiling and 20,480 MiB disk-growth
ceiling: 12 GiB application memory plus 2 GiB runner headroom, and four cap-sized transient disk
representations while retaining 4 GiB of preflight space. Reproduce only that task with:

```console
sudo env BOXFERRY_BIN="$PWD/target/debug/boxferry" BOXFERRY_COMPOSE_BIN="$PWD/target/tools/docker-compose" PATH="$PATH" python3 scripts/migration-readiness.py run --tier pre-release --task supabase-application
```

Successful authentication, database/API, Storage, Realtime, Edge Runtime, persistence, privacy,
selection, ownership, ingress, collision, recreation, and cleanup checks replace the former
Supabase runtime gap. GPU behavior, virtual machines, SELinux-enforcing runtime effects, and booted
systemd are the four explicit non-success gaps retained in every evidence document.

See [ADR 0049](../../../docs/decisions/0049-bounded-supabase-application-acceptance.md) for the
application, supply-chain, and resource-boundary decision.
