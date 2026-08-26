#!/usr/bin/env bash
# Run BoxFerry against isolated, nested Podman instances from the reviewed matrix.
# This is intentionally opt-in: it pulls trusted images and starts privileged containers.

set -Eeuo pipefail
exec 3>&2

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repository_root="$(cd -- "${script_directory}/.." && pwd -P)"
readonly script_directory repository_root

profile=""
matrix_cell=""
matrix_start_at=""
matrix_start_reached=false
engine="podman"
retain_artifacts=false
matrix_path="${repository_root}/fixtures/conformance/podman-live/matrix.tsv"
scenario_path="${repository_root}/fixtures/conformance/podman-live/scenarios.tsv"
limitation_path="${repository_root}/fixtures/conformance/podman-live/limitations.tsv"
workload_image="quay.io/libpod/alpine@sha256:634a8f35b5f16dcf4aaa0822adc0b1964bb786fca12f6831de8ddc45e5986a00"
workload_local_tag="localhost/boxferry-live/alpine:634a8f35b5f16dcf4aaa0822adc0b1964bb786fca12f6831de8ddc45e5986a00"

usage() {
  cat << 'EOF'
Usage: scripts/podman-live-conformance.sh --profile <smoke|full-container> [OPTIONS]

Options:
  --engine <PATH>       Outer Podman executable (default: podman).
  --matrix <PATH>       Reviewed tab-separated image matrix.
  --matrix-cell <ID>    Run one exact reviewed container cell.
  --matrix-start-at <ID>
                        Resume full-container at one exact reviewed container cell.
  --retain-artifacts    Keep target/podman-live/<run-id> after success.
  -h, --help            Show this help.

Both profiles must run as root (for example, `sudo bash ...`) and operate only inside disposable
outer containers. They never mount a host Podman socket, checkout, or credentials into an image.
EOF
}

while (($# > 0)); do
  case "$1" in
    --profile)
      profile="${2:-}"
      shift 2
      ;;
    --engine)
      engine="${2:-}"
      shift 2
      ;;
    --matrix)
      matrix_path="${2:-}"
      shift 2
      ;;
    --matrix-cell)
      matrix_cell="${2:-}"
      shift 2
      ;;
    --matrix-start-at)
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
      printf 'Unknown argument: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

case "${profile}" in
  smoke | full-container) ;;
  *)
    printf '%s\n' '--profile must be smoke or full-container.' >&2
    usage >&2
    exit 2
    ;;
esac

if ((EUID != 0)); then
  printf '%s\n' 'Run this isolated nested-runtime harness as root, for example with sudo.' >&2
  exit 2
fi
if [[ ! -x "${engine}" ]] && ! command -v "${engine}" > /dev/null 2>&1; then
  printf 'Outer Podman executable is unavailable: %s\n' "${engine}" >&2
  exit 2
fi
for command in getcap jq timeout unzip python3; do
  if ! command -v "${command}" > /dev/null 2>&1; then
    printf 'Required live-conformance command is unavailable: %s\n' "${command}" >&2
    exit 2
  fi
done
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

validate_catalogues() {
  local cells container_cells limitations scenarios
  cells="$(awk -F '\t' 'NF && $1 !~ /^#/ { count++ } END { print count + 0 }' "${matrix_path}")"
  container_cells="$(awk -F '\t' 'NF && $1 !~ /^#/ && $6 == "container" { count++ } END { print count + 0 }' "${matrix_path}")"
  limitations="$(awk -F '\t' 'NF && $1 !~ /^#/ { count++ } END { print count + 0 }' "${limitation_path}")"
  scenarios="$(awk -F '\t' 'NF && $1 !~ /^#/ { count++ } END { print count + 0 }' "${scenario_path}")"
  [[ "${cells}" == 48 && "${container_cells}" == 48 && "${limitations}" == 5 && "${scenarios}" -ge 16 ]] || {
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
      if (NF != 2 || !($1 in matrix) || seen[$1]++) {
        printf "Invalid limited matrix cell at line %d: %s\\n", FNR, $0 > "/dev/stderr"; bad = 1
      }
    }
    END { exit bad }
  ' "${matrix_path}" "${limitation_path}"
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
  awk -F '\t' -v expected="${scenario}" '
    $1 == expected { matches++ }
    END { exit matches == 1 ? 0 : 1 }
  ' "${scenario_path}" || {
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
discovery_parent_created=false
started_outer=""
apply_target_outer=""
apply_target_socket=""
current_podman_major=""
current_podman_rootless=""
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

progress_begin() {
  progress_test_name=$1
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

trap progress_fail ERR

cleanup() {
  local status=$?
  local outer directory image pid socket
  for outer in "${outer_containers[@]:-}"; do
    timeout --signal=TERM --kill-after=10s 30s \
      "${engine}" rm --force --ignore -- "${outer}" > /dev/null 2>&1 || true
  done
  for image in "${mounted_images[@]:-}"; do
    timeout --signal=TERM --kill-after=10s 30s \
      "${engine}" image unmount -- "${image}" > /dev/null 2>&1 || true
  done
  for pid in "${fault_proxy_pids[@]:-}"; do
    kill "${pid}" > /dev/null 2>&1 || true
    wait "${pid}" > /dev/null 2>&1 || true
  done
  for socket in "${fault_proxy_sockets[@]:-}"; do
    rm -f -- "${socket}"
  done
  for directory in "${discovery_directories[@]:-}"; do
    rm -f -- "${directory}/podman.sock" "${directory}/bootstrap.log" \
      "${directory}/runtime-evidence.tsv" "${directory}/runtime-evidence.ready" \
      "${directory}/runtime-canaries.log" "${directory}/selected-container-id" \
      "${directory}/smoke-baseline.json" "${directory}/start-api"
    rmdir -- "${directory}" 2> /dev/null || true
  done
  if [[ "${discovery_parent_created}" == true ]]; then
    rmdir -- /run/user/0 2> /dev/null || true
  fi
  rm -rf -- "${runtime_root}"
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
  local include_canaries=${5:-true} deadline=5m
  if [[ "${scope}" == minimal ]]; then
    deadline=2m
  fi
  printf 'Live setup: create %s nested resources for %s\n' "${scope}" "${prefix}"
  # shellcheck disable=SC2016 # ${...} expands in the nested shell, not this script.
  if ! startup_substep "create ${scope} nested resources (deadline ${deadline})" \
    timeout --signal=TERM --kill-after=30s "${deadline}" \
    "${engine}" exec --env "BF_PREFIX=${prefix}" \
    --env "BF_WORKLOAD_IMAGE=${workload_local_tag}" --env "BF_WORKLOAD_SCOPE=${scope}" \
    --env "BF_INCLUDE_CANARIES=${include_canaries}" \
    "${outer}" /bin/sh -ceu '
    image="${BF_WORKLOAD_IMAGE}"
    portable_image="registry.example.invalid/boxferry/${BF_PREFIX}:1"
    run_label="--label io.boxferry.live-run=${BF_PREFIX}"
    compose_labels="${run_label} --label com.docker.compose.project=${BF_PREFIX}"
    canary_log=/boxferry-socket/runtime-canaries.log
    : > "${canary_log}"
    # Rootless conmon/slirp helpers can inherit the outer `podman exec` output pipe even after
    # detached workload commands finish.  Keep all nested setup output in the bind-mounted log
    # so those helpers cannot keep the outer command attached.
    exec > /boxferry-socket/bootstrap.log 2>&1
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
  '; then
    cat -- "${socket_directory}/bootstrap.log" >&2 || true
    cat -- "${socket_directory}/runtime-canaries.log" >&2 || true
    return 1
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
    resolved_digest="$(engine_operation "inspect ${id} image digest" \
      image inspect --format '{{.Digest}}' "${image}")"
  else
    return "${cache_status}"
  fi
  if [[ "${resolved_digest}" != "${expected_digest}" ]]; then
    printf 'Resolved matrix image digest mismatch for %s: expected %s, observed %s\n' \
      "${id}" "${expected_digest}" "${resolved_digest}" >&2
    return 1
  fi
  printf '%s\n' "${resolved_digest}" > "${artifact_root}/${id}.digest"
}

start_outer_runtime() {
  local id=$1 image=$2 mode=$3 socket_directory=$4
  local outer_digest
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
    --volume "${workload_archive}:/boxferry-workload.tar:ro" \
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
          printf "upstream-source-build:%s\n" "$(podman --version)"
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

assert_successful_conversion() {
  local output=$1 selection=$2 output_directory=$3 report=$4
  jq --exit-status '
    .schema_version == 1 and .status == "success" and .exit_category == "success" and
    (.primary_diagnostic_code == null) and
    ([.diagnostics[]? | select(.severity == "error" or .code == "BFP0001")] | length == 0) and
    ((.output_artifacts | length) > 0)
  ' "${report}" > /dev/null || {
    printf 'Conversion report is not a successful BFP0001-free result: %s\n' "${report}" >&2
    return 1
  }
  [[ -d "${output_directory}" ]] && [[ -n "$(find "${output_directory}" -type f -print -quit)" ]] || {
    printf 'Conversion wrote no artifacts: %s\n' "${output_directory}" >&2
    return 1
  }
  local expected_count actual_count
  expected_count="$(jq '.output_artifacts | length' "${report}")"
  actual_count="$(find "${output_directory}" -maxdepth 1 -type f | wc -l)"
  [[ "${expected_count}" == "${actual_count}" ]] || {
    printf 'Report/artifact count mismatch for %s: report=%s files=%s\n' \
      "${output_directory}" "${expected_count}" "${actual_count}" >&2
    return 1
  }
  case "${output}" in
    compose) [[ -s "${output_directory}/compose.yaml" ]] ;;
    quadlet) find "${output_directory}" -maxdepth 1 -type f -name '*.container' -print -quit | grep --quiet . ;;
    podman)
      [[ -s "${output_directory}/podman.json" && -s "${output_directory}/podman-commands.sh" ]]
      sh -n "${output_directory}/podman-commands.sh"
      local operation_count command_count
      operation_count="$(jq '.operations | length' "${output_directory}/podman.json")"
      command_count="$(grep --count '^podman ' "${output_directory}/podman-commands.sh")"
      [[ "${operation_count}" == "${command_count}" ]] || {
        printf 'Podman operation/script count mismatch in %s: operations=%s commands=%s\n' \
          "${output_directory}" "${operation_count}" "${command_count}" >&2
        return 1
      }
      jq --exit-status '
        .schema_version == 1 and (.operations | length > 0) and
        all(.operations[]; .cli.program == "podman" and (.cli.argv | type == "array" and length > 0))
      ' "${output_directory}/podman.json" > /dev/null
      ;;
    *)
      printf 'Unknown output assertion: %s\n' "${output}" >&2
      return 1
      ;;
  esac
  assert_selection_membership "${selection}" "${output}" "${output_directory}"
}

assert_named_member() {
  local output=$1 directory=$2 name=$3 native=$4
  case "${output}" in
    compose)
      awk -v short="${name}" -v native="${native}" '
        $0 == "services:" { inside = 1; next }
        inside && /^[^ ]/ { inside = 0 }
        inside && ($0 == "  " short ":" || $0 == "  " native ":") { found = 1 }
        END { exit !found }
      ' "${directory}/compose.yaml"
      ;;
    quadlet)
      find "${directory}" -maxdepth 1 -type f \( -name "${name}.container" -o -name "${native}.container" \) \
        -print -quit | grep --quiet .
      ;;
    podman)
      jq --exit-status --arg short "${name}" --arg native "${native}" \
        'any(.operations[]?; .resource.kind == "container" and
          (.resource.name == $short or .resource.name == $native))' \
        "${directory}/podman.json" > /dev/null
      ;;
  esac
}

assert_named_absent() {
  local output=$1 directory=$2 name=$3 native=$4
  if assert_named_member "${output}" "${directory}" "${name}" "${native}"; then
    printf 'Selection unexpectedly retained %s in %s.\n' "${native}" "${directory}" >&2
    return 1
  fi
}

assert_resource_member() {
  local output=$1 directory=$2 kind=$3 name=$4
  case "${output}" in
    compose)
      awk -v section="${kind}s" -v key="${name}" '
        $0 == section ":" { inside = 1; next }
        inside && /^[^ ]/ { inside = 0 }
        inside && $0 == "  " key ":" { found = 1 }
        END { exit !found }
      ' "${directory}/compose.yaml"
      ;;
    quadlet)
      if [[ "${kind}" == network ]]; then
        grep --recursive --extended-regexp --quiet "^Network=${name}(\\.network)?$" "${directory}"
      else
        grep --recursive --extended-regexp --quiet "^Volume=${name}(\\.volume)?(:|$)" "${directory}"
      fi
      ;;
    podman)
      jq --exit-status --arg kind "${kind}" --arg name "${name}" \
        '[.external_preconditions[]?, .operations[]?.resource] |
          any(.kind == $kind and .name == $name)' "${directory}/podman.json" > /dev/null
      ;;
  esac
}

assert_resource_absent() {
  local output=$1 directory=$2 kind=$3 name=$4
  if assert_resource_member "${output}" "${directory}" "${kind}" "${name}"; then
    printf 'Selection unexpectedly retained %s %s in %s.\n' "${kind}" "${name}" "${directory}" >&2
    return 1
  fi
}

assert_network_boundary_semantics() {
  local output=$1 directory=$2
  local api="${current_prefix}-large-api"
  local private="${current_prefix}-large-private"
  local edge="${current_prefix}-large-edge"
  local cache="${current_prefix}-large-cache"
  local dual_attachment=true
  if [[ "${current_podman_major}" -lt 4 && "${current_podman_rootless}" == true ]]; then
    dual_attachment=false
  fi
  case "${output}" in
    compose)
      local service_block="${current_case}/api-service.compose.yaml"
      awk -v service="${api}" '
        $0 == "  " service ":" { inside = 1 }
        inside && $0 ~ /^  [^ ].*:$/ && $0 != "  " service ":" { exit }
        inside { print }
      ' "${directory}/compose.yaml" > "${service_block}"
      grep --fixed-strings --quiet "source: ${cache}" "${service_block}"
      grep --fixed-strings --quiet 'target: /cache' "${service_block}"
      grep --fixed-strings --quiet 'read_only: true' "${service_block}"
      grep --fixed-strings --quiet "${private}: {}" "${service_block}"
      if [[ "${dual_attachment}" == true ]]; then
        grep --fixed-strings --quiet "${edge}: {}" "${service_block}"
      else
        local proxy_block="${current_case}/proxy-service.compose.yaml"
        awk -v service="${current_prefix}-large-proxy" '
          $0 == "  " service ":" { inside = 1 }
          inside && $0 ~ /^  [^ ].*:$/ && $0 != "  " service ":" { exit }
          inside { print }
        ' "${directory}/compose.yaml" > "${proxy_block}"
        grep --fixed-strings --quiet "${edge}: {}" "${proxy_block}"
      fi
      ;;
    quadlet)
      local api_unit="${directory}/${api}.container"
      [[ -s "${api_unit}" ]]
      grep --fixed-strings --line-regexp --quiet "Network=${private}" "${api_unit}"
      if [[ "${dual_attachment}" == true ]]; then
        grep --fixed-strings --line-regexp --quiet "Network=${edge}" "${api_unit}"
      else
        grep --fixed-strings --line-regexp --quiet "Network=${edge}" \
          "${directory}/${current_prefix}-large-proxy.container"
      fi
      grep --extended-regexp --quiet "^Volume=${cache}:/cache:ro(,.*)?$" "${api_unit}"
      ;;
    podman)
      if [[ "${dual_attachment}" == true ]]; then
        jq --exit-status --arg api "${api}" --arg private "${private}" --arg edge "${edge}" --arg cache "${cache}" '
          any(.operations[]?;
            .resource.kind == "container" and .resource.name == $api and
            (.libpod.body.json.Networks[$private] != null) and
            (.libpod.body.json.Networks[$edge] != null) and
            any(.libpod.body.json.volumes[]?;
              .Name == $cache and .Dest == "/cache" and (.Options | index("ro") != null)))
        ' "${directory}/podman.json" > /dev/null
      else
        jq --exit-status --arg api "${api}" --arg proxy "${current_prefix}-large-proxy" \
          --arg private "${private}" --arg edge "${edge}" --arg cache "${cache}" '
          any(.operations[]?;
            .resource.kind == "container" and .resource.name == $api and
            (.libpod.body.json.Networks[$private] != null) and
            any(.libpod.body.json.volumes[]?;
              .Name == $cache and .Dest == "/cache" and (.Options | index("ro") != null))) and
          any(.operations[]?;
            .resource.kind == "container" and .resource.name == $proxy and
            (.libpod.body.json.Networks[$edge] != null))
        ' "${directory}/podman.json" > /dev/null
      fi
      ;;
  esac
}

assert_selection_membership() {
  local selection=$1 output=$2 directory=$3 expected excluded name
  case "${selection}" in
    exact | exact-repeat)
      expected='web'
      excluded='api'
      ;;
    prefix)
      expected='api db worker proxy cache'
      excluded='web stopped'
      ;;
    label | all)
      expected='web api db worker stopped options'
      if [[ "${current_podman_major}" -ge 4 ]]; then
        expected+=' pod-member pod-secondary-member'
      fi
      excluded=''
      ;;
    network-boundary)
      expected='api db worker proxy cache options'
      excluded='web'
      ;;
    section-independent)
      expected='stopped'
      excluded='web api'
      ;;
    *) return 0 ;;
  esac
  for name in ${expected}; do
    assert_named_member "${output}" "${directory}" "${name}" "${current_prefix}-${name}" ||
      assert_named_member "${output}" "${directory}" "${name}" "${current_prefix}-small-${name}" ||
      assert_named_member "${output}" "${directory}" "${name}" "${current_prefix}-large-${name}" || {
      printf 'Selection %s omitted expected member %s from %s.\n' "${selection}" "${name}" "${directory}" >&2
      return 1
    }
  done
  for name in ${excluded}; do
    assert_named_absent "${output}" "${directory}" "${name}" "${current_prefix}-${name}"
    assert_named_absent "${output}" "${directory}" "${name}" "${current_prefix}-small-${name}"
    assert_named_absent "${output}" "${directory}" "${name}" "${current_prefix}-large-${name}"
  done
  if [[ "${selection}" == network-boundary ]]; then
    assert_resource_member "${output}" "${directory}" network "${current_prefix}-large-private"
    assert_resource_member "${output}" "${directory}" network "${current_prefix}-large-edge"
    assert_resource_member "${output}" "${directory}" volume "${current_prefix}-large-data"
    assert_resource_member "${output}" "${directory}" volume "${current_prefix}-large-cache"
    assert_resource_absent "${output}" "${directory}" network "${current_prefix}-small-net"
    assert_resource_absent "${output}" "${directory}" volume "${current_prefix}-small-data"
    assert_network_boundary_semantics "${output}" "${directory}"
  fi
}

assert_deterministic_exact_compose() {
  local socket=$1
  run_convert compose "${socket}" exact-repeat --podman-resource "container=${current_prefix}-small-web"
  diff --recursive --unified "${current_case}/outputs/exact-compose" "${current_case}/outputs/exact-repeat-compose"
}

assert_neutral_projection_equivalent() {
  local left=$1 right=$2 assertion=$3 side input output report
  local projection_root="${current_case}/neutral-projections/${assertion}"
  mkdir -p -- "${projection_root}"
  for side in left right; do
    if [[ "${side}" == left ]]; then input="${left}"; else input="${right}"; fi
    output="${projection_root}/${side}"
    report="${projection_root}/${side}.report.json"
    boxferry_operation "BoxFerry ${assertion} ${side} neutral projection" \
      convert compose compose --input-file "${input}" --loss-policy partial \
      --output-directory "${output}" --console-format json > "${report}"
    jq --exit-status '
      .schema_version == 1 and .status == "success" and .exit_category == "success"
    ' "${report}" > /dev/null
    [[ -s "${output}/compose.yaml" ]]
    canonicalize_compose_semantics "${output}/compose.yaml" "${projection_root}/${side}.semantic.yaml"
  done
  diff --unified "${projection_root}/left.semantic.yaml" "${projection_root}/right.semantic.yaml"
}

canonicalize_compose_semantics() {
  local input=$1 output=$2
  python3 "${repository_root}/fixtures/conformance/podman-live/canonicalize_compose.py" "${input}" "${output}"
}

assert_strict_policy_blocks() {
  local socket=$1 report="${current_case}/strict-policy.report.json" error="${current_case}/strict-policy.stderr"
  if ! expected_failure_operation 90s 'BoxFerry strict-policy validation' 2 \
    "${boxferry_bin}" validate podman compose --podman-socket "${socket}" \
    --application-name "${current_prefix}" --podman-resource "container=${current_prefix}-small-web" \
    --loss-policy exact --console-format json > "${report}" 2> "${error}"; then
    printf '%s\n' 'Strict loss policy did not fail with the expected command status.' >&2
    return 1
  fi
  [[ ! -s "${error}" ]] || {
    printf 'Strict policy produced presentation stderr instead of structured JSON: %s\n' "${error}" >&2
    return 1
  }
  jq --exit-status '
    .schema_version == 1 and (.status == "blocked" or .status == "failure") and
    (.primary_diagnostic_code == "BFP0002" or .primary_diagnostic_code == "BFP0003") and
    any(.diagnostics[]?; .code == "BFP0002" or .code == "BFP0003") and
    (.fix_first.code == .primary_diagnostic_code)
  ' "${report}" > /dev/null
}

run_reimports() {
  local selection input output reimport_directory
  mkdir -p -- "${current_case}/reimports"
  for selection in exact prefix label all network-boundary; do
    for input in compose quadlet; do
      require_scenario "${input}-reimports"
      reimport_directory="${current_case}/outputs/${selection}-${input}"
      for output in compose quadlet podman; do
        local -a command=("${boxferry_bin}" convert "${input}" "${output}" --loss-policy partial)
        if [[ "${input}" == compose ]]; then
          command+=(--input-file "${reimport_directory}/compose.yaml")
        else
          while IFS= read -r -d '' file; do command+=(--input-file "${file}"); done < <(find "${reimport_directory}" -maxdepth 1 -type f -print0 | sort -z)
          command+=(--application-name "${current_prefix}")
        fi
        if [[ "${output}" == podman ]]; then command+=(--podman-target-context rootful); fi
        local result="${current_case}/reimports/${selection}-${input}-to-${output}"
        local report="${result}.report.json"
        command+=(--output-directory "${result}" --console-format json)
        timed_operation 90s "BoxFerry ${selection} ${input}-to-${output} reimport" \
          "${command[@]}" > "${report}"
        assert_successful_conversion "${output}" "${selection}" "${result}" "${report}"
      done
    done
    require_scenario neutral-projection-equivalence
    assert_neutral_projection_equivalent \
      "${current_case}/outputs/${selection}-compose/compose.yaml" \
      "${current_case}/reimports/${selection}-compose-to-compose/compose.yaml" \
      "${selection}-compose-reimport"
    if [[ "${selection}" != all ]]; then
      assert_neutral_projection_equivalent \
        "${current_case}/outputs/${selection}-compose/compose.yaml" \
        "${current_case}/reimports/${selection}-quadlet-to-compose/compose.yaml" \
        "${selection}-quadlet-reimport"
    elif [[ "${current_default_podman_network_present}" == true ]]; then
      jq --exit-status '
        .status == "success"
        and any(
          .diagnostics[]?;
          .code == "BFQ0007"
          and any(.fields[]?; .name == "subject" and .value == "networks.podman")
          and any(
            .fields[]?;
            .name == "reason"
            and .value == "network lifecycle ownership is uncertain; no managed .network unit was generated"
          )
        )
      ' "${current_case}/outputs/all-quadlet.report.json" > /dev/null || {
        printf '%s\n' \
          'All-resource Quadlet export did not report its uncertain unreferenced default network.' >&2
        return 1
      }
      printf '%s\n' \
        'all-resource Quadlet export diagnoses the uncertain default Podman network before reimport' \
        >> "${current_case}/feature-gates.txt"
    else
      # Podman 3.0 rootless does not list a default `podman` CNI network.  Assert the
      # lifecycle-ownership diagnostic only for a resource the live inventory actually contains.
      printf '%s\n' \
        'default Podman network absent from live inventory; default-network ownership diagnostic is inapplicable' \
        >> "${current_case}/feature-gates.txt"
    fi
  done
}

should_run_external_apply() {
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
  start_outer_runtime "${id}-apply-target" "${image}" "${mode}" "${socket_directory}"
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
  activate_outer_runtime "${socket_directory}"
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
  jq --exit-status --arg network "${expected_network}" --arg volume "${expected_volume}" \
    --arg container "${expected_container}" '
      (.external_preconditions | length == 2) and
      any(.external_preconditions[]; .kind == "network" and .name == $network) and
      any(.external_preconditions[]; .kind == "volume" and .name == $volume) and
      all(.external_preconditions[];
        (.kind == "network" and .name == $network) or (.kind == "volume" and .name == $volume)) and
      any(.operations[]; .action == "create" and .resource.kind == "container" and .resource.name == $container) and
      all(.operations[]; .cli.external_sensitive_input_required == false)
    ' "${source_plan}/podman.json" > /dev/null

  start_apply_target
  engine_operation 'create apply-target network' \
    exec "${apply_target_outer}" podman network create "${expected_network}" > /dev/null
  engine_operation 'create apply-target volume' \
    exec "${apply_target_outer}" podman volume create "${expected_volume}" > /dev/null
  timed_operation 3m 'execute generated plan inside apply target' \
    "${engine}" exec --interactive \
    "${apply_target_outer}" /bin/sh -seu \
    < "${source_plan}/podman-commands.sh"
  engine_operation 'inspect applied target container' \
    exec "${apply_target_outer}" podman inspect "${expected_container}" > /dev/null

  run_convert podman "${apply_target_socket}" apply-target \
    --podman-resource "container=${expected_container}"
  run_convert compose "${apply_target_socket}" apply-target \
    --podman-resource "container=${expected_container}"
  cmp --silent "${current_case}/outputs/apply-source-podman/podman.json" \
    "${current_case}/outputs/apply-target-podman/podman.json"
  cmp --silent "${current_case}/outputs/apply-source-compose/compose.yaml" \
    "${current_case}/outputs/apply-target-compose/compose.yaml"

  engine_operation 'remove applied target container' \
    exec "${apply_target_outer}" podman rm --force --time 0 "${expected_container}" > /dev/null
  engine_operation 'remove applied target network' \
    exec "${apply_target_outer}" podman network rm "${expected_network}" > /dev/null
  engine_operation 'remove applied target volume' \
    exec "${apply_target_outer}" podman volume rm "${expected_volume}" > /dev/null
  if ! expected_failure_operation 90s 'verify applied target cleanup' 125 \
    "${engine}" exec "${apply_target_outer}" podman inspect \
    "${expected_container}" > /dev/null 2>&1; then
    printf 'Applied conformance container survived exact cleanup: %s\n' "${expected_container}" >&2
    return 1
  fi
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
  contents="$(unzip -p "${archives[0]}")"
  if grep --fixed-strings --quiet 'not-a-secret-test-value' <<< "${contents}"; then
    printf 'Podman support archive retained the protected environment canary.\n' >&2
    return 1
  fi
  if grep --fixed-strings --quiet "${socket}" <<< "${contents}"; then
    printf 'Podman support archive retained its connection endpoint.\n' >&2
    return 1
  fi
  # The snapshot deliberately retains field names but serializes every protected
  # value as a state marker, never as a replacement string containing the value.
  grep --fixed-strings --quiet '"value_state": "redacted"' <<< "${contents}"
  grep --fixed-strings --quiet 'BOXFERRY_LIVE_MODE' <<< "${contents}"
}

podman_socket() {
  local socket=$1 action=${2:-command}
  shift
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
    "${socket_directory}/smoke-baseline.json" "${socket_directory}/start-api"
  start_outer_runtime "${id}" "${image}" "${mode}" "${socket_directory}"
  create_workloads "${started_outer}" "${current_prefix}" "${scope}" "${socket_directory}" false
  current_selected_container_id="$(< "${socket_directory}/selected-container-id")"
  [[ "${current_selected_container_id}" =~ ^[[:xdigit:]]{64}$ ]] || {
    printf 'Could not resolve selected container ID before acquisition started.\n' >&2
    return 1
  }
  activate_outer_runtime "${socket_directory}"
}

assert_runtime_scenarios() {
  local socket=$1 version=$2 major
  major="${version%%.*}"
  current_podman_major="${major}"
  current_podman_rootless="$(podman_socket "${socket}" info --format '{{.Host.Security.Rootless}}')"
  current_default_podman_network_present="$(
    podman_socket "${socket}" network ls --format json | jq --raw-output \
      'any(.[]?; .Name == "podman")'
  )"
  [[ "${current_default_podman_network_present}" == true || "${current_default_podman_network_present}" == false ]]
  printf 'Live scenario: stopped-and-running\n'
  require_scenario stopped-and-running
  [[ "$(podman_socket "${socket}" inspect --format '{{.State.Status}}' "${current_prefix}-running")" == running ]]
  local stopped_status
  stopped_status="$(podman_socket "${socket}" inspect --format '{{.State.Status}}' "${current_prefix}-stopped")"
  [[ "${stopped_status}" == created || "${stopped_status}" == configured ]]
  printf 'Live scenario: pod-members-and-standalone\n'
  require_scenario pod-members-and-standalone
  if ((major >= 4)); then
    [[ -n "$(podman_socket "${socket}" inspect --format '{{.Pod}}' "${current_prefix}-pod-member")" ]]
    [[ -n "$(podman_socket "${socket}" inspect --format '{{.Pod}}' "${current_prefix}-pod-secondary-member")" ]]
    [[ "$(podman_socket "${socket}" inspect --format '{{.Pod}}' "${current_prefix}-pod-member")" != "$(podman_socket "${socket}" inspect --format '{{.Pod}}' "${current_prefix}-pod-secondary-member")" ]]
    [[ -z "$(podman_socket "${socket}" inspect --format '{{.Pod}}' "${current_prefix}-small-web")" ]]
  else
    printf 'pod membership skipped by explicit Podman %s nested-runtime feature gate\n' "${version}" >> "${current_case}/feature-gates.txt"
  fi
  printf 'Live scenario: image-identities\n'
  require_scenario image-identities
  podman_socket "${socket}" inspect "${current_prefix}-small-web" | jq --exit-status '
    (((.[0].Config.Image // "") | startswith("registry.example.invalid/boxferry/")) or
      ((.[0].ImageName // "") | startswith("registry.example.invalid/boxferry/"))) and
    (.[0].Image | type == "string" and length > 0)
  ' > /dev/null
  podman_socket "${socket}" inspect "${current_prefix}-large-db" | jq --exit-status '
    ((((.[0].Config.Image // "") | startswith("registry.example.invalid/boxferry/")) or
      ((.[0].ImageName // "") | startswith("registry.example.invalid/boxferry/"))) and
    (.[0].Image | type == "string" and length > 0))
  ' > /dev/null
  podman_socket "${socket}" inspect "${current_prefix}-stopped" | jq --exit-status '
    (((.[0].Config.Image // "") | startswith("registry.example.invalid/boxferry/")) or
      ((.[0].ImageName // "") | startswith("registry.example.invalid/boxferry/"))) and
    (.[0].Image | type == "string" and length > 0)
  ' > /dev/null
  printf 'Live scenario: network-boundaries\n'
  require_scenario network-boundaries
  podman_socket "${socket}" inspect "${current_prefix}-large-api" > "${current_case}/network-boundaries.inspect.json"
  if ((major >= 4)); then
    jq --exit-status --arg private "${current_prefix}-large-private" --arg edge "${current_prefix}-large-edge" '
      (.[0].NetworkSettings.Networks[$private].Aliases | index("api")) and
      (.[0].NetworkSettings.Networks[$edge].Aliases | index("public-api"))
    ' "${current_case}/network-boundaries.inspect.json" > /dev/null
  elif [[ "${current_podman_rootless}" == true ]]; then
    podman_socket "${socket}" inspect "${current_prefix}-large-proxy" > "${current_case}/network-boundaries-proxy.inspect.json"
    jq --exit-status --arg private "${current_prefix}-large-private" '
      .[0].NetworkSettings.Networks | has($private)
    ' "${current_case}/network-boundaries.inspect.json" > /dev/null
    jq --exit-status --arg edge "${current_prefix}-large-edge" '
      .[0].NetworkSettings.Networks | has($edge)
    ' "${current_case}/network-boundaries-proxy.inspect.json" > /dev/null
    printf 'dual rootless CNI attachment is unavailable in Podman %s; private API and edge proxy prove the boundary\n' \
      "${version}" >> "${current_case}/feature-gates.txt"
  else
    jq --exit-status --arg private "${current_prefix}-large-private" --arg edge "${current_prefix}-large-edge" '
      (.[0].NetworkSettings.Networks | has($private)) and
      (.[0].NetworkSettings.Networks | has($edge))
    ' "${current_case}/network-boundaries.inspect.json" > /dev/null
    printf 'network aliases are unavailable in Podman %s inspect evidence\n' "${version}" \
      >> "${current_case}/feature-gates.txt"
  fi
  printf 'Live scenario: mount-matrix\n'
  require_scenario mount-matrix
  podman_socket "${socket}" inspect "${current_prefix}-large-db" | jq --exit-status '
    [.[0].Mounts[]? | select(.Type == "volume" and .RW == true)] | length == 1
  ' > /dev/null
  podman_socket "${socket}" inspect "${current_prefix}-large-api" | jq --exit-status '
    [.[0].Mounts[]? | select(.Type == "volume" and .RW == false and .Destination == "/cache")] | length == 1
  ' > /dev/null
  podman_socket "${socket}" inspect "${current_prefix}-options" | jq --exit-status '
    ([.[0].Mounts[]? | select(.Type == "bind" and .RW == false and .Propagation == "rprivate")] | length == 1) and
    (([.[0].Mounts[]? | select(.Type == "tmpfs")] | length == 1) or
      (.[0].HostConfig.Tmpfs["/scratch"] != null))
  ' > /dev/null
  printf 'Live scenario: environment-matrix\n'
  require_scenario environment-matrix
  podman_socket "${socket}" inspect "${current_prefix}-options" | jq --exit-status '
    ([.[0].Config.Env[]? | select(. == "BOXFERRY_ENV_FILE=present" or . == "BOXFERRY_ENV=inline")] | length == 2) and
    ([.[0].Config.Env[]? | select(. == "BOXFERRY_PROTECTED_TOKEN=not-a-secret-test-value")] | length == 1)
  ' > /dev/null
  printf 'Live scenario: runtime-policy-matrix\n'
  require_scenario runtime-policy-matrix
  podman_socket "${socket}" inspect "${current_prefix}-options" > "${current_case}/options.inspect.json"
  if ((major >= 4)); then
    jq --exit-status '
      .[0].HostConfig.Memory > 0 and .[0].HostConfig.PidsLimit == 64 and
      (.[0].HostConfig.CapDrop | length > 0) and
      .[0].HostConfig.RestartPolicy.Name == "on-failure" and
      .[0].HostConfig.RestartPolicy.MaximumRetryCount == 3 and
      .[0].HostConfig.LogConfig.Type == "k8s-file" and
      (.[0].HostConfig.IpcMode == "private" or .[0].HostConfig.IpcMode == "shareable") and
      .[0].HostConfig.PidMode == "private" and
      .[0].HostConfig.UTSMode == "private" and
      ((.[0].HostConfig.Annotations["io.boxferry.live"] //
        .[0].Config.Annotations["io.boxferry.live"]) == "present")
    ' "${current_case}/options.inspect.json" > /dev/null
  else
    jq --exit-status '
      .[0].HostConfig.CapDrop | length > 0
    ' "${current_case}/options.inspect.json" > /dev/null
    printf 'resource controls skipped by explicit Podman %s feature gate\n' "${version}" >> "${current_case}/feature-gates.txt"
  fi
  printf 'Live scenario: healthy-and-unhealthy\n'
  require_scenario healthy-and-unhealthy
  if ((major >= 4)); then
    local healthy_status=starting unhealthy_status=starting
    for _ in {1..30}; do
      healthy_status="$(podman_socket "${socket}" inspect --format '{{.State.Health.Status}}' "${current_prefix}-healthy")"
      unhealthy_status="$(podman_socket "${socket}" inspect --format '{{.State.Health.Status}}' "${current_prefix}-unhealthy")"
      [[ "${healthy_status}" == healthy && "${unhealthy_status}" == unhealthy ]] && break
      sleep 1
    done
    printf 'healthy=%s\nunhealthy=%s\n' "${healthy_status}" "${unhealthy_status}" > "${current_case}/health-status.txt"
    [[ "${healthy_status}" == healthy && "${unhealthy_status}" == unhealthy ]] || {
      printf 'Unexpected health states: healthy=%s unhealthy=%s\n' "${healthy_status}" "${unhealthy_status}" >&2
      return 1
    }
  else
    printf 'health states skipped by explicit Podman %s feature gate\n' "${version}" >> "${current_case}/feature-gates.txt"
  fi
  printf 'Live scenario: secret-conditional\n'
  require_scenario secret-conditional
  if ((major >= 4)) &&
    podman_socket "${socket}" secret inspect "${current_prefix}-conditional" > /dev/null 2>&1; then
    podman_socket "${socket}" inspect "${current_prefix}-secret" | jq --exit-status \
      '[.. | strings | select(contains("conditional"))] | length > 0' > /dev/null
    printf 'secret endpoint supported\n' >> "${current_case}/feature-gates.txt"
  else
    printf 'secret endpoint unavailable\n' >> "${current_case}/feature-gates.txt"
  fi
}

assert_smoke_runtime_baseline() {
  local baseline="${socket_directory}/smoke-baseline.json"
  require_scenario stopped-and-running
  jq --exit-status --arg selected "${current_prefix}-small-web" \
    --arg running "${current_prefix}-running" --arg stopped "${current_prefix}-stopped" \
    --arg network "${current_prefix}-small-net" --arg volume "${current_prefix}-small-data" '
      any(.[]; .Name == $running and .State.Status == "running") and
      any(.[]; .Name == $stopped and
        (.State.Status == "created" or .State.Status == "configured")) and
      any(.[]; .Name == $selected and
        ((((.Config.Image // "") | startswith("registry.example.invalid/boxferry/")) or
          ((.ImageName // "") | startswith("registry.example.invalid/boxferry/"))) and
        (.NetworkSettings.Networks[$network] != null) and
        any(.Mounts[]?; .Name == $volume and .Destination == "/var/lib/boxferry" and .RW == true) and
        any(.Config.Env[]?; . == "BOXFERRY_LIVE_MODE=small")))
    ' "${baseline}" > /dev/null
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
    "${uid_socket_directory}/smoke-baseline.json" "${uid_socket_directory}/start-api"
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

configure_cell_progress() {
  local id=$1
  progress_index=0
  if [[ "${profile}" == smoke ]]; then
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
  if should_run_discovery "${id}"; then
    ((progress_total += 1))
  fi
  printf '%s PLAN cell=%s profile=%s tests=%d\n' \
    "$(timestamp)" "${id}" "${profile}" "${progress_total}"
}

run_cell() {
  local id=$1 image=$2 declared_version=$3 distribution=$4 mode=$5 lane=$6 architecture=$7
  current_case="${artifact_root}/${id}"
  current_prefix="${run_id}-${id}"
  # Keep generated container names valid as single DNS-label network aliases.
  current_prefix="${current_prefix:0:48}"
  if [[ "${current_prefix}" == *- ]]; then
    current_prefix="${current_prefix%-}x"
  fi
  mkdir -p -- "${current_case}"
  local socket_directory="${runtime_root}/${id}"
  local outer socket coverage_level workload_scope=full runtime_test_name
  if [[ "${profile}" == smoke ]]; then
    workload_scope=minimal
  fi
  configure_cell_progress "${id}"
  printf '%s CELL START %s (%s, %s profile)\n' "$(timestamp)" "${id}" "${mode}" "${profile}"
  progress_run 'prepare digest-pinned workload archive' prepare_workload_archive
  progress_run 'pull and verify reviewed Podman image' prepare_matrix_image "${id}" "${image}"
  progress_run 'start isolated Podman container and collect runtime evidence' \
    start_outer_runtime "${id}" "${image}" "${mode}" "${socket_directory}"
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
  current_podman_major="$(awk '{ split($3, version, "."); print version[1] }' \
    "${artifact_root}/${id}.podman-version")"
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
  progress_pass

  local selection output
  local -a selections=(exact)
  if [[ "${profile}" == full-container ]]; then
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
  if [[ "${profile}" == full-container ]] || is_smoke_diagnostics_cell "${id}"; then
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
  if [[ "${profile}" == full-container ]]; then
    require_scenario disappeared-selected-container
    progress_run 'diagnose disappeared selected container' run_fault_proxy_case "${socket}" gone
    require_scenario partial-inventory-section
    progress_run 'diagnose partial inventory section' run_partial_section_failure "${socket}"
    progress_run 're-import generated Compose and Quadlet outputs' run_reimports
    if should_run_external_apply "${id}"; then
      progress_run 'externally apply and reacquire Podman plan' \
        run_external_apply_reacquire "${socket}"
    fi
  fi
  if should_run_discovery "${id}"; then
    require_scenario socket-discovery
    progress_run 'discover local Podman socket' run_discovery "${id}" "${image}" "${mode}"
  fi
  progress_run 'remove disposable outer container' remove_outer "${outer}"
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
      printf "upstream-source-build:%s\n" "$(podman --version)"
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
  printf '%s CELL PASS  %s (%d/%d tests, reviewed limitation)\n' \
    "$(timestamp)" "${id}" "${progress_index}" "${progress_total}"
}

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
