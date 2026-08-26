# Podman to Compose

Reconstruct one canonical `compose.yaml` from explicitly selected Podman runtime resources. Input
acquisition is read-only; the generated Compose document describes portable desired intent.

## Prerequisites

- Read-only input configured as described in [Podman input](../../podman-input/). A conventional
  local socket and neutral application name are derived unless you override them.
- At least one exact, prefix, or label selector, or `--podman-all`.
- A decision about whether effective named volumes and networks are portable desired state.

See [Podman input](../../podman-input/) before selecting a shared production environment.

## Convert one bounded workload

The example selects one container from a larger reviewed cassette environment. The promotion flags
explicitly authorize effective named volumes and networks to become portable desired state; omit
either flag when that promotion is not intended.

<!-- boxferry-example: podman-to-compose -->

```console
boxferry convert podman compose --podman-socket /run/user/1000/podman/podman.sock --application-name complex --podman-resource container=c-observer --promote-podman-effective-named-volumes --promote-podman-effective-named-networks --loss-policy partial --output-directory compose-output
```

The result is:

```text
compose-output/
└── compose.yaml
```

Runtime-effective, runtime-assigned, and locally resolved observations do not automatically become
portable desired state. Every required decision or unsupported field remains visible through the
normal diagnostic and loss-policy contract.

## Production checks

- Add `--podman-network-boundary NAME_OR_ID` only for a network that discovery may cross.
- Verify named volumes are intentionally shared; a recreated volume does not contain source data.
- Replace or redesign bind mounts whose host paths do not exist on the target.
- Re-author secret delivery. Runtime inspection cannot recover secret material or complete grant
  intent.
- Review published ports and runtime-assigned addresses; observed values are not automatically
  portable configuration.

The generated Compose document can be imported again for semantic-equivalence and fixed-point
checks.

## Exact policy failure

With exact-only policy, the full complex inventory is blocked before the output directory is
created because its runtime observations require reviewed conversion decisions.

<!-- boxferry-example: podman-to-compose-exact-blocked -->

```console
boxferry convert podman compose --podman-socket /run/user/1000/podman/podman.sock --application-name complex --podman-all --loss-policy exact --output-directory compose-error-output
```

Do not jump directly to `partial`. Start with the reported `fix first` rule, narrow the selected
resource graph, then authorize only the remaining understood losses.

---

[← Compose to Quadlet](../compose-to-quadlet/) · [Next: Podman to Podman →](../podman-to-podman/)
