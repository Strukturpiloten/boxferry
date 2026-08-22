# Quadlet to Podman

This route parses a Quadlet document set, imports it into the neutral application model, and exports
a PodmanLens deployment plan. It does not invoke Quadlet generators or a Podman service.

`--podman-max-version VERSION` optionally caps output to the newest reviewed exact target not
greater than that ceiling; without a ceiling, output targets the newest reviewed version, currently
6.1.0. `--podman-target-context unknown|rootless|rootful` is required, and BoxFerry never infers it
from the source or development machine.

<!-- boxferry-example: quadlet-to-podman -->

```console
boxferry convert quadlet podman --input-file worker.container --application-name route-matrix-tmpfs --podman-target-context unknown --output-directory podman-output
```

Authorized output contains deterministic `podman.json` deployment-v1 operations and a deterministic
`review.sh` POSIX review script.

Both files are inert. BoxFerry never executes the script or applies the operations, and the output
is not a runtime inventory that can be imported again.
