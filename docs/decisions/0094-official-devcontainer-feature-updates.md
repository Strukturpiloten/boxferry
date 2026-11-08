# ADR 0094: Official Dev Container feature updates and candidate qualification

- Status: proposed; producer implementation and consumer admission are separate
- Date: 2026-11-04
- Amended: 2026-11-06; conventional VS Code local development and standard hosted storage
- Amended: 2026-11-08; scoped metadata reader and Python-only queue authority
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
require all seven real credential-free recorded checkouts. Fresh hosted runners
use standard runtime storage; actual DockerRootDir or Podman graphRoot must share
the existing disk budget filesystem without resetting its initial baseline. Only
reviewed verifier/probes execute, not lifecycle commands or cargo
fetch hooks. Image tool versions must match selected canonical automation. Fixed
FCOS, Podman/Docker runtime, workload and provider qualification pins remain
unchanged. Cold resource/budget qualification and issue 446 ARM workspace evidence
remain required and are not established by this ADR.

Hosted Podman reuses the reviewed Ubuntu signed-APT outer prerequisite exception, recording
actual `dpkg` package/version, UID-0 and rootful observations separately from fixed inner qualification pins.
Before consumer activation, cold storage budget proof must cover the actual Docker daemon
filesystem; producer definitions alone establish no measured resource or native evidence.

The maintainer's 2026-11-06 decision supersedes the earlier explicit local-rootful
private-store qualification design. Local development uses VS Code Dev Containers
with the developer's existing Podman connection and storage, preferably rootless.
Normal caches are retained. There is no custom local qualifier, mandatory host
sudo, graphroot override, PID/mount lifetime, live PID selector, installed-runc
custody profile or special-inode finalization requirement. The ordinary complete
local gate retains its Rust/Python/repository checks without a new image-build leaf.

Image qualification remains a hosted-CI producer: native Linux x64/ARM64 lanes use
reviewed explicit backends on fresh GitHub-hosted runners with standard runner
storage. Hosted rootful Podman's existing signed-APT prerequisite and privileged
child deadlines remain; clean engine environment cannot redirect its store.
Narrow metadata observation proves actual store disk coverage. The producer
removes only its nonce-labelled exact containers and unique image tag after
identity checks. Preexisting objects and caches are allowed; no global prune,
store removal or namespace controller exists. Failed builds without a final tag,
probe errors and cancellation retain their primary bounded redacted diagnostics
and attempt bounded owned-resource cleanup. Runner disposal owns residual cache.

Earlier private-store failures and source admission, including PR #599 at
`30ff1a56b745cf68761f7ebe30caa3e1d25c5764`, remain historical evidence of the
superseded implementation, not qualification of this replacement. Already-retained
failed stores remain untouched; separate custody review is required for cleanup.
Issues #578/#575 track retirement and diagnostics; #563 still requires actual
VS Code reopen/rebuild, every owner's native image/tool/failure evidence, and App
acceptance before image/updater activation. Fixed FCOS/runtime/application targets
and separate #446/#453 acceptance remain unchanged.

Existing release-plz App credential names are reused with a one-repository
Contents/Pull requests-write token, without App Checks or Commit statuses grants.
A distinct repository-scoped workflow token has only Contents, Checks and Statuses
read permissions; callers and the reusable workflow declare the same read scopes.
The Python writer and fixed held-head check/status reader share one 60-attempt
API budget. The reader refuses redirects and validates complete resource/head
identity before required-context success can authorize `expectedHeadOid` queue
admission. Both feature streams retain this path and fresh App commits wait for
ordinary CI before a later unchanged pass can enqueue them.

The limited native instance disables both automerge modes and all four optional
status publishers; it may propose updates but cannot merge, enqueue or enable
auto-merge. Three-day age filtering, strict internal checks and canonical trusted
eligibility remain unchanged. Its private config routes only initial current-repository
commit-namespace read requests to the workflow token, with abort-on-error and no
ignored status codes. Other requests, including GraphQL and writes, retain the App
credential. The pinned transport can retain the read token on redirects within
exactly `https://api.github.com`; this trusted-origin exception does not expand
the token's repository/read permissions or establish response resource identity.
Cross-origin redirects strip credentials. Native metadata is advisory and can
never replace Python's independent no-redirect exact-head queue evidence.
Neither token reaches official generation or candidate hooks; secret-bearing
native config stays in private temporary storage and is removed on every exit.
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
