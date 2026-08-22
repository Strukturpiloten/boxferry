# Compose to Podman

This route loads Compose input, imports it into the neutral application model, and exports a
PodmanLens deployment plan.

Use the normal Compose input options. The target context is required because BoxFerry never infers
it from the development machine. `--podman-max-version VERSION` optionally caps output to the
newest reviewed exact target not greater than that ceiling; without a ceiling, output targets the
newest reviewed version, currently 6.1.0.

<!-- boxferry-example: compose-to-podman -->

```console
boxferry convert compose podman --input-file tmpfs-compose.yaml --podman-target-context unknown --output-directory podman-output
```

Authorized output contains:

- `podman.json` — deterministic deployment-v1 operations;
- `review.sh` — a deterministic POSIX review script.

Both files are inert. BoxFerry never executes the script or applies the operations, and the output
is not a runtime inventory that can be imported again.
