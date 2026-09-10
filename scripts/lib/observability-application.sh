#!/usr/bin/env bash
# Prometheus, Loki, Grafana, and Alloy application acceptance helpers.
# Sourced by the live entry point; this file performs no work while sourced.
# shellcheck disable=SC2016,SC2129,SC2154 # Remote expansion and runner globals are intentional.

readonly OBSERVABILITY_ADMIN_PASSWORD="boxferry-public-observability-admin-canary"
readonly OBSERVABILITY_PROVIDER_VERSION="5.5.0"
readonly OBSERVABILITY_PROVIDER_SHA256="c57ab918abd5b05ca7e7d0f275875dd1330a695074f309dc9eab1b49efafcd4b"
readonly OBSERVABILITY_HTTP_PORT="13000"
readonly OBSERVABILITY_ARCHIVE_MAX_BYTES="2147483648"
readonly OBSERVABILITY_MIN_CPUS="2"
readonly OBSERVABILITY_MIN_MEMORY_KIB="4194304"
readonly OBSERVABILITY_MIN_DISK_KIB="8388608"

observability_fixture_root() {
  printf '%s/fixtures/conformance/observability-application\n' \
    "${repository_root:?caller must supply repository_root}"
}

observability_image_reference() {
  local id=$1
  printf 'registry.invalid/boxferry-test/observability-application:%s\n' "${id}"
}

observability_validate_resource_budget() {
  local cpus memory_kib graph_root disk_kib path observed
  cpus="$(nproc)"
  memory_kib="$(awk '$1 == "MemAvailable:" { print $2 }' /proc/meminfo)"
  graph_root="$("${engine}" info --format '{{.Store.GraphRoot}}')"
  disk_kib=""
  for path in "${runtime_root:?caller must supply runtime_root}" "${graph_root}"; do
    observed="$(df --output=avail -k -- "${path}" | awk 'NR == 2 { print $1 }')"
    if [[ -z "${disk_kib}" || "${observed}" -lt "${disk_kib}" ]]; then
      disk_kib="${observed}"
    fi
  done

  [[ "${cpus}" =~ ^[0-9]+$ && "${cpus}" -ge "${OBSERVABILITY_MIN_CPUS}" ]] || {
    printf 'Observability profile requires at least %s CPUs; found %s.\n' \
      "${OBSERVABILITY_MIN_CPUS}" "${cpus:-unknown}" >&2
    return 1
  }
  [[ "${memory_kib}" =~ ^[0-9]+$ && "${memory_kib}" -ge "${OBSERVABILITY_MIN_MEMORY_KIB}" ]] || {
    printf 'Observability profile requires at least 4 GiB available memory; found %s KiB.\n' \
      "${memory_kib:-unknown}" >&2
    return 1
  }
  [[ "${disk_kib}" =~ ^[0-9]+$ && "${disk_kib}" -ge "${OBSERVABILITY_MIN_DISK_KIB}" ]] || {
    printf 'Observability profile requires at least 8 GiB free disk; found %s KiB.\n' \
      "${disk_kib:-unknown}" >&2
    return 1
  }

  printf '%s OBSERVABILITY BUDGET cpus=%s memory-available-kib=%s disk-available-kib=%s\n' \
    "$(timestamp)" "${cpus}" "${memory_kib}" "${disk_kib}"
}

observability_validate_catalogues() {
  local fixture file
  fixture="$(observability_fixture_root)"
  awk -F '\t' -v image_pattern=':[^/@]+@sha256:[0-9a-f]{64}$' '
    NF && $1 !~ /^#/ {
      if (NF != 7 || $2 !~ image_pattern || $3 == "" ||
          $4 == "" || index($5, "https://") != 1 || $6 != "transient-test-pull" ||
          $7 != "linux/amd64" || seen[$1]++) {
        printf "Invalid observability image row at line %d: %s\\n", NR, $0 > "/dev/stderr"
        bad = 1
      }
      count++
    }
    END { exit bad || count != 5 }
  ' "${fixture}/images.tsv"
  awk -F '\t' -v version="${OBSERVABILITY_PROVIDER_VERSION}" \
    -v sha="${OBSERVABILITY_PROVIDER_SHA256}" '
    NF && $1 !~ /^#/ {
      if (NF != 8 || $1 != "docker-compose" || $2 != version ||
          $3 != "https://github.com/docker/compose/releases/download/v5.5.0/docker-compose-linux-x86_64" ||
          $4 != sha || $5 != "Apache-2.0" || $6 != "https://github.com/docker/compose" ||
          $7 != "downloaded-test-tool" || $8 != "linux/amd64") {
        printf "Invalid observability provider row at line %d: %s\\n", NR, $0 > "/dev/stderr"
        bad = 1
      }
      count++
    }
    END { exit bad || count != 1 }
  ' "${fixture}/providers.tsv"
  for file in compose.yaml config.alloy dashboard.json grafana-dashboards.yaml \
    grafana-datasources.yaml images.tsv loki.yaml producer.sh prometheus.yml providers.tsv README.md; do
    [[ -s "${fixture}/${file}" ]]
  done
  jq --exit-status '
    .uid == "boxferry-observability" and
    any(.panels[].targets[]?; .expr == "boxferry_fixture_temperature_celsius{source=\"controlled\"}") and
    any(.panels[].targets[]?; .expr == "{job=\"boxferry_fixture\"} |= \"boxferry-observability-known-log\"")
  ' "${fixture}/dashboard.json" > /dev/null
  grep --fixed-strings --quiet 'retention_period: 24h' "${fixture}/loki.yaml"
  grep --fixed-strings --quiet 'url = "http://prometheus:9090/api/v1/write"' \
    "${fixture}/config.alloy"
  grep --fixed-strings --quiet 'url = "http://loki:3100/loki/api/v1/push"' \
    "${fixture}/config.alloy"
}

observability_validate_provider() {
  local provider=${BOXFERRY_COMPOSE_BIN:-${repository_root}/target/tools/docker-compose}
  local observed_sha observed_version
  [[ -x "${provider}" ]] || {
    printf 'Observability profile requires executable Docker Compose %s at %s.\n' \
      "${OBSERVABILITY_PROVIDER_VERSION}" "${provider}" >&2
    return 1
  }
  observed_sha="$(sha256sum "${provider}" | awk '{ print $1 }')"
  [[ "${observed_sha}" == "${OBSERVABILITY_PROVIDER_SHA256}" ]] || {
    printf 'Docker Compose checksum mismatch: expected %s, observed %s.\n' \
      "${OBSERVABILITY_PROVIDER_SHA256}" "${observed_sha}" >&2
    return 1
  }
  observed_version="$("${provider}" version --short | sed 's/^v//')"
  [[ "${observed_version}" == "${OBSERVABILITY_PROVIDER_VERSION}" ]] || {
    printf 'Docker Compose version mismatch: expected %s, observed %s.\n' \
      "${OBSERVABILITY_PROVIDER_VERSION}" "${observed_version}" >&2
    return 1
  }
}

observability_verify_producer() {
  local temporary status=0
  temporary="$(mktemp -d /tmp/boxferry-observability-producer.XXXXXX)"
  sh "$(observability_fixture_root)/producer.sh" self-test "${temporary}" || status=$?
  rm -rf -- "${temporary}"
  return "${status}"
}

observability_prepare_image_archive() {
  local archive=${1:?archive path required}
  if [[ -s "${archive}" ]]; then
    [[ "$(stat -c '%s' "${archive}")" -le "${OBSERVABILITY_ARCHIVE_MAX_BYTES}" ]] || {
      printf 'Existing observability image archive exceeds 2 GiB cap.\n' >&2
      return 1
    }
    return 0
  fi

  local fixture id reference expected_digest observed_digest cache_status runtime_reference
  local archive_directory archive_size observed_architecture observed_os
  fixture="$(observability_fixture_root)"
  archive_directory="$(mktemp -d "${runtime_root}/observability-image-archives.XXXXXX")"
  while IFS=$'\t' read -r id reference _ _ _ _ platform; do
    [[ -z "${id}" || "${id}" == \#* ]] && continue
    [[ "${platform}" == linux/amd64 ]] || return 1
    expected_digest="${reference##*@}"
    cache_status=0
    engine_image_available "probe observability ${id} image cache" "${reference}" || cache_status=$?
    if ((cache_status == 1)); then
      timed_operation 8m "pull digest-pinned observability ${id} image" \
        "${engine}" pull --quiet --platform linux/amd64 "${reference}" \
        > "${artifact_root}/observability-${id}.pull.log"
    elif ((cache_status != 0)); then
      return "${cache_status}"
    fi
    read -r observed_digest observed_architecture observed_os < <(
      engine_operation "inspect observability ${id} image identity" image inspect \
        --format '{{.Digest}} {{.Architecture}} {{.Os}}' "${reference}"
    )
    [[ "${observed_digest}" == "${expected_digest}" &&
      "${observed_architecture}" == amd64 && "${observed_os}" == linux ]] || {
      printf 'Observability image identity mismatch for %s: %s %s/%s.\n' \
        "${id}" "${observed_digest}" "${observed_os}" "${observed_architecture}" >&2
      return 1
    }
    runtime_reference="$(observability_image_reference "${id}")"
    engine_operation "tag reviewed observability ${id} image" \
      tag "${reference}" "${runtime_reference}"
    timed_operation 10m "archive observability ${id} image" \
      "${engine}" save --format oci-archive \
      --output "${archive_directory}/${id}.oci.tar" "${runtime_reference}"
  done < "${fixture}/images.tsv"
  timed_operation 10m 'bundle digest-pinned observability image archives' \
    tar --create --file "${archive}" --directory "${archive_directory}" .
  rm -rf -- "${archive_directory}"
  chmod 0644 "${archive}"
  archive_size="$(stat -c '%s' "${archive}")"
  [[ "${archive_size}" -le "${OBSERVABILITY_ARCHIVE_MAX_BYTES}" ]] || {
    printf 'Observability image archive exceeds 2 GiB cap: %s bytes.\n' "${archive_size}" >&2
    return 1
  }
}

observability_assert_loaded_images() {
  local outer=$1 fixture id
  fixture="$(observability_fixture_root)"
  while IFS=$'\t' read -r id _ _ _ _ _ _; do
    [[ -z "${id}" || "${id}" == \#* ]] && continue
    engine_operation "verify loaded observability ${id} image" \
      exec "${outer}" podman image exists "$(observability_image_reference "${id}")"
  done < "${fixture}/images.tsv"
}

observability_prepare_application_target() {
  local outer=$1 prefix=$2 socket_directory=$3 fixture destination
  engine_operation 'copy rootless observability network configuration' cp \
    "${repository_root}/fixtures/conformance/podman-live/apply-target-containers.conf" \
    "${outer}:/tmp/99-boxferry-live.conf"
  # shellcheck disable=SC2016 # HOME expands inside the nested target.
  engine_operation 'prepare rootless observability network configuration' \
    exec "${outer}" /bin/sh -ceu \
    'mkdir -p "$HOME/.config/containers/containers.conf.d"; cp /tmp/99-boxferry-live.conf "$HOME/.config/containers/containers.conf.d/99-boxferry-live.conf"'
  # shellcheck disable=SC2016 # Archive loop expands inside the nested target.
  timed_operation 15m 'load digest-pinned observability application archives' \
    "${engine}" exec "${outer}" /bin/sh -ceu '
      directory=/tmp/boxferry-observability-images
      mkdir -p "$directory"
      tar -xf /boxferry-workload.tar -C "$directory"
      for archive in "$directory"/*.oci.tar; do podman load --input "$archive"; done
    ' > /dev/null
  observability_assert_loaded_images "${outer}"
  fixture="$(observability_fixture_root)"
  destination="/tmp/boxferry-fixture/${prefix}"
  engine_operation 'create disposable observability fixture directory' \
    exec "${outer}" mkdir -p -- "${destination}"
  engine_operation 'copy reviewed observability fixture' \
    cp "${fixture}/." "${outer}:${destination}"
  activate_outer_runtime "${socket_directory}"
}

observability_remote() {
  local socket=$1
  shift
  podman_socket "${socket}" "Observability ${1:-command}" "$@"
}

observability_wait_for() {
  local deadline_seconds=$1 description=$2
  shift 2
  local deadline=$((SECONDS + deadline_seconds))
  until "$@" > /dev/null 2>&1; do
    if ((SECONDS >= deadline)); then
      printf 'Timed out after %ss waiting for observability %s.\n' \
        "${deadline_seconds}" "${description}" >&2
      return 1
    fi
    sleep 2
  done
}

observability_assert_clean_prefix() {
  local socket=$1 prefix=$2 listing
  local -a collisions=()
  listing="$({
    observability_remote "${socket}" ps -a --format '{{.Names}}' || exit $?
    observability_remote "${socket}" volume ls --format '{{.Name}}' || exit $?
    observability_remote "${socket}" network ls --format '{{.Name}}'
  })" || {
    printf 'Could not inspect resources for observability prefix %s.\n' "${prefix}" >&2
    return 2
  }
  mapfile -t collisions < <(printf '%s\n' "${listing}" | awk -v prefix="${prefix}" 'index($0, prefix) == 1')
  if ((${#collisions[@]} > 0)); then
    printf 'Refusing observability provisioning because prefix %s owns resources:\n' \
      "${prefix}" >&2
    printf '  %s\n' "${collisions[@]}" >&2
    return 1
  fi
}

observability_expect_collision() {
  local status=0
  observability_assert_clean_prefix "$@" || status=$?
  if ((status == 1)); then
    return 0
  fi
  if ((status == 0)); then
    printf 'Observability collision check unexpectedly found a clean prefix.\n' >&2
    return 1
  fi
  return "${status}"
}

observability_create_edge_and_peer() {
  local socket=$1 prefix=$2 run=$3
  observability_remote "${socket}" network exists "${prefix}-observability-edge" 2> /dev/null ||
    observability_remote "${socket}" network create \
      --label "io.boxferry.live-run=${run}" --label io.boxferry.shared=true \
      "${prefix}-observability-edge" > /dev/null
  observability_remote "${socket}" container exists "${prefix}-observability-boundary-peer" 2> /dev/null ||
    observability_remote "${socket}" run --pull=never --detach \
      --name "${prefix}-observability-boundary-peer" \
      --label "io.boxferry.live-run=${run}" \
      --label "io.boxferry.application=${prefix}-boundary" \
      --network "${prefix}-observability-edge" \
      "$(observability_image_reference producer)" sleep 86400 > /dev/null
}

observability_create_cli_metrics_producer() {
  local socket=$1 prefix=$2 run=$3 fixture_root
  fixture_root="/tmp/boxferry-fixture/${prefix}"
  observability_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-observability-metrics-producer" \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-observability" \
    --network "${prefix}-observability-backend:alias=metrics-producer" \
    --volume "${fixture_root}:/fixture:ro" \
    "$(observability_image_reference producer)" \
    /bin/sh /fixture/producer.sh metrics > /dev/null
}

observability_create_cli_log_producer() {
  local socket=$1 prefix=$2 run=$3 fixture_root
  fixture_root="/tmp/boxferry-fixture/${prefix}"
  observability_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-observability-log-producer" \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-observability" \
    --network "${prefix}-observability-backend" \
    --volume "${fixture_root}:/fixture:ro" \
    --volume "${prefix}-observability-telemetry-logs:/var/log/boxferry" \
    "$(observability_image_reference producer)" \
    /bin/sh /fixture/producer.sh logs > /dev/null
}

observability_create_cli_prometheus() {
  local socket=$1 prefix=$2 run=$3 fixture_root
  fixture_root="/tmp/boxferry-fixture/${prefix}"
  observability_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-observability-prometheus" \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-observability" \
    --requires "${prefix}-observability-metrics-producer" \
    --network "${prefix}-observability-backend:alias=prometheus" \
    --volume "${fixture_root}/prometheus.yml:/etc/prometheus/prometheus.yml:ro" \
    --volume "${prefix}-observability-prometheus-data:/prometheus" \
    "$(observability_image_reference prometheus)" \
    --config.file=/etc/prometheus/prometheus.yml \
    --storage.tsdb.path=/prometheus --storage.tsdb.retention.time=24h \
    --web.enable-remote-write-receiver > /dev/null
}

observability_create_cli_loki() {
  local socket=$1 prefix=$2 run=$3 fixture_root
  fixture_root="/tmp/boxferry-fixture/${prefix}"
  observability_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-observability-loki" \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-observability" \
    --network "${prefix}-observability-backend:alias=loki" \
    --volume "${fixture_root}/loki.yaml:/etc/loki/loki.yaml:ro" \
    --volume "${prefix}-observability-loki-data:/loki" \
    "$(observability_image_reference loki)" \
    -config.file=/etc/loki/loki.yaml > /dev/null
}

observability_create_cli_alloy() {
  local socket=$1 prefix=$2 run=$3 fixture_root
  fixture_root="/tmp/boxferry-fixture/${prefix}"
  observability_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-observability-alloy" \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-observability" \
    --requires "${prefix}-observability-prometheus,${prefix}-observability-loki,${prefix}-observability-log-producer" \
    --network "${prefix}-observability-backend" \
    --volume "${fixture_root}/config.alloy:/etc/alloy/config.alloy:ro" \
    --volume "${prefix}-observability-alloy-data:/var/lib/alloy/data" \
    --volume "${prefix}-observability-telemetry-logs:/var/log/boxferry:ro" \
    "$(observability_image_reference alloy)" \
    run --storage.path=/var/lib/alloy/data /etc/alloy/config.alloy > /dev/null
}

observability_create_cli_grafana() {
  local socket=$1 prefix=$2 run=$3 fixture_root
  fixture_root="/tmp/boxferry-fixture/${prefix}"
  observability_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-observability-grafana" \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-observability" \
    --requires "${prefix}-observability-prometheus,${prefix}-observability-loki,${prefix}-observability-alloy" \
    --network "${prefix}-observability-backend" \
    --network "${prefix}-observability-edge:alias=grafana" \
    --publish "127.0.0.1:${OBSERVABILITY_HTTP_PORT}:3000" \
    --env GF_ANALYTICS_REPORTING_ENABLED=false \
    --env GF_AUTH_ANONYMOUS_ENABLED=true \
    --env GF_AUTH_ANONYMOUS_ORG_ROLE=Viewer \
    --env "GF_SECURITY_ADMIN_PASSWORD=${OBSERVABILITY_ADMIN_PASSWORD}" \
    --env GF_USERS_ALLOW_SIGN_UP=false \
    --volume "${fixture_root}/grafana-datasources.yaml:/etc/grafana/provisioning/datasources/boxferry.yaml:ro" \
    --volume "${fixture_root}/grafana-dashboards.yaml:/etc/grafana/provisioning/dashboards/boxferry.yaml:ro" \
    --volume "${fixture_root}/dashboard.json:/etc/grafana/provisioning/boxferry-dashboards/boxferry.json:ro" \
    --volume "${prefix}-observability-grafana-data:/var/lib/grafana" \
    "$(observability_image_reference grafana)" > /dev/null
}

observability_create_cli_runtime() {
  local socket=$1 prefix=$2 run=$3
  observability_create_cli_metrics_producer "${socket}" "${prefix}" "${run}"
  observability_create_cli_log_producer "${socket}" "${prefix}" "${run}"
  observability_create_cli_prometheus "${socket}" "${prefix}" "${run}"
  observability_create_cli_loki "${socket}" "${prefix}" "${run}"
  observability_create_cli_alloy "${socket}" "${prefix}" "${run}"
  observability_create_cli_grafana "${socket}" "${prefix}" "${run}"
}

observability_provision_cli() {
  local socket=$1 prefix=$2 run=$3 volume
  observability_assert_clean_prefix "${socket}" "${prefix}"
  observability_remote "${socket}" network create --internal \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-observability" \
    "${prefix}-observability-backend" > /dev/null
  observability_create_edge_and_peer "${socket}" "${prefix}" "${run}"
  for volume in alloy-data grafana-data loki-data prometheus-data telemetry-logs; do
    observability_remote "${socket}" volume create \
      --label "io.boxferry.live-run=${run}" \
      --label "io.boxferry.application=${prefix}-observability" \
      "${prefix}-observability-${volume}" > /dev/null
  done
  observability_create_cli_runtime "${socket}" "${prefix}" "${run}"
}

observability_compose_project() {
  local socket=$1 prefix=$2 run=$3
  shift 3
  local provider=${BOXFERRY_COMPOSE_BIN:-${repository_root}/target/tools/docker-compose}
  local fixture fixture_root
  fixture="$(observability_fixture_root)"
  fixture_root="/tmp/boxferry-fixture/${prefix}"
  timed_operation 5m "Docker Compose observability ${1:-command}" \
    env DOCKER_HOST="unix://${socket}" BF_PREFIX="${prefix}" BF_RUN="${run}" \
    BF_FIXTURE_ROOT="${fixture_root}" BF_ADMIN_PASSWORD="${OBSERVABILITY_ADMIN_PASSWORD}" \
    BF_PROMETHEUS_IMAGE="$(observability_image_reference prometheus)" \
    BF_LOKI_IMAGE="$(observability_image_reference loki)" \
    BF_GRAFANA_IMAGE="$(observability_image_reference grafana)" \
    BF_ALLOY_IMAGE="$(observability_image_reference alloy)" \
    BF_PRODUCER_IMAGE="$(observability_image_reference producer)" \
    "${provider}" --project-name "${prefix}-observability" \
    --file "${fixture}/compose.yaml" "$@"
}

observability_provision_compose() {
  local socket=$1 prefix=$2 run=$3
  observability_assert_clean_prefix "${socket}" "${prefix}"
  observability_create_edge_and_peer "${socket}" "${prefix}" "${run}"
  observability_compose_project "${socket}" "${prefix}" "${run}" \
    up --detach --remove-orphans > "${current_case}/observability-compose.log" 2>&1
}

observability_backend_get() {
  local socket=$1 prefix=$2 url=$3
  observability_remote "${socket}" exec "${prefix}-observability-metrics-producer" \
    wget -qO- "${url}"
}

observability_prometheus_has_value() {
  local socket=$1 prefix=$2 query=$3 expected=$4 response
  response="$(observability_backend_get "${socket}" "${prefix}" \
    "http://prometheus:9090/api/v1/query?query=${query}")"
  jq --exit-status --arg expected "${expected}" '
    .status == "success" and
    any(.data.result[]?; .value[1] == $expected)
  ' <<< "${response}" > /dev/null
}

observability_loki_has_known_log() {
  local socket=$1 prefix=$2 response
  response="$(observability_backend_get "${socket}" "${prefix}" \
    'http://loki:3100/loki/api/v1/query_range?query=%7Bjob%3D%22boxferry_fixture%22%7D%20%7C%3D%20%22boxferry-observability-known-log%22&limit=20')"
  jq --exit-status '
    .status == "success" and
    ([.data.result[]?.values[]? | select(.[1] | contains("boxferry-observability-known-log"))] | length) == 1
  ' <<< "${response}" > /dev/null
}

observability_grafana_api() {
  local socket=$1 prefix=$2 path=$3
  observability_backend_get "${socket}" "${prefix}" "http://grafana:3000${path}"
}

observability_wait_application() {
  local socket=$1 prefix=$2
  observability_wait_for 240 'Prometheus readiness' observability_backend_get \
    "${socket}" "${prefix}" http://prometheus:9090/-/ready
  observability_wait_for 240 'Loki readiness' observability_backend_get \
    "${socket}" "${prefix}" http://loki:3100/ready
  observability_wait_for 240 'Grafana readiness' observability_grafana_api \
    "${socket}" "${prefix}" /api/health
  observability_wait_for 240 'controlled PromQL result' observability_prometheus_has_value \
    "${socket}" "${prefix}" \
    'boxferry_fixture_temperature_celsius%7Bsource%3D%22controlled%22%7D' 42
  observability_wait_for 240 'controlled LogQL result' observability_loki_has_known_log \
    "${socket}" "${prefix}"
}

observability_assert_queries_and_grafana() {
  local socket=$1 prefix=$2 prometheus_flags prometheus_source loki_source dashboard
  observability_prometheus_has_value "${socket}" "${prefix}" \
    'boxferry_fixture_temperature_celsius%7Bsource%3D%22controlled%22%7D' 42
  observability_loki_has_known_log "${socket}" "${prefix}"

  prometheus_flags="$(observability_backend_get "${socket}" "${prefix}" \
    http://prometheus:9090/api/v1/status/flags)"
  jq --exit-status '
    .status == "success" and
    .data["storage.tsdb.retention.time"] == "24h" and
    .data["web.enable-remote-write-receiver"] == "true"
  ' <<< "${prometheus_flags}" > /dev/null

  prometheus_source="$(observability_grafana_api "${socket}" "${prefix}" \
    /api/datasources/uid/boxferry-prometheus)"
  loki_source="$(observability_grafana_api "${socket}" "${prefix}" \
    /api/datasources/uid/boxferry-loki)"
  jq --exit-status '
    .uid == "boxferry-prometheus" and .type == "prometheus" and
    .url == "http://prometheus:9090" and .isDefault == true
  ' <<< "${prometheus_source}" > /dev/null
  jq --exit-status '
    .uid == "boxferry-loki" and .type == "loki" and .url == "http://loki:3100"
  ' <<< "${loki_source}" > /dev/null

  observability_grafana_api "${socket}" "${prefix}" \
    /api/datasources/uid/boxferry-prometheus/health |
    jq --exit-status '.status == "OK"' > /dev/null
  observability_grafana_api "${socket}" "${prefix}" \
    /api/datasources/uid/boxferry-loki/health |
    jq --exit-status '.status == "OK"' > /dev/null

  dashboard="$(observability_grafana_api "${socket}" "${prefix}" \
    /api/dashboards/uid/boxferry-observability)"
  jq --exit-status '
    .dashboard.uid == "boxferry-observability" and
    .dashboard.title == "BoxFerry Observability Acceptance" and
    any(.dashboard.panels[].targets[]?; .expr == "boxferry_fixture_temperature_celsius{source=\"controlled\"}") and
    any(.dashboard.panels[].targets[]?; .expr == "{job=\"boxferry_fixture\"} |= \"boxferry-observability-known-log\"")
  ' <<< "${dashboard}" > /dev/null
}

observability_probe_published_grafana() {
  local socket=$1 prefix=$2 response
  response="$(observability_remote "${socket}" run --rm --pull=never --network host \
    "$(observability_image_reference producer)" \
    wget -qO- "http://127.0.0.1:${OBSERVABILITY_HTTP_PORT}/api/health")"
  jq --exit-status '.database == "ok" and .version == "13.2.1"' \
    <<< "${response}" > /dev/null
  observability_remote "${socket}" inspect "${prefix}-observability-grafana" |
    jq --exit-status --arg port "${OBSERVABILITY_HTTP_PORT}" '
      .[0].HostConfig.PortBindings["3000/tcp"][0].HostIp == "127.0.0.1" and
      .[0].HostConfig.PortBindings["3000/tcp"][0].HostPort == $port
    ' > /dev/null
}

observability_assert_application_boundaries() {
  local socket=$1 prefix=$2 container inspect_file
  inspect_file="${current_case}/observability-private-containers.inspect.json"
  observability_remote "${socket}" inspect \
    "${prefix}-observability-metrics-producer" \
    "${prefix}-observability-log-producer" \
    "${prefix}-observability-prometheus" \
    "${prefix}-observability-loki" \
    "${prefix}-observability-alloy" > "${inspect_file}"
  jq --exit-status --arg backend "${prefix}-observability-backend" '
    all(.[];
      (.NetworkSettings.Networks | keys) == [$backend] and
      (.HostConfig.PortBindings == {} or .HostConfig.PortBindings == null) and
      all(.Mounts[]?;
        (.Source | test("(podman|docker)\\.sock$") | not) and
        (.Source | startswith("/run/") | not) and
        (.Source | startswith("/var/run/") | not)
      )
    )
  ' "${inspect_file}" > /dev/null
  observability_remote "${socket}" network inspect "${prefix}-observability-backend" |
    jq --exit-status '.[0].internal == true or .[0].Internal == true' > /dev/null
  observability_remote "${socket}" inspect "${prefix}-observability-boundary-peer" |
    jq --exit-status \
      --arg backend "${prefix}-observability-backend" \
      --arg edge "${prefix}-observability-edge" '
      .[0].NetworkSettings.Networks[$backend] == null and
      .[0].NetworkSettings.Networks[$edge] != null
    ' > /dev/null
  for container in metrics-producer log-producer prometheus loki alloy; do
    observability_remote "${socket}" inspect \
      "${prefix}-observability-${container}" |
      jq --exit-status --arg edge "${prefix}-observability-edge" \
        '.[0].NetworkSettings.Networks[$edge] == null' > /dev/null
  done
}

observability_assert_storage_ownership() {
  local socket=$1 prefix=$2 container volume target writable inspect_file
  while IFS=':' read -r container volume target writable; do
    inspect_file="${current_case}/${prefix}-${container}-${volume}.mount.json"
    observability_remote "${socket}" inspect "${prefix}-observability-${container}" \
      > "${inspect_file}"
    jq --exit-status --arg volume "${prefix}-observability-${volume}" \
      --arg target "${target}" --argjson writable "${writable}" '
      any(.[0].Mounts[]?; .Name == $volume and .Destination == $target and .RW == $writable)
    ' "${inspect_file}" > /dev/null
  done << 'EOF'
prometheus:prometheus-data:/prometheus:true
loki:loki-data:/loki:true
grafana:grafana-data:/var/lib/grafana:true
alloy:alloy-data:/var/lib/alloy/data:true
log-producer:telemetry-logs:/var/log/boxferry:true
alloy:telemetry-logs:/var/log/boxferry:false
EOF

  for volume in prometheus-data loki-data grafana-data alloy-data; do
    observability_remote "${socket}" inspect \
      "${prefix}-observability-metrics-producer" \
      "${prefix}-observability-log-producer" \
      "${prefix}-observability-prometheus" \
      "${prefix}-observability-loki" \
      "${prefix}-observability-alloy" \
      "${prefix}-observability-grafana" |
      jq --exit-status --arg volume "${prefix}-observability-${volume}" \
        '[.[] | select(any(.Mounts[]?; .Name == $volume))] | length == 1' > /dev/null
  done
}

observability_prepare_persistence_sentinels() {
  local socket=$1 prefix=$2
  observability_remote "${socket}" exec "${prefix}-observability-grafana" \
    /bin/sh -ceu 'printf "%s\n" boxferry-grafana-persisted > /var/lib/grafana/boxferry-persistence-marker'
  observability_remote "${socket}" exec "${prefix}-observability-metrics-producer" \
    sed -i 's/ 42$/ 84/' /www/metrics
  observability_wait_for 60 'Prometheus historical sentinel value' \
    observability_prometheus_has_value "${socket}" "${prefix}" \
    'boxferry_fixture_temperature_celsius%7Bsource%3D%22controlled%22%7D' 84
  observability_remote "${socket}" exec "${prefix}-observability-metrics-producer" \
    sed -i 's/ 84$/ 42/' /www/metrics
  observability_wait_for 60 'Prometheus current baseline value' \
    observability_prometheus_has_value "${socket}" "${prefix}" \
    'boxferry_fixture_temperature_celsius%7Bsource%3D%22controlled%22%7D' 42
  observability_loki_has_known_log "${socket}" "${prefix}"
}

observability_recreate_application() {
  local mode=$1 socket=$2 prefix=$3 run=$4
  if [[ "${mode}" == compose ]]; then
    observability_compose_project "${socket}" "${prefix}" "${run}" \
      stop --timeout 30 > "${current_case}/observability-compose-stop.log" 2>&1
    observability_compose_project "${socket}" "${prefix}" "${run}" \
      rm --force >> "${current_case}/observability-compose-stop.log" 2>&1
    observability_compose_project "${socket}" "${prefix}" "${run}" \
      up --detach --remove-orphans > "${current_case}/observability-compose-recreate.log" 2>&1
  else
    observability_remote "${socket}" stop --time 30 \
      "${prefix}-observability-grafana" "${prefix}-observability-alloy" \
      "${prefix}-observability-loki" "${prefix}-observability-prometheus" \
      "${prefix}-observability-log-producer" \
      "${prefix}-observability-metrics-producer" > /dev/null
    observability_remote "${socket}" rm --force \
      "${prefix}-observability-grafana" "${prefix}-observability-alloy" \
      "${prefix}-observability-loki" "${prefix}-observability-prometheus" \
      "${prefix}-observability-log-producer" \
      "${prefix}-observability-metrics-producer" > /dev/null
    observability_create_cli_runtime "${socket}" "${prefix}" "${run}"
  fi
  observability_wait_application "${socket}" "${prefix}"
}

observability_assert_persistence() {
  local socket=$1 prefix=$2
  observability_prometheus_has_value "${socket}" "${prefix}" \
    'max_over_time%28boxferry_fixture_temperature_celsius%7Bsource%3D%22controlled%22%7D%5B30m%5D%29' 84
  observability_loki_has_known_log "${socket}" "${prefix}"
  observability_remote "${socket}" exec "${prefix}-observability-grafana" \
    grep --fixed-strings --line-regexp boxferry-grafana-persisted \
    /var/lib/grafana/boxferry-persistence-marker > /dev/null
  [[ "$(observability_remote "${socket}" exec \
    "${prefix}-observability-log-producer" wc -l /var/log/boxferry/telemetry.log | awk '{ print $1 }')" == 1 ]]
}

observability_assert_output_membership() {
  local selection=$1 output=$2 directory=$3 prefix=$4 service volume
  for service in metrics-producer log-producer prometheus loki alloy grafana; do
    assert_named_member "${output}" "${directory}" "${service}" \
      "${prefix}-observability-${service}"
  done
  if [[ "${selection}" == all ]]; then
    assert_named_member "${output}" "${directory}" boundary-peer \
      "${prefix}-observability-boundary-peer"
  else
    assert_named_absent "${output}" "${directory}" boundary-peer \
      "${prefix}-observability-boundary-peer"
  fi
  assert_resource_member "${output}" "${directory}" network \
    "${prefix}-observability-backend"
  assert_resource_member "${output}" "${directory}" network \
    "${prefix}-observability-edge"
  for volume in alloy-data grafana-data loki-data prometheus-data telemetry-logs; do
    assert_resource_member "${output}" "${directory}" volume \
      "${prefix}-observability-${volume}"
  done
}

observability_assert_external_edge() {
  local output=$1 directory=$2 edge=$3
  case "${output}" in
    compose)
      grep --fixed-strings --quiet "name: ${edge}" "${directory}/compose.yaml"
      grep --fixed-strings --quiet 'external: true' "${directory}/compose.yaml"
      ;;
    quadlet)
      [[ ! -e "${directory}/${edge}.network" ]]
      grep --recursive --fixed-strings --quiet "Network=${edge}" "${directory}"
      ;;
    podman)
      jq --exit-status --arg edge "${edge}" '
        any(.external_preconditions[]?; .kind == "network" and .name == $edge)
      ' "${directory}/podman.json" > /dev/null
      ;;
  esac
}

observability_write_expected_diagnostics() {
  local selection=$1 source_kind=$2 output=$3 resource_prefix=$4 live_bindings=$5 destination=$6
  local fixture diagnostics losses route service bind_count index
  fixture="${repository_root}/fixtures/scenarios/observability-application"
  route="${source_kind}-${output}"
  case "${route}" in
    podman-compose | podman-quadlet | podman-podman | compose-podman | quadlet-podman) ;;
    compose-compose | compose-quadlet | quadlet-compose | quadlet-quadlet)
      : > "${destination}"
      return
      ;;
    *) return 2 ;;
  esac
  diagnostics="${fixture}/expected.${route}.diagnostics"
  losses="${fixture}/expected.${route}.losses.tsv"
  : > "${destination}"

  awk -F '|' -v prefix="${resource_prefix}" -v live="${live_bindings}" '
    function mount_offset(service) {
      if (live != "true") return 0
      if (service == "grafana") return 3
      if (service == "alloy" || service == "log-producer" ||
          service == "loki" || service == "prometheus") return 1
      return 0
    }
    function qualify(subject, rest, parts, service, mount_index) {
      if (live == "true" &&
          subject ~ /^services\.[^.]+\.mounts\[[0-9]+\]$/) {
        rest = substr(subject, length("services.") + 1)
        split(rest, parts, ".")
        service = parts[1]
        mount_index = parts[2]
        sub(/^mounts\[/, "", mount_index)
        sub(/\]$/, "", mount_index)
        return "services." prefix service ".mounts[" \
          (mount_index + mount_offset(service)) "]"
      }
      if (subject ~ /^container:/) {
        return "container:" prefix substr(subject, length("container:") + 1)
      }
      if (subject ~ /^volume:/) {
        return "volume:" prefix substr(subject, length("volume:") + 1)
      }
      if (subject ~ /^services\./) {
        return "services." prefix substr(subject, length("services.") + 1)
      }
      if (subject ~ /^networks\./) {
        return "networks." prefix substr(subject, length("networks.") + 1)
      }
      if (subject ~ /^volumes\./) {
        return "volumes." prefix substr(subject, length("volumes.") + 1)
      }
      return subject
    }
    $1 == "BFP0002" {
      print $1 "\t" qualify($2) "\twarning\tomitted\tpartial"
    }
  ' "${diagnostics}" >> "${destination}"

  awk -F '\t' -v prefix="${resource_prefix}" -v live="${live_bindings}" '
    function mount_offset(service) {
      if (live != "true") return 0
      if (service == "grafana") return 3
      if (service == "alloy" || service == "log-producer" ||
          service == "loki" || service == "prometheus") return 1
      return 0
    }
    function qualify(subject, rest, parts, service, mount_index) {
      if (live == "true" &&
          subject ~ /^services\.[^.]+\.mounts\[[0-9]+\]$/) {
        rest = substr(subject, length("services.") + 1)
        split(rest, parts, ".")
        service = parts[1]
        mount_index = parts[2]
        sub(/^mounts\[/, "", mount_index)
        sub(/\]$/, "", mount_index)
        return "services." prefix service ".mounts[" \
          (mount_index + mount_offset(service)) "]"
      }
      if (subject ~ /^services\./) {
        return "services." prefix substr(subject, length("services.") + 1)
      }
      if (subject ~ /^networks\./) {
        return "networks." prefix substr(subject, length("networks.") + 1)
      }
      if (subject ~ /^volumes\./) {
        return "volumes." prefix substr(subject, length("volumes.") + 1)
      }
      return subject
    }
    $1 == "BFP0003" || $1 == "BFP0007" {
      if ($1 == "BFP0007") {
        decision = "omitted"
        policy = "partial"
      } else if ($3 == "approximate") {
        decision = "approximated"
        policy = "approximate"
      } else {
        decision = "not-promoted"
        policy = "partial"
      }
      print $1 "\t" qualify($2) "\twarning\t" decision "\t" policy
    }
  ' "${losses}" >> "${destination}"

  if [[ "${live_bindings}" == true ]]; then
    while read -r service bind_count; do
      for ((index = 0; index < bind_count; index++)); do
        printf 'BFP0003\tservices.%s%s.mounts[%d]\twarning\tnot-promoted\tpartial\n' \
          "${resource_prefix}" "${service}" "${index}" >> "${destination}"
      done
    done << 'EOF'
alloy 1
grafana 3
log-producer 1
loki 1
metrics-producer 1
prometheus 1
EOF

    if [[ "${selection}" == all ]]; then
      printf '%s\n' \
        "BFP0002\tcontainer:${resource_prefix}boundary-peer\twarning\tomitted\tpartial" \
        "BFP0003\tservices.${resource_prefix}boundary-peer.namespaces\twarning\tnot-promoted\tpartial" \
        "BFP0003\tservices.${resource_prefix}boundary-peer.networks\twarning\tapproximated\tapproximate" \
        "BFP0003\tservices.${resource_prefix}boundary-peer.resource_controls\twarning\tnot-promoted\tpartial" \
        "BFP0003\tservices.${resource_prefix}boundary-peer.security\twarning\tnot-promoted\tpartial" \
        >> "${destination}"
    fi
  fi
}

observability_assert_reviewed_diagnostics() {
  local selection=$1 source_kind=$2 output=$3 resource_prefix=$4 live_bindings=$5 report=$6
  local expected="${report}.diagnostics.expected.tsv"
  local observed="${report}.diagnostics.observed.tsv"
  observability_write_expected_diagnostics \
    "${selection}" "${source_kind}" "${output}" "${resource_prefix}" \
    "${live_bindings}" "${expected}"
  jq --raw-output '
    def field($name): [.fields[]? | select(.name == $name) | .value] | first // "";
    .diagnostics[]?
    | [.code, field("subject"), .severity, field("decision"), field("required_loss_policy")]
    | @tsv
  ' "${report}" > "${observed}"
  sort --output="${expected}" "${expected}"
  sort --output="${observed}" "${observed}"
  diff --unified "${expected}" "${observed}"
}

observability_assert_output_semantics() {
  local selection=$1 source_kind=$2 output=$3 directory=$4 prefix=$5 report=$6 id
  local live_bindings=false
  [[ "${source_kind}" == podman ]] && live_bindings=true
  for id in prometheus loki grafana alloy producer; do
    grep --recursive --fixed-strings --quiet \
      "$(observability_image_reference "${id}")" "${directory}"
  done
  if [[ "${output}" == podman ]]; then
    jq --exit-status 'any(.output_artifacts[]?; .name == "podman.json")' \
      "${report}" > /dev/null
    if grep --recursive --fixed-strings --quiet \
      "${OBSERVABILITY_ADMIN_PASSWORD}" "${directory}"; then
      printf 'Observability Podman plan retained protected environment value.\n' >&2
      return 1
    fi
  else
    grep --recursive --fixed-strings --quiet '/prometheus' "${directory}"
    grep --recursive --fixed-strings --quiet '/loki' "${directory}"
    grep --recursive --fixed-strings --quiet '/var/lib/grafana' "${directory}"
    grep --recursive --fixed-strings --quiet \
      "${OBSERVABILITY_ADMIN_PASSWORD}" "${directory}"
    if [[ "${output}" == compose ]]; then
      grep --fixed-strings --quiet 'host_ip: 127.0.0.1' "${directory}/compose.yaml"
      grep --fixed-strings --quiet "published: \"${OBSERVABILITY_HTTP_PORT}\"" \
        "${directory}/compose.yaml"
      grep --fixed-strings --quiet 'target: 3000' "${directory}/compose.yaml"
    else
      grep --recursive --extended-regexp --quiet \
        "^PublishPort=127\\.0\\.0\\.1:${OBSERVABILITY_HTTP_PORT}:3000(/tcp)?$" \
        "${directory}"
    fi
  fi
  observability_assert_reviewed_diagnostics \
    "${selection}" "${source_kind}" "${output}" "${prefix}-observability-" \
    "${live_bindings}" "${report}"
  grep --recursive --fixed-strings --quiet -- \
    '--storage.tsdb.retention.time=24h' "${directory}"
}
observability_report_conversion_failure() {
  local report=$1
  if [[ -s "${report}" ]]; then
    printf 'Observability conversion report %s:\n' "${report}" >&2
    sed -n '1,240p' "${report}" >&2
  fi
}

observability_run_reimports() {
  local mode=$1 selection=$2 source=$3 prefix=$4 input output result report file
  for input in compose quadlet; do
    for output in compose quadlet podman; do
      result="${current_case}/reimports/${mode}-${selection}-${input}-to-${output}"
      report="${result}.report.json"
      local -a command=(
        "${boxferry_bin:?caller must supply boxferry_bin}" convert "${input}" "${output}"
        --loss-policy partial
      )
      if [[ "${input}" == compose ]]; then
        command+=(
          --project-name "${prefix}-observability"
          --input-file "${source}/${input}/compose.yaml"
        )
      else
        command+=(--application-name "${prefix}-observability")
        while IFS= read -r -d '' file; do
          command+=(--input-file "${file}")
        done < <(find "${source}/${input}" -maxdepth 1 -type f -print0 | sort -z)
      fi
      [[ "${output}" == podman ]] && command+=(--podman-target-context rootless)
      command+=(--output-directory "${result}" --console-format json)
      timed_operation 90s \
        "Observability ${mode} ${selection} ${input}-to-${output} reimport" \
        "${command[@]}" > "${report}"
      jq --exit-status '
        .schema_version == 1 and .status == "success" and .exit_category == "success" and
        ([.diagnostics[]? | select(.severity == "error")] | length == 0) and
        (.output_artifacts | length > 0)
      ' "${report}" > /dev/null
      observability_assert_output_membership "${selection}" "${output}" \
        "${result}" "${prefix}"
      observability_assert_output_semantics \
        "${selection}" "${input}" "${output}" "${result}" "${prefix}" "${report}"
      if [[ "${selection}" != all ]]; then
        observability_assert_external_edge "${output}" "${result}" \
          "${prefix}-observability-edge"
      fi
    done
  done
}

observability_run_exports() {
  local mode=$1 socket=$2 prefix=$3 selection output directory report
  local -a selection_arguments=()
  mkdir -p -- "${current_case}/outputs" "${current_case}/reimports"
  for selection in exact label all; do
    case "${selection}" in
      exact)
        selection_arguments=(--podman-resource "container=${prefix}-observability-grafana")
        ;;
      label)
        selection_arguments=(--podman-label "io.boxferry.application=${prefix}-observability")
        ;;
      all)
        selection_arguments=(--podman-all)
        ;;
    esac
    for output in compose quadlet podman; do
      directory="${current_case}/outputs/${mode}-${selection}/${output}"
      report="${directory}.report.json"
      local -a target_arguments=()
      [[ "${output}" == podman ]] && target_arguments+=(--podman-target-context rootless)
      if ! boxferry_operation "Observability ${mode} ${selection} Podman-to-${output}" \
        convert podman "${output}" --podman-socket "${socket}" \
        --application-name "${prefix}-observability" --loss-policy partial \
        --promote-podman-effective-named-volumes \
        --promote-podman-effective-named-networks \
        --promote-podman-portable-effective-settings \
        --output-directory "${directory}" --console-format json \
        "${target_arguments[@]}" "${selection_arguments[@]}" > "${report}"; then
        observability_report_conversion_failure "${report}"
        return 1
      fi
      jq --exit-status '
        .schema_version == 1 and .status == "success" and .exit_category == "success" and
        ([.diagnostics[]? | select(.severity == "error")] | length == 0) and
        ((.fidelity.invalid // 0) == 0) and (.output_artifacts | length > 0)
      ' "${report}" > /dev/null
      observability_assert_output_membership "${selection}" "${output}" \
        "${directory}" "${prefix}"
      observability_assert_output_semantics \
        "${selection}" podman "${output}" "${directory}" "${prefix}" "${report}"
      if [[ "${selection}" != all ]]; then
        observability_assert_external_edge "${output}" "${directory}" \
          "${prefix}-observability-edge"
      fi
    done
    observability_run_reimports "${mode}" "${selection}" \
      "${current_case}/outputs/${mode}-${selection}" "${prefix}"
  done
  if grep --recursive --include='*.report.json' --fixed-strings --quiet \
    "${OBSERVABILITY_ADMIN_PASSWORD}" "${current_case}/outputs" "${current_case}/reimports"; then
    printf 'Observability conversion report leaked protected-value canary.\n' >&2
    return 1
  fi
}

observability_cleanup_mode() {
  local mode=$1 socket=$2 prefix=$3 run=$4
  if [[ "${mode}" == compose ]]; then
    observability_compose_project "${socket}" "${prefix}" "${run}" \
      down --volumes --remove-orphans > "${current_case}/observability-compose-down.log" 2>&1 || true
  fi
  observability_remote "${socket}" rm --force --time 0 --ignore \
    "${prefix}-observability-grafana" "${prefix}-observability-alloy" \
    "${prefix}-observability-loki" "${prefix}-observability-prometheus" \
    "${prefix}-observability-log-producer" "${prefix}-observability-metrics-producer" \
    "${prefix}-observability-boundary-peer" > /dev/null 2>&1 || true
  observability_remote "${socket}" volume rm --force \
    "${prefix}-observability-alloy-data" "${prefix}-observability-grafana-data" \
    "${prefix}-observability-loki-data" "${prefix}-observability-prometheus-data" \
    "${prefix}-observability-telemetry-logs" > /dev/null 2>&1 || true
  observability_remote "${socket}" network rm \
    "${prefix}-observability-backend" "${prefix}-observability-edge" \
    > /dev/null 2>&1 || true
  observability_assert_clean_prefix "${socket}" "${prefix}"
}

observability_verify_target() {
  local id=$1 image=$2 declared_version=$3 distribution=$4 mode=$5 lane=$6 architecture=$7
  verify_observed_version "${id}" "${declared_version}" "${artifact_root}/${id}.podman-version"
  [[ "$(< "${artifact_root}/${id}.digest")" == "${image##*@}" ]] || {
    printf 'Pulled image digest does not match reviewed observability target %s.\n' "${id}" >&2
    return 1
  }
  [[ "${architecture}" == amd64 &&
    "$(< "${artifact_root}/${id}.architecture")" =~ ^(x86_64|amd64)$ ]] || {
    printf 'Observed architecture does not match observability target %s.\n' "${id}" >&2
    return 1
  }
  append_verified_evidence "${id}" "${image}" "${declared_version}" "${distribution}" \
    "${mode}" "${lane}" "${architecture}" "${artifact_root}/${id}" \
    nested-image observability-application
}

run_observability_application_cell() {
  local id=$1 image=$2 declared_version=$3 distribution=$4 mode=$5 lane=$6 architecture=$7
  [[ "${id}-${mode}" == podman-6.1-rootless-rootless ]] || {
    printf 'Observability profile is bounded to reviewed Podman 6.1 rootless.\n' >&2
    return 1
  }

  current_prefix="${run_id:0:28}-obs"
  current_case="${artifact_root}/${id}-observability-application"
  local socket_directory="${runtime_root}/observability-application-target"
  local application_archive="${runtime_root}/observability-application-images.tar"
  mkdir -p -- "${current_case}" "${socket_directory}"
  chmod 0700 "${current_case}" "${socket_directory}"
  progress_index=0
  progress_total=35
  printf '%s PLAN %s observability-application tests=%d\n' \
    "$(timestamp)" "${id}" "${progress_total}"

  progress_run 'verify observability host resource budget' observability_validate_resource_budget
  progress_run 'validate observability image and provider catalogues' observability_validate_catalogues
  progress_run 'validate Docker Compose provider' observability_validate_provider
  progress_run 'verify deterministic telemetry producer' observability_verify_producer
  progress_run 'prepare bounded digest-pinned observability image archive' \
    observability_prepare_image_archive "${application_archive}"
  progress_run 'start isolated observability Podman target' \
    start_outer_runtime "${id}" "${image}" "${mode}" "${socket_directory}" \
    "${application_archive}"
  progress_run 'verify observability target evidence' \
    observability_verify_target "${id}" "${image}" "${declared_version}" \
    "${distribution}" "${mode}" "${lane}" "${architecture}"
  local outer="${started_outer}"
  progress_run 'load images and copy reviewed observability configuration' \
    observability_prepare_application_target "${outer}" "${current_prefix}" "${socket_directory}"
  local socket="${socket_directory}/podman.sock"

  progress_run 'provision independent Podman CLI observability application' \
    observability_provision_cli "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'wait for CLI metric and log ingestion' \
    observability_wait_application "${socket}" "${current_prefix}"
  progress_run 'query CLI Prometheus Loki and Grafana provisioning' \
    observability_assert_queries_and_grafana "${socket}" "${current_prefix}"
  progress_run 'prove CLI Grafana loopback ingress' \
    observability_probe_published_grafana "${socket}" "${current_prefix}"
  progress_run 'prove CLI private collector and shared edge boundaries' \
    observability_assert_application_boundaries "${socket}" "${current_prefix}"
  progress_run 'prove CLI volume ownership and shared read-only log handoff' \
    observability_assert_storage_ownership "${socket}" "${current_prefix}"
  progress_run 'exercise CLI exact label all exports and reimports' \
    observability_run_exports cli "${socket}" "${current_prefix}"
  progress_run 'refuse colliding CLI observability prefix' \
    observability_expect_collision "${socket}" "${current_prefix}"
  progress_run 'prepare CLI historical persistence sentinels' \
    observability_prepare_persistence_sentinels "${socket}" "${current_prefix}"
  progress_run 'recreate CLI containers without deleting volumes' \
    observability_recreate_application cli "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'prove CLI metric log Grafana persistence' \
    observability_assert_persistence "${socket}" "${current_prefix}"
  progress_run 'revalidate CLI queries after recreation' \
    observability_assert_queries_and_grafana "${socket}" "${current_prefix}"
  progress_run 'clean prefix-scoped Podman CLI observability resources' \
    observability_cleanup_mode cli "${socket}" "${current_prefix}" "${run_id}"

  progress_run 'provision independent Docker Compose observability application' \
    observability_provision_compose "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'wait for Compose metric and log ingestion' \
    observability_wait_application "${socket}" "${current_prefix}"
  progress_run 'query Compose Prometheus Loki and Grafana provisioning' \
    observability_assert_queries_and_grafana "${socket}" "${current_prefix}"
  progress_run 'prove Compose Grafana loopback ingress' \
    observability_probe_published_grafana "${socket}" "${current_prefix}"
  progress_run 'prove Compose private collector and shared edge boundaries' \
    observability_assert_application_boundaries "${socket}" "${current_prefix}"
  progress_run 'prove Compose volume ownership and shared read-only log handoff' \
    observability_assert_storage_ownership "${socket}" "${current_prefix}"
  progress_run 'exercise Compose exact label all exports and reimports' \
    observability_run_exports compose "${socket}" "${current_prefix}"
  progress_run 'refuse colliding Compose observability prefix' \
    observability_expect_collision "${socket}" "${current_prefix}"
  progress_run 'prepare Compose historical persistence sentinels' \
    observability_prepare_persistence_sentinels "${socket}" "${current_prefix}"
  progress_run 'recreate Compose containers without deleting volumes' \
    observability_recreate_application compose "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'prove Compose metric log Grafana persistence' \
    observability_assert_persistence "${socket}" "${current_prefix}"
  progress_run 'revalidate Compose queries after recreation' \
    observability_assert_queries_and_grafana "${socket}" "${current_prefix}"
  progress_run 'clean prefix-scoped Docker Compose observability resources' \
    observability_cleanup_mode compose "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'remove disposable observability outer container' remove_outer "${outer}"

  printf '%s CELL PASS %s observability-application (%d/%d tests)\n' \
    "$(timestamp)" "${id}" "${progress_index}" "${progress_total}"
}
