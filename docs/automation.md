# Shared development automation

BoxFerry owns the shared [automation package](../automation/), [reusable workflows](../.github/workflows/) and composite actions. Lens products consume development tooling without joining product dependency graphs; the website retains its own site checks. [ADR 0092](decisions/0092-shared-development-automation.md) defines ownership and rollout: adoption remains incomplete, and each consumer requires independent validation.

## Revisions and admission

Shared workflow callers bind three immutable identities: `candidate_sha` is the exact tested `github.sha`; `automation_sha` identifies the BoxFerry implementation and must match the reusable workflow reference's full SHA; `policy_sha` identifies the trusted pull request or merge queue base, or the candidate for other events. Policy paths are repository-relative and validated against the trusted checkout. See [shared-source](../.github/actions/shared-source/action.yml) and the [shared workflows](../.github/workflows/).

Dependency admission reads the manifest from the exact trusted base Git object. If that base selects automation A while the candidate proposes B, the [admission reader](../.github/workflows/shared-dependency-wait.yml) checks the exact candidate using A and reports A as implementation; other CI validates B. The base manifest must exist: there is no candidate fallback. Push, manual and release validation add no age-admission hold. See [admission](../automation/src/boxferry_ci/admission.py), [eligibility](../automation/src/boxferry_ci/eligibility.py), [ADR 0093](decisions/0093-exact-candidate-cargo-tools.md), and [ADR 0095](decisions/0095-bounded-older-base-dependency-recovery.md) for age policy and bounded recovery.

## Validation and repository boundaries

For pull requests and merge queues, the planner compares trusted base and tested candidate. Known safe changes select the union of needed checks; unknown or unsafe changes select the complete plan. The stable `Required CI` check requires every selected job to succeed and accepts skips only for explicitly unselected jobs. Missing, failed, cancelled or unexpectedly skipped results fail closed. The [validation policy](../scripts/validation-policy.json) and [automation package](../automation/src/boxferry_ci/) define selection and aggregation.

Main pushes, manual CI, release validation and the local [complete gate](../scripts/check-all.sh) remain complete. Change-aware feedback, formatting and linting do not replace that gate. During adoption, consumers preserve complete existing tasks when trusted policy lacks new definitions. The [Dev Container seed adapter](../automation/src/boxferry_ci/devcontainer_seed.py) remains opt-in with reviewed approvals.

The shared [Python project](../automation/pyproject.toml) and [lockfile](../automation/uv.lock) define canonical uv, Ruff and ty tooling. The interface-1 local loader verifies pinned source before delegating uv setup and locked execution. Each repository owns its feature checks, coverage thresholds, native suites, binaries and evidence; runtime selection and safeguards follow [ADR 0079](decisions/0079-native-test-runtime-selection.md) and the owning harness.

## Daily checks and agent repairs

Use [local verification](development-environment.md#local-verification), the locked Python project and the root [agent contract](../AGENTS.md). Focused checks do not replace complete gates or independent review.

## Pinning and publication

Keep workflow references and automation checkout pins aligned; coordinate upgrades and Renovate ownership across producer and consumers. Rollback restores the previous reviewed full SHA in both places. ADR 0092 permits an honestly labelled, reviewed and complete-gate-tested full SHA bootstrap until the first automation release tag; floating references remain prohibited. External Action pins retain their own integrity and release-tag requirements. After verified bootstrap, `automation-v1` advances non-forced only for shared-source changes admitted by successful complete main CI; consumer-only pins cannot trigger a loop.

Release validation binds repository-specific evidence to the exact candidate. Publication requires the caller's protected release environment and explicit approval. The shared crate component defaults to validation-only, without authentication or publication mutations; unpublished products remain validation-only until separately authorized. Callers retain product-specific safeguards. See [release policy](releasing.md) and [shared release validation](../.github/workflows/shared-release-validation.yml).

## API comparison purpose

[ADR 0096](decisions/0096-development-and-published-api-baselines.md) defines event, initial-API and rollout rules. Ordinary CI explicitly selects development against trusted Git history; legacy/reusable calls default to published. Release selects published and requires `published_api_candidate_sha` for its exact candidate, including validation-only mode. Published checks infer compatibility from the selected manifest version and reject intent overrides. Manual CI remains development even when nested Release retains `workflow_dispatch`. Missing history, source mutation and failed checks cannot produce published evidence. Explicit private workspace libraries receive API checks; default selection excludes private support libraries.

## Nested dependency audit arguments

The shared workflow passes Cargo Deny's `--config` as a global `arguments` value before `check`, with `manifest-path` as the Action's explicit input. The required producer dependency job checks an authored nested fixture with a non-default policy and all four audit categories. Its default policy rejects the synthetic license; this is not a product license exception. The [fixture README](../automation/tests/fixtures/cargo-deny-action/README.md) records controls, upstream provenance and Renovate ownership. Hosted fixture success and complete gates precede admission; consumer adoption, native qualification and publication approval remain separate.
