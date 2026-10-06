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

Podman defaults to `--podman-import-policy portable`: reviewed effective settings and named
relationships are reconstructed. `conservative` requires individual promotion flags. Bind paths
remain opt-in. Both retain `--loss-policy exact`; unsupported omissions and approximations need
explicit authorization. See [Podman input](../../guides/podman-input/) for field boundaries.

| Option                                          | Applies to            | Purpose                                                         |
| ----------------------------------------------- | --------------------- | --------------------------------------------------------------- |
| `--input-file FILE`                             | Compose/Quadlet input | Add one document in input order; repeat as needed.              |
| `--input-directory DIR`                         | Compose/Quadlet input | Add discovered documents at this position.                      |
| `--application-name NAME`                       | Quadlet/Podman input  | Name the application; required for multiple Quadlet files.      |
| `--project-name NAME`                           | Compose input         | Supply a fallback project name.                                 |
| `--project-directory DIR`                       | Compose input         | Resolve project-relative paths from this directory.             |
| `--profile NAME`                                | Compose input         | Activate one profile; repeat as needed.                         |
| `--all-profiles`                                | Compose input         | Activate every declared profile.                                |
| `--interpolate`                                 | Compose input         | Enable interpolation with lazy process-variable fallback.       |
| `--env-file FILE`                               | Interpolated Compose  | Enable interpolation and add assignments; later files win.      |
| `--env NAME=VALUE`                              | Interpolated Compose  | Enable interpolation and add a literal value.                   |
| `--env NAME`                                    | Interpolated Compose  | Enable interpolation and authorize one sensitive process value. |
| `--podman-socket PATH`                          | Podman input          | Override local rootless-first socket discovery.                 |
| `--podman-all`                                  | Podman input          | Select all eligible application roots.                          |
| `--podman-resource [KIND=]REFERENCE`            | Podman input          | Select an exact root; bare references select containers.        |
| `--podman-resource-prefix KIND=PREFIX`          | Podman input          | Add a literal name-prefix root using the same kinds.            |
| `--podman-label NAME[=VALUE]`                   | Podman input          | Add a label root; repeat as needed.                             |
| `--podman-network-boundary NAME_OR_ID`          | Podman input          | Authorize one explicit network crossing; repeatable.            |
| `--podman-import-policy portable\|conservative` | Podman input          | Choose reconstruction or evidence retention.                    |
| `--promote-podman-effective-bind-mounts`        | Podman input          | Authorize reviewed same-host absolute bind paths.               |
| `--promote-podman-portable-effective-settings`  | Podman input          | Promote reviewed fields, not environment-value acquisition.     |
| `--promote-podman-effective-named-volumes`      | Podman input          | Promote named-volume intent.                                    |
| `--promote-podman-effective-named-networks`     | Podman input          | Promote named-network intent.                                   |

Without document selectors, Compose searches only the current directory for one regular,
non-symlink `compose.yaml`, `compose.yml`, `podman-compose.yaml`, `podman-compose.yml`,
`docker-compose.yaml`, or `docker-compose.yml`. Missing, colliding, unreadable or non-regular
candidates fail. `--input-directory` instead uses the first regular file in that order, reporting
ignored candidates. `--project-directory` changes path resolution, not discovery. No `.env`,
overrides or other directories are searched; input is authored files, not deployed services.

`--env` and `--env-file` imply interpolation, but only explicit `--interpolate` authorizes fallback
to other referenced process variables. Explicit assignments override files, later files override
earlier ones, and files override fallback. The process environment is never enumerated.

Podman without selectors chooses one native-evidenced application or requires a bounded terminal
choice/explicit selector; Compose-project groups need consent. Noninteractive ambiguity and empty
inventories fail. Only `--podman-all` selects every eligible root. Discovery checks only
`/run/user/<current-uid>/podman/podman.sock`, then `/run/podman/podman.sock`. Acquisition is read-only,
never invoking `podman`. Naming never selects resources or uses advisory Compose labels.
Exact selectors reject globs, regular expressions and partial IDs; prefixes are literal.
Kinds are container, image, network, pod, secret and volume.

## Output options

| Option                                               | Applies to     | Purpose                                                  |
| ---------------------------------------------------- | -------------- | -------------------------------------------------------- |
| `--output-directory DIR`                             | `convert`      | Write to an absent or existing empty directory.          |
| `--podman-minimum-version VERSION`                   | Quadlet output | Select the inclusive minimum; default resolves to 5.4.0. |
| `--podman-maximum-version VERSION`                   | Quadlet output | Select the inclusive maximum; default resolves to 6.1.2. |
| `--quadlet-grouping separate`                        | Quadlet output | Keep one container unit per service; default.            |
| `--quadlet-grouping pod`                             | Quadlet output | Request one compatible pod.                              |
| `--pod-name NAME`                                    | Pod grouping   | Set the native pod name.                                 |
| `--podman-max-version VERSION`                       | Podman output  | Use newest reviewed exact target at or below ceiling.    |
| `--podman-target-context unknown\|rootless\|rootful` | Podman output  | Select the required explicit target context.             |

Output directories must be absent or empty, including dotfiles; existing files are never replaced.
Podman defaults to exact target 6.1.0: `podman.json` and `podman-commands.sh`. BoxFerry never executes
output or infers targets from the source/development machine.

## Policy and reports

| Option                                      | Purpose                                                                                          |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| `--loss-policy exact\|approximate\|partial` | Authorize documented non-exact output.                                                           |
| `--environment-values withhold\|include`    | Withhold environment values by default; explicitly include them in Compose or Quadlet artifacts. |
| `--console-format json`                     | Emit one machine-readable report.                                                                |
| `--report-file FILE`                        | Write a create-new JSON report.                                                                  |
| `--generate-error-report`                   | Create a local ZIP support bundle.                                                               |
| `--error-report-directory DIR`              | Select or create its direct destination directory.                                               |
| `--include-podman-snapshot`                 | Add always-redacted Podman evidence to a generated ZIP.                                          |
| `--verbose`                                 | Add discovery detail and expand diagnostic occurrences.                                          |
| `--quiet`                                   | Suppress progress and success text.                                                              |

Human output groups diagnostics with counts and bounded samples; verbose expands occurrences,
JSON retains the complete report.

Withheld literal environment assignments become named prerequisites. Exact/approximate policies
block; partial output omits them and needs explicit values before use. `validate` plans identically
without writing. `include` authorizes Compose/Quadlet values in owner-only Unix files. Podman
acquisition additionally requires portable-effective promotion (included in `portable`, explicit
in `conservative`). Protected inline values with `include` block Podman output before writing.
Diagnostics, JSON reports and support ZIPs stay redacted. Service environment-file contents and bind data
are not read; application data is not migrated. Authored commands, healthchecks, images and paths
can remain in artifacts: the environment switch does not redact them. Review output before sharing
or running it.

## Exit status

| Code | Meaning                                                      |
| ---- | ------------------------------------------------------------ |
| `0`  | The operation completed and requested output was written.    |
| `1`  | Input, validation, conversion, or file I/O failed.           |
| `2`  | The selected loss policy blocked otherwise plannable output. |
