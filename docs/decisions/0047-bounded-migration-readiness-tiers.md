# ADR 0047: Bounded migration-readiness tiers

- Status: accepted
- Date: 2026-09-10
- Builds on: [ADR 0039](0039-independent-migration-scenarios.md),
  [ADR 0043](0043-bounded-forgejo-live-application-acceptance.md),
  [ADR 0044](0044-bounded-paperless-document-processing-acceptance.md),
  [ADR 0045](0045-bounded-immich-media-processing-acceptance.md), and
  [ADR 0046](0046-bounded-podman-limitation-revalidation.md)

## Context

BoxFerry has independent offline scenario contracts, a 48-cell Podman catalogue, and six genuine
application harnesses. Running every application against every runtime on every pull request would
repeat expensive evidence without strengthening the claim. Conversely, a skipped privileged job,
an offline parse, or BoxFerry reimport alone must never appear as a successful migration.

ComposeLens and QuadletLens expose test-only application entry points. BoxFerry needs to consume
candidate revisions before their next published release without adding Git or path dependencies to
published packages or letting untrusted candidate code enter an ordinary privileged job.

## Decision

The machine-readable
[`tiers.toml`](../../fixtures/conformance/migration-readiness/tiers.toml) catalogue defines exactly
three gates, their ordered tasks, prerequisites, claims, approved-loss authority, and measured wall,
RSS, disk, and concurrency budgets:

1. `offline` is part of the ordinary local and pull-request gate. It runs every registered authored
   application and captured Podman inventory through every applicable importer/exporter, exact loss,
   diagnostic, redaction, native-artifact, and reimport contract. It performs no privileged work.
2. `trusted-live` is explicit manual evidence on the default branch. Three representative Podman
   package/API cells cover 5.4 rootless and 6.1 rootful/rootless boundaries; Forgejo supplies genuine
   HTTP and SSH application behavior across both deployment origins.
3. `pre-release` is explicit manual evidence on the exact candidate SHA. It adds the complete
   reviewed container distribution matrix, Nextcloud, Forgejo, Paperless-ngx, CPU-only Immich,
   observability, Supabase, and exact-revision ComposeLens and QuadletLens native application
   gates. It is not scheduled and uses no virtual machines.

[`scripts/migration-readiness.py`](../../scripts/migration-readiness.py) is the sole tier-selection,
budget-measurement, and evidence helper locally and in GitHub Actions. It delegates provisioning to
the established live runner rather than duplicating workload logic. Tasks execute sequentially;
catalogue concurrency may never exceed two. Every task has timestamped numbered preflight and
execution events, a hard deadline, process-tree peak RSS and disk-growth measurement, and fail-closed
prerequisites. Every tier also has an independent wall deadline below its enclosing job timeout;
tasks receive only the smaller of their own deadline and the tier time remaining so evidence can be
written before the outer workflow timeout. Existing live harness traps remain responsible for
resource cleanup.

Evidence conforms to
[`migration-readiness-evidence-v1.schema.json`](../schemas/migration-readiness-evidence-v1.schema.json).
The workflow starts the shared runner through `sudo` only so catalogue tasks marked
`privileged = true` can reach rootful Podman. Every task marked `privileged = false`, including
fetched Lens candidates, executes as the validated original non-root sudo account with its own
home and supplementary groups; a missing or inconsistent identity fails preflight. Successful
evidence is rejected unless its recorded effective UID agrees with the catalogue privilege flag.
It binds a fresh run identity, exact BoxFerry revision, exact Lens revisions, workload, source,
target/version claim, approved losses, manual prerequisites, budgets, observations, and outcome.
`unavailable`, `not-run`, timeout, new loss, failed behavior, or missing exporter evidence makes the
tier fail. GPU behavior, virtual machines, SELinux enforcing runtime effects, and booted systemd
remain explicit non-success gaps in every document.

GitHub runs the offline tier once through ordinary CI. Privileged tiers are manual, default-branch,
same-repository jobs. A protected release does not rerun or infer pre-release success: it downloads a
successful `pre-release` artifact for the exact release SHA and validates its content with the same
helper before publication can begin.

Candidate Lens tasks clone only the catalogue's reviewed full 40-character commit into a temporary
directory, verify the resolved revision, invoke the repositories' native `ci-application` and
pinned-generator entry points, then remove the checkout. Workspace manifests and `Cargo.lock`
retain released crates.io dependencies only.

## Consequences

- Ordinary pull requests retain complete deterministic application semantics without six large
  live jobs or an application-by-runtime Cartesian product.
- Trusted live and release claims are reproducible with one tier or one task command; a skipped host
  prerequisite is evidence of a gap and a failed gate, never success.
- Docker Compose provider behavior, Quadlet generator behavior, and disposable Podman plan checks
  remain independent of BoxFerry reimport assertions. Runtime apply remains test-harness-only;
  BoxFerry never executes emitted plans.
- Changing a task, budget, exact revision, approved-loss authority, or gap is a reviewed catalogue
  change. Updating a Lens release remains a separate dependency-policy change.

## Alternatives considered

Running all applications on every pull request was rejected because it multiplies large image and
runtime cost without new semantic coverage. A nightly matrix was rejected because immutable manual
pre-release evidence is the claim boundary. Committed Git/path Lens dependencies were rejected
because they would couple published crates to candidate test sources. Treating environment skips as
successful checks was rejected because it would manufacture migration-readiness evidence.
