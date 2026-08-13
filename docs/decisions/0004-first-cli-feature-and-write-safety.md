# ADR 0004: first CLI feature and write safety

- Status: accepted
- Date: 2026-08-03
- Amended by: [ADR 0025](0025-empty-output-directories-and-final-human-status.md)

## Context

BoxFerry's tested Compose-to-Quadlet conversion was available only through Rust APIs. The package
is also intended to provide an installable command, while embedded consumers must retain a small
dependency option and the CLI must not develop private conversion behavior.

Conversion output can contain credentials and operational configuration. Guessing profiles,
reading the process environment, overwriting an existing directory, or applying generated units
would make the first command unsafe for routine migration work.

## Decision

1. The default feature set is `cli`, `compose`, and `quadlet`, making a normal future
   `cargo install boxferry` useful. Embedded consumers can use `default-features = false` and
   select only the required adapters.
2. Clap is an optional dependency owned by `cli`. The binary declares `cli`, `compose`, and
   `quadlet` as required features; component and no-default builds do not require the parser.
3. The first command calls the public Compose importer, conversion engine, and Quadlet exporter.
   It owns only argument parsing, explicit source reads, diagnostic presentation, output writes,
   and exit status.
4. Compose file order, project name, profiles, target range, grouping, loss authorization, and
   output location are caller-visible inputs. Conservative defaults remain documented.
5. The command does not read process environment variables or invoke Docker, Podman, systemd, or
   another renderer. Environment interpolation requires a future explicit input contract.
6. Conversion and policy authorization finish before writes. The output directory is created
   atomically, must not exist, and receives create-new files. A partial write failure removes only
   files created by that invocation and removes the directory only when empty.
7. The conversion command does not install, enable, start, or otherwise apply generated output.

## Consequences

- The installed command is useful with default Cargo behavior, while library consumers retain a
  minimal core build.
- Existing output cannot be silently replaced; users choose a new location or remove/move an old
  one explicitly.
- Host environment values cannot accidentally become generated secrets.
- Multi-file output has a deterministic directory contract and black-box regression coverage.
- Adding JSON reports, explicit environment sources, overwrite behavior, or apply/deploy commands
  requires separate reviewed contracts.

## Alternatives considered

Keeping all default features disabled was rejected because a normal installed package would not
provide its intended conversion command. A separate CLI package remains possible if binary-only
dependencies or release cadence become costly, but is unnecessary for the first release. An
overwrite flag was deferred because atomic replacement, rollback, permissions, symbolic links,
and user-authored files need a complete design rather than a convenience switch.
