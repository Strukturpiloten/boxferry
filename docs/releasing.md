# Release policy

BoxFerry publishes eight crates in one lockstep version. The manual `Release` workflow validates
the default branch, creates an annotated tag and draft GitHub release, publishes through crates.io
trusted publishing, attaches attested crate archives and checksums, and then publishes the GitHub
release. The initial 0.1.1 publication uses one temporary crates.io token because trusted
publishers cannot be attached before the crate names exist.

## First-time setup

1. Create a protected GitHub environment named `release`; require review and restrict it to the
   default branch.
2. Create a short-lived crates.io API token permitted to publish new crates.
3. Add it to the GitHub `release` environment as the secret `CRATES_IO_BOOTSTRAP_TOKEN`.
4. Merge the prepared release metadata, then run **Actions → Release → Run workflow** from the
   default branch. Do not create the tag manually.
5. After all eight crates are published, revoke the crates.io token and delete the GitHub secret.
6. Add a trusted publisher to every crate using organization `Strukturpiloten`, repository
   `boxferry`, workflow `release.yml`, and environment `release`.

The workflow maps the bootstrap secret to Cargo's `CARGO_REGISTRY_TOKEN` only in publication
steps and rejects it for every version other than 0.1.1. Later releases require the trusted
publisher configuration and do not use a stored crates.io token.

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
