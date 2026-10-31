# Nextcloud application conformance

`scripts/lib/nextcloud-application-probes.sh` owns reusable status, WebDAV payload,
database-row, and cache-activity assertions. Its Podman adapter retains probe
placement, network topology, mounted files, lifecycle, and cleanup. The offline
`scripts/test-nextcloud-application-probes.sh` checks positive and negative cases;
this refactor adds no Docker runtime evidence.

The pure `scripts/lib/docker-nextcloud-input-contract.py` helper checks authored
outcome/selection requests against independently held six-role identities,
excluded proxy/peer IDs, source/recreated generations, explicit profiles, resource
decisions, public-label scopes and three separate consents. Run its offline
controls with `PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-docker-nextcloud-input-contract.py`;
the shared application-probe runner includes them. JSON and in-memory record
budgets, value-free failures and inert imports are part of this test-only seam.
It verifies neither physical custody nor native profile compatibility.

[ADR 0088](../../../docs/decisions/0088-pure-nextcloud-input-contract.md) records
the narrow source recovery. The Redis password in protected command/healthcheck
fields remains an exporter blocker. Environment-value consents do not authorize
those fields, and topology-only output cannot establish a complete executable
Nextcloud plan. No Docker runtime profile, capture integration or admission is
activated by these offline checks.

Archive image-ID verification and stable-reference restoration remain nested CLI integrity steps
before API activation. After activation, loaded-image and owned-network presence checks use the
bounded host client against the explicit owned Unix socket. Only a confirmed absent network permits its original
create operation; missing sockets, diagnostics, and unknown observations fail closed without an
inner-exec fallback. Offline mocks protect this boundary; remote image-query compatibility still
requires the reviewed native application gate.
Configuration setup and fixture-copy failures retain their original status and stop API activation,
including when callers invoke preparation conditionally.

This fixture drives the opt-in `application` profile of the live Podman runner. It is a
repository-authored, production-shaped test topology and contains no deployment data. The
credentials in the harness are public canaries used only inside a disposable nested Podman
target.

The four application images are pulled transiently by exact digest, verified, archived, and
loaded into the nested rootless target before its API starts. Nested Podman never contacts a registry.
Docker Compose is an external test oracle downloaded from the official release asset and
verified against `providers.tsv`; it talks only to the disposable nested Podman socket.

Image and provider license identifiers document the reviewed redistribution basis for test
execution. No image or provider binary is committed or redistributed by this repository.
Redis 8.2 is offered under the upstream AGPLv3, RSALv2, or SSPLv1 tri-license; the harness
chooses only a transient test pull and does not redistribute the image.
Retained logs and artifacts are local diagnostic evidence and require human privacy review before sharing.
