# ADR 0008: Digest-pinned nested Podman runtime conformance

- Status: accepted
- Date: 2026-08-04

## Context

Authored fixtures and source review establish a finite decoding contract, but they do not prove
that real Podman releases still produce decodable responses. Installing every supported Podman
minor line on contributors' machines would conflict with the repository's container-first
development model. Mounting a developer's Podman or Docker socket into the normal Dev Container
would also expose unrelated resources and expand the container's trust boundary.

The official `quay.io/podman/stable` registry provides immutable, digest-addressed local-runtime
images through Podman 5.8.2. It did not provide an exact Podman 6.0.2 image when checked on
2026-08-04. A source-reviewed 6.0.2 fixture is useful evidence but is not runtime conformance.

## Decision

Maintain a repository-owned runtime matrix with one exact official image for each available
supported 5.x minor line: 5.4.0, 5.5.2, 5.6.2, 5.7.1, and 5.8.2. Every image reference includes
both its immutable tag and full SHA-256 digest. A normal pull request validates the matrix shape,
decoder boundaries, version ordering, digest pins, provenance, and explicit evidence gaps without
starting a runtime.

An ignored Rust test may run each selected image as a privileged nested-Podman container through
an explicitly named outer Docker or Podman executable. The outer container receives only the
authored read-only harness and a private temporary evidence directory. It receives no host runtime
socket, repository write mount, credential, or production inspection data. Test resources use a
fixed private prefix, exist inside the ephemeral outer container, and are removed by both the
inner cleanup trap and the outer container's `--rm` lifecycle.

Run the live tier weekly on disposable GitHub-hosted runners and allow an explicit manual version
selection. Keep it outside pull-request CI. Record 6.0.2 as an open exact-runtime evidence gap
until an official immutable local-runtime image or a reviewed reproducible source-build lane is
available. Renovate may signal a new upstream release, but it cannot extend support or declare a
lane conformant automatically.

Provide a separate ignored installed-current test for callers who already have the exact reviewed
ceiling. It must require an explicit executable, verify version 6.0.2 before mutation, use a
process-unique prefix, inspect no ambient resource, and remove only its own resources. This is
current-patch runtime evidence, but it does not close the reproducible scheduled-image gap.

## Consequences

- Contributors do not install five Podman versions or make the normal Dev Container privileged.
- A live lane exercises actual container, image, network, volume, pod, and infrastructure-container
  response shapes without reading host workloads.
- Digest changes and new upstream releases require an explicit evidence review.
- The privileged outer container is suitable only for trusted, digest-reviewed images and
  disposable environments; local execution remains opt-in.
- The decoder's 6.0.2 source-reviewed ceiling remains distinct from the 5.8.2 live-runtime ceiling.
- An installed 6.0.2 runtime can be checked without weakening the scheduled matrix's evidence
  classification.

## Evidence

The matrix records the exact image digests and the registry/release URLs used during review. The
official [Podman-in-Podman image documentation](https://github.com/podman-container-tools/image_build/tree/main/podman)
describes the stable image's nested-runtime purpose. The current tracked release is
[Podman 6.0.2](https://github.com/podman-container-tools/podman/releases/tag/v6.0.2).

## Alternatives

Installing every supported runtime on a host was rejected because it is hard to reproduce and
burdens contributors; an explicitly selected current-only executable remains optional. Mounting a
host socket was rejected because it grants broad runtime control and can
observe unrelated resources. Treating source fixtures as live conformance was rejected because it
would overstate evidence. Building every Podman version from source remains possible for versions
without official images, but it needs a separate reproducibility and supply-chain review before it
can close the current-patch gap.
