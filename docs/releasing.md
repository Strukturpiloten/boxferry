# Release policy

BoxFerry is not publishable yet: every workspace package deliberately has `publish = false`.
The application and internal crates inherit one version from the workspace `Cargo.toml`.

Release automation will be added when the CLI has a useful end-to-end conversion, installation
artifacts are defined, and the support policy is ready. Internal crates remain private unless an
independent public API and semver commitment justify publication.

The future release pipeline must derive versions from Cargo metadata, use a protected `release`
environment, pin every third-party GitHub Action by exact version and commit, attest distributable
artifacts, and publish an immutable GitHub release. Registry or package-manager publication is a
separate decision for each deliverable.
