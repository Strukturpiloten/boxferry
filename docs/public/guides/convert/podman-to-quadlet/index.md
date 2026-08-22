# Podman to Quadlet

This route explicitly acquires selected Podman runtime resources, imports them into the neutral
application model, and writes canonical Quadlet files.

Configure the read-only input as described in [Podman input](../../podman-input/): select one Unix
socket, set the application name, and provide at least one resource or label selector or select all
eligible roots.

<!-- boxferry-example: podman-to-quadlet -->

```console
boxferry convert podman quadlet --podman-socket /run/user/1000/podman/podman.sock --application-name complex --podman-resource container=c-observer --promote-podman-effective-named-volumes --promote-podman-effective-named-networks --loss-policy partial --output-directory quadlet-output
```

Quadlet output retains its explicit minimum and maximum Podman version range and grouping policy.
Source runtime versions never become the target range implicitly.

Runtime-effective, runtime-assigned, and locally resolved observations remain governed by explicit
promotion and loss policy. The two promotion flags in the example authorize only effective named
volumes and networks; they do not promote arbitrary runtime-local state. Generated Quadlet files
can be imported again for semantic-equivalence and fixed-point checks.
