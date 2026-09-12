#!/usr/bin/env bash
# shellcheck disable=SC2016 # Inner Bash programs must expand only in their child shells.

set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
library="${script_directory}/lib/supabase-application.sh"
deadline_helper="${script_directory}/lib/in-shell-deadline.py"
test_root="$(mktemp -d)"
trap 'rm -rf -- "${test_root}"' EXIT

case_status=0
run_case() {
  local output=$1 case_pid
  shift
  case_status=0
  setsid --wait "$@" > "${output}" 2>&1 &
  case_pid=$!
  if wait "${case_pid}" 2> /dev/null; then
    case_status=0
  else
    case_status=$?
  fi
}

assert_process_gone() {
  local pid=$1 _iteration
  for _iteration in {1..250}; do
    [[ ! -e "/proc/${pid}/stat" ]] && return 0
    sleep 0.02
  done
  printf 'Deadline regression left process %s alive.\n' "${pid}" >&2
  return 1
}

unsupported_output="${test_root}/unsupported.output"
run_case "${unsupported_output}" bash -c '
  set -Eeuo pipefail
  exec 3>&2
  script_directory="$(cd -- "$(dirname -- "$1")/.." && pwd -P)"
  source "$1"
  timestamp() { printf "test-time"; }
  format_duration() { printf "%ss" "$1"; }
  run_supabase_application_cell unsupported-cell reviewed-image 0.0 test \
    rootless container amd64
' shell "${library}"
[[ "${case_status}" == 1 ]]
grep --fixed-strings --quiet -- \
  'Supabase application supports only podman-6.1-rootless rootless.' \
  "${unsupported_output}"
if grep --fixed-strings --quiet -- 'failed to run command' "${unsupported_output}"; then
  printf '%s\n' 'GNU timeout still received the Supabase shell function.' >&2
  exit 1
fi

success_output="${test_root}/success.output"
run_case "${success_output}" bash -c '
  set -Eeuo pipefail
  exec 3>&2
  script_directory="$(cd -- "$(dirname -- "$1")/.." && pwd -P)"
  source "$1"
  timestamp() { printf "test-time"; }
  format_duration() { printf "%ss" "$1"; }
  SUPABASE_CELL_TIMEOUT=2s
  supabase_run_application_cell_unbounded() {
    printf "%s\n" "in-shell-supabase-cell-ran"
  }
  run_supabase_application_cell ignored arguments
' shell "${library}"
[[ "${case_status}" == 0 ]]
grep --fixed-strings --quiet -- 'in-shell-supabase-cell-ran' "${success_output}"
grep --fixed-strings --quiet -- \
  'STEP PASS complete Supabase application cell' "${success_output}"

delayed_output="${test_root}/delayed-watchdog.output"
run_case "${delayed_output}" bash -c '
  set -Eeuo pipefail
  exec 3>&2
  script_directory="$(cd -- "$(dirname -- "$1")/.." && pwd -P)"
  source "$1"
  timestamp() { printf "test-time"; }
  format_duration() { printf "%ss" "$1"; }
  SUPABASE_CELL_KILL_AFTER=0.2s
  finish_after_deadline() {
    kill -STOP "${watchdog_pid}"
    (
      sleep 0.2
      kill -CONT "${watchdog_pid}"
    ) &
    sleep 0.12
  }
  supabase_timed_in_shell_operation 0.05s "delayed-watchdog-deadline" \
    finish_after_deadline
' shell "${library}"
[[ "${case_status}" == 143 ]]
grep --fixed-strings --quiet -- \
  'STEP FAIL  delayed-watchdog-deadline (deadline expired; TERM, KILL after 0.2s)' \
  "${delayed_output}"
if grep --fixed-strings --quiet -- 'STEP PASS delayed-watchdog-deadline' \
  "${delayed_output}"; then
  printf '%s\n' 'Delayed deadline observation produced false PASS evidence.' >&2
  exit 1
fi

failure_output="${test_root}/failure.output"
failure_marker="${test_root}/after-failure"
run_case "${failure_output}" bash -c '
  set -Eeuo pipefail
  exec 3>&2
  script_directory="$(cd -- "$(dirname -- "$1")/.." && pwd -P)"
  source "$1"
  timestamp() { printf "test-time"; }
  format_duration() { printf "%ss" "$1"; }
  trap '\''failure_status=$?; printf "test-time TEST FAIL synthetic Supabase cell (exit %d)\n" "${failure_status}" >&3'\'' ERR
  fail_between_commands() {
    printf "%s\n" before-failure
    false
    printf "%s\n" after-failure > "$2"
  }
  supabase_timed_in_shell_operation 2s "intermediate failure" \
    fail_between_commands ignored "$2"
' shell "${library}" "${failure_marker}"
[[ "${case_status}" == 1 ]]
[[ ! -e "${failure_marker}" ]]
grep --fixed-strings --quiet -- 'TEST FAIL synthetic Supabase cell (exit 1)' \
  "${failure_output}"
if grep --fixed-strings --quiet -- 'STEP PASS intermediate failure' "${failure_output}"; then
  printf '%s\n' 'An intermediate cell failure produced false PASS evidence.' >&2
  exit 1
fi

startup_output="${test_root}/startup.output"
startup_marker="${test_root}/startup-workload-ran"
missing_script_directory="${test_root}/missing-script-directory"
run_case "${startup_output}" bash -c '
  set -Eeuo pipefail
  exec 3>&2
  source "$1"
  timestamp() { printf "test-time"; }
  format_duration() { printf "%ss" "$1"; }
  script_directory=$2
  workload() { printf "%s\n" ran > "$3"; }
  supabase_timed_in_shell_operation 2s "watchdog startup failure" workload
' shell "${library}" "${missing_script_directory}" "${startup_marker}"
[[ "${case_status}" == 125 ]]
[[ ! -e "${startup_marker}" ]]
grep --fixed-strings --quiet -- 'deadline helper failed to arm' "${startup_output}"

term_output="${test_root}/term.output"
descendant_pids="${test_root}/term-descendant-pids"
run_case "${term_output}" bash -c '
  set -Eeuo pipefail
  exec 3>&2
  script_directory="$(cd -- "$(dirname -- "$1")/.." && pwd -P)"
  source "$1"
  timestamp() { printf "test-time"; }
  format_duration() { printf "%ss" "$1"; }
  SUPABASE_CELL_KILL_AFTER=0.2s
  nested_term_ignoring_child() {
    (
      trap "" TERM
      sleep 30 &
      nested_sleep=$!
      printf "%s %s\n" "${BASHPID}" "${nested_sleep}" > "$2"
      wait "${nested_sleep}"
    ) &
    nested_shell=$!
    wait "${nested_shell}"
  }
  supabase_timed_in_shell_operation 0.15s "nested-child-deadline" \
    nested_term_ignoring_child ignored "$2"
' shell "${library}" "${descendant_pids}"
[[ "${case_status}" == 143 ]]
grep --fixed-strings --quiet -- \
  'STEP FAIL  nested-child-deadline (deadline expired; TERM, KILL after 0.2s)' \
  "${term_output}"
read -r nested_shell_pid nested_sleep_pid < "${descendant_pids}"
assert_process_gone "${nested_shell_pid}"
assert_process_gone "${nested_sleep_pid}"

late_output="${test_root}/late-descendant.output"
late_root_pid_path="${test_root}/late-root-pid"
late_descendant_pids="${test_root}/late-descendant-pids"
run_case "${late_output}" bash -c '
  set -Eeuo pipefail
  exec 3>&2
  script_directory="$(cd -- "$(dirname -- "$1")/.." && pwd -P)"
  source "$1"
  timestamp() { printf "test-time"; }
  format_duration() { printf "%ss" "$1"; }
  SUPABASE_CELL_KILL_AFTER=0.5s
  spawn_after_runner_exit() {
    (
      trap "" TERM
      sleep 0.22
      (
        trap "" TERM
        sleep 30 &
        late_sleep=$!
        printf "%s %s\n" "${BASHPID}" "${late_sleep}" > "$3"
        wait "${late_sleep}"
      ) &
      exit 0
    ) &
    late_root=$!
    printf "%s\n" "${late_root}" > "$2"
    wait "${late_root}"
  }
  supabase_timed_in_shell_operation 0.1s "late-descendant-deadline" \
    spawn_after_runner_exit ignored "$2" "$3"
' shell "${library}" "${late_root_pid_path}" "${late_descendant_pids}"
[[ "${case_status}" == 143 ]]
grep --fixed-strings --quiet -- \
  'STEP FAIL  late-descendant-deadline (deadline expired; TERM, KILL after 0.5s)' \
  "${late_output}"
for _iteration in {1..100}; do
  [[ -s "${late_descendant_pids}" ]] && break
  sleep 0.02
done
[[ -s "${late_descendant_pids}" ]]
late_root_pid="$(< "${late_root_pid_path}")"
read -r late_shell_pid late_sleep_pid < "${late_descendant_pids}"
assert_process_gone "${late_root_pid}"
assert_process_gone "${late_shell_pid}"
assert_process_gone "${late_sleep_pid}"

cleanup_output="${test_root}/cleanup.output"
cleanup_observation="${test_root}/cleanup-observation"
cleanup_finished="${test_root}/cleanup-finished"
run_case "${cleanup_output}" bash -c '
  set -Eeuo pipefail
  exec 3>&2
  script_directory="$(cd -- "$(dirname -- "$1")/.." && pwd -P)"
  source "$1"
  timestamp() { printf "test-time"; }
  format_duration() { printf "%ss" "$1"; }
  SUPABASE_CELL_KILL_AFTER=0.5s
  retained_state=owned-by-runner
  cleanup_observation_path=$2
  cleanup_finished_path=$3
  cleanup() {
    cleanup_status=$?
    printf "%s %s\n" "${cleanup_status}" "${retained_state}" > \
      "${cleanup_observation_path}"
    sleep 0.15
    printf "%s\n" finished > "${cleanup_finished_path}"
  }
  trap cleanup EXIT
  blocking_workload() { sleep 30; }
  supabase_timed_in_shell_operation 0.1s "cleanup-grace-deadline" \
    blocking_workload
' shell "${library}" "${cleanup_observation}" "${cleanup_finished}"
[[ "${case_status}" == 143 ]]
[[ "$(< "${cleanup_observation}")" == '143 owned-by-runner' ]]
[[ "$(< "${cleanup_finished}")" == finished ]]
grep --fixed-strings --quiet -- \
  'STEP FAIL  cleanup-grace-deadline (deadline expired; TERM, KILL after 0.5s)' \
  "${cleanup_output}"

kill_output="${test_root}/kill.output"
run_case "${kill_output}" bash -c '
  set -Eeuo pipefail
  exec 3>&2
  script_directory="$(cd -- "$(dirname -- "$1")/.." && pwd -P)"
  source "$1"
  timestamp() { printf "test-time"; }
  format_duration() { printf "%ss" "$1"; }
  SUPABASE_CELL_KILL_AFTER=0.2s
  ignore_term_in_runner() {
    trap "" TERM
    while :; do sleep 30; done
  }
  supabase_timed_in_shell_operation 0.15s "term-resistant-runner" \
    ignore_term_in_runner
' shell "${library}"
[[ "${case_status}" == 137 ]]
grep --fixed-strings --quiet -- \
  'STEP FAIL  term-resistant-runner (deadline expired; TERM, KILL after 0.2s)' \
  "${kill_output}"

python3 - "${deadline_helper}" << 'PY'
import os
import pathlib
import sys
import time

helper = os.fsencode(str(pathlib.Path(sys.argv[1]).resolve()))
deadline = time.monotonic() + 5
while True:
    leaked = []
    for command_line in pathlib.Path("/proc").glob("[0-9]*/cmdline"):
        try:
            pid = int(command_line.parent.name)
            arguments = command_line.read_bytes().split(b"\0")
        except (OSError, ValueError):
            continue
        if pid != os.getpid() and helper in arguments:
            leaked.append(pid)
    if not leaked:
        break
    if time.monotonic() >= deadline:
        raise SystemExit(f"deadline watchdog processes leaked: {leaked}")
    time.sleep(0.02)
PY

printf '%s\n' 'Supabase application timeout-boundary regression tests passed.'
