# Forgejo application conformance

This harness-owned fixture drives the opt-in `forgejo-application` live profile. It is
independent from the authored offline migration scenario. The profile provisions the same
reviewed topology once with native Podman commands and once with the standalone Docker Compose
provider against an isolated Podman API.

All credentials are conspicuously fake public test canaries. The generated SSH private key exists
only below the disposable outer container's `/tmp/boxferry-fixture` directory and is removed with
the per-mode probe state and outer container. Retained host artifacts must still receive human
privacy review before sharing.

The host verifies every immutable image digest, builds one archive, and loads it before starting
the nested API. Nested provisioning uses only local `registry.invalid` tags with `--pull=never`.
Forgejo 16.0.3 is GPL-3.0-or-later, PostgreSQL uses the PostgreSQL license, `alpine/git` is
Apache-2.0, and Docker Compose 5.5.0 is Apache-2.0. BoxFerry redistributes none of them.
