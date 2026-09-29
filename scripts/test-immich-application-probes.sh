#!/usr/bin/env bash
set -Eeuo pipefail
script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=scripts/lib/immich-application-probes.sh
source "${script_directory}/lib/immich-application-probes.sh"
[[ "$(type -t immich_probe_ingest_phase)" == function ]]
[[ "$(type -t immich_remote || true)" != function ]]

status=0
ml_status=0
probe_calls=0
ml_calls=0
fixture_asset_rows=1
fixture_job_rows=1
fixture_extensions=$'cube:1.5\nearthdistance:1.2\nvchord:0.4.3\nvector:0.8.0'
seen=()
counter_file="$(mktemp)"
trap 'rm -f -- "${counter_file}"' EXIT
printf '0\n' > "${counter_file}"
stub_wait() {
  [[ "$1" == 480 && "$2" == 'API readiness and administrator login' ]] || return 89
  shift 2
  "$@"
}
stub_probe() {
  probe_calls=$((probe_calls + 1))
  seen=("$@")
  return "${status}"
}
stub_broker() {
  [[ "$1 $2" == 'socket prefix' ]] || return 89
  local calls
  calls="$(< "${counter_file}")"
  calls=$((calls + 1))
  printf '%s\n' "${calls}" > "${counter_file}"
  printf '%s\n' "$((calls + 5))"
}
stub_ml() {
  [[ "$1 $2" == 'socket prefix' ]] || return 89
  ml_calls=$((ml_calls + 1))
  return "${ml_status}"
}
stub_query() {
  [[ "$1 $2" == 'socket database' ]] || return 89
  case "$3" in
    'SELECT count(*) FROM asset;') printf '%s\n' "${fixture_asset_rows}" ;;
    'SELECT count(*) FROM asset_job_status;') printf '%s\n' "${fixture_job_rows}" ;;
    *pg_extension*) printf '%s\n' "${fixture_extensions}" ;;
    *) return 89 ;;
  esac
}
assert_status() {
  local expected=$1 observed=0
  shift
  "$@" || observed=$?
  [[ "${observed}" == "${expected}" ]] || {
    printf 'Immich status %s, expected %s.\n' "${observed}" "${expected}" >&2
    return 1
  }
}
immich_probe_wait_application stub_wait stub_probe socket prefix
[[ "${seen[*]}" == 'socket prefix ready' ]]
immich_probe_verify_asset stub_probe stub_ml socket prefix
[[ "${seen[*]}" == 'socket prefix verify --state /fixture/probe-state.json' ]]
immich_probe_ingest_phase stub_probe stub_broker stub_ml socket prefix baseline
[[ "${seen[*]}" == 'socket prefix ingest --phase baseline --state /fixture/probe-state.json --work-dir /fixture/generated-baseline' ]]
immich_probe_assert_database stub_query socket database 1
for mutation in asset job extension; do
  case "${mutation}" in
    asset) fixture_asset_rows=0 ;;
    job) fixture_job_rows=2 ;;
    extension) fixture_extensions=${fixture_extensions/vchord:0.4.3/vchord:0.4.2} ;;
  esac
  assert_status 1 immich_probe_assert_database stub_query socket database 1
  fixture_asset_rows=1 fixture_job_rows=1
  fixture_extensions=$'cube:1.5\nearthdistance:1.2\nvchord:0.4.3\nvector:0.8.0'
done
status=37
assert_status 37 immich_probe_wait_application stub_wait stub_probe socket prefix
assert_status 37 immich_probe_verify_asset stub_probe stub_ml socket prefix
ml_calls=0
assert_status 37 immich_probe_ingest_phase stub_probe stub_broker stub_ml socket prefix second
[[ "${ml_calls}" == 0 ]]
status=0
ml_status=43
assert_status 43 immich_probe_verify_asset stub_probe stub_ml socket prefix
ml_calls=0
assert_status 43 immich_probe_ingest_phase stub_probe stub_broker stub_ml socket prefix second
[[ "${ml_calls}" == 1 ]]
ml_status=0
stub_broker_unchanged() { printf '5\n'; }
stub_broker_invalid() { printf 'invalid\n'; }
stub_broker_failed() { return 41; }
stub_broker_fail_after() {
  local value calls
  value="$(stub_broker "$@")" || return $?
  calls="$(< "${counter_file}")"
  ((calls == 2)) && return 42
  printf '%s\n' "${value}"
}
assert_status 1 immich_probe_ingest_phase stub_probe stub_broker_unchanged stub_ml socket prefix baseline
assert_status 1 immich_probe_ingest_phase stub_probe stub_broker_invalid stub_ml socket prefix baseline
probe_calls=0
assert_status 41 immich_probe_ingest_phase stub_probe stub_broker_failed stub_ml socket prefix baseline
[[ "${probe_calls}" == 0 ]]
printf '0\n' > "${counter_file}"
ml_calls=0
assert_status 42 immich_probe_ingest_phase stub_probe stub_broker_fail_after stub_ml socket prefix baseline
[[ "${ml_calls}" == 0 ]]

repository_root="$(cd -- "${script_directory}/.." && pwd -P)"
# shellcheck source=scripts/lib/immich-application.sh
source "${script_directory}/lib/immich-application.sh"
[[ "$(type -t immich_podman_probe_query)" == function ]]
[[ "$(grep -c '^[[:space:]]*progress_run ' <(sed -n '/^run_immich_application_cell() {/,/^}/p' "${script_directory}/lib/immich-application.sh"))" == 39 ]]
grep -Fq 'progress_total=38' "${script_directory}/lib/immich-application.sh"
# shellcheck disable=SC2016 # Literal source-code assertion.
grep -Fq 'immich_verify_asset "${socket}" "${current_prefix}"' "${script_directory}/lib/immich-application.sh"
! grep -Eq '\b(podman|docker|--network|--volume|--mount)\b' "${script_directory}/lib/immich-application-probes.sh"
