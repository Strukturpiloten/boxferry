# ADR 0040: Quadlet native value decoder boundary

- Status: accepted
- Date: 2026-09-07
- Amends: [ADR 0022](0022-sole-quadlet-parser-and-deterministic-test-contract.md) and [ADR 0031](0031-quadlet-systemd-environment-and-native-only-reporting.md)

## Context

The Quadlet adapter historically decoded `PublishPort=`, `Volume=`/`Mount=`,
`Exec=`, `Entrypoint=`, and Build `Environment=` with private parsers. Those
parsers accepted BoxFerry's generated subset but rejected valid authored
quoting, IPv6, long mount options, and command forms already understood by
QuadletLens. They also could not retain the owning parser's source spans and
native diagnostics.

## Decision

1. BoxFerry requires QuadletLens 0.2.3 or newer in the compatible 0.2 series
   and consumes its document-level container/pod port and mount views,
   container command view, and container/build environment views.
2. QuadletLens exclusively owns native lexical decoding, ordered directives,
   source spans, deferred-systemd detection, and `QLM0023`–`QLM0024` and `QLM0029`–`QLM0034`
   diagnostics for these values. The adapter does not pass them through its
   generic scalar decoder or keep a second native parser.
3. BoxFerry owns only native-to-neutral meaning. A decoded scalar IPv4/IPv6
   publication, reviewed bind/volume mount, quoted argv, or literal assignment
   enters the neutral model with source provenance. A valid decoded range,
   opaque mount option/type, relative target-host reference, or deferred value
   is `Unsupported`; an `Unmodeled` malformed directive is `Invalid` and keeps
   the QuadletLens finding.
4. Ordered reset directives clear preceding effective values. Where the
   neutral model cannot retain the reset event itself, BoxFerry keeps an
   explicit source-spanned loss outcome. Source-authored command and
   entrypoint arguments remain sensitive protected values even though
   authorized artifacts may contain them.
5. Every Quadlet input still follows importer → neutral model → exporter,
   including Quadlet-to-Quadlet. The authored native-value corpus exercises
   Compose, Podman, and Quadlet exporters and separately records supported and
   policy-controlled unsupported intent.

## Consequences

- Authored native spelling no longer has to match BoxFerry's renderer to be
  accepted.
- QuadletLens upgrades can extend syntax decoding without duplicating parser
  work in BoxFerry; new non-exhaustive variants remain fail-visible until their
  neutral meaning is reviewed.
- Conversion reports retain producer-owned rule codes and exact source spans,
  while protected command/environment values remain absent from debug and
  diagnostic presentation.

## Alternatives considered

Keeping the private parsers was rejected because it recreates native-format
ownership and makes self-generated round trips look more complete than real
authored input. Treating every decoded value as exact was rejected because
native syntactic validity does not prove portable neutral meaning. Retaining
raw native strings in the neutral model was rejected because it would couple
all exporters to Quadlet syntax and enable a same-format bypass.
