# Compose to Podman

Create a deterministic Podman deployment plan from Compose input. BoxFerry writes review material;
it never contacts a target Podman service or executes the generated commands.

## Prerequisites

- A Compose file whose variables have explicit values; see [Compose input](../../compose-input/).
- The intended target context: `rootless`, `rootful`, or `unknown`.
- An absent or empty output directory.

## Convert

<!-- boxferry-example: compose-to-podman -->

```console
boxferry convert compose podman --input-file tmpfs-compose.yaml --podman-target-context unknown --output-directory podman-output
```

Use `unknown` while reviewing a portable plan. Select `rootless` or `rootful` when the deployment
context is known and its capability differences must be checked.

The output tree contains:

```text
podman-output/
├── podman-commands.sh
└── podman.json
```

## Read `podman.json`

The JSON is a versioned deployment plan, not a Podman inventory snapshot:

```json
{
  "schema_version": 1,
  "status": "exact",
  "connection": null,
  "external_preconditions": [],
  "operations": [
    {
      "status": "exact",
      "action": "ensure_image",
      "resource": { "kind": "image", "name": "worker-image" },
      "cli": { "program": "podman", "argv": ["image", "pull", "…"] },
      "libpod": { "method": "POST", "path_and_query": "/v6.1.0/libpod/images/pull?…" }
    }
  ]
}
```

- `schema_version` identifies the deployment-v1 contract.
- top-level `status` summarizes the complete plan.
- `connection` is null unless a caller supplied an explicit connection requirement.
- `external_preconditions` lists work that cannot be represented as a generated operation.
- each operation records its fidelity `status`, `action`, logical `resource`, equivalent `cli`
  arguments, equivalent `libpod` request, and whether sensitive input must be supplied externally.

<!-- boxferry-artifact: compose-to-podman-json -->

<!-- markdownlint-disable MD046 -->

??? example "Full generated podman.json"

    ```json
    {
      "schema_version": 1,
      "status": "exact",
      "connection": null,
      "external_preconditions": [],
      "operations": [
        {
          "status": "exact",
          "action": "ensure_image",
          "resource": {
            "kind": "image",
            "name": "worker-image"
          },
          "cli": {
            "program": "podman",
            "argv": [
              "image",
              "pull",
              "--policy=missing",
              "example.invalid/worker:1"
            ],
            "external_sensitive_input_required": false
          },
          "libpod": {
            "method": "POST",
            "path_and_query": "/v6.1.0/libpod/images/pull?reference=example.invalid%2Fworker%3A1&policy=missing",
            "body": {
              "kind": "empty"
            }
          }
        },
        {
          "status": "exact",
          "action": "create",
          "resource": {
            "kind": "container",
            "name": "worker"
          },
          "cli": {
            "program": "podman",
            "argv": [
              "container",
              "create",
              "--name",
              "worker",
              "--pull=never",
              "--mount",
              "type=tmpfs,target=/run/boxferry",
              "example.invalid/worker:1"
            ],
            "external_sensitive_input_required": false
          },
          "libpod": {
            "method": "POST",
            "path_and_query": "/v6.1.0/libpod/containers/create?name=worker",
            "body": {
              "kind": "json",
              "json": {
                "Networks": {},
                "image": "example.invalid/worker:1",
                "mounts": [
                  {
                    "destination": "/run/boxferry",
                    "options": [
                      "rw"
                    ],
                    "source": "tmpfs",
                    "type": "tmpfs"
                  }
                ]
              }
            }
          }
        },
        {
          "status": "exact",
          "action": "start_container",
          "resource": {
            "kind": "container",
            "name": "worker"
          },
          "cli": {
            "program": "podman",
            "argv": [
              "container",
              "start",
              "worker"
            ],
            "external_sensitive_input_required": false
          },
          "libpod": {
            "method": "POST",
            "path_and_query": "/v6.1.0/libpod/containers/worker/start",
            "body": {
              "kind": "empty"
            }
          }
        }
      ]
    }
    ```

<!-- markdownlint-enable MD046 -->

## Review `podman-commands.sh`

The explicit filename replaces the vague `review.sh`: this file contains commands, not merely a
report. BoxFerry never runs it, but **you will modify a Podman runtime if you run it**.

<!-- boxferry-artifact: compose-to-podman-commands -->

```sh
#!/bin/sh
# Review generated Podman commands before running this file.
set -eu
podman 'image' 'pull' '--policy=missing' 'example.invalid/worker:1'
podman 'container' 'create' '--name' 'worker' '--pull=never' '--mount' 'type=tmpfs,target=/run/boxferry' 'example.invalid/worker:1'
podman 'container' 'start' 'worker'
```

Review the selected Podman connection, command order, bind mounts, network creation, published
ports, secret preconditions, and rootless/rootful context before handing the file to an operator.

## Compatibility and loss

Output defaults to the newest reviewed target, currently 6.1.0. Add
`--podman-max-version VERSION` to cap features for an older production host. The ceiling resolves
to the newest reviewed exact target not greater than that value and fails below the catalogue.

The default exact policy writes nothing when intent cannot be represented exactly. Use a more
permissive policy only after reading every diagnostic; it does not make host paths or secrets
portable.

---

[← Compose to Compose](../compose-to-compose/) · [Next: Compose to Quadlet →](../compose-to-quadlet/)
