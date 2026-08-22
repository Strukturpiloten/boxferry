#!/bin/sh
# Review generated Podman commands before running this file.
set -eu
podman 'image' 'pull' '--policy=missing' 'example.invalid/web:1'
podman 'container' 'create' '--name' 'web' '--pull=never' '--label' 'com.example.metadata=LABEL-CANARY%h' '--label' 'com.example.literal-specifier=literal%h' 'example.invalid/web:1'
podman 'container' 'start' 'web'
