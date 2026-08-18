# ADR 0014: runtime regular-health observations and startup-health separation

- Status: superseded
- Date: 2026-08-04
- Superseded by: [ADR 0032](0032-future-native-lens-boundaries.md)

## Context

Docker and Podman inspection expose effective regular health-check configuration on both images
and containers. The native JSON uses command-marker arrays and Go duration values in nanoseconds,
while BoxFerry's neutral model retains target-independent duration spellings. Container inspection
can contain image defaults as well as caller overrides, so copying the complete effective object
would freeze defaults into every generated definition.

Podman also exposes a separate startup-healthcheck family, health-on-failure actions, and health-log
controls. Those features are not equivalent to Docker/Compose `start_interval`; treating them as
interchangeable would change runtime behavior.

## Decision

1. `RuntimeHealthcheck` represents only effective regular health checks. It contains protected
   `CMD`/`CMD-SHELL` command values, explicit disable state, interval, timeout, retries, start
   period, and a start interval only where the native source has equivalent semantics.
2. `Some(RuntimeHealthcheck::new())` means the adapter inspected the effective field and found no
   regular health check. `None` means no health observation was supplied. This preserves the
   missing-versus-observed-absence distinction needed for image comparison.
3. Native adapters accept the reviewed `CMD`, `CMD-SHELL`, and `NONE` marker shapes. Empty,
   contradictory, negative, or unknown native values fail closed. Positive nanosecond durations
   become the largest exact integral `h`, `m`, `s`, `ms`, `us`, or `ns` spelling; zero-valued
   implementation defaults are omitted.
4. `InferImageOverrides` compares command, disable, interval, timeout, retries, start period, and
   start interval independently. Matching image defaults are omitted; differences retain runtime-
   observation and conversion-decision provenance and receive `BFR0002`.
5. Docker `StartInterval` is accepted only for Engine API 1.44 and newer. Podman
   `StartupHealthCheck`, `HealthcheckOnFailureAction`, health-log controls, and an unverified regular
   `StartInterval` remain named `BFP0002` losses.
6. Runtime health commands use `ProtectedString` and must not appear in debug output or diagnostics.

## Consequences

- Runtime migration can preserve or infer the same regular health subset already represented by
  Compose-to-Quadlet conversion.
- Image defaults do not need to be repeated merely because native inspection reports effective
  values.
- Docker and Podman keep separate native decoders even where their current JSON shapes coincide.
- Podman startup-health behavior remains manual until BoxFerry gains a distinct neutral use case
  and target contract for it.
- A live Podman 6.0.2 lane and authored Docker/Podman fixtures verify native number and command
  shapes without storing production inspect payloads.

## Alternatives considered

Storing `boxferry_model::Healthcheck` directly in observations was rejected because its sourced
fields represent application intent rather than unclassified effective state. Treating a missing
health observation as an inspected absence was rejected because incomplete image data would be
misclassified as a proven override. Mapping Podman startup-health intervals to Compose/Docker
`start_interval` was rejected because their activation and lifecycle semantics differ.
