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

# Exercise the real shared classifier with a fake host client, never a runtime.
# The override permits focused consumer feedback before the #404 dependency is merged.
application_presence_tests() (
  local application=$1 observed reply expected mutation_count
  local socket_directory="${test_root}/${application}-socket"
  mkdir -p "${socket_directory}"
  local socket="${socket_directory}/podman.sock"
  python3 -c 'import socket, sys; peer = socket.socket(socket.AF_UNIX); peer.bind(sys.argv[1]); peer.close()' "${socket}"
  local helper="${BOXFERRY_PRESENCE_HELPER:-${script_directory}/lib/native-presence.sh}"
  # shellcheck source=scripts/lib/native-presence.sh
  source "${helper}"
  repository_root="$(cd -- "${script_directory}/.." && pwd -P)"
  [[ -r "${repository_root}/fixtures/conformance/${application}-application/README.md" ]] || return 116
  case "${application}" in
    nextcloud)
      # shellcheck source=scripts/lib/nextcloud-application.sh
      source "${script_directory}/lib/nextcloud-application.sh"
      ;;
    forgejo)
      # shellcheck source=scripts/lib/forgejo-application.sh
      source "${script_directory}/lib/forgejo-application.sh"
      ;;
  esac
  local engine="${test_root}/${application}-client"
  export PRESENCE_SOCKET="${socket}" PRESENCE_CALLS="${test_root}/${application}-reads"
  export PRESENCE_REPLY=present PRESENCE_REQUIRE_ACTIVE=false PRESENCE_ACTIVE=false PRESENCE_CONTAINER_REPLY=''
  local presence_mutations_log="${test_root}/${application}-mutations" preparation="${test_root}/${application}-prepare"
  local mutation_status=0 preparation_status=0 activation_status=0 validation_status=0 restoration_status=0
  local config_failure='' config_mode failure_description
  local descriptions="${test_root}/${application}-descriptions"
  local -a config_failures=()
  printf '%s\n' '#!/usr/bin/env bash' \
    "[[ \"\$#\" == 5 && \"\$1\" == --url && \"\$2\" == \"unix://\${PRESENCE_SOCKET}\" && \"\$4\" == exists ]] || exit 93" \
    "printf \"%s\\t%s\\n\" \"\$3\" \"\$5\" >> \"\${PRESENCE_CALLS}\"" \
    "if [[ \"\$3\" == image && \"\${PRESENCE_REQUIRE_ACTIVE}\" == true && \"\${PRESENCE_ACTIVE}\" != true ]]; then exit 94; fi" \
    "reply=\"\${PRESENCE_REPLY}\"" \
    "[[ \"\$3\" != container || -z \"\${PRESENCE_CONTAINER_REPLY}\" ]] || reply=\"\${PRESENCE_CONTAINER_REPLY}\"" \
    "case \"\${reply}\" in" \
    '  present) exit 0 ;;' \
    '  absent) exit 1 ;;' \
    '  diagnostic) printf "Error: unavailable API\n" >&2; exit 1 ;;' \
    '  warning) printf "Warning: incomplete observation\n" >&2; exit 0 ;;' \
    '  failure) exit 37 ;;' \
    '  timeout) sleep 6; exit 0 ;;' \
    'esac' > "${engine}"
  chmod +x "${engine}"
  timeout() {
    [[ "$3" == 90s ]] || return 95
    command timeout "$@"
  }
  podman_socket() {
    [[ "$1" == "${socket}" ]] || return 96
    printf '%s\n' "${*:3}" >> "${presence_mutations_log}"
    return "${mutation_status}"
  }
  observability_remote() {
    local selected_socket=$1
    shift
    podman_socket "${selected_socket}" "Observability ${1:-command}" "$@"
  }
  create_application_edge() {
    case "${application}" in
      nextcloud) nextcloud_create_edge_network "$@" ;;
      forgejo) forgejo_create_edge_network "$@" ;;
      observability) observability_create_edge_and_peer "$@" ;;
    esac
  }
  prepare_application_target() {
    case "${application}" in
      nextcloud) nextcloud_prepare_application_target "$@" ;;
      forgejo) forgejo_prepare_application_target "$@" ;;
      observability) observability_prepare_application_target "$@" ;;
    esac
  }
  mutation_count=1
  [[ "${application}" != observability ]] || mutation_count=2
  for reply in present absent diagnostic warning failure timeout; do
    PRESENCE_REPLY="${reply}" native_presence_unverified=false
    : > "${PRESENCE_CALLS}"
    : > "${presence_mutations_log}"
    expected=2
    [[ "${reply}" != present && "${reply}" != absent ]] || expected=0
    observed=0
    create_application_edge "${socket}" presence-fixture owned-run || observed=$?
    [[ "${observed}" == "${expected}" ]] || return 97
    if [[ "${reply}" == absent ]]; then
      [[ "$(wc -l < "${presence_mutations_log}")" == "${mutation_count}" ]] || return 98
      grep -Fq 'network create --label io.boxferry.live-run=owned-run --label io.boxferry.shared=true presence-fixture-' "${presence_mutations_log}"
      if [[ "${application}" == observability ]]; then
        grep -Fq 'run --pull=never --detach --name presence-fixture-observability-boundary-peer' "${presence_mutations_log}"
      fi
    else
      [[ ! -s "${presence_mutations_log}" ]] || return 99
    fi
    if ((expected == 2)); then [[ "${native_presence_unverified}" == true ]] || return 100; fi
  done
  if [[ "${application}" == observability ]]; then
    PRESENCE_REPLY=present
    for reply in absent diagnostic warning failure; do
      PRESENCE_CONTAINER_REPLY="${reply}" observed=0
      : > "${presence_mutations_log}"
      create_application_edge "${socket}" presence-fixture owned-run || observed=$?
      if [[ "${reply}" == absent ]]; then
        [[ "${observed}" == 0 && "$(wc -l < "${presence_mutations_log}")" == 1 ]] || return 110
        grep -Fq 'run --pull=never --detach --name presence-fixture-observability-boundary-peer' "${presence_mutations_log}"
      else
        [[ "${observed}" == 2 && ! -s "${presence_mutations_log}" ]] || return 111
      fi
    done
    PRESENCE_CONTAINER_REPLY=''
  fi
  # Native mutation failure retains its original status instead of becoming presence unknown.
  PRESENCE_REPLY=absent mutation_status=37 observed=0
  create_application_edge "${socket}" presence-fixture owned-run || observed=$?
  [[ "${observed}" == 37 ]] || return 101
  mutation_status=0
  for unavailable_socket in "${socket_directory}/missing" "${engine}"; do
    : > "${PRESENCE_CALLS}"
    : > "${presence_mutations_log}"
    native_presence_unverified=false observed=0
    create_application_edge "${unavailable_socket}" presence-fixture owned-run || observed=$?
    [[ "${observed}" == 2 && "${native_presence_unverified}" == true &&
      ! -s "${PRESENCE_CALLS}" && ! -s "${presence_mutations_log}" ]] || return 102
  done
  # A later present result must not clear an earlier unverified observation.
  PRESENCE_REPLY=present
  "${application}_presence" "${socket}" network presence-fixture-shared-edge
  [[ "${native_presence_unverified}" == true ]] || return 103
  # Archive integrity is covered by the real ID-ledger fake engine suite. These
  # phase fixtures retain its required pre-activation binding/restoration seam.
  verify_archive_image_bindings() {
    [[ "$1" == "${application}" && -s "$2/images.tsv" ]] || return 117
  }
  archive_target_id() {
    [[ "$1" == outer && "${PRESENCE_ACTIVE}" == false ]] || return 118
    printf '%064d\n' 1
  }
  archive_host_alias() {
    [[ "$1" == "${application}" ]] || return 119
    printf 'localhost/boxferry-archive/presence-test/%s:%s\n' "$1" "$2"
  }
  restore_nested_archive_alias() {
    [[ "$1" == outer && "$2" == "localhost/boxferry-archive/presence-test/${application}:"* &&
      "$3" == "registry.invalid/boxferry-test/${application}-application:"* &&
      "${PRESENCE_ACTIVE}" == false ]] || return 120
    printf 'restored %s\n' "$3" >> "${preparation}"
    return "${restoration_status}"
  }
  engine_operation() {
    printf 'operation %s\n' "$*" >> "${preparation}"
    printf '%s\n' "$1" >> "${descriptions}"
    [[ "$1" != "${config_failure}" ]] || return 53
    [[ "$*" != *'podman image exists'* ]] || return 104
    if [[ "$*" == *'podman image inspect'* ]]; then
      [[ "${PRESENCE_ACTIVE}" == false && "$*" == *'--format {{.Id}}'* ]] || return 104
    fi
    if [[ "$*" == *'podman load'* ]]; then return "${preparation_status}"; fi
    if [[ "$*" == 'validate reviewed observability Alloy configuration'* ]]; then return "${validation_status}"; fi
  }
  timed_operation() {
    printf 'timed %s\n' "$*" >> "${preparation}"
    return "${preparation_status}"
  }
  activate_outer_runtime() {
    [[ "$1" == "${socket_directory}" ]] || return 105
    ((activation_status == 0)) || return "${activation_status}"
    printf '%s\n' activated >> "${preparation}"
    PRESENCE_ACTIVE=true
  }
  PRESENCE_REQUIRE_ACTIVE=true
  for reply in present absent diagnostic warning failure; do
    PRESENCE_REPLY="${reply}" PRESENCE_ACTIVE=false observed=0
    : > "${preparation}"
    : > "${PRESENCE_CALLS}"
    : > "${presence_mutations_log}"
    if prepare_application_target outer presence-fixture "${socket_directory}" rootless; then
      create_application_edge "${socket}" presence-fixture owned-run || observed=$?
    else
      observed=$?
    fi
    expected=2
    [[ "${reply}" != absent ]] || expected=1
    [[ "${reply}" != present ]] || expected=0
    [[ "${observed}" == "${expected}" && "${PRESENCE_ACTIVE}" == true ]] || return 106
    grep -Fxq activated "${preparation}"
    [[ -s "${PRESENCE_CALLS}" && ! -s "${presence_mutations_log}" ]] || return 107
  done
  for failure_stage in load restoration activation; do
    preparation_status=0 restoration_status=0 activation_status=0 PRESENCE_ACTIVE=false
    [[ "${failure_stage}" != load ]] || preparation_status=41
    [[ "${failure_stage}" != restoration ]] || restoration_status=43
    [[ "${failure_stage}" != activation ]] || activation_status=42
    : > "${PRESENCE_CALLS}"
    observed=0
    prepare_application_target outer presence-fixture "${socket_directory}" rootless || observed=$?
    [[ "${observed}" == 41 || "${observed}" == 42 || "${observed}" == 43 ]] || return 108
    [[ ! -s "${PRESENCE_CALLS}" ]] || return 109
  done
  restoration_status=0
  if [[ "${application}" == observability ]]; then
    preparation_status=0 activation_status=0 validation_status=37 PRESENCE_ACTIVE=false observed=0
    : > "${PRESENCE_CALLS}"
    prepare_application_target outer presence-fixture "${socket_directory}" rootless || observed=$?
    [[ "${observed}" == 1 && "${PRESENCE_ACTIVE}" == false && ! -s "${PRESENCE_CALLS}" ]] || return 112
  fi
  # Independently enumerate every configuration/setup/copy operation. Conditional
  # invocation suppresses ambient errexit throughout the nested preparation calls.
  case "${application}" in
    nextcloud)
      config_failures=(
        'copy rootless Nextcloud network configuration'
        'prepare rootless Nextcloud network configuration'
        'create disposable Nextcloud fixture directory'
        'copy reviewed Nextcloud frontend configuration'
        'copy reviewed Nextcloud proxy configuration'
        'copy reviewed second application content'
        'copy reviewed WebDAV probe'
        'copy reviewed publication probe'
      )
      ;;
    forgejo)
      config_failures=(
        'copy rootless Forgejo network configuration'
        'prepare rootless Forgejo network configuration'
        'verify rootful Forgejo target uses stock firewall configuration'
        'create disposable Forgejo fixture directory'
        'copy reviewed Forgejo Git probe'
        'copy deterministic Forgejo repository proof'
      )
      ;;
    observability)
      config_failures=(
        'copy rootless observability network configuration'
        'prepare rootless observability network configuration'
        'create disposable observability fixture directory'
        'copy reviewed observability fixture'
      )
      ;;
  esac
  preparation_status=0 activation_status=0 validation_status=0 PRESENCE_REPLY=present
  for failure_description in "${config_failures[@]}"; do
    config_failure="${failure_description}" config_mode=rootless PRESENCE_ACTIVE=false observed=0
    [[ "${failure_description}" != 'verify rootful Forgejo target uses stock firewall configuration' ]] || config_mode=rootful
    : > "${PRESENCE_CALLS}"
    : > "${presence_mutations_log}"
    : > "${descriptions}"
    if prepare_application_target outer presence-fixture "${socket_directory}" "${config_mode}"; then
      create_application_edge "${socket}" presence-fixture owned-run || observed=$?
    else
      observed=$?
    fi
    [[ "${observed}" == 53 && "${PRESENCE_ACTIVE}" == false &&
      ! -s "${PRESENCE_CALLS}" && ! -s "${presence_mutations_log}" ]] || return 113
    [[ "$(tail -n 1 "${descriptions}")" == "${failure_description}" ]] || return 114
  done
  config_failure='' PRESENCE_ACTIVE=false observed=0
  case "${application}" in
    nextcloud) nextcloud_fixture_root() { return 54; } ;;
    forgejo) forgejo_fixture_root() { return 54; } ;;
    observability) observability_fixture_root() { return 54; } ;;
  esac
  : > "${PRESENCE_CALLS}"
  : > "${presence_mutations_log}"
  if prepare_application_target outer presence-fixture "${socket_directory}" rootless; then
    create_application_edge "${socket}" presence-fixture owned-run || observed=$?
  else
    observed=$?
  fi
  [[ "${observed}" == 54 && "${PRESENCE_ACTIVE}" == false &&
    ! -s "${PRESENCE_CALLS}" && ! -s "${presence_mutations_log}" ]] || return 115
)
for application in nextcloud forgejo observability; do
  application_presence_tests "${application}"
done
printf '%s\n' 'Nextcloud, Forgejo, and observability presence boundaries passed offline.'
