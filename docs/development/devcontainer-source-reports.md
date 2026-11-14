# Dev Container source-resolution reports

The canonical source job attempts its exact-path, seven-day artifact upload even after
resolution fails. Resolution and upload must both succeed before native image lanes may run;
a retained failure report never grants acceptance. The resolver records bounded, redacted
causes and validated candidate/policy/automation identities, computes success exports only
after tool authority validation, and reports unavailable output destinations without exposing
raw exceptions. Missing reports or upload failures retain a failed source prerequisite with
an actionable log diagnostic. Every resolver failure also logs a bounded, redacted original
cause and validated bindings, so a later artifact-service failure cannot erase diagnostics.
This log channel neutralizes modern and legacy workflow commands. Atomic report and command-file
writes remove only their own temporary files and preserve prior command-file bytes.

The authored HTTP-boundary fixture in `automation/tests/source_report_fixture.py` exercises
the actual CLI and metadata reader. Its controlled reports are diagnostic fixtures, never
real sibling revisions or native acceptance. Producer tests precede admission/distribution;
operational adoption and hosted diagnostics must then select that immutable admitted source.

## Manual hosted diagnostics

`Dev Container source report diagnostics` is a separate manual, main-only workflow for
BoxFerry. It selects immutable automation solely from the exact candidate's existing
`.github/automation.json`, then uses that source's binding action, locked Python, actual
HTTP-boundary fixture and canonical source-job final Bash. Producer admission/distribution
and coherent operational adoption must precede dispatch. It does not resolve live sibling
revisions or build native images.

The six fixed rows are `success`, `unavailable`, `malformed`, `invalid-revision`,
`missing-report` and `unavailable-report-loss`. The fixture invokes the actual resolver CLI
and propagates its exit. The five negative rows must remain red; `success` must be green.
Missing-report exercises a successful CLI followed by the fixture's own report removal.
Unavailable-report-loss records the generated failure report's SHA-256 in a separate sidecar
before removing only that proven fixture-owned file. This is controlled file loss; it is not
an artifact-provider outage or evidence that one occurred.

An always-on observer validates fixture kind, candidate/policy/automation bindings, selected
module digest, request counts, exit, independently expected original causes and bounded regular
files. It refuses a changed safe cause in either channel before forwarding or report removal.
It forwards the real captured,
redacted `resolver.stderr` to the job log, including the original cause when the report is
removed. The primary upload uses the same pinned Action and exact-file/error/seven-day
contract as the selected canonical workflow. A strict precheck gates the separate fixed-file
sidecar upload. Artifact names include candidate, run, attempt and case and identify diagnostic
fixtures. Cleanup validates the run-owned marker and rejects foreign identities, symlinks and
unknown entries before deleting only the finite evidence tree.

Regression contracts run through the existing shared-automation test entrypoint in local and
CI gates. They exercise the real fixture CLI, all six report/log paths and the actual canonical
final Bash with successful, failed, cancelled and skipped outcomes. Native Renovate extraction
covers the added workflow using the existing Action manager and grouping rules; the manifest
remains the only automation source pin, so no additional custom manager is introduced.

Source tests and an expected-red diagnostic workflow do not prove native acceptance. Review
all six jobs and their setup, fixture, observer, exact report upload, sidecar, final and cleanup
step outcomes on the exact current attempt. Retain artifact IDs/digests/bytes and actual job
logs, correlate them with candidate/policy/automation identities and selected Git blobs, and
preserve unexpected failures. Successful normal protected CI and separate native qualification
remain required; this workflow creates no native receipt or Required-CI waiver. Hosted
execution and artifact-service-outage behavior are not established by this implementation.
