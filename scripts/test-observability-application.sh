#!/usr/bin/env bash

set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repository_root="$(cd -- "${script_directory}/.." && pwd -P)"
# shellcheck source=scripts/lib/observability-application.sh
source "${script_directory}/lib/observability-application.sh"
# shellcheck source=scripts/lib/scenario-validators.sh
source "${script_directory}/lib/scenario-validators.sh"

test_root="$(mktemp -d)"
trap 'rm -rf -- "${test_root}"' EXIT

diagnostic_base="${repository_root}/fixtures/conformance/observability-application/diagnostics/base-compose-provisioned.tsv"
assert_static_diagnostic_tuple() {
  local subject=$1 code=$2 severity=$3 decision=$4 policy=$5 count=${6:-1}
  awk -F '\t' -v subject="${subject}" -v code="${code}" \
    -v severity="${severity}" -v decision="${decision}" -v policy="${policy}" -v count="${count}" '
    $2 == subject {
      if ($1 == code && $3 == severity && $4 == decision && $5 == policy) {
        matching++
      } else {
        mismatched++
      }
    }
    END { exit !(matching == count && mismatched == 0) }
  ' "${diagnostic_base}" || {
    printf 'Incorrect authored observability diagnostic for %s.\n' "${subject}" >&2
    return 1
  }
}

for resource in backend alloy-data grafana-data loki-data prometheus-data telemetry-logs; do
  if [[ "${resource}" == backend ]]; then
    diagnostic_subject="networks.{{resource_prefix}}${resource}.ownership"
  else
    diagnostic_subject="volumes.{{resource_prefix}}${resource}.ownership"
  fi
  assert_static_diagnostic_tuple \
    "${diagnostic_subject}" BFP0003 warning inferred-application-ownership approximate
done
if grep --fixed-strings --quiet $'networks.{{resource_prefix}}edge.ownership\t' "${diagnostic_base}"; then
  printf '%s\n' 'External edge network must not infer application ownership.' >&2
  exit 1
fi

for mount in 'alloy.mounts[0]' 'alloy.mounts[1]' 'grafana.mounts[0]' \
  'log-producer.mounts[0]' 'loki.mounts[0]' 'prometheus.mounts[0]'; do
  assert_static_diagnostic_tuple \
    "services.{{resource_prefix}}${mount}" BFP0009 note reconstructed -
done
for service in alloy grafana log-producer loki metrics-producer prometheus; do
  for field in networks restart_policy; do
    # The library emits per-attachment notes; the CLI deduplicates identical
    # complete diagnostics without removing Grafana's two network attachments.
    assert_static_diagnostic_tuple \
      "services.{{resource_prefix}}${service}.${field}" BFP0009 note reconstructed -
  done
done
[[ "$(awk -F '\t' '$1 == "BFP0009" { count++ } END { print count + 0 }' "${diagnostic_base}")" == 18 ]]
reviewed_diagnostic_base="${diagnostic_base}"
grafana_network_subject='services.{{resource_prefix}}grafana.networks'
awk -F '\t' -v subject="${grafana_network_subject}" '
  $1 == "BFP0009" && $2 == subject && !removed++ { next }
  { print }
' "${reviewed_diagnostic_base}" > "${test_root}/missing-grafana-network.tsv"
cp -- "${reviewed_diagnostic_base}" "${test_root}/extra-grafana-network.tsv"
printf 'BFP0009\t%s\tnote\treconstructed\t-\n' "${grafana_network_subject}" \
  >> "${test_root}/extra-grafana-network.tsv"
for mutation in missing extra; do
  diagnostic_base="${test_root}/${mutation}-grafana-network.tsv"
  if assert_static_diagnostic_tuple "${grafana_network_subject}" BFP0009 note reconstructed - \
    > /dev/null 2>&1; then
    printf 'Observability contract admitted %s Grafana network reconstruction tuple.\n' "${mutation}" >&2
    exit 1
  fi
done
diagnostic_base="${reviewed_diagnostic_base}"

diagnostic_fixture="${repository_root}/fixtures/conformance/observability-application/diagnostics"
assert_withheld_environment_keys() {
  local importer=$1 exporter=$2 expected_count=$3
  [[ "$(wc -l < "${importer}")" == "${expected_count}" ]] || return 1
  awk -F '\t' '
    NF != 5 || $1 != "BFP0002" || $2 !~ /^services\..*\.environment\./ ||
      $3 != "warning" || $4 != "omitted" || $5 != "partial" { exit 1 }
  ' "${importer}" || return 1
  diff --unified=0 \
    <(awk -F '\t' '{ print $2 }' "${importer}" | sort) \
    <(awk -F '\t' '$1 == "BFP0007" && $2 ~ /\.environment\./ { print $2 }' "${exporter}" | sort)
}
assert_withheld_environment_keys \
  "${diagnostic_fixture}/podman-import-withheld-environment.tsv" "${diagnostic_fixture}/podman-export.tsv" 49
assert_withheld_environment_keys \
  "${diagnostic_fixture}/all-podman-import-withheld-environment.tsv" "${diagnostic_fixture}/all-podman-export.tsv" 9
diagnostic_base="${diagnostic_fixture}/non-podman-environment-promotions.tsv"
for service in alloy grafana log-producer loki metrics-producer prometheus; do
  assert_static_diagnostic_tuple \
    "services.{{resource_prefix}}${service}.environment" BFP0003 warning approximated approximate
done
[[ "$(wc -l < "${diagnostic_base}")" == 6 ]]
diagnostic_base="${diagnostic_fixture}/all-non-podman-environment-promotion.tsv"
assert_static_diagnostic_tuple \
  'services.{{resource_prefix}}boundary-peer.environment' BFP0003 warning approximated approximate
[[ "$(wc -l < "${diagnostic_base}")" == 1 ]]
diagnostic_base="${reviewed_diagnostic_base}"
for key in GF_PLUGINS_PREINSTALL_AUTO_UPDATE GF_PLUGINS_PREINSTALL_DISABLED; do
  subject="services.{{resource_prefix}}grafana.environment.${key}"
  for mutation in missing duplicate malformed extra; do
    mutated="${test_root}/${key}-${mutation}.tsv"
    awk -F '\t' -v subject="${subject}" -v mutation="${mutation}" '
      $2 == subject && mutation == "missing" { next }
      $2 == subject && mutation == "malformed" { sub(/omitted/, "approximated") }
      { print }
      $2 == subject && mutation == "duplicate" { print }
      END {
        if (mutation == "extra")
          print "BFP0002\tservices.{{resource_prefix}}grafana.environment.UNREVIEWED\twarning\tomitted\tpartial"
      }
    ' "${diagnostic_fixture}/podman-import-withheld-environment.tsv" > "${mutated}"
    if assert_withheld_environment_keys "${mutated}" "${diagnostic_fixture}/podman-export.tsv" 49 \
      > /dev/null 2>&1; then
      printf 'Withheld key contract admitted %s %s.\n' "${mutation}" "${key}" >&2
      exit 1
    fi
  done
done

assert_rendered_environment_contract() {
  local rendered=$1 selection=$2 output=$3 importer_expected=${4:-true}
  local -a key_templates=("${diagnostic_fixture}/podman-import-withheld-environment.tsv")
  [[ "${selection}" != all ]] || key_templates+=("${diagnostic_fixture}/all-podman-import-withheld-environment.tsv")
  awk -F '\t' -v output="${output}" -v importer_expected="${importer_expected}" '
    FILENAME != ARGV[ARGC - 1] {
      subject = $2
      sub(/\{\{resource_prefix\}\}/, "bf-private-observability-", subject)
      keys[subject] = 1
      next
    }
    $2 ~ /^services\..*\.environment\./ {
      if (!(($2 in keys) && ($1 == "BFP0002" || $1 == "BFP0007") &&
          $3 == "warning" && $4 == "omitted" && $5 == "partial")) invalid++
      counts[$1 SUBSEP $2]++
    }
    END {
      if (invalid) exit 1
      for (key in keys)
        if (counts["BFP0002" SUBSEP key] != (output == "podman" && importer_expected == "true") ||
            counts["BFP0007" SUBSEP key] != (output == "podman")) exit 1
    }
  ' "${key_templates[@]}" "${rendered}"
}

observability_test_report_from_tsv() {
  jq --raw-input --slurp '{
    diagnostics: (split("\n") | map(select(length > 0) | split("\t") |
      {code: .[0], severity: .[2], fields:
        ([{name: "subject", value: .[1]}] +
         (if .[3] == "" then [] else [{name: "decision", value: .[3]}] end) +
         (if .[4] == "" then [] else [{name: "required_loss_policy", value: .[4]}] end))}))
  }' "$1" > "$2"
}

# Exercise every exporter, provisioning mode, and selector without a runtime.
for diagnostic_mode in cli compose; do
  for diagnostic_selection in exact label all; do
    for diagnostic_output in compose quadlet podman; do
      rendered="${test_root}/native-${diagnostic_mode}-${diagnostic_selection}-${diagnostic_output}.tsv"
      observability_write_live_diagnostic_template \
        "${diagnostic_mode}" "${diagnostic_selection}" podman "${diagnostic_output}" \
        bf-private-observability- "${rendered}"
      expected_notes=18 expected_ownership=6 expected_environment=6
      if [[ "${diagnostic_selection}" == all ]]; then
        expected_notes=20 expected_ownership=7 expected_environment=7
      fi
      [[ "${diagnostic_output}" != podman ]] || expected_environment=0
      awk -F '\t' -v notes="${expected_notes}" -v ownership="${expected_ownership}" \
        -v environment="${expected_environment}" '
        $1 == "BFP0009" {
          if (NF != 5 || $3 != "note" || $4 != "reconstructed" || $5 != "") exit 1
          notes_seen++
        }
        $4 == "inferred-application-ownership" {
          if (NF != 5 || $1 != "BFP0003" || $3 != "warning" || $5 != "approximate") exit 1
          ownership_seen++
        }
        $2 ~ /\.environment$/ && $1 == "BFP0003" &&
          $3 == "warning" && $4 == "approximated" && $5 == "approximate" { environment_seen++ }
        $2 ~ /^services\..*\.logging$/ {
          if ($1 != "BFP0003" || $3 != "warning" || $4 != "not-promoted" || $5 != "partial") exit 1
          logging_seen++
        }
        $2 ~ /edge\.ownership$/ { exit 1 }
        END { exit !(notes_seen == notes && ownership_seen == ownership &&
          environment_seen == environment && logging_seen == ownership) }
      ' "${rendered}"
      assert_rendered_environment_contract "${rendered}" "${diagnostic_selection}" "${diagnostic_output}"
      if [[ "${diagnostic_selection}" == all ]]; then
        for field in networks restart_policy; do
          awk -F '\t' -v subject="services.bf-private-observability-boundary-peer.${field}" '
            $2 == subject {
              if ($1 == "BFP0009" && $3 == "note" && $4 == "reconstructed" && $5 == "") matching++
              else mismatched++
            }
            END { exit !(matching == 1 && mismatched == 0) }
          ' "${rendered}"
        done
        grep --fixed-strings --line-regexp --quiet \
          $'BFP0003\tnetworks.podman.ownership\twarning\tinferred-application-ownership\tapproximate' "${rendered}"
      fi
      # Synthetic reports test normalization; the literal tuples above remain
      # independent semantic expectations, never derived from native output.
      diagnostic_report="${rendered}.json"
      observability_test_report_from_tsv "${rendered}" "${diagnostic_report}"
      observability_assert_reviewed_diagnostics live-native-export \
        "${diagnostic_mode}" "${diagnostic_selection}" podman "${diagnostic_output}" \
        bf-private-observability- "${diagnostic_report}"
      for mutation in missing duplicate severity decision policy duplicate-field; do
        jq --arg mutation "${mutation}" '
          ([.diagnostics | to_entries[] | select(.value.code == "BFP0009")][0].key) as $index |
          if $mutation == "missing" then del(.diagnostics[$index])
          elif $mutation == "duplicate" then .diagnostics += [.diagnostics[$index]]
          elif $mutation == "severity" then .diagnostics[$index].severity = "warning"
          elif $mutation == "decision" then
            .diagnostics[$index].fields |= map(if .name == "decision" then .value = "approximated" else . end)
          elif $mutation == "policy" then
            .diagnostics[$index].fields += [{name: "required_loss_policy", value: "approximate"}]
          else .diagnostics[$index].fields += [{name: "decision", value: "reconstructed"}]
          end
        ' "${diagnostic_report}" > "${diagnostic_report}.mutated"
        if observability_assert_reviewed_diagnostics live-native-export \
          "${diagnostic_mode}" "${diagnostic_selection}" podman "${diagnostic_output}" \
          bf-private-observability- "${diagnostic_report}.mutated" > /dev/null 2>&1; then
          printf 'Native contract admitted %s reconstruction note for %s/%s/%s.\n' \
            "${mutation}" "${diagnostic_mode}" "${diagnostic_selection}" "${diagnostic_output}" >&2
          exit 1
        fi
      done
      if [[ "${diagnostic_output}" == podman ]]; then
        for key in GF_PLUGINS_PREINSTALL_AUTO_UPDATE GF_PLUGINS_PREINSTALL_DISABLED; do
          for code in BFP0002 BFP0007; do
            for mutation in missing duplicate malformed extra; do
              jq --arg key "${key}" --arg code "${code}" --arg mutation "${mutation}" '
                ([.diagnostics | to_entries[] | select(.value.code == $code and
                  any(.value.fields[]; .name == "subject" and (.value | endswith(".environment." + $key))))][0].key) as $index |
                if $mutation == "missing" then del(.diagnostics[$index])
                elif $mutation == "duplicate" then .diagnostics += [.diagnostics[$index]]
                elif $mutation == "malformed" then
                  .diagnostics[$index].fields |= map(if .name == "decision" then .value = "approximated" else . end)
                else .diagnostics += [.diagnostics[$index] |
                  .fields |= map(if .name == "subject" then .value += ".UNREVIEWED" else . end)]
                end
              ' "${diagnostic_report}" > "${diagnostic_report}.key-mutated"
              if observability_assert_reviewed_diagnostics live-native-export \
                "${diagnostic_mode}" "${diagnostic_selection}" podman podman \
                bf-private-observability- "${diagnostic_report}.key-mutated" > /dev/null 2>&1; then
                printf 'Native contract admitted %s %s %s.\n' "${mutation}" "${code}" "${key}" >&2
                exit 1
              fi
            done
          done
        done
      fi
    done
  done
done
for malformed in \
  $'BFP0009\tsubject\twarning\treconstructed\t-' \
  $'BFP0009\tsubject\tnote\tapproximated\t-' \
  $'BFP0009\tsubject\tnote\treconstructed\tapproximate' \
  $'BFP0009\tsubject\tnote\treconstructed\t' \
  $'BFP0003\tsubject\tnote\tinferred-application-ownership\tapproximate' \
  $'BFP0003\tsubject\twarning\tinferred-application-ownership\t-'; do
  printf '%s\n' "${malformed}" > "${test_root}/malformed-native.tsv"
  if observability_append_live_diagnostic_template \
    "${test_root}/malformed-native.tsv" bf-private-observability- \
    "${test_root}/malformed-native-rendered.tsv" > /dev/null 2>&1; then
    printf '%s\n' 'A malformed native reconstruction or ownership tuple was accepted.' >&2
    exit 1
  fi
done

# Reimports read emitted documents, not a native Podman inventory.
while read -r diagnostic_mode diagnostic_selection diagnostic_input diagnostic_output expected_rows; do
  rendered="${test_root}/reimport-${diagnostic_mode}-${diagnostic_selection}-${diagnostic_input}-${diagnostic_output}.tsv"
  observability_write_live_reimport_diagnostic_template \
    "${diagnostic_mode}" "${diagnostic_selection}" "${diagnostic_input}" "${diagnostic_output}" \
    bf-private-observability- "${rendered}"
  [[ "$(wc -l < "${rendered}")" == "${expected_rows}" ]]
  if grep --extended-regexp --quiet '^BFP000[239][[:space:]]|\.logging[[:space:]]' "${rendered}"; then
    printf '%s\n' 'A reimport acquired native-importer or observation-only logging diagnostics.' >&2
    exit 1
  fi
  assert_rendered_environment_contract "${rendered}" "${diagnostic_selection}" "${diagnostic_output}" false
  diagnostic_report="${rendered}.json"
  observability_test_report_from_tsv "${rendered}" "${diagnostic_report}"
  observability_assert_reviewed_diagnostics live-reimport \
    "${diagnostic_mode}" "${diagnostic_selection}" "${diagnostic_input}" "${diagnostic_output}" \
    bf-private-observability- "${diagnostic_report}"
  jq '.diagnostics += [{code: "BFP0002", severity: "warning", fields: [
    {name: "subject", value: "services.bf-private-observability-grafana.environment.GF_PLUGINS_PREINSTALL_DISABLED"},
    {name: "decision", value: "omitted"}, {name: "required_loss_policy", value: "partial"}
  ]}]' "${diagnostic_report}" > "${diagnostic_report}.fallback"
  if observability_assert_reviewed_diagnostics live-reimport \
    "${diagnostic_mode}" "${diagnostic_selection}" "${diagnostic_input}" "${diagnostic_output}" \
    bf-private-observability- "${diagnostic_report}.fallback" > /dev/null 2>&1; then
    printf '%s\n' 'A native-importer fallback satisfied a reimport contract.' >&2
    exit 1
  fi
done << 'EOF'
cli exact compose compose 0
cli exact compose quadlet 1
cli exact compose podman 58
cli exact quadlet compose 8
cli exact quadlet quadlet 0
cli exact quadlet podman 65
cli label compose compose 0
cli label compose quadlet 1
cli label compose podman 58
cli label quadlet compose 8
cli label quadlet quadlet 0
cli label quadlet podman 65
cli all compose compose 0
cli all compose quadlet 1
cli all compose podman 68
cli all quadlet compose 9
cli all quadlet quadlet 0
cli all quadlet podman 76
compose exact compose compose 0
compose exact compose quadlet 1
compose exact compose podman 58
compose exact quadlet compose 1
compose exact quadlet quadlet 0
compose exact quadlet podman 58
compose label compose compose 0
compose label compose quadlet 1
compose label compose podman 58
compose label quadlet compose 1
compose label quadlet quadlet 0
compose label quadlet podman 58
compose all compose compose 0
compose all compose quadlet 1
compose all compose podman 68
compose all quadlet compose 2
compose all quadlet quadlet 0
compose all quadlet podman 69
EOF

assert_absent() {
  local value=$1 output=$2
  if grep --fixed-strings --quiet -- "${value}" <<< "${output}"; then
    printf '%s\n' 'A protected diagnostic value escaped.' >&2
    return 1
  fi
}

(
  clock_file="${test_root}/grafana-clock"
  calls_file="${test_root}/grafana-health-calls"
  diagnostics_file="${test_root}/grafana-health-diagnostics"
  private_marker='private-password-runtime-identity-/run/private.sock-10.88.0.9'
  observability_clock() { printf '%s\n' "$(< "${clock_file}")"; }
  sleep() {
    [[ "$1" == 1 || "$1" == 2 ]]
    # Simulate scheduler delay without adding real waits to the offline suite.
    printf '%s\n' "$(($(< "${clock_file}") + 60))" > "${clock_file}"
  }
  observability_grafana_health_request() {
    local path=$3 request_timeout=$4 endpoint=loki now count
    case "${path}" in
      /api/health) endpoint=basic ;;
      /api/datasources/uid/boxferry-prometheus/health) endpoint=prometheus ;;
      /api/datasources/uid/boxferry-loki/health) ;;
      *) return 2 ;;
    esac
    now=$(< "${clock_file}")
    ((request_timeout > 0 && request_timeout <= 5 && request_timeout <= 240 - now)) || return 90
    printf '%s %s %s\n' "${endpoint}" "${request_timeout}" "${now}" >> "${calls_file}"
    if [[ "${mode}" == exhausted ]]; then
      if [[ "${endpoint}" == basic ]]; then
        printf '%s\n' "$((now + 239))" > "${clock_file}"
      else
        printf '%s\n' "$((now + 1))" > "${clock_file}"
      fi
    elif [[ "${mode}" == late ]]; then
      printf '%s\n' 241 > "${clock_file}"
    fi
    if [[ "${endpoint}" == basic ]]; then
      printf 'HTTP/1.1 200 OK\n\n{"database":"ok","version":"fixture"}\n'
      return 0
    fi
    if [[ "${endpoint}" == prometheus || "${mode}" == exhausted ]]; then
      printf 'HTTP/1.1 200 OK\n\n{"status":"OK"}\n'
      return 0
    fi
    count=$(grep --count '^loki ' "${calls_file}")
    case "${mode}" in
      delayed)
        if ((count > 1)); then
          printf 'HTTP/1.1 200 OK\n\n{"status":"OK"}\n'
          return 0
        fi
        ;;
      malformed)
        printf 'HTTP/1.1 200 OK\n\n{"status":42,"message":"%s"}\n' "${private_marker}"
        return 0
        ;;
      unknown)
        printf 'HTTP/1.1 404 Not Found\n\n{"messageId":"%s","message":"%s"}\n' "${private_marker}" "${private_marker}"
        return 1
        ;;
      timeout)
        printf '%s\n' "${private_marker}" >&2
        return 124
        ;;
      failed-client)
        printf 'HTTP/1.1 200 OK\n\n{"status":"OK"}\n'
        return 1
        ;;
    esac
    printf 'HTTP/1.1 404 Not Found\n\n{"messageId":"plugin.notRegistered","message":"%s"}\n' "${private_marker}"
    return 1
  }
  printf '%s\n' 0 > "${clock_file}"
  : > "${calls_file}"
  mode=control
  observability_grafana_health_request /tmp/private.sock bf-private /api/health 5 > /dev/null
  [[ "$(< "${calls_file}")" == 'basic 5 0' ]]
  for mode in delayed persistent malformed unknown timeout failed-client exhausted late; do
    printf '%s\n' 0 > "${clock_file}"
    : > "${calls_file}"
    if observability_wait_for 240 'Grafana readiness' observability_grafana_ready \
      /tmp/private.sock bf-private > /dev/null 2> "${diagnostics_file}"; then
      [[ "${mode}" == delayed ]]
      [[ "$(< "${clock_file}")" == 60 ]]
      [[ "$(wc -l < "${calls_file}")" == 6 ]]
      [[ ! -s "${diagnostics_file}" ]]
    else
      [[ "${mode}" != delayed ]]
      observability_report_grafana_health_failure 2>> "${diagnostics_file}"
      diagnostics=$(< "${diagnostics_file}")
      grep --fixed-strings --quiet 'Timed out after 240s waiting for observability Grafana readiness.' <<< "${diagnostics}"
      case "${mode}" in
        persistent) expected='endpoint=loki http-status=404 error-category=plugin-not-registered' ;;
        malformed) expected='endpoint=loki http-status=200 error-category=malformed-response' ;;
        unknown) expected='endpoint=loki http-status=404 error-category=unknown' ;;
        timeout | exhausted) expected='endpoint=loki http-status=unknown error-category=unknown' ;;
        failed-client) expected='endpoint=loki http-status=200 error-category=transport-failure' ;;
        late) expected='endpoint=prometheus http-status=unknown error-category=unknown' ;;
      esac
      grep --fixed-strings --quiet "${expected}" <<< "${diagnostics}"
      ((${#diagnostics} < 256))
      for forbidden in "${private_marker}" /tmp/private.sock bf-private; do
        assert_absent "${forbidden}" "${diagnostics}"
      done
      if [[ "${mode}" == exhausted ]]; then
        [[ "$(< "${calls_file}")" == $'basic 5 0\nprometheus 1 239' ]]
      elif [[ "${mode}" == late ]]; then
        [[ "$(wc -l < "${calls_file}")" == 1 ]]
      else
        [[ "$(< "${clock_file}")" == 240 ]]
        [[ "$(wc -l < "${calls_file}")" == 12 ]]
      fi
    fi
  done
  classified=$(printf 'HTTP/1.1 200 OK\n\n{"status":"OK"}\ntrailing-private-data' | observability_classify_grafana_health loki)
  [[ "${classified}" == $'200\tmalformed-response\tfalse' ]]
  classified=$(printf 'HTTP/1.1 404 Not Found\n\n{"messageId":"plugin.notImplemented"}' | observability_classify_grafana_health loki)
  [[ "${classified}" == $'404\tmethod-not-implemented\tfalse' ]]
  classified=$(printf 'HTTP/1.1 503 Unavailable\n\n{"messageId":"plugin.unavailable"}' | observability_classify_grafana_health loki)
  [[ "${classified}" == $'503\tplugin-unavailable\tfalse' ]]
  classified=$(printf 'HTTP/1.1 200 OK\n\n{"database":true}' | observability_classify_grafana_health basic)
  [[ "${classified}" == $'200\tmalformed-response\tfalse' ]]
  classified=$(printf 'HTTP/1.1 200 OK\n\n{"database":"failed"}' | observability_classify_grafana_health basic)
  [[ "${classified}" == $'200\tunhealthy-response\tfalse' ]]
  classified=$(printf '  HTTP/1.1 200 OK\r\n  Content-Type: application/json\r\n  X-Fixture: {private}\r\n{"status":"OK"}' | observability_classify_grafana_health loki)
  [[ "${classified}" == $'200\tunknown\ttrue' ]]
  classified=$(printf 'HTTP/1.1 200 OK\n  Content-Type: application/json\n  Content-Length: 17\n  Connection: close\n  \n{"database":"ok"}' | observability_classify_grafana_health basic)
  [[ "${classified}" == $'200\tunknown\ttrue' ]]
  for endpoint in basic prometheus loki; do
    field=status expected_value=OK
    [[ "${endpoint}" != basic ]] || {
      field=database
      expected_value=ok
    }
    for malformed_body in \
      "{\"${field}\":\"failed\",\"${field}\":\"${expected_value}\"}" \
      "{\"${field}\":\"${expected_value}\",\"${field}\":\"${expected_value}\"}" \
      "{\"${field}\":\"${expected_value}\",\"extra\":{\"private\":1,\"private\":2}}"; do
      classified=$(printf 'HTTP/1.1 200 OK\n\n%s' "${malformed_body}" | observability_classify_grafana_health "${endpoint}")
      [[ "${classified}" == $'200\tmalformed-response\tfalse' ]]
    done
    for nonfinite in NaN Infinity -Infinity; do
      classified=$(printf 'HTTP/1.1 200 OK\n\n{"%s":"%s","unused":[%s]}' "${field}" "${expected_value}" "${nonfinite}" | observability_classify_grafana_health "${endpoint}")
      [[ "${classified}" == $'200\tmalformed-response\tfalse' ]]
    done
    classified=$(printf 'HTTP/1.1 200 OK\n\n{"%s":"%s","unused":{"finite":1.5,"nothing":null}}' "${field}" "${expected_value}" | observability_classify_grafana_health "${endpoint}")
    [[ "${classified}" == $'200\tunknown\ttrue' ]]
  done
  classified=$(printf '{"status":"OK"}' | observability_classify_grafana_health loki)
  [[ "${classified}" == $'unknown\tmalformed-response\tfalse' ]]
  for malformed_body in 'not-json' 'HTTP/1.1 200 OK\n\n{"status":"OK"}' '{"status":"OK"}\nHTTP/1.1 200 OK'; do
    classified=$(printf 'HTTP/1.1 404 Not Found\n\n%b' "${malformed_body}" | observability_classify_grafana_health loki)
    [[ "${classified}" == $'404\tunknown\tfalse' ]]
  done
  classified=$(printf 'HTTP/1.1 200 OK\nHTTP/1.1 404 Not Found\n\n{"status":"OK"}' | observability_classify_grafana_health loki)
  [[ "${classified}" == $'200\tmalformed-response\tfalse' ]]
  classified=$(printf 'HTTP/1.1 200 OK\n\nnot-json' | observability_classify_grafana_health loki)
  [[ "${classified}" == $'200\tmalformed-response\tfalse' ]]
  classified=$(printf 'HTTP/1.1 404 Not Found\n\n{"message":"plugin.notRegistered","messageId":"plugin.notRegistered.extra"}' | observability_classify_grafana_health loki)
  [[ "${classified}" == $'404\tunknown\tfalse' ]]
  classified=$(python3 -c 'print("HTTP/1.1 200 OK\n\n" + "x" * 16385)' | observability_classify_grafana_health loki)
  [[ "${classified}" == $'unknown\toversized-response\tfalse' ]]
  OBSERVABILITY_GRAFANA_ENDPOINT=${private_marker}
  OBSERVABILITY_GRAFANA_HTTP_STATUS=${private_marker}
  OBSERVABILITY_GRAFANA_ERROR_CATEGORY=${private_marker}
  diagnostics=$(observability_report_grafana_health_failure 2>&1)
  [[ "${diagnostics}" == 'OBSERVABILITY DIAGNOSTIC endpoint=unknown http-status=unknown error-category=unknown' ]]
)

(
  request_arguments="${test_root}/grafana-request-arguments"
  timeout() { printf '%s\n' "$@" > "${request_arguments}"; }
  engine="fixture-engine"
  started_outer=owned-outer
  observability_grafana_health_request /tmp/nonexistent-observability.sock bf-private /api/health 3
  [[ "$(head -n 7 "${request_arguments}")" == $'--foreground\n--signal=KILL\n3s\nfixture-engine\nexec\nowned-outer\npodman' ]]
  [[ "$(tail -n 2 "${request_arguments}")" == $'http://grafana:3000/api/health\n3' ]]
  socket="${test_root}/grafana-request.sock"
  python3 -c 'import socket,sys; stream=socket.socket(socket.AF_UNIX); stream.bind(sys.argv[1]); stream.close()' "${socket}"
  observability_grafana_health_request "${socket}" bf-private /api/health 1
  [[ "$(head -n 6 "${request_arguments}")" == "$(printf '%s\n' --foreground --signal=KILL 1s fixture-engine --url "unix://${socket}")" ]]
  [[ "$(tail -n 1 "${request_arguments}")" == 1 ]]
)

(
  waits_file="${test_root}/grafana-wait-integration"
  observability_wait_for() { printf '%s\t%s\t%s\n' "$1" "$2" "$3" >> "${waits_file}"; }
  observability_pipeline_roles_running() { return 0; }
  observability_wait_application /tmp/private.sock bf-private
  [[ "$(wc -l < "${waits_file}")" == 5 ]]
  [[ "$(grep --count $'240\tGrafana readiness\tobservability_grafana_ready' "${waits_file}")" == 1 ]]
)

(
  client="${test_root}/grafana-slow-client"
  client_marker="${test_root}/grafana-slow-client.pid"
  printf '%s\n' '#!/bin/sh' "printf \"%s\\n\" \"\$\$\" > \"\$CLIENT_MARKER\"" 'exec sleep 100' > "${client}"
  chmod 0700 "${client}"
  CLIENT_MARKER="${client_marker}" python3 - "${script_directory}/lib/observability-application.sh" "${client}" << 'PY'
import os
import pathlib
import select
import signal
import subprocess
import sys
import time

helper, client = sys.argv[1:]
marker = pathlib.Path(os.environ["CLIENT_MARKER"])
child = subprocess.Popen(
    ["bash", "-c", 'source "$1"; engine="$2"; started_outer=; observability_grafana_health_request /tmp/private.sock bf-private /api/health 1', "fixture", helper, client],
    stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True,
)
client_fd = None
try:
    deadline = time.monotonic() + 3
    while not marker.exists() and time.monotonic() < deadline:
        time.sleep(0.01)
    assert marker.exists(), "owned dummy client must start"
    client_fd = os.pidfd_open(int(marker.read_text()))
    child.communicate(timeout=3)
    assert child.returncode == 137, "hard client timer must fail, never report health"
    poller = select.poll()
    poller.register(client_fd, select.POLLIN)
    assert poller.poll(1000), "owned host client must not survive its deadline"
    assert not pathlib.Path(f"/proc/{int(marker.read_text())}").exists(), "timer must reap its direct host client"
finally:
    if client_fd is not None:
        try:
            signal.pidfd_send_signal(client_fd, signal.SIGKILL)
        except ProcessLookupError:
            pass
        os.close(client_fd)
    if child.poll() is None:
        os.killpg(child.pid, signal.SIGKILL)
    child.communicate(timeout=3)
PY
)

(
  cli_arguments="${test_root}/grafana-cli-arguments"
  observability_remote() { printf '%s\n' "$@" > "${cli_arguments}"; }
  observability_image_reference() { printf '%s\n' fixture-grafana-image; }
  observability_create_cli_grafana /tmp/private.sock bf-private fixture-run
  [[ "$(grep --count '^GF_PLUGINS_PREINSTALL_DISABLED=true$' "${cli_arguments}")" == 1 ]]
  [[ "$(grep --count '^GF_PLUGINS_PREINSTALL_AUTO_UPDATE=false$' "${cli_arguments}")" == 1 ]]
  assert_absent GF_INSTALL_PLUGINS "$(< "${cli_arguments}")"
)

python3 - "${repository_root}" << 'PY'
import json
import pathlib
import sys
import tomllib

root = pathlib.Path(sys.argv[1])
scenario = root / "fixtures/scenarios/observability-application"
native = root / "fixtures/conformance/observability-application"
required = {"GF_PLUGINS_PREINSTALL_AUTO_UPDATE": "false", "GF_PLUGINS_PREINSTALL_DISABLED": "true"}
for key, value in required.items():
    assert f'{key}: "{value}"' in (native / "compose.yaml").read_text()
    assert f"- {key}={value}" in (scenario / "compose.yaml").read_text()
    assert f"Environment={key}={value}" in (scenario / "grafana.container").read_text()
contract = tomllib.loads((scenario / "scenario.toml").read_text())
for key, value in required.items():
    assert f"grafana:{key}={value}" in contract["semantics"]["required-environment"]
cassette = json.loads((scenario / "input-podman.cassette.json").read_text())
def find_grafana(value):
    if isinstance(value, dict):
        if value.get("Name") == "grafana" and "Config" in value:
            return value
        for child in value.values():
            found = find_grafana(child)
            if found is not None:
                return found
    elif isinstance(value, list):
        for child in value:
            found = find_grafana(child)
            if found is not None:
                return found
    return None
grafana = find_grafana(cassette)
assert grafana is not None
for key, value in required.items():
    assert f"{key}={value}" in grafana["Config"]["Env"]
for source in ["compose", "quadlet", "podman"]:
    diagnostics = (scenario / f"expected.{source}-podman.diagnostics").read_text()
    losses = (scenario / f"expected.{source}-podman.losses.tsv").read_text()
    for key in required:
        assert f"BFP0007|services.grafana.environment.{key}" in diagnostics
        assert f"BFP0007\tservices.grafana.environment.{key}\tunsupported\t" in losses
for name in ["podman-export", "reimport-compose-podman", "reimport-quadlet-podman"]:
    diagnostics = (native / "diagnostics" / f"{name}.tsv").read_text()
    for key in required:
        assert f"BFP0007\tservices.{{{{resource_prefix}}}}grafana.environment.{key}\twarning\tomitted\tpartial" in diagnostics
PY

withheld_directory="${test_root}/default-withheld"
mkdir -p -- "${withheld_directory}"
printf '%s\n' 'services:' '  bf-private-observability-grafana:' \
  '    environment:' '      GF_SECURITY_ADMIN_PASSWORD: <withheld>' \
  > "${withheld_directory}/compose.yaml"
printf '%s\n' '[Container]' 'Image=example.invalid/grafana:1' \
  > "${withheld_directory}/bf-private-observability-grafana.container"
withheld_report="${test_root}/default-withheld.report.json"
jq --null-input \
  --arg subject 'services.bf-private-observability-grafana.environment.GF_SECURITY_ADMIN_PASSWORD' \
  '{status: "success", exit_category: "success", output_artifacts: [{name: "compose.yaml"}],
    diagnostics: [{code: "BFP0002", severity: "warning",
      fields: [{name: "subject", value: $subject}]}]}' > "${withheld_report}"
observability_assert_default_withholding \
  compose "${withheld_directory}" "${withheld_report}" bf-private
observability_assert_default_withholding \
  quadlet "${withheld_directory}" "${withheld_report}" bf-private
missing_grafana_directory="${test_root}/default-withheld-missing-grafana"
mkdir -p -- "${missing_grafana_directory}"
printf '%s\n' 'services:' '  unrelated: {}' \
  > "${missing_grafana_directory}/compose.yaml"
if observability_assert_default_withholding \
  compose "${missing_grafana_directory}" "${withheld_report}" bf-private \
  > /dev/null 2>&1; then
  printf '%s\n' 'Default withholding accepted an export missing Grafana.' >&2
  exit 1
fi
jq '.diagnostics = []' "${withheld_report}" > "${test_root}/missing-withholding.report.json"
if observability_assert_default_withholding \
  compose "${withheld_directory}" "${test_root}/missing-withholding.report.json" bf-private \
  > /dev/null 2>&1; then
  printf '%s\n' 'Default withholding passed without a named diagnostic.' >&2
  exit 1
fi
printf '%s\n' "${OBSERVABILITY_ADMIN_PASSWORD}" >> "${withheld_directory}/compose.yaml"
if observability_assert_default_withholding \
  compose "${withheld_directory}" "${withheld_report}" bf-private > /dev/null 2>&1; then
  printf '%s\n' 'Default withholding accepted a retained password canary.' >&2
  exit 1
fi

observability_validate_alloy_scrape_timing \
  "${repository_root}/fixtures/conformance/observability-application/config.alloy"

alias_directory="${test_root}/output-aliases-compose"
mkdir -p "${alias_directory}"
printf '%s\n' '---
services:
  bf-private-observability-grafana:
    networks:
      bf-private-observability-backend:
        aliases: [grafana]
      bf-private-observability-edge:
        aliases: [grafana]
  bf-private-observability-loki:
    networks:
      bf-private-observability-backend:
        aliases: [loki]
  bf-private-observability-metrics-producer:
    networks:
      bf-private-observability-backend:
        aliases: [metrics-producer]
  bf-private-observability-prometheus:
    networks:
      bf-private-observability-backend:
        aliases: [prometheus]
  bf-private-observability-alloy:
    networks: [bf-private-observability-backend]' > "${alias_directory}/compose.yaml"
observability_assert_output_aliases cli podman compose "${alias_directory}" bf-private

compose_alias_fixture="${repository_root}/fixtures/conformance/observability-application/expected-compose-provisioned-aliases.yaml"
compose_alias_directory="${test_root}/output-aliases-compose-provisioned"
mkdir -p "${compose_alias_directory}"
cp -- "${compose_alias_fixture}" "${compose_alias_directory}/compose.yaml"
observability_assert_output_aliases \
  compose podman compose "${compose_alias_directory}" bf-private
observability_assert_output_aliases \
  compose compose compose "${compose_alias_directory}" bf-private

compose_alias_podman_directory="${test_root}/output-aliases-compose-provisioned-podman"
mkdir -p "${compose_alias_podman_directory}"
jq --null-input --arg stem 'bf-private-observability-' '
  def create($role; $networks):
    {action: "create", resource: {kind: "container", name: ($stem + $role)},
     libpod: {body: {json: {Networks: $networks}}}};
  ($stem + "backend") as $backend
  | ($stem + "edge") as $edge
  | {operations: [
      create("alloy"; {($backend): {aliases: [($stem + "alloy"), "alloy"]}}),
      create("grafana"; {
        ($backend): {aliases: [($stem + "grafana"), "grafana"]},
        ($edge): {aliases: [($stem + "grafana"), "grafana"]}
      }),
      create("log-producer"; {($backend): {aliases: [($stem + "log-producer"), "log-producer"]}}),
      create("loki"; {($backend): {aliases: [($stem + "loki"), "loki"]}}),
      create("metrics-producer"; {
        ($backend): {aliases: [($stem + "metrics-producer"), "metrics-producer"]}
      }),
      create("prometheus"; {($backend): {aliases: [($stem + "prometheus"), "prometheus"]}})
    ]}
' > "${compose_alias_podman_directory}/podman.json"
observability_assert_output_aliases \
  compose podman podman "${compose_alias_podman_directory}" bf-private
observability_assert_output_aliases \
  compose compose podman "${compose_alias_podman_directory}" bf-private

compose_alias_quadlet_directory="${test_root}/output-aliases-compose-provisioned-quadlet"
mkdir -p "${compose_alias_quadlet_directory}"
for service in alloy log-producer loki metrics-producer prometheus; do
  printf '[Container]\nNetworkAlias=bf-private-observability-%s\nNetworkAlias=%s\nNetwork=bf-private-observability-backend.network\n' \
    "${service}" "${service}" \
    > "${compose_alias_quadlet_directory}/bf-private-observability-${service}.container"
done
printf '%s\n' \
  '[Container]' \
  'Network=bf-private-observability-backend.network' \
  'Network=bf-private-observability-edge' \
  > "${compose_alias_quadlet_directory}/bf-private-observability-grafana.container"
for source_kind in podman compose quadlet; do
  observability_assert_output_aliases \
    compose "${source_kind}" quadlet "${compose_alias_quadlet_directory}" bf-private
done

compose_alias_without_grafana_directory="${test_root}/output-aliases-compose-provisioned-without-grafana"
mkdir -p "${compose_alias_without_grafana_directory}"
cp -- "${compose_alias_fixture}" "${compose_alias_without_grafana_directory}/compose.yaml"
sed --in-place \
  's/aliases: \[bf-private-observability-grafana, grafana\]/aliases: []/' \
  "${compose_alias_without_grafana_directory}/compose.yaml"
observability_assert_output_aliases \
  compose quadlet compose "${compose_alias_without_grafana_directory}" bf-private

compose_alias_quadlet_podman_directory="${test_root}/output-aliases-compose-provisioned-quadlet-podman"
mkdir -p "${compose_alias_quadlet_podman_directory}"
jq '
  (.operations[]
   | select(.resource.name | endswith("-grafana"))
   | .libpod.body.json.Networks[]
   | .aliases) = []
' "${compose_alias_podman_directory}/podman.json" \
  > "${compose_alias_quadlet_podman_directory}/podman.json"
observability_assert_output_aliases \
  compose quadlet podman "${compose_alias_quadlet_podman_directory}" bf-private

compose_alias_duplicate_directory="${test_root}/output-aliases-compose-provisioned-duplicate-attachment"
mkdir -p "${compose_alias_duplicate_directory}"
jq '.operations += [.operations[0]]' "${compose_alias_podman_directory}/podman.json" \
  > "${compose_alias_duplicate_directory}/podman.json"
status=0
error="$(observability_assert_output_aliases \
  compose podman podman "${compose_alias_duplicate_directory}" bf-private 2>&1)" || status=$?
[[ "${status}" == 1 ]]
grep --fixed-strings --quiet \
  'duplicate-attachments=alloy/backend count=2' <<< "${error}"

cp -- "${compose_alias_fixture}" "${compose_alias_directory}/compose.yaml"
sed --in-place \
  's/aliases: \[bf-private-observability-alloy, alloy\]/aliases: [bf-private-observability-alloy, alloy, alloy]/' \
  "${compose_alias_directory}/compose.yaml"
if observability_assert_output_aliases \
  compose podman compose "${compose_alias_directory}" bf-private 2> /dev/null; then
  printf '%s\n' 'A duplicate Compose-provisioned alias satisfied the observability output contract.' >&2
  exit 1
fi

cp -- "${compose_alias_fixture}" "${compose_alias_directory}/compose.yaml"
sed --in-place \
  's/aliases: \[bf-private-observability-grafana, grafana\]/aliases: [bf-private-observability-grafana]/' \
  "${compose_alias_directory}/compose.yaml"
if observability_assert_output_aliases \
  compose podman compose "${compose_alias_directory}" bf-private 2> /dev/null; then
  printf '%s\n' 'A missing Compose-provisioned alias satisfied the observability output contract.' >&2
  exit 1
fi

cp -- "${compose_alias_fixture}" "${compose_alias_directory}/compose.yaml"
sed --in-place \
  's/aliases: \[bf-private-observability-loki, loki\]/aliases: [bf-private-observability-loki, loki, private-alias-canary]/' \
  "${compose_alias_directory}/compose.yaml"
status=0
error="$(observability_assert_output_aliases \
  compose podman compose "${compose_alias_directory}" bf-private 2>&1)" || status=$?
[[ "${status}" == 1 ]]
assert_absent private-alias-canary "${error}"
grep --fixed-strings --quiet \
  'value-mismatches=loki/backend expected-count=2 actual-count=3' <<< "${error}"

printf '%s\n' '---
services:
  bf-private-observability-loki:
    networks:
      bf-private-observability-backend:
        aliases: [loki, 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef]
  bf-private-observability-metrics-producer:
    networks:
      bf-private-observability-backend:
        aliases: [metrics-producer]
  bf-private-observability-prometheus:
    networks:
      bf-private-observability-backend:
        aliases: [prometheus]' > "${alias_directory}/compose.yaml"
if observability_assert_output_aliases cli quadlet compose "${alias_directory}" bf-private 2> /dev/null; then
  printf '%s\n' 'A runtime container-ID alias satisfied the observability output contract.' >&2
  exit 1
fi

alias_directory="${test_root}/output-aliases-quadlet"
mkdir -p "${alias_directory}"
for service in loki metrics-producer prometheus; do
  printf '[Container]\nNetworkAlias=%s\nNetwork=bf-private-observability-backend.network\n' \
    "${service}" > "${alias_directory}/bf-private-observability-${service}.container"
done
printf '%s\n' \
  '[Container]' \
  'Network=bf-private-observability-backend.network' \
  'Network=bf-private-observability-edge' \
  > "${alias_directory}/bf-private-observability-grafana.container"
observability_assert_output_aliases cli podman quadlet "${alias_directory}" bf-private
grafana_quadlet="${alias_directory}/bf-private-observability-grafana.container"
cp -- "${grafana_quadlet}" "${grafana_quadlet}.valid"
for missing_network in backend edge; do
  cp -- "${grafana_quadlet}.valid" "${grafana_quadlet}"
  sed --in-place "/bf-private-observability-${missing_network}/d" "${grafana_quadlet}"
  if observability_assert_output_aliases cli podman quadlet "${alias_directory}" bf-private 2> /dev/null; then
    printf 'A Quadlet artifact without the Grafana %s network satisfied the alias contract.\n' \
      "${missing_network}" >&2
    exit 1
  fi
done
cp -- "${grafana_quadlet}.valid" "${grafana_quadlet}"

alias_directory="${test_root}/output-aliases-podman"
mkdir -p "${alias_directory}"
jq --null-input \
  --arg stem 'bf-private-observability-' \
  '{operations: [
    {action: "create", resource: {kind: "container", name: ($stem + "grafana")}, libpod: {body: {json: {Networks: {
      ($stem + "backend"): {aliases: ["grafana"]},
      ($stem + "edge"): {aliases: ["grafana"]}
    }}}}},
    {action: "create", resource: {kind: "container", name: ($stem + "loki")}, libpod: {body: {json: {Networks: {
      ($stem + "backend"): {aliases: ["loki"]}
    }}}}},
    {action: "create", resource: {kind: "container", name: ($stem + "metrics-producer")}, libpod: {body: {json: {Networks: {
      ($stem + "backend"): {aliases: ["metrics-producer"]}
    }}}}},
    {action: "create", resource: {kind: "container", name: ($stem + "prometheus")}, libpod: {body: {json: {Networks: {
      ($stem + "backend"): {aliases: ["prometheus"]}
    }}}}}
  ]}' > "${alias_directory}/podman.json"
observability_assert_output_aliases cli podman podman "${alias_directory}" bf-private

bfq_report="${test_root}/bfq-report.json"
printf '%s\n' '{"diagnostics":[{"code":"BFQ0003","severity":"warning","fields":[{"name":"subject","value":"services.bf-private-observability-grafana.networks"},{"name":"reason","value":"reviewed multi-network alias omission"}]}]}' \
  > "${bfq_report}"
observability_assert_reviewed_diagnostics \
  live-reimport cli exact compose quadlet bf-private-observability- "${bfq_report}"
jq '.diagnostics[0].fields += [{"name":"decision","value":"omitted"}]' \
  "${bfq_report}" > "${bfq_report}.unexpected-decision"
if observability_assert_reviewed_diagnostics \
  live-reimport cli exact compose quadlet bf-private-observability- \
  "${bfq_report}.unexpected-decision" > /dev/null 2>&1; then
  printf '%s\n' 'A Quadlet diagnostic with an invented decision field satisfied the contract.' >&2
  exit 1
fi

withheld_report="${test_root}/withheld-environment-report.json"
jq --null-input '{diagnostics: [
  {code: "BFQ0003", severity: "warning", fields: [{name: "subject", value: "services.grafana.environment.GF_PLUGINS_PREINSTALL_AUTO_UPDATE"}]},
  {code: "BFQ0003", severity: "warning", fields: [{name: "subject", value: "services.grafana.environment.GF_PLUGINS_PREINSTALL_DISABLED"}]},
  {code: "BFQ0003", severity: "warning", fields: [{name: "subject", value: "services.grafana.environment.GF_SECURITY_ADMIN_PASSWORD"}]}
]}' > "${withheld_report}"
observability_assert_reviewed_diagnostics offline-scenario cli exact compose quadlet '' "${withheld_report}"
for name in GF_PLUGINS_PREINSTALL_AUTO_UPDATE GF_PLUGINS_PREINSTALL_DISABLED; do
  jq --arg subject "services.grafana.environment.${name}" \
    '.diagnostics |= map(select(all(.fields[]; .name != "subject" or .value != $subject)))' \
    "${withheld_report}" > "${withheld_report}.missing"
  if observability_assert_reviewed_diagnostics offline-scenario cli exact compose quadlet '' \
    "${withheld_report}.missing" > /dev/null 2>&1; then
    printf 'A missing %s withholding diagnostic satisfied the exact route contract.\n' "${name}" >&2
    exit 1
  fi
done
printf '%s\n' '{"diagnostics":[]}' > "${withheld_report}.included"
observability_assert_reviewed_diagnostics offline-scenario compose exact compose quadlet '' "${withheld_report}.included"
if observability_assert_reviewed_diagnostics offline-scenario compose exact compose quadlet '' \
  "${withheld_report}" > /dev/null 2>&1; then
  printf '%s\n' 'Withholding diagnostics incorrectly satisfied the authored include route.' >&2
  exit 1
fi

current_case="${test_root}/export-paths"
observability_prepare_export_paths cli exact
[[ -d "${current_case}/outputs/cli-exact" ]]
[[ -d "${current_case}/reimports" ]]
for output in compose quadlet podman; do
  [[ ! -e "${current_case}/outputs/cli-exact/${output}" ]]
done
for selection in label all; do
  observability_prepare_export_paths cli "${selection}"
  [[ -d "${current_case}/outputs/cli-${selection}" ]]
  for output in compose quadlet podman; do
    [[ ! -e "${current_case}/outputs/cli-${selection}/${output}" ]]
  done
done

current_case="${test_root}/regular-file-parent"
mkdir -p "${current_case}/outputs"
printf '%s\n' 'not-a-directory' > "${current_case}/outputs/cli-exact"
status=0
error="$(observability_prepare_export_paths cli exact 2>&1)" || status=$?
[[ "${status}" == 1 ]]
[[ "${error}" == "Observability export parent is not a directory: ${current_case}/outputs/cli-exact" ]]

current_case="${test_root}/pre-existing-export-target"
mkdir -p "${current_case}/outputs/cli-exact/compose"
status=0
error="$(observability_run_exports cli /tmp/observability.sock bf-private 2>&1)" || status=$?
[[ "${status}" == 1 ]]
[[ "${error}" == "Observability export target must not exist before conversion: ${current_case}/outputs/cli-exact/compose" ]]

assert_compose_external_edge() {
  local name=$1 expected_status=$2 content=$3 directory
  directory="${test_root}/compose-external-edge-${name}"
  mkdir -p "${directory}"
  printf '%s\n' "${content}" > "${directory}/compose.yaml"
  status=0
  observability_assert_external_edge compose "${directory}" bf-private-observability-edge || status=$?
  [[ "${status}" == "${expected_status}" ]]
}

assert_compose_external_edge key-only-success 0 'services:
  grafana:
    image: example.invalid/grafana
networks:
  bf-private-observability-edge:
    external: true
  unrelated-external-network:
    external: true
volumes:
  grafana-data: {}'
assert_compose_external_edge matching-name-success 0 'networks:
  backend:
    name: bf-private-observability-backend
    internal: true
  edge:
    name: bf-private-observability-edge
    external: true
services:
  grafana:
    image: example.invalid/grafana'
assert_compose_external_edge unrelated-mapping-failure 1 'networks:
  unrelated-external-network:
    external: true'
assert_compose_external_edge split-evidence-failure 1 'networks:
  named-but-internal:
    name: bf-private-observability-edge
    internal: true
  unrelated-external-network:
    external: true'
assert_compose_external_edge mismatched-name-failure 1 'networks:
  bf-private-observability-edge:
    name: wrong-network
    external: true'

export_mode=
export_operation_count=0
reimport_count=0
declare -A export_seen=() reimport_seen=()
boxferry_operation() {
  local description=$1 operation_label mode selection target output directory='' argument
  local include_values=false
  shift
  read -r operation_label mode selection target <<< "${description}"
  [[ "${operation_label}" == Observability && "${mode}" == "${export_mode}" ]]
  [[ "$1" == convert && "$2" == podman ]]
  output=$3
  case "${description}" in
    "Observability ${export_mode} exact Podman-to-${output}" | \
      "Observability ${export_mode} label Podman-to-${output}" | \
      "Observability ${export_mode} all Podman-to-${output}" | \
      "Observability ${export_mode} default-withheld Podman-to-${output}") ;;
    *)
      printf 'Unexpected modeled observability export operation: %s\n' "${description}" >&2
      return 1
      ;;
  esac
  while (($#)); do
    argument=$1
    shift
    if [[ "${argument}" == --output-directory ]]; then
      directory=${1:?output directory argument is required}
      shift
    elif [[ "${argument}" == --environment-values ]]; then
      [[ "${1:-}" == include ]]
      include_values=true
      shift
    fi
  done
  if [[ "${selection}" == default-withheld || "${output}" == podman ]]; then
    [[ "${include_values}" == false ]]
  else
    [[ "${include_values}" == true ]]
  fi
  [[ -n "${directory}" ]]
  [[ -d "$(dirname -- "${directory}")" ]]
  [[ ! -e "${directory}" ]]
  mkdir -- "${directory}"
  if [[ "${selection}" == default-withheld ]]; then
    if [[ "${output}" == compose ]]; then
      printf '%s\n' 'services:' '  bf-private-observability-grafana:' \
        > "${directory}/compose.yaml"
    else
      printf '%s\n' '[Container]' 'Image=example.invalid/grafana:1' \
        > "${directory}/bf-private-observability-grafana.container"
    fi
  fi
  export_seen["${mode}-${selection}-${output}"]=$((export_seen["${mode}-${selection}-${output}"] + 1))
  ((export_operation_count += 1))
  if [[ "${selection}" == default-withheld ]]; then
    printf '%s\n' '{"schema_version":1,"status":"success","exit_category":"success","diagnostics":[{"code":"BFP0002","severity":"warning","fields":[{"name":"subject","value":"services.bf-private-observability-grafana.environment.GF_SECURITY_ADMIN_PASSWORD"}]}],"fidelity":{"invalid":0},"output_artifacts":["generated"]}'
  else
    printf '%s\n' '{"schema_version":1,"status":"success","exit_category":"success","diagnostics":[],"fidelity":{"invalid":0},"output_artifacts":["generated"]}'
  fi
}
observability_assert_output_membership() { :; }
observability_assert_output_semantics() { :; }
observability_assert_external_edge() { :; }
observability_run_reimports() {
  local mode=$1 selection=$2 source=$3 prefix=$4
  [[ "${mode}" == "${export_mode}" ]]
  [[ "${source}" == "${current_case}/outputs/${mode}-${selection}" ]]
  [[ -d "${current_case}/reimports" ]]
  [[ "${prefix}" == bf-private ]]
  reimport_seen["${mode}-${selection}"]=$((reimport_seen["${mode}-${selection}"] + 1))
  ((reimport_count += 1))
}

for export_mode in cli compose; do
  current_case="${test_root}/modeled-${export_mode}-exports"
  observability_run_exports "${export_mode}" /tmp/observability.sock bf-private
done
[[ "${export_operation_count}" == 22 ]]
[[ "${reimport_count}" == 6 ]]
for mode in cli compose; do
  for output in compose quadlet; do
    [[ "${export_seen["${mode}-default-withheld-${output}"]}" == 1 ]]
  done
  for selection in exact label all; do
    [[ "${reimport_seen["${mode}-${selection}"]}" == 1 ]]
    for output in compose quadlet podman; do
      [[ "${export_seen["${mode}-${selection}-${output}"]}" == 1 ]]
    done
  done
done

prometheus_flags='{
  "status": "success",
  "data": {
    "storage.tsdb.retention.time": "1d",
    "web.enable-remote-write-receiver": "true"
  }
}'
observability_validate_prometheus_flags "${prometheus_flags}"

for invalid_flags in \
  '{"status":"success","data":{"storage.tsdb.retention.time":"24h","web.enable-remote-write-receiver":"true"}}' \
  '{"status":"success","data":{"storage.tsdb.retention.time":"12h","web.enable-remote-write-receiver":"true"}}' \
  '{"data":{"storage.tsdb.retention.time":"1d","web.enable-remote-write-receiver":"true"}}' \
  '{"status":"success"}' \
  '{"status":"success","data":{"web.enable-remote-write-receiver":"true"}}' \
  '{"status":"success","data":{"storage.tsdb.retention.time":"1d"}}' \
  '{"status":true,"data":{"storage.tsdb.retention.time":"1d","web.enable-remote-write-receiver":"true"}}' \
  '{"status":"success","data":true}' \
  '{"status":"success","data":{"storage.tsdb.retention.time":true,"web.enable-remote-write-receiver":"true"}}' \
  '{"status":"success","data":{"storage.tsdb.retention.time":"1d","web.enable-remote-write-receiver":true}}' \
  '{"status":"error","data":{"storage.tsdb.retention.time":"1d","web.enable-remote-write-receiver":"true"}}'; do
  if observability_validate_prometheus_flags "${invalid_flags}"; then
    printf '%s\n' 'Invalid Prometheus flags unexpectedly satisfied the observability contract.' >&2
    exit 1
  fi
done

write_timing_fixture() {
  local path=$1 interval=$2 timeout=${3:-}
  {
    printf '%s\n' 'prometheus.scrape "boxferry_fixture" {'
    printf '  scrape_interval = "%ss"\n' "${interval}"
    if [[ -n "${timeout}" ]]; then
      printf '  scrape_timeout = "%ss"\n' "${timeout}"
    fi
    printf '%s\n' '}'
  } > "${path}"
}

for invalid in missing equal greater; do
  case "${invalid}" in
    missing) write_timing_fixture "${test_root}/${invalid}.alloy" 2 ;;
    equal) write_timing_fixture "${test_root}/${invalid}.alloy" 2 2 ;;
    greater) write_timing_fixture "${test_root}/${invalid}.alloy" 2 3 ;;
  esac
  status=0
  error="$(observability_validate_alloy_scrape_timing \
    "${test_root}/${invalid}.alloy" 2>&1)" || status=$?
  [[ "${status}" == 1 ]]
  [[ "${error}" == 'Observability Alloy scrape timing requires one positive timeout strictly below its interval.' ]]
done

diagnostic_mode=all-running
observability_remote() {
  local socket=$1
  shift
  [[ "${socket}" == /tmp/observability.sock ]]
  [[ "$1" == inspect && "$2" == --format ]]

  case "${diagnostic_mode}:$4" in
    alloy-failed:bf-private-observability-alloy)
      printf '%s\n' \
        "${OBSERVABILITY_ADMIN_PASSWORD} /run/user/1000/podman.sock 10.88.0.9 runtime-deadbeef" >&2
      printf '%s\n' 'false 1 false'
      ;;
    multiple-failed:bf-private-observability-metrics-producer | \
      multiple-failed:bf-private-observability-alloy)
      printf '%s\n' 'false 1 false'
      ;;
    unavailable:*)
      printf '%s\n' \
        "${OBSERVABILITY_ADMIN_PASSWORD} /run/user/1000/podman.sock 10.88.0.9 runtime-deadbeef"
      ;;
    *) printf '%s\n' 'true 0 false' ;;
  esac
}

diagnostic_mode=alloy-failed
if observability_pipeline_roles_running /tmp/observability.sock bf-private; then
  printf '%s\n' 'Stopped Alloy unexpectedly satisfied the running-role contract.' >&2
  exit 1
fi

diagnostics="$(observability_report_pipeline_states \
  /tmp/observability.sock bf-private 2>&1)"
expected_diagnostics="$(
  cat << 'EOF'
OBSERVABILITY DIAGNOSTIC role=metrics-producer running=true exit-code=0 oom-killed=false
OBSERVABILITY DIAGNOSTIC role=alloy running=false exit-code=1 oom-killed=false
OBSERVABILITY DIAGNOSTIC role=prometheus running=true exit-code=0 oom-killed=false
OBSERVABILITY DIAGNOSTIC role=log-producer running=true exit-code=0 oom-killed=false
OBSERVABILITY DIAGNOSTIC role=loki running=true exit-code=0 oom-killed=false
OBSERVABILITY DIAGNOSTIC role=grafana running=true exit-code=0 oom-killed=false
OBSERVABILITY DIAGNOSTIC first-failed-hop=alloy-process
EOF
)"
[[ "${diagnostics}" == "${expected_diagnostics}" ]]
[[ "$(printf '%s\n' "${diagnostics}" | wc -l)" == 7 ]]
[[ "$(printf '%s' "${diagnostics}" | wc -c)" -le 768 ]]
for forbidden in \
  "${OBSERVABILITY_ADMIN_PASSWORD}" \
  /run/user/1000/podman.sock \
  10.88.0.9 \
  runtime-deadbeef \
  bf-private; do
  assert_absent "${forbidden}" "${diagnostics}"
done

diagnostic_mode=all-running
observability_pipeline_roles_running /tmp/observability.sock bf-private

diagnostic_mode=multiple-failed
diagnostics="$(observability_report_pipeline_states \
  /tmp/observability.sock bf-private 2>&1)"
grep --fixed-strings --quiet \
  'OBSERVABILITY DIAGNOSTIC first-failed-hop=metrics-producer-process' <<< "${diagnostics}"

diagnostic_mode=unavailable
diagnostics="$(observability_report_pipeline_states \
  /tmp/observability.sock bf-private 2>&1)"
[[ "$(printf '%s\n' "${diagnostics}" | wc -l)" == 7 ]]
grep --fixed-strings --quiet \
  'OBSERVABILITY DIAGNOSTIC first-failed-hop=unknown' <<< "${diagnostics}"
for forbidden in \
  "${OBSERVABILITY_ADMIN_PASSWORD}" \
  /run/user/1000/podman.sock \
  10.88.0.9 \
  runtime-deadbeef \
  bf-private; do
  assert_absent "${forbidden}" "${diagnostics}"
done

persistence_prometheus_calls=0
persistence_loki_calls=0
persistence_grafana_calls=0
persistence_log_producer_marker="${test_root}/persistence-log-producer-called"
observability_prometheus_has_value() {
  [[ "$1" == /tmp/observability.sock ]]
  [[ "$2" == bf-private ]]
  [[ "$3" == 'max_over_time%28boxferry_fixture_temperature_celsius%7Bsource%3D%22controlled%22%7D%5B30m%5D%29' ]]
  [[ "$4" == 84 ]]
  ((persistence_prometheus_calls += 1))
}
observability_loki_has_known_log() {
  [[ "$1" == /tmp/observability.sock ]]
  [[ "$2" == bf-private ]]
  ((persistence_loki_calls += 1))
}
observability_remote() {
  local socket=$1
  shift
  [[ "${socket}" == /tmp/observability.sock ]]
  case "$2" in
    bf-private-observability-grafana)
      [[ "$1" == exec ]]
      [[ $# == 6 ]]
      [[ "$3" == grep ]]
      [[ "$4" == -Fx ]]
      [[ "$5" == boxferry-grafana-persisted ]]
      [[ "$6" == /var/lib/grafana/boxferry-persistence-marker ]]
      ((persistence_grafana_calls += 1))
      ;;
    bf-private-observability-log-producer)
      [[ "$1" == exec ]]
      [[ $# == 5 ]]
      [[ "$3" == wc ]]
      [[ "$4" == -l ]]
      [[ "$5" == /var/log/boxferry/telemetry.log ]]
      touch "${persistence_log_producer_marker}"
      printf '%s\n' 1
      ;;
    *)
      printf 'Unexpected persistence remote target: %s\n' "$2" >&2
      return 1
      ;;
  esac
}
observability_assert_persistence /tmp/observability.sock bf-private
[[ "${persistence_prometheus_calls}" == 1 ]]
[[ "${persistence_loki_calls}" == 1 ]]
[[ "${persistence_grafana_calls}" == 1 ]]
[[ -f "${persistence_log_producer_marker}" ]]

printf '%s\n' 'Observability Alloy timing, persistence, and bounded diagnostics tests passed.'
