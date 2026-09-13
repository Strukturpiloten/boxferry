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

alias_directory="${test_root}/output-aliases-compose"
mkdir -p "${alias_directory}"
printf '%s\n' '---
services:
  bf-private-observability-grafana:
    networks:
      bf-private-observability-backend:
        aliases: [grafana]
      bf-private-observability-edge:
        aliases: [grafana]
  bf-private-observability-loki:
    networks:
      bf-private-observability-backend:
        aliases: [loki]
  bf-private-observability-metrics-producer:
    networks:
      bf-private-observability-backend:
        aliases: [metrics-producer]
  bf-private-observability-prometheus:
    networks:
      bf-private-observability-backend:
        aliases: [prometheus]
  bf-private-observability-alloy:
    networks: [bf-private-observability-backend]' > "${alias_directory}/compose.yaml"
observability_assert_output_aliases cli podman compose "${alias_directory}" bf-private

compose_alias_fixture="${repository_root}/fixtures/conformance/observability-application/expected-compose-provisioned-aliases.yaml"
compose_alias_directory="${test_root}/output-aliases-compose-provisioned"
mkdir -p "${compose_alias_directory}"
cp -- "${compose_alias_fixture}" "${compose_alias_directory}/compose.yaml"
observability_assert_output_aliases \
  compose podman compose "${compose_alias_directory}" bf-private
observability_assert_output_aliases \
  compose compose compose "${compose_alias_directory}" bf-private

compose_alias_podman_directory="${test_root}/output-aliases-compose-provisioned-podman"
mkdir -p "${compose_alias_podman_directory}"
jq --null-input --arg stem 'bf-private-observability-' '
  def create($role; $networks):
    {action: "create", resource: {kind: "container", name: ($stem + $role)},
     libpod: {body: {json: {Networks: $networks}}}};
  ($stem + "backend") as $backend
  | ($stem + "edge") as $edge
  | {operations: [
      create("alloy"; {($backend): {aliases: [($stem + "alloy"), "alloy"]}}),
      create("grafana"; {
        ($backend): {aliases: [($stem + "grafana"), "grafana"]},
        ($edge): {aliases: [($stem + "grafana"), "grafana"]}
      }),
      create("log-producer"; {($backend): {aliases: [($stem + "log-producer"), "log-producer"]}}),
      create("loki"; {($backend): {aliases: [($stem + "loki"), "loki"]}}),
      create("metrics-producer"; {
        ($backend): {aliases: [($stem + "metrics-producer"), "metrics-producer"]}
      }),
      create("prometheus"; {($backend): {aliases: [($stem + "prometheus"), "prometheus"]}})
    ]}
' > "${compose_alias_podman_directory}/podman.json"
observability_assert_output_aliases \
  compose podman podman "${compose_alias_podman_directory}" bf-private
observability_assert_output_aliases \
  compose compose podman "${compose_alias_podman_directory}" bf-private

compose_alias_quadlet_directory="${test_root}/output-aliases-compose-provisioned-quadlet"
mkdir -p "${compose_alias_quadlet_directory}"
for service in alloy log-producer loki metrics-producer prometheus; do
  printf '[Container]\nNetworkAlias=bf-private-observability-%s\nNetworkAlias=%s\nNetwork=bf-private-observability-backend.network\n' \
    "${service}" "${service}" \
    > "${compose_alias_quadlet_directory}/bf-private-observability-${service}.container"
done
printf '%s\n' \
  '[Container]' \
  'Network=bf-private-observability-backend.network' \
  'Network=bf-private-observability-edge' \
  > "${compose_alias_quadlet_directory}/bf-private-observability-grafana.container"
for source_kind in podman compose quadlet; do
  observability_assert_output_aliases \
    compose "${source_kind}" quadlet "${compose_alias_quadlet_directory}" bf-private
done

compose_alias_without_grafana_directory="${test_root}/output-aliases-compose-provisioned-without-grafana"
mkdir -p "${compose_alias_without_grafana_directory}"
cp -- "${compose_alias_fixture}" "${compose_alias_without_grafana_directory}/compose.yaml"
sed --in-place \
  's/aliases: \[bf-private-observability-grafana, grafana\]/aliases: []/' \
  "${compose_alias_without_grafana_directory}/compose.yaml"
observability_assert_output_aliases \
  compose quadlet compose "${compose_alias_without_grafana_directory}" bf-private

compose_alias_quadlet_podman_directory="${test_root}/output-aliases-compose-provisioned-quadlet-podman"
mkdir -p "${compose_alias_quadlet_podman_directory}"
jq '
  (.operations[]
   | select(.resource.name | endswith("-grafana"))
   | .libpod.body.json.Networks[]
   | .aliases) = []
' "${compose_alias_podman_directory}/podman.json" \
  > "${compose_alias_quadlet_podman_directory}/podman.json"
observability_assert_output_aliases \
  compose quadlet podman "${compose_alias_quadlet_podman_directory}" bf-private

compose_alias_duplicate_directory="${test_root}/output-aliases-compose-provisioned-duplicate-attachment"
mkdir -p "${compose_alias_duplicate_directory}"
jq '.operations += [.operations[0]]' "${compose_alias_podman_directory}/podman.json" \
  > "${compose_alias_duplicate_directory}/podman.json"
status=0
error="$(observability_assert_output_aliases \
  compose podman podman "${compose_alias_duplicate_directory}" bf-private 2>&1)" || status=$?
[[ "${status}" == 1 ]]
grep --fixed-strings --quiet \
  'duplicate-attachments=alloy/backend count=2' <<< "${error}"

cp -- "${compose_alias_fixture}" "${compose_alias_directory}/compose.yaml"
sed --in-place \
  's/aliases: \[bf-private-observability-alloy, alloy\]/aliases: [bf-private-observability-alloy, alloy, alloy]/' \
  "${compose_alias_directory}/compose.yaml"
if observability_assert_output_aliases \
  compose podman compose "${compose_alias_directory}" bf-private 2> /dev/null; then
  printf '%s\n' 'A duplicate Compose-provisioned alias satisfied the observability output contract.' >&2
  exit 1
fi

cp -- "${compose_alias_fixture}" "${compose_alias_directory}/compose.yaml"
sed --in-place \
  's/aliases: \[bf-private-observability-grafana, grafana\]/aliases: [bf-private-observability-grafana]/' \
  "${compose_alias_directory}/compose.yaml"
if observability_assert_output_aliases \
  compose podman compose "${compose_alias_directory}" bf-private 2> /dev/null; then
  printf '%s\n' 'A missing Compose-provisioned alias satisfied the observability output contract.' >&2
  exit 1
fi

cp -- "${compose_alias_fixture}" "${compose_alias_directory}/compose.yaml"
sed --in-place \
  's/aliases: \[bf-private-observability-loki, loki\]/aliases: [bf-private-observability-loki, loki, private-alias-canary]/' \
  "${compose_alias_directory}/compose.yaml"
status=0
error="$(observability_assert_output_aliases \
  compose podman compose "${compose_alias_directory}" bf-private 2>&1)" || status=$?
[[ "${status}" == 1 ]]
assert_absent private-alias-canary "${error}"
grep --fixed-strings --quiet \
  'value-mismatches=loki/backend expected-count=2 actual-count=3' <<< "${error}"

printf '%s\n' '---
services:
  bf-private-observability-loki:
    networks:
      bf-private-observability-backend:
        aliases: [loki, 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef]
  bf-private-observability-metrics-producer:
    networks:
      bf-private-observability-backend:
        aliases: [metrics-producer]
  bf-private-observability-prometheus:
    networks:
      bf-private-observability-backend:
        aliases: [prometheus]' > "${alias_directory}/compose.yaml"
if observability_assert_output_aliases cli quadlet compose "${alias_directory}" bf-private 2> /dev/null; then
  printf '%s\n' 'A runtime container-ID alias satisfied the observability output contract.' >&2
  exit 1
fi

alias_directory="${test_root}/output-aliases-quadlet"
mkdir -p "${alias_directory}"
for service in loki metrics-producer prometheus; do
  printf '[Container]\nNetworkAlias=%s\nNetwork=bf-private-observability-backend.network\n' \
    "${service}" > "${alias_directory}/bf-private-observability-${service}.container"
done
printf '%s\n' \
  '[Container]' \
  'Network=bf-private-observability-backend.network' \
  'Network=bf-private-observability-edge' \
  > "${alias_directory}/bf-private-observability-grafana.container"
observability_assert_output_aliases cli podman quadlet "${alias_directory}" bf-private
grafana_quadlet="${alias_directory}/bf-private-observability-grafana.container"
cp -- "${grafana_quadlet}" "${grafana_quadlet}.valid"
for missing_network in backend edge; do
  cp -- "${grafana_quadlet}.valid" "${grafana_quadlet}"
  sed --in-place "/bf-private-observability-${missing_network}/d" "${grafana_quadlet}"
  if observability_assert_output_aliases cli podman quadlet "${alias_directory}" bf-private 2> /dev/null; then
    printf 'A Quadlet artifact without the Grafana %s network satisfied the alias contract.\n' \
      "${missing_network}" >&2
    exit 1
  fi
done
cp -- "${grafana_quadlet}.valid" "${grafana_quadlet}"

alias_directory="${test_root}/output-aliases-podman"
mkdir -p "${alias_directory}"
jq --null-input \
  --arg stem 'bf-private-observability-' \
  '{operations: [
    {action: "create", resource: {kind: "container", name: ($stem + "grafana")}, libpod: {body: {json: {Networks: {
      ($stem + "backend"): {aliases: ["grafana"]},
      ($stem + "edge"): {aliases: ["grafana"]}
    }}}}},
    {action: "create", resource: {kind: "container", name: ($stem + "loki")}, libpod: {body: {json: {Networks: {
      ($stem + "backend"): {aliases: ["loki"]}
    }}}}},
    {action: "create", resource: {kind: "container", name: ($stem + "metrics-producer")}, libpod: {body: {json: {Networks: {
      ($stem + "backend"): {aliases: ["metrics-producer"]}
    }}}}},
    {action: "create", resource: {kind: "container", name: ($stem + "prometheus")}, libpod: {body: {json: {Networks: {
      ($stem + "backend"): {aliases: ["prometheus"]}
    }}}}}
  ]}' > "${alias_directory}/podman.json"
observability_assert_output_aliases cli podman podman "${alias_directory}" bf-private

bfq_report="${test_root}/bfq-report.json"
printf '%s\n' '{"diagnostics":[{"code":"BFQ0003","severity":"warning","fields":[{"name":"subject","value":"services.bf-private-observability-grafana.networks"},{"name":"reason","value":"reviewed multi-network alias omission"}]}]}' \
  > "${bfq_report}"
observability_assert_reviewed_diagnostics \
  live-reimport cli exact compose quadlet bf-private-observability- "${bfq_report}"
jq '.diagnostics[0].fields += [{"name":"decision","value":"omitted"}]' \
  "${bfq_report}" > "${bfq_report}.unexpected-decision"
if observability_assert_reviewed_diagnostics \
  live-reimport cli exact compose quadlet bf-private-observability- \
  "${bfq_report}.unexpected-decision" > /dev/null 2>&1; then
  printf '%s\n' 'A Quadlet diagnostic with an invented decision field satisfied the contract.' >&2
  exit 1
fi

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

assert_compose_external_edge() {
  local name=$1 expected_status=$2 content=$3 directory
  directory="${test_root}/compose-external-edge-${name}"
  mkdir -p "${directory}"
  printf '%s\n' "${content}" > "${directory}/compose.yaml"
  status=0
  observability_assert_external_edge compose "${directory}" bf-private-observability-edge || status=$?
  [[ "${status}" == "${expected_status}" ]]
}

assert_compose_external_edge key-only-success 0 'services:
  grafana:
    image: example.invalid/grafana
networks:
  bf-private-observability-edge:
    external: true
  unrelated-external-network:
    external: true
volumes:
  grafana-data: {}'
assert_compose_external_edge matching-name-success 0 'networks:
  backend:
    name: bf-private-observability-backend
    internal: true
  edge:
    name: bf-private-observability-edge
    external: true
services:
  grafana:
    image: example.invalid/grafana'
assert_compose_external_edge unrelated-mapping-failure 1 'networks:
  unrelated-external-network:
    external: true'
assert_compose_external_edge split-evidence-failure 1 'networks:
  named-but-internal:
    name: bf-private-observability-edge
    internal: true
  unrelated-external-network:
    external: true'
assert_compose_external_edge mismatched-name-failure 1 'networks:
  bf-private-observability-edge:
    name: wrong-network
    external: true'

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

persistence_prometheus_calls=0
persistence_loki_calls=0
persistence_grafana_calls=0
persistence_log_producer_marker="${test_root}/persistence-log-producer-called"
observability_prometheus_has_value() {
  [[ "$1" == /tmp/observability.sock ]]
  [[ "$2" == bf-private ]]
  [[ "$3" == 'max_over_time%28boxferry_fixture_temperature_celsius%7Bsource%3D%22controlled%22%7D%5B30m%5D%29' ]]
  [[ "$4" == 84 ]]
  ((persistence_prometheus_calls += 1))
}
observability_loki_has_known_log() {
  [[ "$1" == /tmp/observability.sock ]]
  [[ "$2" == bf-private ]]
  ((persistence_loki_calls += 1))
}
observability_remote() {
  local socket=$1
  shift
  [[ "${socket}" == /tmp/observability.sock ]]
  case "$2" in
    bf-private-observability-grafana)
      [[ "$1" == exec ]]
      [[ $# == 6 ]]
      [[ "$3" == grep ]]
      [[ "$4" == -Fx ]]
      [[ "$5" == boxferry-grafana-persisted ]]
      [[ "$6" == /var/lib/grafana/boxferry-persistence-marker ]]
      ((persistence_grafana_calls += 1))
      ;;
    bf-private-observability-log-producer)
      [[ "$1" == exec ]]
      [[ $# == 5 ]]
      [[ "$3" == wc ]]
      [[ "$4" == -l ]]
      [[ "$5" == /var/log/boxferry/telemetry.log ]]
      touch "${persistence_log_producer_marker}"
      printf '%s\n' 1
      ;;
    *)
      printf 'Unexpected persistence remote target: %s\n' "$2" >&2
      return 1
      ;;
  esac
}
observability_assert_persistence /tmp/observability.sock bf-private
[[ "${persistence_prometheus_calls}" == 1 ]]
[[ "${persistence_loki_calls}" == 1 ]]
[[ "${persistence_grafana_calls}" == 1 ]]
[[ -f "${persistence_log_producer_marker}" ]]

printf '%s\n' 'Observability Alloy timing, persistence, and bounded diagnostics tests passed.'
