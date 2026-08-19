# Releases

Release-plz prepares release pull requests for release-worthy code changes on `main`. Documentation,
tests, CI, and maintenance commits do not create product releases.

## Maintainer flow

1. Merge a Conventional Commit with a release-worthy `feat`, `fix`, `perf`, `refactor`, or `revert`
   type.
2. Review the release-plz pull request, version changes, and changelog entry.
3. Merge the release pull request after all required checks pass.
4. Run the protected release workflow for the selected version.

Supported BoxFerry crates publish in lockstep. Publication verifies package contents, crates.io
ownership, dependency order, checksums, tags, and the GitHub release.

Pre-1.0 breaking API changes use a minor release and concise migration notes. Do not retain unused
compatibility code solely to avoid the version change.
