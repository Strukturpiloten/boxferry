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
if grep -Eq '\b(podman|docker|--network|--volume|--mount)\b' "${script_directory}/lib/nextcloud-application-probes.sh"; then
  printf '%s\n' 'Runtime-specific commands escaped into the shared Nextcloud probe contract.' >&2
  exit 1
fi

# Exercise the actual proxy fetch program offline, including its HTTP classification.
test_root="$(mktemp -d)"
trap 'rm -rf -- "${test_root}"' EXIT
mkdir "${test_root}/bin"
export NEXTCLOUD_TEST_ROOT="${test_root}"
cat > "${test_root}/bin/wget" << 'WGET'
#!/usr/bin/env bash
set -Eeuo pipefail
printf '%s\n' "$*" >> "${NEXTCLOUD_TEST_ROOT}/wget-arguments"
body=''
for argument in "$@"; do
  case "${argument}" in --output-document=*) body=${argument#*=} ;; esac
done
[[ -n "${body}" ]]
read -r response < "${NEXTCLOUD_TEST_ROOT}/responses"
if [[ "${response}" != persistent-502 ]]; then
  tail -n +2 "${NEXTCLOUD_TEST_ROOT}/responses" > "${NEXTCLOUD_TEST_ROOT}/remaining"
  mv "${NEXTCLOUD_TEST_ROOT}/remaining" "${NEXTCLOUD_TEST_ROOT}/responses"
fi
printf 'request\n' >> "${NEXTCLOUD_TEST_ROOT}/requests"
case "${response}" in
  200)
    printf '%s\n' '{"installed":true,"maintenance":false,"needsDbUpgrade":false,"versionstring":"32.0.10"}' > "${body}"
    printf '  HTTP/1.1 200 OK\n' >&2 ;;
  malformed) printf '%s\n' '{private-value-never-print' > "${body}" ;;
  marker-body) printf '%s\n' boxferry-nextcloud-temporary-http > "${body}" ;;
  wrong-version) printf '%s\n' '{"installed":true,"maintenance":false,"needsDbUpgrade":false,"versionstring":"32.0.9"}' > "${body}" ;;
  maintenance) printf '%s\n' '{"installed":true,"maintenance":true,"needsDbUpgrade":false,"versionstring":"32.0.10"}' > "${body}" ;;
  upgrade) printf '%s\n' '{"installed":true,"maintenance":false,"needsDbUpgrade":true,"versionstring":"32.0.10"}' > "${body}" ;;
  uninstalled) printf '%s\n' '{"installed":false,"maintenance":false,"needsDbUpgrade":false,"versionstring":"32.0.10"}' > "${body}" ;;
  multiple) printf '%s\n' '{"installed":false}' '{"installed":true,"maintenance":false,"needsDbUpgrade":false,"versionstring":"32.0.10"}' > "${body}" ;;
  ambiguous-headers)
    printf '  HTTP/1.1 200 OK\n  HTTP/1.1 502 Bad Gateway\n' >&2
    exit 1 ;;
  wget-error) printf '%s\n' 'unrelated wget error private-value-never-print' >&2; exit 1 ;;
  persistent-502 | 502 | 503 | 504 | 500)
    [[ "${response}" != persistent-502 ]] || response=502
    printf '%s\n' private-value-never-print > "${body}"
    printf '  HTTP/1.1 %s temporary\n' "${response}" >&2
    exit 1 ;;
  *) exit 97 ;;
esac
WGET
chmod +x "${test_root}/bin/wget"
export PATH="${test_root}/bin:${PATH}"
assert_status 2 nextcloud_status_remote 5 "${test_root}/absent-runtime-socket" exec private-value-never-print

python3 - "${script_directory}/lib/nextcloud-application.sh" "${test_root}" << 'PYTEST'
from pathlib import Path
import socket
import subprocess
import sys

library, root = Path(sys.argv[1]), Path(sys.argv[2])
with socket.socket(socket.AF_UNIX) as transport:
    path = root / "owned-status.sock"
    transport.bind(str(path))
    script = '''set -Eeuo pipefail
source "$1"
engine=fixture-engine
timed_operation() {
  [[ "$1" == "${expected}s" && "$3" == fixture-engine && "$4" == --url && "$5" == "unix://${owned}" ]]
}
owned=$2
expected=90
nextcloud_status_remote 180 "$owned" exec fixture
expected=3
nextcloud_status_remote 3 "$owned" exec fixture
'''
    # The injected timed operation only inspects arguments; no runtime is invoked.
    result = subprocess.run(["bash", "-c", script, "status-budget", str(library), str(path)], capture_output=True)
    assert result.returncode == 0, result.stderr
PYTEST

# Keep runtime identity/error simulation outside the HTTP request program.
nextcloud_status_remote() {
  local remaining=$1 socket=$2
  shift 2
  [[ "${socket}" == socket && "$1" == exec ]]
  printf '%s\n' "${remaining}" >> "${test_root}/remote-budgets"
  shift
  if [[ "$1" == --user ]]; then
    printf 'app\n' >> "${test_root}/app-requests"
    ((fixture_app_status == 0)) || return "${fixture_app_status}"
    stub_status socket prefix app
  else
    if ((fixture_transport_status != 0)); then
      printf '%s\n' private-value-never-print
      return "${fixture_transport_status}"
    fi
    shift # Exact proxy identity was supplied by the owning adapter.
    "$@"
  fi
}
nextcloud_probe_clock() { cat "${test_root}/clock"; }
sleep() {
  [[ "$1" == 1 || "$1" == 2 ]]
  printf '%s\n' "$(($(< "${test_root}/clock") + $1))" > "${test_root}/clock"
}
reset_status_fixture() {
  printf '%s\n' 0 > "${test_root}/clock"
  printf '%s\n' "$@" > "${test_root}/responses"
  : > "${test_root}/requests"
  : > "${test_root}/app-requests"
  : > "${test_root}/remote-budgets"
  : > "${test_root}/wget-arguments"
  fixture_app_status=0 fixture_transport_status=0
  fixture_version=32.0.10 fixture_maintenance=false status=0
}
run_status_fixture() {
  local expected=$1 deadline=$2 observed=0
  nextcloud_probe_wait_status nextcloud_podman_probe_status socket prefix "${deadline}" \
    > "${test_root}/stdout" 2> "${test_root}/stderr" || observed=$?
  [[ "${observed}" == "${expected}" ]]
  [[ ! -s "${test_root}/stdout" ]]
  ! grep -Fq private-value-never-print "${test_root}/stderr"
}
for transient in 502 503 504; do
  reset_status_fixture "${transient}" 200
  run_status_fixture 0 5
  [[ "$(wc -l < "${test_root}/requests")" == 2 ]]
  [[ "$(< "${test_root}/clock")" == 2 ]]
done
reset_status_fixture persistent-502
run_status_fixture 1 5
[[ "$(wc -l < "${test_root}/requests")" == 3 ]]
[[ "$(< "${test_root}/clock")" == 5 ]]
for fatal in malformed marker-body wrong-version maintenance upgrade uninstalled multiple ambiguous-headers wget-error 500; do
  reset_status_fixture "${fatal}" 200
  run_status_fixture 1 5
  [[ "$(wc -l < "${test_root}/requests")" == 1 ]]
  [[ "$(< "${test_root}/clock")" == 0 ]]
done
for fatal in 2 37 75 125; do
  reset_status_fixture 200
  fixture_transport_status=${fatal}
  expected_status=${fatal}
  ((fatal != 75)) || expected_status=1
  run_status_fixture "${expected_status}" 5
  [[ ! -s "${test_root}/requests" ]]
  [[ "$(wc -l < "${test_root}/app-requests")" == 1 ]]
  [[ "$(< "${test_root}/clock")" == 0 ]]
  reset_status_fixture 200
  fixture_app_status=${fatal}
  run_status_fixture "${expected_status}" 5
  [[ ! -s "${test_root}/requests" ]]
  [[ "$(wc -l < "${test_root}/app-requests")" == 1 ]]
done
# The final checks consume the existing edge wait's remaining allowance only.
reset_status_fixture 502 200
nextcloud_wait_for() {
  [[ "$1" == 600 || "$1" == 180 ]]
  [[ "$2" != 'edge proxy response' ]] || printf '%s\n' 177 > "${test_root}/clock"
}
nextcloud_wait_application socket prefix
[[ "$(< "${test_root}/clock")" == 179 ]]
[[ "$(tr '\n' ' ' < "${test_root}/remote-budgets")" == '3 3 1 1 ' ]]
grep -Fq -- '-T 3 --server-response' "${test_root}/wget-arguments"
grep -Fq -- '-T 1 --server-response' "${test_root}/wget-arguments"

reset_status_fixture 200
run_status_fixture 1 0
[[ ! -s "${test_root}/app-requests" && ! -s "${test_root}/requests" ]]
reset_status_fixture 200
nextcloud_wait_for() {
  [[ "$2" != 'edge proxy response' ]] || printf '%s\n' 180 > "${test_root}/clock"
}
assert_status 1 nextcloud_wait_application socket prefix
[[ ! -s "${test_root}/app-requests" && ! -s "${test_root}/requests" ]]
