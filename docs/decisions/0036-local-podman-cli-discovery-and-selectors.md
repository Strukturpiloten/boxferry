# ADR 0036: Local Podman CLI discovery and exact selection

- Status: accepted
- Date: 2026-08-24
- Amends: [ADR 0021](0021-automatic-local-error-report-names.md) and
  [ADR 0034](0034-podman-lens-adapter-boundary.md)

## Context

The first Podman CLI exposed its library-level explicit-transport requirement directly: every
invocation needed a socket path and an application name. This made a local rootless conversion
needlessly hard to start, while the selector help hid its complete resource-kind vocabulary and
did not clearly show that the selector choices form one required set.

The CLI can offer a narrow, deterministic convenience without changing the reusable facade. A
generated support-bundle directory also should not fail merely because the explicitly requested
leaf directory has not yet been created.

## Decision

1. The reusable BoxFerry facade retains caller-selected Podman transport and discovery request.
   The Linux CLI alone may discover exactly two local Unix sockets: current-user rootless
   `/run/user/<uid>/podman/podman.sock`, then rootful `/run/podman/podman.sock`. It accepts only an
   existing non-symlink socket and reports both candidates when neither is available. An explicit
   `--podman-socket` overrides this convenience.
2. `--application-name` remains available but is optional for Podman input. Its deterministic
   fallback uses the only non-ID exact resource name, the only literal prefix, or the value from one
   exact label selector. Ambiguous selectors, full IDs, name-only labels, and `--podman-all` use
   `podman-import`. It is neutral application identity only; Compose-style ownership labels remain
   optional grouping evidence and neither determine a route nor override this rule.
3. Podman resource selection requires at least one of `--podman-all`, `--podman-resource`,
   `--podman-resource-prefix`, or `--podman-label`. Exact resource selectors show every accepted
   kind in contextual help. Globs, regular expressions, and partial IDs are not selector syntax.
4. An explicitly supplied `--error-report-directory` may name one absent leaf directory. Its
   existing parent and the resulting directory must be non-symlink directories; BoxFerry does not
   create missing parent chains. The archive itself remains create-new and no-clobber.

## Consequences

- A usual rootless or rootful local Podman service needs no redundant socket flag, but remote and
  nonstandard connections remain explicit.
- Users see selector alternatives and exact resource kinds before invoking a conversion.
- Application naming no longer implies Compose ownership or a Compose conversion route.
- A requested diagnostic bundle can use a new direct output directory without weakening archive
  publication safety.

## Alternatives considered

Reading `CONTAINER_HOST`, Podman connection configuration, or the full runtime directory was
rejected because those inputs are ambient, host-dependent, and potentially remote. Guessing a
project from Compose labels was rejected because labels are optional and advisory evidence.
Accepting shell patterns for resource roots was rejected because they make a selected graph
ambiguous; prefix selection is an explicit distinct operation. Recursive directory creation for
support bundles was rejected because it broadens the filesystem boundary and complicates symlink
validation.
