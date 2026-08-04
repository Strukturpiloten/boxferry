# Command-line interface

The `boxferry` executable is a thin consumer of the public library facade. It owns argument
parsing, authorized file reads and writes, diagnostic presentation, and process status; conversion
rules remain in the reusable adapters and engine.

## Compose to Quadlet

`boxferry compose-to-quadlet` accepts:

- one or more `--file` values in explicit Compose merge order;
- a required fallback `--project-name`;
- repeated `--profile` values or `--all-profiles`;
- an inclusive `--podman-minimum-version`, defaulting to `5.4.0`;
- an optional inclusive `--podman-maximum-version`;
- `--grouping separate` or an explicit `--grouping pod` request;
- `--loss-policy exact`, `approximate`, or `partial`; and
- a required `--output-directory` that must not already exist and whose parent must exist.

Versions use the exact numeric `major.minor.patch` form. File order, profile selection, grouping,
target range, and loss authorization are never inferred from an installed Docker or Podman
runtime.

The command deliberately performs no Compose interpolation from the process environment. This
prevents a conversion from silently embedding workstation or CI secrets. Unresolved expressions
remain available to the import adapter and can produce policy-controlled diagnostics. A future
explicit environment input must define parsing, precedence, provenance, and secret handling before
it is added.

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
