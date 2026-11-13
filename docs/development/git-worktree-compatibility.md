# Git worktree compatibility

The Dev Container selects Git through `automation/tool-artifacts.json`, using the
admitted canonical installer and a complete `/opt/boxferry-git` prefix. Both PATH
Git and `/usr/bin/git` use it, including isolated validation subprocesses. Helpers,
templates, Perl modules, base-owned HTTPS CA trust, COPYING and corresponding
source provenance stay together. The bounded C build disables optional Git Rust
components and compatibility-hash support; ordinary SHA-1/SHA-256 repositories and
relative worktrees remain supported. No second installer or version inventory is used.

The local `custom.kernel-git` Renovate owner pairs official stable `.tar.xz`
checksum rows with versions and replaces version, filename, URL and SHA256 together.
It retains canonical downloaded-tool grouping and existing approvals. The checksum
feed supplies no trustworthy publication date or PGP-verification claim; only this
owner uses timestamp-optional lookup. Trusted canonical checksum matching and the
unchanged 72-hour first-seen hold remain mandatory. A release date or local download
cannot backdate that observation. Binary profiles retain their owners and integrity.
The image invokes only the admitted installer with the explicit candidate inventory.

A recent executable alone cannot expose missing repository metadata. For a normal
primary checkout or standalone clone, mount its own `.git` directory with the
source. Host-created relative worktrees also require their common Git directory
at a container path preserving relative topology. The pinned official Dev
Container CLI supports:

```console
devcontainer up --docker-path podman --workspace-folder /path/to/worktree --mount-git-worktree-common-dir true
```

In VS Code, enable the **application** setting
`dev.containers.experimentalMountGitWorktreeCommonDir`, then rebuild/reopen with
your existing Podman connection and storage. A repository workspace setting cannot
enable this application-scoped option. The configuration deliberately leaves
`workspaceMount` and `workspaceFolder` unset so official common-directory mount
detection can operate. No private store or mandatory host sudo is needed.

Startup runs `scripts/git-worktree.py inspect --source CHECKOUT`. When the base's
system Python predates 3.13, the caller uses the image's retained locked Python,
without downloading or bootstrapping during startup. It rejects Git
routing overrides, checks the explicit root and index, and records HEAD, canonical
staged entries and working-file bytes/modes, including staged, unstaged, deleted
and untracked files. It never rewrites repository extensions or metadata, injects
`GIT_DIR`, or adds safe-directory trust. Resolve ownership for the exact intended
checkout instead of granting wildcard trust.

Legacy absolute host paths may be inaccessible after relocation. Run this explicit
fallback **on the host where the original metadata resolves**, choosing a new
outside path owned by this task:

```console
python3 scripts/git-worktree.py standalone --source /path/to/worktree --destination /path/to/new-task-clone
```

Only after success, explicitly open that clone for container validation. The helper
creates a genuine standalone clone without shared objects, copies reviewed working
bytes and modes, reconstructs the staged index, and checks source/clone identities.
It refuses existing destinations, conflicted or sparse/assume-unchanged indexes,
submodules and unsupported file layouts. It removes its own failed clone without
modifying the source. Ignored build caches and credentials are not copied. Only an existing canonical BoxFerry origin and its exact fetched `origin/main`
identity are carried into the clone; credentials/configuration are not copied.
Published API/release validation still selects its proper trusted baseline under ADR 0096.

The new host-created mount qualifier is producer code. Operational callers must
first use an admitted producer containing it; preparing that producer and building
its image cannot prove the new hosted contract. After producer CI and shared-source
distribution succeed, a separate coherent source-pin adoption must obtain fresh
native receipts. Neither stage substitutes candidate host code for admitted code.

Tests use task-owned real repositories. Image qualification separately builds and
checks both native architectures; local CLI execution proves only the architecture
and mounts actually exercised. Neither installer tests nor a CLI run claim VS Code
UI acceptance, other image owners' adoption, or updater qualification. Preserve
remaining evidence under #453, #446 and #563; updater schedules stay held.
