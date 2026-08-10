# Conversion model and diagnostics

## Principle

BoxFerry reports semantic fidelity. Successfully producing a file does not by itself mean a conversion succeeded.

## Application graph

The first application model is an ordered graph rather than a flat list of containers. Implemented
nodes and attachments include:

- application
- service
- image and command
- separately named image-acquisition and image-build resources, plus independent service
  references to those resources
- health-check command, disable intent, timing, retries, and startup timing
- ordered service dependencies with readiness, strength, restart intent, and field provenance
- primary user/group, user namespace, ordered supplementary groups, working directory, and
  read-only root-filesystem intent with field provenance
- network
- storage
- application-owned or external configuration and secret resources with optional runtime names
  and explicit file, environment, or inline material origins
- ordered service config/secret grants retaining short/long syntax plus source, target, UID, GID,
  mode, and field provenance
- environment value
- explicit hostname mapping
- published port
- ordered structural service groups with lifecycle ownership and relationship provenance

Service collections express network and storage attachment, exposure, lifecycle ownership, and
provenance. Runtime imports can mark network and volume lifecycle ownership as `Uncertain` when
inspection establishes existence but cannot prove whether a future definition should create or
reuse the resource. Config and secret material values use `ProtectedString`; their presence in the model
does not authorize BoxFerry to read files or process environment variables. Adapters must report
conflicts such as external ownership combined with managed material rather than silently choosing
one interpretation. A service group records co-membership without implying namespace-sharing or
a target workload kind. Build input and richer workload semantics are added with their native
vertical slices. This permits a single-host Compose project,
a Quadlet pod, and a multi-node Kubernetes workload to be compared without treating them as
identical.

Image acquisition and build resources retain their own ordered settings and provenance without
embedding Compose, Quadlet, or runtime types. A service reference does not replace its runtime
image reference, so adapters can preserve source intent and diagnose target-specific loss without
coupling the application graph to a native artifact format.

## Provenance

Every significant model value should be traceable to one of:

- a source file and location
- a runtime observation and resource identifier
- a user override
- a default introduced by a named implementation profile
- a conversion decision

`ProvenanceKind` makes these categories machine-readable. In particular, a runtime observation is
not silently promoted to authored intent; reconstruction inferences remain separate conversion
decisions and participate in fidelity reporting.

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

The first target adapter assigns `BFQ0001` through `BFQ0008` to invalid target ranges, finite
evidence notes, unsupported target mappings, invalid values, native generation failures,
capability decisions, explicit grouping decisions, and dependency semantics. Its warning/error outcomes retain neutral-model
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
