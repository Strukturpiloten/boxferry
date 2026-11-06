# Development environment

Use the Dev Container's pinned workspace tools. Keep sibling checkouts beside
`boxferry` and open `boxferry-lenses.code-workspace`. After mount changes, use
**Dev Containers: Rebuild and Reopen in Container**; window reload cannot add mounts.

## Build the CLI

Cargo uses its default workspace target directory:

```console
cargo build --release --locked --package boxferry
./target/release/boxferry --version
```

After toolchain updates use **Dev Containers: Rebuild Container**. Existing terminals
may need `unset CARGO_TARGET_DIR`; local validation targets must stay in this worktree.

## Shared development tools

The [locked Python project](../automation/pyproject.toml) owns uv, Ruff and ty.
Coordinate its selected automation revision with consumers. Renovate owns Dev Container
image version/digest pairs, features, Cargo tools and downloaded-tool inventory;
[development policy](dependency-policy.md) defines integrity and exceptions.
The [shared inventory](../automation/tool-artifacts.json) and
[installer](../scripts/install-file-tools.sh) supply file tools. Rebuild after upgrades.
Git comes from the pinned base; `gh` uses the locked feature.

The [Kubernetes installer](../scripts/install-kubernetes-tools.sh) supplies pinned kind,
kubectl, Helm and Kustomize. Its trusted checksum refresher verifies official metadata
and both architectures, then updates only eligible Renovate candidates. Required
integrity checks read exact tested Git blobs; the complete local gate checks the working
tree. [Provisioning](https://github.com/Strukturpiloten/boxferry/issues/445) and
[ARM64 evidence](https://github.com/Strukturpiloten/boxferry/issues/446) remain separate.

## Rust components

`rust-toolchain.toml` selects Rust, Clippy, rustfmt and LLVM coverage tools. If needed:

```console
rustup component add llvm-tools-preview
bash .devcontainer/verify-tools.sh
```

## Dev Container feature updates

The [canonical locked generator](../automation/devcontainer/) uses
`npm ci --ignore-scripts`. Hosted proposals remain held: each consumer needs explicit
seed/adoption and successful native image/tool qualification before automatic activation.
See [ADR 0094](decisions/0094-official-devcontainer-feature-updates.md).

## Local verification

Follow [Podman-only runtime policy](decisions/0079-native-test-runtime-selection.md).
For cleanup without tests:

```console
./scripts/format-lint.sh --fix
```

The matching VS Code task performs no validation. Clippy defaults to two jobs;
`BOXFERRY_LINT_JOBS=1` reduces contention. `--check` avoids source formatting.

`python3 scripts/validation-plan.py run-local` selects development feedback;
`plan --event local` previews it. Known impacts select their union; policy, classifier,
unknown and unsafe changes remain complete. [Rollout #549](https://github.com/Strukturpiloten/boxferry/issues/549)
still requires per-repository acceptance. Focused feedback never replaces:

```console
./scripts/check-all.sh
```

The default gate formats; `--check` runs without source formatting. Any edit invalidates
its result. Preserve [API baseline evidence](decisions/0096-development-and-published-api-baselines.md)
and `target/api-baseline-local.json`.

## Explicit local rootful qualification

`devcontainer-local` defaults to rootless Podman. Explicit `--outer-mode rootful`
uses a fresh private store inside one PID/mount lifetime. Source/npm/uv preparation
and reports remain under the original nonroot user; invoking the whole helper with
sudo is rejected. The local profile selects cgroupfs, file events and installed
`/usr/bin/runc`, with observed versions/checksums and no installation or fallback.

Only proved namespace closure and unchanged ownership permit finite directory
cleanup. Uncertain runs retain the affected resources and report their custody.
Hosted/rootless behavior and qualified inner targets remain separate. See
[#578](https://github.com/Strukturpiloten/boxferry/issues/578) for local rootful
qualification status and [#563](https://github.com/Strukturpiloten/boxferry/issues/563)
for ARM qualification status.

## Issue-to-PR contribution workflow

The fail-closed `Required CI` check must pass before merge. See [AGENTS.md](../AGENTS.md)
for the complete agent Git and GitHub workflow: agents must complete the local gate before
committing, pushing, or opening a PR. Independent verifiers do not edit source; the primary
owns the final gate and standing-authorized merges.

## GitHub authentication and agents

Use the persistent Dev Container `gh` volume and workspace authentication task; never put
tokens into issues, command arguments or Git configuration. Role definitions live in
[`.codex/`](../.codex/); reload a trusted session after configuration changes. Ordinary
shell-runner tests require Linux; model configuration checks remain platform-independent.
