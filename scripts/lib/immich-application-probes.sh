#!/usr/bin/env bash
# Runtime-independent Immich media, queue, ML-exclusion, and database checks.
# Callers own the network, container, and SQL execution transport.

immich_probe_wait_application() {
  local wait_callback=$1 probe_callback=$2 context=$3 prefix=$4
  "${wait_callback}" 480 'API readiness and administrator login' \
    "${probe_callback}" "${context}" "${prefix}" ready
}

immich_probe_ingest_phase() {
  local probe_callback=$1 broker_callback=$2 ml_callback=$3
  local context=$4 prefix=$5 phase=$6 before after
  before="$("${broker_callback}" "${context}" "${prefix}")" || return $?
  [[ "${before}" =~ ^[0-9]+$ ]] || return 1
  "${probe_callback}" "${context}" "${prefix}" ingest --phase "${phase}" \
    --state /fixture/probe-state.json --work-dir "/fixture/generated-${phase}" || return $?
  after="$("${broker_callback}" "${context}" "${prefix}")" || return $?
  [[ "${after}" =~ ^[0-9]+$ && "${after}" -gt "${before}" ]] || {
    printf 'Immich upload did not increase Valkey command activity: %s -> %s.\n' \
      "${before}" "${after}" >&2
    return 1
  }
  "${ml_callback}" "${context}" "${prefix}"
}

immich_probe_verify_asset() {
  local probe_callback=$1 ml_callback=$2 context=$3 prefix=$4
  "${probe_callback}" "${context}" "${prefix}" verify \
    --state /fixture/probe-state.json || return $?
  "${ml_callback}" "${context}" "${prefix}"
}

immich_probe_assert_database() {
  local query_callback=$1 context=$2 database=$3 expected=$4
  local asset_rows job_rows extensions extension
  asset_rows="$("${query_callback}" "${context}" "${database}" \
    'SELECT count(*) FROM asset;')" || return $?
  job_rows="$("${query_callback}" "${context}" "${database}" \
    'SELECT count(*) FROM asset_job_status;')" || return $?
  [[ "${asset_rows}" == "${expected}" && "${job_rows}" == "${expected}" ]] || {
    printf 'Immich PostgreSQL expected %s asset/job rows; observed assets=%s jobs=%s.\n' \
      "${expected}" "${asset_rows}" "${job_rows}" >&2
    return 1
  }
  extensions="$("${query_callback}" "${context}" "${database}" \
    "SELECT extname || ':' || extversion FROM pg_extension WHERE extname IN ('cube','earthdistance','vector','vchord') ORDER BY extname;")" || return $?
  grep --fixed-strings --line-regexp --quiet 'vchord:0.4.3' <<< "${extensions}" || return $?
  for extension in cube earthdistance vector; do
    grep --extended-regexp --line-regexp --quiet "${extension}:[0-9.]+" <<< "${extensions}" || return $?
  done
}
