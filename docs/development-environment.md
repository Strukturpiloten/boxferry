# Development environment

The supported VS Code setup is the repository's Dev Container. Open the repository in VS Code
and run **Dev Containers: Reopen in Container**. The default container is unprivileged and is
suitable for parser, model, rendering, documentation, and repository-policy work.

## Sources of truth

- `rust-toolchain.toml` selects the current Rust toolchain and its components.
- `rust-version` in the workspace `Cargo.toml` declares the MSRV. CI reads it from Cargo
  metadata; the workflow does not duplicate it.
- `Cargo.toml` declares package versions. Release workflows read package versions from Cargo
  metadata rather than accepting a manually typed version.
- `.devcontainer/Dockerfile` records the exact base-image release and pins its digest.
- `.devcontainer/devcontainer-lock.json` pins resolved Dev Container feature versions and
  integrity hashes.
- `.devcontainer/Dockerfile` pins repository tooling; Renovate owns version proposals.

## Included tools

The image provides Rust, Cargo, rustfmt, Clippy, rust-analyzer, CodeLLDB, Git, GitHub CLI, Node.js,
Markdown and YAML support. The cached image layer installs exact versions of `cargo-deny`,
`cargo-semver-checks`, `lychee`, `zizmor`, `markdownlint-cli2`, and `actionlint`.
Actionlint's downloaded archive is verified against its release checksum.

Use the canonical checks documented in [the testing strategy](testing.md). Useful repository
checks include:

```console
cargo fmt --all -- --check
cargo ci-check
cargo ci-policy
cargo ci-clippy
cargo ci-test
cargo deny check
actionlint
zizmor .github/workflows
markdownlint-cli2 "**/*.md" "#target/**"
lychee --root-dir . --no-progress --exclude-path '(^|/)target/' "./**/*.md"
```

## Updating the container

Renovate proposes updates for the base image, feature references, Rust toolchain, and installed
tools. When a feature changes, regenerate the committed lock file with the pinned CLI version:

```console
npx --yes @devcontainers/cli@0.88.0 upgrade --workspace-folder .
```

Review the resolved version and integrity changes, rebuild the container, and run the repository
checks before merging.

## Runtime conformance

The default container deliberately does not mount a Docker or Podman socket, run systemd, or
request privileged mode. Runtime and Quadlet-generator conformance requires explicit, isolated
test environments with documented engine versions, privileges, and host requirements. Those
environments will be added with their test harnesses instead of expanding the trust boundary of
every editor session.
