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
paperless_assert_output_membership() { :; }
paperless_assert_output_semantics() { :; }
immich_assert_output_membership() { :; }
immich_assert_output_semantics() { :; }
privacy_assertions=0
application_assert_default_withholding() { privacy_assertions=$((privacy_assertions + 1)); }
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
