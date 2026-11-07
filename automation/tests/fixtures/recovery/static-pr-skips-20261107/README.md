# Static PR event exclusions

These bounded public GitHub metadata and repository-owned workflow observations were read on
2026-11-07 for BoxFerry #613. They establish job presentation and exact event-only guards,
not original-candidate provenance, dependency eligibility, or successful recovery.

The DockerLens run contains an unrelated Rust failure and the website run already succeeded.
Both are ineligible as positive recovery examples. Positive unit cases are separately authored
synthetic inputs; their effect, registry and canonical-source boundaries are mocked.

`index.json` records source roles and hashes. Docker workflow bytes were read at its recorded
immutable main revision through `gh api repos/Strukturpiloten/docker-lens/contents/...`; the
website workflow is a prepared local caller, without an admission claim. Run/job metadata was
read with `gh api repos/Strukturpiloten/<repository>/actions/workflows/<workflow>/runs?event=pull_request&per_page=1`
and the selected run's exact attempt jobs endpoint with `per_page=100`. No workflow was triggered.
DockerLens LICENSE metadata identifies MPL-2.0. The website license endpoint returned 404;
no website license is asserted. These are maintainer-owned workspace workflow fixtures;
no external native oracle implementation or runtime image is copied or executed. The retained public metadata contains no
credential, registry response, environment dump or build output. These are intentional historical
fixtures, outside automatic dependency update ownership.
