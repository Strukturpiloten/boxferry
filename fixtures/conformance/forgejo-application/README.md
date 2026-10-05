# Forgejo application conformance

This harness-owned fixture drives the opt-in `forgejo-application` live profile. It is
independent from the authored offline migration scenario. The profile provisions the same
reviewed topology once with native Podman commands and once with the standalone Docker Compose
provider against an isolated Podman API.

Archive image-ID verification and stable-reference restoration remain nested CLI integrity steps
before API activation. Loaded-image presence readbacks run after activation through the bounded
host client and the explicit owned Unix socket. Network creation requires confirmed absence;
missing sockets, diagnostic output,
and unknown observations abort without an inner-exec fallback. Offline consumer tests do not replace
native verification of remote image queries in both reviewed application cells.
Configuration setup and fixture-copy failures retain their original status and stop API activation,
including when callers invoke preparation conditionally.

All credentials are conspicuously fake public test canaries. The generated SSH private key exists
only below the disposable outer container's `/tmp/boxferry-fixture` directory and is removed with
the per-mode probe state and outer container. Retained host artifacts must still receive human
privacy review before sharing.

The host verifies every immutable image digest, builds one archive, and loads it before starting
the nested API. Nested provisioning uses only local `registry.invalid` tags with `--pull=never`.
Forgejo 16.0.3 is GPL-3.0-or-later, PostgreSQL uses the PostgreSQL license, `alpine/git` is
Apache-2.0, and Docker Compose 5.5.0 is Apache-2.0. BoxFerry redistributes none of them.

The source-only `scripts/lib/forgejo-application-probes.sh` holds the application behavior
contract: health readiness, administrator creation, Git seed/verify, and the exact repository
count query. The Podman harness supplies runtime callbacks and retains port, network, volume,
provisioning, and cleanup assertions. `scripts/test-forgejo-application-probes.sh` checks the
callbacks offline with injected failures; it does not establish a live Docker runtime claim.
