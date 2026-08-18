# Release process

BoxFerry publishes its eight crates in one lockstep version. Release preparation is automated by
release-plz; the protected `Release` workflow remains the only component allowed to publish
crates, create tags, or create GitHub releases.

## One-time GitHub setup

1. Create one organization-owned GitHub App for the three Strukturpiloten repositories. Disable
   webhooks and grant repository **Contents: read and write** and **Pull requests: read and
   write**.
2. Install the App on `boxferry`, `compose-lens`, and `quadlet-lens`. Whenever repository
   permissions change, review and approve the updated installation permissions in the
   organization before rerunning the workflow.
3. Store the App client ID as the organization Actions variable
   `RELEASE_PLZ_APP_CLIENT_ID`. Store only the private key as the organization Actions secret
   `RELEASE_PLZ_APP_PRIVATE_KEY`. Limit both to these three repositories.
4. Keep the default workflow token read-only. The App token is used only to create or update the
   release pull request so that normal pull-request CI runs.
5. Keep the protected `release` environment, required reviewer, default-branch restriction,
   trusted publishers, tag ruleset, and immutable-release setting unchanged.
6. Require the stable `PR gate` status check in default-branch protection instead of enumerating
   its implementation jobs individually.

The release-plz configuration disables Cargo publication, Git tags, and GitHub releases. Its only
write operation is the `release-plz-*` preparation branch and pull request.

## Routine release

1. Merge ordinary reviewed changes into the default branch. No release issue, local release
   branch, or manually created release pull request is needed.
2. Review the release-plz pull request. It updates the lockstep Cargo version, internal dependency
   requirements, lockfile, and root `CHANGELOG.md`. Normal CI must pass before merge.
3. Merge the release-plz pull request. Only a merged pull request whose head starts with
   `release-plz-` dispatches the protected `Release` workflow.
4. Approve the `release` environment deployment. The workflow revalidates the repository,
   publishes the eight crates in dependency order, creates attestations and checksums, and
   publishes the immutable GitHub release.

Use concise pull-request titles such as `feat: ...`, `fix: ...`, or `feat!: ...`. Release-plz also
accepts other titles, but these prefixes make version selection and changelog grouping clearer.
For intentional pre-1.0 public breaks, use a breaking title and review the resulting minor version.

GitHub release notes are extracted from the matching version section in `CHANGELOG.md`, which is
the only release-history source in the repository. Keep changelog entries short and move technical
detail into the canonical topic documentation. After this transition, do not hand-maintain the
`[Unreleased]` section; release-plz generates the reviewed version section from merged pull-request
and commit titles.

## Current 0.3.0 transition

The current tree already contains the BoxFerry 0.3.0 version and changelog section. Merging the
automation setup pull request will not publish it because that branch is not named `release-plz-*`.
After setup, review the first release-plz pull request and merge it only if it still prepares
0.3.0. If release-plz has no preparation change to propose, run **Actions → Release → Run
workflow** once from the reviewed default-branch commit and approve the environment. Later
releases follow the routine automated path.

## Recovery

`workflow_dispatch` remains available for retries. Rerun `Release` from the same default-branch
commit after a transient failure; the workflow verifies an existing tag, replaces only its own
draft release, and skips crate versions already visible on crates.io. Never replace a published
tag or release. If a corrected workflow needs a new commit after an unpublished tag was created,
remove only that unpublished tag before retrying.

The completed 0.1.1 bootstrap token must remain absent. Normal releases use only the existing
crates.io trusted-publisher identities for `release.yml` and the protected `release` environment.
