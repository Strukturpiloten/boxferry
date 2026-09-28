#!/usr/bin/env bash
set -Eeuo pipefail
script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=scripts/lib/nextcloud-application-probes.sh
source "${script_directory}/lib/nextcloud-application-probes.sh"
[[ "$(type -t nextcloud_probe_webdav_round_trip)" == function ]]
[[ "$(type -t nextcloud_remote || true)" != function ]]

status=0
fixture_rows=1
fixture_host=cache
fixture_stats=$'keyspace_hits:1\r\nkeyspace_misses:0\r'
fixture_version=32.0.10
fixture_maintenance=false
fixture_proxy_version=32.0.10
seen=()
stub_status() {
  [[ "$1 $2" == 'socket prefix' ]] || return 89
  [[ "$3" == app || "$3" == proxy ]] || return 89
  ((status == 0)) || return "${status}"
  local observed_version=${fixture_version}
  if [[ "$3" == proxy ]]; then observed_version=${fixture_proxy_version}; fi
  printf '{"installed":true,"maintenance":%s,"needsDbUpgrade":false,"versionstring":"%s"}\n' \
    "${fixture_maintenance}" "${observed_version}"
}
stub_webdav() {
  seen=("$@")
  return "${status}"
}
stub_rows() { printf '%s\n' "${fixture_rows}"; }
stub_host() { printf '%s\n' "${fixture_host}"; }
stub_stats() { printf '%s\n' "${fixture_stats}"; }
stub_rows_failed() { return 41; }
stub_host_failed() { return 42; }
stub_stats_failed() { return 43; }
assert_status() {
  local expected=$1 observed=0
  shift
  "$@" || observed=$?
  [[ "${observed}" == "${expected}" ]] || {
    printf 'Nextcloud status %s, expected %s.\n' "${observed}" "${expected}" >&2
    return 1
  }
}
nextcloud_probe_assert_status stub_status socket prefix
nextcloud_probe_webdav_round_trip stub_webdav socket prefix cli
[[ "${seen[*]}" == 'socket prefix boxferry-nextcloud-cli-payload true' ]]
nextcloud_probe_webdav_round_trip stub_webdav socket prefix compose false
[[ "${seen[*]}" == 'socket prefix boxferry-nextcloud-compose-payload false' ]]
nextcloud_probe_assert_database_cache stub_rows stub_host stub_stats socket prefix
fixture_version=32.0.9
assert_status 1 nextcloud_probe_assert_status stub_status socket prefix
fixture_version=32.0.10 fixture_proxy_version=32.0.9
assert_status 1 nextcloud_probe_assert_status stub_status socket prefix
fixture_proxy_version=32.0.10
fixture_version=32.0.10 fixture_maintenance=true
assert_status 1 nextcloud_probe_assert_status stub_status socket prefix
fixture_maintenance=false status=37
assert_status 37 nextcloud_probe_webdav_round_trip stub_webdav socket prefix cli
assert_status 37 nextcloud_probe_assert_status stub_status socket prefix
status=0 fixture_rows=0
assert_status 1 nextcloud_probe_assert_database_cache stub_rows stub_host stub_stats socket prefix
fixture_rows=$'1\n2'
assert_status 1 nextcloud_probe_assert_database_cache stub_rows stub_host stub_stats socket prefix
fixture_rows=1 fixture_host=other
assert_status 1 nextcloud_probe_assert_database_cache stub_rows stub_host stub_stats socket prefix
fixture_host=cache fixture_stats=$'keyspace_hits:0\nkeyspace_misses:0'
assert_status 1 nextcloud_probe_assert_database_cache stub_rows stub_host stub_stats socket prefix
assert_status 41 nextcloud_probe_assert_database_cache stub_rows_failed stub_host stub_stats socket prefix
assert_status 42 nextcloud_probe_assert_database_cache stub_rows stub_host_failed stub_stats socket prefix
assert_status 43 nextcloud_probe_assert_database_cache stub_rows stub_host stub_stats_failed socket prefix

repository_root="$(cd -- "${script_directory}/.." && pwd -P)"
# shellcheck source=scripts/lib/nextcloud-application.sh
source "${script_directory}/lib/nextcloud-application.sh"
[[ "$(type -t nextcloud_podman_probe_webdav)" == function ]]
[[ "$(grep -c '^[[:space:]]*progress_run ' <(sed -n '/^run_nextcloud_application_cell() {/,/^}/p' "${script_directory}/lib/nextcloud-application.sh"))" == 18 ]]
grep -Fq 'progress_total=18' "${script_directory}/lib/nextcloud-application.sh"
grep -Fq 'nextcloud_recreate_front_path compose' "${script_directory}/lib/nextcloud-application.sh"
! grep -Eq '\b(podman|docker|--network|--volume|--mount)\b' "${script_directory}/lib/nextcloud-application-probes.sh"
