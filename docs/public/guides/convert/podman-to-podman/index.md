# Podman to Podman

Podman-to-Podman is not passthrough or runtime cloning. The route explicitly acquires selected
runtime resources, imports them into the neutral application model, then exports reviewed desired
operations through PodmanLens.

Configure the read-only input as described in [Podman input](../../podman-input/). Select the output
ceiling with `--podman-max-version VERSION` and the target context with
`--podman-target-context unknown|rootless|rootful`. Source engine version and execution context
never become target choices implicitly.

Authorized output contains deterministic `podman.json` deployment-v1 operations and a deterministic
`review.sh` POSIX review script. BoxFerry never executes either representation.

The output cannot be re-imported because it represents desired operations rather than an observed
inventory and resource graph. Tests compare deterministic bytes, semantic operations, diagnostics,
and redaction instead of claiming a Podman fixed point.
