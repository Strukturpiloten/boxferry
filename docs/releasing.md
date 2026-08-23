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
The validator ensures that a version bump without dated, non-empty notes cannot pass.
