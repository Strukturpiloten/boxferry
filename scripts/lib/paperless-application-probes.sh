#!/usr/bin/env bash
# Runtime-independent Paperless document, broker, and database assertions.
# Callers own probe placement, authentication transport, and runtime commands.

paperless_probe_wait_application() {
  local probe_callback=$1 context=$2 prefix=$3
  "${probe_callback}" "${context}" "${prefix}" ready
}

paperless_probe_ingest_phase() {
  local probe_callback=$1 broker_callback=$2 context=$3 prefix=$4 phase=$5
  local before after
  before="$("${broker_callback}" "${context}" "${prefix}")" || return $?
  before="${before:-0}"
  [[ "${before}" =~ ^[0-9]+$ ]] || return 1
  "${probe_callback}" "${context}" "${prefix}" ingest \
    --phase "${phase}" --state /fixture/probe-state.json \
    --work-dir "/fixture/generated-${phase}" || return $?
  after="$("${broker_callback}" "${context}" "${prefix}")" || return $?
  after="${after:-0}"
  [[ "${after}" =~ ^[0-9]+$ && "${after}" -gt "${before}" ]] || {
    printf 'Valkey LPUSH calls did not increase during %s ingestion: %s -> %s.\n' \
      "${phase}" "${before}" "${after}" >&2
    return 1
  }
}

paperless_probe_verify_documents() {
  local probe_callback=$1 context=$2 prefix=$3
  "${probe_callback}" "${context}" "${prefix}" verify \
    --state /fixture/probe-state.json
}

paperless_probe_assert_database() {
  local query_callback=$1 context=$2 database=$3 password=$4 expected=$5 rows
  [[ "${expected}" == 3 || "${expected}" == 6 ]] || return 1
  rows="$("${query_callback}" "${context}" "${database}" "${password}" \
    "SELECT count(*) FROM documents_document WHERE title LIKE 'BoxFerry migration %';")" || return $?
  [[ "${rows}" == "${expected}" ]]
}
