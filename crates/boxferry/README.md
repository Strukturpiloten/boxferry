# BoxFerry

`boxferry` is the supported library facade and CLI for loss-aware conversion between container
application formats and runtimes.

The default features provide the generic `convert` command for the currently implemented
Compose-to-Quadlet and Quadlet-to-Compose routes. Embedded users can disable default features and
select only the adapters they need.

The Quadlet-to-Compose CLI route targets the rolling Compose Specification and requires an
explicit project name, not a provider or runtime version. Provider-aware Compose targets remain
available to embedded users through the public library API.

For a local, privacy-safe diagnostic archive, add the value-less `--generate-error-report` flag to
`convert` or `validate`. It creates an automatically named ZIP in the current directory, or in an
explicit existing directory selected by `--error-report-directory DIR`; see the repository's
[error-report contract](../../docs/error-reports.md).

```shell
cargo install boxferry
```

See the [BoxFerry repository](https://github.com/Strukturpiloten/boxferry) for usage, compatibility
boundaries, and documentation.
