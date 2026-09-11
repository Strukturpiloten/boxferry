#!/usr/bin/env bash
# Run BoxFerry against isolated, nested Podman instances from the reviewed matrix.
# This is intentionally opt-in: it pulls trusted images and starts privileged containers.

set -Eeuo pipefail
exec 3>&2

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repository_root="$(cd -- "${script_directory}/.." && pwd -P)"
readonly script_directory repository_root
# Scenario helper path is runtime-derived from this checked-in entry point.
# shellcheck source=scripts/lib/scenario-contract.sh
source "${script_directory}/lib/scenario-contract.sh"
# shellcheck source=scripts/lib/scenario-validators.sh
source "${script_directory}/lib/scenario-validators.sh"
# shellcheck source=scripts/lib/nextcloud-application.sh
source "${script_directory}/lib/nextcloud-application.sh"
# shellcheck source=scripts/lib/forgejo-application.sh
source "${script_directory}/lib/forgejo-application.sh"
# shellcheck source=scripts/lib/paperless-application.sh
source "${script_directory}/lib/paperless-application.sh"
# shellcheck source=scripts/lib/immich-application.sh
source "${script_directory}/lib/immich-application.sh"
# shellcheck source=scripts/lib/observability-application.sh
source "${script_directory}/lib/observability-application.sh"
# shellcheck source=scripts/lib/supabase-application.sh
source "${script_directory}/lib/supabase-application.sh"

profile=""
matrix_cell=""
matrix_start_at=""
candidate_cell=""
matrix_start_reached=false
engine="podman"
retain_artifacts=false
matrix_path="${repository_root}/fixtures/conformance/podman-live/matrix.tsv"
scenario_path="${repository_root}/fixtures/conformance/podman-live/scenarios.tsv"
limitation_path="${repository_root}/fixtures/conformance/podman-live/limitations.tsv"
candidate_path="${repository_root}/fixtures/conformance/podman-live/candidates.toml"
revalidation_schema_path="${repository_root}/docs/schemas/podman-limitation-revalidation-v1.schema.json"
revalidation_helper="${script_directory}/lib/podman-revalidation.py"
workload_image="quay.io/libpod/alpine@sha256:634a8f35b5f16dcf4aaa0822adc0b1964bb786fca12f6831de8ddc45e5986a00"
workload_local_tag="localhost/boxferry-live/alpine:634a8f35b5f16dcf4aaa0822adc0b1964bb786fca12f6831de8ddc45e5986a00"

revalidation_requested=false
declare -a original_arguments=("$@")
expected_option_value=""
first_parse_terminal=""
profile_option_count=0
for original_argument in "${original_arguments[@]}"; do
  if [[ -n "${expected_option_value}" ]]; then
    if [[ "${expected_option_value}" == --profile ]]; then
      profile_option_count=$((profile_option_count + 1))
      if [[ "${original_argument}" == limitation-revalidation ]]; then
        revalidation_requested=true
      fi
      if ((profile_option_count > 1)) && [[ -z "${first_parse_terminal}" ]]; then
        first_parse_terminal=error
      fi
    fi
    expected_option_value=""
    continue
  fi
  case "${original_argument}" in
    --profile | --engine | --matrix | --matrix-cell | --candidate-cell | --matrix-start-at)
      expected_option_value="${original_argument}"
      ;;
    --retain-artifacts) ;;
    -h | --help)
      if [[ -z "${first_parse_terminal}" ]]; then
        first_parse_terminal=help
      fi
      ;;
    *)
      if [[ -z "${first_parse_terminal}" ]]; then
        first_parse_terminal=error
      fi
      ;;
  esac
done
if [[ -n "${expected_option_value}" && -z "${first_parse_terminal}" ]]; then
  first_parse_terminal=error
fi
if [[ "${first_parse_terminal}" == help ]]; then
  revalidation_requested=false
fi

revalidation_evidence="${repository_root}/target/podman-revalidation/evidence-v1.json"
revalidation_preflight_started_at="$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
revalidation_preflight_phase="preflight"
revalidation_initialized=false
revalidation_complete=false
revalidation_phase="preflight"
revalidation_failure_code="invalid-invocation"

write_revalidation_initialization_failure() {
  local phase=$1 code=$2 finished_at temporary repository_commit=""
  local -a binding_arguments=(
    --candidate-cell "${candidate_cell}"
    --catalogue "${candidate_path}"
    --matrix "${matrix_path}"
    --limitations "${limitation_path}"
  )
  [[ "${revalidation_requested}" == true ]] || return 0
  case "${phase}:${code}" in
    preflight:invalid-invocation | preflight:prerequisite-unavailable | preflight:interrupted | \
      catalogue:invalid-catalogue | catalogue:interrupted | \
      evidence:evidence-invalid | evidence:interrupted) ;;
    *)
      printf 'Invalid limitation-revalidation initialization failure: %s/%s\n' \
        "${phase}" "${code}" >&2
      return 2
      ;;
  esac
  finished_at="$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
  if command -v git > /dev/null 2>&1; then
    repository_commit="$(git -C "${repository_root}" rev-parse --verify HEAD 2> /dev/null || true)"
  fi
  if [[ "${repository_commit}" =~ ^[0-9a-f]{40}$ ]]; then
    binding_arguments+=(--repository-commit "${repository_commit}")
  fi
  if [[ -x "${revalidation_helper}" ]] && command -v python3 > /dev/null 2>&1; then
    if "${revalidation_helper}" initialize-failure-evidence \
      --output "${revalidation_evidence}" \
      --phase "${phase}" \
      --code "${code}" \
      --started-at "${revalidation_preflight_started_at}" \
      --finished-at "${finished_at}" \
      "${binding_arguments[@]}"; then
      return
    fi
  fi
  [[ ! -L "${revalidation_evidence}" ]] || return 2
  mkdir -p -- "$(dirname -- "${revalidation_evidence}")"
  chmod 0700 "$(dirname -- "${revalidation_evidence}")"
  temporary="$(mktemp "$(dirname -- "${revalidation_evidence}")/.evidence-v1.json.XXXXXX")"
  chmod 0600 "${temporary}"
  printf '%s\n' \
    '{' \
    '  "schema_version": 1,' \
    '  "evidence_kind": "podman-limitation-revalidation-initialization-failure",' \
    '  "status": "failed",' \
    '  "eligibility": false,' \
    '  "invocation": {"candidate_cell": null, "repository_commit": null, "catalogues": null},' \
    "  \"timestamps\": {\"started_at\": \"${revalidation_preflight_started_at}\", \"finished_at\": \"${finished_at}\"}," \
    "  \"failure\": {\"phase\": \"${phase}\", \"code\": \"${code}\"}," \
    '  "initialization_failure": true' \
    '}' > "${temporary}"
  mv -f -- "${temporary}" "${revalidation_evidence}"
}

handle_revalidation_signal() {
  local signal=$1 status=$2
  trap - INT TERM
  if [[ "${revalidation_requested}" == true ]]; then
    if [[ "${revalidation_initialized:-false}" == true ]]; then
      revalidation_failure_code=interrupted
    else
      write_revalidation_initialization_failure \
        "${revalidation_preflight_phase}" interrupted || true
      revalidation_complete=true
    fi
  fi
  printf 'Limitation-revalidation interrupted by %s.\n' "${signal}" >&2
  exit "${status}"
}

if [[ "${revalidation_requested}" == true ]]; then
  write_revalidation_initialization_failure preflight prerequisite-unavailable
  trap 'handle_revalidation_signal INT 130' INT
  trap 'handle_revalidation_signal TERM 143' TERM
fi

profile_seen=false

usage() {
  cat << 'EOF'
Usage: scripts/podman-live-conformance.sh --profile <smoke|full-container|limitation-revalidation|application|forgejo-application|paperless-application|immich-application|observability-application|supabase-application> [OPTIONS]

Options:
  --engine <PATH>       Outer Podman executable (default: podman).
  --matrix <PATH>       Reviewed tab-separated image matrix.
  --matrix-cell <ID>    Run one exact reviewed container cell.
  --candidate-cell <ID> Run one exact reviewed limitation candidate.
  --matrix-start-at <ID>
                        Resume full-container at one exact reviewed container cell.
  --retain-artifacts    Keep target/podman-live/<run-id> after success.
  -h, --help            Show this help.

All profiles must run as root (for example, `sudo bash ...`) and operate only inside disposable
outer containers. They never mount a host Podman socket, checkout, or credentials into an image.
EOF
}

while (($# > 0)); do
  case "$1" in
    --profile)
      if (($# < 2)); then
        write_revalidation_initialization_failure preflight invalid-invocation
        printf '%s\n' '--profile requires a value.' >&2
        exit 2
      fi
      if [[ "${profile_seen}" == true ]]; then
        write_revalidation_initialization_failure preflight invalid-invocation
        printf '%s\n' '--profile may be supplied only once.' >&2
        exit 2
      fi
      profile_seen=true
      profile="${2:-}"
      shift 2
      ;;
    --engine)
      if (($# < 2)); then
        write_revalidation_initialization_failure preflight invalid-invocation
        printf '%s\n' '--engine requires a value.' >&2
        exit 2
      fi
      engine="${2:-}"
      shift 2
      ;;
    --matrix)
      if (($# < 2)); then
        write_revalidation_initialization_failure preflight invalid-invocation
        printf '%s\n' '--matrix requires a value.' >&2
        exit 2
      fi
      matrix_path="${2:-}"
      shift 2
      ;;
    --matrix-cell)
      if (($# < 2)); then
        write_revalidation_initialization_failure preflight invalid-invocation
        printf '%s\n' '--matrix-cell requires a value.' >&2
        exit 2
      fi
      matrix_cell="${2:-}"
      shift 2
      ;;
    --candidate-cell)
      if (($# < 2)); then
        write_revalidation_initialization_failure preflight invalid-invocation
        printf '%s\n' '--candidate-cell requires a value.' >&2
        exit 2
      fi
      candidate_cell="${2:-}"
      shift 2
      ;;
    --matrix-start-at)
      if (($# < 2)); then
        write_revalidation_initialization_failure preflight invalid-invocation
        printf '%s\n' '--matrix-start-at requires a value.' >&2
        exit 2
      fi
      matrix_start_at="${2:-}"
      shift 2
      ;;
    --retain-artifacts)
      retain_artifacts=true
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      write_revalidation_initialization_failure preflight invalid-invocation
      printf 'Unknown argument: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

case "${profile}" in
  smoke | full-container | limitation-revalidation | application | forgejo-application | paperless-application | immich-application | observability-application | supabase-application) ;;
  *)
    write_revalidation_initialization_failure preflight invalid-invocation
    printf '%s\n' '--profile must be smoke, full-container, limitation-revalidation, application, forgejo-application, paperless-application, immich-application, observability-application, or supabase-application.' >&2
    usage >&2
    exit 2
    ;;
esac

if [[ "${profile}" == limitation-revalidation ]]; then
  if [[ -z "${candidate_cell}" ]]; then
    write_revalidation_initialization_failure preflight invalid-invocation
    printf '%s\n' '--candidate-cell is required with --profile limitation-revalidation.' >&2
    exit 2
  fi
  if [[ -n "${matrix_cell}" || -n "${matrix_start_at}" ]]; then
    write_revalidation_initialization_failure preflight invalid-invocation
    printf '%s\n' '--matrix-cell and --matrix-start-at are invalid with --profile limitation-revalidation.' >&2
    exit 2
  fi
elif [[ -n "${candidate_cell}" ]]; then
  printf '%s\n' '--candidate-cell is valid only with --profile limitation-revalidation.' >&2
  exit 2
fi

if ((EUID != 0)); then
  write_revalidation_initialization_failure preflight prerequisite-unavailable
  printf '%s\n' 'Run this isolated nested-runtime harness as root, for example with sudo.' >&2
  exit 2
fi
if [[ ! -x "${engine}" ]] && ! command -v "${engine}" > /dev/null 2>&1; then
  write_revalidation_initialization_failure preflight prerequisite-unavailable
  printf 'Outer Podman executable is unavailable: %s\n' "${engine}" >&2
  exit 2
fi
for command in getcap jq timeout unzip python3; do
  if ! command -v "${command}" > /dev/null 2>&1; then
    write_revalidation_initialization_failure preflight prerequisite-unavailable
    printf 'Required live-conformance command is unavailable: %s\n' "${command}" >&2
    exit 2
  fi
done
if [[ "${profile}" == limitation-revalidation ]]; then
  for command in git mountpoint; do
    if ! command -v "${command}" > /dev/null 2>&1; then
      write_revalidation_initialization_failure preflight prerequisite-unavailable
      printf 'Required limitation-revalidation command is unavailable: %s\n' "${command}" >&2
      exit 2
    fi
  done
fi
if [[ "${profile}" == limitation-revalidation ]]; then
  revalidation_preflight_phase=catalogue
  write_revalidation_initialization_failure catalogue invalid-catalogue
fi
if [[ ! -f "${matrix_path}" ]]; then
  printf 'Matrix is unavailable: %s\n' "${matrix_path}" >&2
  exit 2
fi
if [[ ! -f "${scenario_path}" ]]; then
  printf 'Scenario catalogue is unavailable: %s\n' "${scenario_path}" >&2
  exit 2
fi
if [[ ! -f "${limitation_path}" ]]; then
  printf 'Limitation catalogue is unavailable: %s\n' "${limitation_path}" >&2
  exit 2
fi

if [[ "${profile}" == limitation-revalidation ]]; then
  for required_path in "${candidate_path}" "${revalidation_schema_path}" "${revalidation_helper}"; do
    if [[ ! -f "${required_path}" ]]; then
      if [[ "${required_path}" != "${candidate_path}" ]]; then
        revalidation_preflight_phase=evidence
        write_revalidation_initialization_failure evidence evidence-invalid
      fi
      printf 'Limitation-revalidation input unavailable: %s\n' "${required_path}" >&2
      exit 2
    fi
  done
fi

validate_catalogues() {
  local cells container_cells limitations scenarios
  cells="$(awk -F '\t' 'NF && $1 !~ /^#/ { count++ } END { print count + 0 }' "${matrix_path}")"
  container_cells="$(awk -F '\t' 'NF && $1 !~ /^#/ && $6 == "container" { count++ } END { print count + 0 }' "${matrix_path}")"
  limitations="$(awk -F '\t' 'NF && $1 !~ /^#/ { count++ } END { print count + 0 }' "${limitation_path}")"
  scenarios="$(awk -F '\t' 'NF && $1 !~ /^#/ { count++ } END { print count + 0 }' "${scenario_path}")"
  [[ "${cells}" == 48 && "${container_cells}" == 48 && "${scenarios}" -ge 16 ]] || {
    printf 'Unexpected live-conformance catalogue shape: cells=%s containers=%s limitations=%s scenarios=%s\n' \
      "${cells}" "${container_cells}" "${limitations}" "${scenarios}" >&2
    exit 2
  }
  awk -F '\t' '
    NF && $1 !~ /^#/ {
      if (NF != 7 || $2 !~ /@sha256:[0-9a-f]{64}$/ || $7 == "") {
        printf "Invalid pinned matrix row at line %d: %s\\n", NR, $0 > "/dev/stderr"; bad = 1
      }
    }
    END { exit bad }
  ' "${matrix_path}"
  awk -F '\t' '
    NR == FNR && NF && $1 !~ /^#/ { matrix[$1] = 1; next }
    NF && $1 !~ /^#/ {
      if (NF != 2 || !($1 in matrix) || seen[$1]++ || $2 != "helper-privilege-collision") {
        printf "Invalid limited matrix cell at line %d: %s\\n", FNR, $0 > "/dev/stderr"; bad = 1
      }
    }
    END { exit bad }
  ' "${matrix_path}" "${limitation_path}"
  scenario_catalogue_validate "${scenario_path}"
}

validate_catalogues

if [[ -n "${matrix_start_at}" && "${profile}" != full-container ]]; then
  printf '%s\n' '--matrix-start-at is valid only with --profile full-container.' >&2
  exit 2
fi
if [[ -n "${matrix_cell}" && -n "${matrix_start_at}" ]]; then
  printf '%s\n' '--matrix-cell and --matrix-start-at are mutually exclusive.' >&2
  exit 2
fi
if [[ -n "${matrix_cell}" ]] && ! awk -F '\t' -v expected="${matrix_cell}" '
  $1 == expected { matches++ }
  END { exit matches == 1 ? 0 : 1 }
' "${matrix_path}"; then
  printf 'Matrix cell must name exactly one reviewed row: %s\n' "${matrix_cell}" >&2
  exit 2
fi
if [[ -n "${matrix_start_at}" ]] && ! awk -F '\t' -v expected="${matrix_start_at}" '
  $1 == expected && $6 == "container" { matches++ }
  END { exit matches == 1 ? 0 : 1 }
' "${matrix_path}"; then
  printf 'Matrix start must name exactly one reviewed container row: %s\n' "${matrix_start_at}" >&2
  exit 2
fi

require_scenario() {
  local scenario=$1
  scenario_require "${scenario_path}" "${scenario}" || {
    printf 'Live-conformance scenario catalogue is missing %s.\n' "${scenario}" >&2
    exit 2
  }
}

selection_scenario() {
  case "$1" in
    exact | exact-repeat) printf '%s\n' exact-small ;;
    prefix) printf '%s\n' prefix-large ;;
    label) printf '%s\n' label-large ;;
    all) printf '%s\n' all-resources ;;
    network-boundary) printf '%s\n' network-boundary ;;
    *)
      printf 'Unknown live-conformance selection: %s\n' "$1" >&2
      return 2
      ;;
  esac
}

revalidation_phase="catalogue"
revalidation_failure_code="invalid-catalogue"
revalidation_complete=false
revalidation_ready_to_finalize=false
revalidation_capture_checks=false
declare -A revalidation_checks=()
declare -a revalidation_required_checks=()
declare -a revalidation_contract_arguments=()
revalidation_baseline_runtime_identity=""
revalidation_baseline_socket_namespace=""
revalidation_baseline_graph_root_identity=""
revalidation_baseline_name_prefix=""
revalidation_baseline_outer=""
revalidation_candidate_outer=""
revalidation_apply_target_outer=""
revalidation_mounted_image_active=""
revalidation_mounted_image_root=""
candidate_baseline_image=""
candidate_replacement_image=""
candidate_version=""
candidate_distribution=""
candidate_baseline_observed_distribution=""
candidate_replacement_observed_distribution=""
candidate_mode=""
candidate_lane=""
candidate_architecture=""
candidate_limitation=""
candidate_published_at=""
candidate_source_repository=""
candidate_source_revision=""
candidate_source_license=""
candidate_redistribution=""

run_id="bf65-$(date -u +%Y%m%dt%H%M%Sz)-$$-${RANDOM}"
artifact_root="${repository_root}/target/podman-live/${run_id}"
runtime_root="$(mktemp -d /tmp/boxferry-podman-live.XXXXXX)"
workload_archive="${runtime_root}/workload-image.tar"
suite_started_at="$(date +%s)"
readonly run_id artifact_root runtime_root workload_image workload_local_tag workload_archive suite_started_at
mkdir -p -- "${artifact_root}"
chmod 0700 "${artifact_root}"
chmod 0700 "${runtime_root}"

declare -a outer_containers=()
declare -a discovery_directories=()
declare -a fault_proxy_pids=()
declare -a fault_proxy_sockets=()
declare -a mounted_images=()
# Only images which this invocation had to pull may be removed.  The live
# runner shares its host store with the runner image cache, so removing a
# reviewed image which was already present would be surprising (and can make
# a later matrix cell needlessly pull it again).
declare -a run_owned_matrix_images=()
declare -A run_owned_matrix_image_seen=()
discovery_parent_created=false
started_outer=""
apply_target_outer=""
apply_target_socket=""
current_podman_major=""
current_podman_rootless=""
# shellcheck disable=SC2034 # Consumed by sourced scenario validators.
current_default_podman_network_present=""
current_selected_container_id=""
progress_active=false
progress_failure_reported=false
progress_index=0
progress_started_at=0
progress_test_name=""
progress_total=0

timestamp() {
  date -u '+%Y-%m-%dT%H:%M:%SZ'
}

format_duration() {
  local seconds=$1
  printf '%dm %02ds' "$((seconds / 60))" "$((seconds % 60))"
}

mark_revalidation_result() {
  local result=$1
  "${revalidation_helper}" mark-result \
    --evidence "${revalidation_evidence}" \
    "${revalidation_contract_arguments[@]}" \
    --result "${result}"
}

record_revalidation_observation() {
  local field=$1 value=$2
  "${revalidation_helper}" record-observation \
    --evidence "${revalidation_evidence}" \
    "${revalidation_contract_arguments[@]}" \
    --field "${field}" \
    --value "${value}"
}

persist_revalidation_progress_result() {
  local name=$1 exporter selection
  case "${name}" in
    'create full resources before acquisition starts')
      mark_revalidation_result runtime_results.resource_creation
      ;;
    'verify live runtime semantics (9 scenario groups) and start acquisition socket')
      mark_revalidation_result runtime_results.runtime_semantics
      ;;
    'reject literal glob selector')
      mark_revalidation_result runtime_results.literal_glob_rejection
      ;;
    'verify deterministic Compose export')
      mark_revalidation_result runtime_results.deterministic_export
      ;;
    'block lossy import under strict policy')
      mark_revalidation_result runtime_results.strict_policy
      ;;
    'write redacted support bundle')
      mark_revalidation_result runtime_results.support_bundle
      ;;
    'diagnose malformed selected container')
      mark_revalidation_result runtime_results.malformed_response
      ;;
    'diagnose disappeared selected container')
      mark_revalidation_result runtime_results.disappeared_resource
      ;;
    'diagnose partial inventory section')
      mark_revalidation_result runtime_results.partial_inventory
      ;;
    're-import generated Compose and Quadlet outputs')
      mark_revalidation_result reimports.compose
      mark_revalidation_result reimports.quadlet
      ;;
    'externally apply and reacquire Podman plan')
      mark_revalidation_result external_apply.performed
      mark_revalidation_result external_apply.plan_applied
      mark_revalidation_result external_apply.reacquired
      ;;
    'verify SELinux relabel omission and promotion')
      mark_revalidation_result runtime_results.selinux_intent
      ;;
    convert\ exact\ container\ to\ *) selection=exact ;;
    convert\ prefix\ selection\ to\ *) selection=prefix ;;
    convert\ label\ selection\ to\ *) selection=label ;;
    convert\ all\ resources\ to\ *) selection=all ;;
    convert\ network\ boundary\ to\ *) selection=network ;;
    *) return 0 ;;
  esac
  if [[ -n "${selection:-}" ]]; then
    exporter="${name##* }"
    mark_revalidation_result "selector_exporters.${selection}.${exporter}"
  fi
}

progress_begin() {
  progress_test_name=$1
  if [[ "${revalidation_capture_checks}" == true ]]; then
    case "${progress_test_name}" in
      'start isolated Podman container and collect runtime evidence')
        revalidation_phase="replacement-runtime"
        revalidation_failure_code="runtime-metadata-mismatch"
        ;;
      'externally apply and reacquire Podman plan')
        revalidation_phase="external-apply"
        revalidation_failure_code="external-apply-failed"
        ;;
      'remove disposable outer container')
        revalidation_phase="cleanup"
        revalidation_failure_code="cleanup-failed"
        ;;
      *)
        revalidation_phase="resource-suite"
        revalidation_failure_code="resource-contract-failed"
        ;;
    esac
  fi
  progress_index=$((progress_index + 1))
  progress_started_at="$(date +%s)"
  progress_active=true
  progress_failure_reported=false
  printf '%s TEST %d/%d START %s\n' \
    "$(timestamp)" "${progress_index}" "${progress_total}" "${progress_test_name}"
}

progress_pass() {
  local elapsed=$(($(date +%s) - progress_started_at))
  printf '%s TEST %d/%d PASS  %s (%s)\n' \
    "$(timestamp)" "${progress_index}" "${progress_total}" "${progress_test_name}" \
    "$(format_duration "${elapsed}")"
  progress_active=false
  if [[ "${revalidation_capture_checks}" == true ]]; then
    if [[ -n "${revalidation_checks[${progress_test_name}]:-}" ]]; then
      printf 'Limitation revalidation repeated mandatory check: %s\n' "${progress_test_name}" >&2
      return 2
    fi
    revalidation_checks["${progress_test_name}"]=true
    persist_revalidation_progress_result "${progress_test_name}"
  fi
}

progress_fail() {
  local status=$?
  if [[ "${progress_active}" == true && "${progress_failure_reported}" == false ]]; then
    local elapsed=$(($(date +%s) - progress_started_at))
    printf '%s TEST %d/%d FAIL  %s (%s, exit %d)\n' \
      "$(timestamp)" "${progress_index}" "${progress_total}" "${progress_test_name}" \
      "$(format_duration "${elapsed}")" "${status}" >&2
    progress_failure_reported=true
    progress_active=false
  fi
  return "${status}"
}

progress_run() {
  local name=$1
  shift
  progress_begin "${name}"
  "$@"
  progress_pass
}

startup_substep() {
  local name=$1 started_at elapsed status
  shift
  started_at="$(date +%s)"
  printf '%s STEP START %s\n' "$(timestamp)" "${name}" >&2
  if "$@"; then
    elapsed=$(($(date +%s) - started_at))
    printf '%s STEP PASS  %s (%s)\n' \
      "$(timestamp)" "${name}" "$(format_duration "${elapsed}")" >&2
    return 0
  else
    status=$?
  fi
  elapsed=$(($(date +%s) - started_at))
  printf '%s STEP FAIL  %s (%s, exit %d)\n' \
    "$(timestamp)" "${name}" "$(format_duration "${elapsed}")" "${status}" >&2
  return "${status}"
}

timed_operation() {
  local deadline=$1 name=$2 started_at elapsed status
  shift 2
  started_at="$(date +%s)"
  printf '%s STEP START %s (deadline %s)\n' "$(timestamp)" "${name}" "${deadline}" >&3
  if timeout --signal=TERM --kill-after=10s "${deadline}" "$@"; then
    elapsed=$(($(date +%s) - started_at))
    printf '%s STEP PASS  %s (%s)\n' \
      "$(timestamp)" "${name}" "$(format_duration "${elapsed}")" >&3
    return 0
  else
    status=$?
  fi
  elapsed=$(($(date +%s) - started_at))
  printf '%s STEP FAIL  %s (%s, exit %d)\n' \
    "$(timestamp)" "${name}" "$(format_duration "${elapsed}")" "${status}" >&3
  return "${status}"
}

expected_failure_operation() {
  local deadline=$1 name=$2 expected_status=$3 started_at elapsed status
  shift 3
  started_at="$(date +%s)"
  printf '%s STEP START %s (expect exit %d, deadline %s)\n' \
    "$(timestamp)" "${name}" "${expected_status}" "${deadline}" >&3
  if timeout --signal=TERM --kill-after=10s "${deadline}" "$@"; then
    elapsed=$(($(date +%s) - started_at))
    printf '%s STEP FAIL  %s unexpectedly succeeded (%s)\n' \
      "$(timestamp)" "${name}" "$(format_duration "${elapsed}")" >&3
    return 1
  else
    status=$?
  fi
  elapsed=$(($(date +%s) - started_at))
  if [[ "${status}" == "${expected_status}" ]]; then
    printf '%s STEP PASS  %s failed as expected (%s, exit %d)\n' \
      "$(timestamp)" "${name}" "$(format_duration "${elapsed}")" "${status}" >&3
    return 0
  fi
  printf '%s STEP FAIL  %s returned unexpected status (%s, exit %d)\n' \
    "$(timestamp)" "${name}" "$(format_duration "${elapsed}")" "${status}" >&3
  return "${status}"
}

engine_operation() {
  local name=$1
  shift
  timed_operation 90s "${name}" "${engine}" "$@"
}

engine_image_available() {
  local name=$1 image=$2 started_at elapsed status
  started_at="$(date +%s)"
  printf '%s STEP START %s (deadline 90s)\n' "$(timestamp)" "${name}" >&3
  if timeout --signal=TERM --kill-after=10s 90s "${engine}" image exists "${image}"; then
    elapsed=$(($(date +%s) - started_at))
    printf '%s STEP PASS  %s: present (%s)\n' \
      "$(timestamp)" "${name}" "$(format_duration "${elapsed}")" >&3
    return 0
  else
    status=$?
  fi
  elapsed=$(($(date +%s) - started_at))
  if ((status == 1)); then
    printf '%s STEP PASS  %s: absent (%s)\n' \
      "$(timestamp)" "${name}" "$(format_duration "${elapsed}")" >&3
    return 1
  fi
  printf '%s STEP FAIL  %s (%s, exit %d)\n' \
    "$(timestamp)" "${name}" "$(format_duration "${elapsed}")" "${status}" >&3
  return "${status}"
}

boxferry_operation() {
  local name=$1
  shift
  timed_operation 90s "${name}" "${boxferry_bin}" "$@"
}

record_run_owned_matrix_image() {
  local image=$1
  [[ "${profile}" == full-container ]] || return 0
  if [[ -z "${run_owned_matrix_image_seen[${image}]:-}" ]]; then
    run_owned_matrix_images+=("${image}")
    run_owned_matrix_image_seen[${image}]=true
  fi
}

release_run_owned_matrix_image() {
  local image=$1
  [[ -n "${run_owned_matrix_image_seen[${image}]:-}" ]] || return 0

  timed_operation 90s "release run-owned matrix image ${image}" \
    "${engine}" image rm --ignore --no-prune -- "${image}" > /dev/null
  unset "run_owned_matrix_image_seen[${image}]"
}

release_remaining_run_owned_matrix_images() {
  local index image release_failed=false
  local -a image_indexes=("${!run_owned_matrix_images[@]}")
  # Images come after every outer container and image mount in EXIT cleanup.
  # Reverse acquisition order keeps any future image layering dependency safe.
  for ((index = ${#image_indexes[@]} - 1; index >= 0; index--)); do
    image=${run_owned_matrix_images[${image_indexes[index]}]}
    if ! release_run_owned_matrix_image "${image}"; then
      release_failed=true
    fi
  done
  [[ "${release_failed}" == false ]]
}

trap progress_fail ERR

cleanup() {
  local status=$?
  local outer directory image pid socket existence_status
  local cleanup_failed=false
  local baseline_removed=false replacement_removed=false apply_target_removed=false
  for outer in "${outer_containers[@]}"; do
    if ! timeout --signal=TERM --kill-after=10s 30s \
      "${engine}" rm --force --ignore -- "${outer}" > /dev/null 2>&1; then
      [[ "${profile}" == limitation-revalidation ]] && cleanup_failed=true
    fi
    if [[ "${profile}" == limitation-revalidation ]]; then
      if timeout --signal=TERM --kill-after=10s 30s \
        "${engine}" container exists "${outer}" > /dev/null 2>&1; then
        cleanup_failed=true
      else
        existence_status=$?
        if ((existence_status == 1)); then
          if [[ -n "${revalidation_baseline_outer}" &&
            "${outer}" == "${revalidation_baseline_outer}" ]]; then
            baseline_removed=true
          fi
          if [[ -n "${revalidation_candidate_outer}" &&
            "${outer}" == "${revalidation_candidate_outer}" ]]; then
            replacement_removed=true
          fi
          if [[ -n "${revalidation_apply_target_outer}" &&
            "${outer}" == "${revalidation_apply_target_outer}" ]]; then
            apply_target_removed=true
          fi
        else
          cleanup_failed=true
        fi
      fi
    fi
  done
  if [[ "${profile}" == limitation-revalidation ]]; then
    if [[ -n "${revalidation_mounted_image_active}" ]] &&
      ! timeout --signal=TERM --kill-after=10s 30s \
        "${engine}" image unmount -- "${revalidation_mounted_image_active}" > /dev/null 2>&1; then
      if [[ -z "${revalidation_mounted_image_root}" ]] ||
        mountpoint --quiet -- "${revalidation_mounted_image_root}"; then
        cleanup_failed=true
      fi
    fi
  else
    for image in "${mounted_images[@]}"; do
      timeout --signal=TERM --kill-after=10s 30s \
        "${engine}" image unmount -- "${image}" > /dev/null 2>&1 || true
    done
  fi
  if ! release_remaining_run_owned_matrix_images; then
    cleanup_failed=true
  fi
  for pid in "${fault_proxy_pids[@]}"; do
    kill "${pid}" > /dev/null 2>&1 || true
    wait "${pid}" > /dev/null 2>&1 || true
    if [[ "${profile}" == limitation-revalidation ]] && kill -0 "${pid}" > /dev/null 2>&1; then
      cleanup_failed=true
    fi
  done
  for socket in "${fault_proxy_sockets[@]}"; do
    if ! rm -f -- "${socket}"; then
      [[ "${profile}" == limitation-revalidation ]] && cleanup_failed=true
    elif [[ "${profile}" == limitation-revalidation && -e "${socket}" ]]; then
      cleanup_failed=true
    fi
  done
  for directory in "${discovery_directories[@]}"; do
    rm -f -- "${directory}/podman.sock" "${directory}/bootstrap.log" \
      "${directory}/runtime-evidence.tsv" "${directory}/runtime-evidence.ready" \
      "${directory}/runtime-canaries.log" "${directory}/selected-container-id" \
      "${directory}/smoke-baseline.json" "${directory}/start-api" \
      "${directory}/resource-setup.status" "${directory}/resource-setup.status.tmp"
    rmdir -- "${directory}" 2> /dev/null || true
  done
  if [[ "${discovery_parent_created}" == true ]]; then
    rmdir -- /run/user/0 2> /dev/null || true
  fi
  if ! rm -rf -- "${runtime_root}"; then
    [[ "${profile}" == limitation-revalidation ]] && cleanup_failed=true
  fi
  if [[ "${profile}" == limitation-revalidation && -f "${revalidation_evidence}" &&
    "${cleanup_failed}" == false ]] &&
    jq --exit-status '.evidence_kind == "podman-limitation-revalidation"' \
      "${revalidation_evidence}" > /dev/null 2>&1; then
    if [[ "${baseline_removed}" == true ]] &&
      ! mark_revalidation_result cleanup.baseline_removed; then
      cleanup_failed=true
    fi
    if [[ "${replacement_removed}" == true ]] &&
      ! mark_revalidation_result cleanup.replacement_removed; then
      cleanup_failed=true
    fi
    if [[ "${apply_target_removed}" == true ]] &&
      ! mark_revalidation_result cleanup.apply_target_removed; then
      cleanup_failed=true
    fi
  fi
  if [[ "${profile}" == limitation-revalidation && "${cleanup_failed}" == true ]]; then
    revalidation_phase="cleanup"
    revalidation_failure_code="cleanup-failed"
    status=1
  fi
  if [[ "${profile}" == full-container && "${cleanup_failed}" == true && "${status}" == 0 ]]; then
    status=1
  fi
  if [[ "${profile}" == limitation-revalidation && ! -f "${revalidation_evidence}" ]]; then
    if write_revalidation_initialization_failure evidence evidence-invalid; then
      revalidation_complete=true
    fi
    status=1
  fi
  if [[ "${profile}" == limitation-revalidation && -f "${revalidation_evidence}" &&
    "${status}" == 0 && "${revalidation_ready_to_finalize}" == true ]]; then
    revalidation_phase="evidence"
    revalidation_failure_code="evidence-invalid"
    if "${revalidation_helper}" finalize-evidence \
      --evidence "${revalidation_evidence}" \
      "${revalidation_contract_arguments[@]}" &&
      "${revalidation_helper}" validate-evidence \
        --evidence "${revalidation_evidence}" \
        "${revalidation_contract_arguments[@]}"; then
      revalidation_complete=true
      printf '%s LIMITATION REVALIDATION PASS candidate=%s evidence=%s\n' \
        "$(timestamp)" "${candidate_cell}" "${revalidation_evidence}"
      printf '%s SUITE PASS profile=%s cells=1 limitations=1 duration=%s\n' \
        "$(timestamp)" "${profile}" "$(format_duration "$(($(date +%s) - suite_started_at))")"
    else
      status=1
    fi
  fi
  if [[ "${profile}" == limitation-revalidation && -f "${revalidation_evidence}" &&
    "${revalidation_complete}" != true ]]; then
    if ! "${revalidation_helper}" ensure-failure-evidence \
      --evidence "${revalidation_evidence}" \
      "${revalidation_contract_arguments[@]}" \
      --phase "${revalidation_phase}" \
      --code "${revalidation_failure_code}"; then
      printf '%s\n' 'Unable to finalize failed limitation-revalidation evidence.' >&2
      if write_revalidation_initialization_failure evidence evidence-invalid; then
        revalidation_complete=true
      fi
      status=1
    fi
    if ((status == 0)); then
      status=1
    fi
  fi
  if [[ -f "${artifact_root}/evidence.tsv" ]] && [[ "$(wc -l < "${artifact_root}/evidence.tsv")" -gt 1 ]]; then
    printf 'Live-conformance verified evidence:\n'
    cat -- "${artifact_root}/evidence.tsv"
  fi
  if ((status == 0)) && [[ "${retain_artifacts}" != true ]]; then
    rm -rf -- "${artifact_root}"
  elif ((status != 0)); then
    printf 'Live-conformance artifacts retained at %s\n' "${artifact_root}" >&2
  fi
  exit "${status}"
}
trap cleanup EXIT

if [[ "${profile}" == limitation-revalidation ]]; then
  repository_commit="$(git -C "${repository_root}" rev-parse --verify HEAD)"
  repository_clean_tree=true
  if [[ -n "$(git -C "${repository_root}" status --porcelain --untracked-files=all)" ]]; then
    repository_clean_tree=false
  fi
  evidence_provider=local
  if [[ "${GITHUB_ACTIONS:-}" == true ]]; then
    evidence_provider=github-actions
  fi
  if ! "${revalidation_helper}" initialize-evidence \
    --catalogue "${candidate_path}" \
    --matrix "${matrix_path}" \
    --limitations "${limitation_path}" \
    --schema "${revalidation_schema_path}" \
    --candidate-cell "${candidate_cell}" \
    --output "${revalidation_evidence}" \
    --repository-commit "${repository_commit}" \
    --repository-clean-tree "${repository_clean_tree}" \
    --provider "${evidence_provider}"; then
    revalidation_complete=true
    exit 2
  fi
  revalidation_contract_arguments=(
    --catalogue "${candidate_path}"
    --matrix "${matrix_path}"
    --limitations "${limitation_path}"
    --schema "${revalidation_schema_path}"
    --candidate-cell "${candidate_cell}"
    --expected-repository-commit "${repository_commit}"
  )
  "${revalidation_helper}" validate-evidence \
    --evidence "${revalidation_evidence}" \
    "${revalidation_contract_arguments[@]}"
  revalidation_initialized=true
  candidate_record="$(
    "${revalidation_helper}" resolve-candidate \
      --catalogue "${candidate_path}" \
      --matrix "${matrix_path}" \
      --limitations "${limitation_path}" \
      --candidate-cell "${candidate_cell}" \
      --format tsv
  )"
  IFS=$'\t' read -r candidate_cell candidate_baseline_image candidate_replacement_image \
    candidate_version candidate_distribution candidate_baseline_observed_distribution \
    candidate_replacement_observed_distribution candidate_mode candidate_lane candidate_architecture \
    candidate_limitation candidate_published_at candidate_source_repository candidate_source_revision \
    candidate_source_license candidate_redistribution <<< "${candidate_record}"
  [[ -n "${candidate_redistribution}" ]] || {
    printf '%s\n' 'Candidate resolution returned an incomplete record.' >&2
    exit 2
  }
  [[ "${candidate_limitation}" == helper-privilege-collision ]] || {
    printf '%s\n' 'Candidate resolution returned an unexpected limitation.' >&2
    exit 2
  }
fi

require_binary() {
  local candidate="${BOXFERRY_BIN:-${repository_root}/target/debug/boxferry}"
  if [[ ! -x "${candidate}" ]]; then
    (cd -- "${repository_root}" && cargo build --locked --package boxferry --bin boxferry --features podman)
  fi
  if [[ ! -x "${candidate}" ]]; then
    printf 'BoxFerry binary is unavailable after build: %s\n' "${candidate}" >&2
    exit 1
  fi
  printf '%s\n' "${candidate}"
}

boxferry_bin="$(require_binary)"
readonly boxferry_bin

outer_engine_version="$(engine_operation 'read outer Podman version' --version | tr '\n' ' ')"
readonly outer_engine_version
printf '%s HOST outer-engine=%s version=%s cpus=%s\n' \
  "$(timestamp)" "${engine}" "${outer_engine_version}" "$(getconf _NPROCESSORS_ONLN)"
awk -v timestamp="$(timestamp)" '
  /^MemTotal:/ { total = $2 }
  /^MemAvailable:/ { available = $2 }
  END { printf "%s HOST memory-total-kib=%d memory-available-kib=%d\n", timestamp, total, available }
' /proc/meminfo
df -Pk -- "${repository_root}" | awk -v timestamp="$(timestamp)" '
  NR == 2 {
    printf "%s HOST disk-total-kib=%s disk-available-kib=%s path=%s\n", \
      timestamp, $2, $4, $6
  }
'

contains_smoke_cell() {
  case "$1" in
    podman-5.4-rootless | podman-6.1-rootful | podman-6.1-rootless | \
      podman-debian-11-rootful | podman-debian-11-rootless | podman-debian-12-rootful | \
      podman-ubi-8-rootful | podman-ubuntu-22.04-rootless | podman-ubuntu-24.04-rootless)
      return 0
      ;;
    *) return 1 ;;
  esac
}

is_complete_resource_profile() {
  [[ "${profile}" == full-container || "${profile}" == limitation-revalidation ||
    "${profile}" == observability-application || "${profile}" == supabase-application ]]
}

selected() {
  local id=$1 lane=$2
  if [[ "${profile}" == full-container && -n "${matrix_start_at}" && "${matrix_start_reached}" != true ]]; then
    if [[ "${id}" != "${matrix_start_at}" ]]; then
      return 1
    fi
    matrix_start_reached=true
  fi
  case "${profile}" in
    smoke) contains_smoke_cell "${id}" && [[ -z "${matrix_cell}" || "${id}" == "${matrix_cell}" ]] ;;
    full-container) [[ "${lane}" == container && (-z "${matrix_cell}" || "${id}" == "${matrix_cell}") ]] ;;
    application) [[ "${id}" == podman-6.1-rootless && (-z "${matrix_cell}" || "${id}" == "${matrix_cell}") ]] ;;
    forgejo-application)
      [[ ("${id}" == podman-arch-rootful || "${id}" == podman-6.1-rootless) &&
        (-z "${matrix_cell}" || "${id}" == "${matrix_cell}") ]]
      ;;
    paperless-application)
      [[ "${id}" == podman-6.1-rootless &&
        (-z "${matrix_cell}" || "${id}" == "${matrix_cell}") ]]
      ;;
    immich-application)
      [[ "${id}" == podman-6.1-rootless &&
        (-z "${matrix_cell}" || "${id}" == "${matrix_cell}") ]]
      ;;
    observability-application)
      [[ "${id}" == podman-6.1-rootless &&
        (-z "${matrix_cell}" || "${id}" == "${matrix_cell}") ]]
      ;;
    supabase-application)
      [[ "${id}" == podman-6.1-rootless &&
        (-z "${matrix_cell}" || "${id}" == "${matrix_cell}") ]]
      ;;
  esac
}

limited_cell() {
  local id=$1
  awk -F '\t' -v expected="${id}" '
    $1 == expected { matches++ }
    END { exit matches == 1 ? 0 : 1 }
  ' "${limitation_path}"
}

wait_for_socket() {
  local socket=$1
  for _ in {1..60}; do
    [[ -S "${socket}" ]] && return 0
    sleep 1
  done
  printf 'Timed out waiting for nested Podman socket: %s\n' "${socket}" >&2
  return 1
}

wait_for_file() {
  local file=$1 description=$2
  for _ in {1..60}; do
    [[ -s "${file}" ]] && return 0
    sleep 1
  done
  printf 'Timed out waiting for %s: %s\n' "${description}" "${file}" >&2
  return 1
}

wait_for_workload_completion() {
  local status_file=$1 log_file=$2 deadline_seconds=$3
  local elapsed=0 last_event
  while ((elapsed < deadline_seconds)); do
    if [[ -s "${status_file}" ]]; then
      return 0
    fi
    sleep 1
    ((elapsed += 1))
    if ((elapsed % 15 == 0)); then
      last_event="$(tail -n 1 -- "${log_file}" 2> /dev/null || true)"
      if [[ -n "${last_event}" ]]; then
        printf '%s STEP WAIT  nested resource setup (%s elapsed; last event: %s)\n' \
          "$(timestamp)" "$(format_duration "${elapsed}")" "${last_event}" >&2
      else
        printf '%s STEP WAIT  nested resource setup (%s elapsed; no event logged yet)\n' \
          "$(timestamp)" "$(format_duration "${elapsed}")" >&2
      fi
    fi
  done
  printf 'Timed out waiting for nested resource completion: %s\n' "${status_file}" >&2
  return 1
}

activate_outer_runtime() {
  local socket_directory=$1
  rm -f -- "${socket_directory}/podman.sock"
  : > "${socket_directory}/start-api"
  if ! startup_substep 'wait for nested Podman socket (deadline 60s)' \
    wait_for_socket "${socket_directory}/podman.sock"; then
    cat -- "${socket_directory}/bootstrap.log" >&2 || true
    return 1
  fi
}

create_workloads() {
  local outer=$1 prefix=$2 scope=${3:-full} socket_directory=$4
  local include_canaries=${5:-true} deadline=5m deadline_seconds=300
  local completion_file="${socket_directory}/resource-setup.status" setup_status
  if [[ "${scope}" == minimal ]]; then
    deadline=2m
    deadline_seconds=120
  fi
  rm -f -- "${completion_file}" "${completion_file}.tmp"
  printf 'Live setup: create %s nested resources for %s\n' "${scope}" "${prefix}"
  # shellcheck disable=SC2016 # ${...} expands in the nested shell, not this script.
  if ! startup_substep "launch ${scope} nested resource setup (deadline 30s)" \
    timeout --signal=TERM --kill-after=10s 30s \
    "${engine}" exec --detach --env "BF_PREFIX=${prefix}" \
    --env "BF_WORKLOAD_IMAGE=${workload_local_tag}" --env "BF_WORKLOAD_SCOPE=${scope}" \
    --env "BF_INCLUDE_CANARIES=${include_canaries}" \
    "${outer}" /bin/sh -ceu '
    image="${BF_WORKLOAD_IMAGE}"
    portable_image="registry.example.invalid/boxferry/${BF_PREFIX}:1"
    run_label="--label io.boxferry.live-run=${BF_PREFIX}"
    compose_labels="${run_label} --label com.docker.compose.project=${BF_PREFIX}"
    canary_log=/boxferry-socket/runtime-canaries.log
    completion_file=/boxferry-socket/resource-setup.status
    rm -f "${completion_file}" "${completion_file}.tmp"
    : > "${canary_log}"
    # Nested conmon/slirp helpers may outlive their parent command.  Keep their output in the
    # bind-mounted log and publish an atomic completion status instead of waiting on an attached
    # outer exec stream whose descriptors a helper could retain.
    exec > /boxferry-socket/bootstrap.log 2>&1
    trap "status=\$?; trap - 0; printf \"%s\\n\" \"\${status}\" > \"\${completion_file}.tmp\"; mv -f \"\${completion_file}.tmp\" \"\${completion_file}\"; exit \"\${status}\"" 0
    nested_begin() {
      nested_name=$1
      nested_started_at="$(date +%s)"
      printf "%s NESTED STEP START %s\n" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${nested_name}" >&2
    }
    nested_pass() {
      nested_elapsed="$(( $(date +%s) - nested_started_at ))"
      printf "%s NESTED STEP PASS  %s (%ss)\n" \
        "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${nested_name}" "${nested_elapsed}" >&2
    }

    nested_begin "prepare workload image"
    podman load --input /boxferry-workload.tar
    podman image exists "${image}"
    podman tag "${image}" "${portable_image}"
    major="$(podman version --format "{{.Client.Version}}" | cut -d. -f1)"
    rootless="$(podman info --format "{{.Host.Security.Rootless}}")"
    nested_pass

    nested_begin "create small network, volume, and selected container"
    podman network create "${BF_PREFIX}-small-net"
    podman volume create "${BF_PREFIX}-small-data"
    selected_container_id="$(podman create --name "${BF_PREFIX}-small-web" \
      --label "io.boxferry.live-run=${BF_PREFIX}" --network "${BF_PREFIX}-small-net" \
      --volume "${BF_PREFIX}-small-data:/var/lib/boxferry" \
      --env BOXFERRY_LIVE_MODE=small "${portable_image}" sleep 3600)"
    printf "%s\n" "${selected_container_id}" > /boxferry-socket/selected-container-id
    nested_pass
    if [ "${BF_WORKLOAD_SCOPE}" = minimal ]; then
      if [ "${BF_INCLUDE_CANARIES}" = true ]; then
        nested_begin "create minimal running runtime canary"
        podman run -d --name "${BF_PREFIX}-running" ${run_label} \
          --network none "${portable_image}" sleep 3600 \
          >> "${canary_log}" 2>&1 < /dev/null
        nested_pass
      fi
      nested_begin "create minimal stopped runtime evidence"
      podman create --name "${BF_PREFIX}-stopped" ${run_label} \
        --network "${BF_PREFIX}-small-net" "${portable_image}" true
      nested_pass
      if [ "${BF_INCLUDE_CANARIES}" = true ]; then
        nested_begin "capture minimal runtime baseline"
        podman inspect "${BF_PREFIX}-small-web" "${BF_PREFIX}-running" \
          "${BF_PREFIX}-stopped" > /boxferry-socket/smoke-baseline.json
        nested_pass
      fi
      exit 0
    fi

    nested_begin "create apply and large network-volume topology"
    podman network create "${BF_PREFIX}-apply-net"
    podman volume create "${BF_PREFIX}-apply-data"
    podman create --name "${BF_PREFIX}-apply-web" ${run_label} \
      --network "${BF_PREFIX}-apply-net" --volume "${BF_PREFIX}-apply-data:/srv/data:rw" \
      --env BOXFERRY_LIVE_MODE=apply "${portable_image}" sleep 3600

    podman network create "${BF_PREFIX}-large-edge"
    subnet_octet="$(( (RANDOM % 200) + 20 ))"
    private_subnet="10.89.${subnet_octet}.0/24"
    private_api_ip="10.89.${subnet_octet}.10"
    podman network create --internal --subnet "${private_subnet}" "${BF_PREFIX}-large-private"
    podman volume create "${BF_PREFIX}-large-data"
    podman volume create "${BF_PREFIX}-large-cache"
    nested_pass

    nested_begin "create large service topology"
    podman create --name "${BF_PREFIX}-large-db" ${compose_labels} \
      --label com.docker.compose.service=db --network "${BF_PREFIX}-large-private" \
      --volume "${BF_PREFIX}-large-data:/var/lib/boxferry" \
      --env BOXFERRY_LIVE_ROLE=database "${portable_image}" sleep 3600
    podman create --name "${BF_PREFIX}-large-cache" ${compose_labels} \
      --label com.docker.compose.service=cache --network "${BF_PREFIX}-large-private" \
      --volume "${BF_PREFIX}-large-cache:/var/cache/boxferry" \
      --env BOXFERRY_LIVE_ROLE=cache "${portable_image}" sleep 3600
    podman create --name "${BF_PREFIX}-large-api" ${compose_labels} \
      --label com.docker.compose.service=api --network "${BF_PREFIX}-large-private" --network-alias api \
      --ip "${private_api_ip}" --dns 1.1.1.1 --publish 127.0.0.1::8080 \
      --volume "${BF_PREFIX}-large-cache:/cache:ro" \
      --env BOXFERRY_LIVE_ROLE=api "${portable_image}" sleep 3600
    if [ "${major}" -ge 4 ] || [ "${rootless}" != true ]; then
      podman network connect --alias public-api "${BF_PREFIX}-large-edge" "${BF_PREFIX}-large-api"
    fi
    if [ "${major}" -ge 4 ]; then
      podman create --name "${BF_PREFIX}-large-worker" ${compose_labels} \
        --label com.docker.compose.service=worker --network "${BF_PREFIX}-large-private" \
        --requires "${BF_PREFIX}-large-db" --env BOXFERRY_LIVE_ROLE=worker "${portable_image}" sleep 3600
    else
      podman create --name "${BF_PREFIX}-large-worker" ${compose_labels} \
        --label com.docker.compose.service=worker --network "${BF_PREFIX}-large-private" \
        --env BOXFERRY_LIVE_ROLE=worker "${portable_image}" sleep 3600
    fi
    podman create --name "${BF_PREFIX}-large-proxy" ${compose_labels} \
      --label com.docker.compose.service=proxy --network "${BF_PREFIX}-large-edge" \
      --env BOXFERRY_LIVE_ROLE=proxy "${portable_image}" sleep 3600
    nested_pass

    # Lifecycle and topology are deliberately separate from the main application.  They make
    # observation richer without making a successful route depend on a particular runtime field.
    nested_begin "create runtime-state and pod topology"
    if [ "${BF_INCLUDE_CANARIES}" = true ]; then
      podman run -d --name "${BF_PREFIX}-running" ${run_label} \
        --network none "${portable_image}" sleep 3600 \
        >> "${canary_log}" 2>&1 < /dev/null
    fi
    podman create --name "${BF_PREFIX}-stopped" ${run_label} \
      --network "${BF_PREFIX}-small-net" "${portable_image}" true
    if [ "${major}" -ge 4 ]; then
      podman pod create --infra=false --name "${BF_PREFIX}-pod" --label io.boxferry.live-run="${BF_PREFIX}"
      podman create --name "${BF_PREFIX}-pod-member" ${run_label} \
        --pod "${BF_PREFIX}-pod" "${portable_image}" sleep 3600
      podman pod create --infra=false --name "${BF_PREFIX}-pod-secondary" --label io.boxferry.live-run="${BF_PREFIX}"
      podman create --name "${BF_PREFIX}-pod-secondary-member" ${run_label} \
        --pod "${BF_PREFIX}-pod-secondary" "${portable_image}" sleep 3600
    fi
    nested_pass

    nested_begin "create environment, mount, and runtime-policy evidence"
    mkdir -p /tmp/boxferry-live-bind
    printf "BOXFERRY_ENV_FILE=present\\nBOXFERRY_PROTECTED_TOKEN=not-a-secret-test-value\\n" > /tmp/boxferry-live.env
    if [ "${major}" -ge 4 ]; then
      podman create --name "${BF_PREFIX}-options" ${run_label} \
        --network "${BF_PREFIX}-large-private" --volume /tmp/boxferry-live-bind:/bind:ro,rprivate \
        --tmpfs /scratch:rw,size=65536 --env-file /tmp/boxferry-live.env --env BOXFERRY_ENV=inline \
        --annotation io.boxferry.live=present --restart on-failure:3 --ipc private --pid private --uts private \
        --cap-drop ALL --memory 96m --pids-limit 64 --log-driver k8s-file \
        --health-cmd /bin/true --health-interval 1h "${portable_image}" sleep 3600
    else
      podman create --name "${BF_PREFIX}-options" ${run_label} \
        --network "${BF_PREFIX}-large-private" --volume /tmp/boxferry-live-bind:/bind:ro,rprivate \
        --tmpfs /scratch:rw,size=65536 --env-file /tmp/boxferry-live.env --env BOXFERRY_ENV=inline \
        --cap-drop ALL "${portable_image}" sleep 3600
    fi
    nested_pass
    if [ "${major}" -ge 4 ] && [ "${BF_INCLUDE_CANARIES}" = true ]; then
      nested_begin "create and verify health-state evidence"
      podman run -d --name "${BF_PREFIX}-healthy" ${run_label} \
        --network none --health-cmd /bin/true --health-interval 1h --health-retries 1 \
        "${portable_image}" sleep 3600 >> "${canary_log}" 2>&1 < /dev/null
      podman run -d --name "${BF_PREFIX}-unhealthy" ${run_label} \
        --network none --health-cmd /bin/false --health-interval 1h --health-retries 1 \
        "${portable_image}" sleep 3600 >> "${canary_log}" 2>&1 < /dev/null
      podman healthcheck run "${BF_PREFIX}-healthy"
      podman healthcheck run "${BF_PREFIX}-unhealthy" || true
      healthy="$(podman inspect --format "{{.State.Health.Status}}" "${BF_PREFIX}-healthy")"
      unhealthy="$(podman inspect --format "{{.State.Health.Status}}" "${BF_PREFIX}-unhealthy")"
      [ "${healthy}" = healthy ] && [ "${unhealthy}" = unhealthy ]
      nested_pass
    fi
    nested_begin "create conditional secret evidence when supported"
    if podman secret --help > /dev/null 2>&1 && \
      printf "boxferry-live-not-a-secret\\n" | podman secret create "${BF_PREFIX}-conditional" - > /dev/null 2>&1; then
      podman create --name "${BF_PREFIX}-secret" ${run_label} \
        --network "${BF_PREFIX}-large-private" --secret "${BF_PREFIX}-conditional" "${portable_image}" sleep 3600
    fi
    nested_pass
  ' > /dev/null; then
    cat -- "${socket_directory}/bootstrap.log" >&2 || true
    cat -- "${socket_directory}/runtime-canaries.log" >&2 || true
    return 1
  fi
  if ! startup_substep "wait for ${scope} nested resources (deadline ${deadline})" \
    wait_for_workload_completion "${completion_file}" "${socket_directory}/bootstrap.log" \
    "${deadline_seconds}"; then
    cat -- "${socket_directory}/bootstrap.log" >&2 || true
    cat -- "${socket_directory}/runtime-canaries.log" >&2 || true
    return 1
  fi
  IFS= read -r setup_status < "${completion_file}" || true
  if [[ ! "${setup_status}" =~ ^[0-9]+$ ]] || ((setup_status > 255)); then
    printf 'Nested resource setup wrote an invalid completion status: %q\n' "${setup_status}" >&2
    cat -- "${socket_directory}/bootstrap.log" >&2 || true
    cat -- "${socket_directory}/runtime-canaries.log" >&2 || true
    return 1
  fi
  if ((setup_status != 0)); then
    printf 'Nested resource setup failed with exit %d.\n' "${setup_status}" >&2
    cat -- "${socket_directory}/bootstrap.log" >&2 || true
    cat -- "${socket_directory}/runtime-canaries.log" >&2 || true
    return "${setup_status}"
  fi
  printf 'Live setup: nested resources ready for %s\n' "${prefix}"
}

prepare_workload_archive() {
  if [[ -s "${workload_archive}" ]]; then
    return
  fi
  local expected_digest="${workload_image##*@}" resolved_digest cache_status=0
  printf 'Live setup: prepare digest-pinned workload archive\n'
  engine_image_available 'probe workload image cache' "${workload_image}" || cache_status=$?
  if ((cache_status == 1)); then
    timed_operation 5m 'pull digest-pinned workload image' \
      "${engine}" pull --quiet "${workload_image}" \
      > "${artifact_root}/workload-image.pull.log"
  elif ((cache_status != 0)); then
    return "${cache_status}"
  fi
  resolved_digest="$(engine_operation 'inspect workload image digest' \
    image inspect --format '{{.Digest}}' "${workload_image}")"
  if [[ "${resolved_digest}" != "${expected_digest}" ]]; then
    printf 'Resolved workload image digest mismatch: expected %s, observed %s\n' \
      "${expected_digest}" "${resolved_digest}" >&2
    return 1
  fi
  engine_operation 'tag workload image for nested loading' \
    tag "${workload_image}" "${workload_local_tag}"
  timed_operation 5m 'archive workload image for nested loading' \
    "${engine}" save --format docker-archive \
    --output "${workload_archive}" "${workload_local_tag}"
  chmod 0644 "${workload_archive}"
}

prepare_matrix_image() {
  local id=$1 image=$2
  local classify_revalidation_digest=${3:-false}
  local expected_digest="${image##*@}"
  local resolved_digest cache_status=0
  engine_image_available "probe ${id} image cache" "${image}" || cache_status=$?
  if ((cache_status == 0)); then
    resolved_digest="$(engine_operation "inspect ${id} image digest" \
      image inspect --format '{{.Digest}}' "${image}")"
    printf 'Using cached matrix image with verified digest %s.\n' "${resolved_digest}" \
      > "${artifact_root}/${id}.pull.log"
  elif ((cache_status == 1)); then
    printf 'Live setup: pull reviewed outer image for %s\n' "${id}"
    timed_operation 5m "pull reviewed ${id} image" \
      "${engine}" pull --quiet "${image}" \
      > "${artifact_root}/${id}.pull.log"
    record_run_owned_matrix_image "${image}"
    resolved_digest="$(engine_operation "inspect ${id} image digest" \
      image inspect --format '{{.Digest}}' "${image}")"
  else
    return "${cache_status}"
  fi
  if [[ "${resolved_digest}" != "${expected_digest}" ]]; then
    if [[ "${classify_revalidation_digest}" == true ]]; then
      revalidation_failure_code="digest-mismatch"
    fi
    printf 'Resolved matrix image digest mismatch for %s: expected %s, observed %s\n' \
      "${id}" "${expected_digest}" "${resolved_digest}" >&2
    return 1
  fi
  printf '%s\n' "${resolved_digest}" > "${artifact_root}/${id}.digest"
}

start_outer_runtime() {
  local id=$1 image=$2 mode=$3 socket_directory=$4
  local nested_archive=${5:-${workload_archive}}
  local cleanup_role=${6:-}
  local outer_digest
  case "${cleanup_role}" in
    "" | replacement | apply-target) ;;
    *)
      printf 'Unknown limitation-revalidation cleanup role: %s\n' "${cleanup_role}" >&2
      return 2
      ;;
  esac
  outer_digest="$(printf '%s' "${id}" | sha256sum)"
  local outer="${run_id:0:36}-${outer_digest:0:16}"
  mkdir -p -- "${socket_directory}"
  chmod 0777 "${socket_directory}"
  prepare_workload_archive
  prepare_matrix_image "${id}" "${image}"
  outer_containers+=("${outer}")
  # The caller starts the API only after resource creation and matching-version CLI assertions.
  # This avoids concurrent nested CLI/API storage access and a second long-lived exec session;
  # both have deadlocked nondeterministically on GitHub-hosted outer Podman engines.
  # shellcheck disable=SC2016 # ${...} expands in the nested shell, not this script.
  startup_substep 'create detached outer container (deadline 90s)' \
    timeout --signal=TERM --kill-after=10s 90s \
    "${engine}" run --detach --rm --name "${outer}" --stop-timeout 1 --privileged --device /dev/fuse \
    --security-opt label=disable --volume "${socket_directory}:/boxferry-socket:Z" \
    --volume "${nested_archive}:/boxferry-workload.tar:ro" \
    --env "BF_SOCKET=/boxferry-socket/podman.sock" "${image}" /bin/sh -ceu '
      trap "exit 0" INT TERM
      umask 000
      rm -f "${BF_SOCKET}" /boxferry-socket/runtime-evidence.tsv \
        /boxferry-socket/runtime-evidence.ready /boxferry-socket/start-api
      exec 2> /boxferry-socket/bootstrap.log
      {
        printf "podman-version\t"
        podman --version
        printf "architecture\t"
        uname -m
        printf "package-version\t"
        if test -s /usr/share/strukturpiloten/podman-package-version; then
          cat /usr/share/strukturpiloten/podman-package-version
        else
          podman_version="$(podman --version)"
          printf "upstream-source-build:%s\n" "${podman_version#podman version }"
        fi
        printf "api-version\t%s\n" "$(podman info --format "{{.Version.APIVersion}}")"
        printf "rootless\t%s\n" "$(podman info --format "{{.Host.Security.Rootless}}")"
      } > /boxferry-socket/runtime-evidence.tsv
      printf "ready\n" > /boxferry-socket/runtime-evidence.ready
      while test ! -e /boxferry-socket/start-api; do sleep 1; done
      rm -f /boxferry-socket/start-api
      exec podman system service --time 0 "unix://${BF_SOCKET}" \
        >> /boxferry-socket/bootstrap.log 2>&1
    ' \
    > "${artifact_root}/${id}.outer.log"

  case "${cleanup_role}" in
    replacement) revalidation_candidate_outer="${outer}" ;;
    apply-target) revalidation_apply_target_outer="${outer}" ;;
    "") ;;
  esac

  if ! startup_substep 'wait for nested runtime evidence (deadline 60s)' \
    wait_for_file "${socket_directory}/runtime-evidence.ready" 'nested runtime evidence'; then
    cat -- "${socket_directory}/bootstrap.log" >&2 || true
    return 1
  fi
  [[ -s "${socket_directory}/runtime-evidence.tsv" ]] || {
    printf 'Nested runtime did not write evidence: %s\n' "${id}" >&2
    return 1
  }
  awk '$0 ~ /^podman-version\t/ { sub(/^[^\t]*\t/, ""); print }' \
    "${socket_directory}/runtime-evidence.tsv" > "${artifact_root}/${id}.podman-version"
  awk -F '\t' '$1 == "architecture" { print $2 }' \
    "${socket_directory}/runtime-evidence.tsv" > "${artifact_root}/${id}.architecture"
  awk '$0 ~ /^package-version\t/ { sub(/^[^\t]*\t/, ""); print }' \
    "${socket_directory}/runtime-evidence.tsv" > "${artifact_root}/${id}.package-version"
  awk -F '\t' '$1 == "api-version" { print $2 }' \
    "${socket_directory}/runtime-evidence.tsv" > "${artifact_root}/${id}.api-version"
  awk -F '\t' '$1 == "rootless" { print $2 }' \
    "${socket_directory}/runtime-evidence.tsv" > "${artifact_root}/${id}.rootless"
  [[ -s "${artifact_root}/${id}.podman-version" ]] && [[ -s "${artifact_root}/${id}.package-version" ]] &&
    [[ -s "${artifact_root}/${id}.api-version" ]] && [[ -s "${artifact_root}/${id}.rootless" ]] || {
    printf 'Required Podman evidence was empty for %s.\n' "${id}" >&2
    return 1
  }
  if [[ "${cleanup_role}" == replacement ]]; then
    record_revalidation_candidate_runtime_observations "${id}" "${outer}"
  fi
  if [[ "${mode}" == rootless ]]; then
    [[ "$(< "${artifact_root}/${id}.rootless")" == true ]] || {
      printf 'Matrix rootless cell did not report rootless Podman: %s\n' "${id}" >&2
      return 1
    }
  elif [[ "$(< "${artifact_root}/${id}.rootless")" != false ]]; then
    printf 'Matrix rootful cell did not report rootful Podman: %s\n' "${id}" >&2
    return 1
  fi
  started_outer="${outer}"
  if [[ "${profile}" == limitation-revalidation ]]; then
    collect_revalidation_runtime_identity "${outer}" "${artifact_root}/${id}.distribution"
  fi
}

start_outer() {
  local id=$1 image=$2 mode=$3 socket_directory=$4 prefix=$5
  start_outer_runtime "${id}" "${image}" "${mode}" "${socket_directory}"
  create_workloads "${started_outer}" "${prefix}" full "${socket_directory}" false
  activate_outer_runtime "${socket_directory}"
}

verify_observed_version() {
  local id=$1 declared_version=$2 evidence_file=$3
  local expected_version="${declared_version%%+*}"
  local observed_version
  expected_version="${expected_version%%-*}"
  observed_version="$(< "${evidence_file}")"
  observed_version="${observed_version##* }"
  # RHEL's Podman 4.9 build reports its vendor marker in `podman --version`,
  # while the reviewed image contract records the matching upstream engine
  # version and the complete RPM revision is captured separately.
  observed_version="${observed_version%-rhel}"
  [[ "${observed_version}" == "${expected_version}" ]] || {
    printf 'Observed Podman version does not match matrix declaration for %s: expected %s, observed %s\n' \
      "${id}" "${expected_version}" "${observed_version}" >&2
    return 1
  }
}

single_line_evidence() {
  tr '\t\r\n' '   ' < "$1" | sed 's/[[:space:]]*$//'
}

append_verified_evidence() {
  local id=$1 image=$2 declared_version=$3 distribution=$4 mode=$5 lane=$6 architecture=$7
  local evidence_directory=$8 transport=$9 resource_coverage=${10}
  local observed_version package_revision api_version observed_rootless observed_architecture
  local evidence_prefix="${evidence_directory}/"
  if [[ ! -f "${evidence_prefix}podman-version" ]]; then
    evidence_prefix="${evidence_directory}."
  fi
  observed_version="$(single_line_evidence "${evidence_prefix}podman-version")"
  package_revision="$(single_line_evidence "${evidence_prefix}package-version")"
  api_version="$(single_line_evidence "${evidence_prefix}api-version")"
  observed_rootless="$(single_line_evidence "${evidence_prefix}rootless")"
  observed_architecture="$(single_line_evidence "${evidence_prefix}architecture")"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "${id}" "${image}" "${declared_version}" "${observed_version}" "${package_revision}" \
    "${api_version}" "${distribution}" "${mode}" "${observed_rootless}" "${lane}" \
    "${architecture}" "${observed_architecture}" "${transport}" "${resource_coverage}" \
    >> "${artifact_root}/evidence.tsv"
}

run_convert() {
  local output=$1 socket=$2 selection_name=$3
  shift 3
  local output_directory="${current_case}/outputs/${selection_name}-${output}"
  local -a target_arguments=()
  if [[ "${output}" == podman ]]; then
    target_arguments+=(--podman-target-context rootful)
  fi
  mkdir -p -- "${current_case}/outputs"
  if ! boxferry_operation "BoxFerry ${selection_name} Podman-to-${output}" \
    convert podman "${output}" --podman-socket "${socket}" \
    --application-name "${current_prefix}" --loss-policy partial \
    --promote-podman-effective-named-volumes --promote-podman-effective-named-networks \
    --output-directory "${output_directory}" --console-format json "${target_arguments[@]}" "$@" \
    > "${output_directory}.report.json"; then
    mkdir -p -- "${current_case}/support-bundles"
    boxferry_operation "BoxFerry ${selection_name} support replay to ${output}" \
      validate podman "${output}" --podman-socket "${socket}" \
      --application-name "${current_prefix}" --loss-policy partial \
      --promote-podman-effective-named-volumes --promote-podman-effective-named-networks \
      --generate-error-report --include-podman-snapshot \
      --error-report-directory "${current_case}/support-bundles" --console-format json \
      "${target_arguments[@]}" "$@" > "${output_directory}.support-replay.json" 2> /dev/null || true
    return 1
  fi
  assert_successful_conversion "${output}" "${selection_name}" "${output_directory}" \
    "${output_directory}.report.json"
}

should_run_external_apply() {
  if [[ "${profile}" == limitation-revalidation ]]; then
    return 0
  fi
  case "$1" in
    podman-6.1-rootful | podman-6.1-rootless | podman-debian-11-rootful | podman-debian-11-rootless | \
      podman-debian-12-rootful | podman-debian-12-rootless | podman-ubuntu-22.04-rootful | \
      podman-ubuntu-22.04-rootless | podman-ubuntu-24.04-rootful | podman-ubuntu-24.04-rootless)
      return 0
      ;;
    *) return 1 ;;
  esac
}

start_apply_target() {
  if [[ -n "${apply_target_outer}" ]]; then
    engine_operation 'remove previous apply target' rm --force --ignore "${apply_target_outer}" > /dev/null
    apply_target_outer=""
    apply_target_socket=""
  fi

  local id image declared_version distribution mode lane architecture
  IFS=$'\t' read -r id image declared_version distribution mode lane architecture < <(
    awk -F '\t' '$1 == "podman-6.1-rootful" { print; exit }' "${matrix_path}"
  )
  [[ "${id}" == podman-6.1-rootful && "${declared_version}" == 6.1.0 && "${mode}" == rootful && "${lane}" == container ]] || {
    printf '%s\n' 'Reviewed Podman 6.1 rootful apply target is missing from the matrix.' >&2
    return 1
  }
  local source_outer="${started_outer}"
  local socket_directory="${runtime_root}/apply-target"
  local cleanup_role=""
  if [[ "${profile}" == limitation-revalidation ]]; then
    cleanup_role=apply-target
  fi
  start_outer_runtime "${id}-apply-target" "${image}" "${mode}" "${socket_directory}" \
    "${workload_archive}" "${cleanup_role}"
  apply_target_outer="${started_outer}"
  apply_target_socket="${socket_directory}/podman.sock"
  engine_operation 'prepare apply-target configuration directory' \
    exec "${apply_target_outer}" mkdir -p /etc/containers/containers.conf.d
  engine_operation 'copy apply-target network configuration' cp \
    "${repository_root}/fixtures/conformance/podman-live/apply-target-containers.conf" \
    "${apply_target_outer}:/etc/containers/containers.conf.d/99-boxferry-live.conf"
  engine_operation 'load digest-pinned apply-target workload image' \
    exec "${apply_target_outer}" podman load --input /boxferry-workload.tar > /dev/null
  engine_operation 'tag apply-target workload with configured portable reference' \
    exec "${apply_target_outer}" podman tag "${workload_local_tag}" \
    "registry.example.invalid/boxferry/${current_prefix}:1"
  verify_observed_version "${id}-apply-target" "${declared_version}" \
    "${artifact_root}/${id}-apply-target.podman-version"
  [[ "$(< "${artifact_root}/${id}-apply-target.architecture")" =~ ^(x86_64|amd64)$ ]]
  started_outer="${source_outer}"
}

run_external_apply_reacquire() {
  local source_socket=$1
  require_scenario external-apply-reacquire
  printf 'Live scenario: external-apply-reacquire\n'
  run_convert podman "${source_socket}" apply-source \
    --podman-resource "container=${current_prefix}-apply-web"
  run_convert compose "${source_socket}" apply-source \
    --podman-resource "container=${current_prefix}-apply-web"

  local source_plan="${current_case}/outputs/apply-source-podman"
  local expected_network="${current_prefix}-apply-net"
  local expected_volume="${current_prefix}-apply-data"
  local expected_container="${current_prefix}-apply-web"
  if ! jq --exit-status --arg network "${expected_network}" --arg volume "${expected_volume}" \
    --arg container "${expected_container}" '
      (.external_preconditions | length == 0) and
      ([.operations[] |
        select(.action == "create" and .resource.kind == "network" and .resource.name == $network)] |
        length == 1) and
      ([.operations[] |
        select(.action == "create" and .resource.kind == "volume" and .resource.name == $volume)] |
        length == 1) and
      ([.operations[] |
        select(.action == "create" and .resource.kind == "container" and .resource.name == $container)] |
        length == 1) and
      ([.operations[] |
        select(.action == "create") |
        [.resource.kind, .resource.name]] | sort) ==
      ([["network", $network], ["volume", $volume], ["container", $container]] | sort) and
      all(.operations[]; .cli.external_sensitive_input_required == false)
    ' "${source_plan}/podman.json" > /dev/null; then
    printf 'Generated apply plan did not promote its named network and volume exactly once.\n' >&2
    return 1
  fi

  start_apply_target
  timed_operation 3m 'execute generated plan inside apply target' \
    "${engine}" exec --interactive \
    "${apply_target_outer}" /bin/sh -seu \
    < "${source_plan}/podman-commands.sh"
  engine_operation 'inspect applied target container' \
    exec "${apply_target_outer}" podman inspect "${expected_container}" > /dev/null
  activate_outer_runtime "${runtime_root}/apply-target"

  run_convert podman "${apply_target_socket}" apply-target \
    --podman-resource "container=${expected_container}"
  run_convert compose "${apply_target_socket}" apply-target \
    --podman-resource "container=${expected_container}"
  cmp --silent "${current_case}/outputs/apply-source-podman/podman.json" \
    "${current_case}/outputs/apply-target-podman/podman.json"
  cmp --silent "${current_case}/outputs/apply-source-compose/compose.yaml" \
    "${current_case}/outputs/apply-target-compose/compose.yaml"

  engine_operation 'remove applied target container through API' \
    --url "unix://${apply_target_socket}" \
    rm --force --time 0 "${expected_container}" > /dev/null
  engine_operation 'remove applied target network through API' \
    --url "unix://${apply_target_socket}" network rm "${expected_network}" > /dev/null
  engine_operation 'remove applied target volume through API' \
    --url "unix://${apply_target_socket}" volume rm "${expected_volume}" > /dev/null
  local kind name
  for kind in container network volume; do
    case "${kind}" in
      container) name=${expected_container} ;;
      network) name=${expected_network} ;;
      volume) name=${expected_volume} ;;
    esac
    if ! expected_failure_operation 90s "verify applied target ${kind} cleanup" 1 \
      "${engine}" --url "unix://${apply_target_socket}" "${kind}" exists "${name}" \
      > /dev/null 2>&1; then
      printf 'Applied conformance %s survived exact cleanup: %s\n' "${kind}" "${name}" >&2
      return 1
    fi
  done
  engine_operation 'remove disposable applied target outer container' \
    rm --force --ignore -- "${apply_target_outer}" > /dev/null
  apply_target_outer=""
  apply_target_socket=""
}

run_invalid_glob() {
  local socket=$1 output="${current_case}/invalid-glob.stdout" error="${current_case}/invalid-glob.stderr"
  if ! expected_failure_operation 90s 'BoxFerry literal-glob rejection' 2 \
    "${boxferry_bin}" validate podman compose --podman-socket "${socket}" \
    --application-name "${current_prefix}" --podman-resource "container=${current_prefix}-small-*" \
    --loss-policy partial --console-format json > "${output}" 2> "${error}"; then
    printf '%s\n' 'Literal glob rejection did not return the expected command status.' >&2
    return 1
  fi
  [[ ! -s "${output}" ]] || {
    printf 'Invalid literal glob did not fail as a CLI usage error.\n' >&2
    return 1
  }
  grep --fixed-strings --quiet -- '--podman-resource requires a non-empty exact name or ID' "${error}"
}

assert_redacted_support_bundle() {
  local socket=$1
  local reports="${current_case}/support-bundle"
  local report="${current_case}/support-bundle.report.json"
  local outer_runtime_identity nested_runtime_identifiers private_path runtime_identifier
  local -a runtime_identifiers=()
  if [[ "${profile}" == limitation-revalidation ]]; then
    outer_runtime_identity="$(engine_operation 'read outer runtime identity for privacy review' \
      container inspect --format '{{.Id}}' "${started_outer}")"
    nested_runtime_identifiers="$(podman_socket "${socket}" \
      'read nested runtime identities for privacy review' \
      ps --all --no-trunc --format '{{.ID}}')"
    runtime_identifiers+=("${started_outer}" "${outer_runtime_identity}" "${current_selected_container_id}")
    while IFS= read -r runtime_identifier; do
      [[ -z "${runtime_identifier}" ]] || runtime_identifiers+=("${runtime_identifier}")
    done <<< "${nested_runtime_identifiers}"
    for runtime_identifier in "${runtime_identifiers[@]}"; do
      [[ -n "${runtime_identifier}" ]] || {
        printf '%s\n' 'Runtime privacy review received an empty identity.' >&2
        return 1
      }
    done
  fi
  mkdir -p -- "${reports}"
  boxferry_operation 'BoxFerry redacted support-bundle validation' \
    validate podman compose --podman-socket "${socket}" \
    --podman-label "io.boxferry.live-run=${current_prefix}" --loss-policy partial \
    --promote-podman-effective-named-volumes --promote-podman-effective-named-networks \
    --generate-error-report --include-podman-snapshot --error-report-directory "${reports}" \
    --console-format json > "${report}"
  jq --exit-status '
    .status == "success" and .exit_category == "success" and
    (.redaction.count > 0) and (.redaction.classes | index("podman-support-snapshot") != null)
  ' "${report}" > /dev/null
  if [[ "${profile}" == limitation-revalidation ]]; then
    mark_revalidation_result diagnostic_privacy.redaction_passed
  fi
  if grep --fixed-strings --quiet 'not-a-secret-test-value' "${report}"; then
    printf 'Standalone support report retained the protected environment canary.\n' >&2
    return 1
  fi
  local -a archives=("${reports}"/*.zip)
  [[ "${#archives[@]}" == 1 && -f "${archives[0]}" ]] || {
    printf 'Expected one Podman support archive in %s.\n' "${reports}" >&2
    return 1
  }
  local entries contents
  entries="$(unzip -Z1 "${archives[0]}")"
  for entry in README.md report.json podman-inventory-v1.json podman-discovery-graph-v1.json podman-acquisition-findings-v1.json; do
    grep --fixed-strings --line-regexp --quiet "${entry}" <<< "${entries}"
  done
  [[ "$(wc -l <<< "${entries}")" == 5 ]]
  if [[ "${profile}" == limitation-revalidation ]]; then
    mark_revalidation_result diagnostic_privacy.raw_outputs_absent
  fi
  contents="$(unzip -p "${archives[0]}")"
  if grep --fixed-strings --quiet 'not-a-secret-test-value' <<< "${contents}"; then
    printf 'Podman support archive retained the protected environment canary.\n' >&2
    return 1
  fi
  if [[ "${profile}" == limitation-revalidation ]]; then
    mark_revalidation_result diagnostic_privacy.environment_values_absent
  fi
  if [[ "${profile}" == limitation-revalidation ]]; then
    for private_path in "${socket}" "${runtime_root}" "${repository_root}" "${current_case}"; do
      [[ -n "${private_path}" ]] || {
        printf '%s\n' 'Host-path privacy review received an empty path.' >&2
        return 1
      }
      if grep --fixed-strings --quiet "${private_path}" "${report}" ||
        grep --fixed-strings --quiet "${private_path}" <<< "${contents}"; then
        printf 'Podman support material retained private host path: %s\n' "${private_path}" >&2
        return 1
      fi
    done
    if [[ "${profile}" == limitation-revalidation ]]; then
      mark_revalidation_result diagnostic_privacy.host_paths_absent
    fi
    for runtime_identifier in "${runtime_identifiers[@]}"; do
      if grep --fixed-strings --quiet "${runtime_identifier}" "${report}" ||
        grep --fixed-strings --quiet "${runtime_identifier}" <<< "${contents}"; then
        printf '%s\n' 'Podman support material retained a private runtime identity.' >&2
        return 1
      fi
    done
    if [[ "${profile}" == limitation-revalidation ]]; then
      mark_revalidation_result diagnostic_privacy.runtime_identifiers_absent
    fi
  else
    if grep --fixed-strings --quiet "${socket}" <<< "${contents}"; then
      printf 'Podman support archive retained its connection endpoint.\n' >&2
      return 1
    fi
  fi
  # The snapshot deliberately retains field names but serializes every protected
  # value as a state marker, never as a replacement string containing the value.
  grep --fixed-strings --quiet '"value_state": "redacted"' <<< "${contents}"
  grep --fixed-strings --quiet 'BOXFERRY_LIVE_MODE' <<< "${contents}"
}

assert_selinux_relabel_promotion() {
  local socket=$1
  local container="${current_prefix:?caller must supply current_prefix}-relabel"
  local shared_bind_source="/tmp/${container}-shared-source"
  local private_bind_source="/tmp/${container}-private-source"
  local root="${current_case:?caller must supply current_case}/relabel-promotion"
  local default_output="${root}/default"
  local promoted_output="${root}/promoted"
  local default_report="${root}/default.report.json"
  local promoted_report="${root}/promoted.report.json"
  local application_name="${current_prefix}-relabel"
  local image="registry.example.invalid/boxferry/${current_prefix}:1"

  mkdir -p -- "${root}"
  engine_operation 'create isolated relabel source directories' \
    exec "${started_outer}" mkdir -p -- "${shared_bind_source}" "${private_bind_source}"
  podman_socket "${socket}" 'create isolated relabel evidence' \
    create --name "${container}" --network "${current_prefix}-small-net" \
    --volume "${shared_bind_source}:/relabel-shared:ro,z" \
    --volume "${private_bind_source}:/relabel-private:ro,Z" \
    "${image}" sleep 3600 > /dev/null

  podman_socket "${socket}" 'inspect isolated relabel evidence' inspect "${container}" |
    jq --exit-status \
      --arg shared_source "${shared_bind_source}" \
      --arg private_source "${private_bind_source}" '
      any(.[0].HostConfig.Binds[]?;
        startswith($shared_source + ":/relabel-shared:")
        and ((split(":")[-1] | split(",") | index("z")) != null))
      and any(.[0].HostConfig.Binds[]?;
        startswith($private_source + ":/relabel-private:")
        and ((split(":")[-1] | split(",") | index("Z")) != null))
    ' > /dev/null

  boxferry_operation 'BoxFerry default relabel omission' \
    convert podman quadlet --podman-socket "${socket}" \
    --application-name "${application_name}" \
    --podman-resource "container=${container}" \
    --loss-policy partial --output-directory "${default_output}" \
    --console-format json > "${default_report}"
  assert_successful_conversion quadlet relabel-default "${default_output}" "${default_report}"
  if grep --recursive --fixed-strings --quiet -- "${shared_bind_source}" "${default_output}" ||
    grep --recursive --fixed-strings --quiet -- "${private_bind_source}" "${default_output}"; then
    printf 'Default Podman import unexpectedly promoted same-host bind path.\n' >&2
    return 1
  fi
  local mount_index
  for mount_index in 0 1; do
    jq --exit-status --arg subject "services.${container}.mounts[${mount_index}].selinux_relabel" '
      any(.diagnostics[]?;
        .code == "BFP0003"
        and any(.fields[]?; .name == "subject" and .value == $subject)
        and any(.fields[]?;
          .name == "available_promotion"
          and .value == "--promote-podman-effective-bind-mounts"))
    ' "${default_report}" > /dev/null
  done

  boxferry_operation 'BoxFerry explicit relabel promotion' \
    convert podman quadlet --podman-socket "${socket}" \
    --application-name "${application_name}" \
    --podman-resource "container=${container}" \
    --promote-podman-effective-bind-mounts \
    --loss-policy partial --output-directory "${promoted_output}" \
    --console-format json > "${promoted_report}"
  assert_successful_conversion quadlet relabel-promoted "${promoted_output}" "${promoted_report}"
  grep --recursive --extended-regexp --line-regexp --quiet \
    "Volume=${shared_bind_source}:/relabel-shared:(ro,z|z,ro)" "${promoted_output}"
  grep --recursive --extended-regexp --line-regexp --quiet \
    "Volume=${private_bind_source}:/relabel-private:(ro,Z|Z,ro)" "${promoted_output}"

  podman_socket "${socket}" 'remove isolated relabel evidence' \
    rm --force --ignore -- "${container}" > /dev/null
  engine_operation 'remove isolated relabel source directories' \
    exec "${started_outer}" rmdir -- "${shared_bind_source}" "${private_bind_source}"
}

podman_socket() {
  local socket=$1 action=${2:-command}
  shift 2
  if [[ -n "${started_outer}" && ! -S "${socket}" ]]; then
    engine_operation "nested Podman ${action} through matching container CLI" \
      exec "${started_outer}" podman "$@"
  else
    engine_operation "nested Podman ${action} through acquisition socket" \
      --url "unix://${socket}" "$@"
  fi
}

start_clean_acquisition_outer() {
  local id=$1 image=$2 mode=$3 socket_directory=$4 scope=$5
  engine_operation 'remove runtime-observation outer container' \
    rm --force --ignore -- "${started_outer}" > /dev/null
  rm -f -- "${socket_directory}/podman.sock" "${socket_directory}/bootstrap.log" \
    "${socket_directory}/runtime-evidence.tsv" "${socket_directory}/runtime-evidence.ready" \
    "${socket_directory}/runtime-canaries.log" "${socket_directory}/selected-container-id" \
    "${socket_directory}/smoke-baseline.json" "${socket_directory}/start-api" \
    "${socket_directory}/resource-setup.status" "${socket_directory}/resource-setup.status.tmp"
  start_outer_runtime "${id}" "${image}" "${mode}" "${socket_directory}"
  create_workloads "${started_outer}" "${current_prefix}" "${scope}" "${socket_directory}" false
  current_selected_container_id="$(< "${socket_directory}/selected-container-id")"
  [[ "${current_selected_container_id}" =~ ^[[:xdigit:]]{64}$ ]] || {
    printf 'Could not resolve selected container ID before acquisition started.\n' >&2
    return 1
  }
  activate_outer_runtime "${socket_directory}"
}

wait_for_fault_proxy() {
  local socket=$1
  for _ in {1..30}; do
    [[ -S "${socket}" ]] && return 0
    sleep 1
  done
  printf 'Timed out waiting for fault-proxy socket: %s\n' "${socket}" >&2
  return 1
}

run_fault_proxy_case() {
  local socket=$1 mode=$2 output report fault_socket pid container_id
  fault_socket="${runtime_root}/fault-${mode}.sock"
  report="${current_case}/fault-${mode}.report.json"
  container_id="${current_selected_container_id}"
  [[ -n "${container_id}" ]] || {
    printf 'Could not resolve selected container ID for fault proxy.\n' >&2
    return 1
  }
  rm -f -- "${fault_socket}"
  python3 "${repository_root}/fixtures/conformance/podman-live/fault_proxy.py" \
    --listen "${fault_socket}" --upstream "${socket}" \
    --container "${current_prefix}-small-web" --container-id "${container_id}" --mode "${mode}" \
    > "${current_case}/fault-${mode}.log" 2>&1 &
  pid=$!
  fault_proxy_pids+=("${pid}")
  fault_proxy_sockets+=("${fault_socket}")
  wait_for_fault_proxy "${fault_socket}"
  for output in compose quadlet podman; do
    local -a target_arguments=()
    if [[ "${output}" == podman ]]; then
      target_arguments+=(--podman-target-context rootful)
    fi
    if timeout --signal=TERM --kill-after=10s 60s "${boxferry_bin}" validate podman "${output}" \
      --podman-socket "${fault_socket}" \
      --application-name "${current_prefix}" --podman-resource "container=${current_prefix}-small-web" \
      --loss-policy partial --console-format json "${target_arguments[@]}" \
      > "${report}.${output}" 2> "${report}.${output}.stderr"; then
      printf 'Fault proxy %s unexpectedly allowed %s validation.\n' "${mode}" "${output}" >&2
      return 1
    fi
    [[ ! -s "${report}.${output}.stderr" ]] || {
      printf 'Fault proxy %s did not produce a structured report for %s.\n' "${mode}" "${output}" >&2
      return 1
    }
    jq --exit-status '
      .schema_version == 1 and (.status == "blocked" or .status == "failure") and
      any(.diagnostics[]?; .code == "BFP0001") and
      (.fix_first.code == "BFP0001")
    ' "${report}.${output}" > /dev/null
  done
  kill "${pid}" > /dev/null 2>&1 || true
  wait "${pid}" > /dev/null 2>&1 || true
  rm -f -- "${fault_socket}"
}

run_partial_section_failure() {
  local socket=$1 output selector report fault_socket pid container_id
  fault_socket="${runtime_root}/fault-section-500.sock"
  container_id="${current_selected_container_id}"
  rm -f -- "${fault_socket}"
  python3 "${repository_root}/fixtures/conformance/podman-live/fault_proxy.py" \
    --listen "${fault_socket}" --upstream "${socket}" \
    --container "${current_prefix}-small-web" --container-id "${container_id}" --mode section-500 \
    > "${current_case}/fault-section-500.log" 2>&1 &
  pid=$!
  fault_proxy_pids+=("${pid}")
  fault_proxy_sockets+=("${fault_socket}")
  wait_for_fault_proxy "${fault_socket}"
  for output in compose quadlet podman; do
    local -a target_arguments=()
    if [[ "${output}" == podman ]]; then
      target_arguments+=(--podman-target-context rootful)
    fi
    for selector in all label; do
      report="${current_case}/fault-section-500-${selector}.${output}.report.json"
      local -a selector_arguments=(--podman-all)
      if [[ "${selector}" == label ]]; then
        selector_arguments=(--podman-label "io.boxferry.live-run=${current_prefix}")
      fi
      if timeout --signal=TERM --kill-after=10s 60s "${boxferry_bin}" validate podman "${output}" \
        --podman-socket "${fault_socket}" --application-name "${current_prefix}" \
        --loss-policy partial --console-format json "${target_arguments[@]}" "${selector_arguments[@]}" \
        > "${report}" 2> "${report}.stderr"; then
        printf 'Partial volume inventory failure unexpectedly allowed %s %s validation.\n' \
          "${selector}" "${output}" >&2
        return 1
      fi
      [[ ! -s "${report}.stderr" ]]
      jq --exit-status '
        .schema_version == 1 and (.status == "blocked" or .status == "failure") and
        any(.diagnostics[]?; .code == "BFP0001") and (.fix_first.code == "BFP0001")
      ' "${report}" > /dev/null
    done
    run_convert "${output}" "${fault_socket}" section-independent \
      --podman-resource "container=${current_prefix}-stopped"
  done
  kill "${pid}" > /dev/null 2>&1 || true
  wait "${pid}" > /dev/null 2>&1 || true
  rm -f -- "${fault_socket}"
}

run_discovery() {
  local id=$1 image=$2 mode=$3
  local uid_runtime_directory="/run/user/0"
  local uid_socket_directory="${uid_runtime_directory}/podman"
  if [[ -e "${uid_socket_directory}" ]]; then
    printf 'Refusing discovery test because it would touch existing path: %s\n' "${uid_socket_directory}" >&2
    return 1
  fi
  if [[ ! -e "${uid_runtime_directory}" ]]; then
    mkdir --mode=0700 -- "${uid_runtime_directory}"
    discovery_parent_created=true
  elif [[ ! -d "${uid_runtime_directory}" || -L "${uid_runtime_directory}" ]]; then
    printf 'Refusing discovery test because runtime parent is unsafe: %s\n' "${uid_runtime_directory}" >&2
    return 1
  fi
  mkdir -p -- "${uid_socket_directory}"
  discovery_directories+=("${uid_socket_directory}")
  local outer
  start_outer "${id}-discovery" "${image}" "${mode}" "${uid_socket_directory}" "${current_prefix}-discovery"
  outer="${started_outer}"
  boxferry_operation 'BoxFerry conventional socket discovery conversion' \
    convert podman compose --application-name "${current_prefix}" --loss-policy partial \
    --promote-podman-effective-named-volumes --promote-podman-effective-named-networks \
    --podman-resource "container=${current_prefix}-discovery-small-web" \
    --output-directory "${current_case}/outputs/discovery-compose" --console-format json \
    > "${current_case}/outputs/discovery-compose.report.json"
  [[ -s "${current_case}/outputs/discovery-compose/compose.yaml" ]]
  engine_operation 'remove socket-discovery container' \
    rm --force --ignore -- "${outer}" > /dev/null
  rm -f -- "${uid_socket_directory}/podman.sock" "${uid_socket_directory}/bootstrap.log" \
    "${uid_socket_directory}/runtime-evidence.tsv" \
    "${uid_socket_directory}/runtime-evidence.ready" \
    "${uid_socket_directory}/runtime-canaries.log" \
    "${uid_socket_directory}/selected-container-id" \
    "${uid_socket_directory}/smoke-baseline.json" "${uid_socket_directory}/start-api" \
    "${uid_socket_directory}/resource-setup.status" \
    "${uid_socket_directory}/resource-setup.status.tmp"
  rmdir -- "${uid_socket_directory}"
  if [[ "${discovery_parent_created}" == true ]]; then
    rmdir -- "${uid_runtime_directory}"
    discovery_parent_created=false
  fi
}

is_smoke_diagnostics_cell() {
  [[ "$1" == podman-6.1-rootful ]]
}

should_run_discovery() {
  # Socket-path selection is host-side CLI behavior, independent of the nested Podman version.
  # One newest-version rootful cell proves the conventional socket against a real API service;
  # deterministic CLI tests cover rootless-first ordering and both finite candidates.
  [[ "$1" == podman-6.1-rootful ]]
}

remove_outer() {
  local outer=$1
  timeout --signal=TERM --kill-after=10s 30s \
    "${engine}" rm --force --ignore -- "${outer}" > /dev/null
}

verify_revalidation_candidate_provenance() {
  local image=$1 expected_source=$2 expected_revision=$3 expected_license=$4 expected_timestamp=$5
  local labels observed_source observed_revision observed_license observed_timestamp
  labels="$(engine_operation 'inspect replacement image provenance labels' \
    image inspect --format '{{json .Labels}}' "${image}")" || return 1
  observed_source="$(jq -er '.["org.opencontainers.image.source"]' <<< "${labels}")" || return 1
  observed_revision="$(jq -er '.["org.opencontainers.image.revision"]' <<< "${labels}")" || return 1
  observed_license="$(jq -er '.["org.opencontainers.image.licenses"]' <<< "${labels}")" || return 1
  observed_timestamp="$(jq -er '.["org.opencontainers.image.created"]' <<< "${labels}")" || return 1
  [[ "${observed_source}" == "${expected_source}" &&
    "${observed_revision}" == "${expected_revision}" &&
    "${observed_license}" == "${expected_license}" &&
    "${observed_timestamp}" == "${expected_timestamp}" ]] || {
    printf '%s\n' 'Replacement image provenance does not match the reviewed candidate catalogue.' >&2
    return 1
  }
  printf '%s\n' "${observed_revision}" > "${artifact_root}/${candidate_cell}.source-revision"
}

collect_revalidation_runtime_identity() {
  local outer=$1 evidence_file=$2
  # shellcheck disable=SC2016 # Runtime identity expansion occurs inside the candidate image.
  engine_operation 'collect replacement distribution identity' exec "${outer}" \
    /bin/sh -ceu \
    '. /etc/os-release; printf "%s\t%s\n" "${ID:?}" "${VERSION_ID:?}"' \
    > "${evidence_file}"
}

revalidation_observed_distribution() {
  local expected=$1 evidence_file=$2 os_id os_version
  IFS=$'\t' read -r os_id os_version < "${evidence_file}"
  case "${expected}" in
    opensuse-leap-16.0)
      [[ "${os_id}" == opensuse-leap && "${os_version}" == 16.0 ]] || return 1
      printf '%s\n' "opensuse-leap-${os_version}"
      ;;
    opensuse-tumbleweed)
      [[ "${os_id}" == opensuse-tumbleweed && "${os_version}" =~ ^[0-9]{8}$ ]] || return 1
      printf '%s\n' "opensuse-tumbleweed-${os_version}"
      ;;
    ubi-8)
      [[ "${os_id}" == rhel && "${os_version}" == 8.10 ]] || return 1
      printf '%s\n' "ubi-${os_version}"
      ;;
    ubi-9)
      [[ "${os_id}" == rhel && "${os_version}" == 9.8 ]] || return 1
      printf '%s\n' "ubi-${os_version}"
      ;;
    ubi-10)
      [[ "${os_id}" == rhel && "${os_version}" == 10.2 ]] || return 1
      printf '%s\n' "ubi-${os_version}"
      ;;
    *) return 1 ;;
  esac
}

record_revalidation_candidate_runtime_observations() {
  local id=$1 outer=$2 observed_version observed_distribution
  collect_revalidation_runtime_identity "${outer}" "${artifact_root}/${id}.distribution"
  observed_version="$(awk '{ print $3 }' "${artifact_root}/${id}.podman-version")"
  observed_version="${observed_version%-rhel}"
  observed_distribution="$(revalidation_observed_distribution \
    "${candidate_distribution}" "${artifact_root}/${id}.distribution")" || return 1
  record_revalidation_observation replacement.observed.podman_version "${observed_version}"
  record_revalidation_observation replacement.observed.api_version \
    "$(< "${artifact_root}/${id}.api-version")"
  record_revalidation_observation replacement.observed.package_revision \
    "$(< "${artifact_root}/${id}.package-version")"
  record_revalidation_observation replacement.observed.distribution "${observed_distribution}"
  record_revalidation_observation replacement.observed.architecture \
    "$(< "${artifact_root}/${id}.architecture")"
  record_revalidation_observation replacement.observed.rootless \
    "$(< "${artifact_root}/${id}.rootless")"
}

assert_revalidation_fresh_store() {
  local outer=$1 socket_namespace=$2 name_prefix=$3
  local runtime_identity graph_root_identity
  revalidation_phase="replacement-runtime"
  revalidation_failure_code="runtime-metadata-mismatch"
  runtime_identity="$(engine_operation 'read candidate outer runtime identity' \
    container inspect --format '{{.Id}}' "${outer}")"
  graph_root_identity="$(engine_operation 'read candidate outer graph root identity' \
    container inspect --format '{{.GraphDriver.Data.MergedDir}}' "${outer}")"
  [[ -n "${runtime_identity}" && -n "${socket_namespace}" &&
    -n "${graph_root_identity}" && -n "${name_prefix}" ]] || {
    printf '%s\n' 'Candidate fresh-store proof contained an empty private identity.' >&2
    return 1
  }
  [[ "${runtime_identity}" != "${revalidation_baseline_runtime_identity}" ]] || {
    printf '%s\n' 'Candidate reused the historical outer runtime identity.' >&2
    return 1
  }
  mark_revalidation_result fresh_store.distinct_runtime
  [[ "${socket_namespace}" != "${revalidation_baseline_socket_namespace}" ]] || {
    printf '%s\n' 'Candidate reused the historical socket namespace.' >&2
    return 1
  }
  mark_revalidation_result fresh_store.distinct_socket
  [[ "${graph_root_identity}" != "${revalidation_baseline_graph_root_identity}" ]] || {
    printf '%s\n' 'Candidate reused the historical graph-root identity.' >&2
    return 1
  }
  mark_revalidation_result fresh_store.distinct_graph_root
  [[ "${name_prefix}" != "${revalidation_baseline_name_prefix}" ]] || {
    printf '%s\n' 'Candidate reused the historical application-name prefix.' >&2
    return 1
  }
  mark_revalidation_result fresh_store.distinct_name_prefix
}

configure_cell_progress() {
  local id=$1
  progress_index=0
  if [[ "${profile}" == limitation-revalidation ]]; then
    progress_total=${#revalidation_required_checks[@]}
  elif [[ "${profile}" == smoke ]]; then
    progress_total=10
    if is_smoke_diagnostics_cell "${id}"; then
      progress_total=15
    fi
  else
    progress_total=30
    if should_run_external_apply "${id}"; then
      progress_total=31
    fi
  fi
  if [[ "${profile}" != limitation-revalidation ]] && is_smoke_diagnostics_cell "${id}"; then
    ((progress_total += 1))
  fi
  if [[ "${profile}" != limitation-revalidation ]] && should_run_discovery "${id}"; then
    ((progress_total += 1))
  fi
  printf '%s PLAN cell=%s profile=%s tests=%d\n' \
    "$(timestamp)" "${id}" "${profile}" "${progress_total}"
}

run_cell() {
  local id=$1 image=$2 declared_version=$3 distribution=$4 mode=$5 lane=$6 architecture=$7
  if [[ "${profile}" == application ]]; then
    run_nextcloud_application_cell "$@"
    return
  fi
  if [[ "${profile}" == forgejo-application ]]; then
    run_forgejo_application_cell "$@"
    return
  fi
  if [[ "${profile}" == paperless-application ]]; then
    run_paperless_application_cell "$@"
    return
  fi
  if [[ "${profile}" == immich-application ]]; then
    run_immich_application_cell "$@"
    return
  fi
  if [[ "${profile}" == observability-application ]]; then
    run_observability_application_cell "$@"
    return
  fi
  if [[ "${profile}" == supabase-application ]]; then
    run_supabase_application_cell "$@"
    return
  fi

  current_case="${artifact_root}/${id}"
  if [[ "${profile}" == limitation-revalidation ]]; then
    current_case="${artifact_root}/revalidation-candidate/${id}"
  fi
  current_prefix="${run_id}-${id}"
  # Keep generated container names valid as single DNS-label network aliases.
  current_prefix="${current_prefix:0:48}"
  if [[ "${current_prefix}" == *[-.] ]]; then
    current_prefix="${current_prefix%?}x"
  fi
  mkdir -p -- "${current_case}"
  local socket_directory="${runtime_root}/${id}"
  local cleanup_role=""
  if [[ "${profile}" == limitation-revalidation ]]; then
    socket_directory="${runtime_root}/revalidation-candidate/${id}"
    cleanup_role=replacement
  fi
  local outer socket coverage_level workload_scope=full runtime_test_name
  if [[ "${profile}" == smoke ]]; then
    workload_scope=minimal
  fi
  configure_cell_progress "${id}"
  printf '%s CELL START %s (%s, %s profile)\n' "$(timestamp)" "${id}" "${mode}" "${profile}"
  progress_run 'prepare digest-pinned workload archive' prepare_workload_archive
  progress_run 'pull and verify reviewed Podman image' prepare_matrix_image "${id}" "${image}"
  progress_run 'start isolated Podman container and collect runtime evidence' \
    start_outer_runtime "${id}" "${image}" "${mode}" "${socket_directory}" \
    "${workload_archive}" "${cleanup_role}"
  if [[ "${profile}" == limitation-revalidation ]]; then
    assert_revalidation_fresh_store \
      "${revalidation_candidate_outer}" "${socket_directory}" "${current_prefix}"
  fi
  progress_run "create ${workload_scope} resources before acquisition starts" \
    create_workloads "${started_outer}" "${current_prefix}" "${workload_scope}" "${socket_directory}"
  outer="${started_outer}"
  progress_begin 'verify image, version, architecture, and runtime evidence'
  verify_observed_version "${id}" "${declared_version}" "${artifact_root}/${id}.podman-version"
  [[ "$(< "${artifact_root}/${id}.digest")" == "${image##*@}" ]] || {
    printf 'Pulled image digest does not match reviewed matrix reference for %s.\n' "${id}" >&2
    return 1
  }
  [[ "${architecture}" == amd64 && "$(< "${artifact_root}/${id}.architecture")" =~ ^(x86_64|amd64)$ ]] || {
    printf 'Observed architecture does not match matrix declaration for %s.\n' "${id}" >&2
    return 1
  }
  coverage_level=full
  if [[ "${profile}" == smoke ]]; then
    coverage_level=smoke
  fi
  append_verified_evidence "${id}" "${image}" "${declared_version}" "${distribution}" "${mode}" \
    "${lane}" "${architecture}" "${artifact_root}/${id}" nested-image "${coverage_level}"
  progress_pass
  socket="${socket_directory}/podman.sock"
  # shellcheck disable=SC2034 # Consumed by sourced scenario validators.
  current_podman_major="$(awk '{ split($3, version, "."); print version[1] }' \
    "${artifact_root}/${id}.podman-version")"
  # shellcheck disable=SC2034 # Consumed by sourced scenario validators.
  current_podman_rootless="$(< "${artifact_root}/${id}.rootless")"
  runtime_test_name='verify live runtime semantics (9 scenario groups) and start acquisition socket'
  if [[ "${workload_scope}" == minimal ]]; then
    runtime_test_name='verify runtime baseline and start acquisition socket'
  fi
  progress_begin "${runtime_test_name}"
  if [[ "${workload_scope}" == minimal ]]; then
    assert_smoke_runtime_baseline
  else
    assert_runtime_scenarios "${socket}" "$(awk '{print $3}' "${artifact_root}/${id}.podman-version")"
  fi
  start_clean_acquisition_outer "${id}" "${image}" "${mode}" \
    "${socket_directory}" "${workload_scope}"
  outer="${started_outer}"
  progress_pass

  local selection output
  local -a selections=(exact)
  if is_complete_resource_profile; then
    selections=(exact prefix label all network-boundary)
  fi
  for selection in "${selections[@]}"; do
    require_scenario "$(selection_scenario "${selection}")"
    for output in compose quadlet podman; do
      case "${selection}" in
        exact)
          progress_run "convert exact container to ${output}" run_convert "${output}" "${socket}" \
            "${selection}" --podman-resource "container=${current_prefix}-small-web"
          ;;
        prefix)
          progress_run "convert prefix selection to ${output}" run_convert "${output}" "${socket}" \
            "${selection}" --podman-resource-prefix "container=${current_prefix}-large-"
          ;;
        label)
          progress_run "convert label selection to ${output}" run_convert "${output}" "${socket}" \
            "${selection}" --podman-label "io.boxferry.live-run=${current_prefix}"
          ;;
        all)
          progress_run "convert all resources to ${output}" run_convert "${output}" "${socket}" \
            "${selection}" --podman-all
          ;;
        network-boundary)
          progress_run "convert network boundary to ${output}" run_convert "${output}" "${socket}" \
            "${selection}" --podman-resource "network=${current_prefix}-large-private"
          ;;
      esac
    done
  done
  if is_complete_resource_profile || is_smoke_diagnostics_cell "${id}"; then
    require_scenario invalid-literal-glob
    progress_run 'reject literal glob selector' run_invalid_glob "${socket}"
    require_scenario deterministic-exact-compose
    progress_run 'verify deterministic Compose export' assert_deterministic_exact_compose "${socket}"
    require_scenario strict-policy-blocks
    progress_run 'block lossy import under strict policy' assert_strict_policy_blocks "${socket}"
    require_scenario protected-redaction-support-bundle
    progress_run 'write redacted support bundle' assert_redacted_support_bundle "${socket}"
    require_scenario malformed-selected-container
    progress_run 'diagnose malformed selected container' run_fault_proxy_case "${socket}" malformed
  fi
  if is_complete_resource_profile; then
    require_scenario disappeared-selected-container
    progress_run 'diagnose disappeared selected container' run_fault_proxy_case "${socket}" gone
    require_scenario partial-inventory-section
    progress_run 'diagnose partial inventory section' run_partial_section_failure "${socket}"
    progress_run 're-import generated Compose and Quadlet outputs' run_reimports "${declared_version%%+*}"
    if should_run_external_apply "${id}"; then
      progress_run 'externally apply and reacquire Podman plan' \
        run_external_apply_reacquire "${socket}"
    fi
  fi
  if [[ "${profile}" == limitation-revalidation ]] || is_smoke_diagnostics_cell "${id}"; then
    require_scenario selinux-relabel-promotion
    progress_run 'verify SELinux relabel omission and promotion' \
      assert_selinux_relabel_promotion "${socket}"
  fi

  if should_run_discovery "${id}"; then
    require_scenario socket-discovery
    progress_run 'discover local Podman socket' run_discovery "${id}" "${image}" "${mode}"
  fi
  progress_run 'remove disposable outer container' remove_outer "${outer}"
  release_run_owned_matrix_image "${image}"
  printf '%s CELL PASS  %s (%d/%d tests)\n' \
    "$(timestamp)" "${id}" "${progress_index}" "${progress_total}"
}

run_limited_cell() {
  local id=$1 image=$2 declared_version=$3 distribution=$4 mode=$5 lane=$6 architecture=$7
  local reason
  reason="$(awk -F '\t' -v expected="${id}" '$1 == expected { print $2 }' "${limitation_path}")"
  [[ "${mode}" == rootless && "${reason}" == helper-privilege-collision ]] || {
    printf 'Unsupported container limitation for %s: mode=%s reason=%s\n' "${id}" "${mode}" "${reason}" >&2
    return 2
  }
  current_case="${artifact_root}/${id}"
  mkdir -p -- "${current_case}"
  progress_index=0
  progress_total=4
  printf '%s PLAN cell=%s profile=%s tests=%d limitation=%s\n' \
    "$(timestamp)" "${id}" "${profile}" "${progress_total}" "${reason}"
  printf '%s CELL START %s (%s, reviewed limitation)\n' "$(timestamp)" "${id}" "${mode}"
  progress_run 'pull and verify reviewed Podman image' prepare_matrix_image "${id}" "${image}"
  local outer_digest outer
  outer_digest="$(printf '%s' "limited-${id}" | sha256sum)"
  outer="${run_id:0:36}-${outer_digest:0:16}"
  progress_begin 'start limitation-probe container'
  outer_containers+=("${outer}")
  startup_substep 'create limitation-probe container (deadline 90s)' \
    timeout --signal=TERM --kill-after=10s 90s \
    "${engine}" run --detach --rm --name "${outer}" --stop-timeout 1 --privileged --device /dev/fuse \
    --security-opt label=disable "${image}" /bin/sh -ceu \
    'trap "exit 0" INT TERM; while :; do sleep 3600; done' \
    > "${current_case}/outer.id"
  progress_pass
  progress_begin 'verify helper privilege collision evidence'
  [[ "$(engine_operation 'read limited-cell UID' exec "${outer}" id -u)" == 1000 ]] || {
    printf 'Limited rootless cell did not start as UID 1000: %s\n' "${id}" >&2
    return 1
  }
  engine_operation 'read limited-cell Podman version' \
    exec "${outer}" podman --version > "${current_case}/podman-version"
  engine_operation 'read limited-cell architecture' \
    exec "${outer}" uname -m > "${current_case}/architecture"
  # shellcheck disable=SC2016 # Package queries and command substitution execute inside the image.
  engine_operation 'read limited-cell package version' exec "${outer}" sh -ceu '
    if test -s /usr/share/strukturpiloten/podman-package-version; then
      cat /usr/share/strukturpiloten/podman-package-version
    else
      podman_version="$(podman --version)"
      printf "upstream-source-build:%s\n" "${podman_version#podman version }"
    fi
  ' > "${current_case}/package-version"
  if timeout --signal=TERM --kill-after=10s 30s "${engine}" exec "${outer}" podman info \
    > "${current_case}/podman-info.stdout" 2> "${current_case}/podman-info.stderr"; then
    printf 'Matrix limitation for %s is stale: nested rootless Podman now initializes successfully.\n' "${id}" >&2
    return 1
  fi
  if ! grep --quiet newuidmap "${current_case}/podman-info.stderr" ||
    ! grep --quiet 'Permission denied' "${current_case}/podman-info.stderr"; then
    printf 'Limited cell %s no longer fails with the reviewed helper collision.\n' "${id}" >&2
    return 1
  fi
  local mounted_image_root uid_helper gid_helper uid_capability gid_capability
  mounted_image_root="$(engine_operation 'mount limited-cell image for helper review' \
    image mount "${image}")"
  mounted_images+=("${image}")
  uid_helper="${mounted_image_root}/usr/bin/newuidmap"
  gid_helper="${mounted_image_root}/usr/bin/newgidmap"
  [[ -u "${uid_helper}" && -u "${gid_helper}" ]] || {
    printf 'Limited cell %s no longer has the reviewed setuid helper modes.\n' "${id}" >&2
    return 1
  }
  uid_capability="$(getcap "${uid_helper}")"
  gid_capability="$(getcap "${gid_helper}")"
  [[ "${uid_capability}" == *cap_setuid=ep* && "${gid_capability}" == *cap_setgid=ep* ]] || {
    printf 'Limited cell %s no longer has the reviewed helper file capabilities.\n' "${id}" >&2
    return 1
  }
  {
    ls -ln "${uid_helper}" "${gid_helper}"
    printf '%s\n%s\n' "${uid_capability}" "${gid_capability}"
  } > "${current_case}/helper-privileges"
  engine_operation 'unmount limited-cell image' image unmount -- "${image}" > /dev/null
  printf '%s\n' unavailable > "${current_case}/api-version"
  printf '%s\n' unverified-runtime-unavailable > "${current_case}/rootless"
  printf '%s\n' "${reason}" > "${current_case}/resource-limitation"
  verify_observed_version "${id}" "${declared_version}" "${current_case}/podman-version"
  [[ "${architecture}" == amd64 && "$(< "${current_case}/architecture")" =~ ^(x86_64|amd64)$ ]] || {
    printf 'Limited container architecture does not match matrix declaration for %s.\n' "${id}" >&2
    return 1
  }
  append_verified_evidence "${id}" "${image}" "${declared_version}" "${distribution}" "${mode}" \
    "${lane}" "${architecture}" "${current_case}" container-cli "${reason}"
  progress_pass
  progress_run 'remove disposable limitation container' remove_outer "${outer}"
  release_run_owned_matrix_image "${image}"
  printf '%s CELL PASS  %s (%d/%d tests, reviewed limitation)\n' \
    "$(timestamp)" "${id}" "${progress_index}" "${progress_total}"
}

run_revalidation_baseline_collision() {
  local id=$1 image=$2
  local baseline_case="${artifact_root}/revalidation-baseline/${id}"
  local baseline_socket_namespace="${runtime_root}/revalidation-baseline/${id}"
  local expected_digest="${image##*@}"
  local outer_digest outer status mounted_image_root uid_helper gid_helper
  local uid_capability gid_capability observed_version observed_distribution
  mkdir -p -- "${baseline_case}"
  mkdir -p -- "${baseline_socket_namespace}"
  chmod 0777 "${baseline_socket_namespace}"
  revalidation_phase="baseline-pull"
  revalidation_failure_code="pull-failed"
  prepare_matrix_image "${id}" "${image}" true
  [[ "$(< "${artifact_root}/${id}.digest")" == "${expected_digest}" ]]
  record_revalidation_observation baseline.observed_digest "${image##*@sha256:}"

  revalidation_phase="baseline-metadata"
  revalidation_failure_code="baseline-metadata-mismatch"
  outer_digest="$(printf '%s' "revalidation-baseline-${id}" | sha256sum)"
  outer="${run_id:0:32}-baseline-${outer_digest:0:12}"
  outer_containers+=("${outer}")
  timed_operation 90s 'start historical limitation container' \
    "${engine}" run --detach --rm --name "${outer}" --stop-timeout 1 \
    --privileged --device /dev/fuse --security-opt label=disable \
    --volume "${baseline_socket_namespace}:/boxferry-socket:Z" \
    "${image}" /bin/sh -ceu 'trap "exit 0" INT TERM; sleep 3600' \
    > "${baseline_case}/outer.id"
  revalidation_baseline_outer="${outer}"
  revalidation_baseline_runtime_identity="$(engine_operation \
    'read historical outer runtime identity' container inspect --format '{{.Id}}' "${outer}")"
  revalidation_baseline_socket_namespace="${baseline_socket_namespace}"
  revalidation_baseline_graph_root_identity="$(engine_operation \
    'read historical outer graph root identity' container inspect \
    --format '{{.GraphDriver.Data.MergedDir}}' "${outer}")"
  revalidation_baseline_name_prefix="${run_id:0:32}-baseline-app-${outer_digest:0:8}"
  [[ -n "${revalidation_baseline_runtime_identity}" &&
    -n "${revalidation_baseline_socket_namespace}" &&
    -n "${revalidation_baseline_graph_root_identity}" &&
    -n "${revalidation_baseline_name_prefix}" ]] || {
    printf '%s\n' 'Historical fresh-store reference contained an empty private identity.' >&2
    return 1
  }
  engine_operation 'read historical limitation UID' exec "${outer}" id -u \
    > "${baseline_case}/uid"
  [[ "$(< "${baseline_case}/uid")" == 1000 ]] || {
    printf 'Historical limitation image %s no longer runs as UID 1000.\n' "${id}" >&2
    return 1
  }
  engine_operation 'read historical limitation Podman version' exec "${outer}" podman --version \
    > "${baseline_case}/podman-version"
  engine_operation 'read historical limitation architecture' exec "${outer}" uname -m \
    > "${baseline_case}/architecture"
  # shellcheck disable=SC2016 # Package query expansion occurs inside the baseline image.
  engine_operation 'read historical limitation package version' exec "${outer}" sh -ceu '
    if test -s /usr/share/strukturpiloten/podman-package-version; then
      cat /usr/share/strukturpiloten/podman-package-version
    else
      podman_version="$(podman --version)"
      printf "upstream-source-build:%s\n" "${podman_version#podman version }"
    fi
  ' > "${baseline_case}/package-version"
  collect_revalidation_runtime_identity "${outer}" "${baseline_case}/distribution"
  verify_observed_version "${id}-baseline" "${candidate_version}" \
    "${baseline_case}/podman-version"
  [[ "$(< "${baseline_case}/architecture")" =~ ^(x86_64|amd64)$ ]] || {
    printf 'Historical limitation image %s has unexpected architecture.\n' "${id}" >&2
    return 1
  }
  observed_distribution="$(revalidation_observed_distribution \
    "${candidate_distribution}" "${baseline_case}/distribution")" || return 1
  [[ "${observed_distribution}" == "${candidate_baseline_observed_distribution}" ]] || {
    printf 'Historical limitation image %s distribution mismatch: expected %s, observed %s.\n' \
      "${id}" "${candidate_baseline_observed_distribution}" "${observed_distribution}" >&2
    return 1
  }
  observed_version="$(awk '{ print $3 }' "${baseline_case}/podman-version")"
  observed_version="${observed_version%-rhel}"
  record_revalidation_observation baseline.observed.podman_version "${observed_version}"
  record_revalidation_observation baseline.observed.package_revision \
    "$(< "${baseline_case}/package-version")"
  record_revalidation_observation baseline.observed.distribution "${observed_distribution}"
  record_revalidation_observation baseline.observed.architecture \
    "$(< "${baseline_case}/architecture")"
  record_revalidation_observation baseline.observed.uid "$(< "${baseline_case}/uid")"
  record_revalidation_observation baseline.observed.rootless true

  revalidation_phase="baseline-collision"
  revalidation_failure_code="historical-collision-not-reproduced"
  if timeout --signal=TERM --kill-after=10s 90s \
    "${engine}" exec "${outer}" podman info \
    > "${baseline_case}/podman-info.stdout" 2> "${baseline_case}/podman-info.stderr"; then
    printf 'Historical limitation %s is stale: rootless Podman initialized.\n' "${id}" >&2
    return 1
  else
    status=$?
  fi
  if [[ "${status}" == 124 || "${status}" == 137 ]]; then
    printf 'Historical limitation %s timed out instead of reproducing the collision.\n' "${id}" >&2
    return 1
  fi
  mark_revalidation_result baseline.historical_collision.podman_info_failed
  grep --fixed-strings --quiet newuidmap "${baseline_case}/podman-info.stderr"
  mark_revalidation_result baseline.historical_collision.newuidmap_reported
  grep --fixed-strings --quiet 'Permission denied' "${baseline_case}/podman-info.stderr"
  mark_revalidation_result baseline.historical_collision.permission_denied_reported

  mounted_image_root="$(engine_operation 'mount historical image for helper review' image mount "${image}")"
  mounted_images+=("${image}")
  revalidation_mounted_image_active="${image}"
  revalidation_mounted_image_root="${mounted_image_root}"
  uid_helper="${mounted_image_root}/usr/bin/newuidmap"
  gid_helper="${mounted_image_root}/usr/bin/newgidmap"
  [[ -u "${uid_helper}" ]]
  mark_revalidation_result baseline.historical_collision.newuidmap_setuid
  [[ -u "${gid_helper}" ]]
  mark_revalidation_result baseline.historical_collision.newgidmap_setuid
  uid_capability="$(getcap "${uid_helper}")"
  gid_capability="$(getcap "${gid_helper}")"
  [[ "${uid_capability}" == *cap_setuid=ep* ]]
  mark_revalidation_result baseline.historical_collision.newuidmap_capability
  [[ "${gid_capability}" == *cap_setgid=ep* ]]
  mark_revalidation_result baseline.historical_collision.newgidmap_capability
  engine_operation 'unmount historical image helper review' image unmount -- "${image}" > /dev/null
  revalidation_mounted_image_active=""
  revalidation_mounted_image_root=""

  revalidation_phase="baseline-cleanup"
  revalidation_failure_code="cleanup-failed"
  remove_outer "${outer}"
  if "${engine}" container exists "${outer}"; then
    printf 'Historical limitation container survived cleanup: %s\n' "${id}" >&2
    return 1
  fi
}

configure_revalidation_required_checks() {
  revalidation_required_checks=(
    'prepare digest-pinned workload archive'
    'pull and verify reviewed Podman image'
    'start isolated Podman container and collect runtime evidence'
    'create full resources before acquisition starts'
    'verify image, version, architecture, and runtime evidence'
    'verify live runtime semantics (9 scenario groups) and start acquisition socket'
    'reject literal glob selector'
    'verify deterministic Compose export'
    'block lossy import under strict policy'
    'write redacted support bundle'
    'diagnose malformed selected container'
    'diagnose disappeared selected container'
    'diagnose partial inventory section'
    're-import generated Compose and Quadlet outputs'
    'externally apply and reacquire Podman plan'
    'verify SELinux relabel omission and promotion'
    'remove disposable outer container'
  )
  local selection exporter
  for selection in 'exact container' 'prefix selection' 'label selection' 'all resources' 'network boundary'; do
    for exporter in compose quadlet podman; do
      revalidation_required_checks+=("convert ${selection} to ${exporter}")
    done
  done
}

assert_revalidation_required_checks() {
  local expected actual
  declare -A expected_checks=()
  for expected in "${revalidation_required_checks[@]}"; do
    if [[ -n "${expected_checks[${expected}]:-}" ]]; then
      printf 'Duplicate required limitation-revalidation check: %s\n' "${expected}" >&2
      return 2
    fi
    expected_checks["${expected}"]=true
    [[ "${revalidation_checks[${expected}]:-}" == true ]] || {
      printf 'Missing mandatory limitation-revalidation check: %s\n' "${expected}" >&2
      return 1
    }
  done
  for actual in "${!revalidation_checks[@]}"; do
    [[ "${expected_checks[${actual}]:-}" == true ]] || {
      printf 'Unexpected limitation-revalidation check: %s\n' "${actual}" >&2
      return 1
    }
  done
  [[ "${#revalidation_checks[@]}" == "${#revalidation_required_checks[@]}" ]]
  [[ "${progress_index}" == "${progress_total}" ]]
  [[ "${progress_total}" == "${#revalidation_required_checks[@]}" ]]
}

run_limitation_revalidation() {
  local observed_version observed_distribution
  configure_revalidation_required_checks
  run_revalidation_baseline_collision "${candidate_cell}" "${candidate_baseline_image}"

  revalidation_phase="replacement-pull"
  revalidation_failure_code="pull-failed"
  prepare_matrix_image "${candidate_cell}" "${candidate_replacement_image}" true
  record_revalidation_observation replacement.observed_digest \
    "${candidate_replacement_image##*@sha256:}"
  revalidation_phase="replacement-provenance"
  revalidation_failure_code="source-proof-mismatch"
  verify_revalidation_candidate_provenance \
    "${candidate_replacement_image}" "${candidate_source_repository}" \
    "${candidate_source_revision}" "${candidate_source_license}" "${candidate_published_at}"
  record_revalidation_observation replacement.observed.source_revision \
    "${candidate_source_revision}"

  revalidation_checks=()
  revalidation_capture_checks=true
  revalidation_phase="replacement-runtime"
  revalidation_failure_code="runtime-metadata-mismatch"
  run_cell "${candidate_cell}" "${candidate_replacement_image}" "${candidate_version}" \
    "${candidate_distribution}" "${candidate_mode}" "${candidate_lane}" "${candidate_architecture}"
  revalidation_capture_checks=false
  revalidation_phase="cleanup"
  revalidation_failure_code="cleanup-failed"
  [[ -n "${revalidation_candidate_outer}" ]] || {
    printf '%s\n' 'Candidate cleanup review lost its private outer runtime identity.' >&2
    return 1
  }
  if "${engine}" container exists "${revalidation_candidate_outer}"; then
    printf '%s\n' 'Candidate outer runtime survived limitation-revalidation cleanup.' >&2
    return 1
  fi

  revalidation_phase="replacement-runtime"
  revalidation_failure_code="runtime-metadata-mismatch"
  observed_version="$(awk '{ print $3 }' "${artifact_root}/${candidate_cell}.podman-version")"
  observed_version="${observed_version%-rhel}"
  observed_distribution="$(revalidation_observed_distribution \
    "${candidate_distribution}" "${artifact_root}/${candidate_cell}.distribution")" || return 1
  [[ "${observed_distribution}" == "${candidate_replacement_observed_distribution}" ]] || {
    printf 'Replacement image %s distribution mismatch: expected %s, observed %s.\n' \
      "${candidate_cell}" "${candidate_replacement_observed_distribution}" \
      "${observed_distribution}" >&2
    return 1
  }
  [[ "${observed_version}" == "${candidate_version}" ]]
  [[ "$(< "${artifact_root}/${candidate_cell}.rootless")" == true ]]
  [[ "$(< "${artifact_root}/${candidate_cell}.source-revision")" == "${candidate_source_revision}" ]]
  assert_revalidation_required_checks

  revalidation_phase="cleanup"
  revalidation_failure_code="cleanup-failed"
  if [[ -n "${apply_target_outer}" ]]; then
    remove_outer "${apply_target_outer}"
    if "${engine}" container exists "${apply_target_outer}"; then
      printf '%s\n' 'External-apply target survived limitation-revalidation cleanup.' >&2
      return 1
    fi
  fi

  revalidation_ready_to_finalize=true
}

if [[ "${profile}" == limitation-revalidation ]]; then
  run_limitation_revalidation
  exit 0
fi

printf 'id\treviewed_image\tdeclared_podman_version\tobserved_podman_version\tpackage_revision\tapi_version\tdistribution\tdeclared_mode\tobserved_rootless\tlane\tdeclared_architecture\tobserved_architecture\ttransport\tresource_coverage\n' \
  > "${artifact_root}/evidence.tsv"
cells=0
limited_cells=0
while IFS=$'\t' read -r id image declared_version distribution mode lane architecture; do
  [[ -z "${id}" || "${id}" == \#* ]] && continue
  if selected "${id}" "${lane}"; then
    if limited_cell "${id}"; then
      run_limited_cell "${id}" "${image}" "${declared_version}" "${distribution}" "${mode}" "${lane}" "${architecture}"
      limited_cells=$((limited_cells + 1))
    else
      run_cell "${id}" "${image}" "${declared_version}" "${distribution}" "${mode}" "${lane}" "${architecture}"
    fi
    cells=$((cells + 1))
  fi
done < "${matrix_path}"

if ((cells == 0)); then
  printf 'No matrix cells match profile %s.\n' "${profile}" >&2
  exit 1
fi
suite_elapsed=$(($(date +%s) - suite_started_at))
printf '%s SUITE PASS profile=%s cells=%d limitations=%d duration=%s\n' \
  "$(timestamp)" "${profile}" "${cells}" "${limited_cells}" "$(format_duration "${suite_elapsed}")"
