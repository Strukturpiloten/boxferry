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

The canonical inventory selects `kjanat/actionlint` v1.17.1 for Ubuntu 26.04 workflow
linting. The existing Renovate manager keeps the two architecture archive checksums and
release tags in the `Canonical downloaded tools` group with the existing age, integrity
and approval rules. The publisher bridge from PR #658 is in the admitted preset used by
BoxFerry. Historical immutable revisions retain their original publisher and owner.
[Publisher bridge](https://github.com/Strukturpiloten/boxferry/pull/658)

Both release archives matched the upstream checksums and the GitHub release asset digests.
The GitHub-hosted release workflow attested both archives at tag v1.17.1 and source commit
`220b76cfa568e00ed8b3c476898f849a722b88e2`. The binaries report Go 1.27.3. Enforcing binary
scans with `govulncheck` v1.9.0 exited 3 on both architectures and reported one advisory,
[GO-2026-6859](https://pkg.go.dev/vuln/GO-2026-6859), in `net/http`. Go 1.27.4 fixes the
finding. [Release](https://github.com/kjanat/actionlint/releases/tag/v1.17.1) ·
[Attested build](https://github.com/kjanat/actionlint/actions/runs/42603907655/attempts/1)

The maintainer accepted that finding for these two Linux v1.17.1 release archive digests.

| Architecture | SHA-256                                                            |
| ------------ | ------------------------------------------------------------------ |
| amd64        | `6bfe0f49f0e108b9b23732baf7de63bfa4d89174fa40f9cb98feb9676df3318f` |
| arm64        | `81fa498f30f5dfadcc87fff0ff073281703fc56e9b50bc523b415c69aef0cccc` |

Other findings and changed artifacts require their own qualification. The recheck is due
11 December 2026. [Maintainer decision and recheck #666](https://github.com/Strukturpiloten/boxferry/issues/666) ·
[Qualification and rollout #655](https://github.com/Strukturpiloten/boxferry/issues/655)

The dependency eligibility service declares `cache-mode: none` because its reused
workflow needs no Actions cache access. Its ordinary read and write permissions continue
to govern the eligibility service operations.
[Reusable workflow cache limits](https://docs.github.com/en/actions/how-tos/reuse-automations/reuse-workflows#controlling-cache-access-in-reusable-workflows)

## Local and hosted checks

Both local gates install the candidate's checksum-pinned Actionlint in the worktree
before linting, including when the Dev Container provides an older copy. The complete
local gate runs actual `actionlint` against the repository, then
[the CLI regression harness](../../scripts/test-workflow-lint.py) against the same installed
executable. The existing hosted documentation job installs Actionlint using its candidate
inventory, runs actual workflow lint from the root checkout, and runs the same harness.
`Required CI` already requires this selected job; no new job, skip or failure waiver is added.
Formatting-only checks still run no tests.

The bounded harness verifies the actual version banner against the inventory and proves that
a mismatched version is refused. Authored inert workflows require successful known Linux and
ARM runner labels for Ubuntu 26.04. They require rejection of unknown labels, conflicting OS
labels, invalid expressions, malformed YAML and a reusable `workflow_run` call without an
explicit cache ceiling. The harness lints local fixtures without executing their commands.
Child processes have finite time and output bounds. The harness removes its temporary
directory after success or failure.
