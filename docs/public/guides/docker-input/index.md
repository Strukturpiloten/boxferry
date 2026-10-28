# Docker input and reviewable output

Docker is Unreleased development functionality. The development CLI registers all 16 routes
between Compose, Docker, Podman and Quadlet. An installed release may expose fewer. Compatible
published Lens dependencies, ordinary API/package checks and complete gates remain prerequisites;
source integration is not native consumer or application-migration acceptance. Linux is supported;
macOS client behavior remains unverified. BoxFerry performs no host setup or runtime mutation.

## Read-only source selection

DockerLens owns native acquisition and decoding. The CLI uses a local Unix socket, selected by
`--docker-socket PATH` or conventional lookup: `/run/user/<current-uid>/docker.sock` first, then
`/var/run/docker.sock`. Lookup is finite filesystem selection, not a connection-only preflight.
After an endpoint is selected, failure does not retry another. The CLI does not consult remote
endpoint environment settings, infer a Docker context, shell out to Docker or start a daemon.
Acquisition sends only bounded read-only GET requests. Socket access is not peer authentication
and separate observations are not an atomic snapshot.

Select one family of roots:

| Option                          | Selection                                                |
| ------------------------------- | -------------------------------------------------------- |
| `--docker-container NAME`       | Exact container names; repeatable.                       |
| `--docker-container-id FULL_ID` | Exact full 64-character container IDs; repeatable.       |
| `--docker-network-id FULL_ID`   | Exact standalone network IDs; repeatable.                |
| `--docker-volume NAME`          | Exact standalone named volumes; repeatable.              |
| `--docker-name-prefix PREFIX`   | One literal container-name prefix from bounded metadata. |
| `--docker-label KEY[=VALUE]`    | One literal label filter from bounded metadata.          |
| `--docker-all-containers`       | Explicitly select every container in bounded inventory.  |

Families cannot be mixed. Empty/no-match selectors, conflicting identities and malformed metadata
fail closed. Without a selector, bounded metadata must identify a sole container or one complete,
consistent application group. Empty, mixed, ambiguous, conflicting or one-off discovery is refused;
there is no interactive Docker chooser. Labels can correlate a group but never supply the neutral
application name. `--application-name` defaults to `docker-import`.

Each acquisition call permits at most 128 requests, 32 selected resources, 64 expansions, 2 MiB
per response, 16 MiB total and 15 seconds. Default discovery followed by inspection uses two
separately bounded calls; 15 seconds is not a whole-invocation deadline. Limit, cancellation,
transport or decode failures remain failures and cannot authorize partial inventory as a complete
application.

These help examples are offline: they do not contact a host daemon.

<!-- boxferry-example: docker-input-help -->

```console
boxferry convert docker compose --help
```

## Observation and portable intent

The CLI defaults to `--docker-import-policy portable`, reconstructing a reviewed current-state
subset with source evidence and conversion-decision provenance. `conservative` retains effective
observations as evidence. Library sources default to conservative promotion and unspecified
selection; the portable preset must be selected explicitly by an embedded caller.

Configured, effective, runtime-assigned, unavailable, malformed and redacted values stay distinct.
Portable reconstruction can cover reviewed image/process/health/restart/configured-port settings,
uniquely correlated named-volume attachments, and bounded ID-correlated network membership,
aliases and inspected bridge settings. Reconstructed topology can remain approximate with uncertain
ownership. Observed host-port allocations do not become configured requests. Native evidence is
retained for unmapped settings with structured findings, never silently presented as exact intent.
Unknown observed daemon mode stays evidence without inventing a target mode.

Reconstruction does not prove original authorship, grant target capability, authorize loss or
establish ownership. `--loss-policy exact` remains the default. `approximate` permits documented
semantic differences; `partial` additionally permits explicitly reported unsupported omissions.
Invalid intent always blocks. Review findings per field; a portable preset does not promise an
exact migration of every inspected workload.

## Independent decisions and privacy

| Control                                                | Authorization                                                                                                  |
| ------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------- |
| `--promote-docker-protected-environment-values`        | Protected source environment literals may enter neutral intent.                                                |
| `--environment-values include`                         | Literal environment values may enter supported generated artifacts; default is `withhold`.                     |
| `--promote-docker-same-host-bind-mounts`               | Reviewed effective binds may retain the same absolute host path.                                               |
| `--docker-public-label KIND:SOURCE=KEY`                | One selected observed label may become ordinary public metadata; repeat exact declarations.                    |
| `--docker-resource-decision KIND:SOURCE=ACTION:TARGET` | A checked selected source resource maps to a target identity with `create` or `external`; repeat per resource. |

Source environment promotion and artifact inclusion are independent. Docker source-to-target
literal emission requires both where applicable. Environment consent grants no consent for
protected process, health or other values. Public-label declarations are exact, not wildcards or
replacement values; they neither authorize environment values nor decide lifecycle ownership.
Public-label kinds are `container`, `network` and `volume`; containers/networks require full
canonical IDs and volumes require exact names. Resource-decision kinds are `network` and `volume`.
Resource decisions neither promote binds nor infer destination isolation. Review kind-qualified
source identities and destination names before authorizing them. Redacted/unavailable source
values cannot be reconstructed as empty strings.

Bind promotion is a same-host assumption. It does not inspect or copy host data, authorize another
host's paths, verify configured relabels, prove SELinux effects or establish destination suitability.
Create/external decisions do not prove that external resources exist or satisfy constraints.

Diagnostics, JSON reports and support ZIPs stay redacted even with artifact-value inclusion.
No raw Docker snapshot support-bundle option is added. Required incomplete or protected intent
can refuse the whole Docker candidate even with partial loss. Do not assume secrets will simply
be dropped while a complete service plan is emitted.

## Exact output targets

Every Docker output route requires `--docker-target PROFILE`, including Docker-to-Docker. Choose
one of the four [reviewed profiles](../../reference/compatibility/#docker-development-boundaries):
`debian11-20.10.5-rootful`, `debian11-20.10.5-rootless`, `upstream-29.8.1-rootful`, or
`upstream-29.8.1-rootless`. Debian means Engine `20.10.5+dfsg1`, package
`20.10.5+dfsg1-1+deb11u2`; upstream means Engine `29.8.1`. Each also fixes daemon mode and resolves
native API/evidence context. Neither source mode nor development-host context chooses the target.
These are finite reviewed profiles, not compatibility with arbitrary releases in between.

## Inert artifacts and obligations

Docker output contains `docker-plan.json`. When typed service dependency decisions are present,
it additionally contains `docker-dependencies.json` with provenance, conditions,
`native_execution=false` and SHA-256 of the exact native plan bytes. The sidecar is conditional;
there is no fixed Docker command-script or unit filename. A dependency condition is a review/manual
obligation, not an Engine field, an implemented scheduler or executed readiness check.

POST entries inside `docker-plan.json` describe desired requests. BoxFerry never sends them.
Actual source acquisition uses GET only. Review the complete plan with prerequisites and
bind-source, external-resource and destination obligations; a requests-only projection is incomplete.
External isolation expectations preserve absent, false and true as separate states. A configured
relabel or destination-PIDs obligation does not prove enforcement, readiness or suitable host paths.

Output directories must be absent or empty. Docker publication uses owner-only Unix files and
create-new writes; existing entries are preserved. A required dependency sidecar must be published
before the complete plan is exposed. Protected dependency identities and incomplete required intent
refuse publication. `validate` runs the same planning and loss decisions without writing artifacts.

Docker plans are not observed inventories and cannot be reimported as Docker input. Compose and
Quadlet documents are reimportable; Podman output remains `podman.json` plus
`podman-commands.sh`, which BoxFerry also never executes.

The [conversion guides](../) include offline document-to-plan examples and Docker-source route
help. Synthetic socket tests supply authored replies in the development test suite; these guides
do not require a real host daemon in the offline documentation gate. None of this establishes
application acceptance, deployment, data transfer or normal registry/package compatibility.
