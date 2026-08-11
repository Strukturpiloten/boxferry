# ADR 0018: Generic CLI and privacy-safe diagnostic support bundle

- Status: accepted
- Date: 2026-08-10
- Amended by: [ADR 0019](0019-generic-cli-route-registry.md)
- Amended by: [ADR 0021](0021-automatic-local-error-report-names.md)

## Context

The first `compose-to-quadlet` command proves one public conversion route, but ADR 0017 requires a
generic source/target CLI. Users also need complete structured failure context that can be attached
to an issue without silently collecting definitions, environment files, runtime state, or ambient
host data.

The current command has a fail-closed interpolation boundary and creates only a completely absent
output directory. ADR 0004 deliberately deferred JSON, overwrite behavior, explicit environment
files, and diagnostic archives until each received a reviewed contract.

Podman's reviewed 5.4.0, 5.8.0, 6.0.2, and current Quadlet manuals do not define a `.quadlets`
single-file bundle. A BoxFerry transport archive must not be described as native Quadlet input.

## Decision

1. Add visible `convert`, `validate`, `capabilities`, `help`, and `version` commands. Standard
   `-h`, `--help`, and `--version` forms remain visible. ADR 0019 supersedes the initial route
   scope: the generic interface exposes the reviewed routes and fails closed for unavailable pairs.
2. ADR 0019 removes the unreleased `compose-to-quadlet` command; `convert` and `validate` are the
   only document-conversion commands.
3. `--input-file` and `--input-directory` form one occurrence-ordered source sequence. Compose
   directory discovery is non-recursive and selects exactly one conventional file using the order
   documented in `docs/cli-vnext.md`. Overrides remain explicit. Duplicate resolved files are
   invalid.
4. Compose interpolation remains opt-in through `--interpolate` during the compatibility period.
   There is no implicit `.env` or complete process-environment import. Repeatable `--env-file`
   inputs are applied in order, explicit `--env` inputs override them, and `--env NAME` authorizes
   only that process value. Environment-file input uses a documented strict BoxFerry assignment
   grammar rather than claiming complete Docker Compose `.env` compatibility. All variable values
   are protected in reports.
5. `--project-directory` replaces the first input file's parent as BoxFerry's explicit Compose
   project root for relative target-path resolution. Document origins remain their actual input
   locations. Stdin requires a project directory. Profiles remain explicit; `--all-profiles` is an
   expert option and never selects among mutually exclusive services.
6. Separate Quadlet containers remain the default. Single-pod grouping is explicit and remains an
   approximation governed by the loss policy. `--pod-name` is valid only with pod grouping; when it
   is absent, the exporter uses the resolved application name and fails if that name is invalid.
7. Podman target selectors accept `major.minor` and `major.minor.patch`. Short minimums resolve to
   the lowest reviewed patch and short maximums to the greatest matching patch in the finite
   catalogue. Requested and resolved forms appear in reports. Major-only and out-of-catalogue
   selectors are invalid.
8. The only native Quadlet output layout in this release is ordinary files. Do not implement or
   advertise `.quadlets` bundle output. A future single-file transport needs a separate format,
   extraction, installation, and compatibility contract.
9. `--output-directory` remains required and must be absent. This release does not add overwrite or
   existing-empty-directory behavior. Conversion and policy authorization complete before writes.
10. Normal human output writes progress and success to stdout and diagnostics to stderr.
    `--verbose`, `--quiet`, and JSON console presentation are mutually exclusive. JSON presentation
    emits exactly one structured document and no human progress. Every import diagnostic remains
    visible independently of verbosity.
11. Add a public, versioned report DTO derived from the same structured diagnostics and outcomes
    used by embedded callers. Schema version 1 permits additive fields but not changed or removed
    meaning. Publish its JSON Schema. Terminal text is never reparsed into JSON.
12. `--report-file` writes canonical JSON with create-new behavior. `--generate-error-report`
    writes a local ZIP containing only fixed `README.md` and `report.json` entries. ADR 0021 defines
    its automatic local name, optional output directory, and console-only published-path field. It
    never uploads, invokes a runtime, captures raw sources or outputs, or reads ambient state beyond
    an explicit allowlist. Local filename selection may inspect `TZ` (including a TZif path) and OS
    time-zone configuration, but never persists or reports those values or paths.
13. Report redaction always replaces `ProtectedString`, environment, command, authorization,
    metadata-label, config/secret material, and runtime-protected values with `<redacted>`. A
    normalized credential-name filter and URL/header/private-key filters add defense in depth.
    Absolute paths and source identities use invocation-local aliases. Every support bundle states
    `review_required: true`; BoxFerry never guarantees heuristic output is safe to publish.
14. Report schema version 1 omits source contents, sanitized source copies, raw panic payloads, and
    backtraces. It caps diagnostics and events at 2,048 each, individual text fields at 16 KiB,
    `README.md` at 128 KiB, `report.json` at 4 MiB, and the stored archive at 5 MiB. Truncation is
    explicit in the report.
15. Build the complete stored ZIP in bounded memory, then publish it without replacing an existing
    path through a same-directory create-new temporary file and no-clobber hard-link step. Failure
    removes only that invocation's temporary file. Filesystems without the required safe primitive
    fail closed; there is no overwrite fallback.
16. A requested report-write failure makes an otherwise successful command fail. When conversion
    already failed, preserve its primary exit category and add a redacted secondary report-write
    error. Argument errors before report-option recognition, aborts, out-of-memory termination, and
    early panics remain explicit noncoverage.
17. Add Serde 1.0.229, serde_json 1.0.151, and ZIP 6.0.0 only to the CLI feature boundary. ZIP uses
    no default compression or crypto features, is MIT-licensed, and supports Rust 1.83.0. The
    workspace MSRV remains 1.85.0, and no-default embedded builds retain their existing surface.

## Consequences

- Scripts receive one stable JSON boundary while human progress no longer appears as an error
  stream.
- ADR 0019 establishes the generic command surface; the unreleased legacy command is not retained.
- BoxFerry's interpolation inputs remain explicit and reproducible, but they intentionally do not
  reproduce Docker Compose's ambient shell and `.env` precedence.
- The support bundle is useful for diagnosis without making a false secret-free guarantee.
- Stored ZIP output keeps the dependency and attack surface small but does not compress reports.
- A native single-file Quadlet layout, overwrite behavior, sanitized inputs, and panic backtraces
  remain deferred rather than being implemented under ambiguous safety claims.

## Alternatives considered

Making generic Compose interpolation ambient by default was rejected for this migration because it
would silently change current security behavior. Hiding conventional help/version flags was
rejected because it harms discoverability. Treating multiple conventional Compose filenames as
merge layers was rejected because they are alternatives. A `.quadlets` output was rejected because
it is not supported by the reviewed Podman manuals. Capturing terminal output or raw source files
was rejected because structured data provides a safer diagnostic boundary. Writing directly to the
final archive path was rejected because a crash can expose a partial ZIP, while ordinary rename
may replace an existing path on supported platforms.
