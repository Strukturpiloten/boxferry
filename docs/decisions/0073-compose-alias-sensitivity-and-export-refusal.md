# ADR 0073: Scalar-specific Compose service-alias sensitivity and protected export refusal

- Status: accepted
- Date: 2026-10-08
- Supersedes: [ADR 0054](0054-typed-podman-network-alias-promotion.md) decision 4 only
  for the protected-service-alias Compose export and fresh-text reimport claim
- Also supersedes: ADR 0048 and ADR 0049 only for the successful generated
  protected-service-alias Quadlet-to-Compose reimport promise; runtime probes,
  budgets, cleanup and all route invocations remain required
- Preserves: ADR 0054's native classification, topology correlation and explicit promotion rules;
  [ADR 0058](0058-separate-sensitive-value-artifact-authorization.md) and
  [ADR 0061](0061-compose-interpolation-input-authorization.md)

## Context

The Compose importer previously protected every service-network alias, including ordinary public
literals.
The existing published ComposeLens dependency supplies scalar sensitivity in the same merged
project used to construct its typed network attachments. The typed aliases retain ordered values
and source spans, but their enclosing project value has only aggregate sensitivity. Applying that
aggregate flag would either taint public siblings or conceal a protected sibling.

Generated-string sensitivity is in-memory metadata, not a durable property of ordinary Compose
YAML. Exporting a protected service alias as a literal and then freshly loading those bytes loses its
classification. An importer cannot distinguish that literal from identical authored public text.
The earlier successful protected-service-alias fresh-text reimport promise is therefore unsupported.

## Decision

1. Corroborate the complete ordered typed alias tuple against native scalar metadata at the exact
   original service/network path, including each value and effective source span. Only a fully
   matching tuple may supply per-scalar sensitivity. Missing, malformed, reordered or mismatched
   metadata conservatively protects every typed alias while retaining its value and provenance.
   Do not parse YAML independently or guess from variable names, values or filenames.
2. Public literals and explicitly public embedded interpolation values remain ordinary aliases.
   Sensitive substitutions remain protected. The CLI still classifies all supplied/resolved
   interpolation values as sensitive; no consent, option or classification override is added.
3. Any semantically protected alias in a service network attachment suppresses the entire Compose
   candidate under every loss policy, including partial. Emit an actionable, value-free `Unsupported` outcome for its
   indexed field with its source origins. Catalogue remediation must distinguish this whole-candidate
   privacy refusal from ordinary omissions that a partial loss policy can authorize.
   Do not mutate neutral aliases, omit an alias while
   returning an apparently complete topology, or serialize its value into unmarked Compose text.
4. Ordinary public service aliases may export and freshly reimport their text exactly, with public
   classification and fresh source-document provenance. Protected-service-alias successful Compose
   export/fresh-text reimport is no longer a supported branch. Arbitrary historical plaintext
   cannot recover lost taint; no such recovery is promised or guessed.
5. Other exporters, native promotion rules, protected addresses, health commands and environment
   policies remain unchanged. No markers, sidecars, native parsing, Lens API changes, dependency
   updates or new authorization channels are introduced.
6. Group-network aliases are not serialized. Their existing structured unsupported outcomes and
   partial-loss handling for unmapped group networking/runtime settings remain unchanged. This
   decision does not extend service-alias generation or the whole-candidate guard to group aliases.

## Consequences and verification

Mixed arrays, duplicate resolved values, appended and overriding documents require independent
per-slot expectations. Fault-injection controls must prove missing, malformed or mismatched native
metadata cannot authorize a public alias, and that conservative protection also blocks export.
All Compose target families and loss policies require whole-candidate refusal for protected service
aliases; diagnostics, Debug and reports must not contain protected values. The public roundtrip test must
load generated physical text, not reuse an in-memory generated document's sensitivity metadata.

This narrows one Compose export branch rather than weakening privacy or broadening consent.
Plain aliases explicitly promoted under ADR 0054 remain portable intent. Docker/application
integration, protected health-command handling and runtime acceptance remain separate obligations.
The source metadata API is already supplied by the unchanged published dependency; manifests,
lockfiles, operational pins, extraction paths and Renovate ownership need no changes.

## Conditional protected-alias reimport

ADR 0073 narrows the generated service-alias Quadlet-to-Compose branch. Every
mode, selection and route is still executed. Independently authored per-role
alias expectations bind the actual generated container units; their ordered
indexed refusal subjects and source digest are fixed before conversion and
checked unchanged afterward. Pod/group aliases do not trigger this service guard.

A nonempty protected service-alias inventory requires exit 2, a blocked policy
report, exact value-free BFC0007 indexed unsupported diagnostics and actionable
help, unchanged other diagnostic/fidelity expectations, and no output artifacts
or files. The prior exact mapped-attachment outcome is not claimed when its
protected aliases prevent construction; the exact implementation counter remains
non-negative rather than an invented fixed total. Non-exact fidelity is checked
against the independently authored baseline plus indexed alias refusals.
It is recorded as a known migration gap because fresh Compose text
cannot retain alias confidentiality. It is never counted as successful migration.
A genuinely zero-alias source retains the positive branch; missing, reordered or
substituted expected aliases do not manufacture that control. Direct explicitly
promoted Podman-to-Compose aliases remain positive.

Native application startup, readiness, traffic, persistence, all other routes,
resource/time budgets, thresholds and cleanup remain mandatory. Offline report
mutations and physical document reimport controls prove the branch contract only;
fresh exact-revision live evidence is still required. No consent, declassification,
artifact sidecar, native parser or executor is added.

The two owning application runners share
`scripts/lib/protected-service-alias-contract.py` for bounded regular-file source
reads, inventory mechanics and closed refusal-report predicates. Each runner
retains its independently authored role/ordered-alias specification, source
preflight assertions and diagnostic/fidelity baseline. Both mandatory shell
regression suites parse the helper's Python syntax and exercise its branches;
the facade scenario separately tests physical conversion and reimport.
This local harness primitive uses the existing Python toolchain and adds no
operational version, downloaded tool, dependency or Renovate extraction path.
The existing unique operational-pin managers and consumers remain unchanged.

The prospective Docker application prerequisite also binds this shared semantic
helper in both affected applications and the owning Supabase alias wrapper.
Reviewed wrapper/route source checksums and independent eight-success/one-gap
assertions are updated together; all topology and application-check inventories
remain unchanged. Missing or mutated bindings and superseded offline admissions
must fail closed. This schema-1, non-native prerequisite does not rewrite or
upgrade historical admissions, receipts or native evidence and introduces no
software pin or Renovate manager change.

## Alternatives

Blanket alias protection defeats ordinary authored public intent. Aggregate sensitivity loses
scalar-specific meaning. Literal protected export declassifies data after a fresh load. Alias
omission produces incomplete topology, and partial policy cannot repair missing confidentiality
metadata. Value/name guesses, private declassification, new consent switches and unreviewed durable
markers were rejected. A future supported durable native metadata channel needs its own review.
