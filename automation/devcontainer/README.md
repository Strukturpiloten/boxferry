# Official Dev Container graph generator

This isolated npm project owns the official `@devcontainers/cli` tool. Install
with `npm ci --ignore-scripts --prefix automation/devcontainer`. The package is
MIT-licensed and comes from the public npm registry; the lock records its
published integrity. No upstream implementation is copied into BoxFerry.

Local contributors reopen or rebuild through VS Code with their existing Podman
connection and ordinary caches. The shared image producer is hosted-CI-only:
reviewed Docker/Podman backends use standard fresh runner storage, with actual
store disk coverage and exact owned-container/tag cleanup. It never prunes the
store or deletes unrelated cache entries. Native x64/ARM qualification and
restricted-App acceptance remain separate from source admission under #563.

The trusted `boxferry_ci.devcontainer_updates` helper runs `devcontainer upgrade`
in private scratch without candidate hooks, App tokens, Docker credentials or
host configuration. It validates the complete OCI Feature graph before replacing
only `.devcontainer/devcontainer-lock.json`. Generation failure is diagnostic and
gets one bounded retry. Renovate can retain an `artifactErrors` PR; required graph
and image checks must reject it until coherent. Required dependency eligibility
owns the 72-hour hold for every introduced direct/transitive manifest, including
same-version digest changes and missing-date first-seen quarantine.

The limited Actions-hosted Renovate configuration ignores repository config and
owns native Feature declarations plus the exact nested GitHub CLI version option.
The hosted App retains all other owners, including this npm package. One native
feature group uses `renovate-devcontainer/features`; a scheduled official upgrade
also catches digest/transitive-only changes. The narrow maintenance helper uses
the reserved `renovate-devcontainer/lock-file-maintenance` branch, which Renovate
44.139.0 excludes from ordinary stale-branch pruning. An active native feature PR
takes precedence; the helper refuses human edits, stale bases and changed heads.
No duplicate regex lock owner remains.

This producer seed deliberately holds hosted Feature proposals until the qualified
caller is activated; the other hosted owners remain active.

This is producer code, not admitted automation. The producer must pass complete
primary/independent gates and complete main CI before an immutable successor is
admitted. Then four owners must activate the small caller and hosted partition
seed together, qualify the attenuated existing App token, verify bot-triggered
ordinary CI and native x64/ARM64 execution, and only then admit automatic updates.
The existing App credential names are `RELEASE_PLZ_APP_CLIENT_ID` and
`RELEASE_PLZ_APP_PRIVATE_KEY`. Actual restricted-token compatibility remains a
required hosted smoke; no extra user setup is assumed. DockerLens,
KubernetesLens and the website have no Dev Container feature job.
