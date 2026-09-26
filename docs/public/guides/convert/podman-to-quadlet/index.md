# Podman to Quadlet

Generate canonical Quadlet files from explicitly selected Podman runtime resources. Acquisition is
read-only and every resource crosses the neutral application model.

## Prerequisites

- Read-only input configured as described in [Podman input](../../podman-input/).
- A bounded application selection (automatic when unambiguous, or an explicit selector).
- A target Podman version range and Quadlet grouping decision.

## Convert

<!-- boxferry-example: podman-to-quadlet -->

```console
boxferry convert podman quadlet --podman-socket /run/user/1000/podman/podman.sock --application-name complex --podman-resource container=c-observer --loss-policy partial --output-directory quadlet-output
```

The bounded example writes `quadlet-output/observer.container`. Generated Quadlet files can be
imported again for semantic-equivalence and fixed-point checks.

## Compatibility and loss

Quadlet output retains its explicit minimum and maximum Podman version range and grouping policy.
Source runtime versions never become the target range implicitly.

The default portable import policy retains reviewed effective settings and named relationships;
it does not promote arbitrary runtime-local state. `--loss-policy partial` in this example accepts
separate unsupported fields. Exact remains the default and blocks actual exporter approximations.
Use `--podman-import-policy conservative` to leave effective observations as evidence until the
individual promotion switches are selected. Runtime container-ID aliases remain evidence only,
and pod-member networking remains pod-scoped evidence. Environment values remain withheld unless
`--environment-values include` separately authorizes them; reports and snapshots stay redacted.

The portable preset also promotes typed network-internal, subnet, gateway, lease-range, and attachment-alias
observations. Lease endpoints become Quadlet's supported `<start-IP>-<end-IP>` `IPRange` form. An
IPv6 subnet enables `IPv6=true`, so an application-owned `.network` unit carries these settings
instead of remaining empty. Driver, DNS, IPAM-driver, interface, and standalone IPv6 flags remain
visible losses until PodmanLens exposes safe typed observations.

Typed configured `z`/`Z` evidence is preserved as shared/private relabel intent through the neutral
model. PodmanLens owns decoding `Mounts[].Mode` and `HostConfig.Binds`; BoxFerry never reparses
those native values. Bounded creation evidence may corroborate the typed result but cannot create or
override it. Missing optional corroboration retains the parent mount and reports the relabel omission.

Add `--promote-podman-effective-bind-mounts` only when the target intentionally reuses the same
absolute host paths. BoxFerry preserves the source, destination, and read-only state; native-only
mount options remain visible as findings. Quoted Quadlet output preserves Podman environment names
such as dotted application settings and exec arguments containing spaces.

Before using the units on another host, verify bind mounts, `EnvironmentFile=` paths, secret
references, shared volume data, and network units. BoxFerry writes files; it never installs,
enables, starts, or reloads systemd units.

---

[← Podman to Podman](../podman-to-podman/) · [Next: Quadlet to Compose →](../quadlet-to-compose/)
