#!/bin/sh
# Review generated Podman commands before running this file.
set -eu
podman 'network' 'create' 'frontend'
podman 'image' 'pull' '--policy=missing' 'example.invalid/web:1'
podman 'image' 'pull' '--policy=missing' 'example.invalid/worker:1'
podman 'container' 'create' '--name' 'web' '--pull=never' '--network' 'frontend' 'example.invalid/web:1'
podman 'container' 'create' '--name' 'worker' '--pull=never' '--network' 'frontend' 'example.invalid/worker:1'
podman 'container' 'start' 'web'
podman 'container' 'start' 'worker'
