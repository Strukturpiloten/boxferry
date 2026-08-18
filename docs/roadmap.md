# Roadmap

This roadmap describes dependency order, not delivery dates. A later phase may be explored early, but stable behavior must be built on completed lower layers.

Cross-repository delivery uses the stable task numbers in the [implementation plan](implementation-plan.md). This roadmap remains the detailed internal phase order for BoxFerry.

## Product contract and current milestone

BoxFerry is an N-to-N conversion system. The current implementation milestone completes the shared
engine and the Compose/Quadlet document matrix. Docker runtime resources, Podman runtime resources,
and Kubernetes remain future sources and targets, but their native behavior belongs to deferred
independent DockerLens, PodmanLens, and KubernetesLens projects. Pairwise routes always compose
through the neutral application model; they are never implemented as unrelated converters.

See [ADR 0032](decisions/0032-future-native-lens-boundaries.md) for the dependency boundary and the
[implementation plan](implementation-plan.md#t9-initial-composequadlet-product-completion) for the
ordered current work.

### Source adapters

- [x] Import Docker Compose into the neutral model for the documented first subset.
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
- [x] Complete the reviewed Quadlet document import subset; unrepresented native values remain
      structured losses until the required Lens API and semantic mapping are available.
- [x] Accept typed Quadlet `.image` and `.build` unit families.
- [x] Report every Quadlet `.kube` and `.artifact` document entry individually as native-only,
      including generic and unknown sections; neutral Kubernetes and artifact semantics remain open.

### Target adapters

- [x] Export the documented neutral subset to Docker Compose or `podman-compose` definitions.
- [x] Export the documented neutral subset to Podman Quadlet definitions.
- [x] Consume released ComposeLens 0.1.13 and export ordered neutral environment-file declarations
      through its new short/long generated-document boundary.

### Route orchestration

- [x] Prove the shared importer → neutral model → exporter path in the public library API.
- [x] Expose all four Compose/Quadlet document routes through the CLI registry, including native
      Compose canonicalization and neutral-model Quadlet canonicalization.
- [x] Add the [document conversion CLI contract](cli-vnext.md) with nested input and output selection.
- [x] Add the [privacy-safe local error-report bundle](error-reports.md) without automatic upload
      or raw input collection.
- [x] Preserve structured `boxferry-quadlet` syntax, typed-model, and document-set diagnostics in
      CLI reports with input aliases, static label detail, and path/secret redaction coverage.

The full 16-route matrix is T8 work blocked on future native Lens libraries. T9 completed and
validated the four currently available Compose/Quadlet document routes on 2026-08-18.

### Initial BoxFerry completion

- [x] Complete the deterministic Compose/Quadlet document matrix using released ComposeLens and
      QuadletLens APIs; every unsupported value remains a structured outcome.
- [x] Record project `.env` parsing/materialization, service `env_file` content
      parsing/materialization, Compose `include` processing, and generated config/secret service
      grants as released-Lens API gaps. BoxFerry does not privately implement them; `--env-file`
      remains explicit Compose interpolation assignments.
- [x] Complete positive, negative, loss-policy, same-format, structured-report, fix-first, and
      deterministic golden coverage for all four Compose/Quadlet routes.
- [x] Review T9 corpus candidates against released Lens APIs and existing regression coverage;
      avoid duplicate fixtures and keep the broader corpus-promotion backlog open.
- [x] Review and stabilize the initial public core, Compose, Quadlet, CLI, diagnostic, and report
      contracts; remove inaccurate or unreleased surfaces instead of adding compatibility shims.
- [x] Pass the complete repository validation and prepare the next BoxFerry release.

### Deferred native-library dependencies

The current milestone does not implement or simulate:

- Docker or Podman deployment plans, runtime executors, or runtime CLI routes;
- Kubernetes resource mappings, `.kube` semantics, or cluster validation;
- native artifact execution;
- native resource-label or broader runtime-security semantics without corresponding Lens evidence;
- Compose or Quadlet behavior that requires an unreleased change to an existing Lens API; or
- any private replacement for a released-Lens API gap.

### High-priority documentation replacement

- [ ] After initial implementation completion, replace the development-era documentation with a
      coherent user, operator, library, architecture, compatibility, troubleshooting, and
      contributor documentation set describing only implemented behavior. T10 is the next task.

The replacement is intentionally not designed further until the initial implementation contract
is fixed.

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

## Additive explicit container names — completed

- [x] Consume released ComposeLens 0.1.10 and QuadletLens 0.1.9 from crates.io.
- [x] Add a provenance-bearing neutral service runtime name distinct from its service key.
- [x] Import effective multi-file Compose `container_name` values without losing replacement
      provenance.
- [x] Generate validated Compose `container_name` and capability-checked Quadlet
      `ContainerName=` values.
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
- [ ] Track project `.env` and environment-file materialization as a released-Lens API gap;
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

- [x] Consume released ComposeLens 0.2.0 and QuadletLens 0.2.0 from crates.io.
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
