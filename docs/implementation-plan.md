# Implementation plan

BoxFerry is an N:N conversion and planning tool. Every native input passes through its importer into
the neutral `Application` model, and every native output is produced from that model by its
exporter. Loss policy authorizes inert artifacts only after import and export outcomes are combined.

BoxFerry never applies generated artifacts, invokes generated commands, or sends mutating runtime
API requests.

## Phase 1 — neutral conversion foundation

- [x] Correct Compose-to-Compose so it uses
      `ComposeImporter -> Application -> ComposeExporter`. The same-format shortcut and its public
      BoxFerry API were removed in [PR #43](https://github.com/Strukturpiloten/boxferry/pull/43).
- [x] Enforce the neutral-model pipeline as an invariant for every route and derive route coverage
      from independent importer and exporter dimensions.
- [x] Build the current two-by-two scenario matrix. All seven existing positive fixture manifests,
      containing nine Compose and Quadlet import scenarios, run through every registered exporter.
- [x] Add direct, repeated, and chained Compose and Quadlet re-import; fixed-point,
      semantic-equivalence, path-consistency, policy-lattice, diagnostic, output-safety, and redaction
      tests.
- [x] Fix neutral-model and adapter gaps exposed by the scenario corpus, including deterministic
      quoted assignment re-import, resource-name preservation, health/readiness ordering, and pod
      runtime preservation.
- [ ] Merge the Phase 1 completion pull request for
      [issue #45](https://github.com/Strukturpiloten/boxferry/issues/45), then review and merge the
      release-plz release pull request.

Phase 1 exits when every current route follows the same public orchestration path and the test
harness fails whenever an importer fixture lacks an expectation for a registered exporter.

## Phase 2 — Podman integration

- [ ] Reintroduce `boxferry-podman` without legacy APIs, behavior, compatibility shims, or migration
      documentation.
- [ ] Implement a PodmanLens-backed Podman importer into the neutral model.
- [ ] Implement a Podman exporter from the neutral model. Its output is inert; BoxFerry never
      applies or deploys it.
- [ ] Add Podman CLI input and output, both named `podman`.
- [ ] Default Podman output to the newest version reviewed by the linked PodmanLens release,
      initially 6.1, with an explicit `--podman-max-version` override.
- [ ] Expand the route matrix to all nine Compose, Quadlet, and Podman routes. Every importer
      fixture must run through every exporter.
- [ ] Build immutable PodmanLens-backed fixtures for every reviewed Podman minor from 5.4 through
      6.1. Verify version-specific import evidence, default 6.1 output, and every
      `--podman-max-version` boundary without depending on unavailable runtime containers.
- [ ] Add complex rootful and rootless inventories covering pods and standalone containers,
      isolated network borders and intentionally shared networks, named volumes, bind mounts, tmpfs
      mounts, cross-resource dependencies, secrets and redaction, and partial, disappeared, or
      conflicting resources.
- [ ] Run every Podman import scenario through the Podman, Compose, and Quadlet exporters. Re-import
      every supported generated artifact and assert semantic equivalence, deterministic fixed points,
      path consistency, losses, diagnostics, and redaction.
- [ ] Release the reviewed three-by-three converter. A future live-runtime workflow may repeat the
      version matrix in rootful and rootless containers when all required Podman images are reliably
      available; it is not a Phase 2 prerequisite.

Podman integration proceeds independently of Docker. PodmanLens owns Podman acquisition, native
types, version and capability evidence, diagnostics, and deterministic rendering. BoxFerry owns
semantic mapping, combined fidelity policy, orchestration, and inert artifact publication.

## Phase 3 — Podman documentation

- [ ] Add public PodmanLens guides covering read-only acquisition, discovery, grouping, planning,
      rendering, diagnostics, privacy, and version boundaries.
- [ ] Add exact-revision PodmanLens documentation and Rustdoc to boxferry-website.
- [ ] Add BoxFerry Podman route guides and executable examples.

## Phase 4 — publish boxferry.dev

- [ ] Decide and document hosting, DNS, TLS, security headers, deployment authorization, and
      rollback.
- [ ] Add a production workflow that builds from exact locked documentation revisions.
- [ ] Publish and verify `https://boxferry.dev`.

## Deferred

Docker and DockerLens integration remain deferred. BoxFerry does not add a placeholder Docker
adapter while the Podman-first roadmap is active.
