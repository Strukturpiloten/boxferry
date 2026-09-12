#!/usr/bin/env bash

# Sourced by scripts/podman-live-conformance.sh after its shared helpers.
# This module owns the issue-140 fixture lane registered by the shared runner;
# migration-readiness selection remains with the protected tier catalogue.
# shellcheck disable=SC2016,SC2129,SC2154

SUPABASE_PROVIDER_VERSION="5.5.0"
SUPABASE_PROVIDER_SHA256="c57ab918abd5b05ca7e7d0f275875dd1330a695074f309dc9eab1b49efafcd4b"
SUPABASE_HTTP_PORT="18000"
SUPABASE_ARCHIVE_MAX_BYTES="5368709120"
SUPABASE_MIN_CPUS="4"
SUPABASE_MIN_MEMORY_KIB="12582912"
SUPABASE_MIN_DISK_KIB="25165824"
SUPABASE_MAX_CONCURRENCY="1"
SUPABASE_CELL_TIMEOUT="90m"
SUPABASE_CELL_KILL_AFTER="10s"
SUPABASE_DB_PASSWORD="boxferry-public-supabase-db-password"
SUPABASE_JWT_SECRET="boxferry-public-jwt-secret-at-least-thirty-two-characters"
SUPABASE_ANON_KEY="eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJhdWQiOiJhdXRoZW50aWNhdGVkIiwiZXhwIjo0MTAyNDQ0ODAwLCJpYXQiOjE3MDQwNjcyMDAsImlzcyI6InN1cGFiYXNlIiwicm9sZSI6ImFub24ifQ.lGRwynjOirFYPqa-6IzEfCCIS_yNsl4riV9_TYukv0g"
SUPABASE_SERVICE_KEY="eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJhdWQiOiJhdXRoZW50aWNhdGVkIiwiZXhwIjo0MTAyNDQ0ODAwLCJpYXQiOjE3MDQwNjcyMDAsImlzcyI6InN1cGFiYXNlIiwicm9sZSI6InNlcnZpY2Vfcm9sZSJ9.GqsNLUWNCMg6So_4dAH5LRG2EtPKRYL2wb9gffU8eTU"
SUPABASE_REALTIME_SECRET="boxferry-public-realtime-secret-key-base-64-characters-long-0000000000"
SUPABASE_POOLER_SECRET="boxferry-public-pooler-secret-key-base-64-characters-long-000000000000"
SUPABASE_REALTIME_DB_KEY="boxferry-public-realtime-db-key"
SUPABASE_META_CRYPTO_KEY="boxferry-public-pg-meta-crypto-key"
SUPABASE_VAULT_ENC_KEY="boxferry-public-vault-key-32byte"
SUPABASE_TEST_EMAIL="boxferry-supabase@example.invalid"
SUPABASE_TEST_PASSWORD="boxferry-public-auth-password"

supabase_fixture_root() {
  printf '%s/fixtures/conformance/supabase-application\n' \
    "${repository_root:?caller must supply repository_root}"
}

supabase_source_image_reference() {
  local id=${1:?image id required}
  awk -F '\t' -v id="${id}" '$1 == id { print $2; found = 1 } END { exit !found }' \
    "$(supabase_fixture_root)/images.tsv"
}

supabase_image_reference() {
  local id=${1:?image id required}
  awk -F '\t' -v id="${id}" '
        $1 == id {
            sub(/@sha256:[0-9a-f]+$/, "", $2)
            print $2 "@" $4
            found = 1
        }
        END { exit !found }
    ' "$(supabase_fixture_root)/images.tsv"
}

supabase_validate_resource_budget() {
  local cpus memory_kib graph_root path observed disk_kib=""
  cpus="$(nproc)"
  memory_kib="$(awk '$1 == "MemAvailable:" { print $2 }' /proc/meminfo)"
  graph_root="$("${engine}" info --format '{{.Store.GraphRoot}}')"
  for path in "${runtime_root:?caller must supply runtime_root}" "${graph_root}"; do
    observed="$(df --output=avail -k -- "${path}" | awk 'NR == 2 { print $1 }')"
    if [[ -z "${disk_kib}" || "${observed}" -lt "${disk_kib}" ]]; then
      disk_kib="${observed}"
    fi
  done
  [[ "${cpus}" =~ ^[0-9]+$ && "${cpus}" -ge "${SUPABASE_MIN_CPUS}" ]] || {
    printf 'Supabase profile requires at least %s CPUs; found %s.\n' \
      "${SUPABASE_MIN_CPUS}" "${cpus:-unknown}" >&2
    return 1
  }
  [[ "${memory_kib}" =~ ^[0-9]+$ && "${memory_kib}" -ge "${SUPABASE_MIN_MEMORY_KIB}" ]] || {
    printf 'Supabase profile requires at least 12 GiB available memory; found %s KiB.\n' \
      "${memory_kib:-unknown}" >&2
    return 1
  }
  [[ "${disk_kib}" =~ ^[0-9]+$ && "${disk_kib}" -ge "${SUPABASE_MIN_DISK_KIB}" ]] || {
    printf 'Supabase profile requires at least 24 GiB free disk; found %s KiB.\n' \
      "${disk_kib:-unknown}" >&2
    return 1
  }
  printf '%s SUPABASE BUDGET cpus=%s memory-available-kib=%s disk-available-kib=%s\n' \
    "$(timestamp)" "${cpus}" "${memory_kib}" "${disk_kib}"
}

supabase_validate_catalogues() {
  local fixture
  fixture="$(supabase_fixture_root)"
  [[ "${SUPABASE_MAX_CONCURRENCY}" == 1 && "${SUPABASE_CELL_TIMEOUT}" == 90m &&
    "${SUPABASE_CELL_KILL_AFTER}" == 10s ]] || {
    printf 'Supabase profile requires concurrency one and a 90-minute cell bound.\n' >&2
    return 1
  }
  awk -F '\t' '
    NF && $1 !~ /^#/ {
            if (NF != 12 || $2 !~ /:[^\/@]+@sha256:[0-9a-f]{64}$/ ||
                    $3 != "linux/amd64" || $4 !~ /^sha256:[0-9a-f]{64}$/ ||
                    $8 !~ /^[0-9a-f]{40}$/ || $10 !~ /^[0-9a-f]{64}$/ ||
                    $11 != "not-redistributed-transient-test-pull" || seen[$1]++) {
        printf "invalid Supabase image row %d: %s\\n", NR, $0 > "/dev/stderr"; bad = 1
      }
    }
    END { if (length(seen) != 11) bad = 1; exit bad }
  ' "${fixture}/images.tsv"
  awk -F '\t' '
    NF && $1 !~ /^#/ {
      if (NF != 8 || !$1 || !$2 || !$3 || !$4 || !$5 || !$6 || !$7 || !$8 || seen[$1]++) {
        bad = 1
      }
      if ($3 != "NOASSERTION" && $3 !~ /^https:\/\/github\.com\//) bad = 1
      if ($4 != "NOASSERTION" && $4 !~ /^[0-9a-f]{40}$/) bad = 1
      if ($1 == "supabase-postgres" &&
          ($2 != "17.6.1.136" || $3 != "https://github.com/supabase/postgres" ||
           $4 != "d156ba65c14694c12cc5e782bc15b9b8ed2d1376" ||
           $7 != "source-file-reviewed")) bad = 1
      if (($1 == "pgcrypto" || $1 == "pg_stat_statements") &&
          $7 != "runtime-presence-pending") bad = 1
    }
    END { exit bad || length(seen) != 15 }
  ' "${fixture}/postgres-components.tsv"
  awk -F '\t' -v version="${SUPABASE_PROVIDER_VERSION}" \
    -v sha="${SUPABASE_PROVIDER_SHA256}" '
    NF && $1 !~ /^#/ {
      if (NF != 8 || $1 != "docker-compose" || $2 != version ||
          $3 != "linux-amd64" || $4 != sha || $8 != "downloaded-test-tool") bad = 1
      count++
    }
    END { exit bad || count != 1 }
  ' "${fixture}/providers.tsv"
  local authored_sha
  authored_sha="$(sha256sum "${fixture}/compose.yaml" | awk '{ print $1 }')"
  awk -F '\t' -v authored_sha="${authored_sha}" '
    NF && $1 !~ /^#/ {
      if (NF != 8 || seen[$1]++) bad = 1
      if ($1 == "supabase-self-hosted" &&
          ($2 != "https://github.com/supabase/supabase" ||
           $3 != "9952d6f10fb9a7b2d9d8c3312b279bfab2c4ba96" ||
           $4 != "docker/docker-compose.yml" ||
          $5 != "340bb4d4b79c19e682d1a6297d56a6ec63997a49e0e19611c2cb3a6332eb32de" ||
           $6 != "Apache-2.0" || $8 != "not-redistributed")) bad = 1
      if ($1 == "boxferry-supabase-application" &&
          ($3 != "repository-authored" || $5 != authored_sha ||
           $6 != "MPL-2.0" || $8 != "repository-source")) bad = 1
      count++
    }
    END { exit bad || count != 2 || length(seen) != 2 }
  ' "${fixture}/application.tsv"
  awk -F '\t' '
    BEGIN {
    approved["podman compose"] = "BFP0002,BFP0003,BFC0007,BFC0009"
      approved["podman quadlet"] = "BFP0002,BFP0003"
      approved["podman podman"] = "BFP0002,BFP0003,BFP0007"
    approved["compose compose"] = "BFC0009"
      approved["compose quadlet"] = "-"
      approved["compose podman"] = "BFP0007,BFP0008"
    approved["quadlet compose"] = "BFC0007,BFC0009"
      approved["quadlet quadlet"] = "-"
      approved["quadlet podman"] = "BFP0007,BFP0008"
      contract["podman compose"] = "exact-diagnostic-tuple-multiset-plus-fidelity-v1"
      contract["podman quadlet"] = "exact-diagnostic-tuple-multiset-plus-fidelity-v1"
      contract["podman podman"] = "exact-diagnostic-tuple-multiset-plus-fidelity-v1"
    contract["compose compose"] = "exact-diagnostic-tuple-multiset-plus-fidelity-v1"
      contract["compose quadlet"] = "zero-loss-zero-diagnostic-reimport"
      contract["compose podman"] = "one-error-per-tag-and-digest-image-no-artifacts"
      contract["quadlet compose"] = "exact-diagnostic-tuple-multiset-plus-fidelity-v1"
      contract["quadlet quadlet"] = "zero-loss-zero-diagnostic-reimport"
      contract["quadlet podman"] = "one-error-per-tag-and-digest-image-no-artifacts"
    }
    NF && $1 !~ /^#/ {
      key = $1 "->" $2
      route = $1 " " $2
      if (NF != 6 || seen[key]++ || $4 != "live-unperformed" ||
          !(route in approved) || $5 != approved[route] || $6 != contract[route]) bad = 1
      success += ($3 == "migration-success")
      rejected += ($3 == "expected-rejection")
    }
    END { exit bad || length(seen) != 9 || success != 7 || rejected != 2 }
  ' "${fixture}/routes.tsv"
  awk -F '\t' '
    BEGIN {
      image["db"] = "db";                 networks["db"] = "backend";        dependencies["db"] = "-";                                       mounts["db"] = "pgdata:/var/lib/postgresql/data:rw,bind:/docker-entrypoint-initdb.d/80-boxferry.sql:ro"; proof["db"] = "SQL-row-extension-publication-counts"
      image["auth"] = "auth";             networks["auth"] = "backend";      dependencies["auth"] = "db";                                     mounts["auth"] = "-";                                                                                             proof["auth"] = "sign-up-sign-in-current-user"
      image["rest"] = "rest";             networks["rest"] = "backend";      dependencies["rest"] = "db";                                     mounts["rest"] = "-";                                                                                             proof["rest"] = "insert-select-and-Realtime-source"
      image["realtime"] = "realtime";     networks["realtime"] = "backend";  dependencies["realtime"] = "db";                                 mounts["realtime"] = "-";                                                                                         proof["realtime"] = "Phoenix-WebSocket-PostgreSQL-insert"
      image["imgproxy"] = "imgproxy";     networks["imgproxy"] = "backend";  dependencies["imgproxy"] = "-";                                 mounts["imgproxy"] = "storage:/var/lib/storage:ro";                                                              proof["imgproxy"] = "health-and-shared-consumer-mode"
      image["storage"] = "storage";       networks["storage"] = "backend";   dependencies["storage"] = "db,imgproxy,rest";                     mounts["storage"] = "storage:/var/lib/storage:rw";                                                               proof["storage"] = "private-bucket-upload-byte-exact-download"
      image["meta"] = "meta";             networks["meta"] = "backend";      dependencies["meta"] = "db";                                     mounts["meta"] = "-";                                                                                             proof["meta"] = "HTTP-health"
      image["supavisor"] = "supavisor";   networks["supavisor"] = "backend"; dependencies["supavisor"] = "db";                                mounts["supavisor"] = "-";                                                                                        proof["supavisor"] = "HTTP-health-and-private-database"
      image["functions"] = "functions";   networks["functions"] = "backend"; dependencies["functions"] = "db";                                mounts["functions"] = "bind:/home/deno/functions:ro,deno-cache:/root/.cache/deno:rw";                              proof["functions"] = "seed-SHA-256-cd2c400852048a021086994cc5f266472d53e72ffddb1c1f5d01a17ddaa27ca4-and-verify-SHA-256-71059a67ee64b2891c41a31b66660b18e366342f765ccf07fd68fb0436eb6638"
      image["studio"] = "studio";         networks["studio"] = "backend";    dependencies["studio"] = "meta";                                  mounts["studio"] = "bind:/boxferry-fixture:ro";                                                                  proof["studio"] = "profile-API-and-probe-runtime"
      image["kong"] = "kong";             networks["kong"] = "backend,edge"; dependencies["kong"] = "auth,functions,realtime,rest,storage,studio"; mounts["kong"] = "bind:/etc/kong/kong.yml:ro";                                                                    proof["kong"] = "loopback-only-public-gateway"
    }
    NR == 1 {
      if ($0 != "# schema=2; service image-id networks dependencies mounts runtime-proof") bad = 1
      next
    }
    NF && $1 ~ /^#/ { bad = 1 }
    NF && $1 !~ /^#/ {
      if (NF != 6 || !($1 in image) || seen[$1]++ ||
          $2 != image[$1] || $3 != networks[$1] ||
          $4 != dependencies[$1] || $5 != mounts[$1] ||
          $6 != proof[$1]) bad = 1
    }
    END { exit bad || length(seen) != 11 }
  ' "${fixture}/graph.tsv"
  local expected
  for expected in application.tsv application-probe.mjs compose.yaml db-init.sql \
    graph.tsv images.tsv kong.yml peer.compose.yaml postgres-components.tsv \
    providers.tsv README.md routes.tsv success-contract.jq functions/main/index.ts; do
    [[ -s "${fixture}/${expected}" ]] || {
      printf 'Missing Supabase fixture file: %s\n' "${expected}" >&2
      return 1
    }
  done
  supabase_validate_success_contract_examples
}

supabase_validate_provider() {
  local provider observed_sha observed_version
  provider=${BOXFERRY_COMPOSE_BIN:-${repository_root}/target/tools/docker-compose}
  [[ -x "${provider}" ]] || {
    printf 'Supabase profile requires executable Docker Compose %s at %s.\n' \
      "${SUPABASE_PROVIDER_VERSION}" "${provider}" >&2
    return 1
  }
  observed_sha="$(sha256sum "${provider}" | awk '{ print $1 }')"
  [[ "${observed_sha}" == "${SUPABASE_PROVIDER_SHA256}" ]] || {
    printf 'Docker Compose checksum mismatch: expected %s, observed %s.\n' \
      "${SUPABASE_PROVIDER_SHA256}" "${observed_sha}" >&2
    return 1
  }
  observed_version="$("${provider}" version --short | sed 's/^v//')"
  [[ "${observed_version}" == "${SUPABASE_PROVIDER_VERSION}" ]]
}

supabase_validate_image_archive() {
  local id=$1 archive=$2 archive_reference=$3 expected_digest=$4 digest_file=$5
  local written_digest descriptor descriptor_digest descriptor_reference blob_digest
  [[ -s "${digest_file}" ]] || {
    printf 'Supabase %s archive did not record a manifest digest.\n' "${id}" >&2
    return 1
  }
  written_digest="$(< "${digest_file}")"
  [[ "${written_digest}" == "${expected_digest}" ]] || {
    printf 'Supabase %s archive digest mismatch: expected %s, observed %s.\n' \
      "${id}" "${expected_digest}" "${written_digest}" >&2
    return 1
  }
  descriptor="$(
    tar --extract --to-stdout --file "${archive}" index.json | jq --exit-status --raw-output \
      --arg reference "${archive_reference}" '
                if .schemaVersion == 2 and (.manifests | length) == 1 and
                    .manifests[0].annotations["org.opencontainers.image.ref.name"] == $reference
                then [.manifests[0].digest, $reference] | @tsv
                else error("unexpected Supabase OCI archive index")
                end
            '
  )" || {
    printf 'Supabase %s OCI archive has an invalid descriptor or reference.\n' "${id}" >&2
    return 1
  }
  IFS=$'\t' read -r descriptor_digest descriptor_reference <<< "${descriptor}"
  [[ "${descriptor_digest}" == "${expected_digest}" &&
    "${descriptor_reference}" == "${archive_reference}" ]] || {
    printf 'Supabase %s OCI descriptor mismatch: expected %s at %s, observed %s at %s.\n' \
      "${id}" "${expected_digest}" "${archive_reference}" \
      "${descriptor_digest}" "${descriptor_reference}" >&2
    return 1
  }
  blob_digest="sha256:$(
    tar --extract --to-stdout --file "${archive}" \
      "blobs/sha256/${expected_digest#sha256:}" | sha256sum | awk '{ print $1 }'
  )"
  [[ "${blob_digest}" == "${expected_digest}" ]] || {
    printf 'Supabase %s OCI manifest blob mismatch: expected %s, observed %s.\n' \
      "${id}" "${expected_digest}" "${blob_digest}" >&2
    return 1
  }
}

supabase_prepare_image_archive() {
  local archive=${1:?archive path required}
  if [[ -s "${archive}" ]]; then
    [[ "$(stat -c '%s' "${archive}")" -le "${SUPABASE_ARCHIVE_MAX_BYTES}" ]]
    return
  fi
  local fixture id reference platform platform_digest expected_digest status archive_directory
  local observed_digest observed_architecture observed_os image_id archive_reference archive_path
  local archive_digest_file
  fixture="$(supabase_fixture_root)"
  archive_directory="$(mktemp -d "${runtime_root}/supabase-image-archives.XXXXXX")"
  while IFS=$'\t' read -r id reference platform platform_digest _; do
    [[ -z "${id}" || "${id}" == \#* ]] && continue
    status=0
    engine_image_available "probe Supabase ${id} image cache" "${reference}" || status=$?
    if ((status == 1)); then
      timed_operation 20m "pull digest-pinned Supabase ${id} image" \
        "${engine}" pull --quiet --platform "${platform}" "${reference}" \
        > "${artifact_root}/supabase-${id}.pull.log" 2>&1
      record_run_owned_host_image "${reference}"
    elif ((status != 0)); then
      return "${status}"
    fi
    expected_digest="${reference##*@}"
    read -r observed_digest observed_architecture observed_os image_id < <(
      "${engine}" image inspect \
        --format '{{.Digest}} {{.Architecture}} {{.Os}} {{.Id}}' "${reference}"
    )
    [[ "${observed_digest}" == "${expected_digest}" &&
      "${observed_os}/${observed_architecture}" == "${platform}" ]] || {
      printf 'Supabase %s source identity mismatch: expected %s at %s, observed %s at %s/%s.\n' \
        "${id}" "${expected_digest}" "${platform}" "${observed_digest}" \
        "${observed_os}" "${observed_architecture}" >&2
      return 1
    }
    archive_reference="${reference%@*}"
    archive_path="${archive_directory}/${id}.oci.tar"
    archive_digest_file="${artifact_root}/supabase-${id}.archive-digest"
    timed_operation 8m "write Supabase ${id} OCI archive" \
      "${engine}" push --quiet --digestfile "${archive_digest_file}" \
      "${image_id}" "oci-archive:${archive_path}:${archive_reference}"
    supabase_validate_image_archive "${id}" "${archive_path}" \
      "${archive_reference}" "${platform_digest}" "${archive_digest_file}"
    release_run_owned_host_image "${reference}"
  done < "${fixture}/images.tsv"
  tar --remove-files --sort=name --mtime='UTC 1970-01-01' --owner=0 --group=0 --numeric-owner \
    --mode='u+rw,go+rX,go-w' -C "${archive_directory}" -cf "${archive}" .
  rm -rf -- "${archive_directory}"
  [[ "$(stat -c '%s' "${archive}")" -le "${SUPABASE_ARCHIVE_MAX_BYTES}" ]] || {
    printf 'Supabase application archive exceeds 5 GiB cap.\n' >&2
    return 1
  }
}

supabase_assert_loaded_images() {
  local outer=$1 id platform_digest runtime_reference observed_digest
  while IFS=$'\t' read -r id _ _ platform_digest _; do
    [[ -z "${id}" || "${id}" == \#* ]] && continue
    runtime_reference="$(supabase_image_reference "${id}")"
    engine_operation "verify loaded Supabase ${id} image" \
      exec "${outer}" podman image exists "${runtime_reference}"
    observed_digest="$(
      engine_operation "inspect loaded Supabase ${id} image digest" \
        exec "${outer}" podman image inspect --format '{{.Digest}}' \
        "${runtime_reference}"
    )"
    [[ "${observed_digest}" == "${platform_digest}" ]] || {
      printf 'Loaded Supabase %s digest mismatch: expected %s, observed %s.\n' \
        "${id}" "${platform_digest}" "${observed_digest}" >&2
      return 1
    }
  done < "$(supabase_fixture_root)/images.tsv"
}

supabase_prepare_application_target() {
  local outer=$1 prefix=$2 socket_directory=$3
  local fixture destination
  fixture="$(supabase_fixture_root)"
  destination="/tmp/boxferry-fixture/${prefix}"
  engine_operation 'copy rootless Supabase network configuration' cp \
    "${repository_root}/fixtures/conformance/podman-live/apply-target-containers.conf" \
    "${outer}:/tmp/99-boxferry-live.conf"
  engine_operation 'prepare rootless Supabase network configuration' \
    exec "${outer}" /bin/sh -ceu \
    'mkdir -p "$HOME/.config/containers/containers.conf.d"; cp /tmp/99-boxferry-live.conf "$HOME/.config/containers/containers.conf.d/99-boxferry-live.conf"'
  timed_operation 25m 'load digest-pinned Supabase application archives' \
    "${engine}" exec "${outer}" /bin/sh -ceu '
      directory=/tmp/boxferry-supabase-images
      mkdir -p "$directory"
      trap "rm -rf -- \"$directory\"" EXIT
      tar -xf /boxferry-workload.tar -C "$directory"
 for archive in "$directory"/*.oci.tar; do
   podman load --input "$archive"
   rm -f -- "$archive"
 done
 rmdir "$directory"
    ' > /dev/null
  supabase_assert_loaded_images "${outer}"
  engine_operation 'create disposable Supabase fixture directory' \
    exec "${outer}" mkdir -p -- "${destination}"
  engine_operation 'copy reviewed authored Supabase fixture' \
    cp "${fixture}/." "${outer}:${destination}"
  activate_outer_runtime "${socket_directory}"
}

supabase_remote() {
  local socket=$1
  shift
  podman_socket "${socket}" "Supabase ${1:-command}" "$@"
}

supabase_wait_for() {
  local deadline_seconds=$1 description=$2
  shift 2
  local deadline=$((SECONDS + deadline_seconds))
  until "$@" > /dev/null 2>&1; do
    if ((SECONDS >= deadline)); then
      printf 'Timed out waiting for %s.\n' "${description}" >&2
      return 1
    fi
    sleep 2
  done
}

supabase_container_names() {
  local prefix=$1 service
  for service in db auth rest realtime imgproxy storage meta supavisor functions studio kong; do
    printf '%s-supabase-%s\n' "${prefix}" "${service}"
  done
}

supabase_assert_clean_prefix() {
  local socket=$1 prefix=$2
  local found=""
  found="$(supabase_remote "${socket}" ps -a --format '{{.Names}}' |
    awk -v prefix="${prefix}-supabase-" 'index($0, prefix) == 1')"
  [[ -z "${found}" ]] || {
    printf 'Supabase prefix collision detected:\n%s\n' "${found}" >&2
    return 1
  }
}

supabase_create_networks_volumes() {
  local socket=$1 prefix=$2 run=$3 volume
  supabase_remote "${socket}" network create --internal \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-supabase" \
    "${prefix}-supabase-backend" > /dev/null
  supabase_remote "${socket}" network create \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-supabase" \
    "${prefix}-supabase-edge" > /dev/null
  for volume in pgdata storage deno-cache; do
    supabase_remote "${socket}" volume create \
      --label "io.boxferry.live-run=${run}" \
      --label "io.boxferry.application=${prefix}-supabase" \
      "${prefix}-supabase-${volume}" > /dev/null
  done
}

supabase_create_cli_database() {
  local socket=$1 prefix=$2 run=$3 fixture_root
  fixture_root="/tmp/boxferry-fixture/${prefix}"
  supabase_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-supabase-db" \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-supabase" \
    --network "${prefix}-supabase-backend:alias=db" \
    --volume "${prefix}-supabase-pgdata:/var/lib/postgresql/data" \
    --volume "${fixture_root}/db-init.sql:/docker-entrypoint-initdb.d/80-boxferry.sql:ro" \
    --env POSTGRES_DB=postgres --env POSTGRES_USER=postgres \
    --env "POSTGRES_PASSWORD=${SUPABASE_DB_PASSWORD}" \
    --health-cmd 'pg_isready -U postgres -d postgres' \
    --health-interval 2s --health-retries 150 \
    "$(supabase_image_reference db)" postgres -c log_min_messages=fatal > /dev/null
}

supabase_create_cli_services() {
  local socket=$1 prefix=$2 run=$3 fixture_root
  fixture_root="/tmp/boxferry-fixture/${prefix}"
  supabase_create_cli_database "${socket}" "${prefix}" "${run}"
  supabase_wait_for 360 'Supabase PostgreSQL readiness' \
    supabase_remote "${socket}" exec "${prefix}-supabase-db" \
    pg_isready -U postgres -d postgres

  supabase_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-supabase-auth" \
    --label "io.boxferry.live-run=${run}" --label "io.boxferry.application=${prefix}-supabase" \
    --requires "${prefix}-supabase-db" --network "${prefix}-supabase-backend:alias=auth" \
    --env GOTRUE_API_HOST=0.0.0.0 --env GOTRUE_API_PORT=9999 \
    --env API_EXTERNAL_URL=http://127.0.0.1:18000 \
    --env GOTRUE_SITE_URL=http://127.0.0.1:18000 \
    --env GOTRUE_DB_DRIVER=postgres \
    --env "GOTRUE_DB_DATABASE_URL=postgres://supabase_auth_admin:${SUPABASE_DB_PASSWORD}@db:5432/postgres" \
    --env GOTRUE_DISABLE_SIGNUP=false --env GOTRUE_EXTERNAL_EMAIL_ENABLED=true \
    --env GOTRUE_MAILER_AUTOCONFIRM=true --env GOTRUE_JWT_ADMIN_ROLES=service_role \
    --env GOTRUE_JWT_AUD=authenticated --env GOTRUE_JWT_DEFAULT_GROUP_NAME=authenticated \
    --env GOTRUE_JWT_EXP=3600 --env "GOTRUE_JWT_SECRET=${SUPABASE_JWT_SECRET}" \
    --health-cmd 'wget --no-verbose --tries=1 --spider http://127.0.0.1:9999/health' \
    --health-interval 2s --health-retries 150 "$(supabase_image_reference auth)" > /dev/null

  supabase_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-supabase-rest" \
    --label "io.boxferry.live-run=${run}" --label "io.boxferry.application=${prefix}-supabase" \
    --requires "${prefix}-supabase-db" --network "${prefix}-supabase-backend:alias=rest" \
    --env PGRST_ADMIN_SERVER_HOST=0.0.0.0 --env PGRST_ADMIN_SERVER_PORT=3001 \
    --env PGRST_DB_ANON_ROLE=anon --env PGRST_DB_EXTRA_SEARCH_PATH=public \
    --env PGRST_DB_SCHEMAS=public \
    --env "PGRST_DB_URI=postgres://authenticator:${SUPABASE_DB_PASSWORD}@db:5432/postgres" \
    --env "PGRST_JWT_SECRET=${SUPABASE_JWT_SECRET}" \
    --health-cmd 'postgrest --ready' --health-interval 2s --health-retries 150 \
    "$(supabase_image_reference rest)" > /dev/null

  supabase_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-supabase-realtime" \
    --label "io.boxferry.live-run=${run}" --label "io.boxferry.application=${prefix}-supabase" \
    --requires "${prefix}-supabase-db" \
    --network "${prefix}-supabase-backend:alias=realtime-dev.supabase-realtime" \
    --network-alias realtime --env PORT=4000 --env DB_HOST=db --env DB_PORT=5432 \
    --env DB_USER=supabase_admin --env "DB_PASSWORD=${SUPABASE_DB_PASSWORD}" \
    --env DB_NAME=postgres --env 'DB_AFTER_CONNECT_QUERY=SET search_path TO _realtime' \
    --env "DB_ENC_KEY=${SUPABASE_REALTIME_DB_KEY}" \
    --env "API_JWT_SECRET=${SUPABASE_JWT_SECRET}" \
    --env "SECRET_KEY_BASE=${SUPABASE_REALTIME_SECRET}" \
    --env APP_NAME=realtime --env SEED_SELF_HOST=true --env RUN_JANITOR=false \
    --env 'ERL_AFLAGS=-proto_dist inet_tcp' --env "DNS_NODES=''" --env RLIMIT_NOFILE=10000 \
    --health-cmd "curl -fsS -o /dev/null http://127.0.0.1:4000/api/tenants/realtime-dev/health -H 'Authorization: Bearer ${SUPABASE_ANON_KEY}'" \
    --health-interval 5s --health-timeout 5s --health-retries 120 --health-start-period 10s \
    "$(supabase_image_reference realtime)" > /dev/null

  supabase_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-supabase-imgproxy" \
    --label "io.boxferry.live-run=${run}" --label "io.boxferry.application=${prefix}-supabase" \
    --network "${prefix}-supabase-backend:alias=imgproxy" \
    --volume "${prefix}-supabase-storage:/var/lib/storage:ro" \
    --env IMGPROXY_BIND=:5001 --env IMGPROXY_LOCAL_FILESYSTEM_ROOT=/ \
    --env IMGPROXY_USE_ETAG=true --env IMGPROXY_MAX_SRC_RESOLUTION=16.8 \
    --health-cmd 'imgproxy health' --health-interval 2s --health-retries 150 \
    "$(supabase_image_reference imgproxy)" > /dev/null

  supabase_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-supabase-storage" \
    --label "io.boxferry.live-run=${run}" --label "io.boxferry.application=${prefix}-supabase" \
    --label io.boxferry.selection=storage \
    --requires "${prefix}-supabase-db,${prefix}-supabase-rest,${prefix}-supabase-imgproxy" \
    --network "${prefix}-supabase-backend:alias=storage" \
    --volume "${prefix}-supabase-storage:/var/lib/storage" \
    --env "ANON_KEY=${SUPABASE_ANON_KEY}" --env "SERVICE_KEY=${SUPABASE_SERVICE_KEY}" \
    --env "AUTH_JWT_SECRET=${SUPABASE_JWT_SECRET}" \
    --env "DATABASE_URL=postgres://supabase_storage_admin:${SUPABASE_DB_PASSWORD}@db:5432/postgres" \
    --env POSTGREST_URL=http://rest:3000 --env STORAGE_BACKEND=file \
    --env FILE_STORAGE_BACKEND_PATH=/var/lib/storage --env GLOBAL_S3_BUCKET=stub \
    --env TENANT_ID=stub --env REGION=local --env REQUEST_ALLOW_X_FORWARDED_PATH=true \
    --env FILE_SIZE_LIMIT=1048576 --env ENABLE_IMAGE_TRANSFORMATION=true \
    --env IMGPROXY_URL=http://imgproxy:5001 \
    --env STORAGE_PUBLIC_URL=http://127.0.0.1:18000/storage/v1 \
    --health-cmd 'wget --no-verbose --tries=1 --spider http://127.0.0.1:5000/status' \
    --health-interval 3s --health-retries 150 "$(supabase_image_reference storage)" > /dev/null

  supabase_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-supabase-meta" \
    --label "io.boxferry.live-run=${run}" --label "io.boxferry.application=${prefix}-supabase" \
    --requires "${prefix}-supabase-db" --network "${prefix}-supabase-backend:alias=meta" \
    --env PG_META_PORT=8080 --env PG_META_DB_HOST=db --env PG_META_DB_PORT=5432 \
    --env PG_META_DB_NAME=postgres --env PG_META_DB_USER=postgres \
    --env "PG_META_DB_PASSWORD=${SUPABASE_DB_PASSWORD}" \
    --env "CRYPTO_KEY=${SUPABASE_META_CRYPTO_KEY}" \
    "$(supabase_image_reference meta)" > /dev/null

  supabase_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-supabase-supavisor" \
    --label "io.boxferry.live-run=${run}" --label "io.boxferry.application=${prefix}-supabase" \
    --requires "${prefix}-supabase-db" --network "${prefix}-supabase-backend:alias=supavisor" \
    --env "DATABASE_URL=ecto://supabase_admin:${SUPABASE_DB_PASSWORD}@db:5432/_supabase" \
    --env CLUSTER_POSTGRES=true --env PORT=4000 --env REGION=local \
    --env "SECRET_KEY_BASE=${SUPABASE_POOLER_SECRET}" \
    --env "VAULT_ENC_KEY=${SUPABASE_VAULT_ENC_KEY}" \
    --env "API_JWT_SECRET=${SUPABASE_JWT_SECRET}" \
    --env "METRICS_JWT_SECRET=${SUPABASE_JWT_SECRET}" \
    --env POSTGRES_HOST=db --env POSTGRES_PORT=5432 --env POSTGRES_DB=postgres \
    --env "POSTGRES_PASSWORD=${SUPABASE_DB_PASSWORD}" --env POOLER_TENANT_ID=boxferry \
    --env POOLER_DEFAULT_POOL_SIZE=5 --env POOLER_MAX_CLIENT_CONN=25 \
    --env POOLER_POOL_MODE=transaction --env 'ERL_AFLAGS=-proto_dist inet_tcp' \
    "$(supabase_image_reference supavisor)" /bin/sh -c '/app/bin/migrate && /app/bin/server' \
    > /dev/null

  supabase_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-supabase-functions" \
    --label "io.boxferry.live-run=${run}" --label "io.boxferry.application=${prefix}-supabase" \
    --requires "${prefix}-supabase-db" \
    --network "${prefix}-supabase-backend:alias=functions" \
    --volume "${fixture_root}/functions:/home/deno/functions:ro" \
    --volume "${prefix}-supabase-deno-cache:/root/.cache/deno" \
    --env "JWT_SECRET=${SUPABASE_JWT_SECRET}" --env "SUPABASE_ANON_KEY=${SUPABASE_ANON_KEY}" \
    --env "SUPABASE_SERVICE_ROLE_KEY=${SUPABASE_SERVICE_KEY}" \
    --env "SUPABASE_DB_URL=postgresql://postgres:${SUPABASE_DB_PASSWORD}@db:5432/postgres" \
    --env SUPABASE_URL=http://kong:8000 --env VERIFY_JWT=false \
    --health-cmd "timeout 1 bash -c '</dev/tcp/127.0.0.1/9000'" \
    --health-interval 3s --health-retries 150 \
    "$(supabase_image_reference functions)" start --main-service /home/deno/functions/main \
    > /dev/null

  supabase_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-supabase-studio" \
    --label "io.boxferry.live-run=${run}" --label "io.boxferry.application=${prefix}-supabase" \
    --requires "${prefix}-supabase-meta" --network "${prefix}-supabase-backend:alias=studio" \
    --volume "${fixture_root}:/boxferry-fixture:ro" --env HOSTNAME=0.0.0.0 \
    --env STUDIO_PG_META_URL=http://meta:8080 --env POSTGRES_USER_READ_WRITE=postgres \
    --env "POSTGRES_PASSWORD=${SUPABASE_DB_PASSWORD}" \
    --env "PG_META_CRYPTO_KEY=${SUPABASE_META_CRYPTO_KEY}" \
    --env DEFAULT_ORGANIZATION_NAME=BoxFerry --env 'DEFAULT_PROJECT_NAME=Supabase acceptance' \
    --env SUPABASE_URL=http://kong:8000 --env SUPABASE_PUBLIC_URL=http://127.0.0.1:18000 \
    --env "SUPABASE_ANON_KEY=${SUPABASE_ANON_KEY}" \
    --env "SUPABASE_SERVICE_KEY=${SUPABASE_SERVICE_KEY}" \
    --env "AUTH_JWT_SECRET=${SUPABASE_JWT_SECRET}" --env ENABLED_FEATURES_LOGS_ALL=false \
    "$(supabase_image_reference studio)" > /dev/null

  supabase_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-supabase-kong" \
    --label "io.boxferry.live-run=${run}" --label "io.boxferry.application=${prefix}-supabase" \
    --requires "${prefix}-supabase-auth,${prefix}-supabase-functions,${prefix}-supabase-realtime,${prefix}-supabase-rest,${prefix}-supabase-storage,${prefix}-supabase-studio" \
    --network "${prefix}-supabase-backend:alias=kong" \
    --network "${prefix}-supabase-edge:alias=supabase" \
    --volume "${fixture_root}/kong.yml:/etc/kong/kong.yml:ro" \
    --publish "127.0.0.1:${SUPABASE_HTTP_PORT}:8000" \
    --env KONG_DATABASE=off --env KONG_DECLARATIVE_CONFIG=/etc/kong/kong.yml \
    --env KONG_DNS_ORDER=LAST,A,CNAME --env KONG_PLUGINS=bundled \
    --env KONG_STATUS_LISTEN=0.0.0.0:8100 \
    --health-cmd 'kong health' --health-interval 3s --health-retries 150 \
    "$(supabase_image_reference kong)" > /dev/null
}

supabase_create_cli_peer() {
  local socket=$1 prefix=$2 run=$3
  supabase_remote "${socket}" run --pull=never --detach \
    --name "${prefix}-supabase-boundary-peer" \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-boundary-peer" \
    --network "${prefix}-supabase-edge:alias=boundary-peer" \
    --entrypoint /bin/sh "$(supabase_image_reference db)" -ceu 'sleep 3600' > /dev/null
}

supabase_provision_cli() {
  local socket=$1 prefix=$2 run=$3
  supabase_assert_clean_prefix "${socket}" "${prefix}"
  supabase_create_networks_volumes "${socket}" "${prefix}" "${run}"
  supabase_create_cli_services "${socket}" "${prefix}" "${run}"
  supabase_create_cli_peer "${socket}" "${prefix}" "${run}"
}

supabase_compose_environment() {
  local prefix=$1 run=$2 fixture_root
  shift 2
  fixture_root="/tmp/boxferry-fixture/${prefix}"
  env \
    BF_PREFIX="${prefix}" BF_RUN="${run}" BF_FIXTURE_ROOT="${fixture_root}" \
    BF_DB_PASSWORD="${SUPABASE_DB_PASSWORD}" BF_JWT_SECRET="${SUPABASE_JWT_SECRET}" \
    BF_ANON_KEY="${SUPABASE_ANON_KEY}" BF_SERVICE_KEY="${SUPABASE_SERVICE_KEY}" \
    BF_REALTIME_SECRET="${SUPABASE_REALTIME_SECRET}" \
    BF_POOLER_SECRET="${SUPABASE_POOLER_SECRET}" \
    BF_STUDIO_IMAGE="$(supabase_image_reference studio)" \
    BF_KONG_IMAGE="$(supabase_image_reference kong)" \
    BF_AUTH_IMAGE="$(supabase_image_reference auth)" \
    BF_REST_IMAGE="$(supabase_image_reference rest)" \
    BF_REALTIME_IMAGE="$(supabase_image_reference realtime)" \
    BF_STORAGE_IMAGE="$(supabase_image_reference storage)" \
    BF_IMGPROXY_IMAGE="$(supabase_image_reference imgproxy)" \
    BF_META_IMAGE="$(supabase_image_reference meta)" \
    BF_FUNCTIONS_IMAGE="$(supabase_image_reference functions)" \
    BF_DB_IMAGE="$(supabase_image_reference db)" \
    BF_SUPAVISOR_IMAGE="$(supabase_image_reference supavisor)" \
    "$@"
}

supabase_compose_project() {
  local socket=$1 prefix=$2 run=$3
  shift 3
  local provider=${BOXFERRY_COMPOSE_BIN:-${repository_root}/target/tools/docker-compose}
  timed_operation 15m "Docker Compose Supabase ${1:-command}" \
    supabase_compose_environment "${prefix}" "${run}" \
    env DOCKER_HOST="unix://${socket}" \
    "${provider}" --project-name "${prefix}-supabase" \
    --file "$(supabase_fixture_root)/compose.yaml" "$@"
}

supabase_peer_compose_project() {
  local socket=$1 prefix=$2 run=$3
  shift 3
  local provider=${BOXFERRY_COMPOSE_BIN:-${repository_root}/target/tools/docker-compose}
  timed_operation 3m "Docker Compose Supabase boundary peer ${1:-command}" \
    supabase_compose_environment "${prefix}" "${run}" \
    env DOCKER_HOST="unix://${socket}" \
    "${provider}" --project-name "${prefix}-supabase-peer" \
    --file "$(supabase_fixture_root)/peer.compose.yaml" "$@"
}

supabase_provision_compose() {
  local socket=$1 prefix=$2 run=$3
  supabase_assert_clean_prefix "${socket}" "${prefix}"
  supabase_remote "${socket}" network create \
    --label "io.boxferry.live-run=${run}" \
    --label "io.boxferry.application=${prefix}-supabase" \
    "${prefix}-supabase-edge" > /dev/null
  supabase_compose_project "${socket}" "${prefix}" "${run}" \
    up --detach --remove-orphans > "${current_case}/supabase-compose.log" 2>&1
  supabase_peer_compose_project "${socket}" "${prefix}" "${run}" \
    up --detach --remove-orphans > "${current_case}/supabase-peer-compose.log" 2>&1
}

supabase_wait_healthy() {
  local socket=$1 container=$2
  [[ "$(supabase_remote "${socket}" inspect --format '{{.State.Health.Status}}' "${container}")" == healthy ]]
}

supabase_wait_running() {
  local socket=$1 container=$2
  [[ "$(supabase_remote "${socket}" inspect --format '{{.State.Status}}' "${container}")" == running ]]
}

supabase_wait_application() {
  local socket=$1 prefix=$2 service
  for service in db auth rest realtime imgproxy storage functions studio kong; do
    supabase_wait_for 600 "Supabase ${service} health" \
      supabase_wait_healthy "${socket}" "${prefix}-supabase-${service}"
  done
  for service in meta supavisor; do
    supabase_wait_for 300 "Supabase ${service} process" \
      supabase_wait_running "${socket}" "${prefix}-supabase-${service}"
  done
  supabase_wait_for 300 'Supabase Meta HTTP readiness' \
    supabase_remote "${socket}" exec "${prefix}-supabase-studio" node -e \
    "fetch('http://meta:8080/health').then(r=>{if(!r.ok)process.exit(1)})"
  supabase_wait_for 300 'Supavisor HTTP readiness' \
    supabase_remote "${socket}" exec "${prefix}-supabase-studio" node -e \
    "fetch('http://supavisor:4000/api/health').then(r=>{if(!r.ok)process.exit(1)})"
}

supabase_enable_realtime_table() {
  local socket=$1 prefix=$2
  supabase_remote "${socket}" exec "${prefix}-supabase-db" \
    psql -v ON_ERROR_STOP=1 -U postgres -d postgres -c '
      DO $$
      BEGIN
        IF NOT EXISTS (
          SELECT FROM pg_publication_tables
          WHERE pubname = '\''supabase_realtime'\''
            AND schemaname = '\''public'\''
            AND tablename = '\''boxferry_items'\''
        ) THEN
          ALTER PUBLICATION supabase_realtime ADD TABLE public.boxferry_items;
        END IF;
      END
      $$;
    ' > /dev/null
}

supabase_probe() {
  local socket=$1 prefix=$2 phase=$3
  supabase_remote "${socket}" exec \
    --env "BF_ANON_KEY=${SUPABASE_ANON_KEY}" \
    --env "BF_SERVICE_KEY=${SUPABASE_SERVICE_KEY}" \
    --env "BF_TEST_EMAIL=${SUPABASE_TEST_EMAIL}" \
    --env "BF_TEST_PASSWORD=${SUPABASE_TEST_PASSWORD}" \
    --env BF_SUPABASE_URL=http://kong:8000 \
    --env BF_REALTIME_URL=http://realtime:4000 \
    --env BF_SUPAVISOR_URL=http://supavisor:4000 \
    "${prefix}-supabase-studio" node \
    /boxferry-fixture/application-probe.mjs "${phase}"
}

supabase_probe_published_api() {
  local outer=$1
  engine_operation 'probe Supabase gateway through loopback publication' \
    exec "${outer}" curl --fail --silent --show-error \
    "http://127.0.0.1:${SUPABASE_HTTP_PORT}/auth/v1/health" > /dev/null
}

supabase_assert_database_state() {
  local socket=$1 prefix=$2 expected_items=$3 observed
  observed="$(supabase_remote "${socket}" exec "${prefix}-supabase-db" \
    psql -v ON_ERROR_STOP=1 -U postgres -d postgres -Atqc \
    "SELECT (SELECT count(*) FROM public.boxferry_items) || ':' ||
            (SELECT count(*) FROM auth.users) || ':' ||
            (SELECT count(*) FROM storage.objects);")"
  [[ "${observed}" == "${expected_items}:1:1" ]] || {
    printf 'Supabase database/API state mismatch: expected %s:1:1, observed %s.\n' \
      "${expected_items}" "${observed}" >&2
    return 1
  }
  supabase_remote "${socket}" exec "${prefix}-supabase-db" \
    psql -v ON_ERROR_STOP=1 -U postgres -d postgres -Atqc \
    "SELECT extname FROM pg_extension WHERE extname IN ('pgcrypto','pg_stat_statements') ORDER BY extname;" |
    diff --unified - <(printf 'pg_stat_statements\npgcrypto\n')
  supabase_remote "${socket}" exec "${prefix}-supabase-db" \
    psql -v ON_ERROR_STOP=1 -U postgres -d postgres -Atqc \
    "SELECT count(*) FROM pg_publication_tables WHERE pubname='supabase_realtime' AND schemaname='public' AND tablename='boxferry_items';" |
    grep --fixed-strings --line-regexp --quiet 1
}

supabase_assert_application_boundaries() {
  local provisioner=$1 socket=$2 prefix=$3 service inspect_file
  for service in db auth rest realtime imgproxy storage meta supavisor functions studio; do
    inspect_file="${current_case}/${prefix}-${service}.inspect.json"
    supabase_remote "${socket}" inspect "${prefix}-supabase-${service}" > "${inspect_file}"
    jq --exit-status --arg backend "${prefix}-supabase-backend" '
      .[0].HostConfig.PortBindings == {} and
      ((.[0].NetworkSettings.Ports // {}) | all(.[]; . == null or . == [])) and
      (.[0].NetworkSettings.Networks | keys == [$backend])
    ' "${inspect_file}" > /dev/null
  done
  inspect_file="${current_case}/${prefix}-kong.inspect.json"
  supabase_remote "${socket}" inspect "${prefix}-supabase-kong" > "${inspect_file}"
  jq --exit-status --arg backend "${prefix}-supabase-backend" \
    --arg edge "${prefix}-supabase-edge" --arg port "${SUPABASE_HTTP_PORT}" '
    (.[0].NetworkSettings.Networks |
      length == 2 and has($backend) and has($edge)) and
      .[0].HostConfig.PortBindings["8000/tcp"][0].HostIp == "127.0.0.1" and
      .[0].HostConfig.PortBindings["8000/tcp"][0].HostPort == $port
    ' "${inspect_file}" > /dev/null
  supabase_remote "${socket}" network inspect "${prefix}-supabase-backend" |
    jq --exit-status '.[0].internal == true or .[0].Internal == true' > /dev/null
  supabase_remote "${socket}" inspect "${prefix}-supabase-boundary-peer" |
    jq --exit-status --arg edge "${prefix}-supabase-edge" \
      --arg app "${prefix}-boundary-peer" '
        (.[0].NetworkSettings.Networks | has($edge)) and
        (.[0].NetworkSettings.Networks | length) == 1 and
        .[0].Config.Labels["io.boxferry.application"] == $app
      ' > /dev/null
  supabase_assert_dependency_graph "${provisioner}" "${socket}" "${prefix}"
}

supabase_assert_dependency_graph() {
  local provisioner=$1 socket=$2 prefix=$3
  local service _image _networks dependencies _mounts _proof
  local dependency expected_json
  local -a dependency_names
  declare -A container_ids=()

  if [[ "${provisioner}" == cli ]]; then
    while IFS=$'\t' read -r service _image _networks dependencies _mounts _proof; do
      [[ -z "${service}" || "${service}" == \#* ]] && continue
      container_ids["${service}"]="$(
        supabase_remote "${socket}" inspect --format '{{.Id}}' \
          "${prefix}-supabase-${service}"
      )"
      [[ -n "${container_ids[${service}]}" ]] || {
        printf 'Supabase %s container lacks an inspect ID.\n' "${service}" >&2
        return 1
      }
    done < "$(supabase_fixture_root)/graph.tsv"
  fi

  while IFS=$'\t' read -r service _image _networks dependencies _mounts _proof; do
    [[ -z "${service}" || "${service}" == \#* ]] && continue
    case "${provisioner}" in
      cli)
        expected_json='[]'
        if [[ "${dependencies}" != - ]]; then
          IFS=',' read -r -a dependency_names <<< "${dependencies}"
          for dependency in "${dependency_names[@]}"; do
            expected_json="$(
              jq --null-input --compact-output \
                --argjson current "${expected_json}" \
                --arg id "${container_ids[${dependency}]}" \
                '$current + [$id]'
            )"
          done
        fi
        supabase_remote "${socket}" inspect "${prefix}-supabase-${service}" |
          jq --exit-status --argjson expected "${expected_json}" '
            ((.[0].Dependencies // []) | sort) == ($expected | sort)
          ' > /dev/null
        ;;
      compose)
        supabase_remote "${socket}" inspect "${prefix}-supabase-${service}" |
          jq --exit-status --arg expected "${dependencies}" '
            ((.[0].Config.Labels["com.docker.compose.depends_on"] // "") |
              if $expected == "-" then . == ""
              else ([split(",")[] | split(":")[0]] | sort) == ($expected | split(",") | sort)
              end)
          ' > /dev/null
        ;;
      *)
        printf 'Unknown Supabase provisioner %s.\n' "${provisioner}" >&2
        return 1
        ;;
    esac
  done < "$(supabase_fixture_root)/graph.tsv"
}

supabase_assert_storage_ownership() {
  local socket=$1 prefix=$2
  supabase_remote "${socket}" inspect "${prefix}-supabase-db" |
    jq --exit-status --arg name "${prefix}-supabase-pgdata" '
      (.[0].Mounts | length) == 2 and
      any(.[0].Mounts[]?; .Type == "volume" and .Name == $name and
        .Destination == "/var/lib/postgresql/data" and .RW == true) and
      any(.[0].Mounts[]?; .Type == "bind" and
        .Destination == "/docker-entrypoint-initdb.d/80-boxferry.sql" and .RW == false)
    ' > /dev/null
  supabase_remote "${socket}" inspect "${prefix}-supabase-storage" |
    jq --exit-status --arg name "${prefix}-supabase-storage" '
      (.[0].Mounts | length) == 1 and
      any(.[0].Mounts[]?; .Type == "volume" and .Name == $name and
        .Destination == "/var/lib/storage" and .RW == true)
    ' > /dev/null
  supabase_remote "${socket}" inspect "${prefix}-supabase-imgproxy" |
    jq --exit-status --arg name "${prefix}-supabase-storage" '
      (.[0].Mounts | length) == 1 and
      any(.[0].Mounts[]?; .Type == "volume" and .Name == $name and
        .Destination == "/var/lib/storage" and .RW == false)
    ' > /dev/null
  supabase_remote "${socket}" inspect "${prefix}-supabase-functions" |
    jq --exit-status --arg name "${prefix}-supabase-deno-cache" '
      (.[0].Mounts | length) == 2 and
      any(.[0].Mounts[]?; .Type == "volume" and .Name == $name and
        .Destination == "/root/.cache/deno" and .RW == true) and
      any(.[0].Mounts[]?; .Type == "bind" and
        .Destination == "/home/deno/functions" and .RW == false)
    ' > /dev/null
  supabase_remote "${socket}" inspect "${prefix}-supabase-studio" |
    jq --exit-status '
      (.[0].Mounts | length) == 1 and
      any(.[0].Mounts[]?; .Type == "bind" and
        .Destination == "/boxferry-fixture" and .RW == false)
    ' > /dev/null
  supabase_remote "${socket}" inspect "${prefix}-supabase-kong" |
    jq --exit-status '
      (.[0].Mounts | length) == 1 and
      any(.[0].Mounts[]?; .Type == "bind" and
        .Destination == "/etc/kong/kong.yml" and .RW == false)
    ' > /dev/null
  for service in auth rest realtime meta supavisor; do
    supabase_remote "${socket}" inspect "${prefix}-supabase-${service}" |
      jq --exit-status '.[0].Mounts == []' > /dev/null
  done
  supabase_remote "${socket}" exec "${prefix}-supabase-functions" \
    test -w /root/.cache/deno
  supabase_remote "${socket}" exec "${prefix}-supabase-db" /bin/sh -ceu '
    test -r /var/lib/postgresql/data && test -w /var/lib/postgresql/data
    case "$(stat -c %A /var/lib/postgresql/data)" in ????????w?) exit 1;; esac
  '
}

supabase_expect_collision() {
  local socket=$1 prefix=$2 status=0
  supabase_remote "${socket}" run --pull=never --name "${prefix}-supabase-db" \
    "$(supabase_image_reference db)" true > /dev/null 2>&1 || status=$?
  [[ "${status}" == 125 ]] || {
    printf 'Supabase collision expected exit 125; observed %s.\n' "${status}" >&2
    return 1
  }
}

supabase_output_kind_count() {
  local output=$1 directory=$2 kind=$3 section
  case "${output}" in
    compose)
      case "${kind}" in
        container) section=services ;;
        network) section=networks ;;
        volume) section=volumes ;;
      esac
      awk -v section="${section}" '
        $0 == section ":" { inside = 1; next }
        inside && /^[^ ]/ { inside = 0 }
        inside && /^  [^ ][^:]*:([ ]*\{\})?[ ]*$/ { count++ }
        END { print count + 0 }
      ' "${directory}/compose.yaml"
      ;;
    quadlet)
      case "${kind}" in
        container)
          find "${directory}" -maxdepth 1 -type f -name '*.container' -printf '.\n' |
            awk 'END { print NR + 0 }'
          ;;
        network)
          {
            find "${directory}" -maxdepth 1 -type f -name '*.network' -printf '%f\n' |
              sed 's/[.]network$//'
            find "${directory}" -maxdepth 1 -type f -name '*.container' \
              -exec awk -F= '
                /^Network=/ {
                  name = substr($0, length($1) + 2)
                  sub(/[.]network$/, "", name)
                  print name
                }
              ' {} +
          } | awk 'NF { seen[$0] = 1 } END { print length(seen) + 0 }'
          ;;
        volume)
          {
            find "${directory}" -maxdepth 1 -type f -name '*.volume' -printf '%f\n' |
              sed 's/[.]volume$//'
            find "${directory}" -maxdepth 1 -type f -name '*.container' \
              -exec awk -F= '
                /^Volume=/ {
                  value = substr($0, length($1) + 2)
                  split(value, fields, ":")
                  name = fields[1]
                  if (name !~ /^\// && name !~ /^[.]/) {
                    sub(/[.]volume$/, "", name)
                    print name
                  }
                }
              ' {} +
          } | awk 'NF { seen[$0] = 1 } END { print length(seen) + 0 }'
          ;;
      esac
      ;;
    podman)
      jq --raw-output --arg kind "${kind}" '
        [.external_preconditions[]?, .operations[]?.resource] |
        [.[] | select(.kind == $kind) | .name] |
        unique |
        length
      ' "${directory}/podman.json"
      ;;
  esac
}

supabase_assert_exact_output_topology() {
  local selection=$1 output=$2 directory=$3 prefix=$4
  local expected_services expected_networks expected_volumes kind expected actual
  case "${selection}" in
    exact)
      expected_services=10
      expected_networks=2
      expected_volumes=3
      ;;
    storage)
      expected_services=4
      expected_networks=1
      expected_volumes=2
      ;;
    label)
      expected_services=11
      expected_networks=2
      expected_volumes=3
      ;;
    all)
      expected_services=12
      expected_networks=2
      expected_volumes=3
      ;;
    *)
      printf 'Unknown Supabase selection %s.\n' "${selection}" >&2
      return 1
      ;;
  esac

  assert_resource_member "${output}" "${directory}" network \
    "${prefix}-supabase-backend"
  assert_resource_member "${output}" "${directory}" volume \
    "${prefix}-supabase-pgdata"
  assert_resource_member "${output}" "${directory}" volume \
    "${prefix}-supabase-storage"
  if [[ "${selection}" == storage ]]; then
    assert_resource_absent "${output}" "${directory}" network \
      "${prefix}-supabase-edge"
    assert_resource_absent "${output}" "${directory}" volume \
      "${prefix}-supabase-deno-cache"
  else
    assert_resource_member "${output}" "${directory}" network \
      "${prefix}-supabase-edge"
    assert_resource_member "${output}" "${directory}" volume \
      "${prefix}-supabase-deno-cache"
  fi

  for kind in container network volume; do
    case "${kind}" in
      container) expected=${expected_services} ;;
      network) expected=${expected_networks} ;;
      volume) expected=${expected_volumes} ;;
    esac
    actual="$(supabase_output_kind_count "${output}" "${directory}" "${kind}")"
    [[ "${actual}" == "${expected}" ]] || {
      printf 'Supabase %s %s output has unexpected %s topology: expected=%s actual=%s.\n' \
        "${selection}" "${output}" "${kind}" "${expected}" "${actual}" >&2
      return 1
    }
  done
}

supabase_selection_includes_service() {
  local selection=$1 service=$2
  case "${selection}" in
    exact)
      [[ " auth db functions imgproxy kong meta realtime rest storage studio " == *" ${service} "* ]]
      ;;
    storage)
      [[ " db imgproxy rest storage " == *" ${service} "* ]]
      ;;
    label)
      [[ " auth db functions imgproxy kong meta realtime rest storage studio supavisor " == *" ${service} "* ]]
      ;;
    all)
      [[ " auth boundary-peer db functions imgproxy kong meta realtime rest storage studio supavisor " == *" ${service} "* ]]
      ;;
    *) return 1 ;;
  esac
}

supabase_expected_output_projection() {
  local selection=$1 origin=$2 output=$3
  local service image_id networks dependencies mounts _proof image network mount
  local service_dependency
  local -a values

  while IFS=$'\t' read -r service image_id networks dependencies mounts _proof; do
    [[ -z "${service}" || "${service}" == \#* ]] && continue
    supabase_selection_includes_service "${selection}" "${service}" || continue
    image="$(supabase_image_reference "${image_id}")"
    printf '%s\timage\t%s\n' "${service}" "${image}"

    IFS=',' read -r -a values <<< "${networks}"
    for network in "${values[@]}"; do
      printf '%s\tnetwork\t%s\n' "${service}" "${network}"
    done

    if [[ "${mounts}" != - ]]; then
      IFS=',' read -r -a values <<< "${mounts}"
      for mount in "${values[@]}"; do
        [[ "${mount}" == bind:* ]] && continue
        printf '%s\tmount\tvolume:%s\n' "${service}" "${mount}"
      done
    fi

    if [[ "${output}" == quadlet && "${origin}" != compose &&
      "${dependencies}" != - ]]; then
      IFS=',' read -r -a values <<< "${dependencies}"
      for service_dependency in "${values[@]}"; do
        printf '%s\trequires\t%s\n' "${service}" "${service_dependency}"
        printf '%s\tafter\t%s\n' "${service}" "${service_dependency}"
      done
    fi
  done < "$(supabase_fixture_root)/graph.tsv"

  if [[ "${selection}" == all ]]; then
    printf 'boundary-peer\timage\t%s\n' "$(supabase_image_reference db)"
    printf 'boundary-peer\tnetwork\tedge\n'
  fi
}

supabase_compose_output_projection() {
  local directory=$1 prefix=$2
  awk -v prefix="${prefix}-supabase-" '
    function normalize(value) {
      if (index(value, prefix) == 1) value = substr(value, length(prefix) + 1)
      return value
    }
    function unquote(value) {
      if (value ~ /^".*"$/ || value ~ /^\047.*\047$/) {
        value = substr(value, 2, length(value) - 2)
      }
      return value
    }
    function flush_mount(    mode, name) {
      if (!mount_started) return
      mode = (mount_read_only == "true" ? "ro" : "rw")
      name = unquote(mount_source)
      if (mount_type == "volume") name = normalize(name)
      print service "\tmount\t" mount_type ":" name ":" unquote(mount_target) ":" mode
      mount_started = 0
      mount_type = ""
      mount_source = ""
      mount_target = ""
      mount_read_only = ""
    }
    function flush_service() {
      flush_mount()
      service = ""
      subsection = ""
    }
    $0 == "services:" { in_services = 1; next }
    in_services && /^[^ ]/ { flush_service(); in_services = 0 }
    !in_services { next }
    /^  [^ ][^:]*:[ ]*$/ {
      flush_service()
      service = $0
      sub(/^  /, "", service)
      sub(/:[ ]*$/, "", service)
      service = normalize(unquote(service))
      next
    }
    service == "" { next }
    /^    image: / {
      image = substr($0, length("    image: ") + 1)
      print service "\timage\t" unquote(image)
      next
    }
    /^    networks:[ ]*$/ { flush_mount(); subsection = "networks"; next }
    /^    volumes:[ ]*$/ { flush_mount(); subsection = "volumes"; next }
    /^    tmpfs:/ {
      flush_mount()
      subsection = "tmpfs"
      print service "\tmount\ttmpfs:declared"
      next
    }
    /^    depends_on:[ ]*$/ {
      flush_mount()
      subsection = "depends_on"
      print service "\tdependency\tdeclared"
      next
    }
    /^    [^ ]/ { flush_mount(); subsection = ""; next }
    subsection == "networks" && /^      [^ ][^:]*:/ {
      value = $0
      sub(/^      /, "", value)
      sub(/:.*/, "", value)
      print service "\tnetwork\t" normalize(unquote(value))
      next
    }
    subsection == "networks" && /^      - / {
      value = substr($0, length("      - ") + 1)
      print service "\tnetwork\t" normalize(unquote(value))
      next
    }
    subsection == "volumes" && /^      - type: / {
      flush_mount()
      mount_started = 1
      mount_type = substr($0, length("      - type: ") + 1)
      next
    }
    subsection == "volumes" && /^      - / {
      flush_mount()
      value = unquote(substr($0, length("      - ") + 1))
      count = split(value, fields, ":")
      name = fields[1]
      target = fields[2]
      mode = "rw"
      for (field_index = 3; field_index <= count; field_index++) {
        if (fields[field_index] ~ /(^|,)ro(,|$)/) mode = "ro"
      }
      type = (name ~ /^\// || name ~ /^[.]/ ? "bind" : "volume")
      if (type == "volume") name = normalize(name)
      print service "\tmount\t" type ":" name ":" target ":" mode
      next
    }
    subsection == "volumes" && /^        source: / {
      mount_source = substr($0, length("        source: ") + 1)
      next
    }
    subsection == "volumes" && /^        target: / {
      mount_target = substr($0, length("        target: ") + 1)
      next
    }
    subsection == "volumes" && /^        read_only: / {
      mount_read_only = substr($0, length("        read_only: ") + 1)
      next
    }
    END { flush_service() }
  ' "${directory}/compose.yaml"
}

supabase_quadlet_output_projection() {
  local directory=$1 prefix=$2
  find "${directory}" -maxdepth 1 -type f -name '*.container' -exec \
    awk -F= -v prefix="${prefix}-supabase-" '
      function normalize(value) {
        if (index(value, prefix) == 1) value = substr(value, length(prefix) + 1)
        return value
      }
      function emit_units(kind, value,    count, units, unit_index, name) {
        count = split(value, units, /[[:space:]]+/)
        for (unit_index = 1; unit_index <= count; unit_index++) {
          name = units[unit_index]
          sub(/[.]service$/, "", name)
          if (name != "") print service "\t" kind "\t" normalize(name)
        }
      }
      function trim(value) {
        sub(/^[[:space:]]+/, "", value)
        sub(/[[:space:]]+$/, "", value)
        return value
      }
      function emit_mount(type, name, target, mode) {
        if (mode == "") mode = "rw"
        if (type == "volume") {
          sub(/[.]volume$/, "", name)
          name = normalize(name)
        }
        print service "\tmount\t" type ":" name ":" target ":" mode
      }
      FNR == 1 {
        service = FILENAME
        sub(/^.*\//, "", service)
        sub(/[.]container$/, "", service)
        service = normalize(service)
      }
      $1 == "Image" { print service "\timage\t" substr($0, length($1) + 2) }
      $1 == "Network" {
        name = substr($0, length($1) + 2)
        sub(/[.]network$/, "", name)
        print service "\tnetwork\t" normalize(name)
      }
      $1 == "Volume" {
        value = substr($0, length($1) + 2)
        count = split(value, fields, ":")
        name = fields[1]
        target = fields[2]
        mode = "rw"
        for (field_index = 3; field_index <= count; field_index++) {
          if (fields[field_index] ~ /(^|,)ro(,|$)/) mode = "ro"
        }
        type = (name ~ /^\// || name ~ /^[.]/ ? "bind" : "volume")
        emit_mount(type, name, target, mode)
      }
      $1 == "Tmpfs" {
        value = substr($0, length($1) + 2)
        split(value, fields, ":")
        emit_mount("tmpfs", "tmpfs", fields[1], "rw")
      }
      $1 == "Mount" {
        value = substr($0, length($1) + 2)
        count = split(value, fields, ",")
        type = "unknown"
        name = ""
        target = ""
        mode = "rw"
        for (field_index = 1; field_index <= count; field_index++) {
          field = trim(fields[field_index])
          separator = index(field, "=")
          key = (separator == 0 ? field : substr(field, 1, separator - 1))
          field_value = (separator == 0 ? "" : substr(field, separator + 1))
          if (key == "type") type = field_value
          if (key == "source" || key == "src") name = field_value
          if (key == "destination" || key == "dst" || key == "target") target = field_value
          if (key == "ro" || key == "readonly" || key == "read-only") {
            if (field_value == "" || field_value == "true") mode = "ro"
          }
        }
        if (type == "tmpfs" && name == "") name = "tmpfs"
        emit_mount(type, name, target, mode)
      }
      $1 == "Requires" { emit_units("requires", substr($0, length($1) + 2)) }
      $1 == "After" { emit_units("after", substr($0, length($1) + 2)) }
    ' {} +
}

supabase_podman_output_projection() {
  local directory=$1 prefix=$2
  jq --raw-output --arg prefix "${prefix}-supabase-" '
    def normalize:
      if startswith($prefix) then ltrimstr($prefix) else . end;
    .operations[] |
    select(.resource.kind == "container" and .action == "create") |
    (.resource.name | normalize) as $service |
    .libpod.body.json as $body |
    ($service + "\timage\t" + $body.image),
    ($body.Networks | keys[] | $service + "\tnetwork\t" + normalize),
    ($body.volumes[]? |
      ((if (.Options // [] | index("ro")) == null then "rw" else "ro" end) as $mode |
       $service + "\tmount\tvolume:" + (.Name | normalize) + ":" + .Dest + ":" + $mode)),
    ($body.mounts[]? |
      (.type // "unknown") as $type |
      ((.options // []) |
        if index("ro") != null or index("readonly") != null then "ro" else "rw" end) as $mode |
      ((.source // (if $type == "tmpfs" then "tmpfs" else "" end)) |
        if $type == "volume" then normalize else . end) as $source |
      $service + "\tmount\t" + $type + ":" + $source + ":" +
        (.destination // "") + ":" + $mode)
  ' "${directory}/podman.json"
}

supabase_assert_podman_dependency_order() {
  local selection=$1 directory=$2 prefix=$3
  local service _image _networks dependencies _mounts _proof dependency
  local -a dependency_names

  while IFS=$'\t' read -r service _image _networks dependencies _mounts _proof; do
    [[ -z "${service}" || "${service}" == \#* || "${dependencies}" == - ]] && continue
    supabase_selection_includes_service "${selection}" "${service}" || continue
    IFS=',' read -r -a dependency_names <<< "${dependencies}"
    for dependency in "${dependency_names[@]}"; do
      jq --exit-status \
        --arg prefix "${prefix}-supabase-" \
        --arg service "${service}" \
        --arg dependency "${dependency}" '
          def normalize:
            if startswith($prefix) then ltrimstr($prefix) else . end;
          [.operations | to_entries[] |
            select(.value.resource.kind == "container" and
              .value.action == "start_container") |
            {name: (.value.resource.name | normalize), index: .key}] as $starts |
          ([$starts[] | select(.name == $dependency) | .index] | first) as $before |
          ([$starts[] | select(.name == $service) | .index] | first) as $after |
          $before != null and $after != null and $before < $after
        ' "${directory}/podman.json" > /dev/null || {
        printf 'Supabase Podman plan does not start %s before dependent %s.\n' \
          "${dependency}" "${service}" >&2
        return 1
      }
    done
  done < "$(supabase_fixture_root)/graph.tsv"
}

supabase_assert_output_graph() {
  local selection=$1 origin=$2 output=$3 directory=$4 prefix=$5
  local actual_projection
  case "${output}" in
    compose) actual_projection=supabase_compose_output_projection ;;
    quadlet) actual_projection=supabase_quadlet_output_projection ;;
    podman) actual_projection=supabase_podman_output_projection ;;
  esac

  diff --unified \
    <(supabase_expected_output_projection "${selection}" "${origin}" "${output}" | LC_ALL=C sort) \
    <("${actual_projection}" "${directory}" "${prefix}" | LC_ALL=C sort) || {
    printf 'Supabase %s-to-%s %s selection escaped its exact graph projection.\n' \
      "${origin}" "${output}" "${selection}" >&2
    return 1
  }

  if [[ "${output}" == podman ]]; then
    supabase_assert_podman_dependency_order \
      "${selection}" "${directory}" "${prefix}"
  fi
}

supabase_assert_output_membership() {
  local selection=$1 output=$2 directory=$3 prefix=$4 service
  case "${selection}" in
    exact)
      for service in auth db functions imgproxy kong meta realtime rest storage studio; do
        assert_named_member "${output}" "${directory}" "${service}" \
          "${prefix}-supabase-${service}"
      done
      assert_named_absent "${output}" "${directory}" supavisor \
        "${prefix}-supabase-supavisor"
      assert_named_absent "${output}" "${directory}" boundary-peer \
        "${prefix}-supabase-boundary-peer"
      ;;
    storage)
      for service in db imgproxy rest storage; do
        assert_named_member "${output}" "${directory}" "${service}" \
          "${prefix}-supabase-${service}"
      done
      for service in auth functions kong meta realtime studio supavisor; do
        assert_named_absent "${output}" "${directory}" "${service}" \
          "${prefix}-supabase-${service}"
      done
      assert_named_absent "${output}" "${directory}" boundary-peer \
        "${prefix}-supabase-boundary-peer"
      assert_resource_member "${output}" "${directory}" network \
        "${prefix}-supabase-backend"
      assert_resource_absent "${output}" "${directory}" network \
        "${prefix}-supabase-edge"
      assert_resource_member "${output}" "${directory}" volume \
        "${prefix}-supabase-pgdata"
      assert_resource_member "${output}" "${directory}" volume \
        "${prefix}-supabase-storage"
      assert_resource_absent "${output}" "${directory}" volume \
        "${prefix}-supabase-deno-cache"
      ;;
    label | all)
      for service in db auth rest realtime imgproxy storage meta supavisor functions studio kong; do
        assert_named_member "${output}" "${directory}" "${service}" \
          "${prefix}-supabase-${service}"
      done
      if [[ "${selection}" == all ]]; then
        assert_named_member "${output}" "${directory}" boundary-peer \
          "${prefix}-supabase-boundary-peer"
      else
        assert_named_absent "${output}" "${directory}" boundary-peer \
          "${prefix}-supabase-boundary-peer"
      fi
      ;;
    *)
      printf 'Unknown Supabase selection %s.\n' "${selection}" >&2
      return 1
      ;;
  esac
  supabase_assert_exact_output_topology \
    "${selection}" "${output}" "${directory}" "${prefix}"
}

supabase_assert_output_semantics() {
  local selection=$1 origin=$2 output=$3 directory=$4 prefix=$5
  supabase_assert_output_graph \
    "${selection}" "${origin}" "${output}" "${directory}" "${prefix}"
  if [[ "${selection}" == exact || "${selection}" == storage ||
    "${selection}" == label || "${selection}" == all ]]; then
    assert_resource_member "${output}" "${directory}" network \
      "${prefix}-supabase-backend"
    assert_resource_member "${output}" "${directory}" volume \
      "${prefix}-supabase-pgdata"
    assert_resource_member "${output}" "${directory}" volume \
      "${prefix}-supabase-storage"
  fi
  if [[ "${selection}" == exact || "${selection}" == label || "${selection}" == all ]]; then
    assert_resource_member "${output}" "${directory}" network \
      "${prefix}-supabase-edge"
    assert_resource_member "${output}" "${directory}" volume \
      "${prefix}-supabase-deno-cache"
  fi
  case "${output}" in
    compose)
      grep --fixed-strings --quiet 'internal: true' "${directory}/compose.yaml"
      if [[ "${selection}" != storage ]]; then
        grep --fixed-strings --quiet 'host_ip: 127.0.0.1' "${directory}/compose.yaml"
        grep --fixed-strings --quiet "published: \"${SUPABASE_HTTP_PORT}\"" \
          "${directory}/compose.yaml"
      fi
      ;;
    quadlet)
      grep --recursive --fixed-strings --quiet 'Internal=true' "${directory}"
      if [[ "${selection}" != storage ]]; then
        grep --recursive --extended-regexp --quiet \
          "^PublishPort=127\\.0\\.0\\.1:${SUPABASE_HTTP_PORT}:8000(/tcp)?$" \
          "${directory}"
      fi
      ;;
    podman)
      jq --exit-status '.schema_version == 1 and (.operations | length > 0)' \
        "${directory}/podman.json" > /dev/null
      ;;
  esac
}

supabase_assert_success_contract() {
  local input=$1 output=$2 selection=$3 report=$4 prefix=$5
  jq --exit-status \
    --arg input "${input}" \
    --arg output "${output}" \
    --arg selection "${selection}" \
    --arg resource_prefix "${prefix}-supabase-" \
    --argjson emit_expected false \
    --from-file "$(supabase_fixture_root)/success-contract.jq" \
    "${report}" > /dev/null || {
    printf 'Supabase route emitted a diagnostic multiset outside its exact contract: %s -> %s (%s, %s).\n' \
      "${input}" "${output}" "${selection}" "${report}" >&2
    return 1
  }
}

supabase_success_contract_example_report() {
  local input=$1 output=$2 selection=$3 prefix=$4
  local expected_fidelity
  expected_fidelity="$(
    jq --null-input \
      --arg input "${input}" \
      --arg output "${output}" \
      --arg selection "${selection}" \
      --arg resource_prefix "${prefix}-supabase-" \
      --argjson emit_expected '"fidelity"' \
      --from-file "$(supabase_fixture_root)/success-contract.jq"
  )"
  jq --null-input \
    --arg input "${input}" \
    --arg output "${output}" \
    --arg selection "${selection}" \
    --arg resource_prefix "${prefix}-supabase-" \
    --argjson emit_expected true \
    --from-file "$(supabase_fixture_root)/success-contract.jq" |
    jq --argjson fidelity "${expected_fidelity}" '
      . as $expected |
      {
        schema_version: 1,
        status: "success",
        fidelity: ($fidelity + {exact: 0}),
        diagnostics: [
          $expected[] |
          {
            code: .code,
            severity: .severity,
            name: "exact Supabase contract example",
            fields: [
              {name: "subject", value: .subject},
              {name: "decision", value: .decision}
            ]
          }
        ]
      }
    '
}

supabase_validate_success_contract_examples() {
  local prefix=contract
  supabase_assert_success_contract podman podman storage \
    <(supabase_success_contract_example_report podman podman storage "${prefix}") \
    "${prefix}"

  if supabase_assert_success_contract podman podman storage \
    <(
      supabase_success_contract_example_report podman podman storage "${prefix}" |
        jq '(
          .diagnostics[] |
          select(.code == "BFP0003") |
          .fields[] |
          select(.name == "subject") |
          .value
        ) = "services.contract-supabase-auth.unseen"'
    ) "${prefix}" > /dev/null 2>&1; then
    printf 'Supabase diagnostic contract admitted an unseen BFP0003 subject.\n' >&2
    return 1
  fi

  if supabase_assert_success_contract podman podman storage \
    <(
      supabase_success_contract_example_report podman podman storage "${prefix}" |
        jq '.diagnostics += [first(.diagnostics[] | select(.code == "BFP0007"))]'
    ) "${prefix}" > /dev/null 2>&1; then
    printf 'Supabase diagnostic contract admitted a duplicate BFP0007 tuple.\n' >&2
    return 1
  fi
}

supabase_report_conversion_failure() {
  local report=$1
  [[ -s "${report}" ]] && sed -n '1,240p' "${report}" >&2
}

supabase_run_exports() {
  local mode=$1 socket=$2 prefix=$3
  local selection output directory report
  local -a selection_arguments target_arguments
  mkdir -p -- "${current_case}/outputs"
  for selection in exact storage label all; do
    case "${selection}" in
      exact)
        selection_arguments=(--podman-resource "container=${prefix}-supabase-kong")
        ;;
      storage)
        selection_arguments=(--podman-label io.boxferry.selection=storage)
        ;;
      label)
        selection_arguments=(--podman-label "io.boxferry.application=${prefix}-supabase")
        ;;
      all)
        selection_arguments=(--podman-all)
        ;;
    esac
    for output in compose quadlet podman; do
      directory="${current_case}/outputs/${mode}-${selection}-${output}"
      report="${directory}.report.json"
      target_arguments=()
      [[ "${output}" == podman ]] &&
        target_arguments+=(--podman-target-context rootless)
      if ! boxferry_operation "Supabase ${mode} ${selection} Podman-to-${output}" \
        convert podman "${output}" --podman-socket "${socket}" \
        --application-name "${prefix}-supabase" --loss-policy partial \
        --promote-podman-effective-named-volumes \
        --promote-podman-effective-named-networks \
        --promote-podman-portable-effective-settings \
        --output-directory "${directory}" --console-format json \
        "${target_arguments[@]}" "${selection_arguments[@]}" > "${report}"; then
        supabase_report_conversion_failure "${report}"
        return 1
      fi
      assert_successful_conversion "${output}" "${selection}" "${directory}" "${report}"
      supabase_assert_success_contract podman "${output}" "${selection}" "${report}" "${prefix}"
      supabase_assert_output_membership \
        "${selection}" "${output}" "${directory}" "${prefix}"
      supabase_assert_output_semantics \
        "${selection}" podman "${output}" "${directory}" "${prefix}"
    done
  done
}

supabase_assert_pinned_image_rejection() {
  local input=$1 selection=$2 source=$3 report=$4 status=$5 prefix=$6
  local image_count
  case "${input}" in
    compose)
      image_count="$(awk '/^[[:space:]]+image: .*@sha256:[0-9a-f]{64}$/ { count++ } END { print count + 0 }' \
        "${source}/compose.yaml")"
      ;;
    quadlet)
      image_count="$(find "${source}" -maxdepth 1 -type f -name '*.container' -exec \
        awk '/^Image=.*@sha256:[0-9a-f]{64}$/ { count++ } END { print count + 0 }' {} + |
        awk '{ total += $1 } END { print total + 0 }')"
      ;;
  esac
  [[ "${status}" == 1 && "${image_count}" -gt 0 ]] || {
    printf 'Expected pinned-image migration gap; status=%s images=%s.\n' \
      "${status}" "${image_count}" >&2
    return 1
  }
  jq --exit-status \
    --arg input "${input}" \
    --arg output podman \
    --arg selection "${selection}" \
    --arg resource_prefix "${prefix}-supabase-" \
    --argjson emit_expected false \
    --from-file "$(supabase_fixture_root)/success-contract.jq" \
    "${report}" > /dev/null || {
    printf 'Supabase pinned-image rejection escaped its exact subject contract: %s (%s, %s).\n' \
      "${input}" "${selection}" "${report}" >&2
    return 1
  }
}

supabase_run_reimports() {
  local mode=$1 prefix=$2
  local selection input output source result report
  local -a command
  mkdir -p -- "${current_case}/reimports"
  for selection in exact storage label all; do
    for input in compose quadlet; do
      source="${current_case}/outputs/${mode}-${selection}-${input}"
      for output in compose quadlet podman; do
        result="${current_case}/reimports/${mode}-${selection}-${input}-to-${output}"
        report="${result}.report.json"
        command=("${boxferry_bin}" convert "${input}" "${output}"
          --loss-policy partial)
        if [[ "${input}" == compose ]]; then
          command+=(--input-file "${source}/compose.yaml")
        else
          command+=(--input-directory "${source}"
            --application-name "${prefix}-supabase")
        fi
        [[ "${output}" == podman ]] &&
          command+=(--podman-target-context rootless)
        command+=(--output-directory "${result}" --console-format json)
        if [[ "${output}" == podman ]]; then
          expected_failure_operation 120s \
            "BoxFerry Supabase ${mode} ${selection} ${input}-to-${output} pinned-image rejection" \
            1 "${command[@]}" > "${report}"
          supabase_assert_pinned_image_rejection \
            "${input}" "${selection}" "${source}" "${report}" 1 "${prefix}"
          continue
        fi
        local status=0
        timed_operation 120s \
          "BoxFerry Supabase ${mode} ${selection} ${input}-to-${output} reimport" \
          "${command[@]}" > "${report}" || status=$?
        if ((status != 0)); then
          supabase_report_conversion_failure "${report}"
          return "${status}"
        fi
        assert_successful_conversion "${output}" "${selection}" "${result}" "${report}"
        supabase_assert_success_contract \
          "${input}" "${output}" "${selection}" "${report}" "${prefix}"
        supabase_assert_output_membership \
          "${selection}" "${output}" "${result}" "${prefix}"
        supabase_assert_output_semantics \
          "${selection}" "${input}" "${output}" "${result}" "${prefix}"
      done
    done
  done
}

supabase_assert_report_privacy() {
  local root=$1
  if grep --recursive --include='*.report.json' --fixed-strings --quiet \
    -e "${SUPABASE_DB_PASSWORD}" -e "${SUPABASE_JWT_SECRET}" \
    -e "${SUPABASE_ANON_KEY}" -e "${SUPABASE_SERVICE_KEY}" \
    -e "${SUPABASE_REALTIME_SECRET}" -e "${SUPABASE_POOLER_SECRET}" \
    -e "${SUPABASE_REALTIME_DB_KEY}" -e "${SUPABASE_META_CRYPTO_KEY}" \
    -e "${SUPABASE_VAULT_ENC_KEY}" \
    -e "${SUPABASE_TEST_PASSWORD}" "${root}"; then
    printf 'Supabase BoxFerry report leaked a public protected-value canary.\n' >&2
    return 1
  fi
}

supabase_remove_cli_application_containers() {
  local socket=$1 prefix=$2
  local -a containers
  mapfile -t containers < <(supabase_container_names "${prefix}")
  supabase_remote "${socket}" stop --time 30 "${containers[@]}" > /dev/null
  supabase_remote "${socket}" rm --force "${containers[@]}" > /dev/null
}

supabase_recreate_application() {
  local mode=$1 socket=$2 prefix=$3 run=$4
  if [[ "${mode}" == compose ]]; then
    supabase_compose_project "${socket}" "${prefix}" "${run}" \
      stop --timeout 30 > "${current_case}/supabase-compose-stop.log" 2>&1
    supabase_compose_project "${socket}" "${prefix}" "${run}" \
      rm --force >> "${current_case}/supabase-compose-stop.log" 2>&1
    supabase_compose_project "${socket}" "${prefix}" "${run}" \
      up --detach --remove-orphans > "${current_case}/supabase-compose-recreate.log" 2>&1
  else
    supabase_remove_cli_application_containers "${socket}" "${prefix}"
    supabase_create_cli_services "${socket}" "${prefix}" "${run}"
  fi
  supabase_wait_application "${socket}" "${prefix}"
  supabase_enable_realtime_table "${socket}" "${prefix}"
}

supabase_assert_clean_resources() {
  local socket=$1 prefix=$2
  local found=""
  found="$(supabase_remote "${socket}" ps -a --format '{{.Names}}' |
    awk -v prefix="${prefix}-supabase-" 'index($0, prefix) == 1')"
  found+="$(supabase_remote "${socket}" volume ls --format '{{.Name}}' |
    awk -v prefix="${prefix}-supabase-" 'index($0, prefix) == 1')"
  found+="$(supabase_remote "${socket}" network ls --format '{{.Name}}' |
    awk -v prefix="${prefix}-supabase-" 'index($0, prefix) == 1')"
  [[ -z "${found}" ]] || {
    printf 'Supabase cleanup left prefix-scoped resources:\n%s\n' "${found}" >&2
    return 1
  }
}

supabase_cleanup_mode() {
  local mode=$1 socket=$2 prefix=$3 run=$4
  local -a containers
  if [[ "${mode}" == compose ]]; then
    supabase_peer_compose_project "${socket}" "${prefix}" "${run}" \
      down --volumes --remove-orphans \
      > "${current_case}/supabase-peer-compose-down.log" 2>&1 || true
    supabase_compose_project "${socket}" "${prefix}" "${run}" \
      down --volumes --remove-orphans --timeout 30 \
      > "${current_case}/supabase-compose-down.log" 2>&1 || true
  else
    mapfile -t containers < <(supabase_container_names "${prefix}")
    containers+=("${prefix}-supabase-boundary-peer")
    supabase_remote "${socket}" rm --force --time 0 --ignore \
      "${containers[@]}" > /dev/null 2>&1 || true
    supabase_remote "${socket}" volume rm --force \
      "${prefix}-supabase-pgdata" "${prefix}-supabase-storage" \
      "${prefix}-supabase-deno-cache" > /dev/null 2>&1 || true
    supabase_remote "${socket}" network rm \
      "${prefix}-supabase-backend" > /dev/null 2>&1 || true
  fi
  supabase_remote "${socket}" network rm \
    "${prefix}-supabase-edge" > /dev/null 2>&1 || true
  supabase_assert_clean_resources "${socket}" "${prefix}"
}

supabase_verify_target() {
  local id=$1 image=$2 declared_version=$3 distribution=$4
  local mode=$5 lane=$6 architecture=$7
  verify_observed_version "${id}" "${declared_version}" \
    "${artifact_root}/${id}.podman-version"
  [[ "$(< "${artifact_root}/${id}.digest")" == "${image##*@}" ]]
  [[ "${architecture}" == amd64 &&
    "$(< "${artifact_root}/${id}.architecture")" =~ ^(x86_64|amd64)$ ]]
  append_verified_evidence "${id}" "${image}" "${declared_version}" \
    "${distribution}" "${mode}" "${lane}" "${architecture}" \
    "${artifact_root}/${id}" nested-image supabase-application
}

supabase_run_application_cell_unbounded() {
  local id=$1 image=$2 declared_version=$3 distribution=$4
  local mode=$5 lane=$6 architecture=$7
  [[ "${id}-${mode}" == podman-6.1-rootless-rootless ]] || {
    printf 'Supabase application supports only podman-6.1-rootless rootless.\n' >&2
    return 1
  }
  [[ "${declared_version}" =~ ^6\.1($|\.) ]]
  current_prefix="${run_id:0:27}-supabase"
  current_case="${artifact_root}/${id}-supabase-application"
  socket_directory="${runtime_root}/supabase-application-target"
  application_archive="${runtime_root}/supabase-application-images.tar"
  install -d -m 0700 -- "${current_case}" "${socket_directory}"
  progress_index=0
  progress_total=40
  printf '%s PLAN %s supabase-application tests=%d evidence=live-unperformed-until-run\n' \
    "$(timestamp)" "${id}" "${progress_total}"

  progress_run 'verify Supabase host resource budget' supabase_validate_resource_budget
  progress_run 'validate Supabase provenance catalogues' supabase_validate_catalogues
  progress_run 'validate Docker Compose provider' supabase_validate_provider
  progress_run 'prepare bounded digest-pinned Supabase image archive' \
    supabase_prepare_image_archive "${application_archive}"
  progress_run 'start isolated Supabase Podman target' \
    start_outer_runtime "${id}" "${image}" "${mode}" "${socket_directory}" \
    "${application_archive}"
  progress_run 'verify Podman Supabase target evidence' \
    supabase_verify_target "${id}" "${image}" "${declared_version}" \
    "${distribution}" "${mode}" "${lane}" "${architecture}"
  local outer="${started_outer}"
  progress_run 'load Supabase images and rootless network configuration' \
    supabase_prepare_application_target "${outer}" "${current_prefix}" "${socket_directory}"
  local socket="${socket_directory}/podman.sock"

  progress_run 'provision independent Podman CLI Supabase application' \
    supabase_provision_cli "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'wait for Podman CLI Supabase readiness' \
    supabase_wait_application "${socket}" "${current_prefix}"
  progress_run 'enable Podman CLI Realtime table publication' \
    supabase_enable_realtime_table "${socket}" "${current_prefix}"
  progress_run 'prove Podman CLI gateway loopback publication' \
    supabase_probe_published_api "${outer}"
  progress_run 'exercise Podman CLI auth database storage Realtime edge graph' \
    supabase_probe "${socket}" "${current_prefix}" seed
  progress_run 'prove Podman CLI database auth storage state' \
    supabase_assert_database_state "${socket}" "${current_prefix}" 1
  progress_run 'prove Podman CLI publication and private network boundaries' \
    supabase_assert_application_boundaries cli "${socket}" "${current_prefix}"
  progress_run 'prove Podman CLI shared storage ownership and access' \
    supabase_assert_storage_ownership "${socket}" "${current_prefix}"
  progress_run 'exercise Podman CLI exact partial label all exporters' \
    supabase_run_exports cli "${socket}" "${current_prefix}"
  progress_run 'exercise all Compose and Quadlet reimport exporters from CLI source' \
    supabase_run_reimports cli "${current_prefix}"
  progress_run 'prove Podman CLI conversion report privacy' \
    supabase_assert_report_privacy "${current_case}"
  progress_run 'refuse Podman CLI Supabase resource collision' \
    supabase_expect_collision "${socket}" "${current_prefix}"
  progress_run 'recreate Podman CLI containers without deleting volumes' \
    supabase_recreate_application cli "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'prove Podman CLI behavior and Realtime after recreation' \
    supabase_probe "${socket}" "${current_prefix}" verify
  progress_run 'reprove Podman CLI database auth storage persistence' \
    supabase_assert_database_state "${socket}" "${current_prefix}" 2
  progress_run 'clean prefix-scoped Podman CLI Supabase resources' \
    supabase_cleanup_mode cli "${socket}" "${current_prefix}" "${run_id}"

  progress_run 'provision independent Docker Compose Supabase application' \
    supabase_provision_compose "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'wait for Docker Compose Supabase readiness' \
    supabase_wait_application "${socket}" "${current_prefix}"
  progress_run 'enable Docker Compose Realtime table publication' \
    supabase_enable_realtime_table "${socket}" "${current_prefix}"
  progress_run 'prove Docker Compose gateway loopback publication' \
    supabase_probe_published_api "${outer}"
  progress_run 'exercise Compose auth database storage Realtime edge graph' \
    supabase_probe "${socket}" "${current_prefix}" seed
  progress_run 'prove Compose database auth storage state' \
    supabase_assert_database_state "${socket}" "${current_prefix}" 1
  progress_run 'prove Compose publication and private network boundaries' \
    supabase_assert_application_boundaries compose "${socket}" "${current_prefix}"
  progress_run 'prove Compose shared storage ownership and access' \
    supabase_assert_storage_ownership "${socket}" "${current_prefix}"
  progress_run 'exercise Compose exact partial label all exporters' \
    supabase_run_exports compose "${socket}" "${current_prefix}"
  progress_run 'exercise all Compose and Quadlet reimport exporters from Compose source' \
    supabase_run_reimports compose "${current_prefix}"
  progress_run 'prove Compose conversion report privacy' \
    supabase_assert_report_privacy "${current_case}"
  progress_run 'refuse Docker Compose Supabase resource collision' \
    supabase_expect_collision "${socket}" "${current_prefix}"
  progress_run 'recreate Compose containers without deleting volumes' \
    supabase_recreate_application compose "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'prove Compose behavior and Realtime after recreation' \
    supabase_probe "${socket}" "${current_prefix}" verify
  progress_run 'reprove Compose database auth storage persistence' \
    supabase_assert_database_state "${socket}" "${current_prefix}" 2
  progress_run 'clean prefix-scoped Docker Compose Supabase resources' \
    supabase_cleanup_mode compose "${socket}" "${current_prefix}" "${run_id}"
  progress_run 'remove disposable Supabase outer container' remove_outer "${outer}"
  printf '%s CELL PASS %s supabase-application (%d/%d tests)\n' \
    "$(timestamp)" "${id}" "${progress_index}" "${progress_total}"
}

supabase_timed_in_shell_operation() {
  local deadline=$1 name=$2 started_at elapsed status runner_pid
  local installed_term_trap=false
  local watchdog_fd watchdog_pid watchdog_ready watchdog_start_fd watchdog_status
  shift 2
  started_at="$(date +%s)"
  runner_pid=${BASHPID}
  printf '%s STEP START %s (deadline %s)\n' \
    "$(timestamp)" "${name}" "${deadline}" >&3
  coproc SUPABASE_DEADLINE_WATCHDOG {
    local start_request
    IFS= read -r start_request || exit 125
    [[ "${start_request}" == start ]] || exit 125
    exec python3 "${script_directory}/lib/in-shell-deadline.py" \
      --runner-pid "${runner_pid}" \
      --deadline "${deadline}" \
      --kill-after "${SUPABASE_CELL_KILL_AFTER}" \
      --name "${name}" 2>&3
  }
  watchdog_pid=$!
  watchdog_fd=${SUPABASE_DEADLINE_WATCHDOG[0]}
  watchdog_start_fd=${SUPABASE_DEADLINE_WATCHDOG[1]}
  if ! printf 'start\n' >&"${watchdog_start_fd}"; then
    exec {watchdog_fd}<&-
    exec {watchdog_start_fd}>&-
    wait "${watchdog_pid}" > /dev/null 2>&1 || true
    printf '%s STEP FAIL  %s (deadline helper failed to arm, exit 125)\n' \
      "$(timestamp)" "${name}" >&3
    return 125
  fi
  watchdog_ready=""
  if ! IFS= read -r -u "${watchdog_fd}" watchdog_ready ||
    [[ "${watchdog_ready}" != ready ]]; then
    exec {watchdog_fd}<&-
    exec {watchdog_start_fd}>&-
    kill -TERM "${watchdog_pid}" > /dev/null 2>&1 || true
    if wait "${watchdog_pid}"; then
      watchdog_status=0
    else
      watchdog_status=$?
    fi
    printf '%s STEP FAIL  %s (deadline helper failed to arm, exit %d)\n' \
      "$(timestamp)" "${name}" "${watchdog_status}" >&3
    return 125
  fi

  if [[ -z "$(trap -p TERM)" ]]; then
    trap 'exit 143' TERM
    installed_term_trap=true
  fi
  "$@"
  status=$?
  if ! printf 'cancel\n' >&"${watchdog_start_fd}"; then
    exec {watchdog_fd}<&-
    exec {watchdog_start_fd}>&-
    if wait "${watchdog_pid}"; then
      watchdog_status=0
    else
      watchdog_status=$?
    fi
    [[ "${installed_term_trap}" == false ]] || trap - TERM
    printf '%s STEP FAIL  %s (deadline helper stopped unexpectedly, exit %d)\n' \
      "$(timestamp)" "${name}" "${watchdog_status}" >&3
    return 125
  fi
  exec {watchdog_start_fd}>&-
  watchdog_ready=""
  if ! IFS= read -r -u "${watchdog_fd}" watchdog_ready ||
    [[ "${watchdog_ready}" != cancelled ]]; then
    exec {watchdog_fd}<&-
    if wait "${watchdog_pid}"; then
      watchdog_status=0
    else
      watchdog_status=$?
    fi
    [[ "${installed_term_trap}" == false ]] || trap - TERM
    printf '%s STEP FAIL  %s (deadline helper rejected cancellation, exit %d)\n' \
      "$(timestamp)" "${name}" "${watchdog_status}" >&3
    return 125
  fi
  exec {watchdog_fd}<&-
  if wait "${watchdog_pid}"; then
    watchdog_status=0
  else
    watchdog_status=$?
  fi
  [[ "${installed_term_trap}" == false ]] || trap - TERM
  if ((watchdog_status != 0)); then
    printf '%s STEP FAIL  %s (deadline helper cancellation returned %d)\n' \
      "$(timestamp)" "${name}" "${watchdog_status}" >&3
    return 125
  fi

  elapsed=$(($(date +%s) - started_at))
  if ((status == 0)); then
    printf '%s STEP PASS %s (%s)\n' \
      "$(timestamp)" "${name}" "$(format_duration "${elapsed}")" >&3
    return 0
  fi
  printf '%s STEP FAIL  %s (%s, exit %d)\n' \
    "$(timestamp)" "${name}" "$(format_duration "${elapsed}")" "${status}" >&3
  return "${status}"
}

run_supabase_application_cell() {
  supabase_timed_in_shell_operation "${SUPABASE_CELL_TIMEOUT}" \
    'complete Supabase application cell' \
    supabase_run_application_cell_unbounded "$@"
}
