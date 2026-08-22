# Releases

Release-plz prepares release pull requests. The protected release workflow publishes crates, tags,
checksums, and the GitHub release after that pull request is merged.

## Commit classification contract

Release-worthy code uses `feat`, `fix`, `perf`, `refactor`, or `revert`. Documentation, tests, CI,
build tooling, formatting, and maintenance use `docs`, `test`, `ci`, `build`, `style`, or `chore` and
do not create a product release.

Breaking pre-1.0 changes use `!`, a minor version, and concise migration notes. Do not keep unused
compatibility code solely to avoid the release.

## Maintainer flow

1. Merge release-worthy code after the complete local and hosted gates pass.
2. Review the release-plz pull request, lockstep versions, dependency requirements, and changelog.
   Its PR gate runs the same current-version release-metadata and changelog validator as the local
   and protected release gates; a version bump without dated, non-empty notes cannot pass.
3. Merge the release pull request.
4. Run the protected release workflow for the selected version.
5. Verify crates.io packages, checksums, tag, GitHub release, and installation.

The six supported crates publish in dependency order and use one version. Release notes are
extracted from `CHANGELOG.md`; no separate release-note files are maintained.

Yank a version only when it is unsafe or unusable. A normal bug receives a follow-up release.
