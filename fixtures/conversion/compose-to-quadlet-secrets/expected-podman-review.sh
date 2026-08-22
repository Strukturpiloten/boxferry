#!/bin/sh
# Review generated Podman commands before running this file.
set -eu
# Requires external secret: 'api-token'
# Requires external secret: 'certificate'
# Requires external secret: 'database-password'
: "${PODMAN_LENS_SECRET_INPUT_1:?set to a readable secret file path}"
podman 'secret' 'create' 'local-password' '-' < "${PODMAN_LENS_SECRET_INPUT_1}"
podman 'image' 'pull' '--policy=missing' 'example.invalid/web:1'
podman 'container' 'create' '--name' 'web' '--pull=never' 'example.invalid/web:1'
podman 'container' 'start' 'web'
