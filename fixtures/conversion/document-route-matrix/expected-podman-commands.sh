#!/bin/sh
# Review generated Podman commands before running this file.
set -eu
podman 'image' 'pull' '--policy=missing' 'example.invalid/web:1'
podman 'container' 'create' '--name' 'web' '--pull=never' '--restart' 'no' 'example.invalid/web:1'
podman 'container' 'start' 'web'
