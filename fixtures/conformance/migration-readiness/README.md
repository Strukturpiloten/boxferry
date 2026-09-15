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
sudo env BOXFERRY_BIN="$PWD/target/debug/boxferry" BOXFERRY_COMPOSE_BIN="$PWD/target/tools/docker-compose" PATH="$PATH" python3 scripts/migration-readiness.py run --tier pre-release --task supabase-application
```

Local pre-release runs must select exactly one task for focused reproduction. A complete
pre-release run is parallel aggregate evidence produced only by the GitHub workflow; the runner
rejects an unfiltered serial pre-release invocation.

Serial `offline` and `trusted-live` runs each have an independent wall deadline below the enclosing
GitHub job timeout. Their runner caps every task to the smaller of its own deadline and the tier
time remaining, then writes failed and `not-run` evidence when the tier is exhausted. A focused
pre-release worker retains its reviewed task deadline; the collector separately applies the
aggregate pre-release admission deadline. Workflow setup time and evidence upload remain outside
the catalogue budget with a deliberate job-timeout margin.

Reproduce one failed task without broadening its claim:

```console
sudo env BOXFERRY_BIN="$PWD/target/debug/boxferry" BOXFERRY_COMPOSE_BIN="$PWD/target/tools/docker-compose" PATH="$PATH" python3 scripts/migration-readiness.py run --tier trusted-live --task forgejo-root-modes
```

The default output is `target/migration-readiness/evidence-v2.json`. Validate it against an exact
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

A complete pre-release run is different from serial local and focused commands: GitHub creates one
exact-SHA plan and BoxFerry build, then schedules one fail-fast-false worker matrix capped at four.
The four deterministic `--matrix-shard N/4` workers cover every one of the 48 Podman matrix rows
and all five limitation rows exactly once. Application and Lens tasks are isolated. Workers bind
their evidence to the same coordinator, revision, catalogue digest, shared binary digest, and exact
matrix rows; the collector rejects missing, duplicate, stale, failed, timed-out, or focused evidence
and is the only artifact the release workflow accepts. Its 1,200-second deadline applies to the
aggregate earliest-worker-start through latest-worker-finish interval without truncating historical
task safety deadlines. Resource observations remain per-worker and are never summed across hosts;
only total runner wall work is summed.

The pre-release tier includes bounded `observability-application` and `supabase-application` Podman
6.1 rootless tasks. Supabase is an isolated worker with its historical 5,400-second safety limit
and
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
