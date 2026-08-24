#!/bin/sh
# Review generated Podman commands before running this file.
set -eu
podman 'network' 'create' 'frontend'
podman 'volume' 'create' 'data'
podman 'image' 'pull' '--policy=missing' 'registry.example:5000/team/web@sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'
podman 'container' 'create' '--name' 'web' '--pull=never' '--network' 'frontend:alias=web.local' '--volume' 'data:/var/lib/data:ro,copy' '--user' '1001:1002' '--workdir' '/srv/app' '--restart' 'on-failure' 'registry.example:5000/team/web@sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff' 'php' '-v'
podman 'container' 'start' 'web'
