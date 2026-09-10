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

The pre-release tier includes the bounded `observability-application` Podman 6.1 rootless task.
Successful PromQL, LogQL, Grafana, retention, persistence, privacy, and ownership checks replace the
former observability gap; five explicit non-success gaps remain in every evidence document.
