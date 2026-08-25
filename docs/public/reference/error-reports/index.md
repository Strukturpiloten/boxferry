# Error reports

`--generate-error-report` creates a local ZIP for a bug report. It never uploads anything.

Create a report in an existing directory:

<!-- boxferry-example: error-report -->

```console
boxferry validate compose quadlet --input-file compose.yaml --generate-error-report --error-report-directory reports
```

Without `--error-report-directory`, the current directory is used. The local-time filename is:

```text
boxferry-error-report-2026-08-19_16-42-08.zip
```

Same-second collisions add `-01` through `-99`. Existing files are never replaced. After creation,
BoxFerry prints the absolute path.

## Archive contents

| File          | Contents                                |
| ------------- | --------------------------------------- |
| `README.md`   | Review warning and bundle limitations   |
| `report.json` | Versioned, structured conversion result |

The default bundle excludes input documents, generated files, environment-file contents, runtime
inspection, the full process environment, hostnames, usernames, panic payloads, and backtraces.
Paths are replaced with invocation-local aliases.

Protected values, environment values, command arguments, metadata-label values, and config or
secret material are redacted as `<redacted>`. Additional credential-name and private-key filters
provide defense in depth.

## Podman diagnostic snapshots

For a Podman input route, explicitly add redacted acquisition evidence:

<!-- boxferry-example: podman-snapshot-error-report -->

```console
boxferry validate podman compose --podman-socket /run/user/1000/podman/podman.sock --podman-resource container=c-observer --loss-policy partial --generate-error-report --include-podman-snapshot --error-report-directory reports
```

`--include-podman-snapshot` requires `--generate-error-report`. It adds the redacted inventory,
discovery graph, and value-free acquisition findings. It omits environment values, protected
health commands, credentials, secret payloads and driver values, label values, unknown raw JSON,
and connection endpoints.

The snapshot is diagnostic serialization, not executable input, a replayable Podman inventory, or
a PodmanLens cassette. Resource names, image references, IDs, and topology can still be
operationally sensitive. The report's redaction count measures redaction markers and deliberately
omitted values across serialized snapshot entries; it is not a count of distinct source secrets.

## Before sharing

The bundle always declares `review_required: true`. Open every entry and remove anything you do
not want to publish. BoxFerry reduces exposure; it cannot guarantee that every context value is
safe to share.

Use `--report-file FILE` when automation needs the same JSON schema without a ZIP. JSON console,
report files, and bundles contain the same diagnostic and `fix_first` structures.
