# ADR 0007: Finite policy-controlled Podman relationship expansion

- Status: accepted
- Date: 2026-08-04

## Context

Reconstructing a useful declaration from one deployed container usually requires its image,
networks, and named volumes. A selected pod similarly needs its member containers. Requiring every
related runtime ID from a caller is safe but cumbersome; enumerating all local resources would
silently broaden access, mix unrelated applications, and make library behavior depend on ambient
machine state.

Explicit selectors can also be aliases of discovered native IDs. Passing both to an inspect
command may return the same resource twice, which must not turn a valid acquisition into a
duplicate-identity decoding error.

## Decision

Keep explicit-only acquisition as the default and as the behavior of `PodmanInspector::inspect`.
Add `inspect_with_policy` and a closed `PodmanExpansionPolicy` with two opt-in expansion scopes:

- selected containers to their image, attached networks, and named volumes; and
- selected pods to their member containers, followed by those container resources.

Expansion uses only relationships in responses for selected resources. Every command contains at
least one selector. Bind mounts do not name a separate Podman resource and are not inspected.
Reverse container-to-pod expansion and Podman's container dependency IDs are not followed. Podman
builds that field from both generic dependencies and containers supplying shared IPC, mount,
network, PID, user, UTS, or cgroup namespaces; it is not a portable application dependency list.
Selectors and returned resources are deduplicated in first-observed order; response deduplication
uses private native identities before decoding. Expansion-required malformed JSON fails closed and
does not enter error or debug output.

## Consequences

- Callers can obtain a useful finite resource closure without pre-resolving runtime IDs.
- A pod expansion cannot accidentally import unrelated pods or every object on a host.
- Explicit-only callers retain the original command order and decoding boundary.
- The opt-in path inspects pods before containers so pod members can join the container request.
- Namespace-provider and generic dependency IDs remain diagnosed as unsupported instead of
  silently importing infra containers or asserting a portable dependency meaning.

## Evidence

The [Podman 5.4 container-inspect documentation](https://docs.podman.io/en/v5.4.2/markdown/podman-container-inspect.1.html)
exposes `Dependencies` without classifying individual entries. The exact-version
[Podman 5.4.2 implementation](https://github.com/podman-container-tools/podman/blob/v5.4.2/libpod/container.go)
builds the value from every shared namespace container plus generic dependencies. The
[Podman 6.0.2 inspect implementation](https://github.com/podman-container-tools/podman/blob/v6.0.2/libpod/container_inspect.go)
continues to populate the response through `c.Dependencies()`.

## Alternatives

Listing every resource family and filtering after inspection was rejected because it reads
unrelated machine state. Automatically following every observed relationship was rejected because
reverse pod membership and opaque container dependencies can expand beyond the caller's intended
application. Requiring all related selectors forever was rejected because Podman already provides
a bounded, testable set of direct resource relationships.
