#!/usr/bin/env bash

set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repository_root="$(cd -- "${script_directory}/.." && pwd -P)"
# shellcheck source=scripts/lib/observability-application.sh
source "${script_directory}/lib/observability-application.sh"

test_root="$(mktemp -d)"
trap 'rm -rf -- "${test_root}"' EXIT

assert_absent() {
  local value=$1 output=$2
  if grep --fixed-strings --quiet -- "${value}" <<< "${output}"; then
    printf '%s\n' 'A protected diagnostic value escaped.' >&2
    return 1
  fi
}

observability_validate_alloy_scrape_timing \
  "${repository_root}/fixtures/conformance/observability-application/config.alloy"

current_case="${test_root}/export-paths"
observability_prepare_export_paths cli exact
[[ -d "${current_case}/outputs/cli-exact" ]]
[[ -d "${current_case}/reimports" ]]
for output in compose quadlet podman; do
  [[ ! -e "${current_case}/outputs/cli-exact/${output}" ]]
done
for selection in label all; do
  observability_prepare_export_paths cli "${selection}"
  [[ -d "${current_case}/outputs/cli-${selection}" ]]
  for output in compose quadlet podman; do
    [[ ! -e "${current_case}/outputs/cli-${selection}/${output}" ]]
  done
done

current_case="${test_root}/regular-file-parent"
mkdir -p "${current_case}/outputs"
printf '%s\n' 'not-a-directory' > "${current_case}/outputs/cli-exact"
status=0
error="$(observability_prepare_export_paths cli exact 2>&1)" || status=$?
[[ "${status}" == 1 ]]
[[ "${error}" == "Observability export parent is not a directory: ${current_case}/outputs/cli-exact" ]]

current_case="${test_root}/pre-existing-export-target"
mkdir -p "${current_case}/outputs/cli-exact/compose"
status=0
error="$(observability_run_exports cli /tmp/observability.sock bf-private 2>&1)" || status=$?
[[ "${status}" == 1 ]]
[[ "${error}" == "Observability export target must not exist before conversion: ${current_case}/outputs/cli-exact/compose" ]]

export_mode=
export_operation_count=0
reimport_count=0
declare -A export_seen=() reimport_seen=()
boxferry_operation() {
  local description=$1 operation_label mode selection target output directory='' argument
  shift
  read -r operation_label mode selection target <<< "${description}"
  [[ "${operation_label}" == Observability && "${mode}" == "${export_mode}" ]]
  [[ "$1" == convert && "$2" == podman ]]
  output=$3
  case "${description}" in
    "Observability ${export_mode} exact Podman-to-${output}" | \
      "Observability ${export_mode} label Podman-to-${output}" | \
      "Observability ${export_mode} all Podman-to-${output}") ;;
    *)
      printf 'Unexpected modeled observability export operation: %s\n' "${description}" >&2
      return 1
      ;;
  esac
  while (($#)); do
    argument=$1
    shift
    if [[ "${argument}" == --output-directory ]]; then
      directory=${1:?output directory argument is required}
      shift
      break
    fi
  done
  [[ -n "${directory}" ]]
  [[ -d "$(dirname -- "${directory}")" ]]
  [[ ! -e "${directory}" ]]
  mkdir -- "${directory}"
  export_seen["${mode}-${selection}-${output}"]=$((export_seen["${mode}-${selection}-${output}"] + 1))
  ((export_operation_count += 1))
  printf '%s\n' '{"schema_version":1,"status":"success","exit_category":"success","diagnostics":[],"fidelity":{"invalid":0},"output_artifacts":["generated"]}'
}
observability_assert_output_membership() { :; }
observability_assert_output_semantics() { :; }
observability_assert_external_edge() { :; }
observability_run_reimports() {
  local mode=$1 selection=$2 source=$3 prefix=$4
  [[ "${mode}" == "${export_mode}" ]]
  [[ "${source}" == "${current_case}/outputs/${mode}-${selection}" ]]
  [[ -d "${current_case}/reimports" ]]
  [[ "${prefix}" == bf-private ]]
  reimport_seen["${mode}-${selection}"]=$((reimport_seen["${mode}-${selection}"] + 1))
  ((reimport_count += 1))
}

for export_mode in cli compose; do
  current_case="${test_root}/modeled-${export_mode}-exports"
  observability_run_exports "${export_mode}" /tmp/observability.sock bf-private
done
[[ "${export_operation_count}" == 18 ]]
[[ "${reimport_count}" == 6 ]]
for mode in cli compose; do
  for selection in exact label all; do
    [[ "${reimport_seen["${mode}-${selection}"]}" == 1 ]]
    for output in compose quadlet podman; do
      [[ "${export_seen["${mode}-${selection}-${output}"]}" == 1 ]]
    done
  done
done

prometheus_flags='{
  "status": "success",
  "data": {
    "storage.tsdb.retention.time": "1d",
    "web.enable-remote-write-receiver": "true"
  }
}'
observability_validate_prometheus_flags "${prometheus_flags}"

for invalid_flags in \
  '{"status":"success","data":{"storage.tsdb.retention.time":"24h","web.enable-remote-write-receiver":"true"}}' \
  '{"status":"success","data":{"storage.tsdb.retention.time":"12h","web.enable-remote-write-receiver":"true"}}' \
  '{"data":{"storage.tsdb.retention.time":"1d","web.enable-remote-write-receiver":"true"}}' \
  '{"status":"success"}' \
  '{"status":"success","data":{"web.enable-remote-write-receiver":"true"}}' \
  '{"status":"success","data":{"storage.tsdb.retention.time":"1d"}}' \
  '{"status":true,"data":{"storage.tsdb.retention.time":"1d","web.enable-remote-write-receiver":"true"}}' \
  '{"status":"success","data":true}' \
  '{"status":"success","data":{"storage.tsdb.retention.time":true,"web.enable-remote-write-receiver":"true"}}' \
  '{"status":"success","data":{"storage.tsdb.retention.time":"1d","web.enable-remote-write-receiver":true}}' \
  '{"status":"error","data":{"storage.tsdb.retention.time":"1d","web.enable-remote-write-receiver":"true"}}'; do
  if observability_validate_prometheus_flags "${invalid_flags}"; then
    printf '%s\n' 'Invalid Prometheus flags unexpectedly satisfied the observability contract.' >&2
    exit 1
  fi
done

write_timing_fixture() {
  local path=$1 interval=$2 timeout=${3:-}
  {
    printf '%s\n' 'prometheus.scrape "boxferry_fixture" {'
    printf '  scrape_interval = "%ss"\n' "${interval}"
    if [[ -n "${timeout}" ]]; then
      printf '  scrape_timeout = "%ss"\n' "${timeout}"
    fi
    printf '%s\n' '}'
  } > "${path}"
}

for invalid in missing equal greater; do
  case "${invalid}" in
    missing) write_timing_fixture "${test_root}/${invalid}.alloy" 2 ;;
    equal) write_timing_fixture "${test_root}/${invalid}.alloy" 2 2 ;;
    greater) write_timing_fixture "${test_root}/${invalid}.alloy" 2 3 ;;
  esac
  status=0
  error="$(observability_validate_alloy_scrape_timing \
    "${test_root}/${invalid}.alloy" 2>&1)" || status=$?
  [[ "${status}" == 1 ]]
  [[ "${error}" == 'Observability Alloy scrape timing requires one positive timeout strictly below its interval.' ]]
done

diagnostic_mode=all-running
observability_remote() {
  local socket=$1
  shift
  [[ "${socket}" == /tmp/observability.sock ]]
  [[ "$1" == inspect && "$2" == --format ]]

  case "${diagnostic_mode}:$4" in
    alloy-failed:bf-private-observability-alloy)
      printf '%s\n' \
        "${OBSERVABILITY_ADMIN_PASSWORD} /run/user/1000/podman.sock 10.88.0.9 runtime-deadbeef" >&2
      printf '%s\n' 'false 1 false'
      ;;
    multiple-failed:bf-private-observability-metrics-producer | \
      multiple-failed:bf-private-observability-alloy)
      printf '%s\n' 'false 1 false'
      ;;
    unavailable:*)
      printf '%s\n' \
        "${OBSERVABILITY_ADMIN_PASSWORD} /run/user/1000/podman.sock 10.88.0.9 runtime-deadbeef"
      ;;
    *) printf '%s\n' 'true 0 false' ;;
  esac
}

diagnostic_mode=alloy-failed
if observability_pipeline_roles_running /tmp/observability.sock bf-private; then
  printf '%s\n' 'Stopped Alloy unexpectedly satisfied the running-role contract.' >&2
  exit 1
fi

diagnostics="$(observability_report_pipeline_states \
  /tmp/observability.sock bf-private 2>&1)"
expected_diagnostics="$(
  cat << 'EOF'
OBSERVABILITY DIAGNOSTIC role=metrics-producer running=true exit-code=0 oom-killed=false
OBSERVABILITY DIAGNOSTIC role=alloy running=false exit-code=1 oom-killed=false
OBSERVABILITY DIAGNOSTIC role=prometheus running=true exit-code=0 oom-killed=false
OBSERVABILITY DIAGNOSTIC role=log-producer running=true exit-code=0 oom-killed=false
OBSERVABILITY DIAGNOSTIC role=loki running=true exit-code=0 oom-killed=false
OBSERVABILITY DIAGNOSTIC role=grafana running=true exit-code=0 oom-killed=false
OBSERVABILITY DIAGNOSTIC first-failed-hop=alloy-process
EOF
)"
[[ "${diagnostics}" == "${expected_diagnostics}" ]]
[[ "$(printf '%s\n' "${diagnostics}" | wc -l)" == 7 ]]
[[ "$(printf '%s' "${diagnostics}" | wc -c)" -le 768 ]]
for forbidden in \
  "${OBSERVABILITY_ADMIN_PASSWORD}" \
  /run/user/1000/podman.sock \
  10.88.0.9 \
  runtime-deadbeef \
  bf-private; do
  assert_absent "${forbidden}" "${diagnostics}"
done

diagnostic_mode=all-running
observability_pipeline_roles_running /tmp/observability.sock bf-private

diagnostic_mode=multiple-failed
diagnostics="$(observability_report_pipeline_states \
  /tmp/observability.sock bf-private 2>&1)"
grep --fixed-strings --quiet \
  'OBSERVABILITY DIAGNOSTIC first-failed-hop=metrics-producer-process' <<< "${diagnostics}"

diagnostic_mode=unavailable
diagnostics="$(observability_report_pipeline_states \
  /tmp/observability.sock bf-private 2>&1)"
[[ "$(printf '%s\n' "${diagnostics}" | wc -l)" == 7 ]]
grep --fixed-strings --quiet \
  'OBSERVABILITY DIAGNOSTIC first-failed-hop=unknown' <<< "${diagnostics}"
for forbidden in \
  "${OBSERVABILITY_ADMIN_PASSWORD}" \
  /run/user/1000/podman.sock \
  10.88.0.9 \
  runtime-deadbeef \
  bf-private; do
  assert_absent "${forbidden}" "${diagnostics}"
done

printf '%s\n' 'Observability Alloy timing and bounded diagnostics tests passed.'
