# ADR 0082: Explicit service execution environment deferral

- Status: accepted
- Date: 2026-10-27
- Amends: [ADR 0058](0058-separate-sensitive-value-artifact-authorization.md) and
  [ADR 0061](0061-compose-interpolation-input-authorization.md)
- Preserves: [ADR 0032](0032-future-native-lens-boundaries.md) and
  [ADR 0033](0033-universal-neutral-model-pipeline.md)

## Context

Conversion-time Compose interpolation cannot express a deployment that intentionally obtains a
small named set of values from the service execution environment on its target host. Container
variables and the environment used to expand generated execution arguments are separate scopes.
Provider declarations must not silently authorize process capture, file reads, or disclosure.

## Decision

The first explicit route is Compose to Quadlet. `--quadlet-defer SERVICE:NAME` is an allowlist,
independent of the ordered providers `--quadlet-service-env-file SERVICE=ABSOLUTE_PATH` and
`--quadlet-service-env SERVICE:NAME=VALUE`. Parsing splits only the first service delimiter and
assignment equals sign. Inline input remains raw until controlled value-free validation, then
enters protected storage. Every reference must be declared and covered by an inline provider or
at least one selected external file. Unknown/inactive services, duplicate entries, unbound or
unused names, undeclared inline names, and orphan providers fail even under partial policy.

These options cannot coexist with conversion-time `--env`, `--env-file`, or `--interpolate`.
`ComposeSource::with_interpolation_applied()` records caller-owned native stage evidence for every
applied overlay, including empty-provider and escape-only overlays. CLI always supplies it; embedded
callers must set it after any merge with an overlay. Provider taint and provenance do not establish
processing stage. All overlay results are literal/no-rescan, and deferral rejects them regardless
of builder ordering. Deferral input must be merged with `None`.
No provider file is read, copied, resolved or checked for existence. There is no ambient process
capture. Files are mandatory absolute literal paths with no glob activation; dollar and percent
characters are literal and encoded by the native renderer. Files override all inline assignments;
later files override earlier files, including empty values. Providers are `[Service]` declarations;
container assignments remain `[Container] Environment`. No provider is automatically forwarded.

ComposeLens owns style-aware inspection of decoded uninterpolated source, dollar escapes,
reference operators, bounds, and containing source evidence. Only direct references in plain or
double-quoted environment scalars are mapped. Ordinary eligible authored dollar syntax, including
`$$`, remains opaque source intent without no-opt-in decoding. Excluded quoted/block dollar
values are protected literals with explicit unsupported guidance, never active source intent. Defaults, alternatives, required expressions,
nesting, malformed input, and dollar-bearing single-quoted/block scalars receive field-specific
loss guidance recommending conversion-time resolution. Metadata and unrelated fields are not
inspected as executable expressions. A selected leading-reference short mount is explicitly
promoted to host-bind intent, rather than inferred to be a named volume. The initial bind subset
has one leading direct reference and an optional literal slash suffix. QuadletLens owns encoding,
quoting, expansion escaping, native suffix hazards and reverse decoding; BoxFerry owns no variable
parser. Deferred target paths, options, named volumes and other fields remain unsupported.

An additive `EnvironmentValue::UnresolvedSource` records opaque authored expressions with an
explicit source-format identity. The Compose importer uses it for preexisting unprocessed dollar
syntax, and only a matching Compose exporter forwards that intent. Other targets diagnose it.
Resolved native/inspected/provider values remain `Literal`; provenance is never proof that a
literal may be activated as source syntax. Compose output explicitly refuses literal dollars
until its native renderer provides a safe channel. Withholding protects the opaque expression
as it protected the previous literal representation.

`Mount::with_literal_paths()` explicitly marks decoded, inspected and acquired host-source and
container-target text as literal. Existing mounts keep their target-context interpretation by
default. Exporters never infer this distinction from provenance. Deferred references remain typed
segments, while native literal encoding protects literal dollars/percents in both source and target.

The neutral model retains sourced literal/reference segments, declared names, protected inline
assignments and ordered external files. Expressions and aggregate provider plans are bounded at
1 MiB and 65,536 segments/entries. Segment origins retain containing physical spans, never decoded
scalar offsets mislabeled as physical positions. Application admission checks plan coherence.

ADR 0058 continues to withhold ordinary literal environment assignments. Pure deferred references
can be emitted while withholding; any nonempty literal segment in deferred container environment,
and every inline provider including an empty value, require explicit include authorization. The
Quadlet exporter has the same safe-default explicit policy for embedded callers. Withholding
reports an actionable named prerequisite without rewriting or dropping neutral expression nodes.
Bind suffixes describe path intent rather than acquired environment values. Protected output uses
private file modes, including external-file-only execution plans; reports, Debug and support
artifacts do not expose inline providers.

`BFQ0015` reports informational `external-unverified` requirements. Missing/unset/empty container
environment references contribute empty strings. Effective bind sources must be nonempty absolute
paths without colon, NUL or control characters. Known-invalid inline-only bind roots fail early;
when external files can override them the final value remains an external requirement. Exact
configuration does not mean provider contents or runtime availability were verified.

Existing native capability IDs cover container environment, volume and generic systemd section
pass-through across the explicit supported Podman range. No invented systemd target selector or
provider-specific capability is introduced. Reverse import reports unresolved native execution
references and provider intent as unsupported, never reconstructing deferral consent. Other
exporters explicitly report the deferred expressions and provider plans they cannot represent.

## Verification and consequences

Focused tests cover both native target endpoints, direct/mixed references, source evidence,
unsupported operators/styles/fields, provider scope/order, invalid plans, no acquisition, disclosure,
private output, native literal-dollar/percent parity, reverse limitations and bundle transport.
Executable documentation uses nonexistent external files to exercise configuration without runtime
access. Native generator evidence is distinct from target execution and provider availability.
