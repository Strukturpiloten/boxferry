# Conversion model and diagnostics

## Principle

BoxFerry reports semantic fidelity. Successfully producing a file does not by itself mean a conversion succeeded.

## Application graph

The application model is a graph rather than a flat list of containers. Expected node types include:

- application
- workload
- container
- image or build input
- network
- storage
- secret
- configuration item
- endpoint
- health check

Edges express dependencies, grouping, attachment, exposure, ownership, and provenance. This permits a single-host Compose project, a Quadlet pod, and a multi-node Kubernetes workload to be compared without treating them as identical.

## Provenance

Every significant model value should be traceable to one of:

- a source file and location
- a runtime observation and resource identifier
- a user override
- a default introduced by a named implementation profile
- a conversion decision

Provenance is necessary for useful diagnostics and for distinguishing explicit values from defaults.

## Conversion outcomes

Each mapped feature receives one outcome:

| Outcome        | Meaning                                                                 |
| -------------- | ----------------------------------------------------------------------- |
| `exact`        | Target expresses the same relevant semantics directly.                  |
| `equivalent`   | Representation differs but expected behavior is equivalent.             |
| `approximated` | Target output is usable with a documented semantic difference.          |
| `manual`       | BoxFerry can generate partial output but a person must complete a step. |
| `omitted`      | Feature was intentionally omitted by configured policy.                 |
| `unsupported`  | No acceptable mapping or fallback exists.                               |
| `unknown`      | BoxFerry cannot prove the source or target behavior.                    |

Unsupported and unknown outcomes are errors by default. Policies may allow approximated, manual, or intentionally omitted outcomes, but they must remain visible in the report.

## Diagnostics

A structured diagnostic contains:

- stable diagnostic code
- severity
- human-readable summary
- source location or runtime resource
- source feature
- target and target profile
- conversion outcome
- explanation
- suggested action
- related documentation or capability evidence

Human and machine-readable renderers consume the same diagnostic objects. JSON output must not be reconstructed from terminal text.

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
