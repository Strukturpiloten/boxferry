# CLI reference

BoxFerry selects the input and output formats positionally:

<!-- boxferry-example: route-help -->

```console
boxferry convert compose quadlet --help
```

Use route-specific `--help`; it shows only applicable options.

## Commands

| Command                 | Purpose                                                     |
| ----------------------- | ----------------------------------------------------------- |
| `convert INPUT OUTPUT`  | Plan and write authorized output.                           |
| `validate INPUT OUTPUT` | Run the same planning path without writing converted files. |
| `capabilities`          | List supported routes and target ranges.                    |
| `rules`                 | List diagnostic rules.                                      |
| `explain CODE_OR_NAME`  | Explain one diagnostic rule.                                |

`INPUT` and `OUTPUT` are `compose` or `quadlet` in the current release.

## Input options

| Option                    | Applies to           | Purpose                                             |
| ------------------------- | -------------------- | --------------------------------------------------- |
| `--input-file FILE`       | All routes           | Add one document in input order; repeat as needed.  |
| `--input-directory DIR`   | All routes           | Add discovered documents at this position.          |
| `--application-name NAME` | Quadlet input        | Set the neutral application name.                   |
| `--project-name NAME`     | Compose input        | Supply a fallback project name.                     |
| `--project-directory DIR` | Compose input        | Resolve project-relative paths from this directory. |
| `--profile NAME`          | Compose input        | Activate one profile; repeat as needed.             |
| `--all-profiles`          | Compose input        | Activate every declared profile.                    |
| `--interpolate`           | Compose input        | Enable explicit interpolation.                      |
| `--env-file FILE`         | Interpolated Compose | Add assignments; later files win.                   |
| `--env NAME=VALUE`        | Interpolated Compose | Add a literal value.                                |
| `--env NAME`              | Interpolated Compose | Authorize one sensitive process value.              |

BoxFerry does not read an implicit `.env` file or the complete process environment.

## Output options

| Option                             | Applies to     | Purpose                                                  |
| ---------------------------------- | -------------- | -------------------------------------------------------- |
| `--output-directory DIR`           | `convert`      | Write to an absent or existing empty directory.          |
| `--podman-minimum-version VERSION` | Quadlet output | Select the inclusive minimum; default resolves to 5.4.0. |
| `--podman-maximum-version VERSION` | Quadlet output | Select the inclusive maximum; default resolves to 6.0.2. |
| `--quadlet-grouping separate`      | Quadlet output | Keep one container unit per service; default.            |
| `--quadlet-grouping pod`           | Quadlet output | Request one compatible pod.                              |
| `--pod-name NAME`                  | Pod grouping   | Set the native pod name.                                 |

An output directory containing any entry—including a dotfile—is rejected. BoxFerry never replaces
an existing output file.

## Policy and reports

| Option                                      | Purpose                                      |
| ------------------------------------------- | -------------------------------------------- |
| `--loss-policy exact\|approximate\|partial` | Authorize documented non-exact output.       |
| `--console-format json`                     | Emit one machine-readable report.            |
| `--report-file FILE`                        | Write a create-new JSON report.              |
| `--generate-error-report`                   | Create a local ZIP support bundle.           |
| `--error-report-directory DIR`              | Select its existing destination directory.   |
| `--verbose`                                 | Add discovery and version-resolution detail. |
| `--quiet`                                   | Suppress progress and success text.          |

## Exit status

| Code | Meaning                                                      |
| ---- | ------------------------------------------------------------ |
| `0`  | The operation completed and requested output was written.    |
| `1`  | Input, validation, execution, or file I/O failed.            |
| `2`  | The selected loss policy blocked otherwise plannable output. |
