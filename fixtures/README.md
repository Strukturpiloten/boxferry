# BoxFerry fixtures

Fixtures are stored as `fixtures/<suite>/<id>/`. Every fixture directory contains a `fixture.toml` manifest and all files listed by that manifest.

The common manifest contract is documented in [Fixture format](../docs/fixture-format.md). Executable repository-policy tests live with the application crate at [`crates/boxferry/tests/`](../crates/boxferry/tests/README.md); the root [`tests/`](../tests/README.md) directory owns cross-crate scenario organization.

Do not add credentials, unreviewed external content, or files with unclear redistribution rights.

The [`real-world/corpus.toml`](real-world/corpus.toml) catalogue is a separate pinned-remote test
contract. Its upstream Compose files are retrieved only by the opt-in test and are never treated as
vendored fixture content. See the [real-world corpus policy](../docs/real-world-compose-corpus.md).
