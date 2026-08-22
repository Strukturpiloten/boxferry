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
- [x] Merge the Phase 1 completion [PR #46](https://github.com/Strukturpiloten/boxferry/pull/46)
      for [issue #45](https://github.com/Strukturpiloten/boxferry/issues/45), then merge the
      release-plz [PR #44](https://github.com/Strukturpiloten/boxferry/pull/44) and release
      BoxFerry 0.6.0.

Phase 1 exits when every current route follows the same public orchestration path and the test
harness fails whenever an importer fixture lacks an expectation for a registered exporter.

## Phase 2 — Podman integration

- [x] Reintroduce `boxferry-podman` without legacy APIs, behavior, compatibility shims, or migration
      documentation.
- [x] Implement a PodmanLens 0.1.1 importer that explicitly acquires a read-only inventory and
      resource graph, handles every observation state and origin, and maps only authorized intent
      into the neutral model.
- [x] Implement a neutral-model Podman exporter that plans and renders complete inert
      `podman.json` deployment-v1 and `review.sh` artifacts. BoxFerry never applies or deploys
      them.
- [x] Add Podman CLI input and output, both named `podman`. Input requires one explicit Unix socket,
      application name, and discovery selectors; output requires explicit target context.
- [x] Default Podman output to reviewed exact version 6.1.0. Resolve `--podman-max-version` to the
      newest reviewed exact target not greater than its ceiling over 5.4.0, 5.5.0, 5.6.0, 5.7.0,
      5.8.6, 6.0.0, and 6.1.0.
- [x] Expand the route matrix to all nine Compose, Quadlet, and Podman routes. Every importer
      fixture must run through every exporter.
- [x] Build immutable PodmanLens-backed fixtures for all seven reviewed exact versions in both
      simulated rootful and rootless contexts. Verify version-specific import evidence, default
      6.1.0 output, explicit target context, and every
      `--podman-max-version` boundary without depending on unavailable runtime containers.
- [x] Add complex rootful and rootless inventories covering pods and standalone containers,
      isolated network borders and intentionally shared networks, named volumes, bind mounts,
      cross-resource dependencies, secrets and redaction, and partial, disappeared, malformed,
      ambiguous, or conflicting resources.
- [x] Cover tmpfs mounts from authored neutral, Compose, and Quadlet intent through Podman export.
      PodmanLens 0.1.1 runtime input cannot observe tmpfs and BoxFerry does not claim reconstruction.
- [x] Run every Podman import scenario through the Podman, Compose, and Quadlet exporters. Re-import
      generated Compose and Quadlet artifacts and assert semantic equivalence, deterministic fixed
      points, path consistency, losses, diagnostics, and redaction.
- [x] Verify Podman output independently through deterministic `podman.json` and `review.sh` bytes,
      semantic operation equivalence, complete findings, and redaction. It is desired deployment
      intent, not a re-importable observed inventory.
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
- [ ] Complete BoxFerry Podman route guides with executable, corpus-backed examples.

## Phase 4 — publish boxferry.dev

- [ ] Decide and document hosting, DNS, TLS, security headers, deployment authorization, and
      rollback.
- [ ] Add a production workflow that builds from exact locked documentation revisions.
- [ ] Publish and verify `https://boxferry.dev`.

## Deferred

Docker and DockerLens integration remain deferred. BoxFerry does not add a placeholder Docker
adapter while the Podman-first roadmap is active.
