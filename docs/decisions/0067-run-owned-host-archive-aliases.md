# ADR 0067: Run-owned host archive aliases with stable nested references

- Status: accepted
- Date: 2026-10-05
- Builds on: [ADR 0043](0043-bounded-forgejo-live-application-acceptance.md),
  [ADR 0055](0055-parallel-readiness-and-local-quality-feedback.md), and
  [ADR 0057](0057-run-owned-podman-live-storage.md)

## Context

The live runner prepares offline archives in a shared host image store. Using stable fixture
references as host archive tags collides with retained tags from earlier invocations or other
owners. A collision is not permission to overwrite or delete that historical tag. Changing the
fixture references instead would change acquired application intent and historical cassette
expectations rather than fixing the host ownership boundary.

## Decision

The common workload, Forgejo, Nextcloud, Paperless, Immich and observability archive paths use
host-only `localhost/boxferry-archive/<run>/<suite>:<id>` aliases. Run tokens are lowercase bounded
tokens, suites are closed, and image IDs are bounded single tag components. Before any tag,
confirmed absence is mandatory. Present and unknown results never authorize creation.

One successful narrow source inspection binds the pinned manifest digest and immutable image ID.
The alias and expected ID are registered before tagging that immutable ID, so a nonzero partial
tag still has an exact cleanup candidate. Successful acquisition additionally requires matching
alias ID readback. The same binding is rechecked before archive save and retained for nested
loading. A source image pulled after confirmed cache absence also retains its exact reference
and verified immutable ID; a pre-existing cached source never becomes run-owned.

Only the explicitly registered disposable outer target with an authenticated exact run label and
the immutable container ID returned by successful creation may load the archive. The runner
validates and records creation stdout before loading; an absent binding or even a same-run-label
name replacement is refused. Before API activation or provisioning, loaded image
IDs must match the host ledger; the runner tags those immutable IDs with the original stable
fixture references and verifies them. It removes the host-only tag inside that target without
force or pruning. Thus dynamic host tags do not enter acquired nested image metadata, fixture
definitions, or historical cassettes. Nested loading does not access a registry.

Host cleanup requires confirmed presence and matching immutable image ownership before exact,
non-force, non-pruning removal, then confirmed absence. Confirmed initial absence clears the
obligation without removal. Unknown observations, identity drift, failed removal and survivors
fail cleanup, preserve ambiguous resources and private recovery evidence, and do not stop attempts
on later registered resources. Existing failure status, runtime/profile budgets, application
assertions and software pins remain unchanged.

Supabase is intentionally excluded: its direct digest-preserving Skopeo registry-to-OCI path does
not create host image-store aliases. Its stable archive names, integrity validation and nested
load contract remain unchanged. No software pin or Renovate extraction definition moves.

The canonical `scripts/test-application-probes.sh` offline wrapper now registers both
`test-native-presence.py` and `test-podman-live-cleanup.sh`. Its existing consumers cover local
`check-all`, hosted PR/main/dispatch CI, and Release CI reuse. Standalone local cleanup and
outer-storage registrations remain unchanged. No workflow or software pin changes are needed;
run-specific archive aliases are not software pins and introduce no Renovate manager.

## Limits

Host inspection, tagging, saving and named-tag removal are not one atomic runtime transaction.
Unique run aliases prevent normal independent invocations from sharing a tag. They cannot prevent
an arbitrary external writer from retagging the exact same alias between a check and mutation.
Readbacks detect observed drift but do not establish atomic exclusion; no global lock, prune,
historical recovery or deletion authority is introduced. Traps cannot run after SIGKILL or host
failure, and surviving tags require explicit evidence review rather than automatic next-run
cleanup.

Offline fake-engine tests establish ownership, error propagation and stable-reference ordering,
not native image-format compatibility or application acceptance. New native evidence must bind
the changed candidate; existing receipts and captured cassette bytes remain historical.
