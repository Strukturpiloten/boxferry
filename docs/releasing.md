# Releases

Release-plz prepares release pull requests. The protected release workflow publishes the lockstep
crate set, tag, checksums, and GitHub release after that pull request is merged.

## Commit classification contract

- Release-worthy code uses `feat`, `fix`, `perf`, `refactor`, or `revert`.
- Non-release work uses `docs`, `test`, `ci`, `build`, `style`, or `chore`.

Breaking pre-1.0 changes use `!`, a minor version, and concise migration notes.

## Maintainer flow

1. Merge release-worthy code after local and hosted gates pass.
2. Review the release-plz PR, lockstep versions, internal requirements, and changelog.
3. Merge the release PR.
4. Run the protected release workflow for its version.
5. Verify crates.io packages, checksums, tag, GitHub release, and installation.

Release notes come only from `CHANGELOG.md`. The PR gate runs its validator as a dedicated job
required by the aggregate gate. Yank a version only when it is unsafe or unusable.

Record user-visible changes in the single `Unreleased` section. Ordinary product pull requests
must not create a future numbered section, set its date, or bump the lockstep workspace version.
Release-plz inserts only the dated version heading, with blank lines on both sides; the curated
notes below `Unreleased` thereby become that release's notes exactly once. During release
preparation, the validator requires an empty `Unreleased` section and
one dated, usable numbered section matching the workspace version.

## Publication order and recovery

The workflow publishes crates in manifest dependency order. Normal, build, and development
dependencies must all be available on crates.io before a dependent package is prepared.

The publication helper skips package versions already visible on crates.io. After correcting an
interrupted release, remove only the unpublished release tag and rerun the workflow from the
corrected default branch. Never move or remove a tag belonging to a published GitHub release.
