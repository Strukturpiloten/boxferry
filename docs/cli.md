# Command-line interface

The `boxferry` executable is a thin consumer of the public library facade. It owns argument
parsing, authorized file reads and writes, diagnostic presentation, and process status; conversion
rules remain in the reusable adapters and engine.

The executable supports Linux. Windows users run it inside WSL2; see
[platform support](platform-support.md). Native Windows binaries and Windows containers are not
supported.

`convert` and `validate` are the only document-conversion commands. The typed route registry
currently exposes Compose-to-Quadlet and Quadlet-to-Compose; every other selected pair is
unavailable. Ordered file/directory discovery, presentation modes, canonical `--report-file`,
and the automatically named local `--generate-error-report` support bundle are documented in the
[vNext CLI contract](cli-vnext.md) and [Error reports](error-reports.md).

## Generic routes

Compose-to-Quadlet uses:

- one or more `--input-file` or `--input-directory` values in explicit global order;
- an optional fallback `--project-name` when the Compose project has no top-level name;
- repeated `--profile` values or `--all-profiles`;
- optional explicit Compose interpolation through `--interpolate`, repeatable `--env NAME=VALUE`
  inputs, and repeatable sensitive `--env NAME` authorizations;
- an inclusive `--podman-minimum-version`, defaulting to `5.4`;
- an inclusive `--podman-maximum-version`, defaulting to `6.0`;
- `--quadlet-grouping separate` or an explicit `--quadlet-grouping pod` request;
- `--loss-policy exact`, `approximate`, or `partial`; and
- a required `--output-directory` that must not already exist and whose parent must exist.

Quadlet-to-Compose requires a non-stdin `--project-name` and writes one deterministic,
parse-back-validated `compose.yaml` for the rolling Compose Specification. It does not accept
provider or runtime selection flags, infer installed tools, or guarantee that every historical
Compose consumer accepts the result. The internal BoxFerry profile revision is not a Compose
Specification release version and is never emitted as a CLI provider or version choice.

```console
boxferry convert --input-type quadlet --output-type compose \
  --input-directory ./quadlet --project-name example \
  --output-directory ./compose-output
```

Compose interpolation is disabled unless `--interpolate` is present. With interpolation disabled,
BoxFerry does not read any process variable and unresolved expressions remain available to the
import adapter or fail normal source validation.

With `--interpolate`, ComposeLens evaluates each source file before merge using an environment
that starts empty. Compose default, required, and alternative operators therefore work without
giving the conversion ambient process access. Inputs are additive and explicit:

- `--env NAME=VALUE` supplies a literal. The first `=` separates the name, so
  the value may be empty or contain more `=` characters. Command-line arguments may be visible to
  other local processes and CI logs; all interpolation values are protected in reports, but do not
  use this option for secrets.
- `--env NAME` authorizes BoxFerry to read exactly that named process
  variable. Its value is marked sensitive before interpolation and is not included in errors or
  diagnostic formatting.

Names use ComposeLens's interpolation grammar: an ASCII letter or underscore followed by ASCII
letters, digits, or underscores. A missing or non-Unicode authorized process variable fails
before conversion. Environment files are applied in their supplied order, so later files override
earlier files. Explicit `--env` values are applied afterward and override values from every
environment file; repeated explicit `--env` values are applied in command-line order, so the last
value for a name wins. `--env` requires `--interpolate`.

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

BoxFerry writes every retained structured diagnostic to standard error with sensitive fields
redacted; an aggregate error count never replaces the individual diagnostic codes, summaries, and
safe fields. Successful output paths are written to standard output, one per line. JSON console
mode contains the same diagnostic sequence in its `diagnostics` array and writes no human text.
