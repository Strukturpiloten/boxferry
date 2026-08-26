# Podman to Podman

Re-plan selected Podman resources for an explicit target version and execution context.
Podman-to-Podman is not passthrough, runtime cloning, or deployment.

## Prerequisites

- Read-only input configured as described in [Podman input](../../podman-input/).
- The intended maximum Podman version and `unknown`, `rootless`, or `rootful` target context.
- Explicit promotion decisions for runtime-effective named resources.

## Convert

<!-- boxferry-example: podman-to-podman -->

```console
boxferry convert podman podman --podman-socket /run/user/1000/podman/podman.sock --application-name complex --podman-resource container=c-observer --promote-podman-effective-named-volumes --promote-podman-effective-named-networks --loss-policy partial --podman-target-context rootless --output-directory podman-output
```

Authorized output contains:

```text
podman-output/
├── podman-commands.sh
└── podman.json
```

[The complete Podman artifact examples](../compose-to-podman/) use the same schema and safety
contract for every Podman output route. BoxFerry never executes the script. Running
`podman-commands.sh` yourself performs real operations against your selected Podman connection.

## Production checks

- The source engine version and execution context never become target choices implicitly.
- A rootful-to-rootless move can change privileged ports, namespaces, UID mappings, and host-path
  access. Review every target-context diagnostic.
- `--promote-podman-effective-bind-mounts` reuses absolute source paths. Verify those paths and
  their permissions on the target before running the generated script.
- Network-boundary permission controls discovery, not target firewall policy.
- Named volume recreation does not copy volume data.
- Inspected secrets remain preconditions until an operator supplies reviewed secret intent.

The output cannot be re-imported because it represents desired operations rather than an observed
inventory and resource graph. Tests compare deterministic bytes, semantic operations, diagnostics,
and redaction instead of claiming a Podman fixed point.

---

[← Podman to Compose](../podman-to-compose/) · [Next: Podman to Quadlet →](../podman-to-quadlet/)
