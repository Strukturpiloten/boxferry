# Shared development automation

BoxFerry owns the shared [automation package](../automation/), [reusable workflows](../.github/workflows/) and composite actions. Lens products consume these as development tooling; their product dependency graphs remain independent. The website shares validation mechanics while retaining its own site checks. [ADR 0092](decisions/0092-shared-development-automation.md) records the architecture and rollout constraints. Rollout remains incomplete; validate each consumer independently.

## Revisions and admission

Shared workflow callers bind three distinct immutable identities. `candidate_sha` is the exact tested `github.sha`; `automation_sha` is the BoxFerry implementation and must match the full SHA in the reusable workflow reference; `policy_sha` is the trusted pull request or merge queue base, or the candidate for other events. The common source setup validates the caller and held checkouts. Policy paths are repository-relative and validated against the trusted checkout. See [shared-source](../.github/actions/shared-source/action.yml) and the [shared workflows](../.github/workflows/) for their individual inputs.

Dependency admission for a pull request or merge queue reads the manifest from the exact trusted base Git object. If that base selects automation A while the candidate proposes B, the [admission reader](../.github/workflows/shared-dependency-wait.yml) checks the exact candidate using A and reports A as its implementation. B is validated by other CI; it does not replace the trusted admission authority before merge. The base manifest must already exist; there is no candidate fallback. Push, manual and release validation do not add an age-admission hold. See [admission.py](../automation/src/boxferry_ci/admission.py) and [eligibility.py](../automation/src/boxferry_ci/eligibility.py) for the contract and failure behavior.

## Validation and repository boundaries

For pull requests and merge queues the planner compares the trusted base with the tested candidate. Known safe changes select the union of needed checks; unknown or unsafe changes select the complete plan. The caller's stable `Required CI` check requires every selected job to succeed and accepts skips only for explicitly unselected jobs. Missing, failed, cancelled or unexpectedly skipped results fail closed. The [validation policy](../scripts/validation-policy.json) and [automation package](../automation/src/boxferry_ci/) define selection and aggregation.

The [Dev Container seed adapter](../automation/src/boxferry_ci/devcontainer_seed.py) remains opt-in with reviewed approvals.

Main pushes, manual CI, release validation and the local [complete gate](../scripts/check-all.sh) remain complete. Change-aware local feedback and formatting or linting alone do not replace that gate. During adoption, consumers must preserve complete existing task behavior when trusted policy lacks new definitions.

The shared [Python project](../automation/pyproject.toml) and [lockfile](../automation/uv.lock) define the canonical uv, Ruff and ty tooling. The stable interface-1 local loader verifies the pinned source, then delegates uv setup and locked execution to that shared implementation. Consumer checks cover the loader interface rather than equality with the changing implementation. Repository-owned feature checks, coverage thresholds, native suites, binaries and evidence remain in their respective repositories. Compose provider, Podman observation, Quadlet generator, Docker native and BoxFerry application checks remain distinct; runtime selection, resource limits, privacy and cleanup follow [ADR 0079](decisions/0079-native-test-runtime-selection.md) and each owning harness.

See [ADR 0093](decisions/0093-exact-candidate-cargo-tools.md) for SemVer tooling and age checks.

## Daily checks and agent repairs

From the repository root, `python3 scripts/validation-plan.py plan --event local` previews
selected checks; `python3 scripts/validation-plan.py run-local` runs them. Use `run-local --docs-only`
for documentation feedback or `run-local --full` for the full plan. `./scripts/check-all.sh` runs the
complete gate; `--check` avoids formatting. See [prerequisites](development-environment.md).
Read diagnostics, retain original failure evidence, and rerun relevant checks after repair.

Bound repairs by repository, owned files, and expected outcome. Independently review the original
requirement and expected behavior, not only tests derived from the change.
The root [agent contract](../AGENTS.md) governs all paths; scoped additions for
[automation](../automation/AGENTS.md), [workflows](../.github/AGENTS.md), and
[crates](../crates/AGENTS.md) supplement it.

For focused Python feedback, use `uv run --locked --project automation ruff check automation`
and `uv run --locked --project automation ty check --project automation`. The complete gate runs
automation and native-policy tests with their required setup.

Guidance ideas were reviewed from [Awesome Copilot at 8178d1939a203f691067073dcab2a3f1e14072d3](https://github.com/github/awesome-copilot/tree/8178d1939a203f691067073dcab2a3f1e14072d3)
([MIT, copyright GitHub, Inc.](https://github.com/github/awesome-copilot/blob/8178d1939a203f691067073dcab2a3f1e14072d3/LICENSE));
[issue #553](https://github.com/Strukturpiloten/boxferry/issues/553) records the reviewed sources.
No upstream instructions or code are copied or installed.

## Pinning and publication

Keep workflow references and automation checkout pins aligned. Coordinate upgrades and Renovate ownership across producer and consumers; rollback restores the previous reviewed full SHA in both places. Until the first automation release tag exists, ADR 0092 permits a reviewed, complete-gate-tested full commit SHA with an honest bootstrap comment for this internal infrastructure. It does not permit floating references or authorize a release. External Action pins retain their own reviewed integrity and release-tag requirements. After its initial verified bootstrap, `automation-v1` advances automatically only for shared-source changes admitted by successful complete main CI. The distribution service performs a non-forced branch update; consumer-only pin changes cannot trigger an update loop.

Release validation binds required repository-specific evidence to the exact candidate. Publication remains behind the caller's protected release environment and explicit approval; release preparation alone does not publish. The shared crate component defaults to validation-only, which validates the gate and inventory without authentication or mutating publication operations. Unpublished products remain validation-only until separately authorized. Product-specific release hooks and their safeguards remain with the caller. See [release policy](releasing.md), [shared release validation](../.github/workflows/shared-release-validation.yml), and [shared crate publishing](../.github/actions/shared-crate-publish/action.yml).
