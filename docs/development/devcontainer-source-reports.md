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
