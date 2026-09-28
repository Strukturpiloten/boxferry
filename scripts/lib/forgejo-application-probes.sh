#!/usr/bin/env bash
# Forgejo behavior checks shared by application harnesses. Sourcing this file
# has no runtime side effects. Callers supply semantic health, administrator,
# Git, and SQL callbacks. Runtime flags and probe placement stay with callers.

forgejo_probe_wait_application() {
  local wait_callback=$1 health_callback=$2 context=$3 app=$4 endpoint=$5
  "${wait_callback}" 180 'application health endpoint' \
    "${health_callback}" "${context}" "${app}" "${endpoint}"
}

forgejo_probe_create_admin() {
  local admin_callback=$1 context=$2 app=$3 username=$4 password=$5
  "${admin_callback}" "${context}" "${app}" "${username}" "${password}"
}

forgejo_probe_git() {
  local run_callback=$1 context=$2 prefix=$3 mode=$4 username=$5
  local password=$6 http_port=$7 ssh_port=$8
  "${run_callback}" "${context}" "${prefix}" "${mode}" \
    "${username}" "${password}" "${http_port}" "${ssh_port}"
}

forgejo_probe_assert_database() {
  local query_callback=$1 context=$2 database=$3 password=$4 rows
  rows="$("${query_callback}" "${context}" "${database}" "${password}" \
    "SELECT count(*) FROM repository WHERE lower_name = 'migration-baseline';")" || return $?
  [[ "${rows}" == 1 ]]
}
