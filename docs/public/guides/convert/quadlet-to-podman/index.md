# Quadlet to Podman

This route parses a Quadlet document set, imports it into the neutral application model, and exports
a PodmanLens deployment plan. It does not invoke Quadlet generators or a Podman service.

Podman output accepts `--podman-max-version VERSION` and
`--podman-target-context unknown|rootless|rootful`. The default selects reviewed Podman 6.1.0 and
an unknown target context.

Authorized output contains deterministic `podman.json` deployment-v1 operations and a deterministic
`review.sh` POSIX review script.

Both files are inert. BoxFerry never executes the script or applies the operations, and the output
is not a runtime inventory that can be imported again.
