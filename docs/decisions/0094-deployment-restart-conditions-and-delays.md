# ADR 0094: Deployment restart conditions and delays

- Status: accepted
- Date: 2026-11-02
- Amends: [ADR 0032](0032-future-native-lens-boundaries.md) and
  [ADR 0040](0040-quadlet-native-value-decoder-boundary.md), only for bounded generated delay recognition
- Builds on: [ADR 0033](0033-universal-neutral-model-pipeline.md) and
  [ADR 0041](0041-portable-intent-and-retained-native-evidence.md)

## Context

Compose deployment restart policies have conditions, delays, attempt limits and success-evaluation
windows. They take precedence over service-level container-runtime restart policies. The neutral
`RestartPolicy` deliberately represents only container-runtime behavior; adding a delay to it would
conflate independently configured controllers. systemd uses `Restart=` and `RestartSec=` with its
own failure classification and start-rate limits.

ComposeLens 0.4.0 exposes typed deployment input but no typed deployment generation API. BoxFerry
must not implement native YAML rendering to work around that ownership boundary.

## Decision

1. Add a separate `DeploymentRestartPolicy` with field provenance, a deployment condition, an
   optional exact duration, and independent attempt/window constraints. Leave `RestartPolicy`
   unchanged. Preserve both policies on input; deployment policy takes precedence on Quadlet output.
2. Compose imports `none`, `on-failure` and `any`; an omitted condition means `any`. The neutral
   deployment policy's omitted delay means immediate restart, matching Compose. Quadlet output
   explicitly emits `RestartSec=0us` for that default because systemd otherwise defaults to 100ms.
3. Quadlet maps the conditions to `no`, `on-failure` and `always`. `none` is exact; other conditions
   require approximation authorization for service-manager failure classification and start-rate
   differences. Delay conversion uses checked decimal arithmetic and whole microseconds, rejecting
   overflow or precision loss. It never rounds a positive delay to zero. Valid Compose durations
   outside the neutral representation produce Unsupported outcomes and provenance-bearing field
   markers; these markers require exporters to reject the complete policy, preserving controller
   precedence without inventing a substitute duration. Malformed/deferred source values remain Invalid.
4. An authored attempt count or evaluation window makes the entire deployment policy unsupported
   on Quadlet output. Emit neither restart directive and never fall back to the overridden runtime
   policy. systemd start-rate limits do not implement deployment retry accounting.
5. Quadlet `RestartSec=` activates deployment-policy import, including a default `no` condition
   when `Restart=` is absent. Without `RestartSec=`, keep the existing container-runtime import
   contract. Duplicate, deferred, unsupported or unrepresentable delay directives remain explicit
   findings. Reverse delay recognition accepts only the finite `<digits>us` literal emitted by
   BoxFerry, excluding the reserved unsigned-64-bit maximum/infinity value. Other native timespans,
   including `5s`, are Unsupported. This bounded generated-wire recognition is an explicit exception
   to the native decoder ownership in ADR 0032/0040, not a systemd timespan parser. It consumes the
   existing QuadletLens `TypedEntry` and literal/deferred classification through `direct_value`;
   QuadletLens 0.3.0 has no native `RestartSec` duration semantic view. Wider authored timespan
   decoding remains Lens-owned. Canonical emitted delays survive semantic reimport and output.
6. Compose and Podman exporters explicitly report deployment-policy output as unsupported with
   actionable guidance. Reverse Compose generation awaits a native ComposeLens API; it is not
   claimed as delivered. No dependency upgrade or native rendering shortcut is introduced.

## Consequences

Users can express supported deployment restart delays in Compose and generate reviewable Quadlet
units. Existing runtime restart mappings and policy requirements remain unchanged. Exact-only output
accepts the deployment `none` condition and representable delays; restarting conditions require
`AllowApproximate`, and unsupported constraints require `AllowPartial` with manual completion.
Compose-to-Compose remains policy controlled for deployment policies until native generation exists.

Machine-readable `capabilities` output owns the supported conditions, precision, defaults,
precedence, reverse-generation limitation and native capability evidence. Tests own conversion,
provenance, finite-constraint refusal, target bounds and fixed-point expectations.

## Evidence

- [Compose deployment restart policy](https://docs.docker.com/reference/compose-file/deploy/#restart_policy)
- [systemd service restart behavior and delay](https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html#RestartSec=)
- ComposeLens 0.4.0 public project and generated-service APIs, from the locked crates.io archive
  (`f5408dbb8a4a5f7207a0a300fea4a0d2e7c9a3befec7d493ee3142287b317cef`).
  This is dependency API inspection, not an external conversion oracle; no oracle code or fixtures
  were copied or redistributed.

## Alternatives considered

Adding a delay to container-runtime `RestartPolicy` would erase the controller distinction.
Encoding finite attempts with `StartLimitBurst=` would change retry accounting. Emitting only
`RestartSec=` would retain systemd's default disabled restart condition. Native YAML injection or
an unreleased dependency would violate adapter ownership or make this contribution dependent on
another repository.
