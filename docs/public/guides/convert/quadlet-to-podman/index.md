# Quadlet to Podman

Create a deterministic Podman deployment plan from a Quadlet document set. BoxFerry does not invoke
Quadlet generators, contact Podman, or execute the generated commands.

## Prerequisites

- One or more related Quadlet files and an explicit application name.
- The intended `unknown`, `rootless`, or `rootful` target context.
- An absent or empty output directory.

## Convert

<!-- boxferry-example: quadlet-to-podman -->

```console
boxferry convert quadlet podman --input-file worker.container --application-name route-matrix-tmpfs --podman-target-context unknown --output-directory podman-output
```

Authorized output contains deterministic `podman.json` deployment-v1 operations and a runnable
`podman-commands.sh` script. The [Compose-to-Podman guide](../compose-to-podman/) explains both
artifacts field by field and shows complete examples.

BoxFerry never executes the script. Running it yourself performs real Podman operations. The JSON
is not a runtime inventory and cannot be imported as Podman input.

## Compatibility and loss

Output defaults to the newest reviewed target, currently 6.1.0. `--podman-max-version VERSION`
selects the newest reviewed exact target not greater than that ceiling.
`--podman-target-context unknown|rootless|rootful` is required, and BoxFerry never infers it from
the source or development machine.

Review `EnvironmentFile=` and bind-mount host paths, secret preconditions, network dependencies,
published ports, and any systemd-only relationship before handing the command script to an
operator.

---

[← Quadlet to Compose](../quadlet-to-compose/) · [Next: Quadlet to Quadlet →](../quadlet-to-quadlet/)
