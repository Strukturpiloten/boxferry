#!/usr/bin/env bash
set -Eeuo pipefail
script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=scripts/lib/paperless-application-probes.sh
source "${script_directory}/lib/paperless-application-probes.sh"
[[ "$(type -t paperless_probe_ingest_phase)" == function ]]
[[ "$(type -t paperless_remote || true)" != function ]]

status=0
counter_file="$(mktemp)"
trap 'rm -f -- "${counter_file}"' EXIT
printf '0\n' > "${counter_file}"
fixture_rows=3
seen=()
stub_probe() {
  seen=("$@")
  [[ "$1 $2" == 'socket prefix' ]] || return 89
  return "${status}"
}
stub_broker() {
  [[ "$1 $2" == 'socket prefix' ]] || return 89
  local calls
  calls="$(< "${counter_file}")"
  calls=$((calls + 1))
  printf '%s\n' "${calls}" > "${counter_file}"
  printf '%s\n' "$((calls + 1))"
}
stub_query() {
  [[ "$1 $2 $3" == 'socket database password' ]] || return 89
  [[ "$4" == "SELECT count(*) FROM documents_document WHERE title LIKE 'BoxFerry migration %';" ]] || return 89
  ((status == 0)) || return "${status}"
  printf '%s\n' "${fixture_rows}"
}
assert_status() {
  local expected=$1 observed=0
  shift
  "$@" || observed=$?
  [[ "${observed}" == "${expected}" ]] || {
    printf 'Paperless status %s, expected %s.\n' "${observed}" "${expected}" >&2
    return 1
  }
}

paperless_probe_wait_application stub_probe socket prefix
[[ "${seen[*]}" == 'socket prefix ready' ]]
paperless_probe_ingest_phase stub_probe stub_broker socket prefix baseline
[[ "${seen[*]}" == 'socket prefix ingest --phase baseline --state /fixture/probe-state.json --work-dir /fixture/generated-baseline' ]]
paperless_probe_verify_documents stub_probe socket prefix
[[ "${seen[*]}" == 'socket prefix verify --state /fixture/probe-state.json' ]]
paperless_probe_assert_database stub_query socket database password 3
fixture_rows=6
paperless_probe_assert_database stub_query socket database password 6
for fixture_rows in 0 2 4 $'3\n6' 'error 3'; do
  assert_status 1 paperless_probe_assert_database stub_query socket database password 3
done
assert_status 1 paperless_probe_assert_database stub_query socket database password 2
status=37
assert_status 37 paperless_probe_wait_application stub_probe socket prefix
assert_status 37 paperless_probe_ingest_phase stub_probe stub_broker socket prefix second
assert_status 37 paperless_probe_verify_documents stub_probe socket prefix
assert_status 37 paperless_probe_assert_database stub_query socket database password 3
status=0
stub_broker_unchanged() { printf '2\n'; }
stub_broker_invalid() { printf 'not-a-number\n'; }
stub_broker_failed() { return 41; }
stub_broker_fail_after() {
  local value calls
  value="$(stub_broker "$@")" || return $?
  calls="$(< "${counter_file}")"
  ((calls == 2)) && return 42
  printf '%s\n' "${value}"
}
assert_status 1 paperless_probe_ingest_phase stub_probe stub_broker_unchanged socket prefix baseline
assert_status 1 paperless_probe_ingest_phase stub_probe stub_broker_invalid socket prefix baseline
assert_status 41 paperless_probe_ingest_phase stub_probe stub_broker_failed socket prefix baseline
printf '0\n' > "${counter_file}"
assert_status 42 paperless_probe_ingest_phase stub_probe stub_broker_fail_after socket prefix baseline

# Static adapter and stage contracts prevent a callback-only test from masking
# changed application coverage or accidentally moving runtime flags into probes.
repository_root="$(cd -- "${script_directory}/.." && pwd -P)"
# shellcheck source=scripts/lib/paperless-application.sh
source "${script_directory}/lib/paperless-application.sh"
[[ "$(type -t paperless_podman_probe_query)" == function ]]
[[ "$(grep -c '^[[:space:]]*progress_run ' <(sed -n '/^run_paperless_application_cell() {/,/^}/p' "${script_directory}/lib/paperless-application.sh"))" == 35 ]]
grep -Fq 'progress_total=34' "${script_directory}/lib/paperless-application.sh"
# shellcheck disable=SC2016 # Literal source-code assertion.
grep -Fq 'paperless_assert_database "${socket}" "${current_prefix}" 6' "${script_directory}/lib/paperless-application.sh"
# shellcheck disable=SC2016 # Literal source-code assertion.
grep -Fq 'paperless_assert_database "${socket}" "${current_prefix}" 3' "${script_directory}/lib/paperless-application.sh"
! grep -Eq '\b(podman|docker|--network|--volume|--mount)\b' "${script_directory}/lib/paperless-application-probes.sh"
