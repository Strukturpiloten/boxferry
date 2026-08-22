# Podman to Quadlet

This route explicitly acquires selected Podman runtime resources, imports them into the neutral
application model, and writes canonical Quadlet files.

Configure the read-only input as described in [Podman input](../../podman-input/): select one Unix
socket, set the application name, and provide at least one resource or label selector or select all
eligible roots.

Quadlet output retains its explicit minimum and maximum Podman version range and grouping policy.
Source runtime versions never become the target range implicitly.

Runtime-effective, runtime-assigned, and locally resolved observations remain governed by explicit
promotion and loss policy. Generated Quadlet files can be imported again for semantic-equivalence
and fixed-point checks.
