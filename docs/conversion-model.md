# Conversion model and diagnostics

## Principle

BoxFerry reports semantic fidelity. Successfully producing a file does not by itself mean a conversion succeeded.

## Application graph

The first application model is an ordered graph rather than a flat list of containers. Implemented
nodes and attachments include:

- application
- service
- image and command
- network
- storage
- environment value
- published port

Service collections express network and storage attachment, exposure, lifecycle ownership, and
provenance. Workload grouping, health checks, build input, secrets, configuration objects, and
dependencies are added with the native vertical slices. This permits a single-host Compose project,
a Quadlet pod, and a multi-node Kubernetes workload to be compared without treating them as
identical.

## Provenance

Every significant model value should be traceable to one of:

- a source file and location
- a runtime observation and resource identifier
- a user override
- a default introduced by a named implementation profile
- a conversion decision

Provenance is necessary for useful diagnostics and for distinguishing explicit values from defaults.

## Conversion outcomes

The core planner assigns each mapped subject one fidelity class:

| Outcome       | Meaning                                                                 |
| ------------- | ----------------------------------------------------------------------- |
| `exact`       | Target expresses the relevant intent, even if representation differs.   |
| `approximate` | Target output has a documented semantic difference or manual follow-up. |
| `unsupported` | No acceptable mapping or fallback exists.                               |
| `invalid`     | Source intent or the explicit target profile cannot be planned safely.  |

Detailed diagnostics state whether an approximation needs manual action or an unsupported feature
was intentionally omitted. `ExactOnly`, `AllowApproximate`, and `AllowPartial` policies decide
whether candidate output is released; invalid output is always blocked. Every non-exact outcome
must reference a diagnostic present in the plan.

## Diagnostics

A structured diagnostic currently contains:

- stable diagnostic code
- severity
- human-readable summary
- ordered plain or sensitive fields

Outcomes carry the source provenance that contributed to their decision. Adapter-specific
diagnostics will add source feature, target capability, explanation, suggested action, and evidence
fields without changing the redaction contract.

Human and machine-readable renderers consume the same diagnostic objects. JSON output must not be reconstructed from terminal text.

The first target adapter assigns `BFQ0001` through `BFQ0007` to invalid target ranges, finite
evidence notes, unsupported target mappings, invalid values, native generation failures, and
capability or explicit grouping decisions. Its warning/error outcomes retain neutral-model
provenance, while sensitive environment contents remain absent from diagnostic fields. The
complete boundary is documented in the [Quadlet exporter](quadlet-adapter.md).

## Target ranges

Output must be compatible with the entire configured target range. For example, a Quadlet target with `podmanMinimumVersion = "5.4"` may not require a key introduced later unless a compatible fallback is selected.

When a maximum version is omitted, the report states the latest version covered by the installed capability catalogue. Future compatibility is an assumption, not a fact.

## Round trips

Round-trip equality has several levels:

1. textual equality
2. native-model equality
3. application-model equality
4. operational equivalence

BoxFerry normally targets application-model equality or documented operational equivalence. Textual equality is the responsibility of a native Lens library when it supports lossless editing.
