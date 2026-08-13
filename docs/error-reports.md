# Error-report contract

## Purpose and status

This accepted contract defines a local diagnostic support bundle that users can inspect and attach
to a GitHub issue. It is recorded by [ADR 0018](decisions/0018-generic-cli-and-diagnostic-support-bundle.md)
and [ADR 0021](decisions/0021-automatic-local-error-report-names.md). Its purpose is to give
maintainers the complete safe
diagnostic context that normal console output cannot provide, without automatically collecting or
uploading source files, credentials, or ambient host state.

The implemented option is:

```console
boxferry convert <OPTIONS> \
  --generate-error-report
```

By default, the archive is created in the current working directory. Add
`--error-report-directory DIR` to use an existing non-symlink directory instead. BoxFerry uses the
local wall clock to generate `boxferry-error-report-YYYY-MM-DD_HH-MM-SS.zip`; it fails if the local
clock is unavailable rather than falling back to UTC. Filename selection may inspect the standard
`TZ` setting, including a TZif path, and operating-system time-zone configuration; these values,
names, and paths are never persisted or reported. Create-new publication prevents replacement of
an existing report. A same-second collision tries the base name plus suffixes `-01` through `-99`
(100 candidates), then fails.
The bundle is generated locally and is never uploaded, submitted to GitHub, or sent over the
network by BoxFerry. It is independent of `--verbose`, `--quiet`, and console JSON mode and always
contains the maximum safe structured detail.

The standard-library publication boundary refuses observed symlink parents and destination
collisions, with no overwrite fallback. It cannot defend against a hostile local actor replacing a
parent path concurrently while that actor has permission to mutate the directory hierarchy.

Despite the option name, an explicitly requested bundle is produced for success, non-exact
success, or failure. This lets users report output defects that do not cause a non-zero exit. It
cannot cover argument errors that occur before the option can be parsed, process aborts, out-of-
memory termination, or every panic before report initialization.

## Default bundle

The ZIP archive has fixed entry names rather than user-derived paths:

- `README.md` contains the fixed `review_required: true` warning, the fixed archive-content and
  omission description, instructions to inspect both entries, and GitHub issue attachment guidance.
- `report.json` contains the versioned machine-readable diagnostic report, including the status
  summary.

The default bundle contains no Compose, Quadlet, environment-file, generated-artifact, runtime-
inspection, or other source-file contents. It also does not run Docker, Podman, systemd, Git, or a
network request to collect additional information.

`report.json` includes:

- report schema and BoxFerry versions;
- completion status, exit category, and failed stage;
- a sanitized invocation command kind and actual command-line option names, without values;
- operating-system family and architecture, without hostname or username;
- aliased input order and directory-selection decisions;
- those aliased inputs and decisions also when Compose project-root resolution, target-version
  selection, or interpolation setup fails after input resolution;
- source and target kinds, project/profile/grouping choices, and loss policy;
- route-specific target choices: finite Podman bounds for Quadlet output, or the rolling Compose
  Specification target for Compose output;
- every retained Compose and Quadlet native diagnostic, with its native source code, producer,
  stage, severity, value-free summary, labels, notes, help, and aliased spans;
- requested and resolved route target versions or `rolling` target labels;
- ordered safe diagnostic events and fidelity counts;
- every structured diagnostic currently exposed by the public report DTO, including bounded
  native producer, stage, protected fields, labelled ranges, notes, and help; and
- generated artifact names and sizes, but not their contents;
- redaction counts, applied redaction classes, and known limitations; and
- explicit truncation metadata for every bounded collection or field.

The conversion-report v1 JSON Schema validates both the persisted public DTO and the console JSON
variant's optional `error_report_path`. That path is presentation-only: `--report-file` output and
the archive's `report.json` must omit it.

Absolute paths are replaced with stable invocation-local aliases such as `<project>`, `<input-1>`,
`<output>`, and `<absolute-path>/basename`. Raw runtime IDs, current-working-directory parents,
home directories, usernames, hostnames, IP addresses discovered from the host, and the complete
process environment are not collected.

## Redaction contract

The fixed placeholder is `<redacted>`. Report construction operates on typed diagnostic and
configuration data, not by recording terminal output. It applies these rules before serialization:

1. `ProtectedString` values are never exposed.
2. Every environment variable, command-line variable, secret/config material, command argument,
   metadata-label value, authorization value, and runtime-observed protected value is replaced,
   regardless of its name.
3. A mapping field or variable whose normalized name indicates a credential has its complete value
   or subtree replaced. Matching is ASCII case-insensitive and splits camel case plus `.`, `-`,
   and `_` separators.
4. Remaining plain strings are filtered for URL user information, authorization headers, private-
   key blocks, and named assignments before inclusion.
5. Paths and source identities use the aliases described above.
6. Serialization tests scan the finished archive entries for seeded canary secrets before
   accepting the bundle implementation.

The initial credential-name set includes:

- `password`, `passwd`, `passphrase`, `pwd`, and the exact token `pw`;
- `secret`, `client_secret`, and `secret_key`;
- `token`, `access_token`, `refresh_token`, and `api_token`;
- `credential`, `credentials`, and `authorization`;
- `private_key`, `apikey`, and `api_key`.

For example, `DB_PASSWORD`, `databasePassword`, `client-secret`, and `DB_PW` all retain their field
name but use `<redacted>` as the value. A repeatable future
`--error-report-redact-name NAME` option may add project-specific names to this set; it can only
increase redaction.

Name and value heuristics cannot prove that arbitrary user text is secret-free. A value stored
under an unrelated name may still be sensitive. The bundle therefore carries `review_required:
true`, and the CLI tells the user to inspect it before upload. BoxFerry must never describe the
archive as guaranteed safe to publish.

## Deferred sanitized inputs

The first implementation omits input contents. A later explicit option may add sanitized
copies:

```console
--error-report-include-sanitized-inputs
```

This option requires a native format-aware sanitizer, separate tests for malformed input, and a
clear preview in `README.md`. It must fail closed: when BoxFerry cannot parse or safely sanitize a
document, that document is omitted with a diagnostic. It must never fall back to copying raw
bytes. Environment-file contents, mounted secrets, runtime inspect payloads, and generated files
remain excluded unless they receive their own separately reviewed contracts.

## User workflow

After a failed or suspicious conversion, the user:

1. reruns the same command with `--generate-error-report`, optionally adding
   `--error-report-directory DIR`;
2. opens the bundle and reviews `README.md` and `report.json`;
3. removes the bundle instead of uploading it when it contains unwanted context; and
4. creates a GitHub issue with a short problem description and attaches the reviewed ZIP file.

Scripts may request the report together with `--console-format json`. After successful publication,
normal and verbose output append `error report: /absolute/path.zip`; quiet output contains only the
absolute path; and JSON output adds optional top-level `error_report_path`. The absolute path is a
console-only presentation value: it is absent from `--report-file` output and the archive's
`report.json`. Report generation does not change the conversion result. If conversion succeeds but
the explicitly requested report cannot be written, the command fails because a requested output is
missing. If conversion already fails and report creation also fails, BoxFerry preserves the primary
conversion exit category and emits a separate redacted report-write error.

`--report-file FILE` and `--generate-error-report` may be used together. BoxFerry attempts
the report file first, then builds and publishes the support bundle from the resulting report. This
deterministic order lets the bundle include a report-file write failure. A support-bundle failure
occurs after the bundle's own `report.json` has been finalized, so it is recorded in the final
console report (and not retroactively in an already published report file). In either case, an
earlier conversion failure retains its category and stage while the report-write diagnostic is
added as a secondary event.

The version-one report DTO contains its command kind and allowlisted actual option names,
operating-system family and architecture, finite target bounds, fidelity counts, output artifact
metadata, redaction summary, and truncation metadata. Structured diagnostics contain the BoxFerry
rule code and name, optional native source code, severity, summary, static help, safe fields,
aliased spans, and an optional detailed native-finding object. Blocked and failed reports also
contain the primary diagnostic code and causal failure summary plus a structured `fix_first`
object with the selected rule's code, name, description, static help, and conservative rerun
guidance. Successful reports use `null`. These catalogue-derived fields contain no source or
environment values. Source contents, generated contents, process environment, runtime inspection,
hostname, username, panic payloads, and backtraces remain excluded. Per-stage durations and a
distinct capability-decision collection remain deferred.

## Ownership and testing

The public engine and adapters own structured diagnostics, fidelity outcomes, provenance, and
sensitivity classification. A public report DTO must expose that information without requiring
terminal parsing. The CLI owns invocation sanitization, host metadata allowlisting, path aliasing,
ZIP construction, and local file writes. Archive packaging must not introduce private conversion
decisions that embedded callers cannot obtain.

Tests must cover:

- every default and user-added credential-name spelling;
- all protected model values and seeded canary secrets;
- URLs, authorization headers, private keys, paths, panic text, and malformed native input;
- deterministic field ordering and a versioned JSON schema;
- archive entry-name safety, size limits, and refusal to overwrite;
- report generation in normal, verbose, quiet, JSON, success, policy-blocked, and failed modes;
- no source contents or ambient environment values in the default bundle; and
- failure to sanitize optional input without a raw-content fallback.

## Version-one limits and deferred work

- Schema version 1 accepts additive fields but does not change or remove existing meaning.
- Diagnostics and events are capped at 2,048 entries each; text fields are capped at 16 KiB.
- `README.md` is capped at 128 KiB, `report.json` at 4 MiB, and the stored ZIP at 5 MiB.
- Raw panic payloads and backtraces are omitted in version one.
- Sanitized input support and a GitHub attachment checklist remain deferred.

Compose and Quadlet reports retain the protected native findings exposed by their adapters;
formatted terminal output is never parsed. Native spans use only `<input-N>` aliases. Focused
black-box tests seed path and secret canaries in console JSON, report files, and support bundles.
