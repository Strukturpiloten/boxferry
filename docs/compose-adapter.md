# Compose exporter

`boxferry-compose` maps a neutral `Application` into deterministic Compose YAML through
ComposeLens 0.2.0. The `boxferry` facade exposes `ComposeExporter`, `ComposeRuntime`,
`DOCKER_COMPOSE_TARGET`, and `PODMAN_COMPOSE_TARGET` through the additive `compose` feature.

## Target selection

The provider-neutral target uses the rolling Compose Specification compatibility profile:

```rust
use boxferry::{
    COMPOSE_SPECIFICATION_PROFILE_REVISION, COMPOSE_SPECIFICATION_TARGET, TargetProfile,
};

let target = TargetProfile::new(
    COMPOSE_SPECIFICATION_TARGET,
    COMPOSE_SPECIFICATION_PROFILE_REVISION,
    Some(COMPOSE_SPECIFICATION_PROFILE_REVISION),
)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Its revision is an internal BoxFerry token, not a Compose Specification release version or a
historical-consumer compatibility guarantee. It accepts no backend runtime. The generic
Quadlet-to-Compose CLI route uses this target and reports it as `rolling`.

Embedded callers may instead identify one exact provider release:

```rust
use boxferry::{
    ComposeExporter, ComposeRuntime, DOCKER_COMPOSE_TARGET, PlatformVersion, TargetProfile,
};

let provider = PlatformVersion::new(5, 3, 1);
let target = TargetProfile::new(DOCKER_COMPOSE_TARGET, provider, Some(provider))?;
let exporter = ComposeExporter::new()?.with_runtime(ComposeRuntime::DockerEngine(
    PlatformVersion::new(29, 7, 1),
));
# Ok::<(), Box<dyn std::error::Error>>(())
```

`docker-compose` identifies Docker Compose. `podman-compose` identifies the independent
`containers/podman-compose` provider. `podman compose` is rejected because Podman documents it as
a wrapper around an external provider. The provider version is stored in `TargetProfile`; an
optional exact Docker Engine or Podman backend is attached separately to the exporter. No local
executable or environment value is inspected.

Open-ended or multi-version provider ranges are invalid in this adapter. ComposeLens evidence is
classified for an exact provider/runtime context, so collapsing a range to one version would make
an unsupported compatibility claim. The specification target instead requires exactly its
documented internal profile revision.

## Generated subset

The first slice generates:

- the project name and ordered services;
- optional explicit runtime container names, kept distinct from service keys;
- image references, including tolerant `name:tag@digest` spellings;
- exec, shell, and explicit empty commands;
- ordered short/long environment-file declarations, including explicit `required` and `raw`
  options;
- literal and host-resolved environment entries;
- ordered service-label mappings with empty and protected values;
- combined `user[:group]`, `userns_mode`, supplementary groups, working directory, and read-only
  root intent;
- container restart policies: `no`, `always`, unbounded or finite `on-failure`, and
  `unless-stopped`;
- ordered `extra_hosts`, including the literal `host-gateway` token;
- TCP, UDP, and syntax-preserved SCTP ports;
- named, bind, and anonymous mounts, with deliberate short syntax for SELinux relabeling;
- ordered network attachments and aliases; and
- application-owned or external top-level networks and volumes; and
- application-owned file-backed top-level configs and secrets.

The importer preserves unresolved image spellings in the neutral model and emits source-side
`BFC0105` evidence containing the Compose variable name and service-image subject. Target adapters
remain source-format neutral: for example, the Quadlet exporter independently emits `BFQ0014` when
that preserved value cannot become a native image line. The shared subject correlates the findings
without teaching the Quadlet adapter Compose interpolation syntax.

ComposeLens selects native short/long forms, renders canonical two-space/LF YAML, reparses its own
bytes through the syntax and typed-model layers, and returns `GeneratedComposeDocument`. Sensitive
command, environment-file path, environment, service-label, identity, and context values cause the
complete generated document to redact its `Debug` output. Deployable text remains available only
through `text()`.

Labels in the reserved `com.docker.compose.*` provider namespace remain explicit `BFC0007`
outcomes and are omitted from generated YAML. They are provider-created runtime evidence, not
safe application metadata. Image-build labels, annotations, `label_file`, and resource labels have
separate ownership semantics and remain outside this service-label slice.

The importer also declares one image-build resource for every Compose service `build` declaration
before attaching the service reference. It retains all 25 source leaves, syntax, provenance, and
protected values. Only the reviewed overlaps become target settings: the service image is the
primary tag, followed by explicit tags; `dockerfile`, `target`, explicit mapping or literal
`NAME=VALUE` arguments, and build labels are candidates for Quadlet output. All other leaves stay
policy-controlled source-only losses rather than becoming synthesized Podman arguments.

## Compatibility and loss policy

Compatibility-sensitive fields are evaluated against ComposeLens's selected profile: the rolling
specification profile, or the finite rules for an exact provider and optional runtime. Supported
fields remain exact. Implementation-specific or
deprecated behavior becomes an approximate `BFC0009` outcome. Unsupported or unknown behavior
becomes an unsupported `BFC0009` outcome. The latter requires `LossPolicy::AllowPartial` even when
the generated syntax can retain the value.

Current compatibility-sensitive constructs are tag-plus-digest images, `host-gateway`, Podman
user-namespace values, and short-form SELinux relabeling. SCTP syntax is generated but remains an
unsupported outcome until the selected provider/runtime pair has reviewed execution evidence.

The following neutral intent remains explicit `BFC0007` partial loss in ComposeLens 0.2.0 output:

- a primary group without a primary user;
- environment values that must be absent;
- unknown protocols or future neutral enum variants;
- health checks and service dependencies;
- config/secret service grants, runtime names, external or uncertain ownership, and non-file
  material; and
- structural service groups, because Compose cannot preserve Podman pod/shared-namespace
  semantics.

Generation errors and invalid target profiles use `BFC0008` and `BFC0006` respectively and never
produce an authorizable candidate.

Top-level config and secret output uses ComposeLens's file-only generated definitions. Paths retain
provenance and sensitivity without file I/O; invalid, deferred, multiline, or NUL-bearing paths
fail generation. BoxFerry never turns inline or environment material into a file, and it never
emits a service grant until the native generated boundary can retain its complete contract.

Compose's obsolete `external: {name: ...}` resource syntax is rejected at import. Use
`external: true` together with the top-level `name`; BoxFerry does not retain an obsolete mapping
as a neutral runtime name.

## Restart-policy boundary

The importer maps authored Compose service `restart` values into the container-level neutral
`RestartPolicy`; this remains separate from long-form `depends_on.restart` and the Deploy
Specification's `restart_policy`. `no`, `always`, unbounded or positive retry-limited
`on-failure`, and `unless-stopped` retain complete merge provenance. The exporter emits every
neutral variant exactly through ComposeLens's parse-back-validated generator.

For cross-format import, an unresolved expression, `on-failure:0`, or a retry count outside the
neutral `u64` range is a field-specific `BFC0005` invalid outcome. Compose-to-Compose native
canonicalization instead retains a valid unresolved expression and its default because no neutral
restart-policy interpretation is required. BoxFerry does not reinterpret an explicitly authored
zero as an omitted retry limit. Runtime API decoders may independently interpret a native zero
counter as the provider's absent/default representation because that is runtime observation, not
authored Compose syntax.

Explicit runtime names use Compose's documented portable container-name grammar. A value outside
that grammar is a `BFC0008` invalid generation outcome; BoxFerry does not silently fall back to a
provider-generated name.

## Runtime observations and resource names

Runtime reconstruction feeds the same neutral application into this exporter. A runtime-observed
container gets an explicit `container_name`, while its neutral service key remains independently
available for application relationships. A runtime-observed network or volume gets an explicit
top-level Compose `name` so project scoping cannot change the
reviewed platform resource name. Application/external lifecycle selected through
`RuntimeResolutions` is retained. Uncertain or implicit ownership is emitted conservatively as an
external reference with `BFC0007`; only `AllowPartial` releases that candidate.

The public runtime-to-Compose test covers the complete observation, importer, engine, loss-policy,
exporter, parse-back validation, deterministic output, provenance, and redaction path.
