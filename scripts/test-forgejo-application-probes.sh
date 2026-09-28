#!/usr/bin/env bash
# Offline contract for the Forgejo behavior seam and its Podman adapter.
set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=scripts/lib/forgejo-application-probes.sh
source "${script_directory}/lib/forgejo-application-probes.sh"

[[ "$(type -t forgejo_probe_git)" == function ]]
[[ "$(type -t forgejo_remote || true)" != function ]]

callback_status=0
query_rows=1
observed=()

stub_wait() {
  [[ "$1" == 180 && "$2" == 'application health endpoint' ]] || return 89
  shift 2
  "$@"
}

stub_health() {
  observed=(health "$@")
  [[ "$*" == 'socket app http://127.0.0.1:3000/api/healthz' ]] || return 89
  return "${callback_status}"
}

stub_admin() {
  observed=(admin "$@")
  [[ "$*" == 'socket app user public-canary' ]] || return 89
  return "${callback_status}"
}

stub_git() {
  observed=(git "$@")
  [[ "$1 $2 $4 $5 $6 $7" == 'socket prefix user public-canary 13000 12222' ]] || return 89
  [[ "$3" == seed || "$3" == verify ]] || return 89
  return "${callback_status}"
}

stub_query() {
  [[ "$1" == socket && "$2" == database && "$3" == public-canary ]] || return 89
  [[ "$4" == "SELECT count(*) FROM repository WHERE lower_name = 'migration-baseline';" ]] || return 89
  ((callback_status == 0)) || return "${callback_status}"
  printf '%s\n' "${query_rows}"
}

assert_status() {
  local expected=$1
  shift
  local status=0
  "$@" || status=$?
  [[ "${status}" == "${expected}" ]] || {
    printf 'Expected Forgejo probe status %s, got %s.\n' "${expected}" "${status}" >&2
    return 1
  }
}

forgejo_probe_wait_application stub_wait stub_health socket app http://127.0.0.1:3000/api/healthz
[[ "${observed[*]}" == 'health socket app http://127.0.0.1:3000/api/healthz' ]]
forgejo_probe_create_admin stub_admin socket app user public-canary
[[ "${observed[*]}" == 'admin socket app user public-canary' ]]
forgejo_probe_git stub_git socket prefix seed user public-canary 13000 12222
[[ "${observed[*]}" == 'git socket prefix seed user public-canary 13000 12222' ]]
forgejo_probe_git stub_git socket prefix verify user public-canary 13000 12222
[[ "${observed[*]}" == 'git socket prefix verify user public-canary 13000 12222' ]]
forgejo_probe_assert_database stub_query socket database public-canary

callback_status=37
assert_status 37 forgejo_probe_wait_application stub_wait stub_health socket app http://127.0.0.1:3000/api/healthz
assert_status 37 forgejo_probe_create_admin stub_admin socket app user public-canary
assert_status 37 forgejo_probe_git stub_git socket prefix seed user public-canary 13000 12222
assert_status 37 forgejo_probe_git stub_git socket prefix verify user public-canary 13000 12222
assert_status 37 forgejo_probe_assert_database stub_query socket database public-canary
callback_status=0
for query_rows in 0 2 $'error\n1' $'1\n2'; do
  assert_status 1 forgejo_probe_assert_database stub_query socket database public-canary
done

# The adapter may be sourced without connecting to Podman. Its callbacks retain
# the existing native command shape while the shared module sees no runtime CLI.
# shellcheck disable=SC2034 # The sourced Podman adapter uses this caller-owned path.
repository_root="$(cd -- "${script_directory}/.." && pwd -P)"
# shellcheck source=scripts/lib/forgejo-application.sh
source "${script_directory}/lib/forgejo-application.sh"
forgejo_wait_for() {
  [[ "$1" == 180 && "$2" == 'application health endpoint' ]] || return 89
  shift 2
  "$@"
}
forgejo_remote() {
  observed=("$@")
  ((callback_status == 0)) || return "${callback_status}"
  if [[ "$2" == exec && "$3" == --env ]]; then
    [[ "${*: -8}" == *"SELECT count(*) FROM repository WHERE lower_name = 'migration-baseline';"* ]] || return 89
    printf '%s\n' "${query_rows}"
  fi
}

query_rows=1
forgejo_wait_application socket prefix
[[ "${observed[*]}" == 'socket exec prefix-forge-app wget --quiet --output-document=- http://127.0.0.1:3000/api/healthz' ]]
forgejo_create_admin socket prefix
[[ "${observed[*]}" == *'exec --user 1000:1000 prefix-forge-app forgejo admin user create --username boxferry-live'* ]]
forgejo_git_probe socket prefix seed
[[ "${observed[*]}" == *'run --rm --pull=never --network host --volume /tmp/boxferry-fixture/prefix:/fixture:rw'* ]]
[[ "${observed[*]}" == *'/fixture/git-probe.sh seed' ]]
forgejo_git_probe socket prefix verify
[[ "${observed[*]}" == *'/fixture/git-probe.sh verify' ]]
forgejo_assert_database socket prefix

callback_status=43
assert_status 43 forgejo_wait_application socket prefix
assert_status 43 forgejo_create_admin socket prefix
assert_status 43 forgejo_git_probe socket prefix seed
assert_status 43 forgejo_assert_database socket prefix

runner_steps="$(sed -n '/^run_forgejo_application_cell() {/,/^}/p' \
  "${script_directory}/lib/forgejo-application.sh" | grep -c '^[[:space:]]*progress_run ')"
[[ "${runner_steps}" == 25 ]]
mapfile -t stages < <(sed -n '/^run_forgejo_application_cell() {/,/^}/p' \
  "${script_directory}/lib/forgejo-application.sh" |
  sed -n "s/^[[:space:]]*progress_run '\([^']*\)'.*/\1/p")
[[ "${#stages[@]}" == 25 ]]
[[ "${stages[9]}" == 'create repository and prove HTTP plus SSH Git operations' ]]
[[ "${stages[10]}" == 'push and clone repository through HTTP and SSH' ]]
[[ "${stages[11]}" == 'prove Podman CLI database and publication boundaries' ]]
[[ "${stages[13]}" == 'prove Podman CLI repository persistence after recreation' ]]
[[ "${stages[19]}" == 'prove Docker Compose HTTP and SSH Git operations' ]]
[[ "${stages[20]}" == 'prove Docker Compose database and publication boundaries' ]]
[[ "${stages[22]}" == 'prove Docker Compose repository persistence after recreation' ]]
grep --fixed-strings --quiet 'progress_total=25' "${script_directory}/lib/forgejo-application.sh"
# shellcheck disable=SC2016 # Check literal source text, including parameter syntax.
[[ "$(grep --fixed-strings --count \
  'forgejo_assert_application_boundaries "${socket}" "${current_prefix}"' \
  "${script_directory}/lib/forgejo-application.sh")" == 2 ]]
