#!/bin/sh
# Review generated Podman commands before running this file.
set -eu
podman 'image' 'pull' '--policy=missing' 'example.invalid/worker:1'
podman 'container' 'create' '--name' 'worker' '--pull=never' '--mount' 'type=tmpfs,target=/run/boxferry' 'example.invalid/worker:1'
podman 'container' 'start' 'worker'
