#!/bin/sh
# Review generated Podman commands before running this file.
set -eu
podman 'image' 'pull' '--policy=missing' 'example.invalid/app:2.1'
podman 'container' 'create' '--name' 'app' '--pull=never' '--restart' 'no' 'example.invalid/app:2.1'
podman 'container' 'start' 'app'
