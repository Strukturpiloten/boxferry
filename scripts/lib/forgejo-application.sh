#!/usr/bin/env bash
# Forgejo application acceptance helpers. This file is sourced by the live
# entry point; it never executes the profile by itself.
# shellcheck disable=SC2129,SC2154 # Caller globals and phased logs are intentional.

readonly FORGEJO_ADMIN_USER="boxferry-live"
readonly FORGEJO_ADMIN_PASSWORD="boxferry-public-admin-canary"
readonly FORGEJO_DB_PASSWORD="boxferry-public-database-canary"
readonly FORGEJO_SECRET_KEY="boxferry-public-forgejo-secret-canary"
readonly FORGEJO_PROVIDER_VERSION="5.5.0"
readonly FORGEJO_PROVIDER_SHA256="c57ab918abd5b05ca7e7d0f275875dd1330a695074f309dc9eab1b49efafcd4b"
readonly FORGEJO_HTTP_PORT="13000"
readonly FORGEJO_SSH_PORT="12222"

forgejo_fixture_root() {
  printf '%s/fixtures/conformance/forgejo-application\n' \
    "${repository_root:?caller must supply repository_root}"
}

forgejo_image_reference() {
  local expected=$1
  printf 'registry.invalid/boxferry-test/forgejo-application:%s\n' "${expected}"
}

forgejo_validate_catalogues() {
  local fixture
  fixture="$(forgejo_fixture_root)"
  awk -F '\t' '
    NF && $1 !~ /^#/ {
      if (NF != 6 || $2 !~ /:[^/@]+@sha256:[0-9a-f]{64}$/ || $3 == "" || $4 == "" ||
          $5 !~ /^https:\/\// || $6 != "transient-test-pull" || seen[$1]++) {
        printf "Invalid Forgejo image row at line %d: %s\\n", NR, $0 > "/dev/stderr"
        bad = 1
      }
      count++
    }
    END { exit bad || count != 3 }
  ' "${fixture}/images.tsv"
  awk -F '\t' -v version="${FORGEJO_PROVIDER_VERSION}" \
    -v sha="${FORGEJO_PROVIDER_SHA256}" '
    NF && $1 !~ /^#/ {
      if (NF != 7 || $1 != "docker-compose" || $2 != version ||
          $3 != "https://github.com/docker/compose/releases/download/v5.5.0/docker-compose-linux-x86_64" ||
          $4 != sha || $5 != "Apache-2.0" || $6 != "https://github.com/docker/compose" ||
          $7 != "downloaded-test-tool") {
        printf "Invalid Forgejo provider row at line %d: %s\\n", NR, $0 > "/dev/stderr"
        bad = 1
      }
      count++
    }
    END { exit bad || count != 1 }
  ' "${fixture}/providers.tsv"
  local file
  for file in compose.yaml peer.compose.yaml git-probe.sh images.tsv providers.tsv repository-proof.txt README.md; do
    [[ -s "${fixture}/${file}" ]]
  done
}

forgejo_validate_provider() {
  local provider=${BOXFERRY_COMPOSE_BIN:-${repository_root}/target/tools/docker-compose}
  [[ -x "${provider}" ]] || {
    printf 'Forgejo application profile requires executable Docker Compose %s at %s.\n' \
      "${FORGEJO_PROVIDER_VERSION}" "${provider}" >&2
    return 1
  }
  local observed_sha observed_version
  observed_sha="$(sha256sum "${provider}" | awk '{ print $1 }')"
  [[ "${observed_sha}" == "${FORGEJO_PROVIDER_SHA256}" ]] || {
    printf 'Docker Compose checksum mismatch: expected %s, observed %s.\n' \
      "${FORGEJO_PROVIDER_SHA256}" "${observed_sha}" >&2
    return 1
  }
  observed_version="$("${provider}" version --short | sed 's/^v//')"
  [[ "${observed_version}" == "${FORGEJO_PROVIDER_VERSION}" ]] || {
    printf 'Docker Compose version mismatch: expected %s, observed %s.\n' \
      "${FORGEJO_PROVIDER_VERSION}" "${observed_version}" >&2
    return 1
  }
}

forgejo_prepare_image_archive() {
  local archive=${1:?archive path required}
  [[ -s "${archive}" ]] && return 0
  local fixture id reference expected_digest observed_digest cache_status runtime_reference
  local -a references=()
  fixture="$(forgejo_fixture_root)"
  while IFS=$'\t' read -r id reference _ _ _ _; do
    [[ -z "${id}" || "${id}" == \#* ]] && continue
    expected_digest="${reference##*@}"
    cache_status=0
    engine_image_available "probe Forgejo ${id} image cache" "${reference}" || cache_status=$?
    if ((cache_status == 1)); then
      timed_operation 8m "pull digest-pinned Forgejo ${id} image" \
        "${engine}" pull --quiet "${reference}" \
        > "${artifact_root}/forgejo-${id}.pull.log"
      record_run_owned_host_image "${reference}"
    elif ((cache_status != 0)); then
      return "${cache_status}"
    fi
    observed_digest="$(engine_operation "inspect Forgejo ${id} image digest" \
      image inspect --format '{{.Digest}}' "${reference}")"
    [[ "${observed_digest}" == "${expected_digest}" ]] || {
      printf 'Forgejo image digest mismatch %s: expected %s, observed %s.\n' \
        "${id}" "${expected_digest}" "${observed_digest}" >&2
      return 1
    }
    runtime_reference="$(forgejo_image_reference "${id}")"
    record_run_owned_archive_alias "${reference}" "${runtime_reference}" "Forgejo ${id}"
    references+=("${runtime_reference}")
  done < "${fixture}/images.tsv"
  timed_operation 8m 'archive digest-pinned Forgejo images' \
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

forgejo_assert_loaded_images() {
  local outer=$1 fixture id
  fixture="$(forgejo_fixture_root)"
  while IFS=$'\t' read -r id _ _ _ _ _; do
    [[ -z "${id}" || "${id}" == \#* ]] && continue
    engine_operation "verify loaded Forgejo ${id} image" \
      exec "${outer}" podman image exists "$(forgejo_image_reference "${id}")"
  done < "${fixture}/images.tsv"
}

forgejo_load_image_archive() {
  local outer=$1
  engine_operation 'load digest-pinned Forgejo application archive' \
    exec "${outer}" podman load --input /boxferry-workload.tar > /dev/null
  forgejo_assert_loaded_images "${outer}"
}

forgejo_copy_fixture() {
  local outer=$1 prefix=$2 fixture destination
  fixture="$(forgejo_fixture_root)"
  destination="/tmp/boxferry-fixture/${prefix}"
  engine_operation 'create disposable Forgejo fixture directory' \
    exec "${outer}" mkdir -p -- "${destination}"
  engine_operation 'copy reviewed Forgejo Git probe' \
    cp "${fixture}/git-probe.sh" "${outer}:${destination}/git-probe.sh"
  engine_operation 'copy deterministic Forgejo repository proof' \
    cp "${fixture}/repository-proof.txt" "${outer}:${destination}/repository-proof.txt"
}

forgejo_prepare_application_target() {
  local outer=$1 prefix=$2 socket_directory=$3 mode=$4
  if [[ "${mode}" == rootless ]]; then
    engine_operation 'copy rootless Forgejo network configuration' cp \
      "${repository_root}/fixtures/conformance/podman-live/apply-target-containers.conf" \
      "${outer}:/tmp/99-boxferry-live.conf"
    # shellcheck disable=SC2016 # $HOME expands inside the nested target.
    engine_operation 'prepare rootless Forgejo network configuration' \
      exec "${outer}" /bin/sh -ceu \
      'mkdir -p "$HOME/.config/containers/containers.conf.d"; cp /tmp/99-boxferry-live.conf "$HOME/.config/containers/containers.conf.d/99-boxferry-live.conf"'
  else
    # Rootful publication needs Netavark's stock firewall path. The rootless
    # firewall_driver=none drop-in would prevent real HTTP and SSH DNAT.
    # The selected Arch target includes nft for stock Netavark publication.
    # The upstream-source 6.1 rootful target omits nft and cannot install real DNAT.
    engine_operation 'verify rootful Forgejo target uses stock firewall configuration' \
      exec "${outer}" test ! -e /root/.config/containers/containers.conf.d/99-boxferry-live.conf
  fi
  forgejo_load_image_archive "${outer}"
  forgejo_copy_fixture "${outer}" "${prefix}"
  activate_outer_runtime "${socket_directory}"
}

forgejo_remote() {
  local socket=$1
  shift
  podman_socket "${socket}" "Forgejo ${1:-command}" "$@"
}

forgejo_wait_for() {
  local deadline_seconds=$1 description=$2
  shift 2
  local started
  started="$(date +%s)"
  until "$@" > /dev/null 2>&1; do
    if (($(date +%s) - started >= deadline_seconds)); then
      printf 'Timed out waiting for Forgejo %s after %ss.\n' \
        "${description}" "${deadline_seconds}" >&2
      return 1
    fi
    sleep 2
  done
}

forgejo_create_edge_network() {
  local socket=$1 prefix=$2 run=$3
  forgejo_remote "${socket}" network exists "${prefix}-shared-edge" 2> /dev/null ||
    forgejo_remote "${socket}" network create \
      --label "io.boxferry.live-run=${run}" --label io.boxferry.shared=true \
      "${prefix}-shared-edge" > /dev/null
}

forgejo_assert_clean_prefix() {
  local socket=$1 prefix=$2
  local -a collisions=()
  mapfile -t collisions < <({
    forgejo_remote "${socket}" container ps --all --format '{{.Names}}' || exit $?
    forgejo_remote "${socket}" network ls --format '{{.Name}}' || exit $?
    forgejo_remote "${socket}" volume ls --format '{{.Name}}'
  } | awk -v prefix="${prefix}" 'index($0, prefix) == 1') || {
    printf 'Could not inspect existing resources for Forgejo prefix %s.\n' "${prefix}" >&2
    return 2
  }
  if ((${#collisions[@]} > 0)); then
    printf 'Refusing Forgejo provisioning because prefix %s already owns resources:\n' \
      "${prefix}" >&2
    printf '  %s\n' "${collisions[@]}" >&2
    return 1
  fi
}

forgejo_expect_collision() {
  local status=0
  forgejo_assert_clean_prefix "$@" || status=$?
  if ((status == 1)); then
    return 0
  fi
  if ((status == 0)); then
    printf 'Forgejo collision check unexpectedly found a clean prefix.\n' >&2
    return 1
  fi
  return "${status}"
}

forgejo_create_cli_database() {
  local socket=$1 prefix=$2 run=$3
  forgejo_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-forge-db" \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-forgejo" \
    --network "${prefix}-forge-backend:alias=db" \
    --volume "${prefix}-forge-db:/var/lib/postgresql/data" \
    --env POSTGRES_DB=forgejo --env POSTGRES_USER=forgejo \
    --env "POSTGRES_PASSWORD=${FORGEJO_DB_PASSWORD}" \
    --health-cmd 'pg_isready -U forgejo -d forgejo' \
    --health-interval 2s --health-retries 60 \
    "$(forgejo_image_reference postgres)" > /dev/null
}

forgejo_create_cli_application() {
  local socket=$1 prefix=$2 run=$3
  forgejo_remote "${socket}" run --pull=never --detach --user 1000:1000 \
    --name "${prefix}-forge-app" \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-forgejo" \
    --requires "${prefix}-forge-db" \
    --network "${prefix}-forge-backend:alias=forgejo" \
    --network "${prefix}-shared-edge:alias=forgejo" \
    --volume "${prefix}-forge-data:/var/lib/gitea" \
    --publish "127.0.0.1:${FORGEJO_HTTP_PORT}:3000" \
    --publish "127.0.0.1:${FORGEJO_SSH_PORT}:2222" \
    --env FORGEJO__database__DB_TYPE=postgres \
    --env FORGEJO__database__HOST=db:5432 \
    --env FORGEJO__database__NAME=forgejo \
    --env FORGEJO__database__USER=forgejo \
    --env "FORGEJO__database__PASSWD=${FORGEJO_DB_PASSWORD}" \
    --env FORGEJO__database__SSL_MODE=disable \
    --env FORGEJO__security__INSTALL_LOCK=true \
    --env "FORGEJO__security__SECRET_KEY=${FORGEJO_SECRET_KEY}" \
    --env FORGEJO__service__DISABLE_REGISTRATION=true \
    --env FORGEJO__server__DOMAIN=127.0.0.1 \
    --env "FORGEJO__server__ROOT_URL=http://127.0.0.1:${FORGEJO_HTTP_PORT}/" \
    --env FORGEJO__server__SSH_DOMAIN=127.0.0.1 \
    --env "FORGEJO__server__SSH_PORT=${FORGEJO_SSH_PORT}" \
    --env FORGEJO__server__SSH_LISTEN_PORT=2222 \
    --env FORGEJO__server__START_SSH_SERVER=true \
    "$(forgejo_image_reference forgejo)" > /dev/null
}

forgejo_create_cli_peer() {
  local socket=$1 prefix=$2 run=$3
  forgejo_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-peer-app" \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-peer" \
    --network "${prefix}-shared-edge:alias=peer" \
    --entrypoint /bin/sh "$(forgejo_image_reference git-client)" \
    -ceu 'sleep 3600' > /dev/null
}

forgejo_provision_cli() {
  local socket=$1 prefix=$2 run=$3
  forgejo_assert_clean_prefix "${socket}" "${prefix}"
  forgejo_create_edge_network "${socket}" "${prefix}" "${run}"
  forgejo_remote "${socket}" network create --internal \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-forgejo" \
    "${prefix}-forge-backend" > /dev/null
  forgejo_remote "${socket}" volume create \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-forgejo" \
    "${prefix}-forge-db" > /dev/null
  forgejo_remote "${socket}" volume create \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-forgejo" \
    "${prefix}-forge-data" > /dev/null
  forgejo_create_cli_database "${socket}" "${prefix}" "${run}"
  forgejo_wait_for 180 'Podman CLI PostgreSQL readiness' \
    forgejo_remote "${socket}" exec "${prefix}-forge-db" \
    pg_isready -U forgejo -d forgejo
  forgejo_create_cli_application "${socket}" "${prefix}" "${run}"
  forgejo_create_cli_peer "${socket}" "${prefix}" "${run}"
}

forgejo_compose_project() {
  local socket=$1 prefix=$2 run=$3
  shift 3
  local provider=${BOXFERRY_COMPOSE_BIN:-${repository_root}/target/tools/docker-compose}
  local fixture compose_file fixture_root
  fixture="$(forgejo_fixture_root)"
  compose_file="${fixture}/compose.yaml"
  fixture_root="/tmp/boxferry-fixture/${prefix}"
  timed_operation 90s "Docker Compose Forgejo ${1:-command}" \
    env DOCKER_HOST="unix://${socket}" \
    BF_PREFIX="${prefix}" BF_RUN="${run}" BF_FIXTURE_ROOT="${fixture_root}" \
    BF_DB_PASSWORD="${FORGEJO_DB_PASSWORD}" BF_FORGEJO_SECRET_KEY="${FORGEJO_SECRET_KEY}" \
    BF_FORGEJO_IMAGE="$(forgejo_image_reference forgejo)" \
    BF_POSTGRES_IMAGE="$(forgejo_image_reference postgres)" \
    BF_GIT_CLIENT_IMAGE="$(forgejo_image_reference git-client)" \
    "${provider}" --project-name "${prefix}-forgejo" --file "${compose_file}" "$@"
}

forgejo_peer_compose_project() {
  local socket=$1 prefix=$2 run=$3
  shift 3
  local provider=${BOXFERRY_COMPOSE_BIN:-${repository_root}/target/tools/docker-compose}
  local fixture compose_file
  fixture="$(forgejo_fixture_root)"
  compose_file="${fixture}/peer.compose.yaml"
  timed_operation 90s "Docker Compose Forgejo peer ${1:-command}" \
    env DOCKER_HOST="unix://${socket}" \
    BF_PREFIX="${prefix}" BF_RUN="${run}" \
    BF_GIT_CLIENT_IMAGE="$(forgejo_image_reference git-client)" \
    "${provider}" --project-name "${prefix}-peer" --file "${compose_file}" "$@"
}

forgejo_provision_compose() {
  local socket=$1 prefix=$2 run=$3
  forgejo_assert_clean_prefix "${socket}" "${prefix}"
  forgejo_create_edge_network "${socket}" "${prefix}" "${run}"
  forgejo_compose_project "${socket}" "${prefix}" "${run}" \
    up --detach --no-deps --remove-orphans db \
    > "${current_case}/forgejo-compose.log" 2>&1
  forgejo_wait_for 180 'Compose PostgreSQL readiness' \
    forgejo_remote "${socket}" exec "${prefix}-forge-db" \
    pg_isready -U forgejo -d forgejo
  forgejo_compose_project "${socket}" "${prefix}" "${run}" \
    up --detach --no-deps forgejo \
    >> "${current_case}/forgejo-compose.log" 2>&1
  forgejo_peer_compose_project "${socket}" "${prefix}" "${run}" \
    up --detach --no-deps --remove-orphans peer \
    >> "${current_case}/forgejo-peer-compose.log" 2>&1
}

forgejo_wait_application() {
  local socket=$1 prefix=$2
  forgejo_wait_for 180 'application health endpoint' \
    forgejo_remote "${socket}" exec "${prefix}-forge-app" \
    wget --quiet --output-document=- http://127.0.0.1:3000/api/healthz
}

forgejo_create_admin() {
  local socket=$1 prefix=$2
  forgejo_remote "${socket}" exec --user 1000:1000 \
    "${prefix}-forge-app" forgejo admin user create \
    --username "${FORGEJO_ADMIN_USER}" \
    --password "${FORGEJO_ADMIN_PASSWORD}" \
    --email boxferry@example.invalid --admin --must-change-password=false \
    > /dev/null
}

forgejo_git_probe() {
  local socket=$1 prefix=$2 mode=$3
  local fixture_root="/tmp/boxferry-fixture/${prefix}"
  forgejo_remote "${socket}" run --rm --pull=never --network host \
    --volume "${fixture_root}:/fixture:rw" \
    --env "BF_FORGEJO_USER=${FORGEJO_ADMIN_USER}" \
    --env "BF_FORGEJO_PASSWORD=${FORGEJO_ADMIN_PASSWORD}" \
    --env "BF_HTTP_PORT=${FORGEJO_HTTP_PORT}" \
    --env "BF_SSH_PORT=${FORGEJO_SSH_PORT}" \
    --entrypoint /bin/sh "$(forgejo_image_reference git-client)" \
    /fixture/git-probe.sh "${mode}"
}

forgejo_assert_database() {
  local socket=$1 prefix=$2
  forgejo_remote "${socket}" exec --env "PGPASSWORD=${FORGEJO_DB_PASSWORD}" \
    "${prefix}-forge-db" psql -U forgejo -d forgejo -Atc \
    "SELECT count(*) FROM repository WHERE lower_name = 'migration-baseline';" |
    awk '$1 == 1 { found = 1 } END { exit !found }'
}

forgejo_assert_application_boundaries() {
  local socket=$1 prefix=$2
  [[ "$(forgejo_remote "${socket}" port "${prefix}-forge-app" 3000/tcp)" == "127.0.0.1:${FORGEJO_HTTP_PORT}" ]]
  [[ "$(forgejo_remote "${socket}" port "${prefix}-forge-app" 2222/tcp)" == "127.0.0.1:${FORGEJO_SSH_PORT}" ]]
  [[ -z "$(forgejo_remote "${socket}" port "${prefix}-forge-db")" ]]
  forgejo_remote "${socket}" network inspect "${prefix}-forge-backend" |
    jq --exit-status '.[0].internal == true' > /dev/null
  forgejo_remote "${socket}" inspect "${prefix}-forge-db" "${prefix}-forge-app" |
    jq --exit-status \
      --arg backend "${prefix}-forge-backend" --arg edge "${prefix}-shared-edge" '
        (.[0].NetworkSettings.Networks[$backend] != null) and
        (.[0].NetworkSettings.Networks[$edge] == null) and
        (.[1].NetworkSettings.Networks[$backend] != null) and
        (.[1].NetworkSettings.Networks[$edge] != null)
      ' > /dev/null
  forgejo_remote "${socket}" inspect "${prefix}-peer-app" |
    jq --exit-status \
      --arg backend "${prefix}-forge-backend" --arg edge "${prefix}-shared-edge" '
        (.[0].NetworkSettings.Networks[$backend] == null) and
        (.[0].NetworkSettings.Networks[$edge] != null)
      ' > /dev/null
  forgejo_remote "${socket}" inspect "${prefix}-forge-db" |
    jq --exit-status --arg volume "${prefix}-forge-db" '
      any(.[0].Mounts[]?; .Name == $volume and .Destination == "/var/lib/postgresql/data")
    ' > /dev/null
  forgejo_remote "${socket}" inspect "${prefix}-forge-app" |
    jq --exit-status --arg volume "${prefix}-forge-data" '
      any(.[0].Mounts[]?; .Name == $volume and .Destination == "/var/lib/gitea")
    ' > /dev/null
  forgejo_assert_database "${socket}" "${prefix}"
}

forgejo_recreate_application() {
  local mode=$1 socket=$2 prefix=$3 run=$4
  if [[ "${mode}" == cli ]]; then
    forgejo_remote "${socket}" stop --time 30 \
      "${prefix}-forge-app" "${prefix}-forge-db" > /dev/null
    forgejo_remote "${socket}" rm --force --time 0 \
      "${prefix}-forge-app" "${prefix}-forge-db" > /dev/null
    forgejo_create_cli_database "${socket}" "${prefix}" "${run}"
    forgejo_wait_for 180 'recreated Podman CLI PostgreSQL' \
      forgejo_remote "${socket}" exec "${prefix}-forge-db" \
      pg_isready -U forgejo -d forgejo
    forgejo_create_cli_application "${socket}" "${prefix}" "${run}"
  else
    forgejo_compose_project "${socket}" "${prefix}" "${run}" \
      stop --timeout 30 forgejo db > "${current_case}/forgejo-compose-recreate.log" 2>&1
    forgejo_compose_project "${socket}" "${prefix}" "${run}" \
      rm --force forgejo db >> "${current_case}/forgejo-compose-recreate.log" 2>&1
    forgejo_compose_project "${socket}" "${prefix}" "${run}" \
      up --detach --no-deps db >> "${current_case}/forgejo-compose-recreate.log" 2>&1
    forgejo_wait_for 180 'recreated Compose PostgreSQL' \
      forgejo_remote "${socket}" exec "${prefix}-forge-db" \
      pg_isready -U forgejo -d forgejo
    forgejo_compose_project "${socket}" "${prefix}" "${run}" \
      up --detach --no-deps forgejo >> "${current_case}/forgejo-compose-recreate.log" 2>&1
  fi
  forgejo_wait_application "${socket}" "${prefix}"
  forgejo_git_probe "${socket}" "${prefix}" verify
  forgejo_assert_database "${socket}" "${prefix}"
}

forgejo_assert_output_membership() {
  local selection=$1 output=$2 directory=$3 prefix=$4
  assert_named_member "${output}" "${directory}" forgejo "${prefix}-forge-app"
  case "${selection}" in
    exact | label)
      assert_named_member "${output}" "${directory}" db "${prefix}-forge-db"
      assert_named_absent "${output}" "${directory}" peer "${prefix}-peer-app"
      assert_resource_member "${output}" "${directory}" volume "${prefix}-forge-db"
      assert_resource_member "${output}" "${directory}" volume "${prefix}-forge-data"
      assert_resource_member "${output}" "${directory}" network "${prefix}-forge-backend"
      assert_resource_member "${output}" "${directory}" network "${prefix}-shared-edge"
      ;;
    all)
      assert_named_member "${output}" "${directory}" db "${prefix}-forge-db"
      assert_named_member "${output}" "${directory}" peer "${prefix}-peer-app"
      assert_resource_member "${output}" "${directory}" volume "${prefix}-forge-db"
      assert_resource_member "${output}" "${directory}" volume "${prefix}-forge-data"
      assert_resource_member "${output}" "${directory}" network "${prefix}-forge-backend"
      assert_resource_member "${output}" "${directory}" network "${prefix}-shared-edge"
      ;;
  esac
}

forgejo_assert_external_edge() {
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

forgejo_run_exports() {
  local mode=$1 socket=$2 prefix=$3
  local selection output directory report
  local -a selection_arguments=()
  mkdir -p -- "${current_case}/outputs"
  for selection in exact label all; do
    case "${selection}" in
      exact)
        selection_arguments=(--podman-resource "container=${prefix}-forge-app")
        ;;
      label)
        selection_arguments=(--podman-label "io.boxferry.application=${prefix}-forgejo")
        ;;
      all)
        selection_arguments=(--podman-all)
        ;;
    esac
    for output in compose quadlet podman; do
      directory="${current_case}/outputs/${mode}-${selection}-${output}"
      report="${directory}.report.json"
      local -a target_arguments=()
      [[ "${output}" == podman ]] && target_arguments+=(--podman-target-context rootful)
      boxferry_operation "Forgejo ${mode} ${selection} Podman-to-${output}" \
        convert podman "${output}" --podman-socket "${socket}" \
        --application-name "${prefix}-forgejo" --loss-policy partial \
        --promote-podman-effective-named-volumes --promote-podman-effective-named-networks \
        --output-directory "${directory}" --console-format json \
        "${target_arguments[@]}" "${selection_arguments[@]}" > "${report}"
      jq --exit-status '
        .schema_version == 1 and .status == "success" and .exit_category == "success" and
        ([.diagnostics[]? | select(.severity == "error")] | length == 0) and
        ((.fidelity.invalid // 0) == 0) and (.output_artifacts | length > 0)
      ' "${report}" > /dev/null
      forgejo_assert_output_membership "${selection}" "${output}" "${directory}" "${prefix}"
      if [[ "${selection}" != all ]]; then
        forgejo_assert_external_edge "${output}" "${directory}" "${prefix}-shared-edge"
      fi
    done
  done
  if grep --recursive --include='*.report.json' --fixed-strings --quiet \
    -e "${FORGEJO_ADMIN_PASSWORD}" -e "${FORGEJO_DB_PASSWORD}" \
    -e "${FORGEJO_SECRET_KEY}" "${current_case}/outputs"; then
    printf 'Forgejo conversion report leaked a protected public canary.\n' >&2
    return 1
  fi
}

forgejo_clear_probe_state() {
  local socket=$1 prefix=$2
  local fixture_root="/tmp/boxferry-fixture/${prefix}"
  forgejo_remote "${socket}" run --rm --pull=never \
    --volume "${fixture_root}:/fixture:rw" --entrypoint /bin/sh \
    "$(forgejo_image_reference git-client)" -ceu \
    'rm -rf -- /fixture/probe-state'
}

forgejo_cleanup_mode() {
  local mode=$1 socket=$2 prefix=$3 run=$4
  if [[ "${mode}" == compose ]]; then
    forgejo_peer_compose_project "${socket}" "${prefix}" "${run}" \
      down --remove-orphans > "${current_case}/forgejo-peer-compose-down.log" 2>&1 || true
    forgejo_compose_project "${socket}" "${prefix}" "${run}" \
      down --volumes --remove-orphans > "${current_case}/forgejo-compose-down.log" 2>&1 || true
    # Rootless Podman can remove a container before reporting a netns teardown error.
    # Finish only the exact disposable prefix resources, then require an empty prefix.
    forgejo_remote "${socket}" rm --force --time 0 --ignore \
      "${prefix}-forge-app" "${prefix}-forge-db" "${prefix}-peer-app" > /dev/null 2>&1 || true
    forgejo_remote "${socket}" volume rm --force \
      "${prefix}-forge-db" "${prefix}-forge-data" > /dev/null 2>&1 || true
    forgejo_remote "${socket}" network rm \
      "${prefix}-forge-backend" > /dev/null 2>&1 || true
  else
    forgejo_remote "${socket}" stop --time 30 \
      "${prefix}-forge-app" "${prefix}-forge-db" > /dev/null || true
    forgejo_remote "${socket}" rm --force --time 0 --ignore \
      "${prefix}-forge-app" "${prefix}-forge-db" "${prefix}-peer-app" > /dev/null 2>&1 || true
    forgejo_remote "${socket}" volume rm --force \
      "${prefix}-forge-db" "${prefix}-forge-data" > /dev/null 2>&1 || true
    forgejo_remote "${socket}" network rm \
      "${prefix}-forge-backend" > /dev/null 2>&1 || true
  fi
  forgejo_remote "${socket}" network rm \
    "${prefix}-shared-edge" > /dev/null 2>&1 || true
  forgejo_clear_probe_state "${socket}" "${prefix}"
  forgejo_assert_clean_prefix "${socket}" "${prefix}"
}

forgejo_verify_target() {
  local id=$1 image=$2 declared_version=$3 distribution=$4 mode=$5 lane=$6 architecture=$7
  verify_observed_version "${id}" "${declared_version}" "${artifact_root}/${id}.podman-version"
  [[ "$(< "${artifact_root}/${id}.digest")" == "${image##*@}" ]] || {
    printf 'Pulled image digest does not match reviewed Forgejo target %s.\n' "${id}" >&2
    return 1
  }
  [[ "${architecture}" == amd64 && "$(< "${artifact_root}/${id}.architecture")" =~ ^(x86_64|amd64)$ ]] || {
    printf 'Observed architecture does not match Forgejo target %s.\n' "${id}" >&2
    return 1
  }
  append_verified_evidence "${id}" "${image}" "${declared_version}" "${distribution}" \
    "${mode}" "${lane}" "${architecture}" "${artifact_root}/${id}" nested-image forgejo-application
}

run_forgejo_application_cell() {
  local id=$1 image=$2 declared_version=$3 distribution=$4 mode=$5 lane=$6 architecture=$7
  case "${id}-${mode}" in
    podman-arch-rootful-rootful | podman-6.1-rootless-rootless) ;;
    *)
      printf 'Forgejo profile is bounded to reviewed Podman 6.1 rootful and rootless cells.\n' >&2
      return 1
      ;;
  esac
  current_prefix="${run_id:0:31}-fg-${mode:0:2}"
  current_case="${artifact_root}/${id}-forgejo-application"
  local socket_directory="${runtime_root}/forgejo-application-target"
  local application_archive="${runtime_root}/forgejo-application-images.tar"
  mkdir -p -- "${current_case}" "${socket_directory}"
  chmod 0700 "${current_case}" "${socket_directory}"
  progress_index=0
  progress_total=25
  printf '%s PLAN %s forgejo-application tests=%d\n' \
    "$(timestamp)" "${id}" "${progress_total}"
  progress_run 'validate Forgejo catalogues' forgejo_validate_catalogues
  progress_run 'validate Docker Compose provider' forgejo_validate_provider
  progress_run 'prepare digest-pinned Forgejo image archive' \
    forgejo_prepare_image_archive "${application_archive}"
  progress_run 'start isolated Forgejo Podman target' \
    start_outer_runtime "${id}" "${image}" "${mode}" "${socket_directory}" \
    "${application_archive}"
  progress_run 'verify Podman Forgejo target evidence' \
    forgejo_verify_target "${id}" "${image}" "${declared_version}" "${distribution}" \
    "${mode}" "${lane}" "${architecture}"
  local outer="${started_outer}"
  progress_run 'load Forgejo images and mode-aware network configuration' \
    forgejo_prepare_application_target "${outer}" "${current_prefix}" \
    "${socket_directory}" "${mode}"
  local socket="${socket_directory}/podman.sock"

  progress_run 'provision independent Podman CLI Forgejo application' \
    forgejo_provision_cli "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'wait for Podman CLI Forgejo readiness' \
    forgejo_wait_application "${socket}" "${current_prefix}"
  progress_run 'refuse colliding Podman CLI Forgejo prefix' \
    forgejo_expect_collision "${socket}" "${current_prefix}"
  progress_run 'create repository and prove HTTP plus SSH Git operations' \
    forgejo_create_admin "${socket}" "${current_prefix}"
  progress_run 'push and clone repository through HTTP and SSH' \
    forgejo_git_probe "${socket}" "${current_prefix}" seed
  progress_run 'prove Podman CLI database and publication boundaries' \
    forgejo_assert_application_boundaries "${socket}" "${current_prefix}"
  progress_run 'exercise Podman CLI exact and label source selectors' \
    forgejo_run_exports cli "${socket}" "${current_prefix}"
  progress_run 'prove Podman CLI repository persistence after recreation' \
    forgejo_recreate_application cli "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'clean Podman CLI Forgejo resources and probe key' \
    forgejo_cleanup_mode cli "${socket}" "${current_prefix}" "${run_id}"

  progress_run 'provision independent Docker Compose Forgejo application' \
    forgejo_provision_compose "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'wait for Docker Compose Forgejo readiness' \
    forgejo_wait_application "${socket}" "${current_prefix}"
  progress_run 'refuse colliding Docker Compose Forgejo prefix' \
    forgejo_expect_collision "${socket}" "${current_prefix}"
  progress_run 'create Docker Compose Forgejo repository administrator' \
    forgejo_create_admin "${socket}" "${current_prefix}"
  progress_run 'prove Docker Compose HTTP and SSH Git operations' \
    forgejo_git_probe "${socket}" "${current_prefix}" seed
  progress_run 'prove Docker Compose database and publication boundaries' \
    forgejo_assert_application_boundaries "${socket}" "${current_prefix}"
  progress_run 'exercise Docker Compose exact and label source selectors' \
    forgejo_run_exports compose "${socket}" "${current_prefix}"
  progress_run 'prove Docker Compose repository persistence after recreation' \
    forgejo_recreate_application compose "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'clean Docker Compose Forgejo resources and probe key' \
    forgejo_cleanup_mode compose "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'remove disposable Forgejo outer container' remove_outer "${outer}"
  printf '%s CELL PASS %s forgejo-application (%d/%d tests)\n' \
    "$(timestamp)" "${id}" "${progress_index}" "${progress_total}"
}
