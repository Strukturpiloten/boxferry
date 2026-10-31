# ADR 0084: Bounded Docker source library adapter

- Status: accepted bounded optional-library scope; validation and release qualification pending
- Date: 2026-10-25
- Supersedes only: [ADR 0032](0032-future-native-lens-boundaries.md) decisions 5, 6, 8 and 9
  insofar as they defer the real Docker source adapter and input diagnostic namespace;
  [ADR 0033](0033-universal-neutral-model-pipeline.md) decision 8 insofar as it defers the
  optional Docker input library; [ADR 0083](0083-bounded-docker-target-consumer-preparation.md)
  decisions 1 and 3 only for their input-library and source-rule deferrals
- Preserves: ADR 0083 as the historical target-only checkpoint, its exporter and consumer contracts;
  [ADR 0078](0078-trusted-docker-consumer-validation.md) exact-pair admission and
  [ADR 0079](0079-native-test-runtime-selection.md) runtime selection

## Context

The target checkpoint and its evidence must not silently become Docker input or CLI acceptance.
The preserved issue-343 BoxFerry research tree contains a real source adapter among unrelated CLI,
application, capture and infrastructure work. Only its BoxFerry-owned semantic source modules and
focused tests are recovered here. No native parser or implementation is copied from DockerLens.
The separately preserved source-daemon-context proposal and other research integration records remain unadopted.

## Decision

1. Add `DockerImporter`, `DockerSource`, `DockerSelection`, `DockerPromotionPolicy`, checked
   resource decisions and public-label declarations to the existing nondefault `docker` library
   feature. The importer consumes DockerLens-decoded observations and implements the ordinary
   importer → neutral `Application` → exporter/loss-policy pipeline, including same-format use.
   It never parses Engine JSON, acquires a source, chooses a target or writes an artifact.
   No neutral types, exporter behavior, default features or CLI routes change.
2. The facade's `acquire_docker_source` requires a caller-named application, explicit Unix endpoint,
   native selector, finite limits and cancellation flag. DockerLens owns closed read-only requests,
   decoding and native version/context evidence. No ambient endpoint or workload discovery is
   inferred. Discovery remains metadata-only; empty inspected selections are not exact application
   intent. Acquisition grants neither promotion nor artifact authorization, peer authentication nor
   an atomic-snapshot guarantee. BoxFerry sends no mutating request and executes no command.
3. Already-decoded sources start with `Unspecified` selection, conservative promotion and empty
   resource/public-label decisions. Configured, effective, runtime-assigned, unavailable, malformed
   and redacted observations remain distinct. Reviewed effective values need explicit promotion
   and carry observation plus conversion-decision provenance; approximation remains non-exact.
   Source reconstruction and target loss policy are separate decisions. Unknown observed daemon
   mode retains its native warning without inventing a target mode or losing otherwise portable
   intent; mode-sensitive fields still require their own prerequisites. Missing or incompatible
   Engine/API evidence and other native findings do not receive that exception.
4. Preserve native identity/correlation and finding severity. Native-finding subjects retain only
   safe caller-local `ResourceRef` indices, never native IDs/names, and distinguish global findings.
   DockerLens `Finding` has no phase: the generic stage is `observation`, not an inferred acquisition
   or decoding claim. Phase-specific attribution requires a future native contract.
   Runtime names and IDs do not prove
   lifecycle ownership. Named volumes, same-host binds, ID-correlated network membership, finite
   bridge settings and aliases retain separate policy switches. Explicit create/external decisions
   require checked kind-qualified identities and independent destination names. A portable preset
   does not authorize host binds, protected environment values or public labels. Native mount
   relabel evidence does not prove host suitability, SELinux enforcement or original authorship.
5. Protected environment values and labels are denied by default. Public labels require exact
   resource/key declarations and bounded inspected values; declarations do not supply replacements.
   Redacted values are never reconstructed as empty. Source consent does not imply downstream
   sensitive-value artifact consent. Diagnostics and errors remain value-free; native evidence
   remains protected. Application identity always comes from the caller, never a native label.
6. Preserve configured host-port requests, ephemeral host-port zero and runtime-assigned allocations
   as distinct states. Unmapped runtime ports, network mode, rootfs/limits, security, DNS, logging,
   tmpfs/binds, configured mounts, volumes-from and devices remain native evidence with structured
   unsupported outcomes, not exact intent by omission. A later field mapping needs its own review.
7. Add source rules `BFD0001` invalid, `BFD0002` unsupported, `BFD0003` reconstructed and `BFD0004`
   approximation, with `RuleId` discriminants `2_000_001`–`2_000_004` and a source-adapter owner.
   Preserve every existing discriminant and `BFD0005`–`BFD0007` target definition/owner. Catalogue
   ordering, uniqueness, lookup and machine-reference equality remain mandatory. `BFR` remains
   deferred; no runtime abstraction or speculative namespace is restored.

## Evidence and consequences

Recovered unit tests cover configured/effective provenance, promotion, privacy, identity conflicts,
invalid/discovery selections, network topology and lifecycle, volumes/binds/relabel limitations,
configured/runtime ports, native findings and loss policy. Offline facade tests use an authored
bounded local socket to check endpoint/selector/limits/cancellation forwarding, observed context,
conservative defaults and separate acquisition/decode failures. These are source-level contracts,
not daemon or application acceptance; authoring tests is not execution evidence.

Focused validation, independent review and the complete final repository gate remain required and
pending until separately scheduled. Exact-pair runtime authorization, CLI integration, broader
route/capture fixtures, application acceptance, published API comparison, packaging and release
qualification remain separate work. No compatibility, main-merge or release claim follows here.

This slice changes no manifest, lockfile, tool/dependency pin, extraction path or native API.
Existing Renovate ownership therefore remains unchanged; later dependency selection must receive
normal policy/integrity and exact-source compatibility review. No dependency override substitutes
for published-package proof.

## Alternatives

An importer stub would falsely advertise source support. Wholesale recovery would adopt unrelated
CLI, application and infrastructure work. Native passthrough would bypass neutral intent and loss
policy. Automatic promotion or inferred resource ownership would turn observed defaults and shared
resources into unauthenticated authored intent. All are rejected.
