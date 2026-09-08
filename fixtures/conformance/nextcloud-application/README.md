# Nextcloud application conformance

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
