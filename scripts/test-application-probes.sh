#!/usr/bin/env bash
# Offline application behavior contracts shared by local and hosted gates.
set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
bash "${script_directory}/test-application-probes-runner.sh"
PYTHONDONTWRITEBYTECODE=1 python3 "${script_directory}/test-docker-application-expectations.py"
PYTHONDONTWRITEBYTECODE=1 python3 "${script_directory}/lib/docker-application-expectations.py" check-sources
PYTHONDONTWRITEBYTECODE=1 python3 "${script_directory}/test-docker-application-schedule.py"
PYTHONDONTWRITEBYTECODE=1 python3 "${script_directory}/test-docker-forgejo-authored-fields.py"
bash "${script_directory}/test-application-export-privacy.sh"
for application in forgejo nextcloud paperless immich observability; do
  bash "${script_directory}/test-${application}-application-probes.sh"
done
PYTHONDONTWRITEBYTECODE=1 python3 "${script_directory}/test-docker-core-artifact.py"
PYTHONDONTWRITEBYTECODE=1 python3 "${script_directory}/lib/docker-core-artifact.py" check-sources
PYTHONDONTWRITEBYTECODE=1 python3 "${script_directory}/test-docker-application-contract.py"
PYTHONDONTWRITEBYTECODE=1 python3 "${script_directory}/test-native-presence.py"
PYTHONDONTWRITEBYTECODE=1 python3 "${script_directory}/test-readiness-comparison.py"
bash "${script_directory}/test-podman-live-cleanup.sh"
