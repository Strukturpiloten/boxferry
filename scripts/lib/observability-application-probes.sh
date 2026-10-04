#!/usr/bin/env bash
# Runtime-independent observability HTTP assertions. Sourcing performs no work.
# Callers supply a semantic HTTP callback(context, prefix, url); context is opaque.
# Transport, probe placement, deadlines, and redacted callback diagnostics belong to callers.

observability_probe_prometheus_has_value() {
  local http_callback=$1 context=$2 prefix=$3 query=$4 expected=$5 response
  response="$("${http_callback}" "${context}" "${prefix}" \
    "http://prometheus:9090/api/v1/query?query=${query}")" || return $?
  jq --exit-status --slurp --arg expected "${expected}" '
    length == 1 and (.[0] |
      .status == "success" and
      any(.data.result[]?; .value[1] == $expected))
  ' <<< "${response}" > /dev/null 2>&1 || return $?
}

observability_probe_loki_has_known_log() {
  local http_callback=$1 context=$2 prefix=$3 response
  response="$("${http_callback}" "${context}" "${prefix}" \
    'http://loki:3100/loki/api/v1/query_range?query=%7Bjob%3D%22boxferry_fixture%22%7D%20%7C%3D%20%22boxferry-observability-known-log%22&limit=20')" || return $?
  jq --exit-status --slurp '
    length == 1 and (.[0] |
      .status == "success" and
      ([.data.result[]?.values[]? | select(.[1] | contains("boxferry-observability-known-log"))] | length) == 1)
  ' <<< "${response}" > /dev/null 2>&1 || return $?
}

observability_probe_validate_prometheus_flags() {
  local flags=$1
  jq --exit-status --slurp '
    length == 1 and (.[0] |
      .status == "success" and
      (.data | type == "object") and
      (.data["storage.tsdb.retention.time"] | type == "string" and . == "1d") and
      (.data["web.enable-remote-write-receiver"] | type == "string" and . == "true"))
  ' <<< "${flags}" > /dev/null 2>&1 || return $?
}

observability_probe_assert_queries_and_grafana() {
  local http_callback=$1 context=$2 prefix=$3 response
  observability_probe_prometheus_has_value "${http_callback}" "${context}" "${prefix}" \
    'boxferry_fixture_temperature_celsius%7Bsource%3D%22controlled%22%7D' 42 || return $?
  observability_probe_loki_has_known_log "${http_callback}" "${context}" "${prefix}" || return $?

  response="$("${http_callback}" "${context}" "${prefix}" \
    http://prometheus:9090/api/v1/status/flags)" || return $?
  observability_probe_validate_prometheus_flags "${response}" || return $?

  response="$("${http_callback}" "${context}" "${prefix}" \
    http://grafana:3000/api/datasources/uid/boxferry-prometheus)" || return $?
  jq --exit-status --slurp '
    length == 1 and (.[0] |
      .uid == "boxferry-prometheus" and .type == "prometheus" and
      .url == "http://prometheus:9090" and .isDefault == true)
  ' <<< "${response}" > /dev/null 2>&1 || return $?

  response="$("${http_callback}" "${context}" "${prefix}" \
    http://grafana:3000/api/datasources/uid/boxferry-loki)" || return $?
  jq --exit-status --slurp '
    length == 1 and (.[0] |
      .uid == "boxferry-loki" and .type == "loki" and .url == "http://loki:3100")
  ' <<< "${response}" > /dev/null 2>&1 || return $?

  response="$("${http_callback}" "${context}" "${prefix}" \
    http://grafana:3000/api/datasources/uid/boxferry-prometheus/health)" || return $?
  jq --exit-status --slurp 'length == 1 and (.[0] | .status == "OK")' \
    <<< "${response}" > /dev/null 2>&1 || return $?
  response="$("${http_callback}" "${context}" "${prefix}" \
    http://grafana:3000/api/datasources/uid/boxferry-loki/health)" || return $?
  jq --exit-status --slurp 'length == 1 and (.[0] | .status == "OK")' \
    <<< "${response}" > /dev/null 2>&1 || return $?

  response="$("${http_callback}" "${context}" "${prefix}" \
    http://grafana:3000/api/dashboards/uid/boxferry-observability)" || return $?
  jq --exit-status --slurp '
    length == 1 and (.[0] |
      .dashboard.uid == "boxferry-observability" and
      .dashboard.title == "BoxFerry Observability Acceptance" and
      any(.dashboard.panels[].targets[]?; .expr == "boxferry_fixture_temperature_celsius{source=\"controlled\"}") and
      any(.dashboard.panels[].targets[]?; .expr == "{job=\"boxferry_fixture\"} |= \"boxferry-observability-known-log\""))
  ' <<< "${response}" > /dev/null 2>&1 || return $?
}
