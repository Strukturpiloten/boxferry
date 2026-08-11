# Command-line interface

The `boxferry` executable is a thin consumer of the public library facade. It owns argument
parsing, authorized file reads and writes, diagnostic presentation, and process status; conversion
rules remain in the reusable adapters and engine.

This document describes the legacy compatibility command. The implemented generic `convert` and
`validate` interface, ordered file/directory discovery, presentation modes, canonical
`--report-file`, and local `--generate-error-report` support bundle are documented in the
[vNext CLI contract](cli-vnext.md) and [Error reports](error-reports.md).

## Compose to Quadlet

`boxferry compose-to-quadlet` accepts:

- one or more `--file` values in explicit Compose merge order;
- a required fallback `--project-name`;
- repeated `--profile` values or `--all-profiles`;
- optional explicit Compose interpolation through `--interpolate`, repeatable plain
  `--variable NAME=VALUE` inputs, and repeatable sensitive
  `--variable-from-environment NAME` authorizations;
- an inclusive `--podman-minimum-version`, defaulting to `5.4.0`;
- an optional inclusive `--podman-maximum-version`;
- `--grouping separate` or an explicit `--grouping pod` request;
- `--loss-policy exact`, `approximate`, or `partial`; and
- a required `--output-directory` that must not already exist and whose parent must exist.

Versions use the exact numeric `major.minor.patch` form. File order, profile selection, grouping,
target range, and loss authorization are never inferred from an installed Docker or Podman
runtime.

Compose interpolation is disabled unless `--interpolate` is present. With interpolation disabled,
BoxFerry does not read any process variable and unresolved expressions remain available to the
import adapter or fail normal source validation.

With `--interpolate`, ComposeLens evaluates each source file before merge using an environment
that starts empty. Compose default, required, and alternative operators therefore work without
giving the conversion ambient process access. Inputs are additive and explicit:

- `--variable NAME=VALUE` supplies a non-sensitive literal. The first `=` separates the name, so
  the value may be empty or contain more `=` characters. Command-line arguments may be visible to
  other local processes and CI logs; do not use this option for secrets.
- `--variable-from-environment NAME` authorizes BoxFerry to read exactly that named process
  variable. Its value is marked sensitive before interpolation and is not included in errors or
  diagnostic formatting.

Names use ComposeLens's interpolation grammar: an ASCII letter or underscore followed by ASCII
letters, digits, or underscores. A missing or non-Unicode authorized process variable fails
before conversion. Supplying one name more than once, including once through each source, also
fails instead of defining an implicit precedence rule. `--variable` and
`--variable-from-environment` require `--interpolate`.

This boundary does not read an implicit `.env` file or the contents of service-level `env_file`
paths. BoxFerry converts those declarations without opening them: required files with safe paths
become Quadlet `EnvironmentFile=` entries, and Compose-relative paths resolve lexically from the
first input file's absolute project directory. Because Podman's parser parity with Compose's
default and `raw` formats is not yet proven, emitted declarations require `--loss-policy
approximate`. `required: false`, unsafe paths, and source paths that would acquire systemd
specifier semantics require partial output or block stricter policies.

## Output safety

Conversion and loss-policy authorization complete before the output directory is created. The
command creates the directory atomically and therefore refuses any existing file, directory, or
symbolic link at that path. Every generated file
uses create-new semantics. If a later file cannot be written, files created by that invocation are
removed and the new directory is removed only when it is empty.

Applying, enabling, or starting generated Quadlet units is outside this command. Those operations
require a separate explicit workflow.

## Exit status

| Status | Meaning |
| ------ | ------- |
| `0` | Conversion was authorized and every output file was written. |
| `1` | Arguments passed Clap validation but processing or file I/O failed. |
| `2` | Source/profile diagnostics or the selected loss policy blocked output. |

BoxFerry diagnostics are written to standard error with sensitive fields redacted. Successful
output paths are written to standard output, one per line.
