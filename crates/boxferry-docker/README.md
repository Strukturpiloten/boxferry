# boxferry-docker

BoxFerry's DockerLens semantic adapter imports decoded Docker observations into neutral
`Application` intent and exports neutral intent as inert native requests. The Unreleased facade
and CLI use the same adapter for every Docker route, including Docker-to-Docker. Most embedded
callers should use the `docker` feature of `boxferry` with defaults disabled as needed.

`DockerSource` retains configured, effective, runtime-assigned, malformed, unavailable and
redacted evidence. Already decoded sources start with unspecified selection and conservative
`DockerPromotionPolicy`; reviewed portable reconstruction is an explicit choice. Effective
promotion carries observation and conversion-decision provenance. It does not establish original
authorship, target loss authorization, resource lifecycle ownership or sensitive-value output
consent. Protected environment promotion, same-host binds, exact public-label declarations and
checked create/external decisions are separate controls. Native identities correlate observations;
they do not silently become public application identity.

`DockerExporter` requires one caller-selected exact reviewed DockerLens target profile. A numeric
version range only filters its Engine release; it cannot establish native build, API, daemon mode
or capability evidence. Invalid references, protected required process/health intent and incomplete
required environment intent can block the complete candidate even with partial loss. Environment
output requires independent `DockerEnvironmentPolicy::IncludeLiteral`; default `Withhold` never
discloses literal values. Environment consent grants no process/health-value consent. Debug output
remains redacted.

`DockerOutput::complete_bytes` retains native requests and bind-source, external-resource and
destination obligations. Dependency decisions are typed, retain provenance and approximate
outcomes, and are not Engine startup/readiness fields. The CLI conditionally writes them in
`docker-dependencies.json` with `native_execution=false` and a SHA-256 binding to `docker-plan.json`.
External-network constraints distinguish absence, false and true; relabelled binds do not prove
host path suitability, SELinux effects or enforcement.

DockerLens owns native protocols, acquisition/decoding, validation, planning, capability evidence
and serialization. This adapter opens no socket, writes no file and executes no request. The
facade offers explicit bounded read-only acquisition. Lens products never depend on BoxFerry.

Acquired mount paths retain explicit literal semantics, including named-volume targets.
Docker output reports inert vendor metadata as unsupported and refuses incomplete services
with deferred expressions or service execution providers. It never evaluates providers or
reinterprets source expressions under literal environment-value consent.

See [ADR 0084](../../docs/decisions/0084-bounded-docker-source-adapter.md),
[ADR 0085](../../docs/decisions/0085-docker-cli-neutral-routes.md) and the
[Docker guide](../../docs/public/guides/docker-input/) for the finite development contract.
Inert fixtures and native Lens results do not qualify BoxFerry application migrations or normal
registry compatibility. The existing published 0.3.0 baseline requires ordinary semver comparison;
no first-release waiver or unverified source-compatibility claim applies. Test-only exact-source
Lens overrides must not enter published manifests or main.

Licensed under Mozilla Public License 2.0.
