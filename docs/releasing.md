# Release policy

BoxFerry publishes eight crates in one lockstep version. The manual `Release` workflow validates
the default branch, creates an annotated tag and draft GitHub release, publishes through crates.io
trusted publishing, attaches attested crate archives and checksums, and then publishes the GitHub
release. It never uses a long-lived registry token.

## First-time setup

1. Create a protected GitHub environment named `release`; require review and restrict it to the
   default branch.
2. On crates.io, add a pending trusted publisher for each crate listed below. Use organization
   `Strukturpiloten`, repository `boxferry`, workflow `release.yml`, and environment `release`.
3. Merge the prepared release metadata, then run **Actions → Release → Run workflow** from the
   default branch. Do not create the tag manually.

Pending trusted publishers are required because 0.1.1 is the first crates.io version. Configure
all eight immediately before the release so none expire during setup.

## Publication order

1. `boxferry-model`
2. `boxferry-engine`
3. `boxferry-compose`
4. `boxferry-quadlet`
5. `boxferry-runtime`
6. `boxferry-docker`
7. `boxferry-podman`
8. `boxferry`

The workflow waits for each version to become visible in the registry before publishing a
dependent crate. A rerun skips versions already present on crates.io and resumes the same release;
it refuses tags or published GitHub releases that point elsewhere.

## Preparing later releases

- Update the workspace version and every internal dependency requirement together.
- Add a concise `CHANGELOG.md` entry and `docs/releases/<version>.md`.
- Run the canonical checks and review `cargo package --list` for every crate.
- Use a pre-1.0 minor version for intentional public breaks and include migration notes.

Repository-only tools remain unpublished. Binary bundles are separate future artifacts; the
0.1.1 workflow publishes the Rust crates and the installable `boxferry` binary they contain.
