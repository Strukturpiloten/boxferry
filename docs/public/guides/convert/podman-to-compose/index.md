# Podman to Compose

This route explicitly acquires selected Podman runtime resources, imports them into the neutral
application model, and writes one canonical `compose.yaml`.

Configure the read-only input as described in [Podman input](../../podman-input/): select one Unix
socket, set the application name, and provide at least one resource or label selector or select all
eligible roots.

The example selects one container from a larger reviewed cassette environment. The promotion flags
explicitly authorize effective named volumes and networks to become portable desired state; omit
either flag when that promotion is not intended.

<!-- boxferry-example: podman-to-compose -->

```console
boxferry convert podman compose --podman-socket /run/user/1000/podman/podman.sock --application-name complex --podman-resource container=c-observer --promote-podman-effective-named-volumes --promote-podman-effective-named-networks --loss-policy partial --output-directory compose-output
```

Runtime-effective, runtime-assigned, and locally resolved observations do not automatically become
portable desired state. Every required decision or unsupported field remains visible through the
normal diagnostic and loss-policy contract.

The generated Compose document can be imported again for semantic-equivalence and fixed-point
checks.

With exact-only policy, the full complex inventory is blocked before the output directory is
created because its runtime observations require reviewed conversion decisions.

<!-- boxferry-example: podman-to-compose-exact-blocked -->

```console
boxferry convert podman compose --podman-socket /run/user/1000/podman/podman.sock --application-name complex --podman-all --loss-policy exact --output-directory compose-error-output
```
