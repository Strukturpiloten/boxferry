# Release policy

BoxFerry is not publishable yet: every workspace package deliberately has `publish = false`.
The supported facade, model, engine, and future adapter crates inherit one lockstep version from
the workspace `Cargo.toml`.

Library publication is not blocked on the CLI becoming feature-complete. It is blocked on the
corresponding crate having a useful public API, documentation, tests, package metadata, package
content review, and compliance with the [pre-1.0 compatibility policy](api-stability.md). The
intended publication order is:

1. `boxferry-model` and `boxferry-engine` after T4.
2. `boxferry-compose` and `boxferry-quadlet` with the supported T5/T6 mappings.
3. The `boxferry` facade and executable with the first useful end-to-end conversion.

Workspace dependencies use both a local `path` and the matching released `version`, so packaged
crates resolve through crates.io. Publication automation must respect dependency order and verify
each packaged crate independently.

Each published crate needs its own crates.io ownership establishment and trusted-publisher
configuration. Long-lived registry tokens must not remain in GitHub after that bootstrap. Binary
release automation is a distinct pipeline: it must derive versions from Cargo metadata, use a
protected `release` environment, pin every third-party GitHub Action by exact version and commit,
attest platform artifacts, publish checksums, and create an immutable GitHub release.
