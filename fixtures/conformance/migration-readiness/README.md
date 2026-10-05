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
Focused local runs write `evidence_kind: local`, bind the selected task and checked-out revision,
and retain catalogue, configured binary digest, budgets, outcome, and explicit gaps. They do not
claim a GitHub attempt or hosted coordinator. `validate-evidence --task TASK` can inspect this
diagnostic evidence; the hosted collector rejects it, even when the local task passes.

The trusted-live `podman-api-*` tasks select the smoke profile: they prove read-only acquisition
and exporter contracts, with additional diagnostics on the selected rootful cell, but do not
execute generated output. Disposable apply/reacquire belongs to eligible cells of the complete
`full-container` profile. A focused full-cell diagnostic can exercise that path; it still cannot
replace the complete current-candidate pre-release catalogue.

Serial `offline` and `trusted-live` runs each have an independent wall deadline below the enclosing
GitHub job timeout. Their runner caps every task to the smaller of its own deadline and the tier
time remaining, then writes failed and `not-run` evidence when the tier is exhausted. A focused
pre-release task retains its reviewed task deadline; the collector separately applies the
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
The QuadletLens candidate is bound to the released 0.2.4 commit and its 6.1.2 generator contracts.
Its release tag/commit pair is Renovate-managed but requires explicit native revalidation.
The ComposeLens consumer binds the published 0.3.4 release and independent authored contracts,
including the service/environment association repair from
[release PR #175](https://github.com/Strukturpiloten/compose-lens/pull/175).
It is not evidence of new Compose-provider or Podman runtime compatibility.
Cargo's manager owns published requirements and lockfile integrity separately. One release-ref
manager extracts both annotated Lens tag/commit pairs, requires dashboard approval and never
auto-merges; captured historical revisions remain unchanged.

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

## Filesystem-space measurement

Disk growth measures the decrease in a mounted filesystem's available-space estimate, not exclusive
task writes. Ordinary filesystems retain `st_dev` accounting. Btrfs subvolumes can have different
`st_dev` and statfs IDs while sharing one global `f_bavail` view: the helper counts that view once
using the mounted FSID from the fixed, read-only `BTRFS_IOC_FS_INFO` ABI and the complete canonical
kernel backing-device set. Equal counters and filesystem UUIDs alone never authorize merging;
distinct physical-space identities still contribute separately to the unchanged growth budget.

Linux mountinfo and the opened descriptor's mount ID select the exact longest matching mount,
including overmounts. Valid [opaque nsfs namespace roots](https://github.com/torvalds/linux/blob/v7.2/fs/nsfs.c#L404)
on unrelated mounts are accepted; unrelated mount-table churn does not change the selected view.
Mountinfo paths are decoded once, and retained evidence preserves literal backslashes. Bounded
ioctl/sysfs checks reject malformed, missing, incomplete, ambiguous, or changing identity. The
descriptor supplies both identity and `f_bavail * f_frsize`; all registered paths are revalidated on
every sample and at final evidence collection. A fresh read-only descriptor reopens the requested
path's current ancestor after the counter read, so a same-target overmount cannot hide behind a
still-valid old descriptor. Additive evidence-v2 path observations retain each role, requested path,
measured ancestor, raw subvolume device/statfs IDs, mount provenance, and Btrfs backing devices. A
disk measurement error is sticky: preflight cannot run the task, and a mid-run error stops/reaps its
child and produces failed, non-timeout evidence.

Measurement aborts and timeouts give the entire owned process group the existing 20-second TERM
grace, then issue group-wide KILL before bounded leader reaping. The leader remains unreaped until
group cleanup completes, preventing PID/PGID reuse during signalling. This is not a guarantee that
detached descendants or managed-runtime resources were cleaned; runtime suites retain their own
exact-label cleanup checks. Historical evidence-v1 is unchanged. Recorded backing-device provenance
is validated canonically with kernel device-number bounds and a one-to-one target mapping, without
dereferencing another host's historical sysfs paths.

The ABI and accounting basis are the Linux
[FS_INFO UAPI](https://github.com/torvalds/linux/blob/v6.17/include/uapi/linux/btrfs.h),
[mounted-FSID ioctl](https://github.com/torvalds/linux/blob/v7.2/fs/btrfs/ioctl.c),
[kernel device inventory](https://github.com/torvalds/linux/blob/v7.2/fs/btrfs/sysfs.c), and
[Btrfs statfs implementation](https://github.com/torvalds/linux/blob/v7.2/fs/btrfs/super.c).
This is a shared available-space estimate affected by allocation profiles, metadata and reservations,
not qgroup quota headroom or exclusive-write attribution. The exact local vendor-kernel source is
not bound by those upstream references. Unsupported Btrfs ioctl ABIs fail unavailable; x86_64 and
AArch64 use the reviewed generic encoding, without claiming new native runtime conformance.
