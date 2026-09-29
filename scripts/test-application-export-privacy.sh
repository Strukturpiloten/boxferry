#!/usr/bin/env bash
# Default withholding and explicit fixture-value consent stay separate contracts.
set -Eeuo pipefail
script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=scripts/lib/application-export-privacy.sh
source "${script_directory}/lib/application-export-privacy.sh"
test_root="$(mktemp -d)"
trap 'rm -r -- "${test_root}"' EXIT
directory="${test_root}/export"
mkdir -p -- "${directory}"
printf '%s\n' 'services:' '  application: {}' > "${directory}/compose.yaml"
report="${test_root}/report.json"
subject='services.application.environment.PASSWORD'
canary='public-fixture-environment-canary'
jq --null-input --arg subject "${subject}" '
  {schema_version: 1, status: "success", exit_category: "success",
   fidelity: {invalid: 0}, output_artifacts: [{name: "compose.yaml"}],
   diagnostics: [{code: "BFP0002", severity: "warning", fields: [
     {name: "subject", value: $subject}, {name: "decision", value: "omitted"},
     {name: "required_loss_policy", value: "partial"}]}]}
' > "${report}"
application_assert_default_withholding "${directory}" "${report}" "${subject}" "${canary}"
for mutation in \
  '.diagnostics = []' \
  '.diagnostics[0].fields[0].value = "wrong-subject"' \
  '.diagnostics[0].fields[1].value = "reconstructed"' \
  '.diagnostics[0].fields[2].value = "exact"' \
  '.diagnostics += .diagnostics' \
  '.fidelity.invalid = 1' \
  '.output_artifacts = []' \
  '.status = "failure"'; do
  jq "${mutation}" "${report}" > "${test_root}/invalid.json"
  if application_assert_default_withholding \
    "${directory}" "${test_root}/invalid.json" "${subject}" "${canary}"; then
    printf 'Default withholding accepted invalid prerequisite evidence.\n' >&2
    exit 1
  fi
done
for leak in artifact report; do
  if [[ "${leak}" == artifact ]]; then
    printf '%s\n' "${canary}" > "${directory}/leak.txt"
    candidate_report="${report}"
  else
    jq --arg canary "${canary}" '.unintended_value = $canary' "${report}" > "${test_root}/leak.json"
    candidate_report="${test_root}/leak.json"
  fi
  if application_assert_default_withholding \
    "${directory}" "${candidate_report}" "${subject}" "${canary}" > /dev/null 2>&1; then
    printf 'Default withholding accepted a leaked canary.\n' >&2
    exit 1
  fi
  [[ "${leak}" != artifact ]] || rm -- "${directory}/leak.txt"
done
mkdir -p -- "${test_root}/empty"
if application_assert_default_withholding \
  "${test_root}/empty" "${report}" "${subject}" "${canary}"; then
  printf 'Default withholding accepted an empty artifact directory.\n' >&2
  exit 1
fi

# Check actual adapter command construction without a daemon. Existing semantic
# assertions still run in live suites; these stubs isolate the consent boundary.
# shellcheck source=scripts/lib/paperless-application.sh
source "${script_directory}/lib/paperless-application.sh"
# shellcheck source=scripts/lib/immich-application.sh
source "${script_directory}/lib/immich-application.sh"
# Conversion failures must not echo raw, malformed, or canary-bearing reports.
failure_report="${test_root}/immich-failure.json"
jq --null-input --arg db "${IMMICH_DB_PASSWORD}" '
  {schema_version: 1, status: "failure", diagnostics: [{message: $db}]}
' > "${failure_report}"
failure_output="$(immich_report_conversion_failure "${failure_report}" 2>&1)"
[[ "${failure_output}" == *'report withheld'* &&
  "${failure_output}" != *"${IMMICH_DB_PASSWORD}"* ]]
printf '{"message":"%s"' "${IMMICH_ADMIN_PASSWORD}" > "${failure_report}"
failure_output="$(immich_report_conversion_failure "${failure_report}" 2>&1)"
[[ "${failure_output}" == *'report withheld'* &&
  "${failure_output}" != *"${IMMICH_ADMIN_PASSWORD}"* ]]
jq --null-input '{schema_version: 1, status: "failure", diagnostics: [{code: "BFTEST"}]}' \
  > "${failure_report}"
failure_output="$(immich_report_conversion_failure "${failure_report}" 2>&1)"
[[ "${failure_output}" == *'"status": "failure"'* &&
  "${failure_output}" == *'"code": "BFTEST"'* ]]

# The Paperless broker password is a separate protected live environment value.
# Mutating a default-withheld artifact to contain it must fail privacy validation.
application_assert_default_withholding \
  "${directory}" "${report}" "${subject}" "${canary}" "${PAPERLESS_REDIS_PASSWORD}"
printf '%s\n' "${PAPERLESS_REDIS_PASSWORD}" > "${directory}/broker-leak.txt"
if application_assert_default_withholding \
  "${directory}" "${report}" "${subject}" "${canary}" "${PAPERLESS_REDIS_PASSWORD}" \
  > /dev/null 2>&1; then
  printf 'Default withholding accepted a leaked Paperless broker password.\n' >&2
  exit 1
fi
rm -- "${directory}/broker-leak.txt"

# Only the authored broker command and healthcheck may retain this canary.
broker_directory="${test_root}/broker-export"
mkdir -p -- "${broker_directory}"
printf '%s\n' \
  'services:' \
  '  fixture-paper-broker:' \
  '    container_name: fixture-paper-broker' \
  "    command: [valkey-server, --requirepass, ${PAPERLESS_REDIS_PASSWORD}]" \
  '    healthcheck:' \
  "      test: [CMD, valkey-cli, -a, ${PAPERLESS_REDIS_PASSWORD}, ping]" \
  '  fixture-paper-web:' \
  '    environment: {}' > "${broker_directory}/compose.yaml"
paperless_assert_withheld_broker_command \
  "${broker_directory}" "${report}" compose fixture
cp -- "${broker_directory}/compose.yaml" "${test_root}/valid-broker-compose.yaml"
for mutation in missing-command missing-health misplaced-command misplaced-health; do
  cp -- "${test_root}/valid-broker-compose.yaml" "${broker_directory}/compose.yaml"
  case "${mutation}" in
    missing-command) sed -i '/^    command:/d' "${broker_directory}/compose.yaml" ;;
    missing-health) sed -i '/^    healthcheck:/,+1d' "${broker_directory}/compose.yaml" ;;
    misplaced-command)
      sed -i "s/--requirepass, ${PAPERLESS_REDIS_PASSWORD}/${PAPERLESS_REDIS_PASSWORD}, --requirepass/" \
        "${broker_directory}/compose.yaml"
      ;;
    misplaced-health)
      sed -i "s/-a, ${PAPERLESS_REDIS_PASSWORD}/${PAPERLESS_REDIS_PASSWORD}, -a/" \
        "${broker_directory}/compose.yaml"
      ;;
  esac
  if paperless_assert_withheld_broker_command \
    "${broker_directory}" "${report}" compose fixture > /dev/null 2>&1; then
    printf 'Paperless broker check accepted %s in Compose.\n' "${mutation}" >&2
    exit 1
  fi
done
cp -- "${test_root}/valid-broker-compose.yaml" "${broker_directory}/compose.yaml"
printf '    environment: {PAPERLESS_REDIS: %s}\n' "${PAPERLESS_REDIS_PASSWORD}" \
  >> "${broker_directory}/compose.yaml"
if paperless_assert_withheld_broker_command \
  "${broker_directory}" "${report}" compose fixture > /dev/null 2>&1; then
  printf 'Paperless broker check accepted a leaked Compose environment value.\n' >&2
  exit 1
fi
printf '%s\n' \
  '[Container]' \
  'ContainerName=fixture-paper-broker' \
  "Exec=valkey-server --requirepass ${PAPERLESS_REDIS_PASSWORD}" \
  "HealthCmd=[\"CMD\",\"valkey-cli\",\"-a\",\"${PAPERLESS_REDIS_PASSWORD}\",\"ping\"]" \
  > "${broker_directory}/fixture-paper-broker.container"
rm -- "${broker_directory}/compose.yaml"
paperless_assert_withheld_broker_command \
  "${broker_directory}" "${report}" quadlet fixture
cp -- "${broker_directory}/fixture-paper-broker.container" \
  "${test_root}/valid-broker-quadlet.container"
for mutation in missing-command missing-health misplaced-command misplaced-health; do
  cp -- "${test_root}/valid-broker-quadlet.container" \
    "${broker_directory}/fixture-paper-broker.container"
  case "${mutation}" in
    missing-command)
      sed -i '/^Exec=/d' "${broker_directory}/fixture-paper-broker.container"
      ;;
    missing-health)
      sed -i '/^HealthCmd=/d' "${broker_directory}/fixture-paper-broker.container"
      ;;
    misplaced-command)
      sed -i "s/--requirepass ${PAPERLESS_REDIS_PASSWORD}/${PAPERLESS_REDIS_PASSWORD} --requirepass/" \
        "${broker_directory}/fixture-paper-broker.container"
      ;;
    misplaced-health)
      sed -i "s/\\\"-a\\\",\\\"${PAPERLESS_REDIS_PASSWORD}\\\"/\\\"${PAPERLESS_REDIS_PASSWORD}\\\",\\\"-a\\\"/" \
        "${broker_directory}/fixture-paper-broker.container"
      ;;
  esac
  if paperless_assert_withheld_broker_command \
    "${broker_directory}" "${report}" quadlet fixture > /dev/null 2>&1; then
    printf 'Paperless broker check accepted %s in Quadlet.\n' "${mutation}" >&2
    exit 1
  fi
done
cp -- "${test_root}/valid-broker-quadlet.container" \
  "${broker_directory}/fixture-paper-broker.container"
printf 'Environment=PAPERLESS_REDIS=%s\n' "${PAPERLESS_REDIS_PASSWORD}" \
  >> "${broker_directory}/fixture-paper-broker.container"
if paperless_assert_withheld_broker_command \
  "${broker_directory}" "${report}" quadlet fixture > /dev/null 2>&1; then
  printf 'Paperless broker check accepted a leaked Quadlet environment value.\n' >&2
  exit 1
fi

# A failed early member must survive a caller's conditional/errexit-disabled context.
member_calls=0
resource_calls=0
assert_named_member() {
  member_calls=$((member_calls + 1))
  [[ "${member_calls}" != 1 ]]
}
assert_resource_member() { resource_calls=$((resource_calls + 1)); }
for application in paperless immich; do
  member_calls=0
  resource_calls=0
  if "${application}_assert_output_membership" exact compose "${directory}" fixture; then
    printf '%s output membership ignored the first missing member.\n' "${application}" >&2
    exit 1
  fi
  [[ "${member_calls}" == 1 && "${resource_calls}" == 0 ]]
done

paperless_assert_output_membership() { :; }
paperless_assert_output_semantics() { :; }
immich_assert_output_membership() { :; }
immich_assert_output_semantics() { :; }
privacy_assertions=0
application_assert_default_withholding() {
  privacy_assertions=$((privacy_assertions + 1))
  local subject=$3
  shift 3
  if [[ "${subject}" == *paper-web* ]]; then
    [[ $# == 3 && "$1" == "${PAPERLESS_ADMIN_PASSWORD}" &&
      "$2" == "${PAPERLESS_DB_PASSWORD}" &&
      "$3" == "${PAPERLESS_SECRET_KEY}" ]]
  else
    [[ $# == 1 && "$1" == "${IMMICH_DB_PASSWORD}" ]]
  fi
}
paperless_assert_withheld_broker_command() { :; }
calls=0
withheld_calls=0
included_calls=0
podman_calls=0
boxferry_operation() {
  local description=$1 output=$4 argument previous='' consent='' consent_count=0
  shift
  [[ "$1 $2" == 'convert podman' ]] || return 89
  for argument in "$@"; do
    if [[ "${previous}" == --environment-values ]]; then
      consent="${argument}"
      consent_count=$((consent_count + 1))
    fi
    previous="${argument}"
  done
  if [[ "${description}" == *' default-withheld '* ]]; then
    [[ "${output}" != podman && "${consent_count}" == 0 ]] || return 89
    withheld_calls=$((withheld_calls + 1))
  elif [[ "${output}" == podman ]]; then
    [[ "${consent_count}" == 0 ]] || return 89
    podman_calls=$((podman_calls + 1))
  else
    [[ "${consent}" == include && "${consent_count}" == 1 ]] || return 89
    included_calls=$((included_calls + 1))
  fi
  calls=$((calls + 1))
  printf '%s\n' '{"schema_version":1,"status":"success","exit_category":"success","fidelity":{"invalid":0},"diagnostics":[],"output_artifacts":[{"name":"fixture"}]}'
}
for application in paperless immich; do
  for mode in cli compose; do
    current_case="${test_root}/${application}-${mode}"
    "${application}_run_exports" "${mode}" socket fixture
  done
done
[[ "${calls}" == 44 && "${withheld_calls}" == 8 && "${included_calls}" == 24 && "${podman_calls}" == 12 ]]
[[ "${privacy_assertions}" == 8 ]]
