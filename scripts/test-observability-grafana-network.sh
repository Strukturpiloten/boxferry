#!/usr/bin/env bash
# Protect the CLI Grafana DNS topology independently of Compose service-name DNS.

set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=scripts/lib/observability-application.sh
source "${script_directory}/lib/observability-application.sh"

declare -a metrics_producer_run_arguments=()
declare -a grafana_run_arguments=()
declare -a grafana_probe_arguments=()

observability_remote() {
  local socket=$1
  shift

  [[ "${socket}" == /tmp/observability.sock ]]
  case "$1" in
    run)
      case "${*:1}" in
        *'--name bf-test-observability-metrics-producer'*)
          metrics_producer_run_arguments=("$@")
          ;;
        *'--name bf-test-observability-grafana'*)
          grafana_run_arguments=("$@")
          ;;
        *)
          printf 'Unexpected modeled Podman run: %s\n' "$*" >&2
          return 1
          ;;
      esac
      ;;
    exec)
      grafana_probe_arguments=("$@")
      ;;
    *)
      printf 'Unexpected modeled Podman command: %s\n' "$*" >&2
      return 1
      ;;
  esac
}

observability_image_reference() {
  case "$1" in
    producer | grafana)
      printf 'registry.invalid/boxferry-test/%s:reviewed\n' "$1"
      ;;
    *)
      return 1
      ;;
  esac
}

network_arguments() {
  local argument expect_network=false
  networks=()
  for argument in "$@"; do
    if [[ "${expect_network}" == true ]]; then
      networks+=("${argument}")
      expect_network=false
    elif [[ "${argument}" == --network ]]; then
      expect_network=true
    fi
  done
  [[ "${expect_network}" == false ]]
}

observability_create_cli_metrics_producer /tmp/observability.sock bf-test bf-run
observability_create_cli_grafana /tmp/observability.sock bf-test bf-run

declare -a networks=()
network_arguments "${metrics_producer_run_arguments[@]}"
[[ "${networks[*]}" == bf-test-observability-backend:alias=metrics-producer ]]

network_arguments "${grafana_run_arguments[@]}"
[[ "${networks[*]}" == 'bf-test-observability-backend:alias=grafana bf-test-observability-edge:alias=grafana' ]]

observability_grafana_api /tmp/observability.sock bf-test /api/health > /dev/null
[[ "${grafana_probe_arguments[*]}" == 'exec bf-test-observability-metrics-producer wget -qO- http://grafana:3000/api/health' ]]

printf 'Observability Grafana backend and edge DNS regression test passed.\n'
