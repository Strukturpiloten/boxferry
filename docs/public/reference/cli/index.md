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

Podman input defaults to `--podman-import-policy portable`. This reconstructs the reviewed
effective settings and named resource relationships of the selected application. Use
`--podman-import-policy conservative` to retain those observations as evidence; the three
`--promote-podman-*` options for portable settings and named resources then enable individual
families. The host-bind option remains separate in either mode. Explicit promotion flags already
covered by `portable` are redundant. Both modes keep `--loss-policy exact` as the default;
unsupported omissions and exporter approximations still need an explicit loss-policy choice.

| Option                                          | Applies to            | Purpose                                                                                                                                                |
| ----------------------------------------------- | --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `--input-file FILE`                             | Compose/Quadlet input | Add one document in input order; repeat as needed.                                                                                                     |
| `--input-directory DIR`                         | Compose/Quadlet input | Add discovered documents at this position.                                                                                                             |
| `--application-name NAME`                       | Quadlet/Podman input  | Set the neutral application name (optional for Podman).                                                                                                |
| `--project-name NAME`                           | Compose input         | Supply a fallback project name.                                                                                                                        |
| `--project-directory DIR`                       | Compose input         | Resolve project-relative paths from this directory.                                                                                                    |
| `--profile NAME`                                | Compose input         | Activate one profile; repeat as needed.                                                                                                                |
| `--all-profiles`                                | Compose input         | Activate every declared profile.                                                                                                                       |
| `--interpolate`                                 | Compose input         | Enable explicit interpolation.                                                                                                                         |
| `--env-file FILE`                               | Interpolated Compose  | Add assignments; later files win.                                                                                                                      |
| `--env NAME=VALUE`                              | Interpolated Compose  | Add a literal value.                                                                                                                                   |
| `--env NAME`                                    | Interpolated Compose  | Authorize one sensitive process value.                                                                                                                 |
| `--podman-socket PATH`                          | Podman input          | Override local rootless-first socket discovery.                                                                                                        |
| `--podman-all`                                  | Podman input          | Select all eligible application roots.                                                                                                                 |
| `--podman-resource [KIND=]REFERENCE`            | Podman input          | Add an exact resource root; bare references select containers; kinds: container, image, network, pod, secret, volume.                                  |
| `--podman-resource-prefix KIND=PREFIX`          | Podman input          | Add a literal name-prefix root using the same kinds.                                                                                                   |
| `--podman-label NAME[=VALUE]`                   | Podman input          | Add a label root; repeat as needed.                                                                                                                    |
| `--podman-network-boundary NAME_OR_ID`          | Podman input          | Authorize one explicit network crossing; repeatable.                                                                                                   |
| `--podman-import-policy portable\|conservative` | Podman input          | Reconstruct reviewed portable fields by default, or retain them as evidence for selective expert promotion.                                            |
| `--promote-podman-effective-bind-mounts`        | Podman input          | Promote absolute host bind paths for a reviewed target that uses the same paths.                                                                       |
| `--promote-podman-portable-effective-settings`  | Podman input          | Promote reviewed effective environment names, ports, restart, normal health, DNS, and network aliases; never authorizes environment-value acquisition. |
| `--promote-podman-effective-named-volumes`      | Podman input          | Promote effective named volumes to desired state.                                                                                                      |
| `--promote-podman-effective-named-networks`     | Podman input          | Promote effective named networks to desired state.                                                                                                     |

The portable-effective settings flag also covers typed network-internal, subnet, gateway,
lease-range, IPv6-subnet, and standalone-container effective attachment-alias observations. Alias
promotion additionally requires named-network promotion; runtime container-ID aliases never become
portable intent. Pod-member networking remains pod-scoped evidence. Native network fields without a
typed PodmanLens contract remain reported rather than guessed.

In `conservative` mode, promotion flags are independent: named-network promotion authorizes
ownership, portable-effective promotion authorizes its typed internal/IPAM settings, named-volume
promotion authorizes volume identity, and bind promotion authorizes reviewed same-host paths.
The `portable` preset selects the first three together; bind promotion remains independent. No
flag recovers network driver, IPAM driver, standalone IPv6, build recipes, registry provenance, or
path contents. Untyped configuration fields remain actionable omissions with
`available_promotion=none`. Runtime-assigned
container addresses and local-resolution facts remain structured, non-actionable evidence rather
than portable-intent losses.

BoxFerry does not read an implicit `.env` file or the complete process environment.
Podman input with no selector chooses the only native-evidenced container/pod application. Multiple
applications require a bounded terminal choice, including explicit consent for any complete
Compose-project group offered, or an explicit selector; noninteractive ambiguity
and an empty inventory fail without selecting all. Only `--podman-all` requests every eligible root.
Without `--podman-socket`, it checks only `/run/user/<current-uid>/podman/podman.sock` and then
`/run/podman/podman.sock`. Without `--application-name`, it derives a neutral name from the only
non-ID exact resource or only literal prefix; otherwise it uses `podman-import`. An automatically
selected native application uses its native name when valid, but advisory Compose label values
never supply output names. Naming never selects resources. Exact selectors reject globs, regular
expressions, and partial IDs. Prefix
selectors match literal names only. Human output identifies the selected endpoint and graph before
artifacts. Acquisition is bounded and read-only; BoxFerry never invokes the `podman` executable.

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

Default human output groups repeated occurrences by actionable reason. It prints affected counts
and bounded subject/path samples. `--verbose` also expands every diagnostic occurrence;
`--console-format json` emits the complete structured report.

By default, a literal environment value becomes a named required value in the neutral model. All
three exporters omit it and report `services.<service>.environment.<NAME>` as a target prerequisite.
Exact and approximate loss policies block output; `partial` can write an artifact missing that
assignment, which needs review and an explicit value before use. `validate` makes the same planning
decision without writing files. `--environment-values include` explicitly authorizes environment
values in Compose and Quadlet output files; BoxFerry creates these files with owner-only permissions
on Unix. Podman input acquires environment values only when this option and portable-effective
promotion are both active; the default `portable` import policy activates the latter, whereas
`conservative` needs the explicit `--promote-podman-portable-effective-settings` flag. Current
PodmanLens cannot safely render protected inline values, so Podman output with `include` fails
before writing if such a value is present. Diagnostic text, JSON console output, report files and
support ZIPs remain redacted in
either mode. Source environment-file references and host bind paths remain separate prerequisites;
BoxFerry does not read their contents or migrate application data. Generated artifacts can still
contain authored command, health-check, image, and path text; review all output before sharing or
running it. The environment-value switch does not authorize or redact those distinct fields.

## Exit status

| Code | Meaning                                                      |
| ---- | ------------------------------------------------------------ |
| `0`  | The operation completed and requested output was written.    |
| `1`  | Input, validation, conversion, or file I/O failed.           |
| `2`  | The selected loss policy blocked otherwise plannable output. |
