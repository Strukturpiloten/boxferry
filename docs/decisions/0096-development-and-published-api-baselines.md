# ADR 0096: Development and published API baselines

- Status: accepted; consumer rollout tracked separately
- Date: 2026-11-05
- Amends: [ADR 0092](0092-shared-development-automation.md)'s API validation purpose
- Preserves: [ADR 0093](0093-exact-candidate-cargo-tools.md)'s exact tool/compiler dispatch

## Context

A workflow-only DockerLens adoption encountered seven API incompatibilities between
existing development main and published `0.1.0`; its product source and versions were
unchanged. Published release compatibility and compatibility of a proposed development
change answer different questions. Overriding the release type, changing product versions,
or pretending an existing publication is initial would conceal that distinction.

## Decision

One typed [baseline helper](../../automation/src/boxferry_ci/api_baselines.py) selects
purpose, immutable comparison identity, workspace packages and supported Action/CLI
arguments for hosted and complete local checks. Legacy shared-workflow calls default to
`published`. Ordinary CI explicitly selects `development`; reusable CI defaults to
`published` and Release supplies that purpose explicitly, including validation-only mode.
GitHub reusable calls retain the original event context; purpose never depends on an
invented `workflow_call` event name.

Development compares the actual tested source against trusted Git history. PRs and merge
queues require their exact independently held event base and actual tested merge. Main
pushes compare the exact nonzero event `before` on first-parent default-branch history,
including every commit in a multi-commit push. Manual/local proposed branches compare the
whole proposal with their unique fork base against canonical default main. The helper
reads the allowlisted repository's advertised default HEAD over canonical HTTPS from an
isolated directory and requires its fetched tracking tip to match. It does not fetch or
mutate developer refs. Missing, stale, partial, shallow, ambiguous or rewritten history
fails with a next step. Git replace objects are disabled and grafts rejected.

For an already trusted main commit, clean source compares with its first parent. Dirty
local main may compare with the same Git commit only when the actual working source
fingerprint differs from the baseline snapshot. Index-only differences are insufficient.
The fingerprint covers tracked and nonignored untracked paths, bytes and executable modes,
within bounded reads; normal ignored build artifacts do not become source evidence.
Development uses a fixed minor allowance, permitting compatible additions while rejecting
unannounced breaking changes even if the proposed manifest version increased. Explicit
breaking intent can request major compatibility in development only.

Published checks select an exact canonical non-yanked stable registry version and use the
candidate's actual manifest version to infer the release allowance. Intent overrides are
rejected. Existing protected release approval and every complete validation prerequisite
remain necessary. The shared workflow exposes a distinct `published_api_candidate_sha`
only after published-purpose success. CI carries it to the Release aggregate, which
requires the exact candidate for validation-only and publication alike. Development success
cannot satisfy that prerequisite.

Workspace metadata retains private libraries and their Git baselines. Default workspace
iteration excludes private support libraries; explicit `--package` validates them in
development and published/initial-contract modes. API evidence does not enable publication
or change `publish = false`.

An existing Git crate has a development baseline even before its first publication. If
a crate is genuinely absent from the Git workspace, an existing canonical publication is
a per-package fallback. Only canonical absence and an explicit successful positive
`initial-api-contract` task authorize initial API evidence. HTTP errors, malformed data,
unavailable Git evidence, and prior yanked/prerelease publications never establish absence.
A prior publication without a usable stable baseline fails explicitly.

Evidence records purpose, source fingerprint, candidate, policy, automation, exact
comparison and verified CLI/compiler. A workflow-step-held digest binds the selected
record across candidate compilation; completion also rechecks actual source bytes and
declarations. A failed, cancelled or changed check emits no successful published binding.
The pinned existing Action's Git and registry baseline inputs remain in use. No product
dependency or competing version inventory is introduced.

Local consumers use a typed policy task through the existing locked interface-1 loader;
its command invokes `python -m boxferry_ci.api_baselines run`. One shared identity verifier
binds the selected revision to the actually loaded project. Extracted archives must match
existing verified Git objects, source bytes, modes, locks and exported tool metadata,
even when the consumer HEAD equals the automation revision. Manifest/path/environment
values only locate that evidence; verification never fetches, repairs or falls back.
The same verifier guards the real positive task. Direct checkouts must own their Git root;
dirty producer development stays distinct from clean published evidence.

## Consequences

Development adoption can assess only its own API change while release still discovers
accumulated incompatibilities and insufficient chosen versions. Main/manual/local checks
remain complete; only the API comparison purpose changes. Local selection needs canonical
network authority and fetched complete default-branch history, with explicit fetch guidance.
Each Rust consumer needs coordinated ordinary/local/release adoption and its own complete
and hosted checks; producer success is not workspace rollout evidence.

The Action remains `6b69fcf40e9b5fb17adeb57e4b6ecd020649a239` (v2.9). Read-only GitHub
API inspection of its descriptor, `src/main.ts` and shipped `dist/index.js` confirmed
`baseline-rev`, `baseline-root` and `baseline-version` forwarding to `check-release`.
Its MIT-licensed implementation was inspected, never copied into this implementation or
redistributed. Existing Action and tool pins retain their Renovate owners.

## Development event table

| Development event                               | Immutable comparison                                                                     |
| ----------------------------------------------- | ---------------------------------------------------------------------------------------- |
| PR / merge queue                                | Exact independently held event base to actual tested merge                               |
| Default-branch push                             | Nonzero event `before` on first-parent main history to tested candidate                  |
| Manual / local proposed branch                  | Unique fork base against canonical advertised and fetched default main to whole proposal |
| Clean candidate already on trusted main history | Candidate's first parent to candidate                                                    |
| Dirty local candidate on trusted main history   | Trusted HEAD to actual different working source, bound by fingerprint                    |

## Six-repository adoption map

| Rust consumer        | Definitions to adopt together                                                                                                            | Evidence still required                                          |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| BoxFerry (#576)      | Canonical helper/shared workflow; CI, queue/main/manual, local complete gate and Release                                                 | Complete producer gate and exact-candidate hosted checks         |
| ComposeLens (#196)   | Immutable automation/workflow pins; explicit development CI/local, published release and output guard                                    | Own complete/native and hosted gates                             |
| PodmanLens (#154)    | Same purpose/local/release contract; retain observation suites                                                                           | Own complete/native and hosted gates                             |
| QuadletLens (#168)   | Same purpose/local/release contract; retain generator suites                                                                             | Own complete/native and hosted gates                             |
| DockerLens (#174)    | Same purpose/local/release contract; retain seven existing published failures as release evidence                                        | Own complete/native and hosted gates; currently blocked adoption |
| KubernetesLens (#75) | Explicit Action `package: kubernetes-lens` and local `--package kubernetes-lens`; preserve positive initial API and disabled publication | Own complete/initial-contract and hosted gates                   |

The producer change does not edit or qualify sibling repositories. Pin/automation source
bindings must advance together under existing Renovate ownership; each consumer records
its own review, complete checks and hosted candidate before rollout is complete.
