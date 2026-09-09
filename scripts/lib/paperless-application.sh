#!/usr/bin/env bash
# Paperless-ngx application acceptance helpers. Sourced by the live entry point.
# shellcheck disable=SC2129,SC2154 # Caller-owned globals and phased logs are intentional.

readonly PAPERLESS_ADMIN_USER="boxferry-admin"
readonly PAPERLESS_ADMIN_PASSWORD="boxferry-public-admin-canary"
readonly PAPERLESS_DB_PASSWORD="boxferry-public-database-canary"
readonly PAPERLESS_REDIS_PASSWORD="boxferry-public-broker-canary"
readonly PAPERLESS_SECRET_KEY="boxferry-public-paperless-secret-canary"
readonly PAPERLESS_PROVIDER_VERSION="5.5.0"
readonly PAPERLESS_PROVIDER_SHA256="c57ab918abd5b05ca7e7d0f275875dd1330a695074f309dc9eab1b49efafcd4b"
readonly PAPERLESS_HTTP_PORT="18000"
readonly PAPERLESS_ARCHIVE_MAX_BYTES="2684354560"
readonly PAPERLESS_MIN_CPUS="2"
readonly PAPERLESS_MIN_MEMORY_KIB="6291456"
readonly PAPERLESS_MIN_DISK_KIB="12582912"

paperless_fixture_root() {
  printf '%s/fixtures/conformance/paperless-ngx-application\n' \
    "${repository_root:?caller must supply repository_root}"
}

paperless_image_reference() {
  local id=$1
  printf 'registry.invalid/boxferry-test/paperless-application:%s\n' "${id}"
}

paperless_validate_resource_budget() {
  local cpus memory_kib disk_kib graph_root
  cpus="$(nproc)"
  memory_kib="$(awk '$1 == "MemAvailable:" { print $2 }' /proc/meminfo)"
  graph_root="$("${engine}" info --format '{{.Store.GraphRoot}}')"
  disk_kib="$(
    df --output=avail -k "${runtime_root}" "${graph_root}" |
      awk 'NR > 1 && (minimum == "" || $1 < minimum) { minimum = $1 } END { print minimum }'
  )"
  [[ "${cpus}" =~ ^[0-9]+$ && "${cpus}" -ge "${PAPERLESS_MIN_CPUS}" ]] || {
    printf 'Paperless profile requires at least %s CPUs; found %s.\n' \
      "${PAPERLESS_MIN_CPUS}" "${cpus:-unknown}" >&2
    return 1
  }
  [[ "${memory_kib}" =~ ^[0-9]+$ && "${memory_kib}" -ge "${PAPERLESS_MIN_MEMORY_KIB}" ]] || {
    printf 'Paperless profile requires at least 6 GiB available memory; found %s KiB.\n' \
      "${memory_kib:-unknown}" >&2
    return 1
  }
  [[ "${disk_kib}" =~ ^[0-9]+$ && "${disk_kib}" -ge "${PAPERLESS_MIN_DISK_KIB}" ]] || {
    printf 'Paperless profile requires at least 12 GiB free disk; found %s KiB.\n' \
      "${disk_kib:-unknown}" >&2
    return 1
  }
  printf '%s PAPERLESS BUDGET cpus=%s memory-available-kib=%s disk-available-kib=%s\n' \
    "$(timestamp)" "${cpus}" "${memory_kib}" "${disk_kib}"
}

paperless_validate_catalogues() {
  local fixture
  fixture="$(paperless_fixture_root)"
  awk -F '\t' '
    NF && $1 !~ /^#/ {
      if (NF != 6 || $2 !~ /:[^\/@]+@sha256:[0-9a-f]{64}$/ || $3 == "" || $4 == "" ||
          $5 !~ /^https:\/\// || $6 != "transient-test-pull" || seen[$1]++) {
        printf "Invalid Paperless image row at line %d: %s\\n", NR, $0 > "/dev/stderr"
        bad = 1
      }
      count++
    }
    END { exit bad || count != 5 }
  ' "${fixture}/images.tsv"
  awk -F '\t' -v version="${PAPERLESS_PROVIDER_VERSION}" \
    -v sha="${PAPERLESS_PROVIDER_SHA256}" '
    NF && $1 !~ /^#/ {
      if (NF != 7 || $1 != "docker-compose" || $2 != version ||
          $3 != "https://github.com/docker/compose/releases/download/v5.5.0/docker-compose-linux-x86_64" ||
          $4 != sha || $5 != "Apache-2.0" || $6 != "https://github.com/docker/compose" ||
          $7 != "downloaded-test-tool") {
        printf "Invalid Paperless provider row at line %d: %s\\n", NR, $0 > "/dev/stderr"
        bad = 1
      }
      count++
    }
    END { exit bad || count != 1 }
  ' "${fixture}/providers.tsv"
  local file
  for file in compose.yaml document-probe.py images.tsv providers.tsv README.md; do
    [[ -s "${fixture}/${file}" ]]
  done
}

paperless_validate_provider() {
  local provider=${BOXFERRY_COMPOSE_BIN:-${repository_root}/target/tools/docker-compose}
  [[ -x "${provider}" ]] || {
    printf 'Paperless profile requires executable Docker Compose %s at %s.\n' \
      "${PAPERLESS_PROVIDER_VERSION}" "${provider}" >&2
    return 1
  }
  local observed_sha observed_version
  observed_sha="$(sha256sum "${provider}" | awk '{ print $1 }')"
  [[ "${observed_sha}" == "${PAPERLESS_PROVIDER_SHA256}" ]] || {
    printf 'Docker Compose checksum mismatch: expected %s, observed %s.\n' \
      "${PAPERLESS_PROVIDER_SHA256}" "${observed_sha}" >&2
    return 1
  }
  observed_version="$("${provider}" version --short | sed 's/^v//')"
  [[ "${observed_version}" == "${PAPERLESS_PROVIDER_VERSION}" ]] || {
    printf 'Docker Compose version mismatch: expected %s, observed %s.\n' \
      "${PAPERLESS_PROVIDER_VERSION}" "${observed_version}" >&2
    return 1
  }
}

paperless_verify_document_generator() {
  local temporary status=0
  temporary="$(mktemp -d /tmp/boxferry-paperless-documents.XXXXXX)"
  python3 "$(paperless_fixture_root)/document-probe.py" self-test \
    --work-dir "${temporary}" || status=$?
  rm -rf -- "${temporary}"
  return "${status}"
}

paperless_prepare_image_archive() {
  local archive=${1:?archive path required}
  if [[ -s "${archive}" ]]; then
    if [[ "$(stat -c '%s' "${archive}")" -gt "${PAPERLESS_ARCHIVE_MAX_BYTES}" ]]; then
      printf 'Existing Paperless image archive exceeds 2.5 GiB cap.\n' >&2
      return 1
    fi
    return 0
  fi
  local fixture id reference expected_digest observed_digest cache_status runtime_reference archive_directory
  fixture="$(paperless_fixture_root)"
  archive_directory="$(mktemp -d "${runtime_root}/paperless-image-archives.XXXXXX")"
  while IFS=$'\t' read -r id reference _ _ _ _; do
    [[ -z "${id}" || "${id}" == \#* ]] && continue
    expected_digest="${reference##*@}"
    cache_status=0
    engine_image_available "probe Paperless ${id} image cache" "${reference}" || cache_status=$?
    if ((cache_status == 1)); then
      timed_operation 6m "pull digest-pinned Paperless ${id} image" \
        "${engine}" pull --quiet "${reference}" \
        > "${artifact_root}/paperless-${id}.pull.log"
    elif ((cache_status != 0)); then
      return "${cache_status}"
    fi
    observed_digest="$(engine_operation "inspect Paperless ${id} image digest" \
      image inspect --format '{{.Digest}}' "${reference}")"
    [[ "${observed_digest}" == "${expected_digest}" ]] || {
      printf 'Paperless image digest mismatch %s: expected %s, observed %s.\n' \
        "${id}" "${expected_digest}" "${observed_digest}" >&2
      return 1
    }
    runtime_reference="$(paperless_image_reference "${id}")"
    engine_operation "tag reviewed Paperless ${id} image for nested archive" \
      tag "${reference}" "${runtime_reference}"
    timed_operation 10m "archive compressed Paperless ${id} image" \
      "${engine}" save --format oci-archive \
      --output "${archive_directory}/${id}.oci.tar" "${runtime_reference}"
  done < "${fixture}/images.tsv"
  timed_operation 10m 'bundle compressed digest-pinned Paperless image archives' \
    tar --create --file "${archive}" --directory "${archive_directory}" .
  rm -rf -- "${archive_directory}"
  chmod 0644 "${archive}"
  local archive_size
  archive_size="$(stat -c '%s' "${archive}")"
  [[ "${archive_size}" -le "${PAPERLESS_ARCHIVE_MAX_BYTES}" ]] || {
    printf 'Paperless image archive exceeds 2.5 GiB cap: %s bytes.\n' "${archive_size}" >&2
    return 1
  }
}

paperless_assert_loaded_images() {
  local outer=$1 fixture id
  fixture="$(paperless_fixture_root)"
  while IFS=$'\t' read -r id _ _ _ _ _; do
    [[ -z "${id}" || "${id}" == \#* ]] && continue
    engine_operation "verify loaded Paperless ${id} image" \
      exec "${outer}" podman image exists "$(paperless_image_reference "${id}")"
  done < "${fixture}/images.tsv"
}

paperless_prepare_application_target() {
  local outer=$1 prefix=$2 socket_directory=$3
  engine_operation 'copy rootless Paperless network configuration' cp \
    "${repository_root}/fixtures/conformance/podman-live/apply-target-containers.conf" \
    "${outer}:/tmp/99-boxferry-live.conf"
  # shellcheck disable=SC2016 # $HOME expands inside the nested target.
  engine_operation 'prepare rootless Paperless network configuration' \
    exec "${outer}" /bin/sh -ceu \
    'mkdir -p "$HOME/.config/containers/containers.conf.d"; cp /tmp/99-boxferry-live.conf "$HOME/.config/containers/containers.conf.d/99-boxferry-live.conf"'
  # shellcheck disable=SC2016 # $HOME expands inside the nested target.
  timed_operation 15m 'load digest-pinned Paperless application archives' \
    "${engine}" exec "${outer}" /bin/sh -ceu '
      directory=/tmp/boxferry-paperless-images
      mkdir -p "$directory"
      tar -xf /boxferry-workload.tar -C "$directory"
      for archive in "$directory"/*.oci.tar; do
        podman load --input "$archive"
      done
    ' > /dev/null
  paperless_assert_loaded_images "${outer}"
  local fixture destination
  fixture="$(paperless_fixture_root)"
  destination="/tmp/boxferry-fixture/${prefix}"
  engine_operation 'create disposable Paperless fixture directory' \
    exec "${outer}" mkdir -p -- "${destination}"
  engine_operation 'copy reviewed Paperless document probe' \
    cp "${fixture}/document-probe.py" "${outer}:${destination}/document-probe.py"
  activate_outer_runtime "${socket_directory}"
}

paperless_remote() {
  local socket=$1
  shift
  podman_socket "${socket}" "Paperless ${1:-command}" "$@"
}

paperless_remote_long() {
  local socket=$1 action=$2
  shift 2
  if [[ -n "${started_outer}" && ! -S "${socket}" ]]; then
    timed_operation 12m "Paperless ${action} through matching container CLI" \
      "${engine}" exec "${started_outer}" podman "$@"
  else
    timed_operation 12m "Paperless ${action} through acquisition socket" \
      "${engine}" --url "unix://${socket}" "$@"
  fi
}

paperless_wait_for() {
  local deadline_seconds=$1 description=$2
  shift 2
  local deadline=$((SECONDS + deadline_seconds))
  until "$@" > /dev/null 2>&1; do
    if ((SECONDS >= deadline)); then
      printf 'Timed out waiting for Paperless %s after %ss.\n' \
        "${description}" "${deadline_seconds}" >&2
      return 1
    fi
    sleep 2
  done
}

paperless_assert_clean_prefix() {
  local socket=$1 prefix=$2 listing
  local -a collisions=()
  listing="$(
    paperless_remote "${socket}" ps -a --format '{{.Names}}' || exit $?
    paperless_remote "${socket}" volume ls --format '{{.Name}}' || exit $?
    paperless_remote "${socket}" network ls --format '{{.Name}}'
  )" || {
    printf 'Could not inspect existing resources for Paperless prefix %s.\n' "${prefix}" >&2
    return 2
  }
  mapfile -t collisions < <(printf '%s\n' "${listing}" | awk -v prefix="${prefix}-" 'index($0, prefix) == 1')
  if ((${#collisions[@]} > 0)); then
    printf 'Refusing Paperless provisioning because prefix %s already owns resources:\n' \
      "${prefix}" >&2
    printf '  %s\n' "${collisions[@]}" >&2
    return 1
  fi
}

paperless_expect_collision() {
  local status=0
  paperless_assert_clean_prefix "$@" || status=$?
  if ((status == 1)); then
    return 0
  fi
  if ((status == 0)); then
    printf 'Paperless collision check unexpectedly found a clean prefix.\n' >&2
    return 1
  fi
  return "${status}"
}

paperless_create_cli_database() {
  local socket=$1 prefix=$2 run=$3
  paperless_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-paper-db" \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-paperless" \
    --network "${prefix}-paper-backend:alias=db" \
    --volume "${prefix}-paper-pgdata:/var/lib/postgresql" \
    --env POSTGRES_DB=paperless --env POSTGRES_USER=paperless \
    --env "POSTGRES_PASSWORD=${PAPERLESS_DB_PASSWORD}" \
    --health-cmd 'pg_isready -U paperless -d paperless' \
    --health-interval 2s --health-retries 90 \
    "$(paperless_image_reference postgres)" > /dev/null
}

paperless_create_cli_broker() {
  local socket=$1 prefix=$2 run=$3
  paperless_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-paper-broker" \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-paperless" \
    --network "${prefix}-paper-backend:alias=broker" \
    --volume "${prefix}-paper-redisdata:/data" \
    --health-cmd "valkey-cli -a ${PAPERLESS_REDIS_PASSWORD} ping" \
    --health-interval 2s --health-retries 90 \
    "$(paperless_image_reference valkey)" \
    valkey-server --requirepass "${PAPERLESS_REDIS_PASSWORD}" > /dev/null
}

paperless_create_cli_converters() {
  local socket=$1 prefix=$2 run=$3
  paperless_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-paper-gotenberg" \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-paperless" \
    --network "${prefix}-paper-backend:alias=gotenberg" \
    "$(paperless_image_reference gotenberg)" \
    gotenberg --chromium-disable-javascript=true --chromium-allow-list=file:///tmp/.* > /dev/null
  paperless_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-paper-tika" \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-paperless" \
    --network "${prefix}-paper-backend:alias=tika" \
    "$(paperless_image_reference tika)" > /dev/null
}

paperless_create_cli_webserver() {
  local socket=$1 prefix=$2 run=$3
  local fixture_root="/tmp/boxferry-fixture/${prefix}"
  paperless_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-paper-web" \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-paperless" \
    --requires "${prefix}-paper-db,${prefix}-paper-broker,${prefix}-paper-gotenberg,${prefix}-paper-tika" \
    --network "${prefix}-paper-backend:alias=webserver" \
    --network "${prefix}-paper-edge:alias=paperless" \
    --volume "${prefix}-paper-data:/usr/src/paperless/data" \
    --volume "${prefix}-paper-media:/usr/src/paperless/media" \
    --volume "${prefix}-paper-consume:/usr/src/paperless/consume" \
    --volume "${prefix}-paper-export:/usr/src/paperless/export" \
    --volume "${fixture_root}:/fixture:rw" \
    --publish "127.0.0.1:${PAPERLESS_HTTP_PORT}:8000" \
    --env "PAPERLESS_REDIS=redis://:${PAPERLESS_REDIS_PASSWORD}@${prefix}-paper-broker:6379" \
    --env "PAPERLESS_DBHOST=${prefix}-paper-db" --env PAPERLESS_DBNAME=paperless \
    --env PAPERLESS_DBUSER=paperless --env "PAPERLESS_DBPASS=${PAPERLESS_DB_PASSWORD}" \
    --env "PAPERLESS_SECRET_KEY=${PAPERLESS_SECRET_KEY}" \
    --env "PAPERLESS_ADMIN_USER=${PAPERLESS_ADMIN_USER}" \
    --env "PAPERLESS_ADMIN_PASSWORD=${PAPERLESS_ADMIN_PASSWORD}" \
    --env PAPERLESS_ADMIN_MAIL=boxferry@example.invalid \
    --env "PAPERLESS_URL=http://127.0.0.1:${PAPERLESS_HTTP_PORT}" \
    --env PAPERLESS_TIME_ZONE=Etc/UTC --env PAPERLESS_OCR_LANGUAGE=eng \
    --env PAPERLESS_TASK_WORKERS=1 --env PAPERLESS_THREADS_PER_WORKER=1 \
    --env PAPERLESS_WEBSERVER_WORKERS=1 --env PAPERLESS_TIKA_ENABLED=1 \
    --env "PAPERLESS_TIKA_ENDPOINT=http://${prefix}-paper-tika:9998" \
    --env "PAPERLESS_TIKA_GOTENBERG_ENDPOINT=http://${prefix}-paper-gotenberg:3000" \
    --env PAPERLESS_CONSUMER_POLLING=2 --env USERMAP_UID=1000 --env USERMAP_GID=1000 \
    "$(paperless_image_reference paperless)" > /dev/null
}

paperless_create_cli_runtime() {
  local socket=$1 prefix=$2 run=$3
  paperless_create_cli_database "${socket}" "${prefix}" "${run}"
  paperless_create_cli_broker "${socket}" "${prefix}" "${run}"
  paperless_create_cli_converters "${socket}" "${prefix}" "${run}"
  paperless_wait_for 240 'Podman CLI PostgreSQL readiness' \
    paperless_remote "${socket}" exec "${prefix}-paper-db" \
    pg_isready -U paperless -d paperless
  paperless_wait_for 240 'Podman CLI Valkey readiness' \
    paperless_remote "${socket}" exec "${prefix}-paper-broker" \
    valkey-cli --no-auth-warning -a "${PAPERLESS_REDIS_PASSWORD}" ping
  paperless_create_cli_webserver "${socket}" "${prefix}" "${run}"
}

paperless_provision_cli() {
  local socket=$1 prefix=$2 run=$3
  paperless_assert_clean_prefix "${socket}" "${prefix}"
  paperless_remote "${socket}" network create --internal \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-paperless" \
    "${prefix}-paper-backend" > /dev/null
  paperless_remote "${socket}" network create \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-paperless" \
    "${prefix}-paper-edge" > /dev/null
  local volume
  for volume in data media consume export pgdata redisdata; do
    paperless_remote "${socket}" volume create \
      --label "io.boxferry.live-run=${run}" \
      --label "io.boxferry.application=${prefix}-paperless" \
      "${prefix}-paper-${volume}" > /dev/null
  done
  paperless_create_cli_runtime "${socket}" "${prefix}" "${run}"
}

paperless_compose_project() {
  local socket=$1 prefix=$2 run=$3
  shift 3
  local provider=${BOXFERRY_COMPOSE_BIN:-${repository_root}/target/tools/docker-compose}
  local fixture compose_file fixture_root
  fixture="$(paperless_fixture_root)"
  compose_file="${fixture}/compose.yaml"
  fixture_root="/tmp/boxferry-fixture/${prefix}"
  timed_operation 3m "Docker Compose Paperless ${1:-command}" \
    env DOCKER_HOST="unix://${socket}" \
    BF_PREFIX="${prefix}" BF_RUN="${run}" BF_FIXTURE_ROOT="${fixture_root}" \
    BF_DB_PASSWORD="${PAPERLESS_DB_PASSWORD}" \
    BF_REDIS_PASSWORD="${PAPERLESS_REDIS_PASSWORD}" \
    BF_SECRET_KEY="${PAPERLESS_SECRET_KEY}" \
    BF_ADMIN_PASSWORD="${PAPERLESS_ADMIN_PASSWORD}" \
    BF_PAPERLESS_IMAGE="$(paperless_image_reference paperless)" \
    BF_POSTGRES_IMAGE="$(paperless_image_reference postgres)" \
    BF_VALKEY_IMAGE="$(paperless_image_reference valkey)" \
    BF_GOTENBERG_IMAGE="$(paperless_image_reference gotenberg)" \
    BF_TIKA_IMAGE="$(paperless_image_reference tika)" \
    "${provider}" --project-name "${prefix}-paperless" --file "${compose_file}" "$@"
}

paperless_provision_compose() {
  local socket=$1 prefix=$2 run=$3
  paperless_assert_clean_prefix "${socket}" "${prefix}"
  paperless_compose_project "${socket}" "${prefix}" "${run}" \
    up --detach --remove-orphans > "${current_case}/paperless-compose.log" 2>&1
}

paperless_wait_application() {
  local socket=$1 prefix=$2
  paperless_probe "${socket}" "${prefix}" ready
}

paperless_probe() {
  local socket=$1 prefix=$2
  shift 2
  local fixture_root="/tmp/boxferry-fixture/${prefix}"
  paperless_remote_long "${socket}" "published application probe ${1:-command}" \
    run --rm --pull=never --network host \
    --volume "${fixture_root}:/fixture:rw" \
    --env "BF_PAPERLESS_URL=http://127.0.0.1:${PAPERLESS_HTTP_PORT}" \
    --env "BF_PAPERLESS_USER=${PAPERLESS_ADMIN_USER}" \
    --env "BF_PAPERLESS_PASSWORD=${PAPERLESS_ADMIN_PASSWORD}" \
    --entrypoint python3 "$(paperless_image_reference paperless)" \
    /fixture/document-probe.py "$@"
}

paperless_valkey_lpush_calls() {
  local socket=$1 prefix=$2
  paperless_remote "${socket}" exec "${prefix}-paper-broker" \
    valkey-cli --no-auth-warning -a "${PAPERLESS_REDIS_PASSWORD}" INFO commandstats |
    awk -F'[=,]' '$1 == "cmdstat_lpush:calls" { print $2 }'
}

paperless_ingest_phase() {
  local socket=$1 prefix=$2 phase=$3
  local before after
  before="$(paperless_valkey_lpush_calls "${socket}" "${prefix}")"
  before="${before:-0}"
  [[ "${before}" =~ ^[0-9]+$ ]]
  paperless_probe "${socket}" "${prefix}" ingest \
    --phase "${phase}" --state /fixture/probe-state.json \
    --work-dir "/fixture/generated-${phase}"
  after="$(paperless_valkey_lpush_calls "${socket}" "${prefix}")"
  after="${after:-0}"
  [[ "${after}" =~ ^[0-9]+$ && "${after}" -gt "${before}" ]] || {
    printf 'Valkey LPUSH calls did not increase during %s ingestion: %s -> %s.\n' \
      "${phase}" "${before}" "${after}" >&2
    return 1
  }
}

paperless_verify_documents() {
  local socket=$1 prefix=$2
  paperless_probe "${socket}" "${prefix}" verify \
    --state /fixture/probe-state.json
}

paperless_assert_database() {
  local socket=$1 prefix=$2 expected=$3
  paperless_remote "${socket}" exec --env "PGPASSWORD=${PAPERLESS_DB_PASSWORD}" \
    "${prefix}-paper-db" psql -U paperless -d paperless -Atc \
    "SELECT count(*) FROM documents_document WHERE title LIKE 'BoxFerry migration %';" |
    awk -v expected="${expected}" '$1 == expected { found = 1 } END { exit !found }'
}

paperless_assert_application_boundaries() {
  local socket=$1 prefix=$2
  local broker_host="${prefix}-paper-broker"
  local db_host="${prefix}-paper-db"
  local gotenberg_host="${prefix}-paper-gotenberg"
  local tika_host="${prefix}-paper-tika"
  [[ "$(paperless_remote "${socket}" port "${prefix}-paper-web" 8000/tcp)" == "127.0.0.1:${PAPERLESS_HTTP_PORT}" ]]
  local private
  for private in db broker gotenberg tika; do
    [[ -z "$(paperless_remote "${socket}" port "${prefix}-paper-${private}")" ]]
  done
  paperless_remote "${socket}" network inspect "${prefix}-paper-backend" |
    jq --exit-status '.[0].internal == true' > /dev/null
  paperless_remote "${socket}" inspect \
    "${prefix}-paper-db" "${prefix}-paper-broker" \
    "${prefix}-paper-gotenberg" "${prefix}-paper-tika" "${prefix}-paper-web" |
    jq --exit-status \
      --arg backend "${prefix}-paper-backend" --arg edge "${prefix}-paper-edge" '
        all(.[0:4][]; .NetworkSettings.Networks[$backend] != null and
          .NetworkSettings.Networks[$edge] == null) and
        (.[4].NetworkSettings.Networks[$backend] != null) and
        (.[4].NetworkSettings.Networks[$edge] != null)
      ' > /dev/null
  paperless_remote "${socket}" inspect "${prefix}-paper-web" |
    jq --exit-status \
      --arg redis "PAPERLESS_REDIS=redis://:${PAPERLESS_REDIS_PASSWORD}@${broker_host}:6379" \
      --arg dbhost "PAPERLESS_DBHOST=${db_host}" \
      --arg tika "PAPERLESS_TIKA_ENDPOINT=http://${tika_host}:9998" \
      --arg gotenberg "PAPERLESS_TIKA_GOTENBERG_ENDPOINT=http://${gotenberg_host}:3000" '
      (.[0].Config.Env | any(. == $redis)) and
      (.[0].Config.Env | any(. == $dbhost)) and
      (.[0].Config.Env | map(select(startswith("PAPERLESS_TASK_WORKERS="))) == ["PAPERLESS_TASK_WORKERS=1"]) and
      (.[0].Config.Env | map(select(startswith("PAPERLESS_THREADS_PER_WORKER="))) == ["PAPERLESS_THREADS_PER_WORKER=1"]) and
      (.[0].Config.Env | map(select(startswith("PAPERLESS_WEBSERVER_WORKERS="))) == ["PAPERLESS_WEBSERVER_WORKERS=1"]) and
      (.[0].Config.Env | any(. == "PAPERLESS_TIKA_ENABLED=1")) and
      (.[0].Config.Env | any(. == "PAPERLESS_CONSUMER_POLLING=2")) and
      (.[0].Config.Env | any(. == "PAPERLESS_ADMIN_USER=boxferry-admin")) and
      (.[0].Config.Env | any(. == "PAPERLESS_ADMIN_MAIL=boxferry@example.invalid")) and
      (.[0].Config.Env | any(. == "PAPERLESS_URL=http://127.0.0.1:18000")) and
      (.[0].Config.Env | any(. == $tika)) and
      (.[0].Config.Env | any(. == $gotenberg)) and
      (.[0].Config.Env | any(. == "PAPERLESS_TIME_ZONE=Etc/UTC")) and
      (.[0].Config.Env | any(. == "PAPERLESS_OCR_LANGUAGE=eng")) and
      (.[0].Config.Env | any(. == "USERMAP_UID=1000")) and
      (.[0].Config.Env | any(. == "USERMAP_GID=1000"))
    ' > /dev/null
  paperless_remote "${socket}" inspect "${prefix}-paper-gotenberg" |
    jq --exit-status '
      .[0].Config.Cmd == [
        "gotenberg",
        "--chromium-disable-javascript=true",
        "--chromium-allow-list=file:///tmp/.*"
      ]
    ' > /dev/null
  paperless_remote "${socket}" inspect "${prefix}-paper-web" |
    jq --exit-status \
      --arg data "${prefix}-paper-data" --arg media "${prefix}-paper-media" \
      --arg consume "${prefix}-paper-consume" --arg export "${prefix}-paper-export" '
        any(.[0].Mounts[]?; .Name == $data and .Destination == "/usr/src/paperless/data") and
        any(.[0].Mounts[]?; .Name == $media and .Destination == "/usr/src/paperless/media") and
        any(.[0].Mounts[]?; .Name == $consume and .Destination == "/usr/src/paperless/consume") and
        any(.[0].Mounts[]?; .Name == $export and .Destination == "/usr/src/paperless/export")
      ' > /dev/null
  paperless_remote "${socket}" inspect "${prefix}-paper-db" "${prefix}-paper-broker" |
    jq --exit-status \
      --arg pgdata "${prefix}-paper-pgdata" --arg redisdata "${prefix}-paper-redisdata" '
        any(.[0].Mounts[]?; .Name == $pgdata and .Destination == "/var/lib/postgresql") and
        any(.[1].Mounts[]?; .Name == $redisdata and .Destination == "/data")
      ' > /dev/null
}

paperless_assert_storage_permissions() {
  local socket=$1 prefix=$2
  # shellcheck disable=SC2016 # $path expands inside the application container.
  paperless_remote "${socket}" exec "${prefix}-paper-web" /bin/sh -ceu '
    for path in /usr/src/paperless/data /usr/src/paperless/media /usr/src/paperless/consume /usr/src/paperless/export; do
      test "$(stat -c "%u:%g" "$path")" = 1000:1000
      test -z "$(find "$path" -maxdepth 0 -perm -0002 -print -quit)"
    done
  '
  # shellcheck disable=SC2016 # $path expands inside the application container.
  paperless_remote "${socket}" exec --user 1000:1000 "${prefix}-paper-web" /bin/sh -ceu '
    for path in /usr/src/paperless/data /usr/src/paperless/media /usr/src/paperless/consume /usr/src/paperless/export; do
      test -r "$path" && test -w "$path"
      touch "$path/.boxferry-storage-probe"
      rm -f -- "$path/.boxferry-storage-probe"
    done
  '
}

paperless_recreate_application() {
  local mode=$1 socket=$2 prefix=$3 run=$4
  if [[ "${mode}" == cli ]]; then
    paperless_remote "${socket}" stop --time 30 \
      "${prefix}-paper-web" "${prefix}-paper-tika" "${prefix}-paper-gotenberg" \
      "${prefix}-paper-broker" "${prefix}-paper-db" > /dev/null
    paperless_remote "${socket}" rm --force --time 0 \
      "${prefix}-paper-web" "${prefix}-paper-tika" "${prefix}-paper-gotenberg" \
      "${prefix}-paper-broker" "${prefix}-paper-db" > /dev/null
    paperless_create_cli_runtime "${socket}" "${prefix}" "${run}"
  else
    paperless_compose_project "${socket}" "${prefix}" "${run}" \
      stop --timeout 30 > "${current_case}/paperless-compose-recreate.log" 2>&1
    paperless_compose_project "${socket}" "${prefix}" "${run}" \
      rm --force >> "${current_case}/paperless-compose-recreate.log" 2>&1
    paperless_compose_project "${socket}" "${prefix}" "${run}" \
      up --detach >> "${current_case}/paperless-compose-recreate.log" 2>&1
  fi
  paperless_wait_application "${socket}" "${prefix}"
}

paperless_assert_output_membership() {
  local selection=$1 output=$2 directory=$3 prefix=$4
  assert_named_member "${output}" "${directory}" webserver "${prefix}-paper-web"
  assert_resource_member "${output}" "${directory}" network "${prefix}-paper-backend"
  assert_resource_member "${output}" "${directory}" network "${prefix}-paper-edge"
  local volume
  for volume in data media consume export; do
    assert_resource_member "${output}" "${directory}" volume "${prefix}-paper-${volume}"
  done
  assert_named_member "${output}" "${directory}" db "${prefix}-paper-db"
  assert_named_member "${output}" "${directory}" broker "${prefix}-paper-broker"
  assert_named_member "${output}" "${directory}" gotenberg "${prefix}-paper-gotenberg"
  assert_named_member "${output}" "${directory}" tika "${prefix}-paper-tika"
  for volume in pgdata redisdata; do
    assert_resource_member "${output}" "${directory}" volume "${prefix}-paper-${volume}"
  done
}

paperless_assert_export_publication() {
  local output=$1 directory=$2 prefix=$3 count
  case "${output}" in
    compose)
      count="$(grep --count --extended-regexp '^ +published:' "${directory}/compose.yaml")"
      [[ "${count}" == 1 ]]
      awk -v web="${prefix}-paper-web" -v short="webserver" '
        function finish_port() {
          if (in_web && in_ports && in_port && target && published && host && protocol) matches++
          in_port = target = published = host = protocol = 0
        }
        $0 == "services:" { in_services = 1; next }
        in_services && /^[^ ]/ { finish_port(); in_services = 0 }
        in_services && /^  [^ ].*:$/ {
          finish_port()
          service = substr($0, 3, length($0) - 3)
          gsub(/^"|"$/, "", service)
          in_web = service == web || service == short
          in_ports = 0
        }
        in_web && $0 == "    ports:" { in_ports = 1; next }
        in_web && in_ports && /^    [^ ]/ { finish_port(); in_ports = 0 }
        in_web && in_ports && /^      - / {
          finish_port()
          in_port = 1
        }
        /^[[:space:]]+published: "18000"$/ {
          publications++
          if (in_web && in_ports && in_port) published = 1
        }
        in_web && in_ports && in_port && /target: 8000$/ { target = 1 }
        in_web && in_ports && in_port && /host_ip: 127\.0\.0\.1$/ { host = 1 }
        in_web && in_ports && in_port && /protocol: tcp$/ { protocol = 1 }
        END { finish_port(); exit !(publications == 1 && matches == 1) }
      ' "${directory}/compose.yaml"
      ;;
    quadlet)
      count="$(grep --recursive --count --include='*.container' '^PublishPort=' "${directory}" | awk -F: '{ total += $2 } END { print total + 0 }')"
      [[ "${count}" == 1 ]]
      grep --recursive --fixed-strings --quiet \
        'PublishPort=127.0.0.1:18000:8000/tcp' "${directory}"
      local web_unit=""
      local candidate
      for candidate in \
        "${directory}/webserver.container" \
        "${directory}/${prefix}-paper-web.container"; do
        if [[ -f "${candidate}" ]]; then
          web_unit=${candidate}
          break
        fi
      done
      [[ -n "${web_unit}" ]]
      grep --fixed-strings --line-regexp --quiet \
        'PublishPort=127.0.0.1:18000:8000/tcp' "${web_unit}"
      ;;
    podman)
      jq --exit-status --arg web "${prefix}-paper-web" '
        ([.operations[]? | select(
          .action == "create" and .resource.kind == "container") |
          (.cli.argv // [])[]? | select(. == "--publish" or . == "-p")] | length) == 1 and
        any(.operations[]?;
          .action == "create" and
          .resource.kind == "container" and
          .resource.name == $web and
          any((.cli.argv // [])[]?;
            . == "127.0.0.1:18000:8000/tcp" or . == "127.0.0.1:18000:8000"))
      ' "${directory}/podman.json" > /dev/null
      ;;
  esac
}

paperless_assert_export_networks() {
  local output=$1 directory=$2 prefix=$3 count
  case "${output}" in
    compose)
      count="$(grep --count --extended-regexp '^[[:space:]]+internal: true$' "${directory}/compose.yaml")"
      [[ "${count}" == 1 ]]
      awk -v backend="${prefix}-paper-backend" '
        $0 == "networks:" { in_networks = 1; next }
        in_networks && /^[^ ]/ { in_networks = 0 }
        in_networks && /^  [^ ].*:$/ {
          network = substr($0, 3, length($0) - 3)
          gsub(/^"|"$/, "", network)
        }
        in_networks && /^[[:space:]]+internal: true$/ {
          internals++
          if (network == backend) backend_internals++
        }
        END { exit !(internals == 1 && backend_internals == 1) }
      ' "${directory}/compose.yaml"
      ;;
    quadlet)
      count="$(grep --recursive --count --include='*.network' '^Internal=true$' "${directory}" | awk -F: '{ total += $2 } END { print total + 0 }')"
      [[ "${count}" == 1 ]]
      local backend_unit=""
      local candidate
      for candidate in \
        "${directory}/backend.network" \
        "${directory}/${prefix}-paper-backend.network"; do
        if [[ -f "${candidate}" ]]; then
          backend_unit=${candidate}
          break
        fi
      done
      [[ -n "${backend_unit}" ]]
      grep --fixed-strings --line-regexp --quiet 'Internal=true' "${backend_unit}"
      ;;
    podman)
      jq --exit-status --arg backend "${prefix}-paper-backend" --arg edge "${prefix}-paper-edge" '
        any(.operations[]?;
          .action == "create" and
          .resource.kind == "network" and
          .resource.name == $backend and
          any((.cli.argv // [])[]?; . == "--internal")) and
        any(.operations[]?;
          .action == "create" and
          .resource.kind == "network" and
          .resource.name == $edge and
          all((.cli.argv // [])[]?; . != "--internal"))
      ' "${directory}/podman.json" > /dev/null
      ;;
  esac
}

paperless_assert_web_literal() {
  local output=$1 directory=$2 prefix=$3 literal=$4
  case "${output}" in
    compose)
      awk -v web="${prefix}-paper-web" -v short="webserver" '
        $0 == "services:" { in_services = 1; next }
        in_services && /^[^ ]/ { in_services = 0; selected = 0 }
        in_services && /^  [^ ].*:$/ {
          service = substr($0, 3, length($0) - 3)
          gsub(/^"|"$/, "", service)
          selected = service == web || service == short
        }
        selected { print }
      ' "${directory}/compose.yaml" | grep --fixed-strings --quiet -- "${literal}"
      ;;
    quadlet)
      local web_unit=""
      local candidate
      for candidate in \
        "${directory}/webserver.container" \
        "${directory}/${prefix}-paper-web.container"; do
        if [[ -f "${candidate}" ]]; then
          web_unit=${candidate}
          break
        fi
      done
      [[ -n "${web_unit}" ]]
      grep --fixed-strings --quiet -- "${literal}" "${web_unit}"
      ;;
    podman)
      jq --exit-status --arg web "${prefix}-paper-web" --arg literal "${literal}" '
        ($literal | index("=")) as $separator |
        ($literal[0:$separator]) as $key |
        ($literal[$separator + 1:]) as $value |
        any(.operations[]?;
          .action == "create" and
          .resource.kind == "container" and
          .resource.name == $web and
          (. as $operation |
            any(range(0; (($operation.cli.argv // []) | length) - 1);
              (($operation.cli.argv[.] == "--env" or $operation.cli.argv[.] == "-e") and
               $operation.cli.argv[. + 1] == $literal)) and
            $operation.libpod.body.json.env[$key] == $value))
      ' "${directory}/podman.json" > /dev/null
      ;;
  esac
}

paperless_assert_podman_omissions() {
  local selection=$1 report=$2 directory=$3 prefix=$4
  local subject
  local -a subjects=(
    "networks.${prefix}-paper-backend.internal"
    "services.${prefix}-paper-web.ports"
  )
  local key
  for key in \
    PAPERLESS_REDIS PAPERLESS_DBHOST PAPERLESS_DBNAME PAPERLESS_DBUSER \
    PAPERLESS_DBPASS PAPERLESS_SECRET_KEY PAPERLESS_ADMIN_PASSWORD \
    PAPERLESS_TASK_WORKERS PAPERLESS_THREADS_PER_WORKER \
    PAPERLESS_WEBSERVER_WORKERS PAPERLESS_TIKA_ENABLED \
    PAPERLESS_CONSUMER_POLLING PAPERLESS_ADMIN_USER PAPERLESS_ADMIN_MAIL \
    PAPERLESS_URL PAPERLESS_TIKA_ENDPOINT PAPERLESS_TIKA_GOTENBERG_ENDPOINT \
    PAPERLESS_TIME_ZONE PAPERLESS_OCR_LANGUAGE USERMAP_UID USERMAP_GID; do
    subjects+=("services.${prefix}-paper-web.environment.${key}")
  done
  for key in POSTGRES_DB POSTGRES_USER POSTGRES_PASSWORD; do
    subjects+=("services.${prefix}-paper-db.environment.${key}")
  done

  for subject in "${subjects[@]}"; do
    jq --exit-status --arg subject "${subject}" '
      [.diagnostics[]? |
        select(
          .code == "BFP0007" and
          any(.fields[]?; .name == "subject" and .value == $subject)
        )] as $matches |
      ($matches | length) == 1 and
      ($matches[0] as $diagnostic |
        ($diagnostic.help | type == "string" and length > 0) and
        ([ $diagnostic.fields[]? |
          select(.name == "decision" and .value == "omitted") ] | length) == 1 and
        ([ $diagnostic.fields[]? |
          select(.name == "required_loss_policy" and .value == "partial") ] | length) == 1 and
        ([ $diagnostic.fields[]? |
          select(.name == "available_promotion" and .value == "none") ] | length) == 1 and
        ([ $diagnostic.fields[]? |
          select(.name == "reason" and (.value | type == "string" and length > 0)) ] | length) == 1 and
        ([ $diagnostic.fields[]? |
          select(.name == "remediation" and (.value | type == "string" and length > 0)) ] | length) == 1)
    ' "${report}" > /dev/null
  done

  jq --exit-status \
    --arg backend "${prefix}-paper-backend" \
    --arg web "${prefix}-paper-web" '
      all(.operations[]?;
        all((.cli.argv // [])[]?;
          . != "--env" and . != "-e" and . != "--publish" and . != "-p")) and
      all(.operations[]? |
        select(.action == "create" and .resource.kind == "container");
        (.libpod.body.json | has("env") | not)) and
      all(.operations[]? |
        select(
          .action == "create" and
          .resource.kind == "container" and
          .resource.name == $web
        );
        (.libpod.body.json |
          (has("portmappings") | not) and
          (has("port_mappings") | not) and
          (has("ports") | not))) and
      all(.operations[]? |
        select(
          .action == "create" and
          .resource.kind == "network" and
          .resource.name == $backend
        );
        (all((.cli.argv // [])[]?; . != "--internal") and
         (.libpod.body.json | has("internal") | not)))
    ' "${directory}/podman.json" > /dev/null

  if grep --recursive --fixed-strings --quiet \
    -e 'PAPERLESS_REDIS' -e 'PAPERLESS_DBHOST' -e 'PAPERLESS_DBNAME' \
    -e 'PAPERLESS_DBUSER' -e 'PAPERLESS_DBPASS' -e 'PAPERLESS_SECRET_KEY' \
    -e 'PAPERLESS_ADMIN_PASSWORD' -e 'PAPERLESS_TASK_WORKERS' \
    -e 'PAPERLESS_THREADS_PER_WORKER' -e 'PAPERLESS_WEBSERVER_WORKERS' \
    -e 'PAPERLESS_TIKA_ENABLED' -e 'PAPERLESS_CONSUMER_POLLING' \
    -e 'PAPERLESS_ADMIN_USER' -e 'PAPERLESS_ADMIN_MAIL' -e 'PAPERLESS_URL' \
    -e 'PAPERLESS_TIKA_ENDPOINT' -e 'PAPERLESS_TIKA_GOTENBERG_ENDPOINT' \
    -e 'PAPERLESS_TIME_ZONE' -e 'PAPERLESS_OCR_LANGUAGE' \
    -e 'USERMAP_UID' -e 'USERMAP_GID' -e 'POSTGRES_DB' -e 'POSTGRES_USER' \
    -e 'POSTGRES_PASSWORD' \
    -e "${PAPERLESS_ADMIN_PASSWORD}" -e "${PAPERLESS_DB_PASSWORD}" \
    -e "${PAPERLESS_SECRET_KEY}" "${directory}"; then
    printf 'Paperless Podman plan retained intentionally omitted environment intent.\n' >&2
    return 1
  fi

  # The broker canary is deliberately authored as a command argument, not an
  # environment value. Podman output retains commands, so its presence does not
  # contradict the independently asserted environment omission contract.
}

paperless_assert_output_semantics() {
  local selection=$1 output=$2 directory=$3 prefix=$4 report=$5
  if [[ "${output}" == podman ]]; then
    paperless_assert_podman_omissions \
      "${selection}" "${report}" "${directory}" "${prefix}"
    return
  fi
  local broker_host="${prefix}-paper-broker"
  local db_host="${prefix}-paper-db"
  local gotenberg_host="${prefix}-paper-gotenberg"
  local tika_host="${prefix}-paper-tika"
  local literal
  for literal in \
    "PAPERLESS_REDIS=redis://:${PAPERLESS_REDIS_PASSWORD}@${broker_host}:6379" \
    "PAPERLESS_DBHOST=${db_host}" \
    'PAPERLESS_DBNAME=paperless' \
    'PAPERLESS_DBUSER=paperless' \
    'PAPERLESS_DBPASS=' \
    'PAPERLESS_SECRET_KEY=' \
    'PAPERLESS_ADMIN_PASSWORD=' \
    'PAPERLESS_TASK_WORKERS=1' \
    'PAPERLESS_THREADS_PER_WORKER=1' \
    'PAPERLESS_WEBSERVER_WORKERS=1' \
    'PAPERLESS_TIKA_ENABLED=1' \
    'PAPERLESS_CONSUMER_POLLING=2' \
    'PAPERLESS_ADMIN_USER=boxferry-admin' \
    'PAPERLESS_ADMIN_MAIL=boxferry@example.invalid' \
    'PAPERLESS_URL=http://127.0.0.1:18000' \
    "PAPERLESS_TIKA_ENDPOINT=http://${tika_host}:9998" \
    "PAPERLESS_TIKA_GOTENBERG_ENDPOINT=http://${gotenberg_host}:3000" \
    'PAPERLESS_TIME_ZONE=Etc/UTC' \
    'PAPERLESS_OCR_LANGUAGE=eng' \
    'USERMAP_UID=1000' 'USERMAP_GID=1000'; do
    paperless_assert_web_literal "${output}" "${directory}" "${prefix}" "${literal}"
  done
  for literal in \
    '/usr/src/paperless/data' '/usr/src/paperless/media' \
    '/usr/src/paperless/consume' '/usr/src/paperless/export'; do
    grep --recursive --fixed-strings --quiet -- "${literal}" "${directory}"
  done
  grep --recursive --fixed-strings --quiet -- "$(paperless_image_reference paperless)" "${directory}"
  grep --recursive --fixed-strings --quiet -- "${prefix}-paper-backend" "${directory}"
  local volume
  for volume in data media consume export; do
    grep --recursive --fixed-strings --quiet -- "${prefix}-paper-${volume}" "${directory}"
  done
  paperless_assert_export_publication "${output}" "${directory}" "${prefix}"
  paperless_assert_export_networks "${output}" "${directory}" "${prefix}"
  for literal in \
    "$(paperless_image_reference postgres)" \
    "$(paperless_image_reference valkey)" \
    "$(paperless_image_reference gotenberg)" \
    "$(paperless_image_reference tika)"; do
    grep --recursive --fixed-strings --quiet -- "${literal}" "${directory}"
  done
  for volume in pgdata redisdata; do
    grep --recursive --fixed-strings --quiet -- "${prefix}-paper-${volume}" "${directory}"
  done
  grep --recursive --fixed-strings --quiet -- '/var/lib/postgresql' "${directory}"
  grep --recursive --fixed-strings --quiet -- '/data' "${directory}"
  grep --recursive --fixed-strings --quiet -- 'POSTGRES_PASSWORD=' "${directory}"
  grep --recursive --fixed-strings --quiet -- '--requirepass' "${directory}"
  grep --recursive --fixed-strings --quiet -- '--chromium-disable-javascript=true' "${directory}"
  grep --recursive --fixed-strings --quiet -- '--chromium-allow-list=file:///tmp/.*' "${directory}"
}

paperless_report_conversion_failure() {
  local report=$1
  if [[ ! -s "${report}" ]]; then
    printf 'Paperless conversion failed without a JSON report: %s\n' "${report}" >&2
    return
  fi
  if grep --fixed-strings --quiet \
    -e "${PAPERLESS_ADMIN_PASSWORD}" -e "${PAPERLESS_DB_PASSWORD}" \
    -e "${PAPERLESS_REDIS_PASSWORD}" -e "${PAPERLESS_SECRET_KEY}" \
    "${report}"; then
    printf 'Paperless conversion failure report withheld because redaction failed.\n' >&2
    return
  fi
  if ! jq empty "${report}" > /dev/null 2>&1; then
    printf 'Paperless conversion failed with an invalid JSON report: %s\n' "${report}" >&2
    return
  fi
  jq '{schema_version, status, exit_category, primary_diagnostic_code, diagnostics, fidelity}' \
    "${report}" >&2
}

paperless_run_exports() {
  local mode=$1 socket=$2 prefix=$3
  local selection output directory report
  local -a selection_arguments=()
  mkdir -p -- "${current_case}/outputs"
  for selection in exact label all; do
    case "${selection}" in
      exact) selection_arguments=(--podman-resource "container=${prefix}-paper-web") ;;
      label) selection_arguments=(--podman-label "io.boxferry.application=${prefix}-paperless") ;;
      all) selection_arguments=(--podman-all) ;;
    esac
    for output in compose quadlet podman; do
      directory="${current_case}/outputs/${mode}-${selection}-${output}"
      report="${directory}.report.json"
      local -a target_arguments=()
      [[ "${output}" == podman ]] && target_arguments+=(--podman-target-context rootless)
      if ! boxferry_operation "Paperless ${mode} ${selection} Podman-to-${output}" \
        convert podman "${output}" --podman-socket "${socket}" \
        --application-name "${prefix}-paperless" --loss-policy partial \
        --promote-podman-effective-named-volumes --promote-podman-effective-named-networks \
        --promote-podman-portable-effective-settings \
        --output-directory "${directory}" --console-format json \
        "${target_arguments[@]}" "${selection_arguments[@]}" > "${report}"; then
        paperless_report_conversion_failure "${report}"
        return 1
      fi
      jq --exit-status '
        .schema_version == 1 and .status == "success" and .exit_category == "success" and
        ([.diagnostics[]? | select(.severity == "error")] | length == 0) and
        ((.fidelity.invalid // 0) == 0) and (.output_artifacts | length > 0)
      ' "${report}" > /dev/null
      paperless_assert_output_membership \
        "${selection}" "${output}" "${directory}" "${prefix}"
      paperless_assert_output_semantics \
        "${selection}" "${output}" "${directory}" "${prefix}" "${report}"
    done
  done
  if grep --recursive --include='*.report.json' --fixed-strings --quiet \
    -e "${PAPERLESS_ADMIN_PASSWORD}" -e "${PAPERLESS_DB_PASSWORD}" \
    -e "${PAPERLESS_REDIS_PASSWORD}" -e "${PAPERLESS_SECRET_KEY}" \
    "${current_case}/outputs"; then
    printf 'Paperless BoxFerry reports leaked a public protected-value canary.\n' >&2
    return 1
  fi
}

paperless_capture_candidate() {
  local socket=$1 prefix=$2
  local output=${BOXFERRY_PAPERLESS_CAPTURE_DIRECTORY:?capture candidate path must be explicit}
  local capture_tool="${repository_root}/fixtures/conformance/podman-live/capture_proxy.py"
  local proxy_socket="${current_case}/paperless-capture-proxy.sock"

  python3 "${capture_tool}" \
    --repository "${repository_root}" \
    --output-directory "${output}" \
    --upstream-socket "${socket}" \
    --proxy-socket "${proxy_socket}" \
    --boxferry-bin "${boxferry_bin:?caller must supply boxferry_bin}" \
    --prefix "${prefix}"
}

paperless_clear_probe_state() {
  local outer=$1 prefix=$2
  engine_operation 'remove disposable Paperless document probe state' \
    exec "${outer}" podman unshare rm -rf -- "/tmp/boxferry-fixture/${prefix}/probe-state.json" \
    "/tmp/boxferry-fixture/${prefix}/generated-baseline" \
    "/tmp/boxferry-fixture/${prefix}/generated-second"
}

paperless_cleanup_mode() {
  local mode=$1 socket=$2 prefix=$3 run=$4 outer=$5
  if [[ "${mode}" == compose ]]; then
    paperless_compose_project "${socket}" "${prefix}" "${run}" \
      down --volumes --remove-orphans > "${current_case}/paperless-compose-down.log" 2>&1 || true
  fi
  paperless_remote "${socket}" stop --time 30 \
    "${prefix}-paper-web" "${prefix}-paper-tika" "${prefix}-paper-gotenberg" \
    "${prefix}-paper-broker" "${prefix}-paper-db" > /dev/null 2>&1 || true
  paperless_remote "${socket}" rm --force --time 0 --ignore \
    "${prefix}-paper-web" "${prefix}-paper-tika" "${prefix}-paper-gotenberg" \
    "${prefix}-paper-broker" "${prefix}-paper-db" > /dev/null 2>&1 || true
  paperless_remote "${socket}" volume rm --force \
    "${prefix}-paper-data" "${prefix}-paper-media" "${prefix}-paper-consume" \
    "${prefix}-paper-export" "${prefix}-paper-pgdata" \
    "${prefix}-paper-redisdata" > /dev/null 2>&1 || true
  paperless_remote "${socket}" network rm \
    "${prefix}-paper-backend" "${prefix}-paper-edge" > /dev/null 2>&1 || true
  paperless_clear_probe_state "${outer}" "${prefix}"
  paperless_assert_clean_prefix "${socket}" "${prefix}"
}

paperless_verify_target() {
  local id=$1 image=$2 declared_version=$3 distribution=$4 mode=$5 lane=$6 architecture=$7
  verify_observed_version "${id}" "${declared_version}" "${artifact_root}/${id}.podman-version"
  [[ "$(< "${artifact_root}/${id}.digest")" == "${image##*@}" ]] || {
    printf 'Pulled image digest does not match reviewed Paperless target %s.\n' "${id}" >&2
    return 1
  }
  [[ "${architecture}" == amd64 &&
    "$(< "${artifact_root}/${id}.architecture")" =~ ^(x86_64|amd64)$ ]] || {
    printf 'Observed architecture does not match Paperless target %s.\n' "${id}" >&2
    return 1
  }
  append_verified_evidence "${id}" "${image}" "${declared_version}" "${distribution}" \
    "${mode}" "${lane}" "${architecture}" "${artifact_root}/${id}" \
    nested-image paperless-application
}

run_paperless_application_cell() {
  local id=$1 image=$2 declared_version=$3 distribution=$4 mode=$5 lane=$6 architecture=$7
  [[ "${id}-${mode}" == podman-6.1-rootless-rootless ]] || {
    printf 'Paperless profile is bounded to reviewed Podman 6.1 rootless.\n' >&2
    return 1
  }
  current_prefix="${run_id:0:34}-paper"
  current_case="${artifact_root}/${id}-paperless-application"
  local socket_directory="${runtime_root}/paperless-application-target"
  local application_archive="${runtime_root}/paperless-application-images.tar"
  mkdir -p -- "${current_case}" "${socket_directory}"
  chmod 0700 "${current_case}" "${socket_directory}"
  progress_index=0
  progress_total=34
  if [[ -n "${BOXFERRY_PAPERLESS_CAPTURE_DIRECTORY:-}" ]]; then
    progress_total=$((progress_total + 1))
  fi
  printf '%s PLAN %s paperless-application tests=%d\n' \
    "$(timestamp)" "${id}" "${progress_total}"

  progress_run 'verify Paperless host resource budget' paperless_validate_resource_budget
  progress_run 'validate Paperless catalogues' paperless_validate_catalogues
  progress_run 'validate Docker Compose provider' paperless_validate_provider
  progress_run 'verify deterministic PDF DOCX ODT generation' paperless_verify_document_generator
  progress_run 'prepare bounded digest-pinned Paperless image archive' \
    paperless_prepare_image_archive "${application_archive}"
  progress_run 'start isolated Paperless Podman target' \
    start_outer_runtime "${id}" "${image}" "${mode}" "${socket_directory}" \
    "${application_archive}"
  progress_run 'verify Podman Paperless target evidence' \
    paperless_verify_target "${id}" "${image}" "${declared_version}" "${distribution}" \
    "${mode}" "${lane}" "${architecture}"
  local outer="${started_outer}"
  progress_run 'load Paperless images and rootless network configuration' \
    paperless_prepare_application_target "${outer}" "${current_prefix}" "${socket_directory}"
  local socket="${socket_directory}/podman.sock"

  progress_run 'provision independent Podman CLI Paperless application' \
    paperless_provision_cli "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'wait for Podman CLI Paperless API readiness' \
    paperless_wait_application "${socket}" "${current_prefix}"
  if [[ -n "${BOXFERRY_PAPERLESS_CAPTURE_DIRECTORY:-}" ]]; then
    progress_run 'capture sanitized Podman CLI Paperless evidence candidate' \
      paperless_capture_candidate "${socket}" "${current_prefix}"
  fi
  progress_run 'refuse colliding Podman CLI Paperless prefix' \
    paperless_expect_collision "${socket}" "${current_prefix}"
  progress_run 'ingest searchable PDF DOCX ODT through Podman CLI topology' \
    paperless_ingest_phase "${socket}" "${current_prefix}" baseline
  progress_run 'prove Podman CLI PostgreSQL document rows' \
    paperless_assert_database "${socket}" "${current_prefix}" 3
  progress_run 'prove Podman CLI private services publication and storage' \
    paperless_assert_application_boundaries "${socket}" "${current_prefix}"
  progress_run 'prove Podman CLI storage ownership modes and application access' \
    paperless_assert_storage_permissions "${socket}" "${current_prefix}"
  progress_run 'exercise Podman CLI exact label all exports without execution' \
    paperless_run_exports cli "${socket}" "${current_prefix}"
  progress_run 'recreate Podman CLI containers without deleting volumes' \
    paperless_recreate_application cli "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'search retrieve converter evidence after Podman CLI recreation' \
    paperless_verify_documents "${socket}" "${current_prefix}"
  progress_run 'ingest second Podman CLI document set with broker activity' \
    paperless_ingest_phase "${socket}" "${current_prefix}" second
  progress_run 'prove six persistent Podman CLI PostgreSQL document rows' \
    paperless_assert_database "${socket}" "${current_prefix}" 6
  progress_run 'clean prefix-scoped Podman CLI Paperless resources' \
    paperless_cleanup_mode cli "${socket}" "${current_prefix}" "${run_id}" "${outer}"

  progress_run 'provision independent Docker Compose Paperless application' \
    paperless_provision_compose "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'wait for Docker Compose Paperless API readiness' \
    paperless_wait_application "${socket}" "${current_prefix}"
  progress_run 'refuse colliding Docker Compose Paperless prefix' \
    paperless_expect_collision "${socket}" "${current_prefix}"
  progress_run 'ingest searchable PDF DOCX ODT through Compose topology' \
    paperless_ingest_phase "${socket}" "${current_prefix}" baseline
  progress_run 'prove Compose PostgreSQL private publication storage boundaries' \
    paperless_assert_database "${socket}" "${current_prefix}" 3
  progress_run 'prove Compose private services publication and storage ownership' \
    paperless_assert_application_boundaries "${socket}" "${current_prefix}"
  progress_run 'prove Compose storage modes and application read-write access' \
    paperless_assert_storage_permissions "${socket}" "${current_prefix}"
  progress_run 'exercise Compose-provisioned exact label all exports without execution' \
    paperless_run_exports compose "${socket}" "${current_prefix}"
  progress_run 'recreate Compose containers without deleting volumes' \
    paperless_recreate_application compose "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'search retrieve converter evidence after Compose recreation' \
    paperless_verify_documents "${socket}" "${current_prefix}"
  progress_run 'ingest second Compose document set and prove six database rows' \
    paperless_ingest_phase "${socket}" "${current_prefix}" second
  paperless_assert_database "${socket}" "${current_prefix}" 6
  progress_run 'clean prefix-scoped Docker Compose Paperless resources' \
    paperless_cleanup_mode compose "${socket}" "${current_prefix}" "${run_id}" "${outer}"
  progress_run 'remove disposable Paperless outer container' remove_outer "${outer}"
  printf '%s CELL PASS %s paperless-application (%d/%d tests)\n' \
    "$(timestamp)" "${id}" "${progress_index}" "${progress_total}"
}
