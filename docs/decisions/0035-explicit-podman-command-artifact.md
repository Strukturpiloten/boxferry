# ADR 0035: Explicit Podman command artifact

- Status: accepted
- Date: 2026-08-24
- Amends: [ADR 0034](0034-podman-lens-adapter-boundary.md) decision 5

## Context

ADR 0034 named the generated shell artifact `review.sh` and described both Podman artifacts as
inert. The name under-described the file: it contains deterministic, runnable Podman commands.
BoxFerry never executes them, but a user can. That distinction must be visible before an operator
mistakes the file for a report.

The project is pre-1.0 and has no demonstrated need for a compatibility alias.

## Decision

1. The generated shell artifact is named `podman-commands.sh`.
2. `PodmanOutput::commands_shell()` exposes its contents to Rust callers.
3. BoxFerry never executes the script, contacts a mutating Libpod endpoint, or applies any output.
4. User documentation states that running the script performs real operations and requires review
   of the Podman connection, target version, execution context, host paths, networks, volumes, and
   sensitive preconditions.
5. The deployment-v1 `podman.json` schema and non-executing BoxFerry boundary remain unchanged.

## Consequences

- Existing pre-1.0 callers must use the new filename and accessor.
- The output tree tells operators that the shell file contains commands.
- Golden fixtures and documentation examples protect the exact name and runnable bytes.
