# Quadlet exporter

The `boxferry-quadlet` crate maps BoxFerry's neutral application model into a deterministic set of
Quadlet files through `quadlet-lens` 0.1.1. The `boxferry` facade exposes it through the additive
`quadlet` feature.

## Planning boundary

`QuadletExporter` implements `ExportAdapter`. It accepts an explicit `TargetProfile` whose
implementation is `podman`, evaluates every used native capability through QuadletLens, builds
typed documents, and returns a `ConversionPlan<QuadletOutput>`.

The adapter reads no files, environment variables, or installed Podman version. It writes no
files and does not invoke Podman or systemd. `QuadletOutput` exposes the generated files and the
parse-back-validated `QuadletDocumentSet`; callers decide where authorized output is written.

Generated output can contain sensitive environment values. `Debug` output for `QuadletFile` and
`QuadletOutput` never prints file contents. Calling `QuadletFile::text` is the explicit access
boundary for deployable text.

## First supported target subset

The exporter generates one `.container` file per service by default, so published ports and
network attachments remain owned by their declaring service. It never infers pod grouping.

Callers can explicitly select `QuadletGroupingPolicy::SinglePod`. The adapter accepts that request
only when every service has the same ordered network attachments, no per-service aliases, and no
overlapping declared container or published host port/protocol pair. It then generates one
application-named `.pod`, moves common networks and published ports to `[Pod]`, and links every
container through `Pod=application.pod`. The document-set graph validates all resulting native
references.

Even a compatible pod request is an approximation because the target services share a network
namespace that Compose normally keeps separate. It emits `BFQ0007`, retains every contributing
service origin, and requires `LossPolicy::AllowApproximate`. An incompatible explicit request is
invalid and produces no candidate; the adapter does not silently fall back to separate containers.

It currently maps:

- image references, including `name:tag@digest` spellings;
- exec-form commands whose individual arguments need no systemd quoting;
- literal environment assignments whose names and values need no systemd quoting or specifier
  escaping;
- single published TCP, UDP, and SCTP ports, with an optional IPv4 host address;
- application-owned named volumes through generated `.volume` files;
- external named volumes through direct runtime-name references;
- anonymous mounts, absolute bind sources, and systemd-specifier sources such as `%h/data`;
- `./` and `../` bind sources when the caller configures the exact absolute Compose project root;
- read-only and `z`/`Z` SELinux mount options;
- application-owned networks through generated `.network` files; and
- external networks through direct runtime-name references.

Application-owned resource references are resolved through QuadletLens's document-set graph.
External resources deliberately produce no lifecycle file.

## Explicit losses and limits

The adapter currently reports rather than guesses:

- host-resolved or explicitly unset environment variables;
- shell-form, empty, or quoting-dependent commands;
- quoting-dependent environment values;
- relative bind sources when the caller does not provide their Compose project root;
- IPv6 or otherwise non-simple host-address port spellings;
- container-only exposed ports without host publication;
- implicit resource lifecycles;
- per-network aliases; and
- identifiers requiring systemd unit-name escaping.

Unsupported fields remain in the conversion report and require `LossPolicy::AllowPartial` before
the remaining candidate can be released. Invalid required values and target profiles always block
output.

`QuadletExporter::with_relative_bind_root` supplies the path context explicitly. Resolution is
lexical, does not require the path to exist, and never reads the filesystem. It produces an
absolute Quadlet bind source and rejects roots or traversals that would escape the filesystem root.
Tilde/home and non-POSIX source forms remain explicit losses rather than being treated as the same
thing as systemd `%h`.

## Version evidence

The exporter supports explicit Podman ranges inside QuadletLens's finite verified catalogue,
currently 5.4.0 through 6.0.2. A range below or above that evidence fails closed. When
`podmanMaximumVersion` is omitted, planning evaluates through the catalogue ceiling and emits a
note that later releases remain an assumption.

## Diagnostics

| Code      | Severity      | Meaning                                                                  |
| --------- | ------------- | ------------------------------------------------------------------------ |
| `BFQ0001` | error         | Target implementation or Podman range cannot be planned safely.          |
| `BFQ0002` | note          | Maximum is omitted; the report names the finite verified ceiling.        |
| `BFQ0003` | warning       | Neutral intent is unsupported by the current target mapping.             |
| `BFQ0004` | error         | A required value cannot be emitted safely as native Quadlet.             |
| `BFQ0005` | error         | QuadletLens rejected a generated document or document set.               |
| `BFQ0006` | warning/note  | A required capability is unavailable or deprecated for the target range. |
| `BFQ0007` | warning/error | Explicit pod grouping is approximate or incompatible with source intent. |

Every warning/error fidelity decision carries the contributing neutral-model provenance. Sensitive
values are not copied into diagnostic fields.
