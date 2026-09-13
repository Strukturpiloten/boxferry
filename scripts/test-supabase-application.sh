#!/usr/bin/env bash
# shellcheck disable=SC2016 # Inner Bash programs must expand only in their child shells.

set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
library="${script_directory}/lib/supabase-application.sh"
deadline_helper="${script_directory}/lib/in-shell-deadline.py"
test_root="$(mktemp -d)"
trap 'rm -rf -- "${test_root}"' EXIT
# shellcheck disable=SC2034 # Used by the sourced Supabase module.
repository_root="$(cd -- "${script_directory}/.." && pwd -P)"
# shellcheck source=scripts/lib/supabase-application.sh
source "${library}"

supabase_validate_catalogues

auth_source_reference="$(supabase_source_image_reference auth)"
auth_runtime_reference="$(supabase_image_reference auth)"
[[ "${auth_source_reference}" == docker.io/supabase/gotrue:v2.189.0@sha256:385184459f57569c54c25209f51f3b2be99ddd7c4ce9e3555b5d3eea8447b7cf ]]
[[ "${auth_runtime_reference}" == docker.io/supabase/gotrue:v2.189.0@sha256:0a8557cbe0fd53a067726fe656f79eb1b03a1ab3cdde4b59907ce5a1e1a202ab ]]
[[ "${auth_source_reference}" != "${auth_runtime_reference}" ]]

[[ "$(supabase_skopeo_source_reference "${auth_source_reference}")" == docker://docker.io/supabase/gotrue@sha256:385184459f57569c54c25209f51f3b2be99ddd7c4ce9e3555b5d3eea8447b7cf ]]
if supabase_source_image_reference missing-image > /dev/null 2>&1; then
  printf '%s\n' 'Unknown Supabase source image unexpectedly resolved.' >&2
  exit 1
fi

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

cli_database_argv="${test_root}/cli-database.argv"
bash -c '
  set -Eeuo pipefail
  source "$1"
  cli_output=$2
  supabase_remote() { printf "%s\\n" "$@" > "$cli_output"; }
  supabase_image_reference() { printf "%s" "test-db-image"; }
  supabase_create_cli_database test-socket test-prefix test-run
' bash "${library}" "${cli_database_argv}"
jq --raw-input --slurp --exit-status '
  split("\n")[:-1] as $argv |
  ($argv | index("POSTGRES_USER=supabase_admin")) and
  ($argv | index("POSTGRES_USER=postgres") | not) and
  ($argv | index("/tmp/boxferry-fixture/test-prefix/db-init.sql:/docker-entrypoint-initdb.d/zzzzzzzzzzzz-boxferry.sql:ro")) and
  (any($argv[]; contains("/docker-entrypoint-initdb.d/init-scripts/")) | not) and
  (any($argv[]; contains("/docker-entrypoint-initdb.d/99999999999999-boxferry.sql")) | not) and
  (any($argv[]; contains("80-boxferry.sql")) | not) and
  ($argv | index("pg_isready -U postgres -d postgres")) and
  ($argv | index("pg_isready -U supabase_admin -d postgres") | not)
' "${cli_database_argv}" > /dev/null

python3 - "$(supabase_fixture_root)/compose.yaml" \
  "${repository_root}/fixtures/scenarios/real-world-compose-supabase/scenario.toml" \
  "${cli_database_argv}" "$(supabase_fixture_root)/db-init.sql" \
  "$(supabase_fixture_root)/images.tsv" << 'PY'
import sys
import tomllib
import yaml

compose = yaml.safe_load(open(sys.argv[1], encoding="utf-8"))
db = compose["services"]["db"]
realtime = compose["services"]["realtime"]
rest = compose["services"]["rest"]
with open(sys.argv[2], "rb") as scenario_file:
    scenario = tomllib.load(scenario_file)
expected_command = next(
    command for command in scenario["semantics"]["required-commands"]
    if command["service"] == "db"
)
assert expected_command["kind"] == "exec"
with open(sys.argv[3], encoding="utf-8") as argv_file:
    cli_argv = argv_file.read().splitlines()
assert db["environment"]["POSTGRES_USER"] == "supabase_admin"
assert db["command"] == expected_command["values"]
assert cli_argv[-5:] == expected_command["values"]
assert db["healthcheck"]["test"][0] == "CMD-SHELL"
assert "PGPASSWORD=\"$$POSTGRES_PASSWORD\"" in db["healthcheck"]["test"][1]
assert "hostname -i" not in db["healthcheck"]["test"][1]
assert "-h 127.0.0.1 -U postgres -d postgres -tAc" in db["healthcheck"]["test"][1]
assert "127.0.0.1" in db["healthcheck"]["test"][1]
assert "current_user = 'postgres'" in db["healthcheck"]["test"][1]
assert "current_setting('config_file') = '/etc/postgresql/postgresql.conf'" in db["healthcheck"]["test"][1]
assert "rolname = 'supabase_read_only_user'" in db["healthcheck"]["test"][1]
assert "to_regclass('public.boxferry_items') IS NOT NULL" in db["healthcheck"]["test"][1]
assert "public.boxferry_bootstrap_complete WHERE singleton" in db["healthcheck"]["test"][1]
assert "pg_isready" not in db["healthcheck"]["test"][1]
db_init = open(sys.argv[4], encoding="utf-8").read()
assert "CREATE TABLE IF NOT EXISTS public.boxferry_bootstrap_complete" in db_init
assert db_init.rstrip().endswith("ON CONFLICT (singleton) DO NOTHING;")
assert rest["environment"]["PGRST_ADMIN_SERVER_HOST"] == "127.0.0.1"
assert rest["environment"]["PGRST_ADMIN_SERVER_HOST"] != "0.0.0.0"
assert rest["environment"]["PGRST_ADMIN_SERVER_PORT"] == "3001"
assert rest["healthcheck"]["test"] == ["CMD", "postgrest", "--ready"]
assert "PGRST_SERVER_HOST" not in rest["environment"]
assert "PGRST_DB_CHANNEL_ENABLED" not in rest["environment"]
assert realtime["environment"]["API_JWT_SECRET"] == "${BF_JWT_SECRET:?required}"
assert realtime["environment"]["METRICS_JWT_SECRET"] == "${BF_JWT_SECRET:?required}"
images = {
    row.split("\t", maxsplit=1)[0]: row.rstrip("\n").split("\t")
    for row in open(sys.argv[5], encoding="utf-8")
    if row and not row.startswith("#")
}
assert images["rest"] == [
    "rest",
    "docker.io/postgrest/postgrest:v16.3@sha256:ec0e25a4e24b0a3bc5e4f011369bfc736bd1b19f513bd01079b86329a7636962",
    "linux/amd64",
    "sha256:63b567a462c4fd81ede0bdff0b38a150f732ad5fe4f4b01cebeb6a1aa8dbe0d6",
    "application/vnd.docker.distribution.manifest.v2+json",
    "16.3",
    "MIT",
    "https://github.com/PostgREST/postgrest",
    "b42f5f50e3bdd0e949b40cd8e66eef4536776544",
    "nix/tools/docker/default.nix",
    "4cd8b5042b63438726d8853c90c522ac0958622ac2d440f252f02ee0d5bdc2c5",
    "not-redistributed-transient-test-pull",
    "registry labels absent; release tag and build file were reviewed independently but are not an image-to-source attestation",
]
authored_init_target = "/docker-entrypoint-initdb.d/zzzzzzzzzzzz-boxferry.sql"
authored_init_name = authored_init_target.rsplit("/", maxsplit=1)[1]
assert authored_init_name > "migrate.sh"
assert any(f"{authored_init_target}:ro,z" in mount for mount in db["volumes"])
assert all("/docker-entrypoint-initdb.d/init-scripts/" not in mount for mount in db["volumes"])
assert all(
    "/docker-entrypoint-initdb.d/99999999999999-boxferry.sql" not in mount
    for mount in db["volumes"]
)
assert all("80-boxferry.sql" not in mount for mount in db["volumes"])
PY

cli_services_argv="${test_root}/cli-services.argv"
bash -c '
  set -Eeuo pipefail
  source "$1"
  cli_output=$2
  supabase_remote() {
    printf "%s\\0" "$@" >> "${cli_output}"
    printf "\\0" >> "${cli_output}"
  }
  supabase_image_reference() { printf "%s" test-image; }
  supabase_wait_for() { shift 2; "$@"; }
  supabase_create_cli_services test-socket test-prefix test-run
' bash "${library}" "${cli_services_argv}"

python3 - "${cli_services_argv}" << 'PY'
import pathlib
import sys

records = [
    record.split(b"\0")
    for record in pathlib.Path(sys.argv[1]).read_bytes().split(b"\0\0")
    if record
]
realtime = next(
    record
    for record in records
    if b"--name" in record and b"test-prefix-supabase-realtime" in record
)
assert b"API_JWT_SECRET=boxferry-public-jwt-secret-at-least-thirty-two-characters" in realtime
assert b"METRICS_JWT_SECRET=boxferry-public-jwt-secret-at-least-thirty-two-characters" in realtime
PY
tr '\0' '\n' < "${cli_services_argv}" | grep --fixed-strings --quiet \
  'PGRST_ADMIN_SERVER_HOST=127.0.0.1'
tr '\0' '\n' < "${cli_services_argv}" | grep --fixed-strings --quiet \
  'PGRST_ADMIN_SERVER_PORT=3001'
tr '\0' '\n' < "${cli_services_argv}" | grep --fixed-strings --quiet \
  '["postgrest","--ready"]'
if tr '\0' '\n' < "${cli_services_argv}" | grep --fixed-strings --quiet \
  '["CMD","postgrest","--ready"]'; then
  printf '%s\n' 'PostgREST CLI health command retained the remote-unsafe CMD marker.' >&2
  exit 1
fi
if tr '\0' '\n' < "${cli_services_argv}" | grep --fixed-strings --quiet \
  'postgrest --ready'; then
  printf '%s\n' 'Native PostgREST healthcheck retained shell form.' >&2
  exit 1
fi
if tr '\0' '\n' < "${cli_services_argv}" | grep --fixed-strings --quiet \
  'PGRST_ADMIN_SERVER_HOST=0.0.0.0'; then
  printf '%s\n' 'Native PostgREST administrative host retained wildcard binding.' >&2
  exit 1
fi
for overridden_default in PGRST_SERVER_HOST PGRST_DB_CHANNEL_ENABLED; do
  if tr '\0' '\n' < "${cli_services_argv}" | grep --fixed-strings --quiet \
    "${overridden_default}="; then
    printf 'Native PostgREST CLI unexpectedly overrides %s.\n' "${overridden_default}" >&2
    exit 1
  fi
done

entrypoint_order_root="${test_root}/entrypoint-order"
mkdir -p -- "${entrypoint_order_root}/init-scripts" "${entrypoint_order_root}/migrations"
: > "${entrypoint_order_root}/migrate.sh"
: > "${entrypoint_order_root}/init-scripts/99999999999999-boxferry.sql"
: > "${entrypoint_order_root}/zzzzzzzzzzzz-boxferry.sql"
mapfile -t dispatched_init_files < <(
  export LC_ALL=C
  for init_file in "${entrypoint_order_root}"/*; do
    case "${init_file}" in
      *.sh | *.sql) basename -- "${init_file}" ;;
    esac
  done
)
[[ "${dispatched_init_files[*]}" == 'migrate.sh zzzzzzzzzzzz-boxferry.sql' ]]
: > "${entrypoint_order_root}/99999999999999-boxferry.sql"
mapfile -t early_init_order < <(
  export LC_ALL=C
  for init_file in "${entrypoint_order_root}"/*; do
    case "${init_file}" in
      *.sh | *.sql) basename -- "${init_file}" ;;
    esac
  done
)
[[ "${early_init_order[*]}" == '99999999999999-boxferry.sql migrate.sh zzzzzzzzzzzz-boxferry.sql' ]]

database_failure_output="${test_root}/database-failure.output"
bash -c '
  set -Eeuo pipefail
  source "$1"
  supabase_remote() {
    case "$2" in
      inspect) printf "status=exited exit=1 error=database bootstrap failed" ;;
      logs)
        [[ "$3" == --tail && "$4" == 20 && "$5" == test-prefix-supabase-db ]]
        printf "password boxferry-public-supabase-db-password email boxferry-supabase@example.invalid "
      printf "%*s" "$((SUPABASE_DIAGNOSTIC_CAPTURE_BYTES + 512))" "" | tr " " x
        printf "unbounded-tail-marker"
        ;;
    esac
  }
  supabase_report_database_failure_evidence test-socket test-prefix
' bash "${library}" > "${database_failure_output}" 2>&1
grep --fixed-strings --quiet -- 'status=exited exit=1 error=database bootstrap failed' \
  "${database_failure_output}"
grep --fixed-strings --quiet -- 'bounded log tail:' "${database_failure_output}"
grep --fixed-strings --quiet -- '[REDACTED]' "${database_failure_output}"
if grep --fixed-strings --quiet -- 'boxferry-public-supabase-db-password' \
  "${database_failure_output}"; then
  printf '%s\n' 'Supabase database failure diagnostics leaked a protected value.' >&2
  exit 1
fi
if grep --fixed-strings --quiet -- 'boxferry-supabase@example.invalid' \
  "${database_failure_output}"; then
  printf '%s\n' 'Supabase database failure diagnostics leaked the test identity.' >&2
  exit 1
fi
if grep --fixed-strings --quiet -- 'unbounded-tail-marker' "${database_failure_output}"; then
  printf '%s\n' 'Supabase database failure diagnostics did not cap a long log line.' >&2
  exit 1
fi
[[ "$(wc -c < "${database_failure_output}")" -le "$((SUPABASE_DIAGNOSTIC_OUTPUT_BYTES + 512))" ]]

database_contract_failure_output="${test_root}/database-contract-failure.output"
bash -c '
  set -Eeuo pipefail
  source "$1"
  supabase_database_sql_contract() {
    printf "password boxferry-public-supabase-db-password email boxferry-supabase@example.invalid "
    printf "%*s" "$((SUPABASE_DIAGNOSTIC_CAPTURE_BYTES + 512))" | tr " " x
    printf "unbounded-contract-marker"
    return 2
  }
  supabase_report_database_contract_failure test-socket test-prefix
' bash "${library}" > "${database_contract_failure_output}" 2>&1
grep --fixed-strings --quiet -- 'final SQL probe: exit=2' \
  "${database_contract_failure_output}"
grep --fixed-strings --quiet -- '[REDACTED]' "${database_contract_failure_output}"
if grep --fixed-strings --quiet -- 'boxferry-public-supabase-db-password' \
  "${database_contract_failure_output}"; then
  printf '%s\n' 'Supabase SQL-contract diagnostics leaked protected value.' >&2
  exit 1
fi
if grep --fixed-strings --quiet -- 'boxferry-supabase@example.invalid' \
  "${database_contract_failure_output}"; then
  printf '%s\n' 'Supabase SQL-contract diagnostics leaked test identity.' >&2
  exit 1
fi
if grep --fixed-strings --quiet -- 'unbounded-contract-marker' \
  "${database_contract_failure_output}"; then
  printf '%s\n' 'Supabase SQL-contract diagnostics did not cap probe output.' >&2
  exit 1
fi
[[ "$(wc -c < "${database_contract_failure_output}")" -le "$((SUPABASE_DIAGNOSTIC_OUTPUT_BYTES + 512))" ]]

compose_failure_output="${test_root}/compose-failure.output"
compose_attempt_marker="${test_root}/compose-attempted"
compose_peer_marker="${test_root}/compose-peer-ran"
bash -c '
  set -Eeuo pipefail
  source "$1"
  current_case=$2
  compose_attempt_marker=$3
  compose_peer_marker=$4
  supabase_assert_clean_prefix() { :; }
  supabase_wait_for() { shift 2; "$@" > /dev/null 2>&1; }
  supabase_image_reference() { printf "%s" test-db-image; }
  supabase_remote() {
    case "$2" in
      network) : ;;
      run)
        printf "password boxferry-public-supabase-db-password email boxferry-supabase@example.invalid " >&2
        printf "%*s" "$((SUPABASE_DIAGNOSTIC_CAPTURE_BYTES + 512))" | tr " " x >&2
        printf "unbounded-compose-contract-marker" >&2
        return 2
        ;;
      inspect) printf "status=exited exit=2 error=" ;;
      logs) printf "database bootstrap failed" ;;
    esac
  }
  supabase_compose_project() {
    if [[ " $* " == *" up --detach db "* ]]; then
      : > "${compose_attempt_marker}"
      return 0
    fi
    printf "%s\\n" "full-graph-started" >> "${compose_attempt_marker}"
    return 42
  }
  supabase_peer_compose_project() { : > "${compose_peer_marker}"; }
  if supabase_provision_compose test-socket test-prefix test-run; then
    printf "%s\n" "Compose provisioning unexpectedly accepted a failed database startup." >&2
    exit 1
  fi
  [[ -e "${compose_attempt_marker}" ]]
  if grep --fixed-strings --quiet -- full-graph-started "${compose_attempt_marker}"; then
    printf "%s\\n" "Compose peer failure started the full graph." >&2
    exit 1
  fi
  [[ ! -e "${compose_peer_marker}" ]]
' bash "${library}" "${test_root}" "${compose_attempt_marker}" \
  "${compose_peer_marker}" > "${compose_failure_output}" 2>&1
grep --fixed-strings --quiet -- \
  'Supabase PostgreSQL failure evidence: status=exited exit=2 error=' \
  "${compose_failure_output}"
grep --fixed-strings --quiet -- 'final SQL probe: exit=2' "${compose_failure_output}"
grep --fixed-strings --quiet -- '[REDACTED]' "${compose_failure_output}"
if grep --fixed-strings --quiet -- 'boxferry-public-supabase-db-password' \
  "${compose_failure_output}"; then
  printf '%s\n' 'Compose SQL-contract diagnostics leaked protected value.' >&2
  exit 1
fi
if grep --fixed-strings --quiet -- 'boxferry-supabase@example.invalid' \
  "${compose_failure_output}"; then
  printf '%s\n' 'Compose SQL-contract diagnostics leaked test identity.' >&2
  exit 1
fi
if grep --fixed-strings --quiet -- 'unbounded-compose-contract-marker' \
  "${compose_failure_output}"; then
  printf '%s\n' 'Compose SQL-contract diagnostics did not cap probe output.' >&2
  exit 1
fi
compose_contract_line="$(grep --line-number --max-count=1 --fixed-strings \
  'final SQL probe:' "${compose_failure_output}" | cut -d: -f1)"
compose_evidence_line="$(grep --line-number --max-count=1 --fixed-strings \
  'failure evidence:' "${compose_failure_output}" | cut -d: -f1)"
((compose_contract_line < compose_evidence_line))
[[ "$(wc -c < "${compose_failure_output}")" -le "$((SUPABASE_DIAGNOSTIC_OUTPUT_BYTES * 3 + 1024))" ]]

compose_order_marker="${test_root}/compose-order.marker"
bash -c '
  set -Eeuo pipefail
  source "$1"
  current_case=$2
  marker=$3
  supabase_assert_clean_prefix() { :; }
  supabase_image_reference() { printf "%s" test-db-image; }
  supabase_wait_for() { shift 2; "$@"; }
  supabase_remote() {
    case "$2" in
      network) printf "edge-network\\n" >> "$marker" ;;
      run) printf "readiness-peer\\n" >> "$marker" ;;
      inspect) return 1 ;;
      *) return 1 ;;
    esac
  }
  supabase_compose_project() { printf "compose:%s\\n" "$*" >> "$marker"; }
  supabase_peer_compose_project() { printf "boundary-peer:%s\\n" "$*" >> "$marker"; }
  supabase_provision_compose test-socket test-prefix test-run
' bash "${library}" "${test_root}" "${compose_order_marker}"
mapfile -t compose_order < "${compose_order_marker}"
[[ "${compose_order[0]}" == edge-network ]]
[[ "${compose_order[1]}" == 'compose:test-socket test-prefix test-run up --detach db' ]]
[[ "${compose_order[2]}" == readiness-peer ]]
[[ "${compose_order[3]}" == 'compose:test-socket test-prefix test-run up --detach --remove-orphans' ]]
[[ "${compose_order[4]}" == 'boundary-peer:test-socket test-prefix test-run up --detach --remove-orphans' ]]

compose_lifecycle_marker="${test_root}/compose-lifecycle.marker"
compose_lifecycle_release="${test_root}/compose-lifecycle.release"
bash -c '
  set -Eeuo pipefail
  source "$1"
  current_case=$2
  marker=$3
  release=$4
  supabase_wait_for() { shift 2; "$@"; }
  supabase_database_sql_contract() { printf "peer-sql\n" >> "${marker}"; }
  supabase_compose_project() {
    if [[ " $* " == *" up --detach db "* ]]; then
      printf "database-start\n" >> "${marker}"
      return 0
    fi
    printf "full-graph-blocked\n" >> "${marker}"
    while [[ ! -e "${release}" ]]; do sleep 0.01; done
    printf "full-graph-unblocked\n" >> "${marker}"
  }
  supabase_remote() {
    case "$2" in
      inspect) printf "running\n" ;;
      healthcheck)
        printf "healthcheck:%s\n" "$4" >> "${marker}"
        if [[ "$4" == test-prefix-supabase-kong ]]; then : > "${release}"; fi
        ;;
      *) return 1 ;;
    esac
  }
  supabase_start_compose_graph test-socket test-prefix test-run \
    "${current_case}/database.log" "${current_case}/graph.log"
' bash "${library}" "${test_root}" "${compose_lifecycle_marker}" "${compose_lifecycle_release}"
grep --fixed-strings --quiet -- 'healthcheck:test-prefix-supabase-db' \
  "${compose_lifecycle_marker}"
grep --fixed-strings --quiet -- 'healthcheck:test-prefix-supabase-kong' \
  "${compose_lifecycle_marker}"
compose_lifecycle_kong_line="$(grep --line-number --max-count=1 --fixed-strings \
  'healthcheck:test-prefix-supabase-kong' "${compose_lifecycle_marker}" | cut -d: -f1)"
compose_lifecycle_unblocked_line="$(grep --line-number --max-count=1 --fixed-strings \
  full-graph-unblocked "${compose_lifecycle_marker}" | cut -d: -f1)"
((compose_lifecycle_kong_line < compose_lifecycle_unblocked_line))

compose_lifecycle_failure_marker="${test_root}/compose-lifecycle-failure.marker"
compose_lifecycle_failure_release="${test_root}/compose-lifecycle-failure.release"
if bash -c '
  set -Eeuo pipefail
  source "$1"
  current_case=$2
  marker=$3
  release=$4
  supabase_wait_for() { shift 2; "$@"; }
  supabase_database_sql_contract() { :; }
  supabase_report_database_failure_evidence() { :; }
  supabase_compose_project() {
    if [[ " $* " == *" up --detach db "* ]]; then return 0; fi
    while [[ ! -e "${release}" ]]; do sleep 0.01; done
    printf "provider-failed\n" >> "${marker}"
    return 42
  }
  supabase_remote() {
    case "$2" in
      inspect) printf "running\n" ;;
      healthcheck)
        printf "healthcheck-failed:%s\n" "$4" >> "${marker}"
        : > "${release}"
        return 1
        ;;
      *) return 1 ;;
    esac
  }
  supabase_start_compose_graph test-socket test-prefix test-run \
    "${current_case}/database.log" "${current_case}/graph.log"
' bash "${library}" "${test_root}" "${compose_lifecycle_failure_marker}" \
  "${compose_lifecycle_failure_release}"; then
  printf '%s\n' 'Failed Compose lifecycle healthcheck incorrectly accepted the graph.' >&2
  exit 1
fi
grep --fixed-strings --quiet -- 'healthcheck-failed:test-prefix-supabase-db' \
  "${compose_lifecycle_failure_marker}"
grep --fixed-strings --quiet -- provider-failed "${compose_lifecycle_failure_marker}"

compose_recreate_order_marker="${test_root}/compose-recreate-order.marker"
bash -c '
  set -Eeuo pipefail
  source "$1"
  current_case=$2
  marker=$3
  supabase_image_reference() { printf "%s" test-db-image; }
  supabase_wait_for() { shift 2; "$@"; }
  supabase_remote() {
    case "$2" in
      run) printf "readiness-peer\\n" >> "$marker" ;;
      inspect) return 1 ;;
      *) return 1 ;;
    esac
  }
  supabase_compose_project() { printf "compose:%s\\n" "$*" >> "$marker"; }
  supabase_wait_application() { printf "application-ready\\n" >> "$marker"; }
  supabase_enable_realtime_table() { printf "realtime-enabled\\n" >> "$marker"; }
  supabase_recreate_application compose test-socket test-prefix test-run
' bash "${library}" "${test_root}" "${compose_recreate_order_marker}"
mapfile -t compose_recreate_order < "${compose_recreate_order_marker}"
[[ "${compose_recreate_order[0]}" == 'compose:test-socket test-prefix test-run stop --timeout 30' ]]
[[ "${compose_recreate_order[1]}" == 'compose:test-socket test-prefix test-run rm --force' ]]
[[ "${compose_recreate_order[2]}" == 'compose:test-socket test-prefix test-run up --detach db' ]]
[[ "${compose_recreate_order[3]}" == readiness-peer ]]
[[ "${compose_recreate_order[4]}" == 'compose:test-socket test-prefix test-run up --detach --remove-orphans' ]]
[[ "${compose_recreate_order[5]}" == application-ready ]]
[[ "${compose_recreate_order[6]}" == realtime-enabled ]]

compose_recreate_failure_marker="${test_root}/compose-recreate-failure.marker"
bash -c '
  set -Eeuo pipefail
  source "$1"
  current_case=$2
  marker=$3
  supabase_image_reference() { printf "%s" test-db-image; }
  supabase_wait_for() { shift 2; "$@"; }
  supabase_remote() {
    case "$2" in
      run)
        printf "readiness-peer-failed\\n" >> "$marker"
        return 1
        ;;
      inspect) return 1 ;;
      *) return 1 ;;
    esac
  }
  supabase_compose_project() {
    printf "compose:%s\\n" "$*" >> "$marker"
  }
  supabase_report_database_contract_failure() { printf "contract-evidence\\n" >> "$marker"; }
  supabase_report_database_failure_evidence() { printf "database-evidence\\n" >> "$marker"; }
  supabase_wait_application() { printf "application-ready\\n" >> "$marker"; }
  supabase_enable_realtime_table() { printf "realtime-enabled\\n" >> "$marker"; }
  if supabase_recreate_application compose test-socket test-prefix test-run; then
    printf "%s\\n" "Compose recreation unexpectedly accepted failed peer readiness." >&2
    exit 1
  fi
' bash "${library}" "${test_root}" "${compose_recreate_failure_marker}"
grep --fixed-strings --quiet -- readiness-peer-failed "${compose_recreate_failure_marker}"
grep --fixed-strings --quiet -- contract-evidence "${compose_recreate_failure_marker}"
grep --fixed-strings --quiet -- database-evidence "${compose_recreate_failure_marker}"
if grep --fixed-strings --quiet \
  -e 'up --detach --remove-orphans' -e application-ready -e realtime-enabled -- \
  "${compose_recreate_failure_marker}"; then
  printf '%s\n' 'Compose recreation peer failure started or accepted the full graph.' >&2
  exit 1
fi

database_health_failure_marker="${test_root}/database-health-failure.marker"
bash -c '
  set -Eeuo pipefail
  source "$1"
  marker=$2
  supabase_wait_for() { return 1; }
  supabase_report_database_contract_failure() { printf "contract\n" >> "${marker}"; }
  supabase_report_database_failure_evidence() { printf "evidence\n" >> "${marker}"; }
  if supabase_wait_application test-socket test-prefix; then
    printf "%s\n" "Database health failure unexpectedly passed." >&2
    exit 1
  fi
' bash "${library}" "${database_health_failure_marker}"
[[ "$(tr '\n' ' ' < "${database_health_failure_marker}")" == 'contract evidence ' ]]

database_peer_success_marker="${test_root}/database-peer-success.marker"
bash -c '
  set -Eeuo pipefail
  source "$1"
  marker=$2
  supabase_wait_for() { shift 2; "$@"; }
  supabase_database_sql_contract() { printf "peer-sql\n" >> "${marker}"; }
  supabase_wait_running() { printf "running:%s\n" "$2" >> "${marker}"; }
  supabase_remote() {
    [[ "$1" == test-socket ]]
    shift
    if [[ "$1" == inspect || "$*" == *".State.Health.Status"* ]]; then
      printf "%s\n" "Supabase application readiness must not inspect cached health status." >&2
      return 1
    fi
    printf "remote:%s\n" "$*" >> "${marker}"
  }
  supabase_wait_application test-socket test-prefix
' bash "${library}" "${database_peer_success_marker}"
[[ "$(head -n 1 "${database_peer_success_marker}")" == peer-sql ]]
mapfile -t database_peer_success_order < "${database_peer_success_marker}"
[[ "${database_peer_success_order[*]}" == 'peer-sql remote:healthcheck run test-prefix-supabase-auth remote:healthcheck run test-prefix-supabase-rest remote:healthcheck run test-prefix-supabase-realtime remote:healthcheck run test-prefix-supabase-imgproxy remote:healthcheck run test-prefix-supabase-storage remote:healthcheck run test-prefix-supabase-functions remote:healthcheck run test-prefix-supabase-studio remote:healthcheck run test-prefix-supabase-kong running:test-prefix-supabase-meta running:test-prefix-supabase-supavisor remote:exec test-prefix-supabase-studio node -e fetch('\''http://meta:8080/health'\'').then(r=>{if(!r.ok)process.exit(1)}) remote:exec test-prefix-supabase-studio node -e fetch('\''http://supavisor:4000/api/health'\'').then(r=>{if(!r.ok)process.exit(1)})' ]]

service_health_failure_marker="${test_root}/service-health-failure.marker"
if bash -c '
  set -Eeuo pipefail
  source "$1"
  marker=$2
  supabase_wait_for() { shift 2; "$@"; }
  supabase_database_sql_contract() { printf "peer-sql\n" >> "${marker}"; }
  supabase_remote() {
    [[ "$2" == healthcheck && "$3" == run ]] || {
      printf "unexpected:%s\n" "$*" >> "${marker}"
      return 1
    }
    printf "healthcheck:%s\n" "$4" >> "${marker}"
    [[ "$4" != test-prefix-supabase-auth ]]
  }
  supabase_report_service_health_failure() { printf "report:%s\n" "$3" >> "${marker}"; }
  supabase_wait_running() { printf "running:%s\n" "$2" >> "${marker}"; }
  supabase_wait_application test-socket test-prefix
' bash "${library}" "${service_health_failure_marker}"; then
  printf '%s\n' 'Failed Supabase Auth healthcheck incorrectly allowed readiness.' >&2
  exit 1
fi
[[ "$(tr '\n' ' ' < "${service_health_failure_marker}")" == 'peer-sql healthcheck:test-prefix-supabase-auth report:auth ' ]]

rest_health_failure_marker="${test_root}/rest-health-failure.marker"
if bash -c '
  set -Eeuo pipefail
  source "$1"
  test_marker=$2
  supabase_wait_for() { shift 2; "$@"; }
  supabase_database_sql_contract() { printf "peer-sql\n" >> "${test_marker}"; }
  supabase_run_healthcheck() {
    printf "health:%s\n" "$2" >> "${test_marker}"
    [[ "$2" != test-prefix-supabase-rest ]]
  }
  supabase_wait_running() { printf "running:%s\n" "$2" >> "${marker}"; }
  supabase_remote() {
    case "$2" in
      healthcheck)
        printf "fresh-healthcheck:%s\n" "$4" >> "${test_marker}"
        [[ "$4" != test-prefix-supabase-rest ]]
        ;;
      exec)
        [[ "$3" == test-prefix-supabase-rest && "$4" == postgrest && "$5" == --ready ]]
        printf "direct-probe-succeeded\n" >> "${test_marker}"
        ;;
      inspect)
        case "$*" in
          *".Config.Healthcheck.Test"*) printf "[\"CMD\",\"postgrest\",\"--ready\"]\n" ;;
          *".Config.Env"*) printf "PGRST_ADMIN_SERVER_HOST=127.0.0.1\nPGRST_ADMIN_SERVER_PORT=3001\n" ;;
          *".State.Health.Log"*) printf "stored-health-log\n" ;;
          *) printf "status=running health=unhealthy\n" ;;
        esac
        ;;
      logs) printf "container-log\n" ;;
      *) printf "unexpected:%s\n" "$*" >> "${test_marker}"; return 1 ;;
    esac
  }
  supabase_wait_application test-socket test-prefix
' bash "${library}" "${rest_health_failure_marker}"; then
  printf '%s\n' 'Failed PostgREST healthcheck incorrectly allowed readiness.' >&2
  exit 1
fi
[[ "$(tr '\n' ' ' < "${rest_health_failure_marker}")" == 'peer-sql health:test-prefix-supabase-auth health:test-prefix-supabase-rest fresh-healthcheck:test-prefix-supabase-rest direct-probe-succeeded ' ]]

non_postgrest_health_output="${test_root}/non-postgrest-health.output"
non_postgrest_health_invocations="${test_root}/non-postgrest-health-invocations"
bash -c '
  set -Eeuo pipefail
  source "$1"
  invocation_file=$2
  supabase_remote() {
    printf "%s\n" "$*" >> "${invocation_file}"
    case "$2" in
      healthcheck)
        [[ "$3" == run && "$4" == test-prefix-supabase-auth ]]
        return 12
        ;;
      inspect)
        [[ "$*" != *".Config."* ]]
        printf "bounded inspect\n"
        ;;
      logs)
        [[ "$3" == --tail && "$4" == 20 && "$5" == test-prefix-supabase-auth ]]
        printf "bounded log\n"
        ;;
      *) return 1 ;;
    esac
  }
  supabase_report_service_health_failure test-socket test-prefix auth
' bash "${library}" "${non_postgrest_health_invocations}" \
  > "${non_postgrest_health_output}" 2>&1

[[ "$(wc -l < "${non_postgrest_health_invocations}")" == 4 ]]
grep --fixed-strings --quiet -- 'Supabase auth health failure: final healthcheck exit=12' \
  "${non_postgrest_health_output}"
if grep --fixed-strings --quiet -- ' exec ' "${non_postgrest_health_invocations}" ||
  grep --fixed-strings --quiet -- '.Config.' "${non_postgrest_health_invocations}" ||
  grep --extended-regexp --quiet -- 'PostgREST|PGRST_' "${non_postgrest_health_output}"; then
  printf '%s\n' 'Non-PostgREST health diagnostics invoked PostgREST evidence collection.' >&2
  exit 1
fi

service_health_diagnostic_output="${test_root}/service-health-diagnostic.output"
service_health_invocations="${test_root}/service-health-invocations"
bash -c '
  set -Eeuo pipefail
  source "$1"
  invocation_file=$2
  supabase_remote() {
    case "$2" in
      healthcheck)
        [[ "$3" == run && "$4" == test-prefix-supabase-rest ]]
        printf "healthcheck\n" >> "${invocation_file}"
        printf "password boxferry-public-supabase-db-password email boxferry-supabase@example.invalid "
        printf "%*s" "$((SUPABASE_DIAGNOSTIC_CAPTURE_BYTES + 512))" "" | tr " " x
        printf "unbounded-healthcheck-marker"
        return 12
        ;;
      exec)
        [[ "$3" == test-prefix-supabase-rest && "$4" == postgrest && "$5" == --ready ]]
        printf "postgrest-direct-probe\n" >> "${invocation_file}"
        printf "password boxferry-public-supabase-db-password email boxferry-supabase@example.invalid "
        printf "%*s" "$((SUPABASE_DIAGNOSTIC_CAPTURE_BYTES + 512))" "" | tr " " p
        printf "unbounded-postgrest-probe-marker"
        return 23
        ;;
      inspect)
        if [[ "$*" == *".Config.Healthcheck.Test"* ]]; then
          printf "[\"CMD\",\"postgrest\",\"--ready\"]\n"
        elif [[ "$*" == *".Config.Env"* ]]; then
          printf "PGRST_ADMIN_SERVER_HOST=127.0.0.1\nPGRST_ADMIN_SERVER_PORT=3001\n"
          printf "PGRST_DB_URI=postgres://authenticator:boxferry-public-supabase-db-password@db/postgres\n"
        elif [[ "$*" == *".State.Health.Log"* ]]; then
          printf "health-log password boxferry-public-supabase-db-password "
          printf "%*s" "$((SUPABASE_DIAGNOSTIC_CAPTURE_BYTES + 512))" "" | tr " " y
          printf "unbounded-health-log-marker"
        else
          printf "status=running health=unhealthy email boxferry-supabase@example.invalid "
          printf "%*s" "$((SUPABASE_DIAGNOSTIC_CAPTURE_BYTES + 512))" "" | tr " " z
          printf "unbounded-state-marker"
        fi
        ;;
      logs)
        [[ "$3" == --tail && "$4" == 20 && "$5" == test-prefix-supabase-rest ]]
        printf "container-log password boxferry-public-supabase-db-password "
        printf "%*s" "$((SUPABASE_DIAGNOSTIC_CAPTURE_BYTES + 512))" "" | tr " " q
        printf "unbounded-container-log-marker"
        ;;
      *) return 1 ;;
    esac
  }
  supabase_report_service_health_failure test-socket test-prefix rest
' bash "${library}" "${service_health_invocations}" > "${service_health_diagnostic_output}" 2>&1
[[ "$(wc -l < "${service_health_invocations}")" == 2 ]]
grep --fixed-strings --quiet -- 'Supabase rest health failure: final healthcheck exit=12' \
  "${service_health_diagnostic_output}"
grep --fixed-strings --quiet -- 'PostgREST direct configured-command probe: exit=23' \
  "${service_health_diagnostic_output}"
grep --fixed-strings --quiet -- 'stored health test: ["CMD","postgrest","--ready"]' \
  "${service_health_diagnostic_output}"
grep --fixed-strings --quiet -- 'admin configuration: PGRST_ADMIN_SERVER_HOST=127.0.0.1' \
  "${service_health_diagnostic_output}"
if grep --fixed-strings --quiet -- 'PGRST_DB_URI' "${service_health_diagnostic_output}"; then
  printf '%s\n' 'PostgREST diagnostics exposed non-administrative configuration.' >&2
  exit 1
fi
grep --fixed-strings --quiet -- 'diagnostic health log:' "${service_health_diagnostic_output}"
grep --fixed-strings --quiet -- 'bounded container log tail:' "${service_health_diagnostic_output}"
grep --fixed-strings --quiet -- '[REDACTED]' "${service_health_diagnostic_output}"
for protected in boxferry-public-supabase-db-password boxferry-supabase@example.invalid \
  unbounded-healthcheck-marker unbounded-health-log-marker unbounded-state-marker \
  unbounded-container-log-marker unbounded-postgrest-probe-marker; do
  if grep --fixed-strings --quiet -- "${protected}" "${service_health_diagnostic_output}"; then
    printf 'Supabase service health diagnostics leaked or exceeded their bound: %s\n' \
      "${protected}" >&2
    exit 1
  fi
done
[[ "$(wc -c < "${service_health_diagnostic_output}")" -le "$((SUPABASE_DIAGNOSTIC_OUTPUT_BYTES * 7 + 1536))" ]]

service_health_boundary_output="${test_root}/service-health-boundary.output"
bash -c '
  set -Eeuo pipefail
  source "$1"
  supabase_remote() {
    if [[ "$2" == healthcheck ]]; then
      printf "healthcheck-failed"
      return 12
    fi
    if [[ "$2" == exec ]]; then
      for ((index = 0; index < 230; index++)); do
        printf "%s" "${SUPABASE_DB_PASSWORD}"
      done
      return 23
    fi
    printf "bounded diagnostic"
  }
  supabase_report_service_health_failure test-socket test-prefix rest
' bash "${library}" > "${service_health_boundary_output}" 2>&1
grep --fixed-strings --quiet -- 'final healthcheck exit=12' \
  "${service_health_boundary_output}"
grep --fixed-strings --quiet -- 'PostgREST direct configured-command probe: exit=23' \
  "${service_health_boundary_output}"
grep --fixed-strings --quiet -- '[REDACTED]' "${service_health_boundary_output}"
if grep --fixed-strings --quiet -- 'boxferry-public' "${service_health_boundary_output}"; then
  printf '%s\n' 'Boundary-split protected healthcheck value leaked.' >&2
  exit 1
fi

service_health_interruption_root="${test_root}/service-health-interruption"
mkdir -p -- "${service_health_interruption_root}"
set +e
TMPDIR="${service_health_interruption_root}" timeout --signal=TERM --kill-after=1s 1s \
  bash -c '
    set -Eeuo pipefail
    source "$1"
    supabase_remote() {
      if [[ "$2" == healthcheck ]]; then
        printf "%s" "${SUPABASE_DB_PASSWORD}"
        sleep 30
      fi
    }
    supabase_report_service_health_failure test-socket test-prefix rest
  ' bash "${library}" > /dev/null 2>&1
service_health_interruption_status=$?
set -e
if [[ "${service_health_interruption_status}" -ne 124 ]]; then
  printf 'Interrupted health reporter returned unexpected status: %s\n' \
    "${service_health_interruption_status}" >&2
  exit 1
fi
if find "${service_health_interruption_root}" -mindepth 1 -print -quit | grep --quiet .; then
  printf '%s\n' 'Interrupted health reporter retained a raw diagnostic capture.' >&2
  exit 1
fi

database_peer_failure_marker="${test_root}/database-peer-failure.marker"
bash -c '
  set -Eeuo pipefail
  source "$1"
  marker=$2
  supabase_wait_for() { shift 2; "$@"; }
  supabase_database_sql_contract() { printf "peer-sql-failed\n" >> "${marker}"; return 1; }
  supabase_report_database_contract_failure() { printf "contract-evidence\n" >> "${marker}"; }
  supabase_report_database_failure_evidence() { printf "state-log-evidence\n" >> "${marker}"; }
  supabase_run_healthcheck() { printf "non-db-health-check\n" >> "${marker}"; return 1; }
  supabase_wait_running() { printf "non-db-running-check\n" >> "${marker}"; return 1; }
  supabase_remote() { printf "non-db-http-check\n" >> "${marker}"; return 1; }
  if supabase_wait_application test-socket test-prefix; then
    printf "%s\n" "Failed peer SQL readiness unexpectedly passed." >&2
    exit 1
  fi
' bash "${library}" "${database_peer_failure_marker}"
[[ "$(tr '\n' ' ' < "${database_peer_failure_marker}")" == 'peer-sql-failed contract-evidence state-log-evidence ' ]]

database_contract_marker="${test_root}/database-contract.marker"
database_contract_argv="${test_root}/database-contract.argv"
bash -c '
  set -Eeuo pipefail
  source "$1"
  marker=$2
  probe_argv=$3
  supabase_create_cli_database() { :; }
  supabase_image_reference() { printf "%s" test-db-image; }
  supabase_wait_for() { shift 2; "$@"; }
  supabase_report_database_failure_evidence() { :; }
  supabase_remote() {
    if [[ "$2" == run && " $* " == *psql* ]]; then
      printf "%s\\n" "$@" > "$probe_argv"
      printf "sql-contract-failed\\n" >> "$marker"
      return 1
    elif [[ "$2" == run ]]; then
      printf "dependent-started\\n" >> "$marker"
      return 1
    fi
  }
  if supabase_create_cli_services test-socket test-prefix test-run; then
    printf "%s\\n" "SQL contract failure incorrectly started dependents." >&2
    exit 1
  fi
' bash "${library}" "${database_contract_marker}" "${database_contract_argv}"
grep --fixed-strings --quiet -- 'sql-contract-failed' "${database_contract_marker}"
if grep --fixed-strings --quiet -- 'dependent-started' "${database_contract_marker}"; then
  printf '%s\n' 'Supabase PostgreSQL SQL contract failure started a dependent service.' >&2
  exit 1
fi
jq --raw-input --slurp --exit-status '
  split("\n")[:-1] as $argv |
  ($argv | index("--rm")) and
  ($argv | index("--pull=never")) and
  ($argv | index("--name")) and
  ($argv | index("test-prefix-supabase-db-readiness-peer")) and
  ($argv | index("--network")) and
  ($argv | index("test-prefix-supabase-backend")) and
  ($argv | index("--entrypoint")) and
  ($argv | index("psql")) and
  ($argv | index("test-db-image")) and
  ($argv | index("--env")) and
  ($argv | index("PGPASSWORD=boxferry-public-supabase-db-password")) and
  ($argv | index("-h")) and
  ($argv | index("db")) and
  ($argv | index("-U")) and
  ($argv | index("postgres")) and
  (any($argv[]; contains("exec")) | not) and
  (any($argv[]; contains("hostname")) | not) and
  (any($argv[]; contains("127.0.0.1")) | not) and
  (any($argv[]; contains("current_setting"))) and
  (any($argv[]; contains("config_file"))) and
  (any($argv[]; contains("/etc/postgresql/postgresql.conf"))) and
  (any($argv[]; contains("supabase_read_only_user"))) and
  (any($argv[]; contains("public.boxferry_items"))) and
  (any($argv[]; contains("public.boxferry_bootstrap_complete WHERE singleton"))) and
  (any($argv[]; contains("pg_isready")) | not)
' "${database_contract_argv}" > /dev/null

archive_layout="${test_root}/archive-layout"
archive_path="${test_root}/auth.oci.tar"
archive_digest_file="${test_root}/auth.digest"
archive_reference=docker.io/supabase/gotrue:v2.189.0
archive_media_type=application/vnd.oci.image.manifest.v1+json
mkdir -p -- "${archive_layout}/blobs/sha256"
printf '%s' '{"schemaVersion":2,"config":{"digest":"sha256:test"},"layers":[]}' \
  > "${archive_layout}/manifest.json"
archive_digest="sha256:$(sha256sum "${archive_layout}/manifest.json" | awk '{ print $1 }')"
runtime_reference="${archive_reference}@${archive_digest}"
archive_size="$(stat -c '%s' "${archive_layout}/manifest.json")"
mv -- "${archive_layout}/manifest.json" \
  "${archive_layout}/blobs/sha256/${archive_digest#sha256:}"
printf '%s\n' '{"imageLayoutVersion":"1.0.0"}' > "${archive_layout}/oci-layout"
jq --null-input --arg digest "${archive_digest}" --arg reference "${runtime_reference}" \
  --arg media_type "${archive_media_type}" --argjson size "${archive_size}" '
    {
        schemaVersion: 2,
        manifests: [{
            mediaType: $media_type,
            digest: $digest,
            size: $size,
            annotations: {"org.opencontainers.image.ref.name": $reference}
        }]
    }
' > "${archive_layout}/index.json"
tar --create --file "${archive_path}" --directory "${archive_layout}" \
  index.json oci-layout blobs
printf '%s\n' "${archive_digest}" > "${archive_digest_file}"

jq '.manifests[0].annotations["org.opencontainers.image.ref.name"] = "changed.invalid/image:test"' \
  "${archive_layout}/index.json" > "${archive_layout}/index.changed.json"
mv -- "${archive_layout}/index.changed.json" "${archive_layout}/index.json"
tar --create --file "${test_root}/auth-changed.oci.tar" --directory "${archive_layout}" \
  index.json oci-layout blobs
if supabase_validate_image_archive auth "${test_root}/auth-changed.oci.tar" \
  "${runtime_reference}" "${archive_digest}" "${archive_media_type}" \
  "${archive_digest_file}" > /dev/null 2>&1; then
  printf '%s\n' 'Supabase archive validator accepted a changed descriptor reference.' >&2
  exit 1
fi

changed_digest=sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
jq --arg digest "${changed_digest}" --arg reference "${runtime_reference}" '
    .manifests[0].digest = $digest |
    .manifests[0].annotations["org.opencontainers.image.ref.name"] = $reference
' "${archive_layout}/index.json" > "${archive_layout}/index.changed.json"
mv -- "${archive_layout}/index.changed.json" "${archive_layout}/index.json"
tar --create --file "${test_root}/auth-changed-digest.oci.tar" --directory "${archive_layout}" \
  index.json oci-layout blobs
if supabase_validate_image_archive auth "${test_root}/auth-changed-digest.oci.tar" \
  "${runtime_reference}" "${archive_digest}" "${archive_media_type}" \
  "${archive_digest_file}" > /dev/null 2>&1; then
  printf '%s\n' 'Supabase archive validator accepted a changed descriptor digest.' >&2
  exit 1
fi
supabase_validate_image_archive auth "${archive_path}" "${runtime_reference}" \
  "${archive_digest}" "${archive_media_type}" "${archive_digest_file}"

printf '%s\n' 'sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff' \
  > "${archive_digest_file}"
if supabase_validate_image_archive auth "${archive_path}" "${runtime_reference}" \
  "${archive_digest}" "${archive_media_type}" \
  "${archive_digest_file}" > /dev/null 2>&1; then
  printf '%s\n' 'Supabase archive validator accepted a changed written digest.' >&2
  exit 1
fi
printf '%s\n' "${archive_digest}" > "${archive_digest_file}"

loaded_digest_mode=exact
write_valid_archive_index() {
  jq --null-input --arg digest "${archive_digest}" --arg reference "${runtime_reference}" \
    --arg media_type "${archive_media_type}" --argjson size "${archive_size}" '
      {schemaVersion: 2, manifests: [{
        mediaType: $media_type, digest: $digest, size: $size,
        annotations: {"org.opencontainers.image.ref.name": $reference}
      }]}
    ' > "${archive_layout}/index.json"
}

assert_rejected_archive() {
  local label=$1
  shift
  if supabase_validate_image_archive auth "$@" > /dev/null 2>&1; then
    printf 'Supabase archive validator accepted %s.\n' "${label}" >&2
    exit 1
  fi
}

write_valid_archive_index
jq '.manifests += [.manifests[0]]' "${archive_layout}/index.json" > "${archive_layout}/index.changed.json"
mv -- "${archive_layout}/index.changed.json" "${archive_layout}/index.json"
tar --create --file "${test_root}/auth-changed-index.oci.tar" --directory "${archive_layout}" index.json oci-layout blobs
assert_rejected_archive 'multiple index descriptors' "${test_root}/auth-changed-index.oci.tar" "${runtime_reference}" "${archive_digest}" "${archive_media_type}" "${archive_digest_file}"

write_valid_archive_index
jq '.manifests[0].mediaType = "application/invalid"' "${archive_layout}/index.json" > "${archive_layout}/index.changed.json"
mv -- "${archive_layout}/index.changed.json" "${archive_layout}/index.json"
tar --create --file "${test_root}/auth-changed-media.oci.tar" --directory "${archive_layout}" index.json oci-layout blobs
assert_rejected_archive 'a changed descriptor media type' "${test_root}/auth-changed-media.oci.tar" "${runtime_reference}" "${archive_digest}" "${archive_media_type}" "${archive_digest_file}"

write_valid_archive_index
jq '.manifests[0].size += 1' "${archive_layout}/index.json" > "${archive_layout}/index.changed.json"
mv -- "${archive_layout}/index.changed.json" "${archive_layout}/index.json"
tar --create --file "${test_root}/auth-changed-size.oci.tar" --directory "${archive_layout}" index.json oci-layout blobs
assert_rejected_archive 'a changed descriptor size' "${test_root}/auth-changed-size.oci.tar" "${runtime_reference}" "${archive_digest}" "${archive_media_type}" "${archive_digest_file}"

write_valid_archive_index
printf '%s' changed > "${archive_layout}/blobs/sha256/${archive_digest#sha256:}"
tar --create --file "${test_root}/auth-changed-blob.oci.tar" --directory "${archive_layout}" index.json oci-layout blobs
assert_rejected_archive 'a changed manifest blob' "${test_root}/auth-changed-blob.oci.tar" "${runtime_reference}" "${archive_digest}" "${archive_media_type}" "${archive_digest_file}"

# shellcheck disable=SC2329 # Invoked by the sourced loaded-image assertion.
engine_operation() {
  local description=$1 id runtime_argument="${!#}"
  if [[ "${description}" == "verify loaded Supabase "*" image" ]]; then
    id="${description#verify loaded Supabase }"
    id="${id% image}"
    [[ "${runtime_argument}" == "$(supabase_image_reference "${id}")" ]] || return 1
  fi
  if [[ "${description}" == "inspect loaded Supabase "*" image digest" ]]; then
    id="${description#inspect loaded Supabase }"
    id="${id% image digest}"
    [[ "${runtime_argument}" == "$(supabase_image_reference "${id}")" ]] || return 1
    if [[ "${loaded_digest_mode}" == changed && "${id}" == auth ]]; then
      printf '%s\n' 'sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'
    else
      awk -F '\t' -v id="${id}" '$1 == id { print $4; found = 1 } END { exit !found }' \
        "$(supabase_fixture_root)/images.tsv"
    fi
  fi
}
supabase_assert_loaded_images test-target
loaded_digest_mode=changed
if supabase_assert_loaded_images test-target > /dev/null 2>&1; then
  printf '%s\n' 'Supabase loaded-image check accepted a changed platform digest.' >&2
  exit 1
fi
unset -f engine_operation

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
