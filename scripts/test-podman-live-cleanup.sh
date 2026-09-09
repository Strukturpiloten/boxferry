#!/usr/bin/env bash
# Guard cleanup collections against Bash's empty-array default-value trap.
# shellcheck disable=SC2016 # The guarded snippets must remain literal shell source.

set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
runner="${script_directory}/podman-live-conformance.sh"

collections=(
  outer_containers
  mounted_images
  fault_proxy_pids
  fault_proxy_sockets
  discovery_directories
)

for collection in "${collections[@]}"; do
  unsafe='"${'"${collection}"'[@]:-}"'
  safe=""
  case "${collection}" in
    outer_containers)
      safe='for outer in "${outer_containers[@]}"; do'
      ;;
    mounted_images)
      safe='for image in "${mounted_images[@]}"; do'
      ;;
    fault_proxy_pids)
      safe='for pid in "${fault_proxy_pids[@]}"; do'
      ;;
    fault_proxy_sockets)
      safe='for socket in "${fault_proxy_sockets[@]}"; do'
      ;;
    discovery_directories)
      safe='for directory in "${discovery_directories[@]}"; do'
      ;;
    *)
      printf 'Unknown cleanup collection: %s.\n' "${collection}" >&2
      exit 2
      ;;
  esac

  if grep --fixed-strings --quiet -- "${unsafe}" "${runner}"; then
    printf 'Unsafe empty-array cleanup expansion remains for %s.\n' "${collection}" >&2
    exit 1
  fi
  if ! grep --fixed-strings --quiet -- "${safe}" "${runner}"; then
    printf 'Safe cleanup iteration is missing for %s.\n' "${collection}" >&2
    exit 1
  fi
done

declare -a empty_collection=()
iterations=0
for unused in "${empty_collection[@]}"; do
  : "${unused}"
  iterations=$((iterations + 1))
done
[[ "${iterations}" == 0 ]]

printf 'Podman live cleanup empty-array regression test passed.\n'
