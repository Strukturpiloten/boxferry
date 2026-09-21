# ADR 0055: Parallel readiness and local quality feedback

- Status: accepted
- Date: 2026-09-15
- Builds on: [ADR 0047](0047-bounded-migration-readiness-tiers.md),
  [ADR 0048](0048-bounded-observability-application-acceptance.md), and
  [ADR 0049](0049-bounded-supabase-application-acceptance.md)
- Supersedes in part: ADR 0047's sequential pre-release scheduling and evidence-v1 contract,
  ADR 0048's aggregate scheduling deadline, and ADR 0049's requirement that the Supabase worker
  share one serial pre-release runner

## Context

The complete local gate protects tests, coverage, MSRV, documentation, dependencies, SemVer, and
offline migration semantics, but its cost makes it unsuitable as the only pre-push cleanliness
task on smaller development computers. The exact-SHA pre-release gate had the opposite scaling
problem: six application profiles, two Lens candidates, offline contracts, and all 48 Podman rows
shared one runner. A measured run consumed 47 minutes 12 seconds of runner time; a late independent
failure marked every later task `not-run` and required another complete attempt.

Application-internal work must remain sequential. In particular, Supabase service ordering,
persistence recreation, cleanup, privacy checks, and its 90-minute safety deadline are one atomic
acceptance contract. Parallel commands inside that worker or shared Podman storage between
applications would weaken the evidence.

## Decision

BoxFerry exposes two explicit local quality paths. `scripts/format-lint.sh` supports `--fix` and
`--check`, formats Rust and repository-owned files, checks staged and unstaged whitespace, runs
file and GitHub Actions linters, and runs Clippy. It executes no tests. Clippy defaults to two Cargo
jobs and accepts a positive `BOXFERRY_LINT_JOBS` override. The VS Code task names this no-tests
boundary. It is a pre-push cleanliness aid; `scripts/check-all.sh` and protected GitHub checks
remain complete validation evidence.

An unfiltered exact-SHA pre-release invocation uses one plan, one checksum-bound BoxFerry build,
one worker matrix with `fail-fast: false`, and one global maximum of four hosted runners. The plan
describes privilege and tool requirements so offline and Lens workers do not install Podman or
Docker Compose. Privileged tasks run through `sudo`; non-privileged tasks retain the runner
identity. Every application and Lens candidate has an isolated checkout, temporary directory, and
Podman graph root. Application-internal ordering and every task-specific deadline and resource
budget remain unchanged.

The complete Podman catalogue is split by stable row order into four 12-row modulo shards. Each
fragment binds its shard, exact row IDs, all applicable limitation rows, and SHA-256 digests of the
48-row matrix and five-row limitation catalogue. The collector requires the four ordered shards to
cover every row and limitation exactly once.

Evidence schema v2 binds each fragment to a coordinator UUID, worker/task identity, exact
40-character BoxFerry revision, catalogue SHA-256, both reviewed Lens revisions, and the shared
BoxFerry binary SHA-256. The collector rejects missing, duplicate, failed, timed-out, stale,
wrong-revision, wrong-catalogue, wrong-binary, wrong-worker, or incomplete-shard fragments. It
derives actual concurrency from worker task intervals and aggregate wall time from the earliest
worker start through the latest finish. Per-worker RSS and disk observations remain attached to
their task; they are never summed into a fictional cross-host machine measurement. Aggregate
evidence records only the sum of runner wall work in addition to elapsed wall time.

The pre-release aggregate target and admission deadline is 1,200 seconds, excluding GitHub queue
time. That target does not truncate a worker's preserved task deadline; a worker may finish and
upload diagnostic evidence after the target, but the collector then refuses release evidence.
Workflow job timeouts leave setup and evidence-upload margin beyond every inner task deadline.
Only a complete successful aggregate artifact from the current Release run is
release-acceptable. Its artifact identity includes the exact release SHA and run ID;
the evidence retains the coordinator, worker, catalogue, and binary bindings above. Release invokes
the same reusable workflow used for manual diagnosis, while focused and earlier-run artifacts remain
diagnostic and cannot authorize publication. A validation-only Release invocation exercises this
same contract without granting publication credentials or creating release state. Artifact names
remain stable inside one GitHub run and each task upload replaces only its own prior-attempt
artifact. Partial retries retain successful workers, replace retried workers, and still fail closed
when a stale coordinator, missing task, or different candidate appears. Release also calls the
ordinary reusable CI workflow instead of copying its deterministic tasks.

### Retry timing clarification

Aggregate deadline accounting is the union of active worker task intervals, rather than the span
from the earliest retained worker to the latest retried worker. Evidence still retains those
earliest/latest boundaries and derives actual concurrency from the same intervals. Thus GitHub
queue time and inactive waits between attempts do not consume the 1,200-second active-work budget;
each worker deadline, resource observation, exact binding, and complete-fragment requirement remain
unchanged.

Worker fragments record the positive GitHub run attempt. The collector groups retained and retried
fragments by that explicit identity and records each attempt's boundaries, active interval union,
concurrency, and task IDs. Every attempt independently meets the 1,200-second admission budget;
the aggregate's wall time is the longest attempt, not the sum across attempts. RFC 3339 offsets
are compared as instants, and increasing attempt identities must have non-overlapping chronological
intervals.

## Consequences

- Small computers have a predictable format/lint path without misrepresenting test status.
- Independent pre-release failures are visible in one attempt, while no host runs more than one
  heavy application and global hosted-runner demand remains four.
- Four Podman shards can transfer some images redundantly; bounded wall feedback is preferred over
  caching multi-gigabyte OCI archives or sharing mutable runtime state.
- A worker exceeding 20 minutes remains diagnosable under its reviewed safety budget but blocks
  release until the workflow or workload is deliberately re-evaluated.
- Evidence v1 remains historical. Release and CI consumers use evidence v2.

## Alternatives considered

Running the application profiles in parallel inside one host was rejected because their resource
budgets and cleanup boundaries would interfere. Keeping one serial runner was rejected because a
late failure hides unrelated results. Replacing the complete local gate with linting was rejected
because formatting and Clippy cannot prove tests, coverage, MSRV, SemVer, or migration readiness.
