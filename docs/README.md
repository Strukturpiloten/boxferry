# Documentation map

Start with the smallest document that answers the task. Detailed API facts belong in Rustdoc,
capability facts in machine-readable sources, behavior in tests, and completed history in ADRs or
the changelog.

## Use BoxFerry

- [Getting started](public/getting-started/)
- [Conversion guides](public/guides/)
- [CLI reference](public/reference/cli/)
- [Compatibility](public/reference/compatibility/)
- [Diagnostics and error reports](public/reference/)

These sources are assembled at [boxferry.dev/docs](https://boxferry.dev/docs/). Keep their existing
routes stable and register every displayed `boxferry` command in
[`documentation-examples.toml`](documentation-examples.toml).

## Develop BoxFerry

| Task                                       | Read first                                            |
| ------------------------------------------ | ----------------------------------------------------- |
| Change the model, engine, or an adapter    | [Architecture](architecture.md)                       |
| Change public Rust APIs                    | [API stability](api-stability.md) and Rustdoc         |
| Add or change tests and fixtures           | [Testing](testing.md) and [fixtures](../fixtures/)    |
| Change tools or dependencies               | [Dependency policy](dependency-policy.md)             |
| Prepare or publish a release               | [Releases](releasing.md)                              |
| Set up a checkout or submit a pull request | [Development environment](development-environment.md) |
| Change platform behavior                   | [Platform support](platform-support.md)               |

Repository-wide agent and Git rules live only in [`AGENTS.md`](../AGENTS.md).

## Understand decisions

[Architecture decisions](decisions/) explain constraints and their history. Read the index first,
then only the active or historical records relevant to the change. `CHANGELOG.md` owns completed
release history; GitHub issues own unfinished work.
