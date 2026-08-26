# Podman to Quadlet

Generate canonical Quadlet files from explicitly selected Podman runtime resources. Acquisition is
read-only and every resource crosses the neutral application model.

## Prerequisites

- Read-only input configured as described in [Podman input](../../podman-input/).
- At least one resource or label selector, or `--podman-all`.
- A target Podman version range and Quadlet grouping decision.

## Convert

<!-- boxferry-example: podman-to-quadlet -->

```console
boxferry convert podman quadlet --podman-socket /run/user/1000/podman/podman.sock --application-name complex --podman-resource container=c-observer --promote-podman-effective-named-volumes --promote-podman-effective-named-networks --loss-policy partial --output-directory quadlet-output
```

The bounded example writes `quadlet-output/observer.container`. Generated Quadlet files can be
imported again for semantic-equivalence and fixed-point checks.

## Compatibility and loss

Quadlet output retains its explicit minimum and maximum Podman version range and grouping policy.
Source runtime versions never become the target range implicitly.

Runtime-effective, runtime-assigned, and locally resolved observations remain governed by explicit
promotion and loss policy. The two promotion flags in the example authorize only effective named
volumes and networks; they do not promote arbitrary runtime-local state. Add
`--promote-podman-portable-effective-settings` only after reviewing environment, published-port,
restart, normal-health, and DNS evidence. It allows sensitive environment acquisition, but
diagnostic reports and snapshots remain redacted.

Add `--promote-podman-effective-bind-mounts` only when the target intentionally reuses the same
absolute host paths. BoxFerry preserves the source, destination, and read-only state; native-only
mount options remain visible as findings. Quoted Quadlet output preserves Podman environment names
such as dotted application settings and exec arguments containing spaces.

Before using the units on another host, verify bind mounts, `EnvironmentFile=` paths, secret
references, shared volume data, and network units. BoxFerry writes files; it never installs,
enables, starts, or reloads systemd units.

---

[← Podman to Podman](../podman-to-podman/) · [Next: Quadlet to Compose →](../quadlet-to-compose/)
