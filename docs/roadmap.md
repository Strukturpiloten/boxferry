# Roadmap

This roadmap describes dependency order, not delivery dates. A later phase may be explored early, but stable behavior must be built on completed lower layers.

Cross-repository delivery uses the stable task numbers in the [implementation plan](implementation-plan.md). This roadmap remains the detailed internal phase order for BoxFerry.

## Product contract and first major milestone

BoxFerry is an N-to-N conversion system. Docker runtime resources, Docker Compose, Podman runtime
resources, and Podman Quadlet are each required as both a source and a target for the first major
milestone. Pairwise routes compose through the neutral application model; they are not implemented
as sixteen unrelated converters.

### Source adapters

- [x] Import Docker Compose into the neutral model for the documented first subset.
- [x] Decode explicitly selected Docker runtime resources and reconstruct the documented subset.
- [x] Decode explicitly selected Podman runtime resources and reconstruct the documented subset.
- [x] Establish the first Quadlet importer slice for direct images, container names, owned network
  units, and owned volume units with source provenance and explicit unsupported outcomes.
- [x] Extend Quadlet import with the exact shared command, protected environment, scalar-port,
  named/absolute-mount, and named-network subset, including explicit external resources and an
  exact Quadlet-to-Compose golden route.
- [x] Import the conservative Quadlet metadata and execution-context subset: labels, IPv4/IPv6/
  `host-gateway` host mappings, user and numeric group identity, supplementary groups, user
  namespaces, absolute working directories, and read-only-root state.
- [x] Import section-aware systemd lifecycle and dependency intent: exact `Restart=no`, explicit
  approximations for unbounded restart policies, and complete sibling `Requires`/`Wants` plus
  `After` startup relationships without inventing services for arbitrary host units.
- [x] Import regular Quadlet health commands, explicit disable intent, intervals, timeouts, retry
  counts, and startup grace periods with protected command values and validated Podman scalars.
- [x] Import repeatable default/`type=mount` Quadlet secret grants as references to external
  Podman secrets, preserving reviewed target/UID/GID/mode options and provenance without reading
  secret material.
- [x] Import application-owned `.pod` identities with matching explicit runtime names and ordered
  sibling container membership as provenance-aware neutral service groups.
- [x] Import and export the reviewed container and pod topology keys with explicit reset, conflict,
  capability-floor, and group-scope behavior.
- [x] Import and export the ten reviewed network-definition keys with distinct runtime names and
  one associated IPAM row; native resets, duplicates, and multi-row association remain explicit.
- [x] Import and export all typed Quadlet volume settings, including distinct runtime/service
  names, protected raw lists, 6.0.0 `UID=`/`GID=` floors, and typed image-artifact validation.
- [x] Import repeated absolute-literal Quadlet environment-file declarations without file I/O,
  retaining order and protected paths while making provider-parser uncertainty explicit.
- [ ] Extend Quadlet import across the remaining first-milestone semantic subset, including
  divergent runtime pod names and safely decoded native value forms not covered by the reviewed slices.
- [x] Accept typed Quadlet `.image` and `.build` unit families.
- [ ] Accept typed Quadlet `.kube` and `.artifact` unit families.

### Target adapters

- [x] Export the documented neutral subset to Docker Compose or `podman-compose` definitions.
- [x] Export the documented neutral subset to Podman Quadlet definitions.
- [x] Consume released ComposeLens 0.1.13 and export ordered neutral environment-file declarations
  through its new short/long generated-document boundary.
- [ ] Produce deterministic Docker runtime deployment plans from the neutral model.
- [ ] Produce deterministic Podman runtime deployment plans from the neutral model.
- [ ] Add explicit executors that apply an authorized runtime plan without ambient discovery.

### Route orchestration

- [x] Prove the shared importer → neutral model → exporter path in the public library API.
- [x] Expose all four Compose/Quadlet document routes through the CLI registry, including native
  Compose canonicalization and neutral-model Quadlet canonicalization.
- [x] Add the [document conversion CLI contract](cli-vnext.md) with nested input and output selection.
- [ ] Expose Docker-runtime, Podman-runtime, Compose, and Quadlet inputs and outputs through that
  contract without duplicating conversion rules in the CLI.
- [x] Add the [privacy-safe local error-report bundle](error-reports.md) without automatic upload
  or raw input collection.
- [x] Preserve structured `boxferry-quadlet` syntax, typed-model, and document-set diagnostics in
  CLI reports with input aliases, static label detail, and path/secret redaction coverage.
- [ ] Add offline golden tests for every source/target combination over their shared supported
  subset.

## Status key

- [x] Completed and validated
- [ ] Open

## Phase 0: foundation — completed

- [x] Accept repository and licensing decisions.
- [x] Establish documentation and ADR practice.
- [x] Scaffold the Cargo workspace and crate boundaries.
- [x] Define the public library facade, reusable component-crate, CLI-parity, and publication boundaries.
- [x] Define Rust version, lint, dependency, CI, and release policies.
- [x] Define fixture provenance requirements.
- [x] Define the product diagnostic schema.

## Phase 1: application model and engine — completed

- [x] Implement the minimal application graph for one multi-service application.
- [x] Represent explicit ordered hostname mappings with raw-preserving IPv4, IPv6,
  implementation-token, and deferred-value classification.
- [x] Represent resource ownership, lifecycle, service attachment, and declared-source provenance.
- [x] Implement provenance and structured diagnostics.
- [x] Implement target profiles and conversion outcome policies.
- [x] Support strict planning plus explicitly authorized partial output with a diagnostic for every loss.
- [x] Provide adapter contracts and an in-memory test adapter.

## Phase 2: Compose to Quadlet vertical slice — completed

- [x] Consume released ComposeLens 0.1.3 as an independent crates.io dependency.
- [x] Consume QuadletLens 0.1.3 as an independent crates.io dependency.
- [x] Import Compose images, commands, environment, single ports, named volumes, bind mounts,
  networks, explicit profiles, source provenance, and SELinux relabel intent.
- [x] Consume a loss-aware typed ComposeLens merged-project view for multi-file projects without
  reparsing canonical YAML or losing source provenance.
- [x] Export the first neutral subset through QuadletLens with explicit outcomes for deferred value forms.
- [x] Preserve declaring-service ownership by keeping the first slice in separate container units.
- [x] Select pod grouping only when service networking and port semantics remain compatible.
- [x] Distinguish application-owned, external, and implicit default networks and volumes.
- [x] Translate absolute and systemd-specifier paths and resolve relative paths only with an explicit caller-provided project root.
- [x] Generate a complete compatibility report for every first-slice target decision.
- [x] Validate Podman 5.4.0 through the finite current QuadletLens catalogue ceiling.

Current dependency gates:

- ComposeLens 0.1.3 is published and consumed through its native merged-project view. The adapter
  regression imports complete unquoted short-volume scalars such as `./data:/data:Z,ro` and retains
  all contributing multi-file source origins.
- QuadletLens 0.1.3 is published and consumed through its validated document builder and finite
  capability catalogue. The exporter generates container, optional explicitly selected pod,
  application-owned network, and application-owned volume units; retains external resources as
  direct references; validates its native dependency graph; and redacts generated contents from
  `Debug` output. Single-pod grouping requires compatible declarations and explicit approximation
  authorization; incompatible requests fail without fallback.
- Public-facade golden scenarios prove multi-file Compose input handling, explicit profile selection,
  strict/partial/approximate authorization, exact separate-container and pod-grouped file bytes,
  stable diagnostics, dependency graphs, and provenance.
- Relative bind paths resolve lexically when the caller supplies their absolute Compose project
  root; otherwise they remain explicit losses. Tilde/home, Windows, and other host-specific forms
  resolve only through an exact caller-supplied source-to-target mapping. Per-network aliases and
  quoting-dependent values remain documented target-side losses.

## Additive explicit host mappings — completed

- [x] Add neutral ordered host mappings with explicit `host-gateway` classification.
- [x] Prevent the Quadlet exporter from silently omitting unrecognized host-mapping address forms.
- [x] Consume released ComposeLens 0.1.3 and map merged Compose `extra_hosts` with provenance.
- [x] Consume released QuadletLens 0.1.3 and emit capability-checked container or pod `AddHost`
  entries.
- [x] Add adapter and end-to-end cases that collectively cover sequence/mapping Compose syntax,
  IPv4, bracketed/unbracketed IPv6, arbitrary `name:host-gateway`, and
  `host.docker.internal:host-gateway`.

Separate containers retain their service mappings. Single-pod grouping requires identical ordered
mappings on every service and emits them once on the generated pod; differences reject the
explicit grouping request. BoxFerry consumes both Lens libraries from crates.io without sibling
paths or Git dependencies.

## Coverage audit and additive health checks — completed

- [x] Document separate syntax, native-model, merged-view, neutral-model, target, and end-to-end
  coverage stages across all three repositories.
- [x] Prepare ComposeLens 0.1.3 with source-aware merged health checks and field-level provenance.
- [x] Prepare QuadletLens 0.1.3 with regular health timing keys and full Podman 5.4.0-through-6.0.2
  generator evidence.
- [x] Publish ComposeLens 0.1.3 and QuadletLens 0.1.3 through their protected release workflows.
- [x] Add the neutral health-check model and Compose-to-Quadlet adapters without treating Compose
  `start_interval` as Podman startup-healthcheck configuration.
- [x] Add strict, partial, disabled, malformed, and golden end-to-end scenarios.

BoxFerry now retains field-level health-check provenance, preserves `CMD` versus `CMD-SHELL`, emits
capability-checked `HealthCmd`, `HealthInterval`, `HealthTimeout`, `HealthRetries`, and
`HealthStartPeriod` entries across the verified Podman range, and maps explicit disable intent to
`HealthCmd=none`. `start_interval` remains in the neutral model and blocks exact output with an
actionable unsupported outcome because it is not equivalent to Podman's startup-healthcheck
feature family.

## Dependency ordering and readiness — completed

- [x] Prepare ComposeLens 0.1.4 with effective short/long `depends_on` entries and nested
  condition/restart/required provenance.
- [x] Prepare QuadletLens 0.1.4 with `Notify=healthy`, typed `Requires`/`Wants`/`After`, capability
  records, and full Podman 5.4.0-through-6.0.2 generator evidence.
- [x] Publish ComposeLens 0.1.4 and QuadletLens 0.1.4 through their protected release workflows.
- [x] Consume both released crates without sibling path or Git dependencies.
- [x] Add ordered neutral service dependency edges with field-level provenance.
- [x] Map required and optional `service_started` dependencies to separately capability-checked
  systemd activation and ordering directives.
- [x] Map `service_healthy` only when target readiness and source health intent can be established.
- [x] Report Compose-managed restart propagation and successful-completion conditions explicitly
  until an equivalent target contract is proven.
- [x] Add missing-target, cycle, strict/partial, separate-container, pod-grouped, and golden
  end-to-end tests.

## Execution identity and container context — completed

- [x] Prepare ComposeLens 0.1.5 with effective `user`, `userns_mode`, `group_add`, `working_dir`,
  and `read_only` values plus multi-file provenance.
- [x] Prepare QuadletLens 0.1.5 with typed `User`, `Group`, `UserNS`, repeatable `GroupAdd`,
  `WorkingDir`, and `ReadOnly` keys plus full Podman 5.4.0-through-6.0.2 generator evidence.
- [x] Add neutral primary user/group, user namespace, ordered supplementary groups, working
  directory, and read-only-root-filesystem intent with provenance and redaction.
- [x] Publish ComposeLens 0.1.5 and QuadletLens 0.1.5 through their protected release workflows.
- [x] Consume both released crates without sibling path or Git dependencies.
- [x] Map Compose identity/context values into the neutral model with source and sensitivity
  provenance.
- [x] Map the supported neutral subset to separately capability-checked Quadlet keys.
- [x] Preserve numeric and named identity/group spellings in the neutral model; map numeric primary
  GIDs and named or numeric supplementary groups exactly; define explicit losses for named primary
  groups, unresolved values, pod/user-namespace conflicts, unsafe target values, and unsupported
  encodings; and emit both explicit true and false read-only choices.
- [x] Add strict/partial, multi-file, sensitive-value, separate-container, pod-grouped, and golden
  end-to-end tests.

Separate-container output maps the complete identity slice exactly when its values fit the
reviewed one-line encoding subset. Because Podman uses the pod namespace for grouped containers,
the completed 0.1.6 integration moves identical explicit namespace intent to pod `UserNS=` and
rejects mixed or conflicting intent. It also implements the repeatable container `Secret=`
boundary for pre-existing external Podman secrets.

## Pod-grouped user namespace follow-up — completed

- [x] Prepare QuadletLens 0.1.6 with singleton pod `UserNS`, capability evidence, and the complete
  Podman 5.4.0-through-6.0.2 generator matrix.
- [x] Publish QuadletLens 0.1.6 through its protected trusted-publishing workflow.
- [x] Consume released QuadletLens 0.1.6 without a sibling path or Git dependency.
- [x] Move one identical explicit namespace choice from every grouped service to pod `UserNS`.
- [x] Reject mixed absent/explicit or conflicting grouped namespace intent instead of choosing a
  value by service order.
- [x] Extend grouped golden output and strict/partial/invalid policy tests.

## Config and secret grants — completed

- [x] Prepare ComposeLens 0.1.6 with effective short/long service config and secret grants,
  field-level provenance, unique-by-target merge behavior, and malformed-form recovery.
- [x] Extend the QuadletLens 0.1.6 candidate with repeatable container `Secret`, capability
  evidence, and mounted-file/environment option fixtures across Podman 5.4.0 through 6.0.2.
- [x] Publish ComposeLens 0.1.6 and QuadletLens 0.1.6 through their protected trusted-publishing
  workflows.
- [x] Consume both released crates without sibling path or Git dependencies.
- [x] Add neutral config/secret definitions and ordered service grants without carrying Lens
  types into the model.
- [x] Map externally managed Podman secrets and their target/UID/GID/mode options exactly when
  source and target defaults remain equivalent.
- [x] Report application-owned secret creation, file/environment materialization, config
  lifecycle, and unsupported ownership/default differences as explicit manual actions.
- [x] Add short/long, multi-file, custom-name, sensitive-value, strict/partial, and golden tests.

## First usable command-line path — completed

- [x] Enable the `cli`, `compose`, and `quadlet` features by default while preserving
  no-default and individual-adapter library builds.
- [x] Convert explicitly ordered Compose files through the same public importer, engine, and
  exporter APIs available to embedded callers.
- [x] Expose project naming, profile input, Podman range, grouping, loss policy, and output
  location explicitly, with documented conservative defaults and no ambient runtime discovery.
- [x] Refuse existing output directories and block unauthorized output before performing writes.
- [x] Add black-box tests for exact reviewed bytes, write safety, and loss-policy exit status.

## Phase 3: runtime migration — in progress

- [x] Distinguish runtime observations, authored sources, defaults, overrides, and conversion
  decisions in neutral provenance.
- [x] Establish a pure shared observation/reconstruction crate without Docker or Podman response
  types, daemon access, command execution, or ambient discovery.
- [x] Decode caller-supplied Podman container, pod, network, volume, and image inspect arrays for
  the finite 5.4.0-through-6.0.2 range without daemon or command access.
- [x] Report malformed input, meaningful unmodeled configuration, missing relationships, and
  out-of-range Podman versions through stable native diagnostics.
- [x] Acquire explicitly selected existing Podman resources through a replaceable, shell-free,
  read-only CLI boundary without running commands for empty resource families.
- [x] Expand a container/pod selection to related images, networks, and named volumes under an
  explicit finite caller policy without ambient resource enumeration.
- [x] Do not auto-follow Podman's mixed namespace/generic container dependency IDs; retain their
  unsupported diagnostic instead of importing infra containers or guessing portable intent.
- [x] Add isolated, digest-pinned runtime conformance for every supported minor line with an
  official immutable local-runtime image: 5.4.0, 5.5.2, 5.6.2, 5.7.1, and 5.8.2.
- [x] Add an opt-in exact-current-patch lane for an explicitly selected installed Podman 6.0.2;
  create only uniquely named test resources and remove them after inspection.
- [ ] Add reproducible scheduled-image conformance for the exact current patch. The official
  stable registry had no immutable Podman 6.0.2 local-runtime image on 2026-08-04.
- [x] Decode explicit Docker container, image, network, and volume inspect arrays across the finite
  Engine API 1.40-through-1.55 range, with closed explicit-endpoint acquisition and bounded
  container-resource expansion.
- [x] Add isolated, digest-pinned Docker Engine 29.7.1 conformance with forced API 1.40 and 1.55
  responses, no host runtime socket, and an explicit historical-19.03 evidence limitation.
- [x] Treat runtime `CreateCommand` data as optional, sensitive provenance rather than a required
  source of truth.
- [x] Reconstruct command and environment overrides by comparing effective container and image
  observations under an explicit caller-selected policy.
- [x] Extend container/image comparison to non-empty `user[:group]` and working-directory values,
  split retained identities into neutral user/group fields, and preserve explicit container
  read-only-root state directly.
- [x] Extend the effective-state model with regular health checks, protected command forms,
  timing/retry values, Docker API-aware start intervals, image-default comparison, and explicit
  separation from Podman's startup-healthcheck family.
- [x] Extend effective state with a reviewed container restart-policy slice: decode Docker and
  Podman policy objects, preserve runtime provenance, emit exact `Restart=no`, approximate
  unbounded systemd policies explicitly, and never widen finite retry limits silently.
- [x] Extend effective state with protected Docker/Podman container and image label maps,
  deterministic image-default comparison, reserved Compose-provider metadata diagnostics, and
  explicit target-adapter losses.
- [x] Import and generate typed service labels through ComposeLens 0.1.8 and QuadletLens 0.1.7,
  retaining mapping/list scalar behavior, multi-file provenance, protected values, empty values,
  systemd quoting, literal-specifier escaping, and reserved-provider diagnostics in adapter and
  Compose-to-Quadlet golden tests.
- [x] Preserve explicit service runtime names from Compose and runtime observations, keep them
  distinct from neutral service identifiers, and emit validated Compose `container_name` or
  capability-checked Quadlet `ContainerName=` through ComposeLens 0.1.10 and QuadletLens 0.1.9.
- [ ] Define resource-label ownership separately for networks, volumes, pods, and build/image
  artifacts before adding neutral resource-label fields or native generation.
- [ ] Extend the broader effective-state model with reviewed security slices as their native and
  neutral semantics are defined.
- [x] Preserve multiple network aliases plus network, volume, mount, and container relationships
  in observations and the supported neutral subset.
- [x] Add an ordered neutral service-group relationship with member provenance; consistent Podman
  membership enters the application graph without inferring namespace or lifecycle semantics.
- [x] Reconstruct the supported application subset with application-level and field-level
  uncertainty, provenance, redaction, and policy-controlled outcomes.
- [x] Generate first-slice Quadlet definitions from caller-supplied observations through the
  public importer, engine, loss policy, and exporter path.
- [x] Apply exact-name, provenance-bearing network, volume, and service-group lifecycle
  resolutions and preserve one complete application-owned group as a named Quadlet pod.
- [x] Generate deterministic, parse-back-validated Compose definitions from observations through
  an exact provider target, optional exact backend runtime, compatibility diagnostics, explicit
  loss authorization, resource-name preservation, and the public facade.

## Additive explicit container names — completed

- [x] Consume released ComposeLens 0.1.10 and QuadletLens 0.1.9 from crates.io.
- [x] Add a provenance-bearing neutral service runtime name distinct from its service key.
- [x] Import effective multi-file Compose `container_name` values without losing replacement
  provenance.
- [x] Generate validated Compose `container_name` and capability-checked Quadlet
  `ContainerName=` values.
- [x] Preserve inspected Docker and Podman container names during runtime reconstruction.
- [x] Reject names outside each target runtime's grammar instead of silently reverting to a
  provider-generated name.
- [x] Add neutral-model, adapter, runtime, invalid-value, and golden end-to-end tests.

## Additive authored Compose restart policies — completed

- [x] Consume released ComposeLens 0.1.11 from crates.io.
- [x] Import `no`, `always`, unbounded or positive retry-limited `on-failure`, and
  `unless-stopped` into the distinct neutral container restart policy with complete merge
  provenance.
- [x] Reject unresolved expressions on cross-format routes, explicit zero retry limits, and retry
  counts outside the neutral `u64` range without erasing their services; retain valid expressions
  on Compose-to-Compose native canonicalization.
- [x] Generate every neutral restart policy exactly back to Compose through ComposeLens's
  parse-back-validated document boundary.
- [x] Convert authored Compose `restart: "no"` exactly into Quadlet `[Service] Restart=no` and
  retain explicit approximation/unsupported outcomes for the non-equivalent systemd policies.
- [x] Add offline import/export boundary tests, an end-to-end golden assertion, installed Podman
  6.0.2 generator validation, and a pinned real-world-corpus regression run.
- [x] Promote 73 literal corpus policies from unsupported to exact; keep Mattermost's two
  unresolved `${RESTART_POLICY}` values invalid for cross-format conversion while preserving them
  in Compose-to-Compose output.

## Explicit Compose interpolation inputs — completed

- [x] Keep interpolation disabled by default and preserve the no-ambient-access CLI contract.
- [x] Add opt-in per-file ComposeLens interpolation before merge through `--interpolate`.
- [x] Accept repeatable non-sensitive `--env NAME=VALUE` values without reading the process
  environment.
- [x] Read only individually authorized `--env NAME` values and mark them
  sensitive before interpolation.
- [x] Reject invalid names and missing/non-Unicode authorized values without output creation;
  explicit interpolation sources apply in documented order, with later values overriding earlier
  values.
- [x] Add an authored fixture and black-box tests proving defaults ignore unauthorized ambient
  values, sensitive named input reaches exact output, and interpolation never occurs without
  opt-in.
- [x] Prepare ComposeLens 0.1.12's reviewed native service `env_file` declarations, including
  short/long forms, merge order, nested provenance, required flags, raw-format classification,
  malformed recovery, and sensitive interpolation redaction without file I/O.
- [x] Release ComposeLens 0.1.12, update the crates.io dependency, and add a neutral
  environment-file declaration model before mapping supported semantics to Quadlet
  `EnvironmentFile=`.
- [x] Preserve short/long declaration syntax, order, `required`, `format`, sensitivity, and nested
  provenance without reading referenced files; resolve safe relative paths only from an explicit
  project root, emit required files as capability-checked `EnvironmentFile=`, classify parser
  parity as `BFQ0010` approximate, and report optional files or unsafe paths explicitly.
- [x] Release and consume ComposeLens 0.1.13, then replace the tested Compose-generation loss with
  ordered short/long `env_file` output and parse-back validation.
- [ ] Define separate caller-authorized project `.env` and environment-file content processing;
  declaration conversion must not imply file reads or silently merge file contents.

## Phase 4: Kubernetes — open

- [ ] Read and write core Kubernetes resources.
- [ ] Define workload-controller selection and service/storage mappings.
- [ ] Add Kubernetes version and API capability checks.
- [ ] Validate on disposable clusters.

## Phase 5: Helm and Kustomize — open

- [ ] Consume rendered Helm and Kustomize input.
- [ ] Generate maintainable Kustomize bases and overlays where possible.
- [ ] Investigate Helm chart generation as a separate policy-driven backend.

## Phase 6: ecosystem hardening — open

- [x] Establish the first pinned, licensed real-world Compose corpus and an opt-in ingestion test.
- [x] Use QuadletLens's first pinned, licensed real-world Quadlet corpus as target-format evidence,
  while keeping its parser result distinct from BoxFerry's end-to-end conversion coverage.
- [ ] Promote corpus-derived gaps into minimal offline conversion and golden fixtures as features
  are implemented.
- [ ] Stabilize selected library APIs.
- [x] Add ordered packaging, trusted publishing, checksums, and crate attestations.
- [ ] Publish compatibility matrices and migration guides.
- [ ] Establish contributor governance and long-term maintenance policy.

## Issue-derived evidence

The dated [Podlet and `compose_spec_rs` issue-corpus review](research/podlet-compose-spec-rs-issues-2026-08-01.md)
maps real user reports to these phases and to the owning Lens repositories. Issue state does not
complete a task or establish compatibility; specifications and exact-version tests remain required.
