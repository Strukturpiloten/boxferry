#!/usr/bin/env bash
# Runtime-independent Nextcloud status, WebDAV, database, and cache checks.
# Callers supply probe placement and acquisition callbacks.

nextcloud_probe_clock() { date +%s; }

nextcloud_probe_assert_status() {
  local status_callback=$1 context=$2 prefix=$3 deadline=${4:-} source document remaining=''
  for source in app proxy; do
    if [[ -n "${deadline}" ]]; then
      remaining=$((deadline - $(nextcloud_probe_clock)))
      ((remaining > 0)) || return 1
    fi
    document="$("${status_callback}" "${context}" "${prefix}" "${source}" "${remaining}")" || return $?
    if [[ -n "${deadline}" ]]; then
      (($(nextcloud_probe_clock) < deadline)) || return 1
    fi
    # Slurping rejects multiple JSON documents; parse errors and private payloads stay private.
    jq --exit-status --slurp \
      'length == 1 and (.[0] | .installed == true and .maintenance == false and .needsDbUpgrade == false and .versionstring == "32.0.10")' \
      > /dev/null 2>&1 <<< "${document}" || return 1
  done
}

nextcloud_probe_wait_status() {
  local status_callback=$1 context=$2 prefix=$3 deadline=$4 status now
  while (($(nextcloud_probe_clock) < deadline)); do
    status=0
    nextcloud_probe_assert_status "${status_callback}" "${context}" "${prefix}" "${deadline}" || status=$?
    ((status == 0)) && return 0
    # Only the adapter's classified temporary HTTP response may authorize another attempt.
    ((status == 75)) || return "${status}"
    now="$(nextcloud_probe_clock)"
    ((now < deadline)) || break
    sleep "$((deadline - now < 2 ? deadline - now : 2))"
  done
  printf '%s\n' 'Timed out waiting for Nextcloud final application status.' >&2
  return 1
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
