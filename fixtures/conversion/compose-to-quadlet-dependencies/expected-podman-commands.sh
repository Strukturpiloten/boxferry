#!/bin/sh
# Review generated Podman commands before running this file.
set -eu
podman 'image' 'pull' '--policy=missing' 'example.invalid/cache:1'
podman 'image' 'pull' '--policy=missing' 'example.invalid/database:1'
podman 'image' 'pull' '--policy=missing' 'example.invalid/web:1'
podman 'container' 'create' '--name' 'cache' '--pull=never' 'example.invalid/cache:1'
podman 'container' 'create' '--name' 'database' '--pull=never' 'example.invalid/database:1'
podman 'container' 'create' '--name' 'web' '--pull=never' 'example.invalid/web:1'
podman 'container' 'start' 'cache'
podman 'container' 'start' 'database'
podman 'container' 'start' 'web'
