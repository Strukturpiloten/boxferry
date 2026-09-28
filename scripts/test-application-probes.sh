#!/usr/bin/env bash
# Offline application behavior contracts shared by local and hosted gates.
set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
bash "${script_directory}/test-application-probes-runner.sh"
bash "${script_directory}/test-application-export-privacy.sh"
for application in forgejo nextcloud paperless immich; do
  bash "${script_directory}/test-${application}-application-probes.sh"
done
