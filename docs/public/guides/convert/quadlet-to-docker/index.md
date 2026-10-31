# Quadlet to Docker

This route is Unreleased development functionality. Read the [Docker acquisition, consent and
artifact contract](../../docker-input/) and [exact profile list](../../../reference/compatibility/#docker-development-boundaries).
Installed releases may expose fewer routes; source support does not establish native consumer,
application migration or release qualification.

Save this as `web.container`:

```ini
[Container]
Image=example.invalid/web:1
ContainerName=web-runtime

[Service]
Restart=no
```

Set the neutral application name explicitly and select the reviewed output profile. The image
identity is authored offline intent; BoxFerry does not pull or start it.

<!-- boxferry-example: quadlet-to-docker -->

```console
boxferry convert quadlet docker --input-file web.container --application-name route-matrix --docker-target upstream-29.8.1-rootful --output-directory docker-output
```

This minimal input yields one inert `docker-plan.json` with the reviewed image, runtime name and
no-restart policy. No runtime request is sent. Unmapped systemd/Quadlet intent remains visible in
outcomes; it is governed by the selected loss policy and cannot bypass the neutral model.
Compose/Quadlet source files remain documents, but Docker output is a terminal review plan and
cannot be reimported as observed inventory. Review all prerequisites before any independent use.
