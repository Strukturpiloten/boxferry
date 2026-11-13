# Development environment

Clone BoxFerry and use VS Code's **Dev Containers: Reopen in Container** with
its Dev Containers extension configured for your existing Podman connection,
preferably rootless. Use **Dev Containers: Rebuild Container** after changing
container tools. Rebuilds use normal Podman storage and caches; no separate
qualification command, private graphroot or mandatory host sudo is required.

BoxFerry's Cargo dependencies use published Lens crates. For optional multi-repository
work, keep sibling checkouts beside `boxferry`, configure their mounts explicitly,
and open `boxferry-lenses.code-workspace`.
After mount changes, use **Dev Containers: Rebuild and Reopen in Container**;
window reload cannot add mounts.

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

The installer exposes `tools --git-source` for a future canonical Git source
record. It requires a new prefix (not existing `/usr/local`), the shared Python
package, native build prerequisites, Perl, OpenSSL and base-owned CA trust at
`/etc/ssl/certs/ca-certificates.crt`; caller trust variables cannot select this bundle.
It verifies the archive before bounded extraction/build, validates the staged complete
runtime, and retains COPYING and source provenance. No Git source record or image
invocation is selected. C builds omit Rust-only compatibility hashing, GUI, translations and generated docs.
Builds use two make jobs, wall/child CPU/memory/file/output limits and owned-child
cleanup. This is no process sandbox: Linux UID quotas count unrelated work and may
not apply to root builds. Native builds and worktree/mount acceptance remain under #453.

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
`npm ci --ignore-scripts`. Feature proposals remain held under
[#563](https://github.com/Strukturpiloten/boxferry/issues/563) until CI image/tool,
VS Code reopen/rebuild and restricted-App/ordinary-CI qualification succeeds.
Ordinary shared-tooling adoption enables no image updater or schedule.

## Local verification

Follow [Podman-only runtime policy](decisions/0079-native-test-runtime-selection.md).
For cleanup without tests:

```console
./scripts/format-lint.sh --fix
```

The matching VS Code task performs no validation. Clippy defaults to two jobs;
`BOXFERRY_LINT_JOBS=1` reduces contention. `--check` avoids source formatting.

`python3 scripts/shared-ci.py run-local` selects development feedback;
`python3 scripts/shared-ci.py plan --event local` previews it. Known impacts select their union; policy, classifier,
unknown and unsafe changes remain complete. [Rollout #549](https://github.com/Strukturpiloten/boxferry/issues/549)
still requires per-repository acceptance. Focused feedback never replaces:

```console
./scripts/check-all.sh
```

The default gate formats; `--check` runs without source formatting. Any edit invalidates
its result. Preserve [API baseline evidence](decisions/0096-development-and-published-api-baselines.md)
and `target/api-baseline-local.json`.

## CI image qualification

The shared producer builds held candidates and runs safe tools as the configured
non-root user on fresh GitHub-hosted Linux x64 and ARM64 runners. It uses the
reviewed explicit Docker or rootful Podman backend with standard runner storage,
checks the actual store's filesystem against its disk budget, and cleans only
its exact owned containers and image tag. Runner disposal handles residual caches.
BoxFerry's CI workspace probes retain all seven real checkout revisions; ordinary
local setup does not require those checkouts or add an image build to the local gate.
The caller requires both native lanes while seeding the independently approved prior policy,
then permits only trusted planned skips. Required CI checks the current tested candidate receipt.
The image retains its selected shared-tool metadata for non-root version checks; rebuild it after
changing the shared automation manifest. Updater schedules remain held pending native and App evidence.

Image/updater activation still requires actual per-owner native image evidence,
VS Code reopen/rebuild acceptance and restricted-App/ordinary-CI qualification under
[#563](https://github.com/Strukturpiloten/boxferry/issues/563). Source admission is
not that evidence. [#578](https://github.com/Strukturpiloten/boxferry/issues/578)
supersedes the earlier local private-store/PID controls. Historical failed stores remain untouched.

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
