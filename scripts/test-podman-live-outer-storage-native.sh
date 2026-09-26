#!/usr/bin/env bash
# Optional bounded rootful/rootless host-Podman probe; no nested service or pull.
set -Eeuo pipefail

if (($# < 1 || $# > 2)); then
  printf 'Usage: %s <already-cached-image-ID-with-VOLUME> [--create-only]\n' "$0" >&2
  exit 2
fi
if (($# == 2)) && [[ "$2" != --create-only ]]; then
  printf 'Unknown native probe mode: %s\n' "$2" >&2
  exit 2
fi
create_only=false
if (($# == 2)); then create_only=true; fi

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=scripts/lib/podman-live-outer-storage.sh
source "${script_directory}/lib/podman-live-outer-storage.sh"
engine=podman
image=$1
run_id="bf342-$(date -u +%Y%m%dt%H%M%Sz)-$$-${RANDOM}"
printf 'Native storage probe host=%s rootless=%s image=%s\n' \
  "$(podman --version)" "$(podman info --format '{{.Host.Security.Rootless}}')" "${image}"
declare -a outer_containers=()
baseline_volumes="$(timeout --signal=TERM --kill-after=10s 30s \
  "${engine}" volume ls --format '{{.Name}}' | sort)"

cleanup_native() {
  local status=$? outer owned_volumes current_volumes
  ignore_outer_cleanup_signals
  for outer in "${outer_containers[@]}"; do
    if ! release_outer "${outer}"; then status=1; fi
  done
  if ! owned_volumes="$(timeout --signal=TERM --kill-after=10s 30s "${engine}" volume ls \
    --filter "label=io.boxferry.live.run=${run_id}" --format '{{.Name}}')"; then
    status=1
  elif [[ -n "${owned_volumes}" ]]; then
    printf 'Run-owned storage remains after native probe %s.\n' "${run_id}" >&2
    status=1
  fi
  if ! current_volumes="$(timeout --signal=TERM --kill-after=10s 30s "${engine}" volume ls \
    --format '{{.Name}}' | sort)"; then
    status=1
  elif [[ "${current_volumes}" != "${baseline_volumes}" ]]; then
    printf 'Host volume identities changed during probe %s; investigate without pruning.\n' \
      "${run_id}" >&2
    status=1
  fi
  exit "${status}"
}
trap cleanup_native EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

for iteration in 1 2; do
  outer="${run_id}-${iteration}"
  outer_containers+=("${outer}")
  prepare_outer_storage "${outer}" "${image}"
  if ((${#outer_storage_mount_args[@]} == 0)); then
    printf 'Image %s declares no VOLUME; native probe would be vacuous.\n' "${image}" >&2
    exit 2
  fi
  command_arguments=(--entrypoint /bin/true "${image}")
  if ((iteration == 2)); then
    command_arguments=(--entrypoint /bin/sh "${image}" -c 'sleep 20')
  fi
  timeout --signal=TERM --kill-after=10s 90s "${engine}" create --rm --name "${outer}" --stop-timeout 1 \
    --label "io.boxferry.live.run=${run_id}" --image-volume=ignore \
    "${outer_storage_mount_args[@]}" "${command_arguments[@]}" > /dev/null
  expected_mounts=''
  for volume in "${outer_storage_names[@]}"; do
    if [[ "${outer_storage_owner[${volume}]}" == "${outer}" ]]; then
      expected_mounts+="${volume}"$'\n'
    fi
  done
  expected_mounts="$(printf '%s' "${expected_mounts}" | sort)"
  actual_mounts="$(timeout --signal=TERM --kill-after=10s 30s \
    "${engine}" container inspect --format '{{json .Mounts}}' "${outer}" |
    jq -er '[.[] | select(.Type == "volume") | .Name] | sort | .[]')"
  if [[ "${actual_mounts}" != "${expected_mounts}" ]]; then
    printf 'Unexpected outer volume mounts for %s.\n' "${outer}" >&2
    exit 1
  fi
  if ((iteration == 1)); then
    timeout --signal=TERM --kill-after=10s 90s "${engine}" start --attach "${outer}" > /dev/null
  elif [[ "${create_only}" == false ]]; then
    timeout --signal=TERM --kill-after=10s 90s "${engine}" start "${outer}" > /dev/null
    running="$(timeout --signal=TERM --kill-after=10s 30s \
      "${engine}" container inspect --format '{{.State.Running}}' "${outer}")"
    if [[ "${running}" != true ]]; then
      printf 'Expected live outer container before removal: %s.\n' "${outer}" >&2
      exit 1
    fi
  fi
  release_outer "${outer}"
done
printf 'Repeated native outer-storage cleanup passed for %s.\n' "${image}"
