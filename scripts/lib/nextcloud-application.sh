#!/usr/bin/env bash
# Nextcloud application acceptance helpers. This file is sourced by the live
# entry point; it never executes a scenario by itself.
# shellcheck disable=SC2129,SC2154 # Caller globals and phased logs are intentional.

readonly NEXTCLOUD_ADMIN_PASSWORD="boxferry-public-admin-canary"
readonly NEXTCLOUD_DB_PASSWORD="boxferry-public-database-canary"
readonly NEXTCLOUD_REDIS_PASSWORD="boxferry-public-cache-canary"
readonly NEXTCLOUD_PROVIDER_VERSION="5.5.0"
readonly NEXTCLOUD_PROVIDER_SHA256="c57ab918abd5b05ca7e7d0f275875dd1330a695074f309dc9eab1b49efafcd4b"

nextcloud_fixture_root() {
  printf '%s/fixtures/conformance/nextcloud-application\n' "${repository_root:?caller must supply repository_root}"
}

nextcloud_image_reference() {
  local expected=$1
  printf 'registry.invalid/boxferry-test/nextcloud-application:%s\n' "${expected}"
}

nextcloud_validate_catalogues() {
  local fixture
  fixture="$(nextcloud_fixture_root)"
  awk -F '\t' '
 NF && $1 !~ /^#/ {
 if (NF != 6 || $2 !~ /:[^/@]+@sha256:[0-9a-f]{64}$/ || $3 == "" || $4 == "" ||
 $5 !~ /^https:\/\// || $6 != "transient-test-pull" || seen[$1]++) {
 printf "Invalid Nextcloud image row at line %d: %s\\n", NR, $0 > "/dev/stderr"
 bad = 1
 }
 count++
 }
 END { exit bad || count != 4 }
 ' "${fixture}/images.tsv"
  awk -F '\t' -v version="${NEXTCLOUD_PROVIDER_VERSION}" -v sha="${NEXTCLOUD_PROVIDER_SHA256}" '
 NF && $1 !~ /^#/ {
 count++
 if (NF != 7 || $1 != "docker-compose" || $2 != version ||
 $3 != "https://github.com/docker/compose/releases/download/v5.5.0/docker-compose-linux-x86_64" ||
 $4 != sha || $5 != "Apache-2.0" || $6 != "https://github.com/docker/compose" ||
 $7 != "downloaded-test-tool") {
 printf "Invalid Nextcloud provider row at line %d: %s\\n", NR, $0 > "/dev/stderr"
 bad = 1
 }
 }
 END { exit bad || count != 1 }
 ' "${fixture}/providers.tsv"
  for file in compose.yaml frontend.conf proxy.conf second-index.html published-probe.php webdav-probe.php README.md; do
    [[ -s "${fixture}/${file}" ]]
  done
}

nextcloud_validate_provider() {
  local provider=${BOXFERRY_COMPOSE_BIN:-${repository_root}/target/tools/docker-compose}
  [[ -x "${provider}" ]] || {
    printf 'Nextcloud application profile requires executable Docker Compose %s at %s.\n' \
      "${NEXTCLOUD_PROVIDER_VERSION}" "${provider}" >&2
    return 1
  }
  local observed_sha observed_version
  observed_sha="$(sha256sum "${provider}" | awk '{ print $1 }')"
  [[ "${observed_sha}" == "${NEXTCLOUD_PROVIDER_SHA256}" ]] || {
    printf 'Docker Compose checksum mismatch: expected %s, observed %s.\n' \
      "${NEXTCLOUD_PROVIDER_SHA256}" "${observed_sha}" >&2
    return 1
  }
  observed_version="$("${provider}" version --short | sed 's/^v//')"
  [[ "${observed_version}" == "${NEXTCLOUD_PROVIDER_VERSION}" ]] || {
    printf 'Docker Compose version mismatch: expected %s, observed %s.\n' \
      "${NEXTCLOUD_PROVIDER_VERSION}" "${observed_version}" >&2
    return 1
  }
}

nextcloud_prepare_image_archive() {
  local archive=${1:?archive path required}
  [[ -s "${archive}" ]] && return 0
  local fixture id reference
  local expected_digest observed_digest cache_status
  local runtime_reference
  local -a references=()
  fixture="$(nextcloud_fixture_root)"
  while IFS=$'\t' read -r id reference _ _ _ _; do
    [[ -z "${id}" || "${id}" == \#* ]] && continue
    expected_digest="${reference##*@}"
    cache_status=0
    engine_image_available "probe Nextcloud ${id} image cache" "${reference}" || cache_status=$?
    if ((cache_status == 1)); then
      timed_operation 8m "pull digest-pinned Nextcloud ${id} image" \
        "${engine}" pull --quiet "${reference}" \
        > "${artifact_root}/nextcloud-${id}.pull.log"
      record_run_owned_host_image "${reference}"
    elif ((cache_status != 0)); then
      return "${cache_status}"
    fi
    observed_digest="$(engine_operation "inspect Nextcloud ${id} image digest" \
      image inspect --format '{{.Digest}}' "${reference}")"
    [[ "${observed_digest}" == "${expected_digest}" ]] || {
      printf 'Nextcloud image digest mismatch for %s: expected %s, observed %s.\n' \
        "${id}" "${expected_digest}" "${observed_digest}" >&2
      return 1
    }
    runtime_reference="$(nextcloud_image_reference "${id}")"
    record_run_owned_archive_alias "${reference}" "${runtime_reference}" "Nextcloud ${id}"
    references+=("${runtime_reference}")
  done < "${fixture}/images.tsv"
  timed_operation 10m 'archive digest-pinned Nextcloud images for nested loading' \
    "${engine}" save --multi-image-archive --format docker-archive \
    --output "${archive}" "${references[@]}"
  chmod 0644 "${archive}"
  for runtime_reference in "${references[@]}"; do
    release_run_owned_host_image "${runtime_reference}"
  done
  while IFS=$'\t' read -r id reference _ _ _ _; do
    [[ -z "${id}" || "${id}" == \#* ]] && continue
    release_run_owned_host_image "${reference}"
  done < "${fixture}/images.tsv"
}

nextcloud_assert_loaded_images() {
  local outer=$1
  local fixture id reference
  fixture="$(nextcloud_fixture_root)"
  while IFS=$'\t' read -r id reference _ _ _ _; do
    [[ -z "${id}" || "${id}" == \#* ]] && continue
    engine_operation "verify offline Nextcloud ${id} image alias" \
      exec "${outer}" podman image inspect "$(nextcloud_image_reference "${id}")" > /dev/null
  done < "${fixture}/images.tsv"
}

nextcloud_load_image_archive() {
  local outer=$1
  timed_operation 10m 'load Nextcloud images without nested registry access' \
    "${engine}" exec "${outer}" podman load --input /boxferry-workload.tar
  nextcloud_assert_loaded_images "${outer}"
}

nextcloud_remote() {
  local socket=$1
  shift
  podman_socket "${socket}" "Nextcloud ${1:-command}" "$@"
}

nextcloud_wait_for() {
  local deadline_seconds=$1 description=$2
  shift 2
  local started
  started="$(date +%s)"
  until "$@" > /dev/null 2>&1; do
    if (($(date +%s) - started >= deadline_seconds)); then
      printf 'Timed out waiting for Nextcloud %s after %ss.\n' "${description}" "${deadline_seconds}" >&2
      return 1
    fi
    sleep 2
  done
}

nextcloud_container_stopped() {
  local socket=$1 container=$2 observed
  observed="$(nextcloud_remote "${socket}" inspect --format '{{.State.Running}}' "${container}")"
  [[ "${observed}" == false ]]
}

nextcloud_container_completed() {
  local socket=$1 container=$2 observed
  observed="$(nextcloud_remote "${socket}" inspect --format '{{.State.Status}} {{.State.ExitCode}}' "${container}")"
  [[ "${observed}" == 'exited 0' ]]
}

nextcloud_copy_configs() {
  local outer=$1 prefix=$2 fixture destination
  fixture="$(nextcloud_fixture_root)"
  destination="/tmp/boxferry-fixture/${prefix}"
  engine_operation 'create disposable Nextcloud fixture directory' \
    exec "${outer}" mkdir -p -- "${destination}"
  engine_operation 'copy reviewed Nextcloud frontend configuration' \
    cp "${fixture}/frontend.conf" "${outer}:${destination}/frontend.conf"
  engine_operation 'copy reviewed Nextcloud proxy configuration' \
    cp "${fixture}/proxy.conf" "${outer}:${destination}/proxy.conf"
  engine_operation "copy reviewed second application content" cp "${fixture}/second-index.html" "${outer}:${destination}/second-index.html"
  engine_operation "copy reviewed WebDAV probe" cp "${fixture}/webdav-probe.php" "${outer}:${destination}/webdav-probe.php"
  engine_operation "copy reviewed publication probe" cp "${fixture}/published-probe.php" "${outer}:${destination}/published-probe.php"
}

nextcloud_create_edge_network() {
  local socket=$1 prefix=$2 run=$3
  nextcloud_remote "${socket}" network exists "${prefix}-shared-edge" 2> /dev/null ||
    nextcloud_remote "${socket}" network create \
      --label "io.boxferry.live-run=${run}" --label io.boxferry.shared=true \
      "${prefix}-shared-edge" > /dev/null
}

nextcloud_assert_clean_prefix() {
  local socket=$1 prefix=$2 listing
  local -a collisions=()
  listing="$(
    nextcloud_remote "${socket}" container ps --all --format '{{.Names}}' || exit $?
    nextcloud_remote "${socket}" network ls --format '{{.Name}}' || exit $?
    nextcloud_remote "${socket}" volume ls --format '{{.Name}}'
  )" || {
    printf 'Could not inspect existing resources for Nextcloud prefix %s.\n' "${prefix}" >&2
    return 1
  }
  mapfile -t collisions < <(printf '%s\n' "${listing}" | awk -v prefix="${prefix}-" 'index($0, prefix) == 1')
  if ((${#collisions[@]} > 0)); then
    printf 'Refusing Nextcloud provisioning because prefix %s already owns resources:\n' "${prefix}" >&2
    printf '  %s\n' "${collisions[@]}" >&2
    return 1
  fi
}

nextcloud_create_cli_database() {
  local socket=$1 prefix=$2 run=$3 image=$4
  nextcloud_remote "${socket}" run --pull=never --detach --name "${prefix}-cloud-db" \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-cloud" \
    --network "${prefix}-cloud-backend:alias=db" \
    --volume "${prefix}-cloud-database:/var/lib/postgresql/data" \
    --env POSTGRES_DB=nextcloud --env POSTGRES_USER=boxferry \
    --env "POSTGRES_PASSWORD=${NEXTCLOUD_DB_PASSWORD}" \
    --health-cmd 'pg_isready -U boxferry -d nextcloud' \
    --health-interval 2s --health-retries 60 \
    "${image}" > /dev/null
}

nextcloud_provision_cli() {
  local socket=$1 prefix=$2 run=$3 fixture_root=$4
  nextcloud_assert_clean_prefix "${socket}" "${prefix}"
  local nextcloud postgres redis nginx
  nextcloud="$(nextcloud_image_reference nextcloud)"
  postgres="$(nextcloud_image_reference postgres)"
  redis="$(nextcloud_image_reference redis)"
  nginx="$(nextcloud_image_reference nginx)"
  local -a app_labels=(
    --label "io.boxferry.live-run=${run}"
    --label "io.boxferry.application=${prefix}-cloud"
  )
  nextcloud_create_edge_network "${socket}" "${prefix}" "${run}"
  nextcloud_remote "${socket}" network create --internal "${app_labels[@]}" \
    "${prefix}-cloud-backend" > /dev/null
  nextcloud_remote "${socket}" volume create "${app_labels[@]}" \
    "${prefix}-cloud-database" > /dev/null
  nextcloud_remote "${socket}" volume create "${app_labels[@]}" \
    "${prefix}-cloud-redis" > /dev/null
  nextcloud_remote "${socket}" volume create "${app_labels[@]}" \
    "${prefix}-cloud-nextcloud" > /dev/null
  nextcloud_create_cli_database "${socket}" "${prefix}" "${run}" "${postgres}"
  nextcloud_remote "${socket}" run --pull=never --detach --name "${prefix}-cloud-cache" "${app_labels[@]}" \
    --network "${prefix}-cloud-backend:alias=cache" \
    --volume "${prefix}-cloud-redis:/data" \
    --health-cmd "redis-cli -a ${NEXTCLOUD_REDIS_PASSWORD} ping" \
    --health-interval 2s --health-retries 60 \
    "${redis}" redis-server --requirepass "${NEXTCLOUD_REDIS_PASSWORD}" > /dev/null
  nextcloud_wait_for 180 'PostgreSQL readiness' nextcloud_remote "${socket}" \
    exec "${prefix}-cloud-db" pg_isready -U boxferry -d nextcloud
  nextcloud_wait_for 120 'Redis readiness' nextcloud_remote "${socket}" \
    exec "${prefix}-cloud-cache" redis-cli -a "${NEXTCLOUD_REDIS_PASSWORD}" ping
  nextcloud_create_cli_app "${socket}" "${prefix}" "${run}" "${nextcloud}"
  nextcloud_wait_for 600 'Nextcloud initialization' nextcloud_remote "${socket}" \
    exec --user www-data "${prefix}-cloud-app" php occ status
  nextcloud_create_cli_init "${socket}" "${prefix}" "${run}" "${nextcloud}" \
    "${current_case}/cli-init.log"
  nextcloud_create_cli_cron "${socket}" "${prefix}" "${run}" "${nextcloud}"
  nextcloud_create_cli_frontend "${socket}" "${prefix}" "${run}" "${fixture_root}" "${nginx}"
  nextcloud_remote "${socket}" run --pull=never --detach --name "${prefix}-second-app" \
    --label "io.boxferry.live-run=${run}" --label "io.boxferry.application=${prefix}-second" \
    --network "${prefix}-shared-edge:alias=second" \
    --volume "${fixture_root}/second-index.html:/usr/share/nginx/html/index.html:ro" "${nginx}" > /dev/null
  nextcloud_create_cli_proxy "${socket}" "${prefix}" "${run}" "${fixture_root}" "${nginx}"
}

nextcloud_create_cli_app() {
  local socket=$1 prefix=$2 run=$3 image=$4
  nextcloud_remote "${socket}" run --pull=never --detach --name "${prefix}-cloud-app" \
    --label "io.boxferry.live-run=${run}" --label "io.boxferry.application=${prefix}-cloud" \
    --requires "${prefix}-cloud-db,${prefix}-cloud-cache" \
    --network "${prefix}-cloud-backend:alias=app" \
    --volume "${prefix}-cloud-nextcloud:/var/www/html" \
    --env POSTGRES_HOST=db --env POSTGRES_DB=nextcloud --env POSTGRES_USER=boxferry \
    --env "POSTGRES_PASSWORD=${NEXTCLOUD_DB_PASSWORD}" --env REDIS_HOST=cache \
    --env "REDIS_HOST_PASSWORD=${NEXTCLOUD_REDIS_PASSWORD}" \
    --env NEXTCLOUD_ADMIN_USER=boxferry-admin \
    --env "NEXTCLOUD_ADMIN_PASSWORD=${NEXTCLOUD_ADMIN_PASSWORD}" \
    --env 'NEXTCLOUD_TRUSTED_DOMAINS=cloud.example.invalid frontend proxy localhost' \
    --health-cmd "php -r '\$s=json_decode(@file_get_contents(\"http://localhost/status.php\"),true); exit(is_array(\$s)&&(\$s[\"installed\"]??false)&&!(\$s[\"maintenance\"]??true)&&!(\$s[\"needsDbUpgrade\"]??true)?0:1);'" \
    --health-interval 5s --health-retries 120 "${image}" > /dev/null
}

nextcloud_create_cli_init() {
  local socket=$1 prefix=$2 run=$3 image=$4 log=$5
  nextcloud_remote "${socket}" create --pull=never --user www-data --name "${prefix}-cloud-init" \
    --label "io.boxferry.live-run=${run}" --label "io.boxferry.application=${prefix}-cloud" \
    --requires "${prefix}-cloud-app" --network "${prefix}-cloud-backend" \
    --volume "${prefix}-cloud-nextcloud:/var/www/html" \
    "${image}" php /var/www/html/occ status > /dev/null
  nextcloud_remote "${socket}" start --attach "${prefix}-cloud-init" > "${log}"
}

nextcloud_create_cli_cron() {
  local socket=$1 prefix=$2 run=$3 image=$4
  nextcloud_remote "${socket}" run --pull=never --detach --name "${prefix}-cloud-cron" \
    --label "io.boxferry.live-run=${run}" --label "io.boxferry.application=${prefix}-cloud" \
    --requires "${prefix}-cloud-app" --network "${prefix}-cloud-backend" \
    --volume "${prefix}-cloud-nextcloud:/var/www/html" --entrypoint /cron.sh \
    "${image}" > /dev/null
}

nextcloud_create_cli_frontend() {
  local socket=$1 prefix=$2 run=$3 fixture_root=$4 image=$5
  nextcloud_remote "${socket}" run --pull=never --detach --name "${prefix}-cloud-frontend" \
    --label "io.boxferry.live-run=${run}" --label "io.boxferry.application=${prefix}-cloud" \
    --requires "${prefix}-cloud-app" --network "${prefix}-cloud-backend:alias=frontend" \
    --network "${prefix}-shared-edge:alias=frontend" --volume "${prefix}-cloud-nextcloud:/var/www/html:ro" \
    --volume "${fixture_root}/frontend.conf:/etc/nginx/conf.d/default.conf:ro" \
    "${image}" > /dev/null
}

nextcloud_create_cli_proxy() {
  local socket=$1 prefix=$2 run=$3 fixture_root=$4 image=$5
  nextcloud_remote "${socket}" run --pull=never --detach --name "${prefix}-shared-proxy" \
    --label "io.boxferry.live-run=${run}" --label io.boxferry.shared=true \
    --network "${prefix}-shared-edge:alias=proxy" --publish 127.0.0.1:18443:8080 \
    --volume "${fixture_root}/proxy.conf:/etc/nginx/conf.d/default.conf:ro" \
    "${image}" > /dev/null
}

nextcloud_compose_project() {
  local socket=$1 prefix=$2 run=$3 project=$4
  shift 4
  local provider=${BOXFERRY_COMPOSE_BIN:-${repository_root}/target/tools/docker-compose}
  local fixture compose_file fixture_root
  fixture="$(nextcloud_fixture_root)"
  compose_file="${fixture}/compose.yaml"
  fixture_root="/tmp/boxferry-fixture/${prefix}"
  timed_operation 12m "Docker Compose ${project} ${1:-command}" env \
    DOCKER_HOST="unix://${socket}" \
    BF_PREFIX="${prefix}" BF_RUN="${run}" BF_FIXTURE_ROOT="${fixture_root}" \
    BF_DB_PASSWORD="${NEXTCLOUD_DB_PASSWORD}" BF_REDIS_PASSWORD="${NEXTCLOUD_REDIS_PASSWORD}" \
    BF_ADMIN_PASSWORD="${NEXTCLOUD_ADMIN_PASSWORD}" \
    BF_NEXTCLOUD_IMAGE="$(nextcloud_image_reference nextcloud)" \
    BF_POSTGRES_IMAGE="$(nextcloud_image_reference postgres)" \
    BF_REDIS_IMAGE="$(nextcloud_image_reference redis)" \
    BF_NGINX_IMAGE="$(nextcloud_image_reference nginx)" \
    "${provider}" --project-name "${prefix}-${project}" --file "${compose_file}" "$@"
}

nextcloud_compose() {
  local socket=$1 prefix=$2 run=$3
  shift 3
  nextcloud_compose_project "${socket}" "${prefix}" "${run}" cloud "$@"
}

nextcloud_provision_compose() {
  local socket=$1 prefix=$2 run=$3
  nextcloud_assert_clean_prefix "${socket}" "${prefix}"
  nextcloud_create_edge_network "${socket}" "${prefix}" "${run}"
  nextcloud_compose "${socket}" "${prefix}" "${run}" up --detach --no-deps --remove-orphans \
    db cache \
    > "${current_case}/compose-provider.log" 2>&1
  nextcloud_wait_for 180 'Compose PostgreSQL readiness' nextcloud_remote "${socket}" \
    exec "${prefix}-cloud-db" pg_isready -U boxferry -d nextcloud
  nextcloud_wait_for 120 'Compose Redis readiness' nextcloud_remote "${socket}" \
    exec "${prefix}-cloud-cache" redis-cli -a "${NEXTCLOUD_REDIS_PASSWORD}" ping
  nextcloud_compose "${socket}" "${prefix}" "${run}" up --detach --no-deps app \
    >> "${current_case}/compose-provider.log" 2>&1
  nextcloud_wait_for 600 'Compose Nextcloud initialization' nextcloud_remote "${socket}" \
    exec --user www-data "${prefix}-cloud-app" php occ status
  nextcloud_compose "${socket}" "${prefix}" "${run}" up --detach --no-deps init \
    >> "${current_case}/compose-provider.log" 2>&1
  nextcloud_wait_for 120 'Compose initialization role completion' \
    nextcloud_container_completed "${socket}" "${prefix}-cloud-init"
  nextcloud_compose "${socket}" "${prefix}" "${run}" up --detach --no-deps cron frontend \
    >> "${current_case}/compose-provider.log" 2>&1
  nextcloud_compose_project "${socket}" "${prefix}" "${run}" second \
    up --detach --no-deps --remove-orphans second \
    >> "${current_case}/compose-provider.log" 2>&1
  nextcloud_compose_project "${socket}" "${prefix}" "${run}" shared \
    up --detach --no-deps --remove-orphans proxy \
    >> "${current_case}/compose-provider.log" 2>&1
}

nextcloud_wait_application() {
  local socket=$1 prefix=$2
  nextcloud_wait_for 600 'Nextcloud application status' nextcloud_remote "${socket}" \
    exec --user www-data "${prefix}-cloud-app" php occ status
  nextcloud_wait_for 180 'frontend response' nextcloud_remote "${socket}" \
    exec "${prefix}-cloud-frontend" wget -T 30 --quiet --spider http://127.0.0.1:8080/status.php
  nextcloud_wait_for 180 'edge proxy response' nextcloud_remote "${socket}" \
    exec "${prefix}-shared-proxy" wget -T 30 --quiet --spider http://127.0.0.1:8080/status.php
  nextcloud_remote "${socket}" exec --user www-data "${prefix}-cloud-app" \
    php occ status --output=json | jq --exit-status \
    '.installed == true and .maintenance == false and .needsDbUpgrade == false and .versionstring == "32.0.10"' > /dev/null
  nextcloud_remote "${socket}" exec "${prefix}-shared-proxy" \
    wget -T 30 --quiet --output-document=- http://127.0.0.1:8080/status.php | jq --exit-status \
    '.installed == true and .maintenance == false and .needsDbUpgrade == false and .versionstring == "32.0.10"' > /dev/null
}

nextcloud_webdav_round_trip() {
  local socket=$1 prefix=$2 phase=$3
  local upload=${4:-true}
  local fixture_root="/tmp/boxferry-fixture/${prefix}"
  local payload="boxferry-nextcloud-${phase}-payload"
  nextcloud_remote "${socket}" run --rm --pull=never \
    --network "${prefix}-shared-edge" \
    --volume "${fixture_root}/webdav-probe.php:/boxferry-webdav-probe.php:ro" \
    --env "BF_PASSWORD=${NEXTCLOUD_ADMIN_PASSWORD}" --env "BF_PAYLOAD=${payload}" \
    --env "BF_UPLOAD=${upload}" --entrypoint php \
    "$(nextcloud_image_reference nextcloud)" /boxferry-webdav-probe.php
}

nextcloud_assert_database_cache_and_shared_state() {
  local socket=$1 prefix=$2
  nextcloud_remote "${socket}" exec --env "PGPASSWORD=${NEXTCLOUD_DB_PASSWORD}" \
    "${prefix}-cloud-db" psql -U boxferry -d nextcloud -Atc \
    "SELECT count(*) FROM oc_filecache WHERE path LIKE '%boxferry-live.txt';" |
    awk '$1 >= 1 { found = 1 } END { exit !found }'
  [[ "$(nextcloud_remote "${socket}" exec --user www-data "${prefix}-cloud-app" \
    php occ config:system:get redis host)" == cache ]]
  nextcloud_remote "${socket}" exec "${prefix}-cloud-cache" redis-cli \
    -a "${NEXTCLOUD_REDIS_PASSWORD}" INFO stats |
    awk -F: '
 /^keyspace_hits:/ || /^keyspace_misses:/ { gsub(/\r/, "", $2); total += $2 }
 END { exit total > 0 ? 0 : 1 }
 '
  nextcloud_remote "${socket}" inspect "${prefix}-cloud-cache" |
    jq --exit-status --arg volume "${prefix}-cloud-redis" \
      'any(.[0].Mounts[]?; .Name == $volume and .Destination == "/data")' > /dev/null
  for container in app cron frontend; do
    nextcloud_remote "${socket}" exec "${prefix}-cloud-${container}" \
      test -s /var/www/html/config/config.php
  done
  nextcloud_remote "${socket}" inspect "${prefix}-cloud-init" |
    jq --exit-status --arg volume "${prefix}-cloud-nextcloud" '
 any(.[0].Mounts[]?; .Name == $volume and .Destination == "/var/www/html")
 ' > /dev/null
}

nextcloud_assert_publication_and_networks() {
  local socket=$1 prefix=$2
  [[ "$(nextcloud_remote "${socket}" port "${prefix}-shared-proxy" 8080/tcp)" == "127.0.0.1:18443" ]]
  for container in db cache app init cron frontend; do
    [[ -z "$(nextcloud_remote "${socket}" port "${prefix}-cloud-${container}" 2> /dev/null || true)" ]]
  done
  [[ -z "$(nextcloud_remote "${socket}" port "${prefix}-second-app" 2> /dev/null || true)" ]]
  nextcloud_remote "${socket}" network inspect "${prefix}-cloud-backend" |
    jq --exit-status '.[0].internal == true' > /dev/null
  nextcloud_remote "${socket}" network inspect "${prefix}-shared-edge" |
    jq --exit-status '.[0].internal == false' > /dev/null
  nextcloud_remote "${socket}" inspect \
    "${prefix}-cloud-db" "${prefix}-cloud-cache" "${prefix}-cloud-app" "${prefix}-cloud-init" "${prefix}-cloud-cron" |
    jq --exit-status --arg backend "${prefix}-cloud-backend" --arg edge "${prefix}-shared-edge" '
 all(.[]; (.NetworkSettings.Networks[$backend] != null) and (.NetworkSettings.Networks[$edge] == null))
 ' > /dev/null
  nextcloud_remote "${socket}" inspect "${prefix}-shared-proxy" "${prefix}-second-app" |
    jq --exit-status --arg backend "${prefix}-cloud-backend" --arg edge "${prefix}-shared-edge" '
 all(.[]; (.NetworkSettings.Networks[$edge] != null) and (.NetworkSettings.Networks[$backend] == null))
 ' > /dev/null
  nextcloud_remote "${socket}" inspect "${prefix}-cloud-frontend" |
    jq --exit-status --arg backend "${prefix}-cloud-backend" --arg edge "${prefix}-shared-edge" '
 (.[0].NetworkSettings.Networks[$backend] != null) and (.[0].NetworkSettings.Networks[$edge] != null)
 ' > /dev/null
}

nextcloud_assert_published_routes() {
  local socket=$1 prefix=$2
  local fixture_root="/tmp/boxferry-fixture/${prefix}"
  if ! nextcloud_remote "${socket}" run --rm --pull=never --network host \
    --volume "${fixture_root}/published-probe.php:/boxferry-published-probe.php:ro" \
    --entrypoint php "$(nextcloud_image_reference nextcloud)" /boxferry-published-probe.php; then
    nextcloud_remote "${socket}" logs "${prefix}-shared-proxy" >&2 || true
    return 1
  fi
}

nextcloud_prepare_application_target() {
  local outer=$1 prefix=$2 socket_directory=$3
  engine_operation "copy rootless Nextcloud network configuration" cp "${repository_root}/fixtures/conformance/podman-live/apply-target-containers.conf" "${outer}:/tmp/99-boxferry-live.conf"
  # shellcheck disable=SC2016 # $HOME expands inside the nested target.
  engine_operation "prepare rootless Nextcloud network configuration" exec "${outer}" /bin/sh -ceu 'mkdir -p "$HOME/.config/containers/containers.conf.d"; cp /tmp/99-boxferry-live.conf "$HOME/.config/containers/containers.conf.d/99-boxferry-live.conf"'
  nextcloud_load_image_archive "${outer}"
  nextcloud_copy_configs "${outer}" "${prefix}"
  activate_outer_runtime "${socket_directory}"
}

nextcloud_assert_application_behavior() {
  local socket=$1 prefix=$2 phase=$3
  nextcloud_webdav_round_trip "${socket}" "${prefix}" "${phase}"
  nextcloud_assert_database_cache_and_shared_state "${socket}" "${prefix}"
  nextcloud_assert_publication_and_networks "${socket}" "${prefix}"
  nextcloud_assert_published_routes "${socket}" "${prefix}"
}

nextcloud_seed_database_canary() {
  local socket=$1 prefix=$2
  nextcloud_remote "${socket}" exec --env "PGPASSWORD=${NEXTCLOUD_DB_PASSWORD}" \
    "${prefix}-cloud-db" psql -v ON_ERROR_STOP=1 -U boxferry -d nextcloud -c \
    "CREATE TABLE IF NOT EXISTS boxferry_acceptance (id text PRIMARY KEY, value text NOT NULL); INSERT INTO boxferry_acceptance VALUES ('persistence', 'boxferry-database-persisted') ON CONFLICT (id) DO UPDATE SET value = EXCLUDED.value;" \
    > /dev/null
}

nextcloud_assert_database_canary() {
  local socket=$1 prefix=$2 observed
  observed="$(nextcloud_remote "${socket}" exec --env "PGPASSWORD=${NEXTCLOUD_DB_PASSWORD}" \
    "${prefix}-cloud-db" psql -U boxferry -d nextcloud -Atc \
    "SELECT value FROM boxferry_acceptance WHERE id = 'persistence';")"
  [[ "${observed}" == boxferry-database-persisted ]]
  nextcloud_remote "${socket}" inspect "${prefix}-cloud-db" | jq --exit-status \
    --arg volume "${prefix}-cloud-database" \
    'any(.[0].Mounts[]?; .Name == $volume and .Destination == "/var/lib/postgresql/data")' > /dev/null
}

nextcloud_recreate_front_path() {
  local mode=$1 socket=$2 prefix=$3 run=$4 fixture_root=$5
  nextcloud_seed_database_canary "${socket}" "${prefix}"
  if [[ "${mode}" == cli ]]; then
    nextcloud_remote "${socket}" rm --force --time 0 "${prefix}-shared-proxy" > /dev/null
    local container
    for container in cron frontend; do
      nextcloud_remote "${socket}" kill --signal KILL "${prefix}-cloud-${container}" > /dev/null
    done
    nextcloud_remote "${socket}" exec "${prefix}-cloud-app" apache2ctl -k graceful-stop > /dev/null
    nextcloud_wait_for 60 'Nextcloud application termination' \
      nextcloud_container_stopped "${socket}" "${prefix}-cloud-app"
    nextcloud_remote "${socket}" exec --user postgres "${prefix}-cloud-db" \
      pg_ctl -D /var/lib/postgresql/data -m fast stop > /dev/null || true
    nextcloud_wait_for 60 'PostgreSQL termination' \
      nextcloud_container_stopped "${socket}" "${prefix}-cloud-db"
    nextcloud_remote "${socket}" rm --force --time 0 --depend "${prefix}-cloud-db" > /dev/null
    nextcloud_create_cli_database "${socket}" "${prefix}" "${run}" \
      "$(nextcloud_image_reference postgres)"
    nextcloud_wait_for 180 'recreated PostgreSQL' nextcloud_remote "${socket}" \
      exec "${prefix}-cloud-db" pg_isready -U boxferry -d nextcloud
    nextcloud_create_cli_app "${socket}" "${prefix}" "${run}" "$(nextcloud_image_reference nextcloud)"
    nextcloud_wait_for 600 'recreated Nextcloud application' nextcloud_remote "${socket}" \
      exec --user www-data "${prefix}-cloud-app" php occ status
    nextcloud_create_cli_init "${socket}" "${prefix}" "${run}" \
      "$(nextcloud_image_reference nextcloud)" "${current_case}/cli-recreate-init.log"
    nextcloud_create_cli_cron "${socket}" "${prefix}" "${run}" \
      "$(nextcloud_image_reference nextcloud)"
    nextcloud_create_cli_frontend "${socket}" "${prefix}" "${run}" "${fixture_root}" \
      "$(nextcloud_image_reference nginx)"
    nextcloud_create_cli_proxy "${socket}" "${prefix}" "${run}" "${fixture_root}" \
      "$(nextcloud_image_reference nginx)"
  else
    nextcloud_remote "${socket}" exec "${prefix}-cloud-app" apache2ctl -k graceful-stop > /dev/null
    nextcloud_wait_for 60 'Compose Nextcloud recreation termination' \
      nextcloud_container_stopped "${socket}" "${prefix}-cloud-app"
    nextcloud_compose "${socket}" "${prefix}" "${run}" up --detach --no-deps \
      --force-recreate db \
      > "${current_case}/compose-recreate.log" 2>&1
    nextcloud_wait_for 180 'recreated Compose PostgreSQL' nextcloud_remote "${socket}" \
      exec "${prefix}-cloud-db" pg_isready -U boxferry -d nextcloud
    nextcloud_compose "${socket}" "${prefix}" "${run}" up --detach --no-deps \
      --force-recreate app >> "${current_case}/compose-recreate.log" 2>&1
    nextcloud_compose "${socket}" "${prefix}" "${run}" up --detach --no-deps \
      --force-recreate frontend >> "${current_case}/compose-recreate.log" 2>&1
    nextcloud_compose_project "${socket}" "${prefix}" "${run}" shared \
      up --detach --no-deps --force-recreate proxy \
      >> "${current_case}/compose-recreate.log" 2>&1
  fi
  nextcloud_wait_application "${socket}" "${prefix}"
  nextcloud_webdav_round_trip "${socket}" "${prefix}" "${mode}" false
  nextcloud_assert_database_cache_and_shared_state "${socket}" "${prefix}"
  nextcloud_assert_database_canary "${socket}" "${prefix}"
}

nextcloud_assert_output_membership() {
  local selection=$1 output=$2 directory=$3 prefix=$4
  assert_named_member "${output}" "${directory}" app "${prefix}-cloud-app"
  case "${selection}" in
    exact | prefix | label)
      for service in db cache app init cron frontend; do
        assert_named_member "${output}" "${directory}" "${service}" "${prefix}-cloud-${service}"
      done
      assert_named_absent "${output}" "${directory}" proxy "${prefix}-shared-proxy"
      assert_named_absent "${output}" "${directory}" second "${prefix}-second-app"
      for volume in database redis nextcloud; do
        assert_resource_member "${output}" "${directory}" volume "${prefix}-cloud-${volume}"
      done
      assert_resource_member "${output}" "${directory}" network "${prefix}-cloud-backend"
      assert_resource_member "${output}" "${directory}" network "${prefix}-shared-edge"
      ;;
    all)
      for service in db cache init cron frontend; do
        assert_named_member "${output}" "${directory}" "${service}" "${prefix}-cloud-${service}"
      done
      assert_named_member "${output}" "${directory}" proxy "${prefix}-shared-proxy"
      assert_named_member "${output}" "${directory}" second "${prefix}-second-app"
      for volume in database redis nextcloud; do
        assert_resource_member "${output}" "${directory}" volume "${prefix}-cloud-${volume}"
      done
      assert_resource_member "${output}" "${directory}" network "${prefix}-cloud-backend"
      assert_resource_member "${output}" "${directory}" network "${prefix}-shared-edge"
      ;;
  esac
}

nextcloud_assert_external_edge() {
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

nextcloud_run_exports() {
  local mode=$1 socket=$2 prefix=$3
  local selection output directory report
  local -a selection_arguments=()
  mkdir -p -- "${current_case}/outputs"
  for selection in exact prefix label all; do
    case "${selection}" in
      exact) selection_arguments=(--podman-resource "container=${prefix}-cloud-app") ;;
      prefix) selection_arguments=(
        --podman-resource-prefix "container=${prefix}-cloud-"
        --podman-resource-prefix "network=${prefix}-cloud-"
        --podman-resource-prefix "volume=${prefix}-cloud-"
      ) ;;
      label) selection_arguments=(--podman-label "io.boxferry.application=${prefix}-cloud") ;;
      all) selection_arguments=(--podman-all) ;;
    esac
    for output in compose quadlet podman; do
      directory="${current_case}/outputs/${mode}-${selection}-${output}"
      report="${directory}.report.json"
      local -a target_arguments=()
      [[ "${output}" == podman ]] && target_arguments+=(--podman-target-context rootful)
      boxferry_operation "Nextcloud ${mode} ${selection} Podman-to-${output}" \
        convert podman "${output}" --podman-socket "${socket}" \
        --application-name "${prefix}-cloud" --loss-policy partial \
        --promote-podman-effective-named-volumes --promote-podman-effective-named-networks \
        --output-directory "${directory}" --console-format json \
        "${target_arguments[@]}" "${selection_arguments[@]}" > "${report}"
      jq --exit-status '
 .schema_version == 1 and .status == "success" and .exit_category == "success" and
 ([.diagnostics[]? | select(.severity == "error")] | length == 0) and
 ((.fidelity.invalid // 0) == 0) and (.output_artifacts | length > 0)
 ' "${report}" > /dev/null
      nextcloud_assert_output_membership "${selection}" "${output}" "${directory}" "${prefix}"
      if [[ "${selection}" != all ]]; then
        nextcloud_assert_external_edge "${output}" "${directory}" "${prefix}-shared-edge"
      fi
    done
  done
  if grep --recursive --include='*.report.json' --fixed-strings --quiet \
    -e "${NEXTCLOUD_ADMIN_PASSWORD}" -e "${NEXTCLOUD_DB_PASSWORD}" \
    -e "${NEXTCLOUD_REDIS_PASSWORD}" "${current_case}/outputs"; then
    printf 'Nextcloud conversion report leaked a protected public canary.\n' >&2
    return 1
  fi
}

nextcloud_cleanup_mode() {
  local mode=$1 socket=$2 prefix=$3 run=$4
  if [[ "${mode}" == compose ]]; then
    nextcloud_compose_project "${socket}" "${prefix}" "${run}" shared \
      down --remove-orphans > "${current_case}/compose-shared-down.log" 2>&1 || true
    nextcloud_compose_project "${socket}" "${prefix}" "${run}" second \
      down --remove-orphans > "${current_case}/compose-second-down.log" 2>&1 || true
    nextcloud_remote "${socket}" exec "${prefix}-cloud-app" apache2ctl -k graceful-stop > /dev/null
    nextcloud_wait_for 60 'Compose Nextcloud cleanup termination' \
      nextcloud_container_stopped "${socket}" "${prefix}-cloud-app"
    nextcloud_compose "${socket}" "${prefix}" "${run}" down --volumes --remove-orphans \
      > "${current_case}/compose-down.log" 2>&1 || true
  else
    nextcloud_remote "${socket}" rm --force --time 0 "${prefix}-shared-proxy" > /dev/null
    local container
    for container in second-app cloud-cron cloud-frontend cloud-cache; do
      nextcloud_remote "${socket}" kill --signal KILL "${prefix}-${container}" > /dev/null
    done
    nextcloud_remote "${socket}" exec "${prefix}-cloud-app" apache2ctl -k graceful-stop > /dev/null
    nextcloud_wait_for 60 'Nextcloud cleanup termination' \
      nextcloud_container_stopped "${socket}" "${prefix}-cloud-app"
    nextcloud_remote "${socket}" exec --user postgres "${prefix}-cloud-db" \
      pg_ctl -D /var/lib/postgresql/data -m fast stop > /dev/null || true
    nextcloud_wait_for 60 'PostgreSQL cleanup termination' \
      nextcloud_container_stopped "${socket}" "${prefix}-cloud-db"
    nextcloud_remote "${socket}" rm --force --time 0 --depend "${prefix}-cloud-db" > /dev/null
    nextcloud_remote "${socket}" rm --force --time 0 \
      "${prefix}-cloud-cache" "${prefix}-second-app" > /dev/null
  fi
  local -a containers=(
    "${prefix}-cloud-db" "${prefix}-cloud-cache" "${prefix}-cloud-app"
    "${prefix}-cloud-init" "${prefix}-cloud-cron" "${prefix}-cloud-frontend"
    "${prefix}-shared-proxy" "${prefix}-second-app"
  )
  nextcloud_remote "${socket}" rm --force --time 0 --ignore "${containers[@]}" > /dev/null 2>&1 || true
  nextcloud_remote "${socket}" volume rm --force \
    "${prefix}-cloud-database" "${prefix}-cloud-redis" \
    "${prefix}-cloud-nextcloud" > /dev/null 2>&1 || true
  nextcloud_remote "${socket}" network rm \
    "${prefix}-cloud-backend" "${prefix}-shared-edge" > /dev/null 2>&1 || true
  nextcloud_assert_clean_prefix "${socket}" "${prefix}"
}

nextcloud_verify_target() {
  local id=$1 image=$2 declared_version=$3 distribution=$4 mode=$5 lane=$6 architecture=$7
  verify_observed_version "${id}" "${declared_version}" "${artifact_root}/${id}.podman-version"
  [[ "$(< "${artifact_root}/${id}.digest")" == "${image##*@}" ]] || {
    printf 'Pulled image digest does not match reviewed application target %s.\n' "${id}" >&2
    return 1
  }
  [[ "${architecture}" == amd64 && "$(< "${artifact_root}/${id}.architecture")" =~ ^(x86_64|amd64)$ ]] || {
    printf 'Observed architecture does not match application target %s.\n' "${id}" >&2
    return 1
  }
  append_verified_evidence "${id}" "${image}" "${declared_version}" "${distribution}" \
    "${mode}" "${lane}" "${architecture}" "${artifact_root}/${id}" nested-image application
}

run_nextcloud_application_cell() {
  local id=$1 image=$2 declared_version=$3 distribution=$4 mode=$5 lane=$6 architecture=$7
  [[ "${id}-${mode}" == podman-6.1-rootless-rootless ]] || {
    printf 'Application profile is bounded to the reviewed Podman 6.1 rootless cell.\n' >&2
    return 1
  }
  current_prefix="${run_id:0:34}-nc"
  current_case="${artifact_root}/${id}-application"
  local socket_directory="${runtime_root}/application-target"
  local application_archive="${runtime_root}/nextcloud-application-images.tar"
  local fixture_root="/tmp/boxferry-fixture/${current_prefix}"
  mkdir -p -- "${current_case}" "${socket_directory}"
  chmod 0700 "${current_case}" "${socket_directory}"
  progress_index=0
  progress_total=18
  printf '%s PLAN %s application tests=%d\n' "$(timestamp)" "${id}" "${progress_total}"
  progress_run 'validate Nextcloud catalogues' \
    nextcloud_validate_catalogues
  progress_run 'validate Docker Compose provider' nextcloud_validate_provider
  progress_run 'prepare digest-pinned Nextcloud image archive' \
    nextcloud_prepare_image_archive "${application_archive}"
  progress_run 'start isolated Nextcloud Podman target' \
    start_outer_runtime "${id}" "${image}" "${mode}" "${socket_directory}" \
    "${application_archive}"
  progress_run 'verify Podman application target evidence' \
    nextcloud_verify_target "${id}" "${image}" "${declared_version}" "${distribution}" \
    "${mode}" "${lane}" "${architecture}"
  local outer="${started_outer}"
  progress_run 'load application images and reviewed configuration' \
    nextcloud_prepare_application_target "${outer}" "${current_prefix}" "${socket_directory}"
  local socket="${socket_directory}/podman.sock"
  progress_run 'provision independent Podman CLI application' \
    nextcloud_provision_cli "${socket}" "${current_prefix}" "${run_id}" "${fixture_root}"
  progress_run 'wait for Podman CLI application readiness' \
    nextcloud_wait_application "${socket}" "${current_prefix}"
  progress_run 'prove Podman CLI application behavior and boundaries' \
    nextcloud_assert_application_behavior "${socket}" "${current_prefix}" cli
  progress_run 'exercise Podman CLI source selectors and exporters' \
    nextcloud_run_exports cli "${socket}" "${current_prefix}"
  progress_run 'prove Podman CLI persistence after recreation' \
    nextcloud_recreate_front_path cli "${socket}" "${current_prefix}" "${run_id}" "${fixture_root}"
  progress_run 'clean Podman CLI application resources' \
    nextcloud_cleanup_mode cli "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'provision independent Docker Compose application' \
    nextcloud_provision_compose "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'wait for Docker Compose application readiness' \
    nextcloud_wait_application "${socket}" "${current_prefix}"
  progress_run 'prove Docker Compose application behavior and boundaries' \
    nextcloud_assert_application_behavior "${socket}" "${current_prefix}" compose
  progress_run 'exercise Docker Compose source selectors and exporters' \
    nextcloud_run_exports compose "${socket}" "${current_prefix}"
  progress_run 'prove Docker Compose persistence after recreation' \
    nextcloud_recreate_front_path compose "${socket}" "${current_prefix}" "${run_id}" "${fixture_root}"
  progress_run 'clean Docker Compose application resources and target' \
    nextcloud_cleanup_mode compose "${socket}" "${current_prefix}" "${run_id}"
  remove_outer "${outer}"
  printf '%s CELL PASS %s application (%d/%d tests)\n' \
    "$(timestamp)" "${id}" "${progress_index}" "${progress_total}"
}
