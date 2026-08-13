# ADR 0021: Automatic local diagnostic report names and publication

- Status: accepted
- Date: 2026-08-11
- Amends: [ADR 0018](0018-generic-cli-and-diagnostic-support-bundle.md)

## Context

ADR 0018 made the diagnostic archive an explicit local output, but its required destination path
made the common failure-reporting workflow needlessly manual. A generated report must retain the
same no-overwrite guarantee without exposing absolute filesystem paths in persisted reports.

## Decision

1. `--generate-error-report` is a value-less opt-in flag. `--error-report-directory DIR` is valid
   only with that flag and selects an existing, non-symlink directory; otherwise the current working
   directory is used. The resolved directory is canonicalized, so every published path is absolute.
2. BoxFerry obtains the local wall-clock time before choosing a name. It fails closed if local time
   is unavailable and never substitutes UTC. This may inspect the standard `TZ` setting, including a
   TZif path, and operating-system time-zone configuration solely to choose the filename. It never
   persists or reports a time-zone setting, name, path, or value. Names are exactly
   `boxferry-error-report-YYYY-MM-DD_HH-MM-SS.zip` in that local clock representation.
3. A same-second collision retries names with `-01` through `-99` before `.zip`. Each candidate is
   published with the existing create-new, no-clobber mechanism; BoxFerry never performs a
   check-then-create sequence or overwrites an archive. Exhausting the base name plus `-01` through
   `-99` (100 candidates) fails.
4. The archive remains local and contains only the fixed `README.md` and `report.json` entries.
   `report.json` in a report file or archive never includes the generated archive's absolute path.
5. After successful archive publication, normal and verbose output append
   `error report: /absolute/path.zip`; quiet output contains only that absolute path. JSON console
   output adds optional top-level `error_report_path`. This presentation-only field is not added to
   the public `ConversionReport` DTO.
6. Add `jiff` 0.2.24, with default features disabled and only `std` plus `tz-system`, at the CLI
   feature boundary. The exact pin records a reviewed fallible local-time API and Rust 1.85
   compatibility.

## Consequences

- Users can request a report with one flag while retaining an explicit directory boundary when
  their workflow needs it.
- Console JSON has an optional additive field; saved and bundled reports remain privacy-stable and
  independent of where the archive was created.
- A machine without usable local-clock support cannot create an ambiguously named report.
- Time-zone discovery remains limited to choosing the local filename and never expands report
  collection or disclosure.

## Alternatives considered

Using UTC on local-clock failure was rejected because it would silently change the documented
local-name meaning. A user-supplied archive filename was rejected for the common workflow because
it complicates retries and collision safety. Adding the absolute path to the public DTO or archived
report was rejected because it would persist host-local location data.
