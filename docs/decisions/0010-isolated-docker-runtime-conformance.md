# ADR 0010: Isolated Docker API conformance in a nested daemon

- Status: superseded
- Date: 2026-08-04
- Superseded by: [ADR 0032](0032-future-native-lens-boundaries.md)

## Context

Pure authored fixtures protect stable decoder behavior but cannot prove that a real Docker Engine
produces usable inspect responses. Running tests against a developer's default daemon would make
the version, state, privileges, resource ownership, and cleanup ambiguous. The local `docker`
command is also provided by `podman-docker`, so it is not Docker Engine evidence.

The Docker Engine API is version-negotiated. A current daemon can emit compatibility responses for
the reviewed API 1.40 floor and the current 1.55 ceiling when `DOCKER_API_VERSION` is forced. This
is valuable compatibility evidence, but it is not equivalent to exercising the historical Docker
19.03 implementation that originally introduced API 1.40.

## Decision

Keep the exact Engine release, official image reference, immutable digest, reviewed API values,
and provenance in `tools/docker-runtime-matrix.toml`. Run the official Docker 29.7.1 `dind` image
as a privileged ephemeral container through a caller-selected outer Docker or Podman engine. Start
a private nested daemon inside it, create only uniquely prefixed resources, capture explicit
container, image, network, and volume selectors at API 1.40 and 1.55, decode them, then remove the
outer container and temporary host evidence.

The harness mounts only its read-only script and a unique temporary evidence directory. It does
not mount a host Docker/Podman socket, repository write path, home directory, or credential store.
The nested daemon has no persistent volume. Pulling the digest-pinned official image and granting
privilege remain explicit opt-in operations locally. A separate weekly/manual GitHub workflow runs
the same command on disposable hosted runners.

The matrix test runs during normal CI and fails if API bounds, upstream release signal, image tag,
digest, or provenance drift. Renovate may propose an Engine/image update, but the decoder ceiling
and conformance claim change only after fixture, source, and live review.

## Consequences

- Real current Docker Engine responses exercise both finite decoder bounds without using ambient
  resources.
- Podman 6.0.2 may safely act as the outer container engine; it is not treated as the inspected
  Docker implementation.
- API 1.40 evidence proves current-daemon downgrade behavior, not every historical 19.03 daemon
  quirk. A historical-engine lane may be added separately if it is secure and reproducible.
- The privileged boundary is isolated from host runtime sockets and normal pull-request CI.
- No Docker Engine installation is required on developer machines.

## Evidence

Docker documents the current 1.55 and minimum 1.40 API versions plus forced version selection in
the [Engine API reference](https://docs.docker.com/reference/api/engine/). The
[API version history](https://docs.docker.com/reference/api/engine/version-history/) documents its
open-schema and compatibility changes. Docker's
[29.7.1 release notes](https://docs.docker.com/engine/release-notes/29/#2971) identify the reviewed
current Engine release, and the official [`docker` image](https://hub.docker.com/_/docker) provides
the `dind` distribution used by the digest-pinned lane.

## Alternatives

Using the host daemon directly was rejected because version, state, and cleanup would be ambient.
Treating `podman-docker` output as Docker evidence was rejected because it exercises Podman's
implementation. Running privileged nested Docker on every pull request was rejected because the
additional privilege, network pull, and runtime variability belong in a separate conformance tier.
Maintaining only static fixtures was rejected because response drift would never meet an actual
daemon.
