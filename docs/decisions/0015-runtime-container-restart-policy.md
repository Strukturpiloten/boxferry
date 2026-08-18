# ADR 0015: runtime container restart policy and systemd approximation

- Status: superseded
- Date: 2026-08-05
- Superseded by: [ADR 0032](0032-future-native-lens-boundaries.md)

## Context

Docker and Podman inspection expose a container-level restart policy as `HostConfig.RestartPolicy`
with a name and maximum retry count. Their shared names hide important implementation differences.
Docker applies a successful-start gate, treats explicit stops differently across daemon restarts,
and gives `unless-stopped` persistent state. Older supported Podman releases documented
`unless-stopped` as identical to `always`; current Podman retains an explicit-stop distinction
across reboot. Podman suppresses a policy after `podman stop` or `podman kill` and recommends
systemd for containers run as system services.

Quadlet services use systemd's generic `[Service] Restart=` directive. systemd classifies more
failure causes than a non-zero container exit code and applies independent time-window start-rate
limits. `StartLimitBurst=` is therefore not an equivalent encoding of
`on-failure:maximum-retries`.

Container restart policy is also distinct from Compose dependency `restart: true` and Compose
Deploy Specification `restart_policy`. Runtime `RestartCount` records history, not authored
policy, and cannot reconstruct a retry limit.

## Decision

1. `boxferry_model::RestartPolicy` represents only container-level automatic restart intent:
   `Never`, `Always`, unlimited or non-zero retry-limited `OnFailure`, and `UnlessStopped`.
2. `ContainerObservation` retains an optional effective policy. Docker and Podman decode the
   reviewed `HostConfig.RestartPolicy` object independently. Unknown names, negative counters,
   malformed objects, and counters attached to policies other than `on-failure` fail closed.
   Podman's empty default name and current `never` synonym with a zero counter are accepted as
   `Never`.
3. Restart policy is a container host setting, not an image default. Both runtime reconstruction
   policies preserve a supplied value directly with runtime-observation provenance and an exact
   source-mapping outcome.
4. Quadlet generation emits `Restart=no` exactly for `Never`. `Always`, unlimited `OnFailure`, and
   `UnlessStopped` emit the closest `[Service] Restart=` value only as an approximate outcome that
   requires explicit loss authorization.
5. A finite `OnFailure` retry limit remains unsupported and emits no restart directive. BoxFerry
   does not widen a bounded policy to infinite retries or pretend systemd rate limiting is
   equivalent.
6. Compose service `restart` maps through a separately tested, source-aware ComposeLens value into
   the same neutral contract and generates exactly back to Compose. An explicitly authored zero
   retry limit is invalid rather than reinterpreted as an omitted limit; native runtime APIs keep
   their independently documented zero-as-default decoding. Dependency restart propagation and
   deployment restart policy remain separate model concepts.

## Consequences

- Existing Docker and Podman containers can carry reviewed restart intent into the neutral model
  without relying on creation-command evidence or restart history.
- Runtime-to-Quadlet output is useful but honest about lifecycle-manager differences.
- `LossPolicy::ExactOnly` accepts only `Never`; approximate policies require
  `AllowApproximate`, while finite retry limits require `AllowPartial` and manual completion.
- Authored Compose and runtime-observed policies generate exactly back to Compose. Their Quadlet
  fidelity remains governed by the systemd distinctions above.
- Live Docker and Podman conformance create a test container with `on-failure:4` and verify the
  native object shape across the reviewed runtime lanes.

## Evidence

- [Docker restart policy behavior](https://docs.docker.com/engine/containers/start-containers-automatically/)
- [Podman restart option](https://docs.podman.io/en/latest/markdown/podman-run.1.html#--restartpolicy)
- [Podman Quadlet manual](https://docs.podman.io/en/latest/markdown/podman-systemd.unit.5.html)
- [systemd `Restart=` behavior](https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html#Restart=)

## Alternatives considered

Emitting Podman `--restart` through `PodmanArgs=` was rejected because Quadlet containers are
systemd services and Podman recommends letting systemd own service restart behavior. Translating a
finite retry count to `StartLimitBurst=` was rejected because the latter counts starts inside a
time window and is affected by systemd's separate rate-limit reset rules. Treating every
`unless-stopped` value as `always` exactly was rejected because Docker persists an explicit-stop
distinction that the generated directive alone cannot represent.
