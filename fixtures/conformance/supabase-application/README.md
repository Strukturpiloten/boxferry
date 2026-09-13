# Supabase application conformance

This directory is the independently authored fixture for the opt-in
`supabase-application` live profile. It defines an eleven-service application:
Studio, Kong, Auth, PostgREST, Realtime, Storage, imgproxy, Postgres Meta, Edge
Runtime, PostgreSQL, and Supavisor. It is deliberately separate from
`fixtures/scenarios/real-world-compose-supabase/`, which is upstream-derived
conversion evidence and remains unchanged.

## Evidence state

The catalogues, YAML, JavaScript, SQL, and shell module are reviewable offline. The shared live
runner sources the module and registers a manual `supabase-application` profile for only the
reviewed `podman-6.1-rootless` amd64 cell. The profile dispatches
`run_supabase_application_cell` through the existing deadline, isolation, target-verification,
cleanup, and result machinery and uses the already pinned Docker Compose 5.5.0 tool.

The profile is also an ordered privileged task in the manual exact-revision `pre-release`
migration-readiness tier. This integration is a gate definition, not checked-in live-success
evidence: a claim requires a fresh successful numbered run and evidence record for the exact
BoxFerry revision. Raw live output must never be committed; retained failure artifacts require
human privacy review before they are shared. This directory adds no second runner, workflow, or
captured-native cassette.

[`graph.tsv`](graph.tsv) is the static eleven-service behavior/ownership
contract. Its schema-2 rows fix each image identity, exact network set,
dependency set, mount destination/mode set, and runtime proof. Catalogue
validation rejects any unreviewed row or relationship. [`routes.tsv`](routes.tsv)
records one expectation for all nine route families and marks every live
observation unperformed until the profile runs.

The registered offline schema-1 scenario at
[`../../scenarios/supabase-application/`](../../scenarios/supabase-application/)
uses independently authored Compose, Quadlet, and Podman-cassette inputs. Its
exact diagnostic and loss contracts use sidecars for non-empty evidence and
explicit empty expectations, covering all nine importer/exporter routes. This
offline evidence remains independent of the manual live gate and cannot satisfy it.

[`success-contract.jq`](success-contract.jq) builds an exact diagnostic tuple
multiset for every successful live route and selection. Each tuple fixes the
code, severity, subject, and decision. It includes Podman acquisition findings,
promoted healthchecks, Compose dependency and healthcheck losses, and the
`BFC0009` tag-plus-digest approximation for each selected Compose image. Its
loss-fidelity totals fix the `approximate`, `unsupported`, `invalid`, and
`other` counts, including repeated native-finding occurrences. The silent
`exact` implementation counter has no independent semantic oracle and is
therefore constrained only to a non-negative integer. Catalogue validation
admits a complete positive example while rejecting an unseen `BFP0003` subject
and a duplicate `BFP0007` diagnostic.

## Independent application contract

[`compose.yaml`](compose.yaml), [`kong.yml`](kong.yml),
[`db-init.sql`](db-init.sql), and [`functions/main/index.ts`](functions/main/index.ts)
are repository-authored MPL-2.0 test material. They were written around the
public service interfaces; they are not copies or mechanical translations of
Supabase deployment files. The immutable Supabase stack source in
[`application.tsv`](application.tsv) was used only to choose a mutually
compatible component set and to inspect required integration roles.

Both native Podman CLI and standalone Docker Compose provision the same graph.
The backend network is internal. Only Kong joins the externally owned edge and
publishes port 8000 as loopback port 18000. A separately labelled boundary peer
joins only that edge. PostgreSQL, Storage, and the Deno cache use named volumes;
Storage and imgproxy share the storage volume with read-write/read-only access,
respectively.

The standard-library Node probe performs behavior checks through Kong and
direct private service endpoints:

- create and authenticate a GoTrue user;
- resolve the tenant-scoped Realtime endpoint, await its PostgreSQL subscription acknowledgement,
  and observe inserts over a Phoenix WebSocket;
- insert and read a PostgREST row and observe its Realtime event;
- create a private Storage bucket, upload an object, and retrieve exact bytes;
- invoke the authored Edge Runtime function with distinct seed and verify
  payloads and require their exact SHA-256 checksums;
- require Studio, Postgres Meta, Realtime, and Supavisor HTTP readiness; and
- repeat authentication, Realtime insertion, API reads, storage retrieval, and
  function execution after all eleven containers are recreated without their
  volumes.

Independent SQL assertions count database, Auth, and Storage rows, confirm the
Realtime publication, and require the exercised `pgcrypto` and
`pg_stat_statements` extensions. Container inspection proves unpublished
private services, loopback-only Kong publication, internal-network membership,
the cross-application edge boundary, named-volume consumers and modes, and
application-user write access.

For each provisioner, the module exercises exact-container, partial-storage
label, whole-application label, and all-resource acquisition. Each selection
exports to Compose, Quadlet, and Podman. Generated Compose and Quadlet are each
reimported to all three exporters, covering all nine route families without
executing any BoxFerry-generated deployment artifact. Kong selection retains
exactly eleven application services through its reviewed dependencies, including
Realtime and Supavisor, while excluding only the boundary peer. The Storage label identifies the
Storage root, but PodmanLens preserves the complete evidenced application group: native container
dependencies connect Storage through Kong to all eleven services, and complete Compose ownership
provides the same grouping boundary. Exact, Storage, and application-label selection therefore
retain the same eleven services, both networks, and all three volumes while excluding the unrelated
boundary peer. Only all-resource selection includes that peer. Presence and absence are asserted in every
native artifact form. Each artifact must also project the selected services'
catalog images, normalized networks, named-volume mount destinations and modes,
and target-specific dependency semantics. Unknown extras or bind mounts rejected
by the reviewed loss contract fail the gate. Label selection excludes the peer;
all-resource selection includes it. CLI and Compose provisioners carry the same
Realtime healthcheck
and reviewed dependency graph. Collision refusal, structured report redaction,
and prefix-scoped cleanup are hard assertions.

Podman-origin Compose and Quadlet artifacts carry digest-only platform references accepted by the
Podman target. All nine route families must therefore succeed. Every Podman-target reimport checks
its exact `BFP0007` target-loss tuple multiset, fidelity counts, and generated artifact projection;
the harness never executes those generated deployment artifacts.

All credentials are fixed public test canaries. They are intentionally safe to
place in explicitly authorized deployment artifacts, but must not occur in any
BoxFerry JSON report. The behavior probe keeps access tokens in memory and
prints only a phase success marker.

## Supply-chain and license boundary

[`images.tsv`](images.tsv) pins every image tag to its registry reference
digest. For a multi-architecture image, that digest identifies the index, not
the linux/amd64 child manifest. The separate `linux/amd64` field constrains the
pull and archive lane. Each row also records the reviewed release version, license,
source repository and full revision, build-file path and SHA-256, redistribution
status, and an image-specific inspection caveat. Images are transiently pulled,
verified, passed into the disposable nested engine, and removed; BoxFerry does
not redistribute them. [`providers.tsv`](providers.tsv) applies the same
immutable check to Docker Compose.

The catalogue also records each independently resolved `linux/amd64` child in
`platform-manifest-digest` and `platform-manifest-media-type`. The index reference remains
acquisition authority; the child manifest is the OCI archive and live-runtime identity. The child
values were captured on 2026-09-12 with Skopeo 1.24.0 from
`skopeo inspect --raw docker://<repository>@<index-digest>`, selecting the unique descriptor whose
platform is `linux/amd64`. The live lane records its installed `skopeo --version`, then uses
`skopeo copy --preserve-digests` directly from `docker://<repository>@<index-digest>` to a bounded
OCI archive. It verifies the sole descriptor's child digest, media type, byte size, blob hash, and
exact `tag@child-digest` annotation before loading. These are registry-observed values, not digests
inferred from generated archives. Revalidate both immutable identities before changing a row.

Most registries do not attest a source revision in image labels. A tag, source
commit, and build-file hash therefore document review inputs, not a reproducible
image-to-source proof. Kong and imgproxy provide useful OCI revision labels;
the remaining absence is explicit in the catalogue. Base-image and distribution
package closures also remain outside a single Dockerfile hash.

The Supabase PostgreSQL image is not governed by one license merely because its
repository root uses the PostgreSQL License. It bundles PostgreSQL contrib and
many independently licensed extensions. [`postgres-components.tsv`](postgres-components.tsv)
records the directly exercised components and representative installed Apache,
PostgreSQL, GPL, LGPL, and BSD extension terms. It binds the image build to its
exact reviewed revision, identifies each extension's upstream source, and
candidly marks unobserved component versions and revisions `NOASSERTION`; the
live-unperformed state means runtime-presence checks remain pending. The table
is not a complete SBOM: transitive Alpine/Nix packages and other installed
extensions also remain `NOASSERTION`. This inspection supports the
transient-test-use boundary, not permission to redistribute the compound image.

## Bounded live claim

The module admits only Podman 6.1 rootless on linux/amd64. Before pulling it
requires at least four CPUs, 12 GiB available memory, and 24 GiB free space on
both archive and graph-root filesystems. The eleven-image archive is capped at
5 GiB. The profile is single-cell with concurrency one, and this module wraps
the complete cell in a 90-minute deadline in addition to deadlines on image
pulls, archive operations, Compose operations, readiness waits, and every
BoxFerry conversion.

The readiness task caps process-tree RSS at 14,336 MiB: the 12 GiB application
requirement plus 2 GiB runner/provisioner headroom. Its 20,480 MiB disk-growth
ceiling allows four concurrent cap-sized transient representations during a
cold run—the host image store, packed workload archive, extracted OCI members,
and nested image store—while the 24 GiB preflight leaves 4 GiB beyond measured
growth. These are conservative fail-closed admission limits, not checked-in
successful measurements.

Because no successful live evidence is checked in, the first admitted run may
expose a previously unseen diagnostic subject. Such a subject requires a narrow,
reviewed contract addition; the validator has no catch-all allowance.

The fixture does not claim rootful behavior, other Podman releases,
non-amd64 architectures, production credentials, SMTP/SMS/OAuth/SAML, S3
storage, arbitrary Edge functions, GPU behavior, Logflare/Vector analytics,
Quadlet systemd execution, ComposeLens provider conformance, or execution of
BoxFerry-generated plans. An essential failure in any of the eleven services
fails the profile; no component is silently removed to obtain a partial pass.
