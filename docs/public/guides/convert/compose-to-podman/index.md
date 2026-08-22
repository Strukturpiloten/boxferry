# Compose to Podman

This route loads Compose input, imports it into the neutral application model, and exports a
PodmanLens deployment plan.

Use the normal Compose input options. Podman output accepts `--podman-max-version VERSION` and
`--podman-target-context unknown|rootless|rootful`. The default selects reviewed Podman 6.1.0 and
an unknown target context; neither value is inferred from the development machine.

Authorized output contains:

- `podman.json` — deterministic deployment-v1 operations;
- `review.sh` — a deterministic POSIX review script.

Both files are inert. BoxFerry never executes the script or applies the operations, and the output
is not a runtime inventory that can be imported again.
