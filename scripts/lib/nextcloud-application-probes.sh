#!/usr/bin/env bash
# Runtime-independent Nextcloud status, WebDAV, database, and cache checks.
# Callers supply probe placement and acquisition callbacks.

nextcloud_probe_assert_status() {
  local status_callback=$1 context=$2 prefix=$3 source document
  for source in app proxy; do
    document="$("${status_callback}" "${context}" "${prefix}" "${source}")" || return $?
    jq --exit-status \
      '.installed == true and .maintenance == false and .needsDbUpgrade == false and .versionstring == "32.0.10"' \
      > /dev/null <<< "${document}" || return $?
  done
}

nextcloud_probe_webdav_round_trip() {
  local probe_callback=$1 context=$2 prefix=$3 phase=$4 upload=${5:-true}
  local payload="boxferry-nextcloud-${phase}-payload"
  "${probe_callback}" "${context}" "${prefix}" "${payload}" "${upload}"
}

nextcloud_probe_assert_database_cache() {
  local query_callback=$1 cache_host_callback=$2 cache_stats_callback=$3
  local context=$4 prefix=$5 rows host stats
  rows="$("${query_callback}" "${context}" "${prefix}")" || return $?
  [[ "${rows}" =~ ^[0-9]+$ && "${rows}" -ge 1 ]] || return 1
  host="$("${cache_host_callback}" "${context}" "${prefix}")" || return $?
  [[ "${host}" == cache ]] || return 1
  stats="$("${cache_stats_callback}" "${context}" "${prefix}")" || return $?
  awk -F: '
    /^keyspace_hits:/ || /^keyspace_misses:/ { gsub(/\r/, "", $2); total += $2 }
    END { exit total > 0 ? 0 : 1 }
  ' <<< "${stats}"
}
