# ADR 0057: Exact run ownership for live Podman outer storage

- Status: accepted
- Date: 2026-09-26
- Builds on: [ADR 0037](0037-finite-podman-input-and-live-conformance.md) and
  [ADR 0047](0047-bounded-migration-readiness-tiers.md)

## Context

The reviewed outer compatibility images declare persistent container-storage paths. Podman may
allocate an anonymous host volume for each such path. Removing the outer container without volume
removal can leave a complete nested engine store behind. Historical unreferenced volumes are not
deletion authority: they may belong to another run or user.

## Decision

Before starting each outer container, the live runner inspects the reviewed image's declared
volume paths and creates one exact, run-labelled scratch volume per path. It passes those volumes
explicitly and disables implicit image-volume allocation. Every create generation gets a distinct
outer name; its volume names and expected owner are registered before creation. The same cleanup
helper handles ordinary completion, replacement, failed startup and trappable interruption.

Cleanup verifies the exact run label before removing a container or volume. It force-removes the
owned container with anonymous-volume removal enabled, then removes only its own named scratch
volumes without force and verifies absence of both kinds. Missing resources are accepted only when
the runtime positively reports absence; inspection errors and unexpected survivors fail every
profile, including smoke. Cleanup continues attempting later owned resources after one failure.
The normal `--rm` behavior remains an additional fallback, not proof of cleanup.

The runner never scans for volumes by prefix, prunes the host store, removes a pre-existing volume,
or treats another run's label as ownership. Temporary socket bind mounts and application data
remain outside the scratch-volume deletion list. The local lifecycle regression uses a fake
engine; bounded rootful and rootless live checks establish actual Podman behavior. No operational
pin moves or changes in this decision, so existing Renovate extraction remains unchanged.

## Limits

Shell traps cannot execute after SIGKILL, power loss, or a host crash. The run-labelled volumes
make a future explicit recovery tool possible, but this runner does not silently recover old
resources at startup. A volume manager that creates an object despite a failed create command
without preserving its requested label cannot be safely identified; such a failure requires
manual evidence review. Historical storage remains untouched.

## Exact-candidate hosted evidence

The manual `cleanup-regression` dispatch is a bounded check for changes to the outer storage
lifecycle. Supply the reviewed branch with `--ref` and its 40-character commit as `expected_sha`;
admission refuses a different event commit. On disposable hosted runners it repeats each reviewed
6.1 rootful/rootless nested smoke cell twice on one rootful host store, checking that volume
identities and run-owned containers/volumes return to baseline after each repetition. It also runs
the image-volume removal probe in both rootful and rootless host Podman stores. Logs identify host
version and mode, image digest, binary digest, run ID, and attempt. Pull request events cannot
trigger this privileged job; it neither publishes nor deploys and cannot replace the complete
release gate. The probe cannot establish trap cleanup after SIGKILL or power loss and does not
touch historical unlabelled volumes.
