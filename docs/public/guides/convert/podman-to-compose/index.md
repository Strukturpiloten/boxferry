# Podman to Compose

This route explicitly acquires selected Podman runtime resources, imports them into the neutral
application model, and writes one canonical `compose.yaml`.

Configure the read-only input as described in [Podman input](../../podman-input/): select one Unix
socket, set the application name, and provide at least one resource or label selector or select all
eligible roots.

Runtime-effective, runtime-assigned, and locally resolved observations do not automatically become
portable desired state. Every required decision or unsupported field remains visible through the
normal diagnostic and loss-policy contract.

The generated Compose document can be imported again for semantic-equivalence and fixed-point
checks.
