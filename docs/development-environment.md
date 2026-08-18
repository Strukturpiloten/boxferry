# Development environment

The supported VS Code setup is the repository's Linux Dev Container. It is unprivileged and does
not mount Docker or Podman sockets.

## Prerequisites

- Install VS Code and the **Dev Containers** extension.
- Start a Docker-compatible Linux container engine and verify `docker info` succeeds.
- Ensure `docker buildx version` succeeds; the image build uses BuildKit mounts.
- Allow outbound HTTPS access to Docker Hub, Microsoft Container Registry, GHCR, crates.io, npm,
  and GitHub Releases during the first image build.

On Windows, install WSL2 and enable Docker Desktop's WSL integration and Linux-container mode. Keep
the checkout in the WSL filesystem, open a WSL terminal in the repository, and run `code .`. Do not
open the checkout from a native Windows VS Code window or run Docker Desktop in Windows-container
mode. BoxFerry does not support native Windows development or execution.

## Start in VS Code

For the normal BoxFerry-only view:

1. Open the BoxFerry repository in a Linux or WSL2 VS Code window.
2. Run **Dev Containers: Reopen in Container**.
3. Wait for `BoxFerry Dev Container tooling is ready.` in the creation log.

For the supported three-repository view, do not create an untitled multi-root workspace with
**Add Folder to Workspace** and do not open the `.code-workspace` file in a local window first.
Enter the container through the BoxFerry folder:

```console
code .
```

Run **Dev Containers: Reopen in Container**. After the BoxFerry folder has opened in the container,
run **File: Open Workspace from File...** and select
`/workspaces/boxferry/boxferry-lenses.code-workspace`. BoxFerry is the primary configuration and
mounts the host's `../compose-lens` and `../quadlet-lens` checkouts below
`/workspaces/boxferry/.boxferry-workspace` in the container. The committed workspace refers only to
BoxFerry and these container-side subdirectories, as required by VS Code for a multi-root workspace
in one Dev Container. All three paths remain host bind mounts, so edits persist in their respective
repositories. The sibling repositories must use exactly those directory names and share
BoxFerry's parent.

After connection, the lower-left remote indicator must show **Dev Container: BoxFerry**. A new
terminal must report `/workspaces/boxferry` for `pwd`; if it reports a host path, the window is still
local. Close that window, open the BoxFerry folder with `code .`, and repeat **Dev Containers: Reopen
in Container** before opening the multi-root workspace.

The lifecycle check is offline and only verifies the installed tools. Cargo downloads the locked
Rust dependencies when the first build or check needs them, so a registry outage does not prevent
an already-built editor container from starting.

Container builds use persistent named volumes at `/workspaces/.boxferry-cargo`,
`/workspaces/.boxferry-target`, and `/workspaces/.boxferry-gh` through `CARGO_HOME`,
`CARGO_TARGET_DIR`, and `GH_CONFIG_DIR`. The first gives Cargo a writable registry and package
cache. The second keeps the host checkout's `target/` separate, preventing binaries built against
the host's C library from being reused inside the Debian container. The third isolates GitHub CLI
configuration and authentication from the host and from unrelated containers. `${devcontainerId}`
makes each volume specific to this Dev Container while keeping it stable across rebuilds.

The GitHub CLI is installed by the pinned Dev Container feature. Authenticate only after rebuilding
the container:

```shell
./scripts/configure-github-cli.sh
```

Alternatively, run the **BoxFerry: Configure GitHub CLI authentication** VS Code task. The script
prompts without echoing the token, never accepts it as a command-line argument, verifies the stored
authentication, and leaves the existing multi-account Git credential helpers unchanged. Dev
Containers normally have no desktop credential store, so the script uses `--insecure-storage` to
write the token to `$GH_CONFIG_DIR/hosts.yml`. The lifecycle check restricts the configuration
directory to the container user. The token is not committed, not stored in the host GitHub CLI
configuration, and not shared with unrelated Dev Containers. Anyone with sufficient access to the
local container engine or named volume can still read it. Removing the `boxferry-gh-*` volume
deletes the persisted authentication.

For the issue, branch, workflow-file, and pull-request operations used in this workspace, select the
three Strukturpiloten repositories and grant the fine-grained token these repository permissions:

- **Contents: Read and write**
- **Issues: Read and write**
- **Pull requests: Read and write**
- **Workflows: Read and write**

Optional **Actions: Read-only** and **Commit statuses: Read-only** permissions let the CLI inspect
workflow runs and status checks. Metadata read access is granted automatically.

The committed `.vscode/settings.json` enables format-on-save, excludes Cargo build output from file
watching and search, activates all Cargo features in rust-analyzer, and uses Clippy for live Rust
diagnostics. `.vscode/extensions.json` recommends the shared editor toolset for checkouts opened
without the container and the host-side Dev Containers extension. Use **Tasks: Run Task** for
individual checks or **BoxFerry: Required Rust checks**; the default build and test tasks compile
the workspace and run the complete test suite. The **BoxFerry: CLI help** Run/Debug configuration
builds and starts the current CLI under CodeLLDB.

## Extension policy

Keep editor extensions in the narrowest applicable scope:

- `.devcontainer/devcontainer.json` installs the account-neutral project baseline automatically
  inside the container. Add an extension there only when the shared development experience depends
  on it.
- `.vscode/extensions.json` contains optional project recommendations. VS Code prompts contributors
  and provides **Extensions: Show Recommended Extensions**; it does not silently install the list.
- VS Code User Settings contain personal, account-bound, AI-assisted, or otherwise preference-based
  extensions. Do not add these extensions to either repository-controlled list.

For example, a developer who wants Codex available in every Dev Container window can open
**Preferences: Open User Settings (JSON)** in the local VS Code window and add the official
extension ID to their personal settings:

```json
{
  "dev.containers.defaultExtensions": ["openai.chatgpt"]
}
```

Preserve any existing entries in that array. Reopen or rebuild the container after changing the
setting. To use a personal extension only in the current container, install it from the Extensions
view with the Dev Container selected as the installation target instead.

The setting above is the container-native option: Codex runs as a workspace extension and reads the
container user's configuration. Codex reads personal agent configuration from
`~/.codex/config.toml` in the environment where its extension process runs.

To use the live host configuration instead, remove `openai.chatgpt` from
`dev.containers.defaultExtensions` and add this personal VS Code setting:

```json
{
  "remote.extensionKind": {
    "openai.chatgpt": ["ui"]
  }
}
```

This deliberately runs the Codex extension in VS Code's local UI extension host; it does not move
the host configuration into the container. Omit the override when Codex itself must execute as a
container workspace extension. Do not bind-mount the complete host `.codex` directory: it includes
authentication and mutable session databases in addition to configuration. BoxFerry's committed
`.codex/config.toml` and `.codex/agents/` remain the trusted project-level configuration and are
available through the workspace mount.

The personal setting is not committed and therefore does not affect other contributors. It also
works for `boxferry-lenses.code-workspace`: BoxFerry still owns one container, and the extension can
see all three mounted repository folders. VS Code documents the setting under
[Dev Container tips and tricks](https://code.visualstudio.com/docs/devcontainers/tips-and-tricks),
and distinguishes it from
[workspace recommendations](https://code.visualstudio.com/docs/configure/extensions/extension-marketplace#_workspace-recommended-extensions).
The current official Codex setup is documented in the
[Codex IDE extension guide](https://learn.chatgpt.com/docs/codex/ide) and
[Codex configuration guide](https://learn.chatgpt.com/docs/config-file/config-basic). VS Code
documents `remote.extensionKind` as an execution-location override and warns that incompatible
extensions can malfunction when forced to another extension host.

If startup fails, run **Dev Containers: Show Container Log** before rebuilding. The pinned CLI can
separate configuration, image-build, and container-start failures:

```console
npx --yes @devcontainers/cli@0.88.0 read-configuration --workspace-folder . --include-merged-configuration
npx --yes @devcontainers/cli@0.88.0 build --workspace-folder . --frozen-lockfile
npx --yes @devcontainers/cli@0.88.0 up --workspace-folder . --frozen-lockfile
```

Common causes are a stopped container engine, Windows-container mode, missing Docker Desktop WSL
integration, an old engine without BuildKit/buildx, a proxy blocking one of the required registries,
or insufficient disk space. After changing the image, run **Dev Containers: Rebuild Container
Without Cache**.

## Run all local checks

Run the complete local formatter, linter, and deterministic test suite from the BoxFerry checkout
root:

```console
./scripts/check-all.sh
```

The script formats Rust; formats Markdown with Prettier before applying and checking Markdownlint
rules; formats and parses JSON, JSONC, YAML, and TOML; formats and lints shell scripts; and lints
Dockerfiles before checking the resulting tree. It
then runs the complete feature-boundary, policy, Clippy, unit, integration, black-box CLI, doctest,
documentation, coverage, MSRV, dependency, workflow, local-link, and published-API validation. It
derives the MSRV from Cargo metadata and installs that Rust toolchain on first use.

Offline Tombi runs select the repository's structural Cargo-manifest schema instead of Tombi
1.4.0's embedded Cargo schema, whose lint subsections reference two remote-only schemas. Cargo
metadata, checks, Clippy, tests, and packaging remain the authoritative semantic manifest
validation. This keeps an empty CI cache equivalent to an established local cache without
disabling schema discovery for other TOML files.
Advisory-database and published-API checks require outbound network access. Local documentation
links are checked without network access. Every command and its normal output remain visible;
execution stops at the first failing step with its name.

The complete script keeps coverage and API-compatibility artifacts below
`$CARGO_TARGET_DIR/check-all/boxferry`. ComposeLens and QuadletLens use their own sibling
namespaces. This prevents one repository's coverage cleanup or SemVer build from deleting or
reusing another repository's files when the three VS Code tasks share the Dev Container target
volume. The SemVer check also uses an isolated Cargo home, so it never reuses a read-only global
package lock.

File discovery uses `git ls-files --cached --others --exclude-standard` and passes literal paths to
the tools. Do not replace it with a recursive `**/*.md` or similar workspace glob: the Dev
Container mounts ComposeLens and QuadletLens below the BoxFerry checkout, and recursive globs can
cross repository ownership boundaries or enter generated trees. Run only the non-Rust layer with
`./scripts/check-files.sh --fix`; CI and release validation use its non-mutating `--check` mode.

In VS Code, run **Tasks: Run Test Task** or **BoxFerry: Format, lint, and test all** for the same
command. In `boxferry-lenses.code-workspace`, use **Workspace: Format, lint, and test all
repositories** to run BoxFerry, ComposeLens, and QuadletLens sequentially. The repository-specific
temporary directories also make separately started full-check tasks safe, while the sequential
workspace task avoids unnecessary CPU and memory contention. The narrower commands remain useful
when iterating on one failure:

```console
cargo fmt --all -- --check
./scripts/check-files.sh --check
cargo ci-check
cargo ci-core
cargo ci-compose
cargo ci-docker
cargo ci-quadlet
cargo ci-podman
cargo ci-runtime
cargo ci-policy
cargo ci-clippy
cargo ci-test
cargo ci-doctest
RUSTDOCFLAGS="-D warnings" cargo ci-doc
cargo llvm-cov clean --locked
cargo llvm-cov --locked --no-clean --workspace --all-features --all-targets --summary-only \
  --fail-under-regions 82 --fail-under-functions 87 --fail-under-lines 82
cargo +1.85.0 ci-check
cargo +1.85.0 ci-policy
cargo deny --all-features check
actionlint
zizmor .github/workflows
markdownlint-cli2 "**/*.md" "#target/**" "#.boxferry-workspace/**"
lychee --config lychee.toml --root-dir . --offline './**/*.md'
cargo semver-checks check-release --workspace --all-features
```

## Issue-to-PR contribution workflow

For an issue-backed change, inspect the complete working-tree diff first and preserve unrelated
work. Search for a duplicate issue, create the issue, synchronize local `main` with `origin/main`,
and create `TheRealBecks/issue<NUMBER>`. After implementation and final diff review, run:

```console
./scripts/check-all.sh
```

All steps must pass before the change is committed, pushed, or submitted as a pull request. If any
source, test, configuration, or documentation file changes after that successful run, run the
complete task again. Then stage only explicit in-scope paths, check the staged diff, commit, push,
and open a ready-for-review pull request containing `Closes #<NUMBER>`. Read the created pull
request back from GitHub to verify its base branch, head branch, issue linkage, draft state, and
check status.

The primary Sol agent uses high reasoning effort and owns the issue, branch, final integration,
complete local gate, staging, commit, push, pull request, and GitHub readback. Terra agents may
perform bounded implementation, research, read-only review, or non-mutating verification, but they
never perform Git or GitHub writes. The formatting `./scripts/check-all.sh` task therefore remains
Sol's final responsibility.

The all-checks script intentionally excludes the macOS portability job, privileged Docker/Podman
conformance, installed-current Podman mutation, network-fetched real-world corpus, and external
documentation-link health. Those tiers cannot run safely or reproducibly in the normal
unprivileged Linux Dev Container. CI runs the macOS job. A weekly/manual workflow checks external
links with a fourteen-day success cache, two concurrent requests per host, and a delay between
requests; transient remote outages therefore do not block local work or pull requests. The
explicitly opt-in tiers and their prerequisites are in [the testing strategy](testing.md).

## Compile and test the CLI

Compile the complete workspace, then run the CLI through Cargo while developing:

```console
cargo build --locked --workspace --all-features
cargo run --locked --bin boxferry -- version
```

`cargo run` is the recommended checkout-local command because Cargo checks whether the executable
matches the current sources and dependencies before starting it. Use the optimized profile when
testing release behavior:

```console
cargo run --locked --release --bin boxferry -- version
```

Invoking `./target/release/boxferry` directly does not perform that freshness check. If a script or
manual test needs the path itself, rebuild it immediately beforehand and rebuild it again after
changing source code, features, or dependencies:

```console
cargo build --locked --release --bin boxferry
./target/release/boxferry version
```

Use the repository fixture for a non-privileged Compose-to-Quadlet smoke test:

```console
smoke_root="$(mktemp -d)"
cargo run --locked --release --bin boxferry -- convert compose quadlet \
  --input-file fixtures/conversion/compose-to-quadlet-dependencies/compose.yaml \
  --output-directory "${smoke_root}/quadlet-output"
find "${smoke_root}/quadlet-output" -maxdepth 1 -type f -print | sort
rm -r "${smoke_root}"
```

This exercises the real CLI and public conversion facade but does not start containers or install
the generated units. See the [CLI contract](cli.md) for all four Compose/Quadlet routes and other
options.

## Included tools and sources of truth

The image includes the pinned Rust toolchain, rustfmt, Clippy, rust-analyzer, CodeLLDB, Git, GitHub
CLI, Node.js, `cargo-deny`, `cargo-llvm-cov`, `cargo-semver-checks`, `lychee`, `zizmor`,
`markdownlint-cli2`, and `actionlint`. Actionlint's archive is checksum-verified.

- `rust-toolchain.toml` selects the current Rust toolchain and components.
- Workspace `rust-version` declares the MSRV.
- `.devcontainer/Dockerfile` pins base-image digests and tool versions.
- `.devcontainer/devcontainer-lock.json` pins Dev Container features and integrity hashes.
- `Cargo.toml` and `Cargo.lock` define the Rust dependency graph.
- `package.json` and `package-lock.json` define the locked repository-only Markdownlint and Prettier
  tool graph; they are not runtime or crate dependencies.

Renovate proposes tool and image updates. When a Dev Container feature changes, regenerate its lock
with the pinned CLI, review the resolved digest, and rebuild:

```console
npx --yes @devcontainers/cli@0.88.0 upgrade --workspace-folder .
```

## Runtime conformance

The default container deliberately does not mount a Docker or Podman socket, run systemd, or request
privileged mode. Runtime and Quadlet-generator conformance belongs in explicit isolated test
environments, not in every editor session.
