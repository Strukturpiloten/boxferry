# ADR 0091: Trusted-main Paperless custody mechanism

- Status: accepted
- Date: 2026-11-02
- Owns: BoxFerry #530, following the source preparation reviewed under #529
- Amends only: [ADR 0078](0078-trusted-docker-consumer-validation.md)'s
  volume-only dispatch boundary to permit the fixed main-only mechanism below
- Preserves: [ADR 0079](0079-native-test-runtime-selection.md), existing candidate
  pair admission, product non-execution and all application acceptance boundaries

## Context

The six-directory custody code needs an actual root owner, direct rootless
dockerd peer, genuine proc-root view and storage bind. Offline process and FD
tests cannot establish those native preconditions. The prepared feature source
also changes Rust contracts beyond published SDK dependencies. A Python-only
mechanism must be independently runnable from coherent main without publishing
that feature or an incomplete SDK milestone.

## Decision

Add one closed `paperless-custody-mechanism` task to the existing trusted consumer
workflow. It runs only the actual clean default-main workflow SHA. Both real
actors need fresh write/maintain/admin permission in BoxFerry and DockerLens.
The existing read-token creation, immediate revocation barrier and narrow
credential-free worker environment apply before execution and again before
independent evidence validation. No feature PR, command, provider, PID or storage
path selects this root code. Existing candidate modes retain their exact
open-PR-pair checks and three-binary transfer contract.

The owner/deadline infrastructure is extracted without the prepared product
Rust, semantic projection, scenario or loader changes. Main Cargo manifests,
lockfiles, application schedule and catalogue remain unchanged. The original
fourteen-path source authority remains complete, including passive
`docker-paperless-transport.py`; its application constructor is never invoked by
the mechanism. The legacy custody union remains thirty-two files, formed by
the union of the original twenty-three candidate paths, fourteen guard paths and
fixed launcher.
That historical union is not the trusted-main mechanism closure.

The first main mechanism attempt exposed an incorrect assumption in the original
copy-closure decision: four candidate Rust example/test paths and the candidate
core-reacquisition expectations file are absent from the actual tracked main
tree. Requiring those files rejected clean main before native execution. Correct
that boundary with a separate explicit twenty-seven-file main closure: the same
fourteen guard paths, the fixed launcher and twelve reviewed main support paths.
It neither filters by path existence nor depends on future candidate-list changes.
Missing main members still refuse execution. The candidate twenty-three-file
admission set and legacy thirty-two-file union remain unchanged.

The trusted worker authenticates the explicit main closure before root import.
The guard reauthenticates its original fourteen-path roster and compiles retained
module bytes; callback digests cannot authorize code. A regression uses actual
tracked source bytes in a clean temporary Git snapshot and the unmocked production
main-source gate; fabricated declared-file fixtures alone are not this evidence.

The immutable DockerLens helper dependency is
`ec58b844a46c881ea175bccae85a049f609b43c1`. Its full source and lock digests and
eight consumed helper hashes are held in the trusted integrity record. No SDK
Rust is built or executed. This deliberately does not follow pending diagnostic
source or qualify its native behavior. The existing candidate Rust build path
still has its source override when that separate work needs it.

Renovate owns `CUSTODY_HELPER_REVISION` through a distinct exact extraction. Its
temporary tracked ref is truthfully `TheRealBecks/issue172`, with the existing
manual DockerLens dashboard approval and `automerge: false` policy. A digest
proposal alone fails integrity validation: the full source/lock/eight-file record
must be reviewed and refreshed together. The existing #343/#173 milestone
handoff must transition this tracked ref to reviewed main or release **before**
deleting that infrastructure branch. This creates no release authority and no
permanent unattended freeze. The admission helper's independent main pin and
historical authorization/evidence records remain unchanged.

## Native lifetime and authority

The fixed launcher is the original root owner session before preparation and
guard creation. It uses the existing canonical outer Docker helpers and temporary
rootless runner prerequisite, one original data volume and exactly the original
data/socket mounts. Canonical fixture creation uses a root-owned `0700` parent
and nested root-owned `0777` socket directory. Acquisition never repairs modes,
ownership, mappings or host policy. Legacy owners still require their original
`0700` socket directory and plain-user behavior.

Only six literal empty inner local volumes are created, with exact run/application
labels, local driver and no options. No application or BusyBox image, generated
plan, target recreation or product CLI executes. The guard alone retains the six
storage capabilities. Separate actual root callback processes use the production
hold/check/close channel, original challenges, sequences and binding.

The already existing trusted worker is the read-only observer. Before allowing
creation or inducing loss, it pins the actual Popen owner and original guard
incarnations, checks the real guard invocation and sealed custody, and retains
the canonical outer daemon identity. After hold/check it corroborates the six
real held role objects and current root/namespace/graph objects. Temporary
metadata traversal uses only the fixed actual daemon proc-root transition and
no-follow components; it retains no storage directory and enters no namespace.
No broker, guest helper, extra holder, mount, descriptor-transfer channel or guard
operation is added.

Each replacement control uses a fresh original owner/lease. The worker observes
the old held object and the changed current role object before the failing check.
Finite private request receipts establish the actual callback request. Lost ACK
also requires a challenged response receipt before intentional missing confirmation.
These receipts are observations, never new code or cleanup authority.

Original guard poisoning/expiry can kill the original owner. That owner is not
expected to attest after death. The worker must already hold authentic identities;
a mutable or incomplete journal cannot grant authority. It requires actual owner
and guard exit, FD closure before core removal, canonical exact-resource absence
and original private-directory absence. It performs no teardown. Only the existing
guard/original live owner has original exact-owned recovery authority. Missing
proof is refusal with retained residual obligations, not native success.

Expiry means the already authenticated original guard selected its existing lease-expiry
recovery branch. It does not claim an exclusive operating-system cause of owner death.
Cancellation, observed owner loss and the original global deadline take priority and
cannot emit that attribution. A later verifier or cleanup clock is never expiry proof.
The guard records its actual decision, original custody/source/guard incarnation,
lease binding and sequence, original monotonic and wall cutoffs, and decision clocks
in one private root-owned `0600` no-follow, exclusive-create observation of at most
4096 bytes. Its path is a fixed sibling of the original owner directory, so original
cleanup cannot erase the proof before the existing worker retains and validates it.
The guard first preserves the original kill and lease-close order, then writes a
private pending file and publishes it without replacement only after successful
write, flush and close. Any observation fault leaves expiry unproved while original
cleanup continues. Pending data is a refusal residual, never qualifying evidence.
The worker checks the exact original identities and cutoffs before removing only
that observation data. Missing, stale or mismatched proof refuses expiry success.
The original recovery, cutoffs and kill behavior are unchanged; no new RPC, actor,
storage holder, namespace entry or retained descriptor is introduced. The actual
observation write and verifier read are charged within the original byte allowance.

## Bounds and evidence

Read-only Unix-socket connects can update the socket pathname's access clock.
The guard acquisition and launcher peer checks therefore compare the same nine
security fields: device, inode, full mode, UID, GID, link count, size, and
nanosecond modification/change times. Only access time is excluded. Exact kernel
peer credentials, original process identity, no-follow pathname/held-descriptor
binding, call order and resource limits remain unchanged. Private regular-file
reads keep `O_NOATIME` and their existing complete stat equality. The two socket
checks use local pure tuples to avoid new runtime imports or authority reads;
an independent cross-consumer regression fixes their common field contract.

The Linux mechanism reads owned private files with `O_NOATIME` on the existing
read descriptor. Reading must not change its own access timestamp: complete
before/after descriptor and pathname stat equality remains mandatory, including
inode, mode, owner, links, size, modification and change metadata. A denied or
unsupported flag refuses the read without fallback or retry. This adds no
privilege, descriptor, runtime operation or alternate file authority.

Keep the worker's 45-minute backstop, original 780-second execution and 120-second
cleanup, existing two-CPU/4-GiB/512-PID outer policy, two mounts, seventeen retained
and conservative thirty-six holder/frame FDs, 130-byte request read and 4096-byte
reply. A fixed earlier lease cutoff exercises expiry without extending an owner.
The work budget stays 256 calls and 16 MiB. Whole-check/read reservations are
availability checks; only actual observed usage is charged and never refunded.
Unacknowledged negative attempts retain conservative upper bounds and poison
remaining use. Original lifecycle cleanup keeps its existing independent deadline;
it is not another work lease.

Every negative case first requires successful real hold/check and observed role
objects. Operationally run the standalone positive case first, then the six
replacement controls and owner/guard loss, expiry and lost ACK. Namespace/LSM,
peer/proxy, mapped credential, proc-root, bind or timing failures stop at that
precondition; there is no per-read sudo or privilege fallback.

Only bounded trusted-validated summaries are uploaded. They bind actual main,
SDK/helper and copied-source identities, real run/job/attempt/actors, owner/guard/
daemon and callback incarnations, actual object identities, counters and cleanup.
Refused records identify only already retained resource obligations. Raw logs,
proc data and custody remain private. Scope is always mechanism-only: zero
applications accepted, no full driver, target recreation or native SDK qualification.
The post-producer receipt handoff inspects only the fixed staged custody directory
and its single bounded canonical JSON receipt as root. It binds no-follow held
descriptors to each pathname component, the original runner-owned `0700` staged
parent, root-owned `0700` publication directory and root-owned single-link `0600`
regular receipt. A closed metadata envelope preserves success versus refusal;
it does not repeat native proof validation or grant cleanup authority. Ownership
transfers on the held receipt descriptor first and publication descriptor last,
with metadata/path rechecks and unchanged modes. Missing records, substitutions,
extra entries, invalid metadata or partial handoffs refuse without retry. Only a
successful handoff permits upload, even when the producer failed and retained a
refusal record. The credential-free trusted-main helper uses an isolated Python
interpreter with explicit bytecode suppression (`-I -B`), excluding working-directory
and Python environment module authority. It emits only a fixed error;
it neither widens traversal modes nor reads private logs or bindings.

Offline tests and the complete exact-source gate remain mandatory. The manual
mechanism task is not future Docker/full-application automation. Main/PR/Release
gates retain their current authority; no release, publication or deployment is
authorized here.

## Consequences

The manual task can establish only the bounded custody mechanism on the recorded
main/helper pair. Source tests, the complete repository gate and actual native
controls are separate evidence; none substitutes for another. Product and
application qualification remain unchanged.

## Alternatives

Reject publishing incomplete SDK work to run a Python test, copying the prepared
feature overlay onto main, narrowing the original guard roster, changing existing
candidate admission into a generic dispatcher, and treating a health/proxy check
or saved JSON as a live directory capability. Keep the source-only and native
acceptance records distinct.
