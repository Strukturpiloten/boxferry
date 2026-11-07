# Shared development automation

[ADR 0092](decisions/0092-shared-development-automation.md) assigns shared [automation](../automation/) and [workflows](../.github/workflows/) to BoxFerry. Lens product graphs and website checks remain independent. Adoption remains incomplete; consumers require independent validation.

## Revisions and admission

The [zero-age-delta bootstrap](decisions/0097-read-only-zero-age-delta-bootstrap.md) covers only reviewed Podman PR159 and Quadlet PR175 permission repairs. Complete immutable trees stay inert; isolated exact A confirms unchanged inputs. Live canonical S supplies proof; A retains policy authority. Only `proved-zero` bypasses receipts. Unknown or changed inputs use the service path; invalid custody fails and selected CI remains required. Proof is no service receipt; hosted accessibility and recovery need separate evidence.

Callers bind immutable identities: `candidate_sha` is tested `github.sha`; `automation_sha` matches the reusable workflow's full BoxFerry SHA; `policy_sha` is the trusted PR/merge-queue base, or candidate for other events. Validate policy paths against the trusted checkout. The [admission reader](../.github/workflows/shared-dependency-wait.yml) reads the exact base manifest: when it selects A and the candidate proposes B, A checks the exact candidate and reports itself as implementation; CI validates B. There is no candidate fallback. Ordinary changed dependencies retain the 72-hour hold and the complete first-party Lens exemption; pushes, manual and release validation add no age hold. See [ADR 0093](decisions/0093-exact-candidate-cargo-tools.md) and [ADR 0095](decisions/0095-bounded-older-base-dependency-recovery.md).

## Validation and repository boundaries

For pull requests and merge queues, the planner compares trusted base and tested candidate, selecting known unions; unknown or unsafe changes select the complete plan. `Required CI` requires successful selected jobs and only planned skips; missing, failed, cancelled or unexpected skips fail closed. BoxFerry’s [policy](../scripts/shared-validation-policy.json) retains all fifteen prior policy jobs; the [automation package](../automation/src/boxferry_ci/) owns selection and aggregation.

Main, manual, release and local [complete gates](../scripts/check-all.sh) remain complete; focused feedback cannot replace them. Consumers preserve complete tasks when trusted policy lacks definitions. The [Dev Container seed adapter](../automation/src/boxferry_ci/devcontainer_seed.py) stays opt-in with reviewed approvals. Image qualification and updater activation remain separate under #563; ordinary adoption enables no image job or schedule.

The [Python project](../automation/pyproject.toml) and [lockfile](../automation/uv.lock) own uv, Ruff and ty. The interface-1 loader verifies pinned source. The [Node action](../.github/actions/shared-node/action.yml) uses locked Python, producer CI version and cache inputs. Native automation validates Node ownership and replacements; Rust checks Action pins. Updater caching stays disabled; adoption awaits producer admission. Repositories own checks, thresholds, native suites, binaries and evidence under [ADR 0079](decisions/0079-native-test-runtime-selection.md).

[Proposed original-graph recovery](decisions/0098-preserved-original-dependency-recheck.md) defines
how dependency versions stay fixed while main moves. Its historical metadata receipt cannot approve
a current merge: the original immutable waiter and fresh protected queue remain mandatory.
Consumer activation and live recovery evidence remain separate.

## Daily checks and agent repairs

Use [local verification](development-environment.md#local-verification), locked Python and [agent contract](../AGENTS.md). Complete gates and independent review remain required.

## Pinning and publication

Align workflow and checkout pins; coordinate upgrades and Renovate ownership. Rollback restores both reviewed full SHAs. ADR 0092 permits a labelled, reviewed, complete-gate-tested full SHA bootstrap until the first automation release tag; floating refs remain prohibited. External Actions retain integrity and release-tag requirements. `automation-v1` advances non-forced only for shared-source changes admitted by complete main CI; consumer-only pins cannot trigger a loop.

Release evidence binds the exact candidate. Publication requires the caller’s protected environment and explicit approval. The shared crate component defaults to validation-only without authentication or publication mutations; unpublished products remain validation-only until separately authorized. Callers retain product-specific safeguards. See [release policy](releasing.md) and [shared release validation](../.github/workflows/shared-release-validation.yml).

## API comparison purpose

[ADR 0096](decisions/0096-development-and-published-api-baselines.md) defines event, initial-API and rollout rules. Ordinary CI selects development against trusted Git history; legacy calls default to published. Release selects published and requires `published_api_candidate_sha` for its exact candidate, including validation-only mode. Published checks infer compatibility from the selected manifest version and reject intent overrides. Manual CI remains development even when nested Release retains `workflow_dispatch`. Missing history, source mutation and failed checks cannot produce published evidence. Explicit private workspace libraries receive API checks; default selection excludes private support libraries.

## Nested dependency audit arguments

The shared workflow passes Cargo Deny `--config` before `check` and sets `manifest-path`. The required Box-only producer job tests a nested fixture with non-default policy and four audit categories; default policy rejects its synthetic license. Its guarded `dependency-action-fixture/` stays isolated from `_automation`. The [fixture README](../automation/tests/fixtures/cargo-deny-action/README.md) records controls, provenance and ownership. Hosted fixture success and complete gates precede admission; consumer adoption, native qualification, image and App evidence, and publication approval remain separate.

The complete unittest entrypoint captures file descriptors with a bounded worker, redacts and neutralizes diagnostics, and treats workflow-command fixtures as data. Test and reporting failures remain distinct. CI, local validation and inherited Release share one discovery. Clipping is reported; exceeded capture, result or report budgets and incomplete drainage fail validation.

## Eligibility resolution diagnostics

Non-authorizing diagnostics report validated event/run/attempt/SHA, stage/guard, counts and bindings; unknowns are `null`. Raw API, error, environment, credential and URL values are excluded. At most four 4 KiB records preserve original failures; receipt, retry, merge and callback semantics remain unchanged.
