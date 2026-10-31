# Compatibility

The CLI supports Linux. The Unreleased development source exposes all 16 Compose, Docker,
Quadlet, and Podman routes; installed releases may expose fewer. This matrix describes development
functionality, not native consumer/application or release qualification.

| Input   | Output  | Target contract                                    |
| ------- | ------- | -------------------------------------------------- |
| Compose | Docker  | Explicit exact reviewed Docker profile; inert plan |
| Quadlet | Docker  | Explicit exact reviewed Docker profile; inert plan |
| Podman  | Docker  | Explicit exact reviewed Docker profile; inert plan |
| Docker  | Compose | Rolling Compose Specification generation           |
| Docker  | Quadlet | Podman 5.4.0 through 6.1.2 by default              |
| Docker  | Podman  | Reviewed exact Podman target, 6.1.0 by default     |
| Docker  | Docker  | Explicit exact reviewed Docker profile; inert plan |
| Compose | Compose | Rolling Compose Specification generation           |
| Compose | Quadlet | Podman 5.4.0 through 6.1.2 by default              |
| Compose | Podman  | Reviewed exact Podman target, 6.1.0 by default     |
| Quadlet | Compose | Rolling Compose Specification generation           |
| Quadlet | Quadlet | Podman 5.4.0 through 6.1.2 by default              |
| Quadlet | Podman  | Reviewed exact Podman target, 6.1.0 by default     |
| Podman  | Compose | Rolling Compose Specification generation           |
| Podman  | Quadlet | Podman 5.4.0 through 6.1.2 by default              |
| Podman  | Podman  | Reviewed exact Podman target, 6.1.0 by default     |

Run `boxferry capabilities` for the installed build's exact route and version data.

## Podman input

Input compatibility and output targets are separate contracts. The finite input catalogue is:

| Podman line | Accepted versions                                                         |
| ----------- | ------------------------------------------------------------------------- |
| 3.0         | 3.0.1                                                                     |
| 3.4         | 3.4.4                                                                     |
| 4.3         | 4.3.1                                                                     |
| 4.9         | 4.9.3 and 4.9.4                                                           |
| 5.4–5.8     | The reviewed minor line, starting at 5.4.0, 5.5.0, 5.6.0, 5.7.0, or 5.8.0 |
| 6.0–6.1     | The reviewed minor line, starting at 6.0.0 or 6.1.0                       |

The selected read-only Libpod service reports its Podman and API versions. BoxFerry fails closed
outside this catalogue. Use `boxferry capabilities --verbose` for precise inclusive and exclusive
bounds in the installed build. Legacy input is migration evidence, not a claim that BoxFerry can
generate output for that old version.

## Output target versions

Quadlet output must be valid across the complete selected Podman range. BoxFerry does not inspect
the installed Podman or systemd version. A feature introduced after the selected minimum is used
only when a reviewed fallback exists across the range.

Podman deployment output selects one exact reviewed target: 5.4.0, 5.5.0, 5.6.0, 5.7.0, 5.8.6,
6.0.0, or 6.1.0. The default is 6.1.0. A Podman 3.x or 4.x source can therefore migrate to
Compose, Quadlet, or a reviewable modern Podman plan.

`--podman-max-version` chooses the newest exact target no greater than its ceiling.
`--podman-target-context` explicitly chooses `rootful`, `rootless`, or `unknown`. BoxFerry never
infers the output context from the source or development machine.

Compose output is provider-neutral. The CLI does not ask for a Docker Compose or podman-compose
version and does not claim every historical provider accepts the document. Embedded Rust callers
can select exact provider-aware targets through the library API.

## Docker development boundaries

Docker input uses finite DockerLens native version/context evidence, not an arbitrary Engine/API
range. Docker output requires `--docker-target` to resolve exactly one reviewed native profile:

| Preset                      | Engine/build                                                        | Daemon mode |
| --------------------------- | ------------------------------------------------------------------- | ----------- |
| `debian11-20.10.5-rootful`  | Debian 11 Engine `20.10.5+dfsg1`, package `20.10.5+dfsg1-1+deb11u2` | rootful     |
| `debian11-20.10.5-rootless` | Debian 11 Engine `20.10.5+dfsg1`, package `20.10.5+dfsg1-1+deb11u2` | rootless    |
| `upstream-29.8.1-rootful`   | Upstream Engine `29.8.1`                                            | rootful     |
| `upstream-29.8.1-rootless`  | Upstream Engine `29.8.1`                                            | rootless    |

Each preset resolves native API versions and reviewed capability evidence from DockerLens.
The shorter Debian preset spelling is not a generic upstream `20.10.5` claim. Neither an observed
source daemon mode nor the development host selects an output target. Unknown source mode is
retained as evidence; mode-sensitive intent still needs its own prerequisites.

See [Docker input and artifacts](../../guides/docker-input/) for selection, reconstruction, consent
and output obligations. Source functionality and native Lens evidence do not establish actual
BoxFerry consumer/application acceptance or normal registry compatibility. Compatible published
dependencies and ordinary API/package and complete checks remain required.

## Platform support

- Linux is supported.
- Windows users run the Linux CLI in WSL2.
- Native Windows containers and Windows path semantics are not supported.
- Linux CI validates deterministic and native behavior. macOS client-side compatibility is
  intended but unverified; no macOS runner or native Quadlet runtime claim is made.

## Scope

Podman input is an acquired runtime inventory and resource graph. Podman output is a desired,
review-only deployment artifact and cannot be re-imported as runtime evidence. Docker input is
read-only inventory; Docker output is inert native requests and conditional dependency decisions,
never observed input evidence. Kubernetes remains deferred. BoxFerry has no apply/deploy command.

Recognizing a native key does not guarantee every value converts exactly. When source and target
semantics differ, the diagnostic and loss policy are the compatibility contract.

## Vendor metadata

Compose application-root and direct-service `x-*` extensions can round-trip through inert Quadlet
vendor sections: `x-yantr` becomes `[X-YANTR]`, while exact case-sensitive identities stay distinct.
Supported values include nulls, booleans, decimal numbers retained as text, strings, ordered sequences
and string-keyed mappings. Folded `>` strings retain their semantic value and presentation hint;
comments, anchors and ordinary YAML formatting are not retained. Unsupported shapes or scopes produce
loss diagnostics. No metadata is executed or interpreted as runtime configuration.

Application metadata uses one canonical existing unit as its carrier and references it from the
other generated units. Supply the complete document set when importing. A missing carrier is an
explicit loss identifying the required basename; removing every metadata-bearing file cannot be
detected from the remaining files. Version 1 permits depth 16, 64 KiB per encoded record and 1 MiB
across records. Metadata never creates a runtime unit solely to carry data.

Public metadata can return to Compose. Protected metadata blocks the whole Compose candidate under
every loss policy because fresh YAML cannot retain sensitivity. The environment-value option does
not authorize declassification. Podman output reports metadata as unsupported; keep its source
separately when choosing that route.

## Quadlet transport

`quadlet.bundle` is a BoxFerry transport format, not native Podman input. Extract it into native
units before installation. Packaging retains authorized contents and references; conversion from
bundle members still uses native parsing and the neutral model. Transport extraction alone makes
no semantic compatibility claim. See [bundle guidance](../../guides/quadlet-bundles/) for framing
limits, safe extraction and report membership.

## Deferred service execution environment

Compose-to-Quadlet supports explicitly allowlisted direct references in plain/double-quoted
container environment scalars and leading-reference host-bind sources with safe literal suffixes.
Operators/defaults, single-quoted/block dollar expressions and other deferred fields remain
unsupported. Mandatory external provider files are ordered after protected inline providers;
later files win, including empty overrides. File contents and final runtime values remain
external-unverified (`BFQ0015`). Configuration can be exact without proving runtime availability.
A bind root must ultimately be a nonempty absolute path without colons or control characters.
Native unit-reference suffix hazards remain unsupported. Reverse conversion reports unresolved
execution references and service-provider intent as losses; it never reconstructs caller consent.

Native-decoded and provider-supplied literal dollars cannot currently be preserved safely by the
Compose generation API. Compose export reports an explicit loss for those environment values and
bind paths rather than activating them as references. Opaque unprocessed eligible plain/double-quoted Compose environment
expressions keep the existing matching-format forwarding behavior and separate unresolved-source
diagnostics; they are represented distinctly from resolved literal values in the embedding API.

Embedded callers must mark every native interpolation overlay with
`ComposeSource::with_interpolation_applied()`, including empty-provider and escape-only overlays.
Those results remain literal and are never rescanned. Deferral requires a merge with no overlay;
the input processing stage is explicit caller context, independent of provenance and sensitivity.
