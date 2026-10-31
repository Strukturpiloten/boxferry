# Podman to Docker

This route is Unreleased development functionality. Read the [Docker acquisition, consent and
artifact contract](../../docker-input/) and [exact profile list](../../../reference/compatibility/#docker-development-boundaries).
Installed releases may expose fewer routes; source support does not establish native consumer,
application migration or release qualification.

Use the [Podman input guide](../../podman-input/) for explicit read-only acquisition, selectors
and source reconstruction. A Podman source version or daemon mode does not select a Docker target.
This offline command shows the route options without connecting to a daemon:

<!-- boxferry-example: podman-to-docker-help -->

```console
boxferry convert podman docker --help
```

Docker output requires one exact `--docker-target` preset. It contains inert `docker-plan.json`
and, when dependencies need review, the SHA-256-bound `docker-dependencies.json` sidecar. Consult
[artifact obligations](../../docker-input/#inert-artifacts-and-obligations); no generated request,
command, dependency condition or readiness check is executed.

This page uses help rather than a live-host walkthrough so the documentation gate stays offline.
The authored synthetic socket journeys exercise source routes separately; help output alone is
not conversion or native/application acceptance evidence.
