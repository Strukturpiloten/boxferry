# Workflow linting

[The canonical tool inventory](../../automation/tool-artifacts.json) owns Actionlint's
publisher, release tag and both Linux archive SHA-256 values. The candidate-owned
[installer](../../automation/src/boxferry_ci/tooling.py) validates the complete inventory
before downloading or executing a tool. Its six binary tools and four-tool `core` profile
remain unchanged; Actionlint is an explicit `--tools actionlint` selection.

The installer accepts only `rhysd/actionlint` and `kjanat/actionlint` for Actionlint.
Both architecture rows must select the same publisher and release. Unknown publishers,
mixed origins and partial version updates fail before download. The other five publishers
retain their exact existing allowlist. Asset URLs use the actual validated row repository.
Schema-1 and source-aware schema-2 readers retain their existing integrity and cleanup rules.

## Publisher migration

The maintainer selected `kjanat/actionlint` for its newer workflow checks and Ubuntu 26.04
runner definitions. Publisher compatibility is an inert bridge: the active inventory still
selects RH v1.7.12. The initial fork v1.17.1 archives passed checksum/provenance verification,
but both binary advisory scans failed on Go 1.27.3's
[GO-2026-6859](https://pkg.go.dev/vuln/GO-2026-6859), fixed in Go 1.27.4. That release remains
blocked. This change installs no fork artifact and grants no advisory waiver.

The selected delivery path waits for clean upstream binaries; no downstream compiler or
source-build installer is introduced. A qualified fixed release and an admitted shared preset
must precede an actual inventory switch. Current immutable consumers retain their RH owner. The candidate preset extends the existing atomic attachment manager and `Canonical
downloaded tools` group to recognize both publishers, without a second manager or changed
approval, age or integrity rules. Native Renovate SDK tests exercise extraction, actual
publisher URLs, release/checksum replacement and grouping for current and isolated future
metadata; synthetic tags are never operational releases. Ongoing adoption belongs to
[issue #655](https://github.com/Strukturpiloten/boxferry/issues/655).

Review the fork's default cache-safety findings at reusable `workflow_run` calls and
set explicit reviewed cache ceilings before activating it. Keep those checks enabled;
a new diagnostic alone does not justify an exception.

## Local and hosted checks

The complete local gate runs actual `actionlint` against the repository, then
[the CLI regression harness](../../scripts/test-workflow-lint.py) against the same installed
executable. The existing hosted documentation job installs Actionlint using its candidate
inventory, runs actual workflow lint from the root checkout, and runs the same harness.
`Required CI` already requires this selected job; no new job, skip or failure waiver is added.
Formatting-only checks still run no tests.

The bounded harness verifies the actual version banner against the inventory and proves that
a mismatched version is refused. Authored inert workflows require successful known Linux and
ARM runner labels and rejected unknown labels, conflicting OS labels, invalid expressions and
malformed YAML. RH uses Ubuntu 24.04 labels; a future fork selection must pass Ubuntu 26.04
and ARM labels. No workflow or Action executes, no remote fixture is fetched, and every child
process has finite time/output bounds. A temporary owned directory is removed after success
or failure. A diagnostic build's runtime success cannot substitute for admitted artifact selection,
native architecture evidence or complete caller gates.
