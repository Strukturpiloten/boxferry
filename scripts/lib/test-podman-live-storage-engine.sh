#!/usr/bin/env bash
# Small stateful fake for exact outer-storage lifecycle tests. Never contacts Podman.
set -euo pipefail

root=${FAKE_PODMAN_ROOT:?}
kind=${1:?}
shift
case "${kind} ${1:-}" in
  'image inspect')
    printf '%s\n' "${FAKE_IMAGE_VOLUMES:-null}"
    ;;
  'volume exists' | 'container exists')
    resource=${kind}
    name=${2:?}
    if [[ "${FAKE_FAIL_EXISTS:-}" == "${resource}:${name}" ]]; then
      exit 124
    fi
    [[ -d "${root}/${resource}s/${name}" ]]
    ;;
  'volume create')
    shift
    run='' outer=''
    while (($# > 0)); do
      case "$1" in
        --label)
          case "$2" in
            io.boxferry.live.run=*) run=${2#*=} ;;
            io.boxferry.live.outer=*) outer=${2#*=} ;;
          esac
          shift 2
          ;;
        --)
          shift
          break
          ;;
        *) exit 2 ;;
      esac
    done
    name=${1:?}
    [[ -n "${run}" && -n "${outer}" ]]
    mkdir -- "${root}/volumes/${name}"
    printf '%s\n' "${run}" > "${root}/volumes/${name}/run"
    printf '%s\n' "${outer}" > "${root}/volumes/${name}/outer"
    if [[ "${FAKE_COLLIDE_VOLUME_CREATE:-}" == "${name}" ]]; then
      printf '%s\n' 'another-run' > "${root}/volumes/${name}/run"
    fi
    printf '%s\n' "${name}"
    ;;
  'volume inspect')
    name=${*: -1}
    directory=${root}/volumes/${name}
    [[ -d "${directory}" ]]
    jq -cn --arg run "$(< "${directory}/run")" \
      --arg outer "$(< "${directory}/outer")" \
      '{"io.boxferry.live.run": $run, "io.boxferry.live.outer": $outer}'
    ;;
  'container inspect')
    name=${*: -1}
    directory=${root}/containers/${name}
    [[ -d "${directory}" ]]
    jq -cn --arg id "$(< "${directory}/id")" --arg run "$(< "${directory}/run")" \
      '{Id: $id, Config: {Labels: {"io.boxferry.live.run": $run}}}'
    ;;
  'volume rm')
    name=${*: -1}
    [[ "${FAKE_FAIL_VOLUME_RM:-}" != "${name}" ]]
    for directory in "${root}"/containers/*; do
      [[ -d "${directory}" ]] || continue
      if [[ -f "${directory}/volume" && "$(< "${directory}/volume")" == "${name}" ]]; then
        exit 2
      fi
    done
    rm -r -- "${root}/volumes/${name}"
    ;;
  'rm --force')
    id=${*: -1}
    [[ "${FAKE_FAIL_CONTAINER_RM:-}" != "${id}" ]]
    for directory in "${root}"/containers/*; do
      [[ -d "${directory}" ]] || continue
      if [[ "$(< "${directory}/id")" == "${id}" ]]; then
        rm -r -- "${directory}"
        exit 0
      fi
    done
    exit 1
    ;;
  *)
    printf 'Unexpected fake Podman call: %s %s\n' "${kind}" "$*" >&2
    exit 2
    ;;
esac
