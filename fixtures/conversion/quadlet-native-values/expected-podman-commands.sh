#!/bin/sh
# Review generated Podman commands before running this file.
set -eu
# Requires external volume: 'external-data'
podman 'image' 'pull' '--policy=missing' 'example.invalid/quadlet-native-values:1'
podman 'container' 'create' '--name' 'native' '--pull=never' '--volume' 'external-data:/var/lib/native:ro,copy' '--entrypoint' '["/usr/bin/env","sh"]' 'example.invalid/quadlet-native-values:1' '/usr/bin/printf' 'hello quoted command'
podman 'container' 'start' 'native'
