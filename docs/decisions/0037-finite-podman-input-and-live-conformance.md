# ADR 0037: Finite Podman input compatibility and live conformance

- Status: accepted
- Date: 2026-08-25
- Amends: [ADR 0018](0018-generic-cli-and-diagnostic-support-bundle.md),
  [ADR 0021](0021-automatic-local-error-report-names.md), and
  [ADR 0034](0034-podman-lens-adapter-boundary.md)

## Context

Podman input must help migrate workloads still running on supported distribution packages,
including Podman 3.x and 4.x. That is a different claim from generating commands for an old Podman
target. Synthetic fixtures also did not expose socket limits, unstable health observations,
version-specific topology behavior, or malformed live responses. Diagnostic evidence is useful for
reproducing those failures, but raw runtime inventory is too sensitive for a default support bundle.

## Decision

1. BoxFerry depends on PodmanLens 0.2.1. PodmanLens owns the finite input capability catalogue. Each
   entry records one accepted Podman interval, its minimum Libpod API, reviewed observation version,
   and whether that line is also an output target. `boxferry capabilities` exposes this machine
   evidence. Unknown versions fail closed.
2. Input and output compatibility remain separate. Finite input includes reviewed distribution
   anchors 3.0.1, 3.4.4, 4.3.1, 4.9.3, and 4.9.4 plus reviewed 5.4–6.1 lines. Podman deployment output
   remains the exact 5.4.0–6.1.0 target catalogue from ADR 0034. Legacy input may therefore migrate
   through the neutral model to a modern target without claiming old-version output support.
3. ADR 0036 governs CLI-only conventional socket discovery, optional application identity, exact
   roots, literal name prefixes, label roots, and all-resource selection. The reusable facade keeps
   caller-selected read-only transport acquisition.
4. `--include-podman-snapshot` is opt-in, available only with a generated error report on Podman
   input routes. It adds always-redacted inventory, discovery graph, and value-free acquisition
   findings. Environment values, protected health commands, credentials, secret payloads and driver
   values, label values, unknown raw JSON, and connection endpoints are omitted. The redaction count
   covers serialized markers and omitted values, not distinct source secrets.
5. These snapshots are diagnostic serialization only. They are not Podman input, replay cassettes,
   deployment plans, or authorization to mutate a runtime. ADR 0021 publication safety and ADR
   0035's inert BoxFerry output boundary remain unchanged.
6. Live conformance uses all 48 reviewed digest-pinned rootful/rootless images as disposable
   container cells. The shared runner creates isolated small and large workloads, exercises every
   selector and exporter, checks re-importable outputs, deterministic Podman plans, support
   redaction, and deterministic malformed/404 fault responses. The BoxFerry process remains
   read-only; only the isolated test harness executes generated commands inside a fresh disposable
   Podman 6.1 target before reacquiring and comparing the result. The runner never mounts an
   ambient host socket into a nested image. Resource creation and matching-version CLI assertions
   finish before the API service starts; after socket activation no CLI touches the same store.
7. A separate reviewed limitation catalogue prevents incomplete cells from being reported as
   resource coverage. The current UBI 8/9/10 and openSUSE Leap/Tumbleweed rootless digests combine
   package-supplied file capabilities with recipe-added setuid bits on `newuidmap`/`newgidmap`; the
   helpers then cannot open the second namespace's `uid_map`. Those five cells verify that exact
   container failure until corrected images are published. Pull requests run a representative
   nine-cell smoke matrix covering every finite legacy input anchor plus oldest/newest root-mode
   edges; manual dispatch runs all 48 container cells. There are no VM or nightly lanes.

## Consequences

- Users can migrate reviewed legacy distribution runtimes without conflating source and target
  capabilities.
- `capabilities` rather than prose is the exact installed-build compatibility ledger.
- Deterministic fixtures remain the normal gate; live evidence detects integration failures that
  fixtures cannot model.
- Retained live artifacts and opt-in support snapshots still require human privacy review.

## Alternatives considered

Accepting every Podman version in a broad major range was rejected because unreviewed response
changes could become silent loss. Treating legacy input as a legacy output target was rejected
because acquisition and deployment features have different version boundaries. Capturing raw
inventory by default was rejected because it expands support-bundle disclosure. Nightly privileged
execution was rejected while required historical images are not guaranteed to be available.
Treating an image-level helper failure as successful live-resource conformance was rejected because
it would overstate BoxFerry coverage.
