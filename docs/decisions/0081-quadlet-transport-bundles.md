# ADR 0081: Bounded Quadlet transport and explicit native extraction

- Status: accepted
- Date: 2026-10-27
- Supersedes: [ADR 0018](0018-generic-cli-and-diagnostic-support-bundle.md) decision 8, and its decision 17 only for Serde/serde_json's feature boundary
- Preserves: [ADR 0025](0025-empty-output-directories-and-final-human-status.md), [ADR 0033](0033-universal-neutral-model-pipeline.md), and [ADR 0063](0063-sole-quadlet-unit-application-identity.md)

## Context

A complete generated Quadlet application may contain multiple interdependent native files. A
single transport artifact helps sharing and review, but Podman does not treat `---` separators as
a native multi-document unit. Transport extraction must preserve native bytes without pretending
to be semantic conversion or installation.

## Decision

The facade owns immutable `QuadletBundle` and `QuadletBundleMember` APIs behind `quadlet`, including
construction from authorized native output, bounded decoding/encoding and explicit extraction.
Native Lens parsing/generation remains unchanged. Existing pinned Serde/serde_json dependencies are
also enabled by `quadlet` for strict transport headers; no new package version, operational pin or
Renovate manager is introduced. Cargo's existing native manager owns these declarations.

### Version 1 framing

The exact ASCII preamble is `BOXFERRY-QUADLET-BUNDLE 1\n`, followed by `members N\n`. Each of the N
members has `---\n`, a compact JSON object line with exactly `name` and `bytes`, then the declared
number of UTF-8 payload bytes, then one mandatory LF framing byte. EOF immediately follows the last
framing byte. Payloads may contain arbitrary separators, framing-looking text, CRLF, Unicode and no
final newline. The mandatory framing LF is not part of the native payload. Byte lengths, rather
than delimiter searches, determine member boundaries.

The member count uses canonical unsigned decimal with no signs or leading zeros. Counts are 1–4096.
Header `bytes` is a nonnegative integer; negative, floating-point and string lengths are rejected.
Duplicate, unknown or missing header fields are rejected after JSON escape decoding. Encoding emits
canonical compact JSON and preserves member order and every native payload byte. Decoding accepts
ordinary JSON whitespace within the bounded single header line. Empty native payloads are legal
transport data; native validity is a separate concern.

The complete serialized transport is at most 64 MiB, each member at most 8 MiB, and each JSON header
line including LF at most 16 KiB. Checked arithmetic precedes payload allocation/slicing. Unsupported
reserved versions, malformed framing, overflow, short content, invalid UTF-8 boundaries and trailing
bytes fail with value-free errors. Debug output omits payloads. The exact native basename and inferred
unit type are retained, without renaming.

A member must pass native `UnitFileName` validation and portable basename restrictions: no paths,
drive colon, slash/backslash, dot traversal, NUL or control characters. The owning native filename type recognizes
`.container`, `.volume`, `.network`, `.pod`, `.image`, `.build`, `.kube` and `.artifact`.
Transport of a native-only unit type does not implement another semantic adapter. Exact duplicates and ASCII
case-insensitive name collisions are rejected across the complete expanded input set when a bundle
is present. Bundle and native-file inputs cannot shadow one another.

### CLI and output boundary

Native files remain the default layout. `--output-layout quadlet-bundle` is an explicit option on
all Quadlet-output conversion and validation routes, producing the physical artifact `quadlet.bundle`.
Grouping remains an independent topology decision. Framing occurs after normal environment-value
and loss-policy authorization; it cannot hide a blocked conversion or remove diagnostics.

Explicit Quadlet input files with the reserved magic are expanded before application-name inference,
duplicate checking and source identity allocation. A sole member uses its native stem for inference;
multiple members require an explicit application name. Directory discovery remains native-only.
Conversion still uses native parser → importer → neutral model → exporter. Bundled source origins
use invocation-local transport/member aliases, never raw input names in reports.

`extract quadlet` validates the entire transport before any output write, then writes byte-identical
native members. It does not parse native semantics, apply loss policy, redact payloads, install units,
run a generator or mutate a runtime. Output must be absent with an existing parent or an existing
empty non-symlink directory, including no dotfiles. A shared facade writer uses create-new files,
tracks only invocation-owned files and removes only those after failure. It removes the directory
only when this invocation created it. Bundled conversion and protected-metadata Quadlet output also use private permissions, derived from
the typed neutral metadata rather than envelope parsing or environment consent. Extraction uses
Unix 0600 files and 0700 newly-created
directories; an existing caller directory keeps its permissions. The path-based writer makes no
claim of protection against hostile directory replacement races.

The additive, non-exhaustive facade `report::InvocationReport` envelope owns `operation`,
`bundle_members` and `input_bundle_members`, and flattens its original `ConversionReport`
when serialized. `ConversionReport` retains its original exhaustive construction, bounds and
serialization contract. Embedders can construct and inspect either type without CLI support;
conversion accessors and explicit envelope bounds/reduction include the transport metadata.
All CLI report and support-bundle paths serialize the envelope. Physical `output_artifacts`
contains one bundle for bundled conversion or the native files for extraction. Member records retain
native output names, types and byte sizes; input provenance uses aliases. Extraction reports no
conversion fidelity outcomes and explicitly labels target versions not applicable. Schema v1 adds
optional properties for old-report compatibility; new reports emit them. Existing bounds, explicit
truncation and private diagnostic/support-report behavior cover the new collections.

## Verification

Independent framing tests cover version/count/header/length/EOF failures, JSON duplicate/unknown
fields, UTF-8 byte boundaries, separator-looking payloads, CRLF and final-newline fidelity, empty
members, count/size limits, name traversal/collisions, private modes, overwrite/symlink refusal and
owned cleanup after partial writes. CLI tests exercise Compose, Quadlet and cassette-only Podman
output routes, validate without writes, expansion before naming, mixed-input collisions, reports,
source privacy, native defaults and the distinction between real conversion losses and extraction.
Public facade checks include Quadlet without CLI. None of this constitutes native installation.
