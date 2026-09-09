#!/usr/bin/env bash
# Immich application acceptance helpers. Sourced by the live entry point.
# shellcheck disable=SC2016,SC2129,SC2154 # Remote expansion and phased logs are intentional.

readonly IMMICH_ADMIN_EMAIL="boxferry@example.invalid"
readonly IMMICH_ADMIN_PASSWORD="boxferry-public-admin-canary"
readonly IMMICH_DB_PASSWORD="boxferry-public-immich-db-password-canary"
readonly IMMICH_PROVIDER_VERSION="5.5.0"
readonly IMMICH_PROVIDER_SHA256="c57ab918abd5b05ca7e7d0f275875dd1330a695074f309dc9eab1b49efafcd4b"
readonly IMMICH_HTTP_PORT="18283"
readonly IMMICH_ARCHIVE_MAX_BYTES="2684354560"
readonly IMMICH_MIN_CPUS="2"
readonly IMMICH_MIN_MEMORY_KIB="8388608"
readonly IMMICH_MIN_DISK_KIB="12582912"

immich_fixture_root() {
  printf '%s/fixtures/conformance/immich-application\n' \
    "${repository_root:?caller must supply repository_root}"
}

immich_image_reference() {
  local id=$1
  printf 'registry.invalid/boxferry-test/immich-application:%s\n' "${id}"
}

immich_validate_resource_budget() {
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

  [[ "${cpus}" =~ ^[0-9]+$ && "${cpus}" -ge "${IMMICH_MIN_CPUS}" ]] || {
    printf 'Immich profile requires at least %s CPUs; found %s.\n' \
      "${IMMICH_MIN_CPUS}" "${cpus:-unknown}" >&2
    return 1
  }
  [[ "${memory_kib}" =~ ^[0-9]+$ && "${memory_kib}" -ge "${IMMICH_MIN_MEMORY_KIB}" ]] || {
    printf 'Immich profile requires at least 8 GiB available memory; found %s KiB.\n' \
      "${memory_kib:-unknown}" >&2
    return 1
  }
  [[ "${disk_kib}" =~ ^[0-9]+$ && "${disk_kib}" -ge "${IMMICH_MIN_DISK_KIB}" ]] || {
    printf 'Immich profile requires at least 12 GiB free disk on archive and graph-root filesystems; found %s KiB.\n' \
      "${disk_kib:-unknown}" >&2
    return 1
  }
  printf '%s IMMICH BUDGET cpus=%s memory-available-kib=%s disk-available-kib=%s\n' \
    "$(timestamp)" "${cpus}" "${memory_kib}" "${disk_kib}"
}

immich_validate_catalogues() {
  local fixture file
  fixture="$(immich_fixture_root)"
  awk -F '\t' '
    NF && $1 !~ /^#/ {
      if (NF != 6 || $2 !~ /:[^\/@]+@sha256:[0-9a-f]{64}$/ || $3 == "" ||
          $4 == "" || $5 !~ /^https:\/\// || $6 != "transient-test-pull" || seen[$1]++) {
        printf "Invalid Immich image row at line %d: %s\\n", NR, $0 > "/dev/stderr"
        bad = 1
      }
      count++
    }
    END { exit bad || count != 4 }
  ' "${fixture}/images.tsv"
  awk -F '\t' -v version="${IMMICH_PROVIDER_VERSION}" -v sha="${IMMICH_PROVIDER_SHA256}" '
    NF && $1 !~ /^#/ {
      if (NF != 7 || $1 != "docker-compose" || $2 != version ||
          $3 != "https://github.com/docker/compose/releases/download/v5.5.0/docker-compose-linux-x86_64" ||
          $4 != sha || $5 != "Apache-2.0" || $6 != "https://github.com/docker/compose" ||
          $7 != "downloaded-test-tool") {
        printf "Invalid Immich provider row at line %d: %s\\n", NR, $0 > "/dev/stderr"
        bad = 1
      }
      count++
    }
    END { exit bad || count != 1 }
  ' "${fixture}/providers.tsv"
  for file in compose.yaml media-probe.py images.tsv providers.tsv README.md; do
    [[ -s "${fixture}/${file}" ]]
  done
}

immich_validate_provider() {
  local provider=${BOXFERRY_COMPOSE_BIN:-${repository_root}/target/tools/docker-compose}
  local observed_sha observed_version
  [[ -x "${provider}" ]] || {
    printf 'Immich profile requires executable Docker Compose %s at %s.\n' \
      "${IMMICH_PROVIDER_VERSION}" "${provider}" >&2
    return 1
  }
  observed_sha="$(sha256sum "${provider}" | awk '{ print $1 }')"
  [[ "${observed_sha}" == "${IMMICH_PROVIDER_SHA256}" ]] || {
    printf 'Docker Compose checksum mismatch: expected %s, observed %s.\n' \
      "${IMMICH_PROVIDER_SHA256}" "${observed_sha}" >&2
    return 1
  }
  observed_version="$("${provider}" version --short | sed 's/^v//')"
  [[ "${observed_version}" == "${IMMICH_PROVIDER_VERSION}" ]] || {
    printf 'Docker Compose version mismatch: expected %s, observed %s.\n' \
      "${IMMICH_PROVIDER_VERSION}" "${observed_version}" >&2
    return 1
  }
}

immich_verify_media_generator() {
  local temporary status=0
  temporary="$(mktemp -d /tmp/boxferry-immich-media.XXXXXX)"
  python3 "$(immich_fixture_root)/media-probe.py" self-test \
    --work-dir "${temporary}" || status=$?
  rm -rf -- "${temporary}"
  return "${status}"
}

immich_prepare_image_archive() {
  local archive=${1:?archive path required}
  if [[ -s "${archive}" ]]; then
    [[ "$(stat -c '%s' "${archive}")" -le "${IMMICH_ARCHIVE_MAX_BYTES}" ]] || {
      printf 'Existing Immich image archive exceeds 2.5 GiB cap.\n' >&2
      return 1
    }
    return 0
  fi

  local fixture id reference expected_digest observed_digest cache_status runtime_reference
  local archive_directory archive_size
  fixture="$(immich_fixture_root)"
  archive_directory="$(mktemp -d "${runtime_root}/immich-image-archives.XXXXXX")"
  while IFS=$'\t' read -r id reference _ _ _ _; do
    [[ -z "${id}" || "${id}" == \#* ]] && continue
    expected_digest="${reference##*@}"
    cache_status=0
    engine_image_available "probe Immich ${id} image cache" "${reference}" || cache_status=$?
    if ((cache_status == 1)); then
      timed_operation 8m "pull digest-pinned Immich ${id} image" \
        "${engine}" pull --quiet "${reference}" \
        > "${artifact_root}/immich-${id}.pull.log"
    elif ((cache_status != 0)); then
      return "${cache_status}"
    fi
    observed_digest="$(engine_operation "inspect Immich ${id} image digest" \
      image inspect --format '{{.Digest}}' "${reference}")"
    [[ "${observed_digest}" == "${expected_digest}" ]] || {
      printf 'Immich image digest mismatch %s: expected %s, observed %s.\n' \
        "${id}" "${expected_digest}" "${observed_digest}" >&2
      return 1
    }
    runtime_reference="$(immich_image_reference "${id}")"
    engine_operation "tag reviewed Immich ${id} image for nested archive" \
      tag "${reference}" "${runtime_reference}"
    timed_operation 12m "archive Immich ${id} image" \
      "${engine}" save --format oci-archive \
      --output "${archive_directory}/${id}.oci.tar" "${runtime_reference}"
  done < "${fixture}/images.tsv"
  timed_operation 12m 'bundle digest-pinned Immich image archives' \
    tar --create --file "${archive}" --directory "${archive_directory}" .
  rm -rf -- "${archive_directory}"
  chmod 0644 "${archive}"
  archive_size="$(stat -c '%s' "${archive}")"
  [[ "${archive_size}" -le "${IMMICH_ARCHIVE_MAX_BYTES}" ]] || {
    printf 'Immich image archive exceeds 2.5 GiB cap: %s bytes.\n' "${archive_size}" >&2
    return 1
  }
}

immich_assert_loaded_images() {
  local outer=$1 fixture id
  fixture="$(immich_fixture_root)"
  while IFS=$'\t' read -r id _ _ _ _ _; do
    [[ -z "${id}" || "${id}" == \#* ]] && continue
    engine_operation "verify loaded Immich ${id} image" \
      exec "${outer}" podman image exists "$(immich_image_reference "${id}")"
  done < "${fixture}/images.tsv"
}

immich_prepare_application_target() {
  local outer=$1 prefix=$2 socket_directory=$3 fixture destination
  engine_operation 'copy rootless Immich network configuration' cp \
    "${repository_root}/fixtures/conformance/podman-live/apply-target-containers.conf" \
    "${outer}:/tmp/99-boxferry-live.conf"
  # shellcheck disable=SC2016 # $HOME expands inside the nested target.
  engine_operation 'prepare rootless Immich network configuration' \
    exec "${outer}" /bin/sh -ceu \
    'mkdir -p "$HOME/.config/containers/containers.conf.d"; cp /tmp/99-boxferry-live.conf "$HOME/.config/containers/containers.conf.d/99-boxferry-live.conf"'
  # shellcheck disable=SC2016 # Archive loop expands inside the nested target.
  timed_operation 18m 'load digest-pinned Immich application archives' \
    "${engine}" exec "${outer}" /bin/sh -ceu '
      directory=/tmp/boxferry-immich-images
      mkdir -p "$directory"
      tar -xf /boxferry-workload.tar -C "$directory"
      for archive in "$directory"/*.oci.tar; do podman load --input "$archive"; done
    ' > /dev/null
  immich_assert_loaded_images "${outer}"
  fixture="$(immich_fixture_root)"
  destination="/tmp/boxferry-fixture/${prefix}"
  engine_operation 'create disposable Immich fixture directory' \
    exec "${outer}" mkdir -p -- "${destination}"
  engine_operation 'copy reviewed Immich media probe' \
    cp "${fixture}/media-probe.py" "${outer}:${destination}/media-probe.py"
  activate_outer_runtime "${socket_directory}"
}

immich_remote() {
  local socket=$1
  shift
  podman_socket "${socket}" "Immich ${1:-command}" "$@"
}

immich_wait_for() {
  local deadline_seconds=$1 description=$2
  shift 2
  local deadline=$((SECONDS + deadline_seconds))
  until "$@" > /dev/null 2>&1; do
    if ((SECONDS >= deadline)); then
      printf 'Timed out after %ss waiting for Immich %s.\n' "${deadline_seconds}" "${description}" >&2
      return 1
    fi
    sleep 2
  done
}

immich_assert_clean_prefix() {
  local socket=$1 prefix=$2
  local listing
  local -a collisions=()
  listing="$({
    immich_remote "${socket}" ps -a --format '{{.Names}}' || exit $?
    immich_remote "${socket}" volume ls --format '{{.Name}}' || exit $?
    immich_remote "${socket}" network ls --format '{{.Name}}'
  })" || {
    printf 'Could not inspect existing resources for Immich prefix %s.\n' "${prefix}" >&2
    return 2
  }
  mapfile -t collisions < <(printf '%s\n' "${listing}" | awk -v prefix="${prefix}-" 'index($0, prefix) == 1')
  if ((${#collisions[@]} > 0)); then
    printf 'Refusing Immich provisioning because prefix %s already owns resources:\n' \
      "${prefix}" >&2
    printf '  %s\n' "${collisions[@]}" >&2
    return 1
  fi
}

immich_expect_collision() {
  local socket=$1 prefix=$2
  local status=0
  immich_assert_clean_prefix "${socket}" "${prefix}" || status=$?
  if ((status == 1)); then
    return 0
  fi
  if ((status == 0)); then
    printf 'Immich collision check unexpectedly found a clean prefix.\n' >&2
    return 1
  fi
  return "${status}"
}

immich_create_cli_database() {
  local socket=$1 prefix=$2 run=$3
  immich_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-immich-database" \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-immich" \
    --network "${prefix}-immich-backend:alias=database" \
    --volume "${prefix}-immich-pgdata:/var/lib/postgresql/data" \
    --env POSTGRES_DB=immich --env POSTGRES_INITDB_ARGS=--data-checksums \
    --env POSTGRES_USER=immich --env "POSTGRES_PASSWORD=${IMMICH_DB_PASSWORD}" \
    --health-cmd 'pg_isready -U immich -d immich' \
    --health-interval 2s --health-retries 120 \
    "$(immich_image_reference postgres)" postgres \
    -c shared_preload_libraries=vchord.so \
    -c 'search_path="$user", public, vectors' \
    -c shared_buffers=256MB -c wal_compression=on > /dev/null
}

immich_create_cli_redis() {
  local socket=$1 prefix=$2 run=$3
  immich_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-immich-redis" \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-immich" \
    --network "${prefix}-immich-backend:alias=redis" \
    --volume "${prefix}-immich-redisdata:/data" \
    --health-cmd 'valkey-cli ping' --health-interval 2s --health-retries 120 \
    "$(immich_image_reference valkey)" > /dev/null
}

immich_create_cli_machine_learning() {
  local socket=$1 prefix=$2 run=$3 fixture_root
  fixture_root="/tmp/boxferry-fixture/${prefix}"
  immich_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-immich-machine-learning" \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-immich" \
    --network "${prefix}-immich-backend:alias=immich-machine-learning" \
    --volume "${prefix}-immich-model-cache:/cache" \
    --volume "${fixture_root}:/fixture:rw" \
    --health-cmd "python3 -c \"import urllib.request; urllib.request.urlopen('http://127.0.0.1:3003/ping', timeout=5)\"" \
    --health-interval 2s --health-retries 120 \
    "$(immich_image_reference machine-learning)" > /dev/null
}

immich_create_cli_server() {
  local socket=$1 prefix=$2 run=$3
  immich_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-immich-server" \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-immich" \
    --requires "${prefix}-immich-database,${prefix}-immich-redis,${prefix}-immich-machine-learning" \
    --network "${prefix}-immich-backend:alias=immich-server" \
    --network "${prefix}-immich-edge:alias=immich-server" \
    --volume "${prefix}-immich-library:/data" \
    --publish "127.0.0.1:${IMMICH_HTTP_PORT}:2283" \
    --env DB_DATABASE_NAME=immich --env DB_HOSTNAME=database \
    --env "DB_PASSWORD=${IMMICH_DB_PASSWORD}" --env DB_USERNAME=immich \
    --env IMMICH_MACHINE_LEARNING_URL=http://immich-machine-learning:3003 \
    --env REDIS_HOSTNAME=redis --env TZ=Etc/UTC \
    "$(immich_image_reference server)" > /dev/null
}

immich_wait_dependencies() {
  local socket=$1 prefix=$2 label=$3
  immich_wait_for 300 "${label} PostgreSQL readiness" \
    immich_remote "${socket}" exec "${prefix}-immich-database" \
    pg_isready -U immich -d immich
  immich_wait_for 300 "${label} Valkey readiness" \
    immich_remote "${socket}" exec "${prefix}-immich-redis" valkey-cli ping
  immich_wait_for 300 "${label} ML /ping readiness" \
    immich_remote "${socket}" exec "${prefix}-immich-machine-learning" \
    python3 -c "import urllib.request; urllib.request.urlopen('http://127.0.0.1:3003/ping', timeout=5)"
}

immich_create_cli_runtime() {
  local socket=$1 prefix=$2 run=$3
  immich_create_cli_database "${socket}" "${prefix}" "${run}"
  immich_create_cli_redis "${socket}" "${prefix}" "${run}"
  immich_create_cli_machine_learning "${socket}" "${prefix}" "${run}"
  immich_wait_dependencies "${socket}" "${prefix}" CLI
  immich_create_cli_server "${socket}" "${prefix}" "${run}"
}

immich_provision_cli() {
  local socket=$1 prefix=$2 run=$3 volume
  immich_assert_clean_prefix "${socket}" "${prefix}"
  immich_remote "${socket}" network create --internal \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-immich" \
    "${prefix}-immich-backend" > /dev/null
  immich_remote "${socket}" network create \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-immich" \
    "${prefix}-immich-edge" > /dev/null
  for volume in library model-cache pgdata redisdata; do
    immich_remote "${socket}" volume create \
      --label "io.boxferry.live-run=${run}" \
      --label "io.boxferry.application=${prefix}-immich" \
      "${prefix}-immich-${volume}" > /dev/null
  done
  immich_create_cli_runtime "${socket}" "${prefix}" "${run}"
}

immich_compose_project() {
  local socket=$1 prefix=$2 run=$3
  shift 3
  local provider=${BOXFERRY_COMPOSE_BIN:-${repository_root}/target/tools/docker-compose}
  local fixture fixture_root
  fixture="$(immich_fixture_root)"
  fixture_root="/tmp/boxferry-fixture/${prefix}"
  timed_operation 5m "Docker Compose Immich ${1:-command}" \
    env DOCKER_HOST="unix://${socket}" \
    BF_PREFIX="${prefix}" BF_RUN="${run}" BF_FIXTURE_ROOT="${fixture_root}" \
    BF_DB_PASSWORD="${IMMICH_DB_PASSWORD}" \
    BF_SERVER_IMAGE="$(immich_image_reference server)" \
    BF_MACHINE_LEARNING_IMAGE="$(immich_image_reference machine-learning)" \
    BF_VALKEY_IMAGE="$(immich_image_reference valkey)" \
    BF_POSTGRES_IMAGE="$(immich_image_reference postgres)" \
    "${provider}" --project-name "${prefix}-immich" --file "${fixture}/compose.yaml" "$@"
}

immich_start_compose_services() {
  local socket=$1 prefix=$2 run=$3 log=$4
  immich_compose_project "${socket}" "${prefix}" "${run}" \
    up --detach --no-deps --remove-orphans database redis immich-machine-learning \
    > "${log}" 2>&1
  immich_wait_dependencies "${socket}" "${prefix}" 'Docker Compose'
  immich_compose_project "${socket}" "${prefix}" "${run}" \
    up --detach --no-deps immich-server >> "${log}" 2>&1
}

immich_provision_compose() {
  local socket=$1 prefix=$2 run=$3
  immich_assert_clean_prefix "${socket}" "${prefix}"
  immich_start_compose_services \
    "${socket}" "${prefix}" "${run}" "${current_case}/immich-compose.log"
}

immich_probe() {
  local socket=$1 prefix=$2
  shift 2
  immich_remote "${socket}" exec \
    --env BF_IMMICH_URL=http://immich-server:2283 \
    --env "BF_IMMICH_EMAIL=${IMMICH_ADMIN_EMAIL}" \
    --env "BF_IMMICH_PASSWORD=${IMMICH_ADMIN_PASSWORD}" \
    "${prefix}-immich-machine-learning" python3 /fixture/media-probe.py "$@"
}

immich_wait_application() {
  local socket=$1 prefix=$2
  immich_wait_for 480 'API readiness and administrator login' \
    immich_probe "${socket}" "${prefix}" ready
}

immich_probe_published_api() {
  local socket=$1 prefix=$2 fixture_root
  fixture_root="/tmp/boxferry-fixture/${prefix}"
  immich_remote "${socket}" run --rm --pull=never --network host \
    --volume "${fixture_root}:/fixture:ro" \
    --env "BF_IMMICH_URL=http://127.0.0.1:${IMMICH_HTTP_PORT}" \
    --env "BF_IMMICH_EMAIL=${IMMICH_ADMIN_EMAIL}" \
    --env "BF_IMMICH_PASSWORD=${IMMICH_ADMIN_PASSWORD}" \
    --entrypoint python3 "$(immich_image_reference machine-learning)" \
    /fixture/media-probe.py ready
}

immich_valkey_commands() {
  local socket=$1 prefix=$2
  immich_remote "${socket}" exec "${prefix}-immich-redis" \
    valkey-cli INFO commandstats | awk -F '[:,=]' \
    '/^cmdstat_/ { for (field = 1; field <= NF; field++) if ($field == "calls") total += $(field + 1) } END { print total + 0 }'
}

immich_assert_ml_boundary() {
  local socket=$1 prefix=$2
  immich_remote "${socket}" exec "${prefix}-immich-machine-learning" \
    python3 -c "import urllib.request; urllib.request.urlopen('http://127.0.0.1:3003/ping', timeout=5)" \
    > /dev/null
  if immich_remote "${socket}" exec "${prefix}-immich-machine-learning" \
    /bin/sh -ceu 'test -z "$(find /cache -mindepth 1 -type f -print -quit)"' &&
    ! immich_remote "${socket}" logs "${prefix}-immich-machine-learning" |
    grep --fixed-strings --quiet '/predict'; then
    return 0
  fi
  printf 'Immich ML boundary observed a model-cache file or /predict request.\n' >&2
  return 1
}

immich_ingest_phase() {
  local socket=$1 prefix=$2 phase=$3 before after
  before="$(immich_valkey_commands "${socket}" "${prefix}")"
  immich_probe "${socket}" "${prefix}" ingest --phase "${phase}" \
    --state /fixture/probe-state.json --work-dir "/fixture/generated-${phase}"
  after="$(immich_valkey_commands "${socket}" "${prefix}")"
  [[ "${before}" =~ ^[0-9]+$ && "${after}" =~ ^[0-9]+$ && "${after}" -gt "${before}" ]] || {
    printf 'Immich upload did not increase Valkey command activity: %s -> %s.\n' \
      "${before}" "${after}" >&2
    return 1
  }
  immich_assert_ml_boundary "${socket}" "${prefix}"
}

immich_verify_asset() {
  local socket=$1 prefix=$2
  immich_probe "${socket}" "${prefix}" verify --state /fixture/probe-state.json
  immich_assert_ml_boundary "${socket}" "${prefix}"
}

immich_assert_database() {
  local socket=$1 prefix=$2 expected=$3 asset_rows job_rows extensions
  asset_rows="$(immich_remote "${socket}" exec "${prefix}-immich-database" \
    psql -U immich -d immich -Atqc 'SELECT count(*) FROM asset;')"
  job_rows="$(immich_remote "${socket}" exec "${prefix}-immich-database" \
    psql -U immich -d immich -Atqc 'SELECT count(*) FROM asset_job_status;')"
  [[ "${asset_rows}" == "${expected}" && "${job_rows}" == "${expected}" ]] || {
    printf 'Immich PostgreSQL expected %s asset/job rows; observed assets=%s jobs=%s.\n' \
      "${expected}" "${asset_rows}" "${job_rows}" >&2
    return 1
  }
  extensions="$(immich_remote "${socket}" exec "${prefix}-immich-database" \
    psql -U immich -d immich -Atqc \
    "SELECT extname || ':' || extversion FROM pg_extension WHERE extname IN ('cube','earthdistance','vector','vchord') ORDER BY extname;")"
  grep --fixed-strings --line-regexp --quiet 'vchord:0.4.3' <<< "${extensions}"
  for extension in cube earthdistance vector; do
    grep --extended-regexp --line-regexp --quiet "${extension}:[0-9.]+" <<< "${extensions}"
  done
}

immich_assert_application_boundaries() {
  local socket=$1 prefix=$2 container inspect_file
  for container in database redis machine-learning; do
    inspect_file="${current_case}/${prefix}-${container}.inspect.json"
    immich_remote "${socket}" inspect "${prefix}-immich-${container}" > "${inspect_file}"
    jq --exit-status '
      .[0].HostConfig.PortBindings == {} and
      ((.[0].NetworkSettings.Ports // {}) | all(.[]; . == null or . == []))
    ' "${inspect_file}" > /dev/null
  done
  inspect_file="${current_case}/${prefix}-server.inspect.json"
  immich_remote "${socket}" inspect "${prefix}-immich-server" > "${inspect_file}"
  jq --exit-status --arg backend "${prefix}-immich-backend" --arg edge "${prefix}-immich-edge" '
    (.[0].NetworkSettings.Networks | has($backend) and has($edge)) and
    .[0].HostConfig.PortBindings["2283/tcp"][0].HostIp == "127.0.0.1" and
    .[0].HostConfig.PortBindings["2283/tcp"][0].HostPort == "18283" and
    ((.[0].HostConfig.Devices // []) | length) == 0
  ' "${inspect_file}" > /dev/null
  for container in database redis machine-learning; do
    immich_remote "${socket}" inspect --format '{{json .NetworkSettings.Networks}}' \
      "${prefix}-immich-${container}" | jq --exit-status \
      --arg backend "${prefix}-immich-backend" \
      'has($backend) and (keys | length == 1)' > /dev/null
  done
  immich_remote "${socket}" network inspect "${prefix}-immich-backend" |
    jq --exit-status '.[0].internal == true or .[0].Internal == true' > /dev/null
}

immich_assert_storage_permissions() {
  local socket=$1 prefix=$2 container path volume permission inspect_file
  while IFS=':' read -r container path volume permission; do
    inspect_file="${current_case}/${prefix}-${container}-mount.inspect.json"
    immich_remote "${socket}" inspect "${prefix}-immich-${container}" > "${inspect_file}"
    jq --exit-status --arg destination "${path}" --arg name "${prefix}-immich-${volume}" '
      any(.[0].Mounts[]?;
        .Type == "volume" and .Name == $name and .Destination == $destination and .RW == true)
    ' "${inspect_file}" > /dev/null
    immich_remote "${socket}" exec "${prefix}-immich-${container}" /bin/sh -ceu '
      path=$1
      permission=$2
      test -r "$path" && test -w "$path"
      mode="$(stat -c %a "$path")"
      case "$permission:$mode" in
        private:*)
          case "$(stat -c %A "$path")" in ????????w?) exit 1 ;; esac
          ;;
        sticky:1777) ;;
        *) exit 1 ;;
      esac
    ' sh "${path}" "${permission}"
  done << 'EOF'
database:/var/lib/postgresql/data:pgdata:private
redis:/data:redisdata:sticky
machine-learning:/cache:model-cache:private
server:/data:library:private
EOF

  local redisdata="${prefix}-immich-redisdata"
  for container in database machine-learning server; do
    immich_remote "${socket}" inspect "${prefix}-immich-${container}" |
      jq --exit-status --arg redisdata "${redisdata}" \
        'all(.[0].Mounts[]?; .Name != $redisdata)' > /dev/null
  done
}

immich_recreate_application() {
  local mode=$1 socket=$2 prefix=$3 run=$4
  if [[ "${mode}" == compose ]]; then
    immich_compose_project "${socket}" "${prefix}" "${run}" \
      stop --timeout 30 > "${current_case}/immich-compose-stop.log" 2>&1
    immich_compose_project "${socket}" "${prefix}" "${run}" \
      rm --force >> "${current_case}/immich-compose-stop.log" 2>&1
    immich_start_compose_services \
      "${socket}" "${prefix}" "${run}" "${current_case}/immich-compose-recreate.log"
  else
    immich_remote "${socket}" stop --time 30 \
      "${prefix}-immich-server" "${prefix}-immich-machine-learning" \
      "${prefix}-immich-redis" "${prefix}-immich-database" > /dev/null
    immich_remote "${socket}" rm --force \
      "${prefix}-immich-server" "${prefix}-immich-machine-learning" \
      "${prefix}-immich-redis" "${prefix}-immich-database" > /dev/null
    immich_create_cli_runtime "${socket}" "${prefix}" "${run}"
  fi
  immich_wait_application "${socket}" "${prefix}"
}

immich_assert_output_membership() {
  local selection=$1 output=$2 directory=$3 prefix=$4
  assert_named_member "${output}" "${directory}" immich-server "${prefix}-immich-server"
  assert_named_member "${output}" "${directory}" database "${prefix}-immich-database"
  assert_named_member "${output}" "${directory}" redis "${prefix}-immich-redis"
  assert_named_member "${output}" "${directory}" immich-machine-learning \
    "${prefix}-immich-machine-learning"
  assert_resource_member "${output}" "${directory}" network "${prefix}-immich-backend"
  assert_resource_member "${output}" "${directory}" network "${prefix}-immich-edge"
  local volume
  for volume in library model-cache pgdata redisdata; do
    assert_resource_member "${output}" "${directory}" volume "${prefix}-immich-${volume}"
  done
  [[ "${selection}" == exact || "${selection}" == label || "${selection}" == all ]]
}

immich_assert_output_semantics() {
  local output=$1 directory=$2 prefix=$3 report=$4
  local id
  for id in server machine-learning valkey postgres; do
    grep --recursive --fixed-strings --quiet \
      "$(immich_image_reference "${id}")" "${directory}"
  done
  if [[ "${output}" == podman ]]; then
    local subject key
    local -a subjects=(
      "networks.${prefix}-immich-backend.internal"
      "networks.${prefix}-immich-edge.internal"
      "services.${prefix}-immich-server.ports"
    )
    for key in POSTGRES_DB POSTGRES_INITDB_ARGS POSTGRES_PASSWORD POSTGRES_USER; do
      subjects+=("services.${prefix}-immich-database.environment.${key}")
    done
    for key in DB_DATABASE_NAME DB_HOSTNAME DB_PASSWORD DB_USERNAME \
      IMMICH_MACHINE_LEARNING_URL REDIS_HOSTNAME TZ; do
      subjects+=("services.${prefix}-immich-server.environment.${key}")
    done
    for subject in "${subjects[@]}"; do
      jq --exit-status --arg subject "${subject}" '
        [.diagnostics[]? |
          select(
            .code == "BFP0007" and
            any(.fields[]?; .name == "subject" and .value == $subject)
          )] as $matches |
        ($matches | length) == 1 and
        ($matches[0] |
          any(.fields[]?; .name == "decision" and .value == "omitted") and
          any(.fields[]?; .name == "required_loss_policy" and .value == "partial"))
      ' "${report}" > /dev/null
    done
    jq --exit-status 'any(.output_artifacts[]?; .name == "podman.json")' \
      "${report}" > /dev/null
    jq --exit-status --arg server "${prefix}-immich-server" '
      all(.operations[]? | select(.resource.name == $server);
        all((.cli.argv // [])[]?; . != "--publish" and . != "-p" and . != "--env" and . != "-e"))
    ' "${directory}/podman.json" > /dev/null
    if grep --recursive --fixed-strings --quiet \
      -e DB_PASSWORD -e DB_HOSTNAME -e IMMICH_MACHINE_LEARNING_URL \
      -e POSTGRES_PASSWORD -e "${IMMICH_DB_PASSWORD}" "${directory}"; then
      printf 'Immich Podman plan retained omitted environment or protected values.\n' >&2
      return 1
    fi
  else
    local literal
    for literal in \
      '/data' '/cache' '/var/lib/postgresql/data'; do
      grep --recursive --fixed-strings --quiet "${literal}" "${directory}"
    done
    grep --recursive --fixed-strings --quiet "${IMMICH_DB_PASSWORD}" "${directory}"
    if [[ "${output}" == compose ]]; then
      for literal in \
        '- DB_DATABASE_NAME=immich' '- DB_HOSTNAME=database' '- DB_USERNAME=immich' \
        '- IMMICH_MACHINE_LEARNING_URL=http://immich-machine-learning:3003' \
        '- REDIS_HOSTNAME=redis' '- TZ=Etc/UTC' '- POSTGRES_DB=immich' \
        '- POSTGRES_INITDB_ARGS=--data-checksums' '- POSTGRES_USER=immich'; do
        grep --fixed-strings --quiet -- "${literal}" "${directory}/compose.yaml"
      done
      grep --fixed-strings --quiet 'host_ip: 127.0.0.1' "${directory}/compose.yaml"
      grep --fixed-strings --quiet 'published: "18283"' "${directory}/compose.yaml"
      grep --fixed-strings --quiet 'target: 2283' "${directory}/compose.yaml"
      grep --fixed-strings --quiet 'internal: true' "${directory}/compose.yaml"
    else
      for literal in \
        'DB_DATABASE_NAME=immich' 'DB_HOSTNAME=database' 'DB_USERNAME=immich' \
        'IMMICH_MACHINE_LEARNING_URL=http://immich-machine-learning:3003' \
        'REDIS_HOSTNAME=redis' 'TZ=Etc/UTC' 'POSTGRES_DB=immich' \
        'POSTGRES_INITDB_ARGS=--data-checksums' 'POSTGRES_USER=immich'; do
        grep --recursive --fixed-strings --quiet "${literal}" "${directory}"
      done
      grep --recursive --extended-regexp --quiet \
        '^PublishPort=127\.0\.0\.1:18283:2283(/tcp)?$' "${directory}"
      grep --recursive --fixed-strings --quiet 'Internal=true' "${directory}"
    fi
  fi
}

immich_report_conversion_failure() {
  local report=$1
  if [[ -s "${report}" ]]; then
    jq . "${report}" >&2 || sed -n '1,240p' "${report}" >&2
  fi
}

immich_run_exports() {
  local mode=$1 socket=$2 prefix=$3 selection output directory report
  local -a selection_arguments target_arguments
  mkdir -p "${current_case}/outputs"
  for selection in exact label all; do
    case "${selection}" in
      exact) selection_arguments=(--podman-resource "container=${prefix}-immich-server") ;;
      label) selection_arguments=(--podman-label "io.boxferry.application=${prefix}-immich") ;;
      all) selection_arguments=(--podman-all) ;;
    esac
    for output in compose quadlet podman; do
      directory="${current_case}/outputs/${mode}-${selection}-${output}"
      report="${directory}.report.json"
      target_arguments=()
      [[ "${output}" == podman ]] && target_arguments+=(--podman-target-context rootless)
      if ! boxferry_operation "Immich ${mode} ${selection} Podman-to-${output}" \
        convert podman "${output}" --podman-socket "${socket}" \
        --application-name "${prefix}-immich" --loss-policy partial \
        --promote-podman-effective-named-volumes \
        --promote-podman-effective-named-networks \
        --promote-podman-portable-effective-settings \
        --output-directory "${directory}" --console-format json \
        "${target_arguments[@]}" "${selection_arguments[@]}" > "${report}"; then
        immich_report_conversion_failure "${report}"
        return 1
      fi
      jq --exit-status '
        .schema_version == 1 and .status == "success" and .exit_category == "success" and
        ([.diagnostics[]? | select(.severity == "error")] | length == 0) and
        ((.fidelity.invalid // 0) == 0) and (.output_artifacts | length > 0)
      ' "${report}" > /dev/null
      immich_assert_output_membership "${selection}" "${output}" "${directory}" "${prefix}"
      immich_assert_output_semantics "${output}" "${directory}" "${prefix}" "${report}"
    done
  done
  if grep --recursive --include='*.report.json' --fixed-strings --quiet \
    -e "${IMMICH_ADMIN_PASSWORD}" -e "${IMMICH_DB_PASSWORD}" "${current_case}/outputs"; then
    printf 'Immich BoxFerry reports leaked public protected-value canary.\n' >&2
    return 1
  fi
}

immich_capture_candidate() {
  local socket=$1 prefix=$2
  local output=${BOXFERRY_IMMICH_CAPTURE_DIRECTORY:?capture candidate path must be explicit}
  local capture_tool="${repository_root}/fixtures/conformance/podman-live/capture_proxy.py"
  # Keep AF_UNIX paths below portable limits; the runner removes runtime_root on EXIT.
  local proxy_socket="${runtime_root}/immich-capture.sock"
  python3 "${capture_tool}" \
    --application immich \
    --repository "${repository_root}" \
    --output-directory "${output}" \
    --upstream-socket "${socket}" \
    --proxy-socket "${proxy_socket}" \
    --boxferry-bin "${boxferry_bin:?caller must supply boxferry_bin}" \
    --prefix "${prefix}"
}

immich_clear_probe_state() {
  local socket=$1 prefix=$2 fixture_root
  fixture_root="/tmp/boxferry-fixture/${prefix}"
  immich_remote "${socket}" run --rm --pull=never --network none --user 0:0 \
    --volume "${fixture_root}:/fixture:rw" --entrypoint /bin/sh \
    "$(immich_image_reference machine-learning)" -ceu \
    'rm -rf -- /fixture/probe-state.json /fixture/generated-baseline'
}

immich_cleanup_mode() {
  local mode=$1 socket=$2 prefix=$3 run=$4
  if [[ "${mode}" == compose ]]; then
    immich_compose_project "${socket}" "${prefix}" "${run}" \
      down --volumes --remove-orphans \
      > "${current_case}/immich-compose-down.log" 2>&1
  else
    immich_remote "${socket}" stop --time 30 \
      "${prefix}-immich-server" "${prefix}-immich-machine-learning" \
      "${prefix}-immich-redis" "${prefix}-immich-database" > /dev/null 2>&1 || true
    immich_remote "${socket}" rm --force --time 0 --ignore \
      "${prefix}-immich-server" "${prefix}-immich-machine-learning" \
      "${prefix}-immich-redis" "${prefix}-immich-database" > /dev/null
    immich_remote "${socket}" volume rm --force \
      "${prefix}-immich-library" "${prefix}-immich-model-cache" \
      "${prefix}-immich-pgdata" "${prefix}-immich-redisdata" > /dev/null
    immich_remote "${socket}" network rm \
      "${prefix}-immich-backend" "${prefix}-immich-edge" > /dev/null
  fi
  immich_assert_clean_prefix "${socket}" "${prefix}"
}

immich_verify_target() {
  local id=$1 image=$2 declared_version=$3 distribution=$4 mode=$5 lane=$6 architecture=$7
  verify_observed_version "${id}" "${declared_version}" "${artifact_root}/${id}.podman-version"
  [[ "$(< "${artifact_root}/${id}.digest")" == "${image##*@}" ]] || {
    printf 'Pulled image digest does not match reviewed Immich target %s.\n' "${id}" >&2
    return 1
  }
  [[ "${architecture}" == amd64 &&
    "$(< "${artifact_root}/${id}.architecture")" =~ ^(x86_64|amd64)$ ]] || {
    printf 'Observed architecture does not match Immich target %s.\n' "${id}" >&2
    return 1
  }
  append_verified_evidence "${id}" "${image}" "${declared_version}" "${distribution}" \
    "${mode}" "${lane}" "${architecture}" "${artifact_root}/${id}" \
    nested-image immich-application
}

run_immich_application_cell() {
  local id=$1 image=$2 declared_version=$3 distribution=$4 mode=$5 lane=$6 architecture=$7
  [[ "${id}-${mode}" == podman-6.1-rootless-rootless ]] || {
    printf 'Immich profile is bounded to reviewed Podman 6.1 rootless.\n' >&2
    return 1
  }

  current_prefix="${run_id:0:32}-immich"
  current_case="${artifact_root}/${id}-immich-application"
  local socket_directory="${runtime_root}/immich-application-target"
  local application_archive="${runtime_root}/immich-application-images.tar"
  mkdir -p -- "${current_case}" "${socket_directory}"
  chmod 0700 "${current_case}" "${socket_directory}"
  progress_index=0
  progress_total=38
  if [[ -n "${BOXFERRY_IMMICH_CAPTURE_DIRECTORY:-}" ]]; then
    progress_total=$((progress_total + 1))
  fi
  printf '%s PLAN %s immich-application tests=%d\n' \
    "$(timestamp)" "${id}" "${progress_total}"

  progress_run 'verify Immich host resource budget' immich_validate_resource_budget
  progress_run 'validate Immich image and provider catalogues' immich_validate_catalogues
  progress_run 'validate Docker Compose provider' immich_validate_provider
  progress_run 'verify deterministic PNG generation' immich_verify_media_generator
  progress_run 'prepare bounded digest-pinned Immich image archive' \
    immich_prepare_image_archive "${application_archive}"
  progress_run 'start isolated Immich Podman target' \
    start_outer_runtime "${id}" "${image}" "${mode}" "${socket_directory}" \
    "${application_archive}"
  progress_run 'verify Podman Immich target evidence' \
    immich_verify_target "${id}" "${image}" "${declared_version}" "${distribution}" \
    "${mode}" "${lane}" "${architecture}"
  local outer="${started_outer}"
  progress_run 'load Immich images and rootless network configuration' \
    immich_prepare_application_target "${outer}" "${current_prefix}" "${socket_directory}"
  local socket="${socket_directory}/podman.sock"

  progress_run 'provision independent Podman CLI Immich application' \
    immich_provision_cli "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'wait for Podman CLI Immich API readiness' \
    immich_wait_application "${socket}" "${current_prefix}"
  progress_run 'prove Podman CLI loopback publication through host network' \
    immich_probe_published_api "${socket}" "${current_prefix}"
  if [[ -n "${BOXFERRY_IMMICH_CAPTURE_DIRECTORY:-}" ]]; then
    progress_run 'capture sanitized Podman CLI Immich evidence candidate' \
      immich_capture_candidate "${socket}" "${current_prefix}"
  fi
  progress_run 'prove Podman CLI ML health without model inference' \
    immich_assert_ml_boundary "${socket}" "${current_prefix}"
  progress_run 'upload process retrieve deterministic PNG through CLI topology' \
    immich_ingest_phase "${socket}" "${current_prefix}" baseline
  progress_run 'prove CLI PostgreSQL asset job extension state' \
    immich_assert_database "${socket}" "${current_prefix}" 1
  progress_run 'prove CLI publication network and CPU-only boundaries' \
    immich_assert_application_boundaries "${socket}" "${current_prefix}"
  progress_run 'prove CLI named-volume mounts and application write access' \
    immich_assert_storage_permissions "${socket}" "${current_prefix}"
  progress_run 'exercise CLI exact label all exports without execution' \
    immich_run_exports cli "${socket}" "${current_prefix}"
  progress_run 'refuse collision with CLI Immich resources' \
    immich_expect_collision "${socket}" "${current_prefix}"
  progress_run 'recreate CLI containers without deleting volumes' \
    immich_recreate_application cli "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'retrieve original preview thumbnail after CLI recreation' \
    immich_verify_asset "${socket}" "${current_prefix}"
  progress_run 'reprove CLI PostgreSQL row and extension persistence' \
    immich_assert_database "${socket}" "${current_prefix}" 1
  progress_run 'clean prefix-scoped Podman CLI Immich resources' \
    immich_cleanup_mode cli "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'remove CLI probe state before provider-independent run' \
    immich_clear_probe_state "${socket}" "${current_prefix}"

  progress_run 'provision independent Docker Compose Immich application' \
    immich_provision_compose "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'wait for Docker Compose Immich API readiness' \
    immich_wait_application "${socket}" "${current_prefix}"
  progress_run 'prove Docker Compose loopback publication through host network' \
    immich_probe_published_api "${socket}" "${current_prefix}"
  progress_run 'prove Compose ML health without model inference' \
    immich_assert_ml_boundary "${socket}" "${current_prefix}"
  progress_run 'upload process retrieve deterministic PNG through Compose topology' \
    immich_ingest_phase "${socket}" "${current_prefix}" baseline
  progress_run 'prove Compose PostgreSQL asset job extension state' \
    immich_assert_database "${socket}" "${current_prefix}" 1
  progress_run 'prove Compose publication network and CPU-only boundaries' \
    immich_assert_application_boundaries "${socket}" "${current_prefix}"
  progress_run 'prove Compose named-volume mounts and application write access' \
    immich_assert_storage_permissions "${socket}" "${current_prefix}"
  progress_run 'exercise Compose exact label all exports without execution' \
    immich_run_exports compose "${socket}" "${current_prefix}"
  progress_run 'refuse collision with Compose Immich resources' \
    immich_expect_collision "${socket}" "${current_prefix}"
  progress_run 'recreate Compose containers without deleting volumes' \
    immich_recreate_application compose "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'retrieve original preview thumbnail after Compose recreation' \
    immich_verify_asset "${socket}" "${current_prefix}"
  progress_run 'reprove Compose PostgreSQL row and extension persistence' \
    immich_assert_database "${socket}" "${current_prefix}" 1
  progress_run 'clean prefix-scoped Docker Compose Immich resources' \
    immich_cleanup_mode compose "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'remove disposable Immich outer container' remove_outer "${outer}"
  printf '%s CELL PASS %s immich-application (%d/%d tests)\n' \
    "$(timestamp)" "${id}" "${progress_index}" "${progress_total}"
}
