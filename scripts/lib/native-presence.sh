#!/usr/bin/env bash
# Typed read-only queries. The caller owns privilege, identity and authorization.
# shellcheck disable=SC2034 # Sticky uncertainty is consumed by the owning cleanup runner.
native_presence_unverified=${native_presence_unverified:-false}

native_presence() {
  local maximum=$1 engine=$2 kind=$3 name=$4 socket=${5:-} observation status=0 helper
  helper="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/native-presence.py"
  local -a arguments=(--engine "${engine}" --kind "${kind}" --name "${name}")
  [[ -z "${socket}" ]] || arguments+=(--socket "${socket}")
  observation=$(
    timeout --signal=TERM --kill-after=10s "${maximum}" \
      python3 "${helper}" "${arguments[@]}" 2> /dev/null
    status=$?
    printf '#'
    exit "${status}"
  ) || status=$?
  case "${status}:${observation}" in
    $'0:present\n#') return 0 ;;
    $'1:absent\n#') return 1 ;;
  esac
  # shellcheck disable=SC2034 # Sticky uncertainty is consumed by the owning cleanup runner.
  native_presence_unverified=true
  return 2
}
