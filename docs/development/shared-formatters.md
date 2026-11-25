# Shared formatters

BoxFerry's root [npm manifest](../../package.json) and [lock](../../package-lock.json)
own Prettier and markdownlint-cli2 versions and registry integrity. Consumers retain their
formatting rules, file selection and other native checks. [Dependency policy](../dependency-policy.md)
owns update requirements; this guide describes invocation and adoption.

## BoxFerry checks and the producer runner

From the BoxFerry repository root:

```sh
npm ci --ignore-scripts
bash scripts/check-files.sh --check
bash scripts/check-files.sh --fix
```

The file runner selects [scripts/run-node-tool.sh](../../scripts/run-node-tool.sh), passes
explicit caller configuration, and preserves its Git-selected files and reviewed fixture
exclusions. Fix mode formats Markdown, applies linter fixes, then lints and checks it.
Other file formats retain their own existing checks.

The producer runner's interface is:

```sh
bash scripts/run-node-tool.sh <installation-root> <prettier|markdownlint-cli2> [arguments...]
```

It requires an existing installation and downloads nothing. It preserves cwd, argv, stdin,
stdout and tool status, selecting absolute package entry points rather than formatter
executables on PATH. It refuses manifests/locks that differ from its producer checkout or
installed package/bin identities that disagree with that lock. Markdown also checks the
library selected by npm resolution, including nested dependencies, before a check or fix.

BoxFerry explicitly selects its image installation with
`BOXFERRY_NODE_TOOLS_ROOT=/opt/boxferry-file-tools`; ordinary contributor checks use the root npm
installation. Image verification compares the checkout npm pair and installed versions with
the retained graph. Rebuild after changing it.

Run `bash scripts/test-node-tools.sh` after installation for actual check/fix, configuration,
ignore, nested-parent, PATH and failure-return contracts. This offline regression copies the
installed packages into a temporary graph, confirms stale CLI/library metadata prevents
edits, restores metadata and proves the same files can be fixed and checked. Complete gates
and hosted/native image qualification remain separate acceptance requirements.

An existing full immutable producer checkout can supply the same runner and npm pair to a
dedicated installation. Authenticate its checkout against the consumer's admitted revision
and install with `npm ci --ignore-scripts --prefix <installation-root>` before calling its
runner from the consumer cwd. The fresh-checkout route below centralizes this acquisition
without needing a sibling checkout.

## Fresh Lens checkouts

After admitting a producer revision containing the formatter runner and this interface,
Lens callers can use the same command locally, in hosted CI, and while building a container:

```sh
python3 scripts/shared-ci.py node-tool --cwd "$PWD" prettier --config .prettierrc.json --ignore-path .prettierignore --no-editorconfig --check README.md
python3 scripts/shared-ci.py node-tool --cwd "$PWD" markdownlint-cli2 --config .markdownlint.json :README.md
```

The caller owns cwd, rules, ignores, literal filenames and check/fix arguments. The sole
revision is `.github/automation.json`'s exact SHA. No sibling checkout, independent Node
revision, PATH formatter or consumer downloader is used. The verified standard-library
`run_project` routes `node-tool` before uv acquisition or Python environment setup. The
existing verified bare Git cache supplies exactly `package.json`, `package-lock.json` and
`scripts/run-node-tool.sh` into a separate `target/boxferry-node-tools/<revision>` installation;
the closed Python export retains its existing inventory. Archives reject missing, duplicate,
nonregular, unexpected and excessive members. A missing runner in an older admitted revision
is an error, never a fallback to another graph.

Every writable managed call uses
`npm ci --ignore-scripts --include=dev --no-audit --no-fund --prefer-offline` in a fresh temporary
installation. The producer lock owns registry integrity; npm verifies extracted assets from
its content cache, using the network only for missing assets rather than revalidating cache
hits. Both installed formatter identities are verified with the producer runner before
publishing the complete installation. Writable installed files and their co-located inventory
are never independent execution authority, even when they agree. Every non-root npm lock record
must declare a registry.npmjs.org HTTPS
asset and SHA512 integrity; npm verifies the artifact content and coherent installation graph.
Failed npm setup preserves the previous root and its failure status. Setup messages go to
stderr, with setup stdin closed;
the selected tool alone owns stdout and inherits caller stdin, argv and exit status. Linux `flock` serializes
frontloader export replacement through formatter dispatch, and a separate installation lock protects
each installation. Retain the refreshed `scripts/shared-ci.py` template when adopting this
interface: older frontloaders do not serialize concurrent export reconstruction.
Other shared tasks release the dispatch lock at handoff so nested formatter calls can run.
This is a one-time consumer template refresh alongside initial adoption: a revision change
alone cannot serialize an old frontloader before it loads shared code. Old thin entries
retain sequential compatibility. Subsequent producer upgrades use only the existing grouped
revision references and require no new consumer downloader or formatter-version edits.

To prepare an image-owned persistent root, run:

```sh
python3 scripts/shared-ci.py node-tool --cwd "$PWD" --installation-root /opt/lens-node-tools --install-only
```

This installer command needs Node, npm, Python, Bash, Git, the consumer manifest/frontloader,
and a writable cache/build context. Keep the complete installation including
`.boxferry-node.json` and the three source files. Set image PATH to include
`/opt/lens-node-tools/node_modules/.bin` for ordinary native version/help probes.
At runtime pass the explicit `--installation-root /opt/lens-node-tools` before the tool name;
explicit installation roots default to read-only execution unless `--install-only` is set.
There is no directory-presence auto-selection. `--read-only` also explicitly selects this
behavior for the default revision cache, but never grants read-only custody to writable data.
Image installation must run as root, with root-owned files/directories and ancestors that the
runtime user cannot modify or replace. User-owned data with write bits removed is insufficient,
since its owner can restore permissions. A protected root-owned child of a root-owned sticky
temporary directory is also supported. The ordinary non-root image user then verifies the
manifest-selected immutable blobs and complete inventory without creating its lock, reinstalling,
or modifying its contents. Writable explicit roots fail visibly; use managed execution or
rebuild the image. The caller's
automation cache remains writable for ordinary frontloader verification. Read-only mode
requires an externally immutable image root; do not replace it concurrently. A stale image
fails with a rebuild/reinstall diagnostic. No mutable full-checkout override qualifies as
immutable evidence; hosted callers can use this same cache route without a second checkout.

For an offline image with a read-only consumer checkout, use the installed public entry point:

```sh
python3 /opt/lens-node-tools/boxferry-node-tools.py image-node-tool --repository-root "$PWD" --cwd "$PWD" prettier --config .prettierrc.json --ignore-path .prettierignore --no-editorconfig --check README.md
```

The installer retains its already authenticated standard-library bootstrap code beside the
three npm source files. This entry point requires protected image custody, checks the bounded
caller manifest revision against the root-issued installation receipt, verifies the complete
installed inventory and executes the selected runner. It performs no network request, Git
operation, cache/export preparation, lock creation or checkout write. It supports ordinary
non-root native probes with the existing read-only mounts/rootfs. A caller manifest revision
change requires rebuilding the image at that same revision. No fourth version pin or consumer
acquisition code is introduced; the existing Python and three-file npm exports remain closed.

Native image qualification retains consumer-root npm expectations when no source marker is
present. An explicit `.github/node-tools.json` may contain exactly
`{"schema":1,"source":"local"}` or `{"schema":1,"source":"automation"}`. The latter requires
the consumer manifest revision to equal the independently trusted automation SHA, then reads
the npm graph from that verified clean producer checkout. Unrelated consumer npm packages may
coexist, but direct declarations of either shared formatter fail as duplicate ownership.
Malformed, missing or ambiguous selected metadata fails without source fallback. Both native
architectures retain the exact Markdown CLI/library banner, usage, empty stderr and exit-2
help probe. A marker selects expectations; it does not prove image installation or rollout.

No dependency pin is added or moved by acquisition. Native npm continues to own the producer
root manifest/lock, grouped by `renovate-presets/common.json`'s **Node development tools** rule;
the existing **Shared BoxFerry automation** regex/native Actions group owns consumer manifest,
presets, workflow and checkout adoption together. Derived cache paths, receipts and the source
marker carry no Renovate version declarations. Existing approvals and image qualification
holds remain. Consumers must remove only their migrated formatter declarations and keep
unrelated application graphs. Actual cold/warm local, hosted, native amd64/arm64 and subsequent
grouped revision-upgrade evidence is required under #723 before claiming consumer adoption.
