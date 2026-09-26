#!/usr/bin/env bash
# Exact, run-scoped outer Podman storage. The caller supplies engine and run_id.
# shellcheck disable=SC2154 # engine and run_id are supplied by the sourcing runner.

declare -a outer_storage_names=()
declare -A outer_storage_owner=()
declare -a outer_storage_mount_args=()

ignore_outer_cleanup_signals() {
  # A second cancellation signal must not interrupt EXIT cleanup half-way.
  trap '' INT TERM
}

outer_resource_absent() {
  local kind=$1 name=$2 status=0
  timeout --signal=TERM --kill-after=10s 30s \
    "${engine}" "${kind}" exists "${name}" > /dev/null 2>&1 || status=$?
  case "${status}" in
    0) return 1 ;;
    1) return 0 ;;
    *)
      printf 'Cannot verify %s absence for %s (exit %d).\n' "${kind}" "${name}" "${status}" >&2
      return 2
      ;;
  esac
}

verify_outer_storage_volume_ownership() {
  local volume=$1 outer=$2 labels
  labels="$(timeout --signal=TERM --kill-after=10s 30s \
    "${engine}" volume inspect --format '{{json .Labels}}' "${volume}")" || return 1
  if ! jq -e --arg run "${run_id}" --arg outer "${outer}" \
    '."io.boxferry.live.run" == $run and ."io.boxferry.live.outer" == $outer' \
    <<< "${labels}" > /dev/null; then
    printf 'Refusing volume without exact run ownership: %s.\n' "${volume}" >&2
    return 1
  fi
}

prepare_outer_storage() {
  local outer=$1 image=$2 declared destination volume created status index=0
  outer_storage_mount_args=()
  declared="$(timeout --signal=TERM --kill-after=10s 90s \
    "${engine}" image inspect --format '{{json .Config.Volumes}}' "${image}")" || return 1
  # A missing Volumes object means the image declares no persistent paths.
  if ! jq -e '(. == null) or (type == "object")' <<< "${declared}" > /dev/null; then
    printf 'Invalid image volume declaration for %s.\n' "${image}" >&2
    return 1
  fi
  # Validate complete JSON keys before jq emits newline-delimited paths. A
  # newline inside a key must not become two separately accepted destinations.
  if ! jq -e '(. // {}) | keys | all(.[];
    startswith("/") and length > 1 and
    (test("[^A-Za-z0-9_./-]") | not) and
    (contains("/../") | not) and (endswith("/..") | not) and
    (contains("/./") | not))' <<< "${declared}" > /dev/null; then
    printf 'Unsafe image volume declaration for %s.\n' "${image}" >&2
    return 1
  fi
  while IFS= read -r destination; do
    [[ -n "${destination}" ]] || continue
    if [[ ! "${destination}" =~ ^/[a-zA-Z0-9_./-]+$ ||
      "${destination}" == / || "${destination}" == *'/../'* ||
      "${destination}" == */.. || "${destination}" == *'/./'* ]]; then
      printf 'Unsafe image volume destination for %s: %s\n' "${image}" "${destination}" >&2
      return 1
    fi
    index=$((index + 1))
    volume="${outer}-store-${index}"
    if outer_resource_absent volume "${volume}"; then
      :
    else
      status=$?
      ((status == 1)) && printf 'Refusing existing outer storage volume %s.\n' "${volume}" >&2
      return 1
    fi
    # Register after proving absence but before creation so an interrupted
    # create still has an exact, label-checked cleanup candidate.
    outer_storage_names+=("${volume}")
    outer_storage_owner[${volume}]="${outer}"
    created="$(timeout --signal=TERM --kill-after=10s 90s "${engine}" volume create \
      --label "io.boxferry.live.run=${run_id}" \
      --label "io.boxferry.live.outer=${outer}" -- "${volume}")" || return 1
    [[ "${created}" == "${volume}" ]] || {
      printf 'Unexpected created outer volume identity: %s.\n' "${created}" >&2
      return 1
    }
    verify_outer_storage_volume_ownership "${volume}" "${outer}" || return 1
    outer_storage_mount_args+=(--volume "${volume}:${destination}")
  done < <(jq -r '(. // {}) | keys[]' <<< "${declared}")
}

release_outer_storage_volume() {
  local volume=$1 outer=$2 status=0
  if outer_resource_absent volume "${volume}"; then
    return 0
  else
    status=$?
    ((status == 1)) || return 1
  fi
  verify_outer_storage_volume_ownership "${volume}" "${outer}" || return 1
  # Never force-remove volumes: Podman may remove other containers using them.
  if ! timeout --signal=TERM --kill-after=10s 30s \
    "${engine}" volume rm -- "${volume}" > /dev/null; then
    printf 'Could not remove run-owned storage volume %s.\n' "${volume}" >&2
    return 1
  fi
  if ! outer_resource_absent volume "${volume}"; then
    printf 'Run-owned storage volume remains: %s.\n' "${volume}" >&2
    return 1
  fi
}

release_outer() {
  local outer=$1 metadata id volume failed=false status=0
  if outer_resource_absent container "${outer}"; then
    :
  else
    status=$?
    if ((status != 1)); then
      failed=true
    else
      metadata="$(timeout --signal=TERM --kill-after=10s 30s \
        "${engine}" container inspect --format '{{json .}}' "${outer}")" || metadata=""
      if [[ -z "${metadata}" ]] || ! jq -e --arg run "${run_id}" \
        '.Config.Labels."io.boxferry.live.run" == $run' <<< "${metadata}" > /dev/null; then
        printf 'Refusing container without exact run ownership: %s.\n' "${outer}" >&2
        failed=true
      else
        id="$(jq -er '.Id | select(test("^[0-9a-f]{64}$"))' <<< "${metadata}")" || id=""
        if [[ -z "${id}" ]] || ! timeout --signal=TERM --kill-after=10s 30s \
          "${engine}" rm --force --ignore --volumes -- "${id}" > /dev/null; then
          printf 'Could not remove run-owned outer container %s.\n' "${outer}" >&2
          failed=true
        fi
      fi
    fi
  fi
  if ! outer_resource_absent container "${outer}"; then
    printf 'Run-owned outer container remains or cannot be verified: %s.\n' "${outer}" >&2
    failed=true
  fi
  for volume in "${outer_storage_names[@]}"; do
    [[ "${outer_storage_owner[${volume}]}" == "${outer}" ]] || continue
    if ! release_outer_storage_volume "${volume}" "${outer}"; then
      failed=true
    fi
  done
  [[ "${failed}" == false ]]
}
