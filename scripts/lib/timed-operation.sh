#!/usr/bin/env bash
# Shared external-command deadline wrapper. The caller supplies timestamp,
# format_duration, and file descriptor 3 for structured progress output.

timed_operation() {
  local deadline=$1 name=$2 started_at elapsed status
  shift 2
  started_at="$(date +%s)"
  printf '%s STEP START %s (deadline %s)\n' \
    "$(timestamp)" "${name}" "${deadline}" >&3
  if timeout --signal=TERM --kill-after=10s "${deadline}" "$@"; then
    elapsed=$(($(date +%s) - started_at))
    printf '%s STEP PASS  %s (%s)\n' \
      "$(timestamp)" "${name}" "$(format_duration "${elapsed}")" >&3
    return 0
  else
    status=$?
  fi
  elapsed=$(($(date +%s) - started_at))
  printf '%s STEP FAIL  %s (%s, exit %d)\n' \
    "$(timestamp)" "${name}" "$(format_duration "${elapsed}")" "${status}" >&3
  return "${status}"
}
