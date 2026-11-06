# ADR 0094: Official Dev Container feature updates and candidate qualification

- Status: proposed; producer implementation and consumer admission are separate
- Date: 2026-11-04
- Builds on: ADR 0092 shared development automation and ADR 0079 runtime selection

## Context

The [canonical Renovate runtime](../../automation/renovate-runtime.json) extracts
Dev Container feature declarations but provides
no feature-lock artifact generator. A regex owner for a single lock record cannot
maintain the official transitive graph. Metadata-only CI cannot prove that an
updated candidate image builds and its declared tools execute on native x64 and
ARM64. The maintainer selected an Actions-hosted limited Renovate instance and
the official generator, retaining the hosted App for all other dependencies.

## Decision

BoxFerry owns a typed reusable updater, integrity-locked official Dev Container
CLI npm project, and candidate qualification. Four literal Dev Container owners
may call it: BoxFerry, ComposeLens, PodmanLens and QuadletLens. DockerLens,
KubernetesLens and the website gain no Dev Container build/update job.

The limited instance reuses the canonical Renovate OCI pin and ignores repository
configuration. Native feature declarations and one exact nested GitHub CLI option
form one group; the hosted preset yields exactly those owners. The obsolete
single-record lock regex is removed. Administrator-owned post-upgrade commands
are fixed absolute trusted paths with no templates or candidate hooks. Official
`upgrade` runs against private scratch with no App token, user configuration,
Docker credentials or Node hooks. Only its validated whole graph replaces the
candidate lock. Feature installers and lifecycle commands do not execute.

The isolated npm declaration/lock owns the CLI version and official registry
integrity; native hosted npm owns its updates. Its complete lock participates in
trusted dependency snapshots and ordinary 72-hour dependency age. All newly
introduced or changed direct/transitive OCI identities retain 72 hours, including
digest-only changes and trusted first-seen quarantine where publication time is
missing. The feature App has no first-party exemption.

One native version group uses `renovate-devcontainer/features`. Since the pinned
native manager cannot refresh unchanged declarations, the scheduled helper uses
its reserved `renovate-devcontainer/lock-file-maintenance` branch, which the
exact pinned native prune implementation exempts. Active version proposals take
precedence; only independently verified App-owned maintenance proposals may be
closed or refreshed. Unknown authors, files, commits, repositories or moved heads
fail closed. Maintenance updates are non-force and use current-main trees plus
the generated lock; stale owned branches preserve ancestry through merge parents.
No unrelated branch is deleted or changed. Protected exact-head queue admission
requires ordinary required checks and dependency eligibility; queue/protection
and independent review remain authoritative. Enqueued never means merged.

The operational graph reader validates declared roots and the reachable
transitive DAG, exact OCI content, versions/selectors and metadata-backed edges.
It supports explicit exact roots and reachable transitive major/minor/digest
selectors. Official selector-alias collapse is rejected pending a reviewed
identity/options resolver. Optional metadata annotations may be absent: the
reader still obtains metadata from digest-verified tarballs and compares whole
parsed metadata when an annotation exists. Missing, orphan, cyclic, unsupported
or inconsistent graphs fail.

Required image qualification binds the actual candidate, independently trusted
policy, admitted automation, complete input fingerprint and image identity. It
builds and executes reviewed non-root probes on native Linux x64 and ARM64 using
reviewed backends; local execution remains Podman-only. BoxFerry workspace probes
require all seven real credential-free recorded checkouts and fresh owned
storage. Only reviewed verifier/probes execute, not lifecycle commands or cargo
fetch hooks. Image tool versions must match selected canonical automation. Fixed
FCOS, Podman/Docker runtime, workload and provider qualification pins remain
unchanged. Cold resource/budget qualification and issue 446 ARM workspace evidence
remain required and are not established by this ADR.

Hosted Podman reuses the reviewed Ubuntu signed-APT outer prerequisite exception, recording
actual `dpkg` package/version, UID-0 and rootful observations separately from fixed inner qualification pins.
Before consumer activation, cold storage budget proof must cover the actual Docker daemon
filesystem; producer definitions alone establish no measured resource or native evidence.

Explicit operator-selected local rootful qualification keeps preparation/reports under
the original user. Before its first root bridge, the literal installed Podman executable
and every parent/symlink hop require root ownership, no operator write access, and
bounded user-observed version/hash evidence with drift checks. Trusted selection/npm bootstrap stay outside; existing candidate
snapshot/context copies may run inside after the verified groups/GID/UID drop. The
fixed isolated stdlib bootstrap/root PID1 owns one foreground PID/mount namespace.
A sole close-on-exec FIFO writer is established with a temporary reader that closes
before root launch. The claimed FIFO EOF lease and pre-cleanup abort handoff handle startup and cancellation;
only bound PID1 pidfd/supervisor closure authorizes outside anchored, no-follow finite
cleanup. Within that exact private store, only root-owned, mode-000, single-link
character-device 0:0 whiteouts below `store/overlay/<64-lowercase-hex>/diff/` may
join ordinary cleanup. Metadata-only, no-follow descriptors must corroborate
identity, device and mount before a final entry check and anchored unlink. Existing
regular-file, symlink, FIFO and socket cleanup stays unchanged; other device nodes
and unknown inode kinds still fail closed. Cache tmpfs mounts use the explicitly selected
backend's ownership syntax while preserving nonroot mode-0700 access, 256-MiB
limits, nosuid/nodev and the existing read-only/capability boundaries. These
compatibility repairs establish no native qualification or schema activation.
No stale-store recovery, global process scan or service controller is added.
The local-only cgroupfs/file-events profile selects installed `/usr/bin/runc` with observed
root-owned resolved executable, version and digest, not a downloadable/cross-architecture
pin or invented Renovate owner. Existing pins and hosted/rootless selection stay intact.
Live inspection uses an opt-in opaque output/private-parser boundary; default runner
logs remain unchanged. Exact-owned numeric schema capture and independent selector
review precede a bounded container/conmon snapshot. This establishes no global absence
of namespace export, delegation or other holders, and no engine readiness.

The previous detached cancellation failed. Independent source review, exact-payload
harmless preflight and fresh-store compatibility remain mandatory before admission;
these definitions establish no native, ARM, App or updater readiness.

Existing release-plz App credential names are reused with a one-repository
contents/PR-write token and the checks/status reads required for eligibility.
Actual restricted-token compatibility and bot-triggered ordinary CI must be
qualified before each consumer activates automatic updates. Inactive producer
implementation may be admitted after its complete gates and complete main CI. No new App setup, publication, release, deployment,
workflow administration or protection bypass is authorized or assumed.

## Consequences

Producer code must pass primary/independent complete gates and complete main CI
before an immutable successor is admitted. Consumer callers activate only after their own successful native image/tool and
App/CI qualification, through an explicit seed/adoption. The producer seed holds
hosted feature proposals until that activation, preserving trusted
old-policy checks and unknown-change complete behavior. A producer success does
not prove rollout or native acceptance. Failed official generation may leave a
red Renovate artifact-error PR, but required coherent-graph/image/age checks must
prevent merge and bounded scheduled retries provide actionable diagnostics.

The native pruning behavior is version-specific and regression-tested against
the existing pinned runtime. Upgrading Renovate or the official CLI requires
requalifying their contract. No copied upstream implementation is introduced;
Renovate remains the existing AGPL-3.0-only external tool, and the official CLI
is MIT-licensed npm software with recorded registry integrity.

## Alternatives

A custom lock-record regex cannot regenerate transitive graphs. Hosted Renovate
alone has no trusted command hook for this owner. Manual lock regeneration stalls
automation. A generalized GitHub controller or a strict prohibition on any red
partial PR adds unnecessary machinery; required coherent candidate checks and a
narrow bounded maintenance helper supply the necessary merge boundary.
