#!/usr/bin/env bash
# Guard cleanup collections against Bash's empty-array default-value trap.
# shellcheck disable=SC2016 # The guarded snippets must remain literal shell source.

set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
runner="${script_directory}/podman-live-conformance.sh"

collections=(
  outer_containers
  mounted_images
  run_owned_host_images
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
    run_owned_host_images)
      # EXIT release deliberately uses an index loop so reverse order is
      # explicit and remains correct if a future path creates sparse entries.
      safe='local -a image_indexes=("${!run_owned_host_images[@]}")'
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

grep --fixed-strings --quiet -- 'record_run_owned_host_image "${image}"' "${runner}"
grep --fixed-strings --quiet -- 'release_run_owned_host_image "${image}"' "${runner}"
grep --fixed-strings --quiet -- 'release_remaining_run_owned_host_images' "${runner}"
grep --fixed-strings --quiet -- '"${engine}" image rm --ignore --no-prune -- "${image}"' "${runner}"
if grep --fixed-strings --quiet -- 'image rm --force' "${runner}"; then
  printf '%s\n' 'Run-owned matrix images must not be force-removed.' >&2
  exit 1
fi
if grep --extended-regexp --quiet -- 'image rm .*--prune' "${runner}"; then
  printf '%s\n' 'Run-owned matrix images must not prune parent images.' >&2
  exit 1
fi

# Bind ownership to the actual absent-cache pull branch, and clear it only
# after the exact non-pruning remove completes successfully.
absent_branch_line="$(grep -n --fixed-strings -- 'elif ((cache_status == 1)); then' "${runner}" | tail -n 1 | cut -d: -f1)"
pull_line="$(grep -n --fixed-strings -- '"${engine}" pull --quiet "${image}"' "${runner}" | tail -n 1 | cut -d: -f1)"
record_line="$(grep -n --fixed-strings -- 'record_run_owned_host_image "${image}"' "${runner}" | tail -n 1 | cut -d: -f1)"
remove_line="$(grep -n --fixed-strings -- '"${engine}" image rm --ignore --no-prune -- "${image}"' "${runner}" | cut -d: -f1)"
unset_line="$(grep -n --fixed-strings -- 'unset "run_owned_host_image_seen[${image}]"' "${runner}" | cut -d: -f1)"
[[ "${absent_branch_line}" -lt "${pull_line}" && "${pull_line}" -lt "${record_line}" ]]
[[ "${remove_line}" -lt "${unset_line}" ]]

# Model a pull followed by a cache hit, release, then a repull of the same
# digest. Ownership is current rather than historical: duplicate live
# ownership is deduplicated, a cache-only image is never owned, and a repull
# acquires a fresh release obligation.
declare -a owned_images=()
declare -A owned_seen=()
declare -a release_log=()
record_owned() {
  local image=$1
  if [[ -z "${owned_seen[${image}]:-}" ]]; then
    owned_images+=("${image}")
    owned_seen[${image}]=true
  fi
}
release_owned() {
  local image=$1
  [[ -n "${owned_seen[${image}]:-}" ]] || return 0
  release_log+=("${image}")
  unset "owned_seen[${image}]"
}
record_owned 'example.invalid/matrix@sha256:one'
record_owned 'example.invalid/matrix@sha256:one'
[[ "${#owned_images[@]}" == 1 ]]
release_owned 'example.invalid/matrix@sha256:one'
# A cache hit does not call record_owned. A later repull records a fresh
# acquisition generation, while duplicate simultaneous ownership is deduped.
record_owned 'example.invalid/matrix@sha256:one'
release_owned 'example.invalid/matrix@sha256:one'
[[ "${#release_log[@]}" == 2 ]]
[[ "${#owned_images[@]}" == 2 ]]
cached_image='example.invalid/matrix@sha256:cached'
[[ -z "${owned_seen[${cached_image}]:-}" ]]

declare -a sparse_images=()
sparse_images[3]=first
sparse_images[9]=last
declare -a sparse_indexes=("${!sparse_images[@]}") released_images=()
for ((index = ${#sparse_indexes[@]} - 1; index >= 0; index--)); do
  released_images+=("${sparse_images[${sparse_indexes[index]}]}")
done
[[ "${released_images[*]}" == 'last first' ]]

# EXIT cleanup must attempt later run-owned refs even when an earlier exact
# image removal fails, then report the aggregate failure.
declare -a attempted_images=(first second)
declare -a attempted_release_log=()
release_model() {
  local image=$1
  attempted_release_log+=("${image}")
  [[ "${image}" != first ]]
}
release_all_model() {
  local image failed=false
  for image in "${attempted_images[@]}"; do
    if ! release_model "${image}"; then
      failed=true
    fi
  done
  [[ "${failed}" == false ]]
}
if release_all_model; then
  printf '%s\n' 'Aggregate release model did not report the first failure.' >&2
  exit 1
fi
[[ "${attempted_release_log[*]}" == 'first second' ]]
grep --fixed-strings --quiet -- 'if ! release_run_owned_host_image "${image}"; then' "${runner}"
grep --fixed-strings --quiet -- 'release_failed=true' "${runner}"
grep --fixed-strings --quiet -- '[[ "${release_failed}" == false ]]' "${runner}"

# Containers and mounts must be released before the reverse image pass.
containers_line="$(grep -n --fixed-strings -- 'for outer in "${outer_containers[@]}"; do' "${runner}" | head -n 1 | cut -d: -f1)"
mounts_line="$(grep -n --fixed-strings -- 'for image in "${mounted_images[@]}"; do' "${runner}" | head -n 1 | cut -d: -f1)"
images_line="$(grep -n --fixed-strings -- 'release_remaining_run_owned_host_images' "${runner}" | tail -n 1 | cut -d: -f1)"
[[ "${containers_line}" -lt "${mounts_line}" && "${mounts_line}" -lt "${images_line}" ]]

# Releases emit their own timed evidence and must not change a cell's declared
# scenario-check count.
if grep --fixed-strings --quiet -- "progress_run 'release run-owned host image" "${runner}"; then
  printf '%s\n' 'Run-owned image release must not increment cell progress.' >&2
  exit 1
fi
[[ "$(grep --fixed-strings --count -- 'release_run_owned_host_image "${image}"' "${runner}")" -ge 2 ]]
grep --fixed-strings --quiet -- '"${profile}" != smoke && "${cleanup_failed}" == true && "${status}" == 0' "${runner}"

# Limited rootless cells mount before their explicit outer removal. Their
# release must remain after both operations.
limited_unmount_line="$(grep -n --fixed-strings -- "engine_operation 'unmount limited-cell image'" "${runner}" | cut -d: -f1)"
limited_remove_line="$(grep -n --fixed-strings -- "progress_run 'remove disposable limitation container'" "${runner}" | cut -d: -f1)"
limited_release_line="$(grep -n --fixed-strings -- 'release_run_owned_host_image "${image}"' "${runner}" | tail -n 1 | cut -d: -f1)"
[[ "${limited_unmount_line}" -lt "${limited_remove_line}" && "${limited_remove_line}" -lt "${limited_release_line}" ]]

# A clean acquisition replaces the first outer runtime. Refresh the local
# handle before later work can start an apply target, then remove that exact
# acquisition runtime before releasing its matrix image.
clean_restart_line="$(grep -n --fixed-strings -- 'start_clean_acquisition_outer "${id}" "${image}" "${mode}"' "${runner}" | tail -n 1 | cut -d: -f1)"
outer_refresh_line="$(grep -n --fixed-strings -- 'outer="${started_outer}"' "${runner}" | tail -n 1 | cut -d: -f1)"
normal_remove_line="$(grep -n --fixed-strings -- "progress_run 'remove disposable outer container'" "${runner}" | cut -d: -f1)"
normal_release_line="$(grep -n --fixed-strings -- 'release_run_owned_host_image "${image}"' "${runner}" | sed -n '2p' | cut -d: -f1)"
[[ "${clean_restart_line}" -lt "${outer_refresh_line}" && "${outer_refresh_line}" -lt "${normal_remove_line}" && "${normal_remove_line}" -lt "${normal_release_line}" ]]

if grep --fixed-strings --quiet -- 'run_owned_matrix_image' "${runner}"; then
  printf '%s\n' 'Obsolete matrix-only image ownership remains in the live runner.' >&2
  exit 1
fi

declare -a empty_collection=()
iterations=0
for unused in "${empty_collection[@]}"; do
  : "${unused}"
  iterations=$((iterations + 1))
done
[[ "${iterations}" == 0 ]]

applications=(nextcloud forgejo paperless immich observability supabase)
for application in "${applications[@]}"; do
  module="${script_directory}/lib/${application}-application.sh"
  grep --fixed-strings --quiet -- 'record_run_owned_host_image "${reference}"' "${module}"
  grep --fixed-strings --quiet -- 'release_run_owned_host_image "${reference}"' "${module}"
done

# Aliases are never overwritten: the helper probes first and records only after tag succeeds.
grep --fixed-strings --quiet -- 'Refusing to overwrite existing %s archive alias %s.' "${runner}"
alias_probe_line="$(grep -n --fixed-strings -- 'engine_image_available "probe ${description} archive alias"' "${runner}" | cut -d: -f1)"
alias_tag_line="$(grep -n --fixed-strings -- 'engine_operation "tag ${description} image for nested archive"' "${runner}" | cut -d: -f1)"
alias_record_line="$(grep -n --fixed-strings -- 'record_run_owned_host_image "${alias}"' "${runner}" | cut -d: -f1)"
[[ "${alias_probe_line}" -lt "${alias_tag_line}" && "${alias_tag_line}" -lt "${alias_record_line}" ]]

# Large OCI bundles never retain all source members alongside their completed tarball,
# and the nested target drops each extracted member immediately after loading it.
for application in paperless immich observability; do
  module="${script_directory}/lib/${application}-application.sh"
  grep --fixed-strings --quiet -- 'tar --create --remove-files' "${module}"
  grep --fixed-strings --quiet -- 'trap "rm -rf -- \"$directory\"" EXIT' "${module}"
  grep --fixed-strings --quiet -- 'rm -f -- "$archive"' "${module}"
  grep --fixed-strings --quiet -- 'rmdir "$directory"' "${module}"
done
grep --fixed-strings --quiet -- 'tar --remove-files' "${script_directory}/lib/supabase-application.sh"
grep --fixed-strings --quiet -- 'trap "rm -rf -- \"$directory\"" EXIT' "${script_directory}/lib/supabase-application.sh"
grep --fixed-strings --quiet -- 'rm -f -- "$archive"' "${script_directory}/lib/supabase-application.sh"
grep --fixed-strings --quiet -- 'rmdir "$directory"' "${script_directory}/lib/supabase-application.sh"
grep --fixed-strings --quiet -- 'PAPERLESS_ARCHIVE_MAX_BYTES="2684354560"' "${script_directory}/lib/paperless-application.sh"
grep --fixed-strings --quiet -- 'IMMICH_ARCHIVE_MAX_BYTES="2684354560"' "${script_directory}/lib/immich-application.sh"
grep --fixed-strings --quiet -- 'OBSERVABILITY_ARCHIVE_MAX_BYTES="2147483648"' "${script_directory}/lib/observability-application.sh"
grep --fixed-strings --quiet -- 'SUPABASE_ARCHIVE_MAX_BYTES="5368709120"' "${script_directory}/lib/supabase-application.sh"

printf 'Podman live cleanup empty-array regression test passed.\n'
