# BoxFerry

`boxferry` is the supported library facade and CLI for loss-aware conversion between container
application definition formats.

The CLI supports Linux. Windows users must install and run it inside WSL2; native Windows binaries
and Windows containers are unsupported. See the repository's platform-support documentation.

The default features provide nested `convert <INPUT_TYPE> <OUTPUT_TYPE>` and
`validate <INPUT_TYPE> <OUTPUT_TYPE>` commands for all four Compose/Quadlet document routes.
Embedded users can disable default features and select only the adapters they need.

Every route uses the public importer, neutral `Application`, exporter, and loss-policy pipeline.
Same-format conversion is not passthrough; unresolved or native-only intent follows the same
diagnostic and authorization contract as cross-format conversion.

Compose output targets the rolling Compose Specification. Quadlet input requires an explicit
application name, not a provider or runtime version. Provider-aware Compose targets remain
available to embedded users through the public library API.

For a local, privacy-safe diagnostic archive, add the value-less `--generate-error-report` flag to
`convert` or `validate`. It creates an automatically named ZIP in the current directory, or in an
explicit existing directory selected by `--error-report-directory DIR`; see the repository's
[error-report contract](../../docs/public/reference/error-reports/).

```shell
cargo install boxferry
```

See the [BoxFerry repository](https://github.com/Strukturpiloten/boxferry) for usage, compatibility
boundaries, and documentation.
