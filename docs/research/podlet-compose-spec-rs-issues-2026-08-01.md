# Podlet and `compose_spec_rs` issue-corpus review

- Reviewed: 2026-08-01
- Sources: [`containers/podlet` issues](https://github.com/containers/podlet/issues) and
  [`k9withabone/compose_spec_rs` issues](https://github.com/k9withabone/compose_spec_rs/issues)
- Scope: every open and closed issue visible through the GitHub REST API; pull requests excluded

## Method and limits

The review covered 139 Podlet issues, of which 37 were open and 102 closed, and 20
`compose_spec_rs` issues, of which 9 were open and 11 closed. Counts are a dated snapshot and will
change. Titles and states were checked for every issue; bodies and relevant maintainer comments
were reviewed for every potentially product-relevant report. Closed issues remain useful because
they document real inputs and earlier failure modes.

An issue is evidence that a user encountered or requested something. It is not by itself proof of
the Compose Specification, Quadlet semantics, or runtime behavior. Before a finding becomes a
compatibility claim, implementation rule, or redistributed fixture, the owning project must still:

1. verify the authoritative specification or implementation documentation;
2. reproduce behavior against exact provider/runtime versions where relevant;
3. reduce the report to an independently authored minimal fixture or review upstream licensing;
4. record provenance, environment, and expected diagnostics; and
5. avoid copying implementation source from either project.

## Conclusion

The trackers are highly useful. They do not suggest changing the three-repository architecture.
Instead, they validate its most important boundaries:

- Compose parsing must preserve real inputs that strict models reject.
- Compose processing stages such as interpolation and merge must be explicit.
- Quadlet parsing/rendering needs systemd-aware quoting, paths, repeated keys, and version evidence.
- Conversion must preserve source ownership and topology, not only individual scalar values.
- Partial output is useful only when every loss remains structured, visible, and policy-controlled.
- Runtime reconstruction cannot depend on one optional `CreateCommand` field.

## Highest-priority requirements

### Partial conversion with complete diagnostics

[Podlet #189](https://github.com/containers/podlet/issues/189) asks for usable Quadlets even when
some source keys cannot be converted. [Podlet #143](https://github.com/containers/podlet/issues/143)
and [#206](https://github.com/containers/podlet/issues/206) show the practical motivation: one
unsupported external network, path expression, config, or implementation field can otherwise
prevent all output.

BoxFerry should produce the complete conversion plan even when strict policy prevents rendering.
An explicit partial-output policy may render supported resources, but it must never be named or
implemented as “ignore unsupported.” Every unconverted feature needs its source span, outcome,
reason, affected target resource, and suggested manual action. Strict remains the safe default.

Owner: `boxferry-engine`, with provenance supplied by both Lens adapters.

### Preserve application topology and resource ownership

The following reports are one connected requirement rather than isolated mapping bugs:

- [Podlet #190](https://github.com/containers/podlet/issues/190): Compose's implicit default
  network is part of application isolation and service discovery.
- [Podlet #191](https://github.com/containers/podlet/issues/191): top-level named volumes carry
  labels, names, drivers, and lifecycle intent that an implicit runtime-created volume loses.
- [Podlet #158](https://github.com/containers/podlet/issues/158) and
  [#95](https://github.com/containers/podlet/issues/95): external resource identity differs from
  application-owned resource creation.
- [Podlet #90](https://github.com/containers/podlet/issues/90) and
  [#114](https://github.com/containers/podlet/issues/114): native file references and systemd
  dependencies require target-aware naming.

The neutral model therefore needs explicit ownership and lifecycle on networks and storage, plus
edges from services to resources. A generated `.network` or `.volume` file and a reference to an
already existing resource are different outcomes.

Owner: BoxFerry application graph and mappings; native reference syntax belongs to QuadletLens.

### Do not assume that a Compose project should become one Podman pod

[Podlet #225](https://github.com/containers/podlet/issues/225) asks to retain service-level port
ownership, while [#92](https://github.com/containers/podlet/issues/92) and
[#137](https://github.com/containers/podlet/issues/137) report network differences in generated
multi-container pods. A Podman pod shares a network namespace, whereas Compose normally models
separate service network attachments. Moving published ports to the pod may be operationally
necessary, but it also changes their representation and can conceal which service declared them.

BoxFerry must preserve the declaring service as provenance even when the target port is pod-owned.
Pod grouping should be a target policy decision based on requested semantics. If per-service
network isolation, overlapping ports, or incompatible attachments make one pod unsuitable, the
planner should select separate containers or report a manual/unsupported outcome. It must not emit
container-level publishing inside a shared-network pod merely because a source service owned the
port unless exact runtime evidence establishes that target behavior.

Owner: BoxFerry planner. QuadletLens owns `.pod`/`.container` references and capability checks.

### Treat paths as source- and target-language values

[Podlet #166](https://github.com/containers/podlet/issues/166) demonstrates that literal `~` in a
Quadlet path is not shell-expanded; `%h` is the relevant systemd form. Related reports cover
relative paths ([#52](https://github.com/containers/podlet/issues/52),
[#102](https://github.com/containers/podlet/issues/102), and
[#140](https://github.com/containers/podlet/issues/140)), Compose environment expressions in paths
([#143](https://github.com/containers/podlet/issues/143)), and deliberate systemd-specifier output
([#53](https://github.com/containers/podlet/issues/53)).

ComposeLens should retain the authored path and resolve it only with explicit origin, environment,
and home context. BoxFerry should classify the mapping and select an absolute target path or an
explicit systemd specifier. QuadletLens must preserve `%h` as native syntax and warn about path
forms that the target generator will pass through incorrectly.

### Build an evidence-backed target-version catalogue

Podlet's version tickets and failures around older Podman arguments show why “supports Podman 5” is
not precise enough. Particularly useful reports include
[#45](https://github.com/containers/podlet/issues/45),
[#94](https://github.com/containers/podlet/issues/94),
[#142](https://github.com/containers/podlet/issues/142),
[#162](https://github.com/containers/podlet/issues/162), and the explicit version-gating proposal
in [#200](https://github.com/containers/podlet/issues/200).

QuadletLens should record key/unit introductions, command-argument fallbacks, deprecations,
removals, and known broken patch ranges. BoxFerry must validate the complete
`podmanMinimumVersion`/optional `podmanMaximumVersion` range and report the newest evidence-covered
version when the maximum is omitted.

## Compose input and processing corpus

### Cases already protected by ComposeLens 0.1

| Behavior                              | Issue evidence                                                                                                                                                                                            | Current protection                             |
| ------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------- |
| Image registry ports                  | [`compose_spec_rs` #22](https://github.com/k9withabone/compose_spec_rs/issues/22), [Podlet #91](https://github.com/containers/podlet/issues/91)                                                           | tolerant image parser unit tests               |
| Combined image tag and digest         | [Podlet #101](https://github.com/containers/podlet/issues/101)                                                                                                                                            | typed model plus provider conformance          |
| Bracketed IPv6 short ports            | [`compose_spec_rs` #24](https://github.com/k9withabone/compose_spec_rs/issues/24), [Podlet #96](https://github.com/containers/podlet/issues/96)                                                           | short-port parser regression                   |
| Mixed short/long mounts               | [Podlet #26](https://github.com/containers/podlet/issues/26)                                                                                                                                              | field-specific volume variants                 |
| Long-form ports and retained metadata | [Podlet #207](https://github.com/containers/podlet/issues/207)                                                                                                                                            | typed long-port fields and unknown retention   |
| YAML anchors/fragments                | [`compose_spec_rs` #2](https://github.com/k9withabone/compose_spec_rs/issues/2), [Podlet #58](https://github.com/containers/podlet/issues/58) and [#154](https://github.com/containers/podlet/issues/154) | loss-aware syntax and merge-key tests          |
| Explicit interpolation                | [`compose_spec_rs` #3](https://github.com/k9withabone/compose_spec_rs/issues/3), [Podlet #81](https://github.com/containers/podlet/issues/81)                                                             | caller-owned interpolation stage               |
| Multi-file merge                      | [`compose_spec_rs` #4](https://github.com/k9withabone/compose_spec_rs/issues/4), [Podlet #59](https://github.com/containers/podlet/issues/59)                                                             | provenance-preserving merge stage              |
| Non-string/empty label values         | [Podlet #62](https://github.com/containers/podlet/issues/62) and [#191](https://github.com/containers/podlet/issues/191)                                                                                  | list/map label forms and scalar-kind retention |
| Source reference validation           | [`compose_spec_rs` #18](https://github.com/k9withabone/compose_spec_rs/issues/18)                                                                                                                         | selected-service reference validation          |

These links are additional regression provenance, not proof that ComposeLens reproduces the other
projects' internal behavior.

### ComposeLens backlog candidates

- `extra_hosts`: `=` and `:` delimiters, bracketed/unbracketed IPv6, and implementation-specific
  `host-gateway` from [`compose_spec_rs` #51](https://github.com/k9withabone/compose_spec_rs/issues/51)
  and [Podlet #155](https://github.com/containers/podlet/issues/155).
- User/group values from [`compose_spec_rs` #23](https://github.com/k9withabone/compose_spec_rs/issues/23)
  and [#41](https://github.com/k9withabone/compose_spec_rs/issues/41). The specification is
  insufficiently precise, so raw spelling must remain available.
- Unlimited `ulimits` represented by `-1` from
  [`compose_spec_rs` #31](https://github.com/k9withabone/compose_spec_rs/issues/31) and
  [Podlet #117](https://github.com/containers/podlet/issues/117).
- Dependency conditions and health-check references from
  [`compose_spec_rs` #48](https://github.com/k9withabone/compose_spec_rs/issues/48),
  [Podlet #145](https://github.com/containers/podlet/issues/145), and
  [#164](https://github.com/containers/podlet/issues/164).
- Anonymous container-path-only volumes on Windows from
  [`compose_spec_rs` #38](https://github.com/k9withabone/compose_spec_rs/issues/38) and
  [Podlet #99](https://github.com/containers/podlet/issues/99). Host-platform path APIs must not
  determine whether a Linux container path is absolute.
- Podman/Compose extensions such as `userns_mode`, CDI devices, mount `chown`, and restart
  `max-retries` from [Podlet #31](https://github.com/containers/podlet/issues/31),
  [#107](https://github.com/containers/podlet/issues/107),
  [`compose_spec_rs` #47](https://github.com/k9withabone/compose_spec_rs/issues/47), and
  [`compose_spec_rs` #49](https://github.com/k9withabone/compose_spec_rs/issues/49).
- Build and deploy fields from [Podlet #126](https://github.com/containers/podlet/issues/126),
  [#173](https://github.com/containers/podlet/issues/173),
  [#215](https://github.com/containers/podlet/issues/215), and
  [`compose_spec_rs` #15](https://github.com/k9withabone/compose_spec_rs/issues/15). These need
  field-by-field conversion outcomes rather than rejecting or silently dropping the whole section.

## Quadlet syntax and generator corpus

The most valuable native-output regressions are:

- quote-bearing label values: [Podlet #202](https://github.com/containers/podlet/issues/202);
- multiline environment values: [Podlet #32](https://github.com/containers/podlet/issues/32);
- scalar commands, argument boundaries, and multi-argument entrypoints:
  [#36](https://github.com/containers/podlet/issues/36),
  [#97](https://github.com/containers/podlet/issues/97), and
  [#119](https://github.com/containers/podlet/issues/119);
- security option delimiters: [Podlet #120](https://github.com/containers/podlet/issues/120);
- repeated label entries: [Podlet #216](https://github.com/containers/podlet/issues/216);
- health-check command form and `CMD-SHELL` semantics:
  [Podlet #160](https://github.com/containers/podlet/issues/160);
- restart and boot behavior: [Podlet #153](https://github.com/containers/podlet/issues/153),
  [#163](https://github.com/containers/podlet/issues/163), and
  [#185](https://github.com/containers/podlet/issues/185); and
- native `.pod`, `.volume`, and `.network` relationships:
  [Podlet #184](https://github.com/containers/podlet/issues/184),
  [#191](https://github.com/containers/podlet/issues/191), and
  [#190](https://github.com/containers/podlet/issues/190).

Tests must validate the generated Podman command or runtime effect where syntax acceptance alone
cannot establish correct escaping or semantics.

## Runtime migration corpus

[Podlet #23](https://github.com/containers/podlet/issues/23) establishes user demand for generating
definitions from running containers. [#134](https://github.com/containers/podlet/issues/134) shows
why `CreateCommand` cannot be mandatory: objects created through APIs, Podman Desktop, or
`podman play kube` may omit it even though inspect data contains the effective configuration.
[#98](https://github.com/containers/podlet/issues/98) adds multiple network aliases as a concrete
reconstruction regression.

The Docker and Podman inspectors should:

- treat `CreateCommand` as optional provenance, never the source of truth;
- compare container inspection with image defaults to identify runtime overrides;
- read effective mounts, ports, networks, aliases, environment, health checks, restart policy, and
  security configuration independently;
- distinguish observed state from inferred author intent; and
- redact and sanitize inspect data before any fixture is committed.

## Broader product validation

[Podlet #9](https://github.com/containers/podlet/issues/9) requested Compose-to-Quadlet and
Compose-to-Kubernetes output early in Podlet's history. The newer
[#221](https://github.com/containers/podlet/issues/221) requests Kubernetes Pod YAML as input for
Quadlet generation. Together they support BoxFerry's broader adapter architecture, but they do not
change delivery order: the Compose-to-Quadlet vertical slice remains first, while Kubernetes input
belongs to the Kubernetes phase.

[Podlet #201](https://github.com/containers/podlet/issues/201) requests a reusable library. The
separate ComposeLens and QuadletLens libraries already address this more cleanly than exposing one
converter's internal model.

## Candidate real-world projects

Issue reports identify useful future corpus candidates:

- Dependency-Track for `deploy` and a larger application ([Podlet #215](https://github.com/containers/podlet/issues/215));
- Coop Cloud recipes for non-standard fields, configs, and tolerant partial conversion ([#206](https://github.com/containers/podlet/issues/206));
- Frigate for mixed mount forms ([#26](https://github.com/containers/podlet/issues/26));
- Invidious for multiline environment values ([#32](https://github.com/containers/podlet/issues/32));
- Docker Awesome Compose Angular for anonymous container-path volumes
  ([`compose_spec_rs` #38](https://github.com/k9withabone/compose_spec_rs/issues/38)); and
- Immich for combined tag-and-digest image references ([Podlet #101](https://github.com/containers/podlet/issues/101)).

These are leads only. Admission still requires an immutable revision, redistribution review,
secret review, minimal scope, and a stated regression purpose. A project should not be imported
merely because an issue links to it.

## Findings not added to the product backlog

The complete review also contained release cadence, repository transfer, security-policy wording,
icons, Discord channels, installation help, container-image packaging, architecture builds, CI
failures, duplicated reports, and questions already answered by project documentation. Those may
matter to project operations, but they do not define Compose, Quadlet, BoxFerry conversion, or
runtime-inspection behavior and were therefore not converted into product requirements.

Issue state was also not used as a priority signal. A closed parser bug remains a valuable
regression; an open feature request remains unproven until the owning project's evidence process
confirms its semantics.

## Refresh procedure

1. Query both repository issue endpoints with `state=all`, following every page.
2. Exclude objects containing the GitHub REST `pull_request` field.
3. Record open/closed issue counts and the review date.
4. Read new issue bodies and relevant maintainer resolution comments.
5. Update only requirements whose evidence or priority materially changed.
6. Never mark a compatibility claim supported solely because an upstream issue was closed.
