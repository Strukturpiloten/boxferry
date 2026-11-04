#!/usr/bin/env bash

set -Eeuo pipefail

if (($# > 1)); then
  printf 'Usage: %s [install-directory]\n' "$0" >&2
  exit 2
fi

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repository_root="$(cd -- "${script_directory}/.." && pwd -P)"
readonly repository_root

exec python3 "${repository_root}/automation/src/boxferry_ci/tooling.py" \
  --destination "${1:-/usr/local/bin}" --profile core
