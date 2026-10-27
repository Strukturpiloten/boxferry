# ADR 0080: Inert vendor metadata through the neutral model

- Status: accepted
- Date: 2026-10-27
- Amends: [ADR 0033](0033-universal-neutral-model-pipeline.md), [ADR 0040](0040-quadlet-native-value-decoder-boundary.md), and [ADR 0041](0041-portable-intent-and-retained-native-evidence.md)
- Preserves: [ADR 0073](0073-compose-alias-sensitivity-and-export-refusal.md)

## Context

Compose root and direct-service `x-*` extensions can describe application catalogues without
changing runtime configuration. Treating them as unsupported loses useful vendor data. Preserving
raw YAML or passing arbitrary native configuration through would violate the neutral pipeline and
would not provide reversible types, ownership or privacy.

## Decision

`Application::vendor_metadata` holds ordered inert typed extensions. Identity is the pair of
application/direct-service scope and exact case-sensitive namespace. The model rejects duplicates,
absent service owners, duplicate mapping keys, nesting deeper than 64 and aggregate accounting above
1 MiB. Nodes and string keys carry source provenance; sensitivity is explicit per node and key. Protected collection construction recursively protects
contained keys and nodes, preserving provenance and safe Debug output even after direct extraction.
Null, booleans, finite decimal number spellings, strings, ordered sequences and ordered string-key
maps are supported. Numbers never pass through binary floating point. Non-decimal YAML numbers,
tags, non-string keys, unresolved aliases and other scopes remain explicit source-aware losses.
Alias resolution belongs exclusively to ComposeLens.

Folded string presentation is retained as an explicit hint, including its semantic string value.
ComposeLens owns generation of validator-compatible folded scalars. Unsupported folded layouts are
reported rather than silently changing their value. Comments, anchors, quote choice, indentation
and other ordinary source formatting are outside fidelity. Metadata never supplies commands,
environment generation, port assignments or runtime policy.

### Quadlet wire version 1

The original namespace `x-yantr` maps to `[X-YANTR]`, using ASCII uppercase only for the native
section. The exact original key is carried in every repeated `Metadata=` record. Case collisions
share a section while retaining distinct exact identities. Namespace spelling must fit the native
`X-` ASCII section grammar. Protected namespaces are rejected before deriving printable sections.

A record is compact strict JSON with exactly `protocol` (`boxferry.vendor-metadata`), `version`
(`1`), `namespace` (`value`, `sensitive`), `scope` (`kind`: `application` or `service`, with `owner`
only for service), `carrier` (unit basename), and `content`. Content has `kind: payload` plus a
`value` node, or `kind: reference`. Every node has `sensitive`, `folded` and a tagged `value`.
Value tags are `null`, `boolean`, `number`, `string`, `sequence`, and `mapping`; non-null tags carry
`value`. Number payloads are validated decimal strings. Mapping payloads are ordered arrays of
`{key: {value, sensitive}, value: node}` entries, retaining key order and taint. No source paths are
serialized; every imported origin comes from the newly supplied native document and entry span.

The wire limit is semantic depth 16, 64 KiB per encoded record and 1 MiB across all records,
including references. These explicit target limits can be narrower than the neutral model.
The existing JSON recursion guard remains enabled. Unknown/duplicate envelope fields, malformed
nodes, contradictory identities and payload conflicts are rejected without exposing parser error
text. Future protocol versions and supported-limit excess produce explicit losses, never truncation.
JSON-shaped `Metadata=` content that fails strict decoding is invalid, including escaped protocol
or field spellings; plain ordinary vendor content remains unsupported. JSON owns string escapes
for CR/LF, quotes, backslashes and Unicode. Dollar and percent signs remain
inert bytes. Native QuadletLens builders own physical-line safety. Records end in `}` and never use
physical continuation.

Native builders are staged until all final filenames exist, then built once. Application payloads
belong to the lexicographically smallest generated unit basename, with references in every other
unit under the same vendor section. Service payloads belong to their owning `.container`. Metadata
does not create a standalone runtime unit. If no owning generated unit exists, export reports loss.

Import scans only the supplied document set. Missing carriers are unsupported with remediation naming
the exact required basename. Correct-carrier identical payloads deduplicate with merged provenance;
conflicting, misplaced and wrong-owner data are invalid, with no last-wins recovery. References
cannot invent payloads. If every metadata-bearing file is removed externally, the remaining document
set cannot prove that metadata ever existed. Ordinary unknown `X-` content remains unsupported.

### Privacy and other routes

Quadlet envelopes durably retain key/node sensitivity. Any protected vendor metadata suppresses the
entire Compose candidate under every loss policy: ordinary YAML cannot durably retain that taint.
This follows ADR 0073's privacy boundary. `--environment-values` grants no metadata consent and is
not reused. Public metadata can freshly round-trip. Debug, diagnostics and reports redact protected
namespaces, keys and values. Podman output reports one unsupported loss per item; Podman import does
not manufacture vendor metadata from observations.

The Quadlet adapter uses the existing serde crate/version for strict typed envelope decoding.
Cargo's existing native manager owns this declaration; there is no new resolved dependency/version,
operational tool pin, extraction path or Renovate manager. Native extension rendering APIs belong to
ComposeLens and QuadletLens, and registry release integration remains a separate publication gate.

## Verification

Focused model and route tests protect typed values, lexical numbers, folded strings, case collisions,
source provenance, namespace and shape limits, safe native bytes, fresh reimports, deterministic fixed
points, canonical multi-file ownership, missing/duplicate/conflicting carriers, future versions,
unknown schema fields, protected Debug/refusals and explicit Podman losses. These tests do not claim
native runtime execution or external vendor UI behavior.
