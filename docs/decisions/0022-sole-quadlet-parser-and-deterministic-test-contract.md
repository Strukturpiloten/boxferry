# ADR 0022: Sole Quadlet parser and deterministic test contract

- Status: accepted
- Date: 2026-08-11
- Amends: [ADR 0020](0020-rolling-compose-specification-cli-target.md)

## Context

The published 0.1.1 Quadlet adapter exposed both an aggregate parser and an additive detailed
parser. The aggregate API could return an incomplete document graph without exposing the native
document-set diagnostics that explained it.

## Decision

1. Set all supported BoxFerry crates to the next lockstep version, 0.2.0. Remove
   `QuadletSourceError` and make `QuadletSource::parse` the only parser API.
2. `QuadletParseResult` retains syntax, model, and document-set diagnostics in native collection
   order. Recoverable document-set diagnostics retain an incomplete graph. Syntax/model errors and
   non-recoverable construction failures return `QuadletParseError`.
3. Required deterministic tests include parser panic containment, fixed-seed repeatability,
   diagnostic ordering and span bounds, seeded redaction canaries, public facade coverage, and
   offline small, medium, and large CLI scenarios. They add no fuzzing, mutation, Miri, or test
   dependency requirement.
4. A pinned `cargo-llvm-cov` 0.8.7 workspace/all-feature/all-target coverage job is a coarse
   regression ratchet, not correctness evidence. macOS and Windows run the pure deterministic
   check/test lanes, and an always-running aggregate PR gate is the branch-protection check.

## Consequences

Callers must consume the parse result explicitly before importing. ADR 0020's old parse-failure
classification is amended: an incomplete document set crosses parsing and is reported as a
conversion failure with retained native diagnostics. Privileged conformance needs a privileged
Linux runtime and the remote corpus needs network access, so both remain scheduled/manual evidence
rather than required PR jobs; every promoted issue receives an offline regression.

## Alternatives considered

Keeping a deprecated aggregate parser was rejected because it could silently discard diagnostics
and would leave two parser contracts to support during the pre-1.0 transition.
