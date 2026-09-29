#!/usr/bin/env bash
# Shared default-withholding assertions for synthetic application exports.

application_assert_default_withholding() {
  local directory=$1 report=$2 subject=$3 canary status
  shift 3
  [[ -d "${directory}" && -f "${report}" && $# -gt 0 ]] || return 1
  [[ -n "$(find "${directory}" -type f -print -quit)" ]] || return 1
  jq --exit-status --arg subject "${subject}" '
    .schema_version == 1 and .status == "success" and .exit_category == "success" and
    ((.fidelity.invalid // 0) == 0) and (.output_artifacts | length > 0) and
    ([.diagnostics[]? | select(.severity == "error")] | length) == 0 and
    ([.diagnostics[]? | select(.code == "BFP0002" and .severity == "warning" and
      any(.fields[]?; .name == "subject" and .value == $subject) and
      any(.fields[]?; .name == "decision" and .value == "omitted") and
      any(.fields[]?; .name == "required_loss_policy" and .value == "partial"))] | length) == 1
  ' "${report}" > /dev/null || return 1
  for canary in "$@"; do
    [[ -n "${canary}" ]] || return 1
    if grep --recursive --fixed-strings --quiet -- "${canary}" "${directory}" "${report}"; then
      printf 'Default-withheld application export retained an environment canary.\n' >&2
      return 1
    else
      status=$?
      [[ "${status}" == 1 ]] || return "${status}"
    fi
  done
}
