# BoxFerry application integration tests

The scenario-contract suite discovers versioned migration scenarios, validates
independent neutral expectations and exact subject-level losses/diagnostics,
and executes every exporter and applicable semantic reimport. Mutation tests
must fail for their own reason; unperformed checks cannot count as passed.

Cargo-discovered integration-test entry points for application-level and cross-repository policy checks live here. Shared test-only helpers live in `support/` and must not become part of the application or a public library API.

The root [`tests/`](../../../tests/README.md) directory documents cross-crate scenario ownership, and the root [`fixtures/`](../../../fixtures/README.md) directory contains their inputs.

`public_api.rs` protects the core and additive facade surfaces. `compose_to_quadlet.rs` owns the
first public end-to-end golden conversion when both document-adapter features are enabled.
`document_route_matrix.rs` owns deterministic document-pair public facade, CLI,
validate-without-writing, chained conversion, discovery, fixed-point, path-consistency, policy,
diagnostic, and redaction boundaries using `fixtures/conversion/document-route-matrix/`.

With the `docker` feature, `docker_fixture_volume_labels.rs` checks independently
authored volume identities/labels over six fixtures (23 requests per profile), and
`docker_authored_bind_rehearsal.rs` owns the exact Nextcloud/Supabase bind and external-constraint
checks named in [ADR 0078](../../../docs/decisions/0078-trusted-docker-consumer-validation.md).
The latter retains full Nextcloud and only the explicitly named Supabase bind/mount projection,
plus the full Supabase protected-health negative control. Shared
`support/docker_volume_fixtures.rs` supplies loading/import/projection mechanics, not expectations.
The two validation examples produce inert volume artifacts or bounded read-only daemon metadata;
their unit tests use private temporary paths/fake sockets, not a runtime. These historical
checkpoint checks do not establish host-path/SELinux effects, application readiness or full
migrations; current CLI behavior is covered separately below. See
[ADR 0083](../../../docs/decisions/0083-bounded-docker-target-consumer-preparation.md).

`fixture_route_corpus.rs` discovers every positive adapter-contract and conversion manifest,
requires each input scenario to define every exporter reported by live capabilities, validates
reviewed artifacts and diagnostic sequences across the policy lattice, and re-imports eligible
document output to a same-format fixed point. The remaining route, CLI, report, and
repository-policy tests exercise focused Compose/Quadlet behaviors and their public orchestration
boundary.

`vendor_metadata.rs` owns inert metadata type/scope round trips, folded text, native wire records,
carrier ownership/conflicts, fresh provenance, privacy refusal and explicit Podman losses.

`quadlet_bundle.rs` owns bounded framing and create-new extraction. `quadlet_bundle_cli.rs` owns
transport output across routes, expanded input identity/provenance, physical/member reports and
byte-preserving extraction versus semantic conversion losses.

`repository_policy.rs` also owns static and focused helper contracts for the bounded Nextcloud,
Forgejo, Paperless, and Immich live application profiles. The Immich checks keep the authored
offline replay distinct from runtime evidence, pin the sole rootless cell and every external digest,
and protect CPU-only upload/derivative/persistence, capture privacy, selector, redaction, resource,
and cleanup boundaries without executing the live profile during the deterministic test suite.
Reviewed Paperless and Immich sanitizer-v3 cassettes additionally replay complete production
acquisition while repository policy keeps their redacted evidence separate from authored semantics.

## Docker development routes

The Unreleased CLI adds Docker to both axes for 16 neutral routes. Existing document/nine-route
subset checks remain independent regressions. `cli_docker_journeys.rs` owns authored bounded
read-only Unix-socket journeys, source selectors and ambiguity/no-match refusal, exact reviewed
targets, reconstruction/loss-policy separation, independent privacy/resource controls,
validate/convert parity, terminal artifacts and output safety. It uses synthetic replies, not a
host daemon. Facade acquisition and adapter source tests protect conservative library defaults,
explicit endpoint/limits/cancellation forwarding, observation provenance and BFD source findings.

`documentation_examples.rs` runs registered offline document-to-Docker conversions, validation and
refusal/no-output contracts. Docker-source help is option evidence only; the harness has no Docker
socket replay hook. Independent reviewed fixtures own expected native plans and sidecars.
`docker-dependencies.json` is conditional, retains typed decisions and `native_execution=false`,
and binds exact `docker-plan.json` bytes by SHA-256. Neither artifact is observed input or an
executor. Protected/incomplete required intent can refuse the complete Docker candidate even
under partial loss.

Test source is not passing evidence. Complete gates, compatible published dependency/API/package
checks, native Lens results, exact-pair admission, actual BoxFerry consumers and application
acceptance remain separate requirements. See [ADR 0085](../../../docs/decisions/0085-docker-cli-neutral-routes.md).

## Scenario acceptance ownership

`scenario_contract.rs` owns catalogue-only application acceptance. It validates both supported manifest spellings, all exporters for every input, independent neutral intent, exact artifact/diagnostic/loss sidecars, protected-value counterfactuals, and applicable reimports. Real-world Compose blobs are vendored with pinned Git provenance and reviewed legal files so this suite remains offline. Remote corpus fetching is a refresh check, not a test dependency.

Podman JSON and command artifacts are validated as deployment plans—including creates, external preconditions, image operations, start order, and CLI/API parity—and are never treated as observed inventory. `native-validation`, `runtime-probe`, and `reimport` remain separate dimensions; a reviewed gap in one cannot be renamed migration success by updating a golden file.

`deferred_quadlet.rs` covers embedded authorization, sourced expressions, target endpoints and
native literal parity. `deferred_quadlet_cli.rs` covers explicit provider plans, scope/order,
no acquisition, disclosure, invalid expressions and private output.
