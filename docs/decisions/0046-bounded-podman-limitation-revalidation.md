# ADR 0046: Bounded Podman limitation revalidation

- Status: accepted
- Date: 2026-09-09
- Builds on: [ADR 0037](0037-finite-podman-input-and-live-conformance.md) and
  [ADR 0042](0042-bounded-podman-creation-evidence-and-intent-promotion.md)

## Context

The reviewed live matrix retains five rootless openSUSE and UBI images whose nested Podman cannot
initialize because `newuidmap` loses usable helper privileges. New images were published after the
container recipes changed, but a floating tag or successful `podman info` alone cannot replace an
accepted matrix digest. The old failure and the replacement's complete resource behavior must be
compared without conflating image compatibility, host-kernel capability, and BoxFerry input or
output compatibility.

The replacement decision changes two accepted evidence ledgers: the immutable image in
`matrix.tsv` and the corresponding exception in `limitations.tsv`. A live job must not turn a
transient result into an unaudited support claim by editing either file.

## Decision

Keep candidate execution and candidate admission separate.

`fixtures/conformance/podman-live/candidates.toml` is a temporary versioned catalogue. Its five
entries bind an accepted baseline to one replacement digest, exact container-repository revision,
publication time, license, redistribution policy, and hashes for the image definition, platform
recipe, and runtime configuration. The resolver rejects a candidate not exactly paired with a
current `helper-privilege-collision` row. It also rejects repository, tag, matrix metadata, role,
path, digest, or candidate-set drift before pulling an image.

The `limitation-revalidation` profile runs one candidate at a time. On the same disposable amd64
root runner it first pulls the accepted digest and reproduces the exact bounded rootless helper
collision, including helper modes and capabilities. It removes that outer container, then pulls the
candidate and verifies its immutable digest and OCI source evidence. The candidate uses new outer,
socket, graph-root, and resource-name identities. It must complete production acquisition, every
selector through Compose, Quadlet, and Podman, diagnostic failures, privacy checks, Compose and
Quadlet re-imports, SELinux relabel intent, and isolated external apply/reacquire. Cleanup is a
required result, including failure paths.

One manual `workflow_dispatch` workflow accepts only a catalogue candidate or `all`, refuses any
non-default-branch revision, builds BoxFerry once, and runs candidates serially. It has read-only
repository permission, pinned actions, no credentials, a bounded inner timeout, a longer job
timeout, and non-cancelling global concurrency. It uploads only the schema-1 evidence document
after validating it against the exact workflow commit and candidate. A failing command, invalid
evidence, timeout, signal, or cleanup failure remains a failed job; artifact upload cannot turn it
green. There is no scheduled or virtual-machine lane.

Evidence records catalogue hashes; declared and observed baseline and replacement identity; source
proof; fresh-store proof; granular resource, selector/exporter, privacy, re-import, external-apply,
and cleanup results; execution boundary; and a bounded failure classification. A document is
eligible only when every required result is true. Cgroup and kernel capability observations belong
to the shared host. SELinux enforcement and systemd execution remain explicitly unperformed.
Initialization-failure evidence binds the requested candidate, repository commit, and catalogue
hashes whenever they are recoverable. A deliberately unbound parsing failure is schema-valid for
local diagnosis but cannot validate against a valid workflow invocation.

After human review of all five exact-commit artifacts, a separate admission change may replace only
passing matrix digests and delete only their matching limitations. Retained limitations keep their
current immutable reproducer. The admission change records reviewed run and artifact identities;
the workflow never mutates accepted support data.

## Consequences

- A replacement cannot silently acquire resource coverage from a pull or version check.
- Failures remain useful and privacy-safe without committing raw output or host identifiers.
- Distro userspace and Podman/API/package evidence are explicit; shared-host kernel, SELinux,
  systemd, other architectures, and virtual machines remain outside the claim.
- Candidate entries are removed after all decisions are admitted or retained. The matrix and
  limitation ledger remain the published compatibility source.

## Alternatives considered

- Trusting the new tag was rejected because tags do not bind reviewed bytes or source provenance.
- Testing only the replacement was rejected because it would not reproduce the recorded limitation
  under the same runner boundary.
- Updating matrix rows in the workflow was rejected because a transient privileged job does not
  perform reviewed repository admission.
- A nightly or virtual-machine matrix was rejected because this is a finite digest decision.
