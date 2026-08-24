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

`INPUT` and `OUTPUT` are `compose`, `quadlet`, or `podman`.

## Input options

| Option                                      | Applies to            | Purpose                                              |
| ------------------------------------------- | --------------------- | ---------------------------------------------------- |
| `--input-file FILE`                         | Compose/Quadlet input | Add one document in input order; repeat as needed.   |
| `--input-directory DIR`                     | Compose/Quadlet input | Add discovered documents at this position.           |
| `--application-name NAME`                   | Quadlet/Podman input  | Set the neutral application name.                    |
| `--project-name NAME`                       | Compose input         | Supply a fallback project name.                      |
| `--project-directory DIR`                   | Compose input         | Resolve project-relative paths from this directory.  |
| `--profile NAME`                            | Compose input         | Activate one profile; repeat as needed.              |
| `--all-profiles`                            | Compose input         | Activate every declared profile.                     |
| `--interpolate`                             | Compose input         | Enable explicit interpolation.                       |
| `--env-file FILE`                           | Interpolated Compose  | Add assignments; later files win.                    |
| `--env NAME=VALUE`                          | Interpolated Compose  | Add a literal value.                                 |
| `--env NAME`                                | Interpolated Compose  | Authorize one sensitive process value.               |
| `--podman-socket PATH`                      | Podman input          | Select one local Unix socket explicitly.             |
| `--podman-all`                              | Podman input          | Select all eligible application roots.               |
| `--podman-resource KIND=REFERENCE`          | Podman input          | Add an exact resource root; repeat as needed.        |
| `--podman-label NAME[=VALUE]`               | Podman input          | Add a label root; repeat as needed.                  |
| `--podman-network-boundary NAME_OR_ID`      | Podman input          | Authorize one explicit network crossing; repeatable. |
| `--promote-podman-effective-named-volumes`  | Podman input          | Promote effective named volumes to desired state.    |
| `--promote-podman-effective-named-networks` | Podman input          | Promote effective named networks to desired state.   |

BoxFerry does not read an implicit `.env` file or the complete process environment.
Podman input requires `--podman-socket`, `--application-name`, and at least one of
`--podman-all`, `--podman-resource`, or `--podman-label`. It performs bounded read-only Libpod
acquisition and never invokes the `podman` executable.

## Output options

| Option                                               | Applies to     | Purpose                                                  |
| ---------------------------------------------------- | -------------- | -------------------------------------------------------- |
| `--output-directory DIR`                             | `convert`      | Write to an absent or existing empty directory.          |
| `--podman-minimum-version VERSION`                   | Quadlet output | Select the inclusive minimum; default resolves to 5.4.0. |
| `--podman-maximum-version VERSION`                   | Quadlet output | Select the inclusive maximum; default resolves to 6.0.2. |
| `--quadlet-grouping separate`                        | Quadlet output | Keep one container unit per service; default.            |
| `--quadlet-grouping pod`                             | Quadlet output | Request one compatible pod.                              |
| `--pod-name NAME`                                    | Pod grouping   | Set the native pod name.                                 |
| `--podman-max-version VERSION`                       | Podman output  | Use newest reviewed exact target at or below ceiling.    |
| `--podman-target-context unknown\|rootless\|rootful` | Podman output  | Select the required explicit target context.             |

An output directory containing any entry—including a dotfile—is rejected. BoxFerry never replaces
an existing output file.

Podman output defaults to exact target 6.1.0 and contains reviewable `podman.json` plus runnable
`podman-commands.sh`. BoxFerry never executes the script. The maximum version and target context are
never inferred from the source or development machine.

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
| `1`  | Input, validation, conversion, or file I/O failed.           |
| `2`  | The selected loss policy blocked otherwise plannable output. |
