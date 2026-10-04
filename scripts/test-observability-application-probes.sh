#!/usr/bin/env bash
# Independently authored semantic HTTP assertions and the unchanged Podman transport seam.
set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
test_root="$(mktemp -d)"
trap 'rm -r -- "${test_root}"' EXIT
probe_log="${test_root}/calls"
probe_output="${test_root}/output"
opaque_context="opaque context ; \$literal /not/a/socket"
prefix='opaque prefix'

# A source-only helper must not invoke tools, load a native wrapper, or require runner globals.
(
  for tool in jq python3 curl wget podman docker; do
    eval "${tool}() { printf '%s\\n' unexpected >> \"\${probe_log}\"; return 91; }"
  done
  # shellcheck source=scripts/lib/observability-application-probes.sh
  source "${script_directory}/lib/observability-application-probes.sh"
  [[ ! -e "${probe_log}" ]]
  [[ "$(type -t observability_probe_assert_queries_and_grafana)" == function ]]
  [[ "$(type -t observability_remote || true)" != function ]]
  [[ "$(type -t observability_backend_get || true)" != function ]]
)
# shellcheck source=scripts/lib/observability-application-probes.sh
source "${script_directory}/lib/observability-application-probes.sh"

urls=(
  'http://prometheus:9090/api/v1/query?query=boxferry_fixture_temperature_celsius%7Bsource%3D%22controlled%22%7D'
  'http://loki:3100/loki/api/v1/query_range?query=%7Bjob%3D%22boxferry_fixture%22%7D%20%7C%3D%20%22boxferry-observability-known-log%22&limit=20'
  'http://prometheus:9090/api/v1/status/flags'
  'http://grafana:3000/api/datasources/uid/boxferry-prometheus'
  'http://grafana:3000/api/datasources/uid/boxferry-loki'
  'http://grafana:3000/api/datasources/uid/boxferry-prometheus/health'
  'http://grafana:3000/api/datasources/uid/boxferry-loki/health'
  'http://grafana:3000/api/dashboards/uid/boxferry-observability'
)
baseline=(
  '{"status":"success","data":{"result":[{"value":[123,"42"]}]}}'
  '{"status":"success","data":{"result":[{"values":[["123","boxferry-observability-known-log"]]}]}}'
  '{"status":"success","data":{"storage.tsdb.retention.time":"1d","web.enable-remote-write-receiver":"true"}}'
  '{"uid":"boxferry-prometheus","type":"prometheus","url":"http://prometheus:9090","isDefault":true}'
  '{"uid":"boxferry-loki","type":"loki","url":"http://loki:3100"}'
  '{"status":"OK"}'
  '{"status":"OK"}'
  '{"dashboard":{"uid":"boxferry-observability","title":"BoxFerry Observability Acceptance","panels":[{"targets":[{"expr":"boxferry_fixture_temperature_celsius{source=\"controlled\"}"}]},{"targets":[{"expr":"{job=\"boxferry_fixture\"} |= \"boxferry-observability-known-log\""}]}]}}'
)
responses=("${baseline[@]}")
failed_index=-1
checks=0

semantic_http() {
  local index
  [[ "$#" == 3 && "$1" == "${opaque_context}" && "$2" == "${prefix}" ]] || return 81
  index="$(wc -l < "${probe_log}")"
  [[ "${index}" -lt "${#urls[@]}" && "$3" == "${urls[index]}" ]] || return 82
  printf '%s\n' "${index}" >> "${probe_log}"
  printf '%s\n' "${responses[index]}"
  # A valid response body never rescues a failed HTTP operation.
  [[ "${index}" != "${failed_index}" ]] || return 73
}

reset_probe() {
  : > "${probe_log}"
  : > "${probe_output}"
  responses=("${baseline[@]}")
  failed_index=-1
}

assert_success() {
  if "$@" > "${probe_output}" 2>&1; then
    [[ "$(wc -l < "${probe_log}")" == 8 && ! -s "${probe_output}" ]]
  else
    printf '%s\n' 'Authored observability HTTP responses did not satisfy the shared contract.' >&2
    return 1
  fi
  checks=$((checks + 1))
}

assert_failure() {
  local index=$1 expected_status=$2 status
  shift 2
  # if and && disable errexit inside the invoked shell function. Explicit returns must suffice.
  if "$@" > "${probe_output}" 2>&1; then
    printf 'Observability failed operation at index %s was accepted.\n' "${index}" >&2
    return 1
  else
    status=$?
  fi
  [[ "${expected_status}" == nonzero || "${status}" == "${expected_status}" ]]
  [[ "$(wc -l < "${probe_log}")" == "$((index + 1))" ]]
  [[ ! -s "${probe_output}" ]]
  checks=$((checks + 1))
}

reset_probe
assert_success observability_probe_assert_queries_and_grafana semantic_http "${opaque_context}" "${prefix}"
for index in "${!urls[@]}"; do
  reset_probe
  failed_index="${index}"
  assert_failure "${index}" 73 observability_probe_assert_queries_and_grafana semantic_http "${opaque_context}" "${prefix}"
  for malformed in '' '{' 'null' '[]' '{"unassessed":"private-response-canary"}' \
    "${baseline[index]}"$'\n'"${baseline[index]}"; do
    reset_probe
    responses[index]="${malformed}"
    assert_failure "${index}" nonzero observability_probe_assert_queries_and_grafana semantic_http "${opaque_context}" "${prefix}"
  done
done

# Explicit missing/wrong/type cases, independently authored rather than projected from the helper.
mutations=(
  '0|{"data":{"result":[{"value":[123,"42"]}]}}'
  '0|{"status":"success","data":{}}'
  '0|{"status":"success","data":{"result":[{"value":[123,"43"]}]}}'
  '0|{"status":"success","data":{"result":[{"value":[123,42]}]}}'
  '0|{"status":"error","data":{"result":[{"value":[123,"42"]}]}}'
  '1|{"status":"success","data":{"result":[]}}'
  '1|{"status":"success","data":{"result":[{"values":[["123","wrong-log"]]}]}}'
  '1|{"status":"success","data":{"result":[{"values":[["123","boxferry-observability-known-log"],["124","boxferry-observability-known-log"]]}]}}'
  '1|{"status":"success","data":{"result":[{"values":[["123",true]]}]}}'
  '2|{"status":"success","data":{"storage.tsdb.retention.time":"24h","web.enable-remote-write-receiver":"true"}}'
  '2|{"status":"success","data":{"storage.tsdb.retention.time":"1d","web.enable-remote-write-receiver":true}}'
  '2|{"status":"success","data":{"storage.tsdb.retention.time":"1d","web.enable-remote-write-receiver":"false"}}'
  '2|{"status":"success","data":{"storage.tsdb.retention.time":"1d"}}'
  '2|{"status":"success","data":true}'
  '3|{"uid":"wrong","type":"prometheus","url":"http://prometheus:9090","isDefault":true}'
  '3|{"uid":"boxferry-prometheus","type":"loki","url":"http://prometheus:9090","isDefault":true}'
  '3|{"uid":"boxferry-prometheus","type":"prometheus","url":"http://wrong:9090","isDefault":true}'
  '3|{"uid":"boxferry-prometheus","type":"prometheus","url":"http://prometheus:9090","isDefault":"true"}'
  '3|{"uid":"boxferry-prometheus","type":"prometheus","url":"http://prometheus:9090"}'
  '4|{"uid":"wrong","type":"loki","url":"http://loki:3100"}'
  '4|{"uid":"boxferry-loki","type":"prometheus","url":"http://loki:3100"}'
  '4|{"uid":"boxferry-loki","type":"loki","url":"http://wrong:3100"}'
  '4|{"uid":"boxferry-loki","type":"loki"}'
  '5|{"status":"ok"}'
  '5|{"status":true}'
  '6|{"status":"error"}'
  '6|{"status":null}'
  '7|{"dashboard":{"uid":"wrong","title":"BoxFerry Observability Acceptance","panels":[]}}'
  '7|{"dashboard":{"uid":"boxferry-observability","title":"wrong","panels":[]}}'
  '7|{"dashboard":{"uid":"boxferry-observability","title":"BoxFerry Observability Acceptance","panels":[{"targets":[{"expr":"{job=\"boxferry_fixture\"} |= \"boxferry-observability-known-log\""}]}]}}'
  '7|{"dashboard":{"uid":"boxferry-observability","title":"BoxFerry Observability Acceptance","panels":[{"targets":[{"expr":"boxferry_fixture_temperature_celsius{source=\"controlled\"}"}]}]}}'
)
# Keep both valid expressions while independently mutating dashboard identity/title.
mutations+=(
  "7|${baseline[7]/boxferry-observability/incorrect-dashboard}"
  "7|${baseline[7]/BoxFerry Observability Acceptance/Incorrect title}"
)
for mutation in "${mutations[@]}"; do
  reset_probe
  index="${mutation%%|*}"
  responses[index]="${mutation#*|}"
  assert_failure "${index}" nonzero observability_probe_assert_queries_and_grafana semantic_http "${opaque_context}" "${prefix}"
done

# The generic metric seam still supports the caller-owned historical persistence query/value.
reset_probe
original_metric_url="${urls[0]}"
urls[0]='http://prometheus:9090/api/v1/query?query=historical%5B30m%5D'
responses[0]='{"status":"success","data":{"result":[{"value":[123,"84"]}]}}'
observability_probe_prometheus_has_value semantic_http "${opaque_context}" "${prefix}" 'historical%5B30m%5D' 84
[[ "$(wc -l < "${probe_log}")" == 1 ]]
urls[0]="${original_metric_url}"
checks=$((checks + 1))

# Source the real wrapper only now. No native tool is executed: the runtime callback is stubbed.
# shellcheck source=scripts/lib/observability-application.sh
source "${script_directory}/lib/observability-application.sh"
observability_remote() {
  [[ "$#" == 6 && "$1" == "${opaque_context}" && "$2" == exec &&
    "$3" == "${prefix}-observability-metrics-producer" && "$4" == wget && "$5" == -qO- ]] || return 83
  semantic_http "$1" "${prefix}" "$6"
}
reset_probe
assert_success observability_assert_queries_and_grafana "${opaque_context}" "${prefix}"
for index in "${!urls[@]}"; do
  reset_probe
  failed_index="${index}"
  assert_failure "${index}" 73 observability_assert_queries_and_grafana "${opaque_context}" "${prefix}"
done
reset_probe
failed_index=0
assert_failure 0 73 observability_prometheus_has_value "${opaque_context}" "${prefix}" \
  'boxferry_fixture_temperature_celsius%7Bsource%3D%22controlled%22%7D' 42
reset_probe
responses[2]='{"status":"success","data":{"storage.tsdb.retention.time":"1d","web.enable-remote-write-receiver":"true"}}'
observability_validate_prometheus_flags "${responses[2]}"
if observability_validate_prometheus_flags ''; then
  printf '%s\n' 'Empty Prometheus flags were accepted.' >&2
  exit 1
fi
checks=$((checks + 1))

# An && invocation must not let a later successful assertion hide an earlier callback failure.
reset_probe
failed_index=3
and_status=0
observability_probe_assert_queries_and_grafana semantic_http "${opaque_context}" "${prefix}" && and_status=99
[[ "${and_status}" == 0 && "$(wc -l < "${probe_log}")" == 4 ]]
checks=$((checks + 1))
printf 'Observability shared HTTP probes: %s offline checks passed.\n' "${checks}"
