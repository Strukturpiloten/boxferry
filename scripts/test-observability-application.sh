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
