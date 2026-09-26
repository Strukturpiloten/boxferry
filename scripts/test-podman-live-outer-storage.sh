#!/usr/bin/env bash
# Behavioral cleanup checks with an isolated fake Podman state store.
set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=scripts/lib/podman-live-outer-storage.sh
source "${script_directory}/lib/podman-live-outer-storage.sh"
engine="${script_directory}/lib/test-podman-live-storage-engine.sh"
export FAKE_PODMAN_ROOT
FAKE_PODMAN_ROOT="$(mktemp -d /tmp/boxferry-live-storage-test.XXXXXX)"
trap 'rm -r -- "${FAKE_PODMAN_ROOT}"' EXIT
mkdir -- "${FAKE_PODMAN_ROOT}/volumes" "${FAKE_PODMAN_ROOT}/containers"
export FAKE_IMAGE_VOLUMES='{ "/var/lib/containers": {}, "/var/cache/nested": {} }'
run_id='bf-test-first'

register_container() {
  local name=$1 number=$2 owner=$3 volume=${4:-}
  mkdir -- "${FAKE_PODMAN_ROOT}/containers/${name}"
  printf '%064d\n' "${number}" > "${FAKE_PODMAN_ROOT}/containers/${name}/id"
  printf '%s\n' "${owner}" > "${FAKE_PODMAN_ROOT}/containers/${name}/run"
  if [[ -n "${volume}" ]]; then
    printf '%s\n' "${volume}" > "${FAKE_PODMAN_ROOT}/containers/${name}/volume"
  fi
}

prepare_outer_storage first image
[[ "${#outer_storage_mount_args[@]}" == 4 ]]
[[ "${#outer_storage_names[@]}" == 2 ]]
first_volume=${outer_storage_names[0]}
second_volume=${outer_storage_names[1]}
register_container first 1 "${run_id}" "${first_volume}"
release_outer first
release_outer first
[[ ! -d "${FAKE_PODMAN_ROOT}/volumes/${first_volume}" ]]
[[ ! -d "${FAKE_PODMAN_ROOT}/volumes/${second_volume}" ]]

# Auto-removal may erase the container before EXIT. Its owned storage remains
# exact and removable without enumerating unrelated volumes.
prepare_outer_storage vanished image
vanished_volume=${outer_storage_names[2]}
release_outer vanished
[[ ! -d "${FAKE_PODMAN_ROOT}/volumes/${vanished_volume}" ]]

# A concurrent run and a user resource survive this run's cleanup.
run_id='bf-test-second'
prepare_outer_storage other image
other_volume=${outer_storage_names[4]}
run_id='bf-test-first'
if release_outer other; then
  printf '%s\n' 'Cross-run storage was accepted as this run owned.' >&2
  exit 1
fi
[[ -d "${FAKE_PODMAN_ROOT}/volumes/${other_volume}" ]]
mkdir -- "${FAKE_PODMAN_ROOT}/volumes/user-sentinel"
if prepare_outer_storage other image; then
  printf '%s\n' 'An existing volume name was overwritten.' >&2
  exit 1
fi
[[ -d "${FAKE_PODMAN_ROOT}/volumes/user-sentinel" ]]
run_id='bf-test-second'
release_outer other

# A competing creator between the absence check and create must not be
# mounted or later deleted merely because the requested name was returned.
run_id='bf-test-race'
export FAKE_COLLIDE_VOLUME_CREATE=race-store-1
if prepare_outer_storage race image; then
  printf '%s\n' 'A raced foreign volume was accepted.' >&2
  exit 1
fi
[[ -d "${FAKE_PODMAN_ROOT}/volumes/race-store-1" ]]
if release_outer race; then
  printf '%s\n' 'A raced foreign volume was accepted during cleanup.' >&2
  exit 1
fi
[[ -d "${FAKE_PODMAN_ROOT}/volumes/race-store-1" ]]
unset FAKE_COLLIDE_VOLUME_CREATE

# A removal failure is reported and cannot force-delete an in-use volume.
run_id='bf-test-third'
prepare_outer_storage failure image
failure_volume=${outer_storage_names[7]}
register_container failure 3 "${run_id}" "${failure_volume}"
export FAKE_FAIL_CONTAINER_RM
FAKE_FAIL_CONTAINER_RM="$(< "${FAKE_PODMAN_ROOT}/containers/failure/id")"
if release_outer failure; then
  printf '%s\n' 'Container removal failure was accepted.' >&2
  exit 1
fi
[[ -d "${FAKE_PODMAN_ROOT}/volumes/${failure_volume}" ]]
unset FAKE_FAIL_CONTAINER_RM
release_outer failure

# A failed volume removal must not suppress attempts on later owned volumes.
run_id='bf-test-volume-failure'
prepare_outer_storage volume-failure image
stuck_volume=${outer_storage_names[9]}
later_volume=${outer_storage_names[10]}
export FAKE_FAIL_VOLUME_RM=${stuck_volume}
if release_outer volume-failure; then
  printf '%s\n' 'Volume removal failure was accepted.' >&2
  exit 1
fi
[[ -d "${FAKE_PODMAN_ROOT}/volumes/${stuck_volume}" ]]
[[ ! -d "${FAKE_PODMAN_ROOT}/volumes/${later_volume}" ]]
unset FAKE_FAIL_VOLUME_RM
release_outer volume-failure

# Wrong labels are never deletion authority, even at the expected exact name.
run_id='bf-test-fourth'
register_container sentinel 4 user-owned
if release_outer sentinel; then
  printf '%s\n' 'Unowned container was removed.' >&2
  exit 1
fi
[[ -d "${FAKE_PODMAN_ROOT}/containers/sentinel" ]]

# The actual signal-guard helper must keep an EXIT cleanup alive after a
# second TERM. The child exit status still reflects the first cancellation.
ready_marker="${FAKE_PODMAN_ROOT}/signal-ready"
start_marker="${FAKE_PODMAN_ROOT}/signal-start"
done_marker="${FAKE_PODMAN_ROOT}/signal-done"
bash -c '
  source "$1"
  trap '\''ignore_outer_cleanup_signals; touch "$3"; sleep 0.3; touch "$4"'\'' EXIT
  trap '\''exit 143'\'' TERM
  touch "$2"
  while :; do sleep 0.1; done
' bash "${script_directory}/lib/podman-live-outer-storage.sh" \
  "${ready_marker}" "${start_marker}" "${done_marker}" &
signal_pid=$!
for _ in {1..50}; do
  [[ -e "${ready_marker}" ]] && break
  sleep 0.01
done
[[ -e "${ready_marker}" ]]
kill -TERM "${signal_pid}"
for _ in {1..50}; do
  [[ -e "${start_marker}" ]] && break
  sleep 0.01
done
[[ -e "${start_marker}" ]]
kill -TERM "${signal_pid}"
signal_status=0
wait "${signal_pid}" || signal_status=$?
[[ "${signal_status}" == 143 && -e "${done_marker}" ]]

# An unavailable existence answer is not permission to create or delete.
export FAKE_FAIL_EXISTS=volume:unavailable-store-1
if prepare_outer_storage unavailable image; then
  printf '%s\n' 'Unknown volume-existence result was accepted.' >&2
  exit 1
fi
[[ ! -d "${FAKE_PODMAN_ROOT}/volumes/unavailable-store-1" ]]
unset FAKE_FAIL_EXISTS

# JSON keys are validated before line-based decoding. Otherwise one embedded
# newline could turn one declared path into two unrelated, apparently safe ones.
FAKE_IMAGE_VOLUMES='{"/var/lib/a\n/b": {}}'
if prepare_outer_storage newline image; then
  printf '%s\n' 'Split image volume destination was accepted.' >&2
  exit 1
fi
[[ ! -d "${FAKE_PODMAN_ROOT}/volumes/newline-store-1" ]]
FAKE_IMAGE_VOLUMES='{"": {}}'
if prepare_outer_storage empty image; then
  printf '%s\n' 'Empty image volume destination was accepted.' >&2
  exit 1
fi
[[ ! -d "${FAKE_PODMAN_ROOT}/volumes/empty-store-1" ]]
FAKE_IMAGE_VOLUMES='{ "/var/lib/containers": {}, "/var/cache/nested": {} }'

# Two independently running invocations share the fake host store but hold
# disjoint run labels and release only their own exact volumes.
concurrent_probe() (
  # shellcheck source=scripts/lib/podman-live-outer-storage.sh
  source "${script_directory}/lib/podman-live-outer-storage.sh"
  local suffix=$1
  run_id="bf-test-concurrent-${suffix}"
  prepare_outer_storage "concurrent-${suffix}" image
  touch "${FAKE_PODMAN_ROOT}/${suffix}-ready"
  for _ in {1..100}; do
    [[ -e "${FAKE_PODMAN_ROOT}/${suffix}-release" ]] && break
    sleep 0.01
  done
  [[ -e "${FAKE_PODMAN_ROOT}/${suffix}-release" ]]
  release_outer "concurrent-${suffix}"
)
concurrent_probe a &
a_pid=$!
concurrent_probe b &
b_pid=$!
for _ in {1..100}; do
  [[ -e "${FAKE_PODMAN_ROOT}/a-ready" && -e "${FAKE_PODMAN_ROOT}/b-ready" ]] && break
  sleep 0.01
done
[[ -e "${FAKE_PODMAN_ROOT}/a-ready" && -e "${FAKE_PODMAN_ROOT}/b-ready" ]]
touch "${FAKE_PODMAN_ROOT}/a-release"
wait "${a_pid}"
[[ -d "${FAKE_PODMAN_ROOT}/volumes/concurrent-b-store-1" ]]
touch "${FAKE_PODMAN_ROOT}/b-release"
wait "${b_pid}"
[[ ! -d "${FAKE_PODMAN_ROOT}/volumes/concurrent-b-store-1" ]]

printf 'Podman live outer-storage lifecycle regression test passed.\n'
