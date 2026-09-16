#!/usr/bin/env bash
# shellcheck disable=SC2016 # Inner Bash programs must expand only in their child shells.

set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
library="${script_directory}/lib/supabase-application.sh"
deadline_helper="${script_directory}/lib/in-shell-deadline.py"
timed_operation_helper="${script_directory}/lib/timed-operation.sh"
test_root="$(mktemp -d)"
trap 'rm -rf -- "${test_root}"' EXIT
# shellcheck disable=SC2034 # Used by the sourced Supabase module.
repository_root="$(cd -- "${script_directory}/.." && pwd -P)"
# shellcheck source=scripts/lib/supabase-application.sh
source "${library}"

supabase_validate_catalogues

contract_report="${test_root}/success-contract.report.json"
contract_drift_report="${test_root}/success-contract-drift.report.json"
contract_drift_output="${test_root}/success-contract-drift.output"
supabase_success_contract_example_report podman podman storage contract false cli \
  > "${contract_report}"
jq '
  (
    .diagnostics[] |
    select(.code == "BFP0002") |
    .fields[] |
    select(
      .name == "subject" and
      .value == "container:contract-supabase-auth"
    ) |
    .value
  ) = "network:podman"
' "${contract_report}" > "${contract_drift_report}"
if supabase_assert_success_contract podman podman storage \
  "${contract_drift_report}" contract false cli > "${contract_drift_output}" 2>&1; then
  printf '%s\n' 'Supabase diagnostic tuple drift unexpectedly satisfied its exact contract.' >&2
  exit 1
fi
grep --fixed-strings --quiet \
  'Supabase route failed its exact success contract:' \
  "${contract_drift_output}"
grep --fixed-strings 'Supabase success-contract mismatch summary: ' \
  "${contract_drift_output}" | sed 's/^Supabase success-contract mismatch summary: //' |
  jq --exit-status \
    '.failed_predicates == [] and .fidelity_counters == {} and .invalid_diagnostic_names == 0' \
    > /dev/null
grep --fixed-strings --quiet \
  '"subject":"container:contract-supabase-auth","decision":"omitted","count":1' \
  "${contract_drift_output}"
grep --fixed-strings --quiet \
  '"subject":"network:podman","decision":"omitted","count":1' \
  "${contract_drift_output}"
if grep --fixed-strings --quiet -- "${SUPABASE_TEST_PASSWORD}" \
  "${contract_drift_output}"; then
  printf '%s\n' 'Supabase diagnostic contract failure output leaked a protected value.' >&2
  exit 1
fi

assert_contract_predicate_failure() {
  local name=$1 report=$2 expected_label=$3 expected_counter=${4:-} output mismatch
  output="${test_root}/${name}.output"
  if supabase_assert_success_contract podman podman storage \
    "${report}" contract false cli > "${output}" 2>&1; then
    printf 'Supabase %s predicate drift unexpectedly satisfied its exact contract.\n' "${name}" >&2
    exit 1
  fi
  mismatch="$(grep --fixed-strings 'Supabase success-contract mismatch summary: ' "${output}" |
    sed 's/^Supabase success-contract mismatch summary: //')"
  jq --exit-status --arg label "${expected_label}" \
    '.failed_predicates == [$label]' <<< "${mismatch}" > /dev/null
  if [[ -n "${expected_counter}" ]]; then
    jq --exit-status --argjson expected_counter "${expected_counter}" \
      '.fidelity_counters.unsupported == {expected: $expected_counter - 1, actual: $expected_counter}' \
      <<< "${mismatch}" > /dev/null
  else
    jq --exit-status '.fidelity_counters == {}' <<< "${mismatch}" > /dev/null
  fi
  if grep --fixed-strings --quiet -- "${SUPABASE_TEST_PASSWORD}" "${output}"; then
    printf 'Supabase %s mismatch output leaked a protected value.\n' "${name}" >&2
    exit 1
  fi
}

schema_drift_report="${test_root}/success-contract-schema-drift.report.json"
status_drift_report="${test_root}/success-contract-status-drift.report.json"
fidelity_shape_drift_report="${test_root}/success-contract-fidelity-shape-drift.report.json"
non_object_fidelity_drift_report="${test_root}/success-contract-non-object-fidelity-drift.report.json"
fidelity_counter_drift_report="${test_root}/success-contract-fidelity-counter-drift.report.json"
empty_name_drift_report="${test_root}/success-contract-empty-name-drift.report.json"
jq '.schema_version = 2' "${contract_report}" > "${schema_drift_report}"
jq --arg status "${SUPABASE_TEST_PASSWORD}" '.status = $status' \
  "${contract_report}" > "${status_drift_report}"
jq '.fidelity = {exact: 1}' "${contract_report}" > "${fidelity_shape_drift_report}"
jq '.fidelity = 1' "${contract_report}" > "${non_object_fidelity_drift_report}"
jq '.fidelity.unsupported += 1' "${contract_report}" > "${fidelity_counter_drift_report}"
jq '.diagnostics[0].name = ""' "${contract_report}" > "${empty_name_drift_report}"
assert_contract_predicate_failure schema-version "${schema_drift_report}" schema-version
assert_contract_predicate_failure status "${status_drift_report}" status
assert_contract_predicate_failure fidelity-shape "${fidelity_shape_drift_report}" fidelity-shape
assert_contract_predicate_failure non-object-fidelity \
  "${non_object_fidelity_drift_report}" fidelity-shape
assert_contract_predicate_failure fidelity-unsupported \
  "${fidelity_counter_drift_report}" fidelity-unsupported 1528
assert_contract_predicate_failure diagnostic-names \
  "${empty_name_drift_report}" diagnostic-names

bounded_mismatch_output="$(
  SUPABASE_DIAGNOSTIC_DELTA_OUTPUT_BYTES=8 \
    supabase_report_success_contract_mismatches podman podman storage \
    "${schema_drift_report}" contract false cli 2>&1
)"
grep --extended-regexp --quiet \
  'Supabase success-contract mismatch summary omitted: byte-limit=8; observed-bytes=[0-9]+; truncated=true\.' \
  <<< "${bounded_mismatch_output}"
if grep --fixed-strings --quiet -- "${SUPABASE_TEST_PASSWORD}" <<< "${bounded_mismatch_output}"; then
  printf '%s\n' 'Supabase bounded mismatch output leaked a protected value.' >&2
  exit 1
fi

cli_recreation_removal_argv="${test_root}/cli-recreation-removal.argv"
bash -c '
  set -Eeuo pipefail
  source "$1"
  output=$2
  supabase_remote() {
    printf "%s\0" "$@" >> "${output}"
    printf "\0" >> "${output}"
  }
  supabase_remove_cli_application_containers test-socket test-prefix
' bash "${library}" "${cli_recreation_removal_argv}"
python3 - "${cli_recreation_removal_argv}" << 'PY'
import pathlib
import sys

records = [
    record.split(b"\0")
    for record in pathlib.Path(sys.argv[1]).read_bytes().split(b"\0\0")
    if record
]
dependent_first = [
    f"test-prefix-supabase-{service}".encode()
    for service in (
        "kong",
        "studio",
        "functions",
        "supavisor",
        "meta",
        "storage",
        "imgproxy",
        "realtime",
        "rest",
        "auth",
        "db",
    )
]
assert records == [
    [b"test-socket", b"stop", b"--time", b"30", *dependent_first],
    [b"test-socket", b"rm", b"--force", *dependent_first],
]
arguments = {argument for record in records for argument in record}
assert b"test-prefix-supabase-pgdata" not in arguments
assert b"test-prefix-supabase-deno-cache" not in arguments
assert all(record[1] != b"volume" for record in records)
PY

cli_cleanup_argv="${test_root}/cli-cleanup.argv"
bash -c '
  set -Eeuo pipefail
  source "$1"
  output=$2
  containers_removed=0
  volumes_removed=0
  networks_removed=0
  supabase_remote() {
    printf "%s\0" "$@" >> "${output}"
    printf "\0" >> "${output}"
    case "$2" in
      rm)
        expected=(
          test-prefix-supabase-boundary-peer
          test-prefix-supabase-kong
          test-prefix-supabase-studio
          test-prefix-supabase-functions
          test-prefix-supabase-supavisor
          test-prefix-supabase-meta
          test-prefix-supabase-storage
          test-prefix-supabase-imgproxy
          test-prefix-supabase-realtime
          test-prefix-supabase-rest
          test-prefix-supabase-auth
          test-prefix-supabase-db
        )
        if [[ "$3" != --force || "$4" != --time || "$5" != 0 || "$6" != --ignore ||
          "$#" != 18 || "${*:7}" != "${expected[*]}" ]]; then
          return 125
        fi
        containers_removed=1
        ;;
      volume)
        if [[ "${containers_removed}" != 1 || "$3" != rm || "$4" != --force ]]; then
          return 125
        fi
        volumes_removed=1
        ;;
      network)
        if [[ "${containers_removed}" != 1 || "${volumes_removed}" != 1 || "$3" != rm ]]; then
          return 125
        fi
        networks_removed=$((networks_removed + 1))
        ;;
      *)
        printf "Unexpected cleanup command: %s\n" "$2" >&2
        return 1
        ;;
    esac
  }
  supabase_assert_clean_resources() {
    [[ "${containers_removed}" == 1 && "${volumes_removed}" == 1 &&
      "${networks_removed}" == 2 ]]
  }
  supabase_cleanup_mode cli test-socket test-prefix test-run
' bash "${library}" "${cli_cleanup_argv}"
python3 - "${cli_cleanup_argv}" << 'PY'
import pathlib
import sys

records = [
    record.split(b"\0")
    for record in pathlib.Path(sys.argv[1]).read_bytes().split(b"\0\0")
    if record
]
dependent_first = [
    f"test-prefix-supabase-{service}".encode()
    for service in (
        "kong",
        "studio",
        "functions",
        "supavisor",
        "meta",
        "storage",
        "imgproxy",
        "realtime",
        "rest",
        "auth",
        "db",
    )
]
assert records == [
    [
        b"test-socket",
        b"rm",
        b"--force",
        b"--time",
        b"0",
        b"--ignore",
        b"test-prefix-supabase-boundary-peer",
        *dependent_first,
    ],
    [
        b"test-socket",
        b"volume",
        b"rm",
        b"--force",
        b"test-prefix-supabase-pgdata",
        b"test-prefix-supabase-storage",
        b"test-prefix-supabase-deno-cache",
    ],
    [b"test-socket", b"network", b"rm", b"test-prefix-supabase-backend"],
    [b"test-socket", b"network", b"rm", b"test-prefix-supabase-edge"],
]
PY

node --test \
  "$(supabase_fixture_root)/realtime-readiness.test.mjs" \
  "$(supabase_fixture_root)/realtime-websocket.test.mjs"
[[ "${SUPABASE_REALTIME_DB_KEY}" == boxferry-rt-key1 ]]
[[ "$(LC_ALL=C printf '%s' "${SUPABASE_REALTIME_DB_KEY}" | wc -c)" == 16 ]]
compose_environment="$(supabase_compose_environment test-prefix test-run env)"
grep --fixed-strings --line-regexp --quiet \
  "BF_REALTIME_DB_KEY=${SUPABASE_REALTIME_DB_KEY}" <<< "${compose_environment}"
invalid_realtime_key_output="${test_root}/invalid-realtime-key.output"
valid_realtime_db_key=${SUPABASE_REALTIME_DB_KEY}
SUPABASE_REALTIME_DB_KEY=too-short
if supabase_validate_catalogues > "${invalid_realtime_key_output}" 2>&1; then
  printf '%s\n' 'Supabase catalogue accepted an invalid Realtime AES-128 key.' >&2
  exit 1
fi
SUPABASE_REALTIME_DB_KEY=${valid_realtime_db_key}
grep --fixed-strings --quiet \
  'Supabase Realtime DB encryption key must be exactly 16 bytes for AES-128.' \
  "${invalid_realtime_key_output}"

auth_source_reference="$(supabase_source_image_reference auth)"
auth_runtime_reference="$(supabase_image_reference auth)"
auth_observed_reference="$(supabase_podman_observed_image_reference auth)"
[[ "${auth_source_reference}" == docker.io/supabase/gotrue:v2.189.0@sha256:385184459f57569c54c25209f51f3b2be99ddd7c4ce9e3555b5d3eea8447b7cf ]]
[[ "${auth_runtime_reference}" == docker.io/supabase/gotrue:v2.189.0@sha256:0a8557cbe0fd53a067726fe656f79eb1b03a1ab3cdde4b59907ce5a1e1a202ab ]]
[[ "${auth_observed_reference}" == docker.io/supabase/gotrue@sha256:0a8557cbe0fd53a067726fe656f79eb1b03a1ab3cdde4b59907ce5a1e1a202ab ]]
[[ "${auth_source_reference}" != "${auth_runtime_reference}" ]]

digest_projection="$(supabase_expected_output_projection exact compose compose podman)"
grep --fixed-strings --line-regexp --quiet \
  $'auth\timage\tdocker.io/supabase/gotrue@sha256:0a8557cbe0fd53a067726fe656f79eb1b03a1ab3cdde4b59907ce5a1e1a202ab' \
  <<< "${digest_projection}"
if grep --fixed-strings --line-regexp --quiet \
  $'auth\timage\tdocker.io/supabase/gotrue:v2.189.0@sha256:0a8557cbe0fd53a067726fe656f79eb1b03a1ab3cdde4b59907ce5a1e1a202ab' \
  <<< "${digest_projection}"; then
  printf '%s\n' 'Podman-origin reimport projection restored an unavailable image tag.' >&2
  exit 1
fi

reimport_argument_log="${test_root}/reimport-semantics.tsv"
bash -c '
  set -Eeuo pipefail
  source "$1"
  current_case=$2
  argument_log=$3
  boxferry_bin=unused
  timed_operation() { return 0; }
  assert_successful_conversion() { :; }
  supabase_assert_success_contract() {
    [[ "$#" == 7 && "$1" =~ ^(compose|quadlet)$ && "$7" == not-podman ]]
  }
  supabase_assert_output_membership() { :; }
  supabase_assert_output_semantics() {
    [[ "$#" == 9 ]]
    printf "%s\t%s\t%s\t%s\t%s\t%s\n" \
      "$1" "$2" "$3" "$7" "$8" "$9" >> "${argument_log}"
  }
  supabase_run_reimports cli test-prefix
  supabase_run_reimports compose test-prefix
' bash "${library}" "${test_root}/reimport-case" "${reimport_argument_log}"
awk -F '\t' '
  NF != 6 ||
  $1 !~ /^(exact|storage|label|all)$/ ||
  $2 !~ /^(compose|quadlet)$/ ||
  $3 !~ /^(compose|quadlet|podman)$/ ||
  $4 != "podman" ||
  $6 !~ /^(cli|compose)$/ ||
  ($2 == "compose" && $5 != "false") ||
  ($2 == "quadlet" && $5 != "true") { bad = 1 }
  { seen[$6 ":" $2 "->" $3] = 1; count[$6]++; total++ }
  END {
    exit bad || total != 48 || count["cli"] != 24 || count["compose"] != 24 ||
      length(seen) != 12
  }
' "${reimport_argument_log}"

supabase_selection_includes_service exact supavisor
if supabase_selection_includes_service exact boundary-peer; then
  printf '%s\n' 'Supabase exact selection admitted the all-only boundary peer.' >&2
  exit 1
fi
for provisioner_mode in cli compose; do
  supabase_selection_includes_system_network "${provisioner_mode}" all
  if supabase_selection_includes_system_network "${provisioner_mode}" exact; then
    printf 'Supabase %s exact selection admitted the persistent system network.\n' \
      "${provisioner_mode}" >&2
    exit 1
  fi
done

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
studio = compose["services"]["studio"]
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
assert realtime["environment"]["DB_ENC_KEY"] == "${BF_REALTIME_DB_KEY:?required}"
assert realtime["networks"] == {
    "backend": {"aliases": ["realtime-dev.supabase-realtime", "realtime"]}
}
studio_health_command = (
    'node -e "fetch(\'http://127.0.0.1:3000/api/platform/profile\').then('
    '(r) => { if (!r.ok) process.exit(1) })"'
)
assert studio["healthcheck"] == {
    "test": ["CMD-SHELL", studio_health_command],
    "interval": "3s",
    "timeout": "5s",
    "retries": 150,
    "start_period": "10s",
}
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
studio = next(
    record
    for record in records
    if b"--name" in record and b"test-prefix-supabase-studio" in record
)
assert b"API_JWT_SECRET=boxferry-public-jwt-secret-at-least-thirty-two-characters" in realtime
assert b"METRICS_JWT_SECRET=boxferry-public-jwt-secret-at-least-thirty-two-characters" in realtime
assert b"DB_ENC_KEY=boxferry-rt-key1" in realtime
network = realtime.index(b"--network")
assert realtime[network + 1] == b"test-prefix-supabase-backend"
aliases = [
    realtime[index + 1]
    for index, argument in enumerate(realtime[:-1])
    if argument == b"--network-alias"
]
assert aliases == [b"realtime-dev.supabase-realtime", b"realtime"]
assert all(b":alias=" not in argument for argument in realtime)
studio_health_command = (
    b'node -e "fetch(\'http://127.0.0.1:3000/api/platform/profile\').then('
    b'(r) => { if (!r.ok) process.exit(1) })"'
)
health_command = studio.index(b"--health-cmd")
assert studio[health_command + 1] == studio_health_command
for option, value in (
    (b"--health-interval", b"3s"),
    (b"--health-timeout", b"5s"),
    (b"--health-retries", b"150"),
    (b"--health-start-period", b"10s"),
):
    index = studio.index(option)
    assert studio[index + 1] == value
assert all(not argument.startswith(b'["CMD') for argument in studio)
PY

published_api_transient_marker="${test_root}/published-api-transient.marker"
published_api_transient_argv="${test_root}/published-api-transient.argv"
bash -c '
  set -Eeuo pipefail
  source "$1"
  marker=$2
  argv=$3
  engine=test-engine
  attempts=0
  clock=0
  supabase_monotonic_seconds() { printf "%s\\n" "${clock}"; }
  sleep() { clock=$((clock + 1)); }
  timed_operation() {
    printf "%s\\0" "$@" >> "${argv}"
    printf "\\0" >> "${argv}"
    attempts=$((attempts + 1))
    printf "%s\\n" "${attempts}" >> "${marker}"
    ((attempts > 2))
  }
  supabase_probe_published_api test-outer test-socket test-prefix
  [[ "$(tr "\\n" " " < "${marker}")" == "1 2 3 " ]]
' bash "${library}" "${published_api_transient_marker}" "${published_api_transient_argv}"

published_api_timeout_output="${test_root}/published-api-timeout.output"
published_api_timeout_attempts="${test_root}/published-api-timeout-attempts.marker"
published_api_timeout_sleeps="${test_root}/published-api-timeout-sleeps.marker"
published_api_timeout_argv="${test_root}/published-api-timeout.argv"
if bash -c '
  set -Eeuo pipefail
  source "$1"
  attempts_marker=$2
  sleeps_marker=$3
  argv=$4
  diagnostic_marker=$5
  engine=test-engine
  clock=0
  supabase_monotonic_seconds() { printf "%s\\n" "${clock}"; }
  sleep() {
    printf "%s\\n" "$1" >> "${sleeps_marker}"
    clock=$((clock + $1))
  }
  timed_operation() {
    printf "%s\\0" "$@" >> "${argv}"
    printf "\\0" >> "${argv}"
    printf "attempt\\n" >> "${attempts_marker}"
    return 1
  }
  supabase_report_published_api_failure() {
    printf "%s\\n" "$*" >> "${diagnostic_marker}"
  }
  supabase_probe_published_api test-outer test-socket test-prefix
' bash "${library}" "${published_api_timeout_attempts}" "${published_api_timeout_sleeps}" \
  "${published_api_timeout_argv}" "${test_root}/published-api-timeout-diagnostic.marker" \
  > "${published_api_timeout_output}" 2>&1; then
  printf "%s\\n" "Published Supabase API probe unexpectedly passed permanently failing ingress." >&2
  exit 1
fi
grep --fixed-strings --quiet \
  'Timed out waiting for Supabase gateway through loopback publication.' \
  "${published_api_timeout_output}"
[[ "$(sed -n '$=' "${published_api_timeout_attempts}")" == 45 ]]
[[ "$(sed -n '$=' "${published_api_timeout_sleeps}")" == 45 ]]
[[ "$(cat "${test_root}/published-api-timeout-diagnostic.marker")" == 'test-outer test-socket test-prefix' ]]

published_api_late_argv="${test_root}/published-api-late.argv"
if bash -c '
  set -Eeuo pipefail
  source "$1"
  argv=$2
  engine=test-engine
  clock=0
  attempts=0
  supabase_monotonic_seconds() { printf "%s\\n" "${clock}"; }
  sleep() { clock=$((clock + $1)); }
  timed_operation() {
    printf "%s\\0" "$@" >> "${argv}"
    printf "\\0" >> "${argv}"
    attempts=$((attempts + 1))
    if ((attempts == 1)); then
      clock=87
    else
      clock=$((clock + ${1%s}))
    fi
    return 1
  }
  supabase_report_published_api_failure() { :; }
  supabase_probe_published_api test-outer test-socket test-prefix
' bash "${library}" "${published_api_late_argv}" > /dev/null 2>&1; then
  printf "%s\\n" "Late Supabase API probe unexpectedly passed." >&2
  exit 1
fi

python3 - "${published_api_transient_argv}" "${published_api_timeout_argv}" \
  "${published_api_late_argv}" << 'PY'
import pathlib
import sys

expected_tail = [
    "probe Supabase gateway through loopback publication",
    "test-engine",
    "exec",
    "test-outer",
    "curl",
    "--fail",
    "--silent",
    "--show-error",
    "http://127.0.0.1:18000/auth/v1/health",
]

def records(path):
    return [record.split(b"\0") for record in pathlib.Path(path).read_bytes().split(b"\0\0")[:-1]]

transient, permanent, late = map(records, sys.argv[1:])
transient = [[field.decode() for field in record] for record in transient]
permanent = [[field.decode() for field in record] for record in permanent]
late = [[field.decode() for field in record] for record in late]
assert transient == [
    ["90s", *expected_tail],
    ["89s", *expected_tail],
    ["88s", *expected_tail],
], transient
assert [record[0] for record in permanent] == [
    f"{seconds}s" for seconds in range(90, 0, -2)
], permanent
assert all(record[1:] == expected_tail for record in permanent), permanent
assert late == [
    ["90s", *expected_tail],
    ["1s", *expected_tail],
], late
PY

published_api_diagnostic_transport_argv="${test_root}/published-api-diagnostic-transport.argv"
bash -c '
  set -Eeuo pipefail
  source "$1"
  argv=$2
  engine=test-engine
  timed_operation() {
    printf "%s\\0" "$@" >> "${argv}"
    printf "\\0" >> "${argv}"
  }
  supabase_remote_diagnostic test-socket inspect test-container
  started_outer=test-outer
  supabase_remote_diagnostic test-missing-socket logs --tail 20 test-container
' bash "${library}" "${published_api_diagnostic_transport_argv}"

python3 - "${published_api_diagnostic_transport_argv}" << 'PY'
import pathlib
import sys

records = [
    [field.decode() for field in record.split(b"\0")]
    for record in pathlib.Path(sys.argv[1]).read_bytes().split(b"\0\0")[:-1]
]
assert records == [
    [
        "5s",
        "diagnose nested Podman inspect through acquisition socket",
        "test-engine",
        "--url",
        "unix://test-socket",
        "inspect",
        "test-container",
    ],
    [
        "5s",
        "diagnose nested Podman logs through matching container CLI",
        "test-engine",
        "exec",
        "test-outer",
        "podman",
        "logs",
        "--tail",
        "20",
        "test-container",
    ],
], records
PY

published_api_diagnostic_output="${test_root}/published-api-diagnostic.output"
published_api_diagnostic_argv="${test_root}/published-api-diagnostic.argv"
bash -c '
  set -Eeuo pipefail
  source "$1"
  argv=$2
  engine=test-engine
  supabase_remote_diagnostic() {
    printf "%s\\0" "$@" >> "${argv}"
    printf "\\0" >> "${argv}"
    case "$2" in
      inspect)
        printf "status=running exit=0 error=%s bindings=loopback runtime-ports=ready %s\\n" \
          "${SUPABASE_ANON_KEY}" "${SUPABASE_DB_PASSWORD}"
        ;;
      port)
        printf "8000/tcp -> 127.0.0.1:18000\\n"
        ;;
      logs)
        printf "Kong bounded log %s\\n" "${SUPABASE_TEST_PASSWORD}"
        ;;
    esac
  }
  timed_operation() {
    printf "%s\\0" "$@" >> "${argv}"
    printf "\\0" >> "${argv}"
    printf "/proc/net/tcp local=0100007F:4650 state=0A %s\\n" "${SUPABASE_SERVICE_KEY}"
    printf "/proc/net/tcp local=00000000:4650 state=0A\\n"
  }
  supabase_report_published_api_failure test-outer test-socket test-prefix
' bash "${library}" "${published_api_diagnostic_argv}" \
  > "${published_api_diagnostic_output}" 2>&1
grep --fixed-strings --quiet \
  'Kong inspection: status=running exit=0 error=[REDACTED] bindings=loopback runtime-ports=ready [REDACTED]' \
  "${published_api_diagnostic_output}"
grep --fixed-strings --quiet 'podman port: 8000/tcp -> 127.0.0.1:18000' \
  "${published_api_diagnostic_output}"
grep --fixed-strings --quiet \
  'outer published-port listeners: /proc/net/tcp local=0100007F:4650 state=0A [REDACTED]' \
  "${published_api_diagnostic_output}"
grep --fixed-strings --quiet '/proc/net/tcp local=00000000:4650 state=0A' \
  "${published_api_diagnostic_output}"
grep --fixed-strings --quiet 'bounded Kong log tail: Kong bounded log [REDACTED]' \
  "${published_api_diagnostic_output}"
for protected in \
  "${SUPABASE_DB_PASSWORD}" "${SUPABASE_ANON_KEY}" \
  "${SUPABASE_SERVICE_KEY}" "${SUPABASE_TEST_PASSWORD}"; do
  if grep --fixed-strings --quiet "${protected}" "${published_api_diagnostic_output}"; then
    printf 'Published API diagnostic leaked a protected fixture value.\n' >&2
    exit 1
  fi
done

python3 - "${published_api_diagnostic_argv}" << 'PY'
import pathlib
import sys

records = [
    [field.decode() for field in record.split(b"\0")]
    for record in pathlib.Path(sys.argv[1]).read_bytes().split(b"\0\0")[:-1]
]
assert len(records) == 4, records
assert records[0] == [
    "test-socket",
    "inspect",
    "--format",
    "status={{.State.Status}} exit={{.State.ExitCode}} error={{.State.Error}} "
    "bindings={{json .HostConfig.PortBindings}} runtime-ports={{json .NetworkSettings.Ports}}",
    "test-prefix-supabase-kong",
], records[0]
assert records[1] == ["test-socket", "port", "test-prefix-supabase-kong"], records[1]
assert records[2][0:3] == [
    "5s",
    "inspect outer Supabase published-port listeners",
    "test-engine",
], records[2]
assert records[2][3:6] == ["exec", "test-outer", "/bin/sh"], records[2]
assert records[2][-2:] == ["sh", "4650"], records[2]
assert records[3] == [
    "test-socket",
    "logs",
    "--tail",
    "20",
    "test-prefix-supabase-kong",
], records[3]
PY

dependency_order_root="${test_root}/dependency-order"
mkdir -p -- "${dependency_order_root}/reversed" "${dependency_order_root}/retained"
jq --null-input --arg prefix test-prefix \
  --argjson names '["auth","db","functions","imgproxy","meta","realtime","rest","storage","studio","kong","supavisor"]' \
  '{operations: [$names[] as $name | {
    action: "start_container",
    resource: {kind: "container", name: ($prefix + "-supabase-" + $name)}
  }]}' > "${dependency_order_root}/reversed/podman.json"
jq --null-input --arg prefix test-prefix \
  --argjson names '["db","auth","functions","imgproxy","meta","realtime","rest","storage","studio","kong","supavisor"]' \
  '{operations: [$names[] as $name | {
    action: "start_container",
    resource: {kind: "container", name: ($prefix + "-supabase-" + $name)}
  }]}' > "${dependency_order_root}/retained/podman.json"
if supabase_assert_podman_dependency_order exact \
  "${dependency_order_root}/reversed" test-prefix > /dev/null 2>&1; then
  printf '%s\n' 'Supabase dependency-order assertion admitted a reversed retained plan.' >&2
  exit 1
fi
supabase_assert_podman_dependency_order exact \
  "${dependency_order_root}/retained" test-prefix

[[ "$(supabase_reimport_dependency_order_required compose)" == false ]]
[[ "$(supabase_reimport_dependency_order_required quadlet)" == true ]]
if supabase_reimport_dependency_order_required unsupported > /dev/null 2>&1; then
  printf '%s\n' 'Supabase dependency expectation admitted an unsupported source.' >&2
  exit 1
fi

bash -c '
  set -Eeuo pipefail
  source "$1"
  supabase_expected_output_projection() { :; }
  supabase_podman_output_projection() { :; }
  dependency_calls=0
  supabase_assert_podman_dependency_order() {
    dependency_calls=$((dependency_calls + 1))
    return 1
  }

  if supabase_assert_output_graph all compose podman unused test-prefix podman; then
    printf "%s\n" "Supabase graph assertion did not default to strict dependency order." >&2
    exit 1
  fi
  [[ "${dependency_calls}" == 1 ]]

  dependency_calls=0
  supabase_assert_output_graph all compose podman unused test-prefix podman false
  [[ "${dependency_calls}" == 0 ]]

  if supabase_assert_output_graph all compose podman unused test-prefix podman typo \
      > /dev/null 2>&1; then
    printf "%s\n" "Supabase graph assertion admitted an invalid dependency expectation." >&2
    exit 1
  fi
' bash "${library}"

alias_output_root="${test_root}/alias-outputs"
mkdir -p -- "${alias_output_root}/compose" "${alias_output_root}/quadlet" \
  "${alias_output_root}/podman"
printf '%s\n' \
  '---' \
  'services:' \
  '  test-prefix-supabase-realtime:' \
  '    networks:' \
  '      test-prefix-supabase-backend:' \
  '        aliases:' \
  '          - realtime-dev.supabase-realtime' \
  '          - realtime' \
  > "${alias_output_root}/compose/compose.yaml"
printf '%s\n' \
  '[Container]' \
  'Network=test-prefix-supabase-backend.network' \
  'NetworkAlias=realtime-dev.supabase-realtime' \
  'NetworkAlias=realtime' \
  > "${alias_output_root}/quadlet/test-prefix-supabase-realtime.container"
jq --null-input '
  {
    operations: [{
      action: "create",
      resource: {kind: "container", name: "test-prefix-supabase-realtime"},
      libpod: {body: {json: {Networks: {
        "test-prefix-supabase-backend": {
          aliases: ["realtime-dev.supabase-realtime", "realtime"]
        }
      }}}}
    }]
  }
' > "${alias_output_root}/podman/podman.json"
for alias_output in compose quadlet podman; do
  supabase_assert_realtime_output_aliases \
    "${alias_output}" "${alias_output_root}/${alias_output}" test-prefix cli
done

write_realtime_alias_fixture() {
  local output=$1 directory=$2
  shift 2
  local aliases_json alias_value
  local -a fixture_aliases=("$@")

  mkdir -p -- "${directory}"
  case "${output}" in
    compose)
      {
        printf '%s\n' \
          '---' \
          'services:' \
          '  test-prefix-supabase-realtime:' \
          '    networks:' \
          '      test-prefix-supabase-backend:' \
          '        aliases:'
        for alias_value in "${fixture_aliases[@]}"; do
          printf '          - %s\n' "${alias_value}"
        done
      } > "${directory}/compose.yaml"
      ;;
    quadlet)
      {
        printf '%s\n' \
          '[Container]' \
          'Network=test-prefix-supabase-backend.network'
        for alias_value in "${fixture_aliases[@]}"; do
          printf 'NetworkAlias=%s\n' "${alias_value}"
        done
      } > "${directory}/test-prefix-supabase-realtime.container"
      ;;
    podman)
      aliases_json="$(
        printf '%s\n' "${fixture_aliases[@]}" |
          jq --raw-input --slurp 'split("\n")[:-1]'
      )"
      jq --null-input --argjson aliases "${aliases_json}" '
        {
          operations: [{
            action: "create",
            resource: {kind: "container", name: "test-prefix-supabase-realtime"},
            libpod: {body: {json: {Networks: {
              "test-prefix-supabase-backend": {aliases: $aliases}
            }}}}
          }]
        }
      ' > "${directory}/podman.json"
      ;;
  esac
}

for provisioner_mode in cli compose; do
  case "${provisioner_mode}" in
    cli)
      expected_aliases=("realtime-dev.supabase-realtime" "realtime")
      ;;
    compose)
      expected_aliases=(
        "test-prefix-supabase-realtime"
        "realtime"
        "realtime-dev.supabase-realtime"
      )
      ;;
  esac
  for alias_output in compose quadlet podman; do
    alias_directory="${alias_output_root}/${provisioner_mode}-strict/${alias_output}"
    write_realtime_alias_fixture \
      "${alias_output}" "${alias_directory}" "${expected_aliases[@]}"
    supabase_assert_realtime_output_aliases \
      "${alias_output}" "${alias_directory}" test-prefix "${provisioner_mode}"

    for alias_mutation in missing extra reordered; do
      case "${alias_mutation}" in
        missing)
          mutated_aliases=("${expected_aliases[@]:0:${#expected_aliases[@]}-1}")
          ;;
        extra)
          mutated_aliases=("${expected_aliases[@]}" private-alias-canary)
          ;;
        reordered)
          mutated_aliases=("${expected_aliases[@]}")
          first_alias=${mutated_aliases[0]}
          mutated_aliases[0]=${mutated_aliases[1]}
          mutated_aliases[1]=${first_alias}
          ;;
      esac
      write_realtime_alias_fixture \
        "${alias_output}" "${alias_directory}" "${mutated_aliases[@]}"
      alias_error="${alias_output_root}/${provisioner_mode}-${alias_output}-${alias_mutation}.error"
      if supabase_assert_realtime_output_aliases \
        "${alias_output}" "${alias_directory}" test-prefix "${provisioner_mode}" \
        2> "${alias_error}"; then
        printf 'Supabase %s-origin %s aliases admitted %s drift.\n' \
          "${provisioner_mode}" "${alias_output}" "${alias_mutation}" >&2
        exit 1
      fi
      for alias_value in "${expected_aliases[@]}" private-alias-canary; do
        if grep --fixed-strings --quiet "${alias_value}" "${alias_error}"; then
          printf 'Supabase alias failure exposed an alias value.\n' >&2
          exit 1
        fi
      done
    done
  done
done

ownership_output_root="${test_root}/ownership-outputs"
for selection in exact all; do
  include_system_network=false
  [[ "${selection}" == all ]] && include_system_network=true
  mkdir -p -- \
    "${ownership_output_root}/${selection}/compose" \
    "${ownership_output_root}/${selection}/quadlet" \
    "${ownership_output_root}/${selection}/podman"
  if [[ "${selection}" == exact ]]; then
    printf '%s\n' \
      '---' \
      'networks:' \
      '  test-prefix-supabase-backend:' \
      '    internal: true' \
      '  test-prefix-supabase-edge:' \
      '    external: true' \
      > "${ownership_output_root}/${selection}/compose/compose.yaml"
  else
    printf '%s\n' \
      '---' \
      'networks:' \
      '  test-prefix-supabase-backend:' \
      '    internal: true' \
      '  test-prefix-supabase-edge:' \
      '    name: test-prefix-supabase-edge' \
      '    external: true' \
      '  podman:' \
      '    internal: false' \
      > "${ownership_output_root}/${selection}/compose/compose.yaml"
  fi
  printf '%s\n' '[Network]' \
    > "${ownership_output_root}/${selection}/quadlet/test-prefix-supabase-backend.network"
  if [[ "${selection}" == all ]]; then
    printf '%s\n' '[Network]' 'Internal=false' \
      > "${ownership_output_root}/${selection}/quadlet/podman.network"
  fi
  jq --null-input \
    --argjson include_system_network "${include_system_network}" '
      {
        operations: ([{
          action: "create",
          resource: {kind: "network", name: "test-prefix-supabase-backend"}
        }] + if $include_system_network then [{
          action: "create",
          resource: {kind: "network", name: "podman"}
        }] else [] end)
      }
    ' > "${ownership_output_root}/${selection}/podman/podman.json"
done

for ownership_output in compose quadlet podman; do
  supabase_assert_network_ownership exact "${ownership_output}" \
    "${ownership_output_root}/exact/${ownership_output}" test-prefix
  supabase_assert_network_ownership all "${ownership_output}" \
    "${ownership_output_root}/all/${ownership_output}" test-prefix true
  if supabase_assert_network_ownership all "${ownership_output}" \
    "${ownership_output_root}/all/${ownership_output}" test-prefix false > /dev/null 2>&1; then
    printf 'Supabase %s ownership assertion admitted an unexpected system network.\n' \
      "${ownership_output}" >&2
    exit 1
  fi
done

invalid_external_network_root="${test_root}/invalid-external-network"
mkdir -p -- "${invalid_external_network_root}"
printf '%s\n' \
  '---' \
  'networks:' \
  '  test-prefix-supabase-backend:' \
  '    internal: true' \
  '  test-prefix-supabase-edge:' \
  '    name: renamed-supabase-edge' \
  '    external: true' \
  > "${invalid_external_network_root}/compose.yaml"
if supabase_assert_network_ownership exact compose \
  "${invalid_external_network_root}" test-prefix > /dev/null 2>&1; then
  printf '%s\n' 'Supabase Compose ownership assertion admitted a renamed external edge.' >&2
  exit 1
fi
printf '%s\n' \
  '---' \
  'networks:' \
  '  test-prefix-supabase-backend:' \
  '    internal: true' \
  '  test-prefix-supabase-edge:' \
  '    external: true' \
  '    driver: bridge' \
  > "${invalid_external_network_root}/compose.yaml"
if supabase_assert_network_ownership exact compose \
  "${invalid_external_network_root}" test-prefix > /dev/null 2>&1; then
  printf '%s\n' 'Supabase Compose ownership assertion admitted extra external-edge configuration.' >&2
  exit 1
fi

owned_edge_output_root="${test_root}/owned-edge-outputs"
mkdir -p -- \
  "${owned_edge_output_root}/compose" \
  "${owned_edge_output_root}/quadlet" \
  "${owned_edge_output_root}/podman"
printf '%s\n' \
  '---' \
  'networks:' \
  '  test-prefix-supabase-backend:' \
  '    internal: true' \
  '  test-prefix-supabase-edge:' \
  '    internal: false' \
  > "${owned_edge_output_root}/compose/compose.yaml"
printf '%s\n' '[Network]' \
  > "${owned_edge_output_root}/quadlet/test-prefix-supabase-backend.network"
printf '%s\n' '[Network]' \
  > "${owned_edge_output_root}/quadlet/test-prefix-supabase-edge.network"
jq --null-input '
    {
      operations: [
        {
          action: "create",
          resource: {kind: "network", name: "test-prefix-supabase-backend"}
        },
        {
          action: "create",
          resource: {kind: "network", name: "test-prefix-supabase-edge"}
        }
      ]
    }
  ' > "${owned_edge_output_root}/podman/podman.json"
for ownership_output in compose quadlet podman; do
  if supabase_assert_network_ownership all "${ownership_output}" \
    "${owned_edge_output_root}/${ownership_output}" test-prefix > /dev/null 2>&1; then
    printf 'Supabase %s ownership assertion admitted the externally owned edge as creatable.\n' \
      "${ownership_output}" >&2
    exit 1
  fi
done
tr '\0' '\n' < "${cli_services_argv}" | grep --fixed-strings \
  'PGRST_ADMIN_SERVER_HOST=127.0.0.1' > /dev/null
tr '\0' '\n' < "${cli_services_argv}" | grep --fixed-strings \
  'PGRST_ADMIN_SERVER_PORT=3001' > /dev/null
tr '\0' '\n' < "${cli_services_argv}" | grep --fixed-strings \
  '["postgrest","--ready"]' > /dev/null
if tr '\0' '\n' < "${cli_services_argv}" | grep --fixed-strings \
  '["CMD","postgrest","--ready"]' > /dev/null; then
  printf '%s\n' 'PostgREST CLI health command retained the remote-unsafe CMD marker.' >&2
  exit 1
fi
if tr '\0' '\n' < "${cli_services_argv}" | grep --fixed-strings \
  'postgrest --ready' > /dev/null; then
  printf '%s\n' 'Native PostgREST healthcheck retained shell form.' >&2
  exit 1
fi
if tr '\0' '\n' < "${cli_services_argv}" | grep --fixed-strings \
  'PGRST_ADMIN_SERVER_HOST=0.0.0.0' > /dev/null; then
  printf '%s\n' 'Native PostgREST administrative host retained wildcard binding.' >&2
  exit 1
fi
for overridden_default in PGRST_SERVER_HOST PGRST_DB_CHANNEL_ENABLED; do
  if tr '\0' '\n' < "${cli_services_argv}" | grep --fixed-strings \
    "${overridden_default}=" > /dev/null; then
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

network_volume_marker="${test_root}/network-volume.marker"
bash -c '
  set -Eeuo pipefail
  source "$1"
  marker=$2
  supabase_remote() { printf "%s\n" "$*" >> "$marker"; }
  supabase_create_networks_volumes test-socket test-prefix test-run
' bash "${library}" "${network_volume_marker}"
mapfile -t network_volume_calls < "${network_volume_marker}"
[[ "${network_volume_calls[0]}" == 'test-socket network create --internal --label io.boxferry.live-run=test-run --label io.boxferry.application=test-prefix-supabase test-prefix-supabase-backend' ]]
[[ "${network_volume_calls[1]}" == 'test-socket network create --label io.boxferry.live-run=test-run test-prefix-supabase-edge' ]]

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
 network) printf "edge-network:%s\\n" "$*" >> "$marker" ;;
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
[[ "${compose_order[0]}" == 'edge-network:test-socket network create --label io.boxferry.live-run=test-run test-prefix-supabase-edge' ]]
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
  supabase_realtime_websocket_ready() {
    printf "realtime-websocket:%s:%s\n" "$1" "$2" >> "${marker}"
  }
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
[[ "${database_peer_success_order[*]}" == 'peer-sql remote:healthcheck run test-prefix-supabase-auth remote:healthcheck run test-prefix-supabase-rest remote:healthcheck run test-prefix-supabase-realtime remote:healthcheck run test-prefix-supabase-imgproxy remote:healthcheck run test-prefix-supabase-storage remote:healthcheck run test-prefix-supabase-functions remote:healthcheck run test-prefix-supabase-studio remote:healthcheck run test-prefix-supabase-kong running:test-prefix-supabase-meta running:test-prefix-supabase-supavisor remote:exec test-prefix-supabase-studio node -e fetch('\''http://meta:8080/health'\'').then(r=>{if(!r.ok)process.exit(1)}) remote:exec test-prefix-supabase-studio node -e fetch('\''http://supavisor:4000/api/health'\'').then(r=>{if(!r.ok)process.exit(1)}) realtime-websocket:test-socket:test-prefix' ]]

realtime_readiness_argv="${test_root}/realtime-readiness.argv"
bash -c '
  set -Eeuo pipefail
  source "$1"
  marker=$2
  engine=test-engine
  timed_operation() { printf "%s\0" "$@" > "${marker}"; }
  SUPABASE_WAIT_REMAINING_SECONDS=17 \
    supabase_realtime_websocket_ready test-socket test-prefix
' bash "${library}" "${realtime_readiness_argv}"
mapfile -d '' -t realtime_readiness_arguments < "${realtime_readiness_argv}"
[[ "${#realtime_readiness_arguments[@]}" == 15 ]]
[[ "${realtime_readiness_arguments[0]}" == 7s ]]
[[ "${realtime_readiness_arguments[1]}" == 'nested Podman Supabase Realtime readiness through acquisition socket' ]]
[[ "${realtime_readiness_arguments[2]}" == test-engine ]]
[[ "${realtime_readiness_arguments[3]}" == --url ]]
[[ "${realtime_readiness_arguments[4]}" == unix://test-socket ]]
[[ "${realtime_readiness_arguments[5]}" == exec ]]
[[ "${realtime_readiness_arguments[6]}" == --env ]]
[[ "${realtime_readiness_arguments[7]}" == BF_ANON_KEY=* ]]
[[ "${realtime_readiness_arguments[8]}" == --env ]]
[[ "${realtime_readiness_arguments[9]}" == BF_SUPABASE_URL=http://kong:8000 ]]
[[ "${realtime_readiness_arguments[10]}" == --env ]]
[[ "${realtime_readiness_arguments[11]}" == SUPABASE_WAIT_REMAINING_SECONDS=7 ]]
[[ "${realtime_readiness_arguments[12]}" == test-prefix-supabase-studio ]]
[[ "${realtime_readiness_arguments[13]}" == node ]]
[[ "${realtime_readiness_arguments[14]}" == /boxferry-fixture/realtime-readiness.mjs ]]
if tr '\0' '\n' < "${realtime_readiness_argv}" |
  grep --extended-regexp --quiet 'BF_(SERVICE_KEY|TEST_EMAIL|TEST_PASSWORD)='; then
  printf '%s\n' 'Realtime readiness received unrelated protected application values.' >&2
  exit 1
fi

late_realtime_readiness_marker="${test_root}/late-realtime-readiness.marker"
if bash -c '
  set -Eeuo pipefail
  source "$1"
  marker=$2
  engine=test-engine
  timed_operation() { printf "invoked\n" > "${marker}"; }
  SUPABASE_WAIT_REMAINING_SECONDS=10 \
    supabase_realtime_websocket_ready test-socket test-prefix
' bash "${library}" "${late_realtime_readiness_marker}"; then
  printf '%s\n' 'Late Realtime readiness attempt ignored the transport kill grace.' >&2
  exit 1
fi
[[ ! -e "${late_realtime_readiness_marker}" ]]

realtime_readiness_failure_marker="${test_root}/realtime-readiness-failure.marker"
if bash -c '
  set -Eeuo pipefail
  source "$1"
  marker=$2
  supabase_wait_for() { shift 2; "$@"; }
  supabase_database_sql_contract() { printf "peer-sql\n" >> "${marker}"; }
  supabase_run_healthcheck() { printf "health:%s\n" "$2" >> "${marker}"; }
  supabase_wait_running() { printf "running:%s\n" "$2" >> "${marker}"; }
  supabase_remote() { printf "remote:%s\n" "$*" >> "${marker}"; }
  supabase_realtime_websocket_ready() {
    printf "realtime-websocket-failed:%s:%s\n" "$1" "$2" >> "${marker}"
    return 1
  }
  supabase_wait_application test-socket test-prefix
' bash "${library}" "${realtime_readiness_failure_marker}"; then
  printf '%s\n' 'Failed Realtime WebSocket readiness incorrectly allowed application readiness.' >&2
  exit 1
fi
[[ "$(tail -n 1 "${realtime_readiness_failure_marker}")" == realtime-websocket-failed:test-socket:test-prefix ]]

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

studio_health_failure_marker="${test_root}/studio-health-failure.marker"
if bash -c '
set -Eeuo pipefail
source "$1"
marker=$2
supabase_wait_for() { shift 2; "$@"; }
supabase_database_sql_contract() { printf "peer-sql\n" >> "${marker}"; }
supabase_run_healthcheck() {
  printf "health:%s\n" "$2" >> "${marker}"
  [[ "$2" != test-prefix-supabase-studio ]]
}
supabase_report_service_health_failure() { printf "report:%s\n" "$3" >> "${marker}"; }
supabase_wait_running() { printf "process-fallback:%s\n" "$2" >> "${marker}"; }
supabase_remote() { printf "remote-fallback:%s\n" "$*" >> "${marker}"; }
supabase_wait_application test-socket test-prefix
' bash "${library}" "${studio_health_failure_marker}"; then
  printf '%s\n' 'Failed Studio healthcheck incorrectly allowed readiness.' >&2
  exit 1
fi
[[ "$(tr '\n' ' ' < "${studio_health_failure_marker}")" == 'peer-sql health:test-prefix-supabase-auth health:test-prefix-supabase-rest health:test-prefix-supabase-realtime health:test-prefix-supabase-imgproxy health:test-prefix-supabase-storage health:test-prefix-supabase-functions health:test-prefix-supabase-studio report:studio ' ]]

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

compose_failure_report_output="${test_root}/compose-failure-report.output"
compose_failure_report_log="${test_root}/compose-failure-report.log"
compose_boundary_output="${test_root}/compose-boundary.output"
bash -c '
  set -Eeuo pipefail
  source "$1"
  log=$2
  boundary_output=$3
  : > "${boundary_output}"
  printf "raw-compose-prefix-marker-" > "${log}"
  head -c "$((SUPABASE_DIAGNOSTIC_CAPTURE_BYTES + 512))" /dev/zero | tr "\\0" x >> "${log}"
  printf "%s " "${SUPABASE_DB_PASSWORD: -7}" >> "${log}"
  head -c 5000 /dev/zero | tr "\\0" x >> "${log}"
  printf "%s %s %s %s %s %s %s %s %s %s %s terminal-compose-failure-cause-END" \
    "${SUPABASE_DB_PASSWORD}" "${SUPABASE_JWT_SECRET}" "${SUPABASE_ANON_KEY}" \
    "${SUPABASE_SERVICE_KEY}" "${SUPABASE_REALTIME_SECRET}" \
    "${SUPABASE_POOLER_SECRET}" "${SUPABASE_REALTIME_DB_KEY}" \
    "${SUPABASE_META_CRYPTO_KEY}" "${SUPABASE_VAULT_ENC_KEY}" \
    "${SUPABASE_TEST_EMAIL}" "${SUPABASE_TEST_PASSWORD}" >> "${log}"
  supabase_report_compose_failure initial-database "${log}"
  for protected in \
    "${SUPABASE_DB_PASSWORD}" "${SUPABASE_JWT_SECRET}" "${SUPABASE_ANON_KEY}" \
    "${SUPABASE_SERVICE_KEY}" "${SUPABASE_REALTIME_SECRET}" \
    "${SUPABASE_POOLER_SECRET}" "${SUPABASE_REALTIME_DB_KEY}" \
    "${SUPABASE_META_CRYPTO_KEY}" "${SUPABASE_VAULT_ENC_KEY}" \
    "${SUPABASE_TEST_EMAIL}" "${SUPABASE_TEST_PASSWORD}"; do
    printf "\nleading-tail-boundary=%s\n" \
      "$(supabase_redact_runtime_text "${protected:1}tail" tail)" >> "${boundary_output}"
    printf "trailing-head-boundary=%s\n" \
      "$(supabase_redact_runtime_text "head${protected:0:${#protected}-1}")" >> "${boundary_output}"
  done
' bash "${library}" "${compose_failure_report_log}" "${compose_boundary_output}" \
  > "${compose_failure_report_output}" 2>&1
grep --fixed-strings --quiet \
  'Supabase Docker Compose failure evidence: stage=initial-database;' \
  "${compose_failure_report_output}"
grep --fixed-strings --quiet '[REDACTED]' "${compose_failure_report_output}"
grep --fixed-strings --quiet 'terminal-compose-failure-cause-END' \
  "${compose_failure_report_output}"
[[ "$(grep --count --fixed-strings 'leading-tail-boundary=[REDACTED]tail' \
  "${compose_boundary_output}")" == 11 ]]
[[ "$(grep --count --fixed-strings 'trailing-head-boundary=head[REDACTED]' \
  "${compose_boundary_output}")" == 11 ]]
if grep --fixed-strings --quiet 'raw-compose-prefix-marker-' \
  "${compose_failure_report_output}"; then
  printf '%s\n' 'Compose failure diagnostics retained the raw leading provider output.' >&2
  exit 1
fi
for protected_value in \
  "${SUPABASE_DB_PASSWORD}" "${SUPABASE_JWT_SECRET}" "${SUPABASE_ANON_KEY}" \
  "${SUPABASE_SERVICE_KEY}" "${SUPABASE_REALTIME_SECRET}" \
  "${SUPABASE_POOLER_SECRET}" "${SUPABASE_REALTIME_DB_KEY}" \
  "${SUPABASE_META_CRYPTO_KEY}" "${SUPABASE_VAULT_ENC_KEY}" \
  "${SUPABASE_TEST_EMAIL}" "${SUPABASE_TEST_PASSWORD}"; do
  if grep --fixed-strings --quiet -- "${protected_value}" \
    "${compose_failure_report_output}" "${compose_boundary_output}"; then
    printf '%s\n' 'Compose failure diagnostics leaked a protected-value canary.' >&2
    exit 1
  fi
done
[[ "$(wc -c < "${compose_failure_report_output}")" -le "$((SUPABASE_DIAGNOSTIC_OUTPUT_BYTES + 256))" ]]

assert_compose_failure_stage() {
  local failure_stage=$1 marker="${test_root}/compose-stage-${1}.marker"
  if bash -c '
    set -Eeuo pipefail
    source "$1"
    current_case=$2
    failure_stage=$3
    marker=$4
    supabase_report_compose_failure() { printf "stage:%s\\n" "$1" >> "${marker}"; }
    supabase_assert_clean_prefix() { :; }
    supabase_remote() { :; }
    supabase_database_sql_contract() { :; }
    supabase_wait_for() { shift 2; "$@"; }
    supabase_start_compose_provider() {
      printf "provider\\n" >> "${marker}"
      [[ "${failure_stage}" == *full-graph ]] && return 41
      return 0
    }
    supabase_peer_compose_project() {
      printf "peer\\n" >> "${marker}"
      [[ "${failure_stage}" == boundary-peer ]] && return 42
      return 0
    }
    supabase_wait_application() { printf "application\\n" >> "${marker}"; }
    supabase_enable_realtime_table() { printf "realtime\\n" >> "${marker}"; }
    supabase_compose_project() {
      printf "compose:%s\\n" "$*" >> "${marker}"
      case "${failure_stage}:$*" in
        recreation-stop:*" stop --timeout 30") return 43 ;;
        recreation-remove:*" rm --force") return 44 ;;
        recreation-database:*" up --detach db") return 45 ;;
      esac
    }
    case "${failure_stage}" in
      initial-full-graph)
        supabase_start_compose_graph test-socket test-prefix test-run \
          "${current_case}/database.log" "${current_case}/graph.log" ;;
      boundary-peer)
        supabase_provision_compose test-socket test-prefix test-run ;;
      recreation-*)
        supabase_recreate_application compose test-socket test-prefix test-run ;;
    esac
  ' bash "${library}" "${test_root}" "${failure_stage}" "${marker}"; then
    printf 'Compose %s failure was unexpectedly accepted.\n' "${failure_stage}" >&2
    exit 1
  fi
  grep --fixed-strings --quiet "stage:${failure_stage}" "${marker}"
  if grep --fixed-strings --quiet -e application -e realtime "${marker}"; then
    printf 'Compose %s failure continued into application readiness.\n' "${failure_stage}" >&2
    exit 1
  fi
  if [[ "${failure_stage}" == recreation-stop || "${failure_stage}" == recreation-remove ]]; then
    if grep --fixed-strings --quiet -- 'up --detach db' "${marker}"; then
      printf 'Compose %s failure started graph recreation.\n' "${failure_stage}" >&2
      exit 1
    fi
  fi
}

for compose_failure_stage in initial-full-graph boundary-peer recreation-stop \
  recreation-remove recreation-database recreation-full-graph; do
  assert_compose_failure_stage "${compose_failure_stage}"
done

initial_database_failure_output="${test_root}/initial-database-failure.output"
initial_database_failure_marker="${test_root}/initial-database-failure.marker"
bash -c '
  set -Eeuo pipefail
  source "$1"
  current_case=$2
  marker=$3
  supabase_assert_clean_prefix() { :; }
  supabase_remote() { :; }
  supabase_report_database_contract_failure() { :; }
  supabase_report_database_failure_evidence() { :; }
  supabase_wait_for() { printf "readiness-ran\\n" >> "${marker}"; return 1; }
  supabase_start_compose_provider() { printf "provider-ran\\n" >> "${marker}"; return 1; }
  supabase_peer_compose_project() { printf "peer-ran\\n" >> "${marker}"; return 1; }
  supabase_compose_project() {
    printf "compose:%s\\n" "$*" >> "${marker}"
    if [[ " $* " == *" up --detach db "* ]]; then
      printf "compose-exit-127\\n" >&2
      return 127
    fi
    return 1
  }
  if supabase_provision_compose test-socket test-prefix test-run; then
    printf "%s\n" "Initial Compose database failure was unexpectedly accepted." >&2
    exit 1
  fi
' bash "${library}" "${test_root}" "${initial_database_failure_marker}" \
  > "${initial_database_failure_output}" 2>&1
grep --fixed-strings --quiet \
  'Supabase Docker Compose failure evidence: stage=initial-database;' \
  "${initial_database_failure_output}"
grep --fixed-strings --quiet 'compose-exit-127' \
  "${initial_database_failure_output}"
if grep --fixed-strings --quiet -e readiness-ran -e provider-ran -e peer-ran \
  "${initial_database_failure_marker}"; then
  printf '%s\n' 'Initial Compose database failure continued into a later lifecycle stage.' >&2
  exit 1
fi

compose_timeout_provider="${test_root}/timeout-compatible-compose-provider"
compose_timeout_capture="${test_root}/timeout-compatible-compose"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'set -Eeuo pipefail' \
  'env | LC_ALL=C sort > "${SUPABASE_PROVIDER_CAPTURE:?}.env"' \
  'printf "%s\\0" "$@" > "${SUPABASE_PROVIDER_CAPTURE:?}.argv"' \
  > "${compose_timeout_provider}"
chmod +x "${compose_timeout_provider}"
bash -c '
  set -Eeuo pipefail
  source "$1"
  source "$2"
  provider=$3
  capture=$4
  repository_root=$5
  timestamp() { printf "test-time"; }
  format_duration() { printf "%ss" "$1"; }
  exec 3> "${capture}.timing"
  BOXFERRY_COMPOSE_BIN="${provider}" \
    SUPABASE_PROVIDER_CAPTURE="${capture}-application" \
    supabase_compose_project test-socket test-prefix test-run up --detach
  BOXFERRY_COMPOSE_BIN="${provider}" \
    SUPABASE_PROVIDER_CAPTURE="${capture}-peer" \
    supabase_peer_compose_project test-socket test-prefix test-run up --detach --remove-orphans
' bash "${library}" "${timed_operation_helper}" "${compose_timeout_provider}" \
  "${compose_timeout_capture}" "${PWD}"
grep --fixed-strings --quiet \
  'test-time STEP START Docker Compose Supabase up (deadline 15m)' \
  "${compose_timeout_capture}.timing"
grep -E --quiet \
  '^test-time STEP PASS  Docker Compose Supabase up \([0-9]+s\)$' \
  "${compose_timeout_capture}.timing"
grep --fixed-strings --quiet \
  'test-time STEP START Docker Compose Supabase boundary peer up (deadline 3m)' \
  "${compose_timeout_capture}.timing"
grep -E --quiet \
  '^test-time STEP PASS  Docker Compose Supabase boundary peer up \([0-9]+s\)$' \
  "${compose_timeout_capture}.timing"
for compose_timeout_kind in application peer; do
  capture_path="${compose_timeout_capture}-${compose_timeout_kind}"
  expected_environment_path="${capture_path}.expected-env"
  expected_environment=(
    'BF_PREFIX=test-prefix'
    'BF_RUN=test-run'
    'BF_FIXTURE_ROOT=/tmp/boxferry-fixture/test-prefix'
    'BF_DB_PASSWORD=boxferry-public-supabase-db-password'
    'BF_JWT_SECRET=boxferry-public-jwt-secret-at-least-thirty-two-characters'
    'BF_ANON_KEY=eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJhdWQiOiJhdXRoZW50aWNhdGVkIiwiZXhwIjo0MTAyNDQ0ODAwLCJpYXQiOjE3MDQwNjcyMDAsImlzcyI6InN1cGFiYXNlIiwicm9sZSI6ImFub24ifQ.lGRwynjOirFYPqa-6IzEfCCIS_yNsl4riV9_TYukv0g'
    'BF_SERVICE_KEY=eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJhdWQiOiJhdXRoZW50aWNhdGVkIiwiZXhwIjo0MTAyNDQ0ODAwLCJpYXQiOjE3MDQwNjcyMDAsImlzcyI6InN1cGFiYXNlIiwicm9sZSI6InNlcnZpY2Vfcm9sZSJ9.GqsNLUWNCMg6So_4dAH5LRG2EtPKRYL2wb9gffU8eTU'
    'BF_REALTIME_SECRET=boxferry-public-realtime-secret-key-base-64-characters-long-0000000000'
    'BF_REALTIME_DB_KEY=boxferry-rt-key1'
    'BF_POOLER_SECRET=boxferry-public-pooler-secret-key-base-64-characters-long-000000000000'
    'BF_STUDIO_IMAGE=docker.io/supabase/studio:2026.08.03-sha-022b374@sha256:2616bb9ed337963fe27ce682b1783875083537d5fb54bfea4c399fb0c56ff03e'
    'BF_KONG_IMAGE=docker.io/kong/kong:3.9.3@sha256:61591af560fc9ba4d1e2fcc8be87f28e374c4b2f4a8f0e637702ee12dcaddade'
    'BF_AUTH_IMAGE=docker.io/supabase/gotrue:v2.189.0@sha256:0a8557cbe0fd53a067726fe656f79eb1b03a1ab3cdde4b59907ce5a1e1a202ab'
    'BF_REST_IMAGE=docker.io/postgrest/postgrest:v16.3@sha256:63b567a462c4fd81ede0bdff0b38a150f732ad5fe4f4b01cebeb6a1aa8dbe0d6'
    'BF_REALTIME_IMAGE=docker.io/supabase/realtime:v2.102.3@sha256:2cc87edf0db5cebf1f58c9a4116bb80a25ff764c8706b6802fa68d976e66e5d7'
    'BF_STORAGE_IMAGE=docker.io/supabase/storage-api:v1.60.4@sha256:6f706c1184d97b081446527bb62a3193d3d47ad0daafcf738fd5c3e5a62aed97'
    'BF_IMGPROXY_IMAGE=docker.io/darthsim/imgproxy:v3.30.1@sha256:965c3782818766a477a056016e18f88f9a028bf68b39cb2316978945ac2c0492'
    'BF_META_IMAGE=docker.io/supabase/postgres-meta:v0.96.6@sha256:b9edad6fff2d4fb991ecd57837dbe3f21d2efa0f0ccb186f6ccf0e2d57192fed'
    'BF_FUNCTIONS_IMAGE=docker.io/supabase/edge-runtime:v1.74.0@sha256:8c17262ecf2fcc43fe19c48d239280592129f2164e61f0f17ba56533120f92d5'
    'BF_DB_IMAGE=docker.io/supabase/postgres:17.6.1.136@sha256:5a4314708484bec672de2c09653a5c01fb1c84a998564ac231b0325e2238ed5b'
    'BF_SUPAVISOR_IMAGE=docker.io/supabase/supavisor:2.9.5@sha256:4dd940610c0ef5c8284ef88a28530566d45b52f7d3de285497a67c169e00cec9'
    'DOCKER_HOST=unix://test-socket'
  )
  printf '%s\n' "${expected_environment[@]}" |
    LC_ALL=C sort > "${expected_environment_path}"
  grep -E '^(BF_|DOCKER_HOST=)' "${capture_path}.env" |
    LC_ALL=C sort > "${capture_path}.actual-env"
  diff --unified=3 "${expected_environment_path}" "${capture_path}.actual-env"
  mapfile -d '' -t compose_timeout_argv < "${capture_path}.argv"
  if [[ "${compose_timeout_kind}" == application ]]; then
    expected_project=test-prefix-supabase
    expected_file="$(supabase_fixture_root)/compose.yaml"
    expected_arguments=(up --detach)
  else
    expected_project=test-prefix-supabase-peer
    expected_file="$(supabase_fixture_root)/peer.compose.yaml"
    expected_arguments=(up --detach --remove-orphans)
  fi
  expected_compose_argv=(
    --project-name "${expected_project}"
    --file "${expected_file}"
    "${expected_arguments[@]}"
  )
  [[ "${#compose_timeout_argv[@]}" == "${#expected_compose_argv[@]}" ]]
  for compose_timeout_index in "${!expected_compose_argv[@]}"; do
    [[ "${compose_timeout_argv[compose_timeout_index]}" == "${expected_compose_argv[compose_timeout_index]}" ]]
  done
done

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
