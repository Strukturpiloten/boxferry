#!/usr/bin/env bash

# Exercise the format/lint command closure without invoking compilers or linters.
set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
bash_executable="$(command -v bash)"
test_root="$(mktemp -d)"
trap 'rm -r -- "${test_root}"' EXIT

mkdir -p "${test_root}/repository/scripts" "${test_root}/bin"
cp -- "${script_directory}/format-lint.sh" "${test_root}/repository/scripts/format-lint.sh"
cp -- "${script_directory}/check-files.sh" "${test_root}/repository/scripts/check-files.sh"
printf '# Fixture\n' > "${test_root}/repository/README.md"

cat > "${test_root}/bin/mock" << 'MOCK'
#!/usr/bin/env bash
set -Eeuo pipefail
command_name="${0##*/}"
{
  printf '%s' "${command_name}"
  if (("$#" != 0)); then
    printf ' %q' "$@"
  fi
  printf ' [jobs=%s]\n' "${CARGO_BUILD_JOBS:-unset}"
} >> "${FORMAT_LINT_TEST_LOG}"
if [[ "${command_name}" == "${FORMAT_LINT_TEST_FAIL:-}" ]]; then
  exit 17
fi
if [[ "${command_name}" == git && "${1:-}" == ls-files ]]; then
  printf 'README.md\0'
fi
MOCK
chmod +x "${test_root}/bin/mock"

for tool in actionlint cargo git hadolint markdownlint-cli2 prettier shellcheck shfmt tombi zizmor; do
  ln -s "${test_root}/bin/mock" "${test_root}/bin/${tool}"
done

run_quality() {
  local label=$1
  shift
  PATH="${test_root}/bin:${PATH}" \
    FORMAT_LINT_TEST_LOG="${test_root}/${label}.commands" \
    BOXFERRY_LINT_JOBS="${BOXFERRY_LINT_JOBS:-}" \
    CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-}" \
    "${bash_executable}" "${test_root}/repository/scripts/format-lint.sh" "$@" \
    > "${test_root}/${label}.output" 2>&1
}

unset BOXFERRY_LINT_JOBS CARGO_BUILD_JOBS
run_quality default
run_quality check --check

grep --fixed-strings --quiet 'cargo fmt --all [jobs=unset]' "${test_root}/default.commands"
grep --fixed-strings --quiet '+ bash scripts/check-files.sh --fix' "${test_root}/default.output"
grep --fixed-strings --quiet 'cargo fmt --all -- --check [jobs=unset]' "${test_root}/check.commands"
grep --fixed-strings --quiet '+ bash scripts/check-files.sh --check' "${test_root}/check.output"
grep --fixed-strings --quiet 'git --no-pager diff --check' "${test_root}/default.commands"
grep --fixed-strings --quiet 'git --no-pager diff --cached --check' \
  "${test_root}/default.commands"
grep --fixed-strings --quiet 'actionlint [jobs=unset]' "${test_root}/default.commands"
grep --fixed-strings --quiet 'zizmor .github/workflows [jobs=unset]' "${test_root}/default.commands"
grep --fixed-strings --quiet 'cargo ci-clippy [jobs=2]' "${test_root}/default.commands"
grep --extended-regexp --quiet '^\[07/07\] PASS Run Clippy without executing tests \([0-9]+s\)$' \
  "${test_root}/default.output"
grep --extended-regexp --quiet \
  '^BoxFerry format/lint passed all 7 steps in [0-9]+s; no tests were executed\.$' \
  "${test_root}/default.output"

export BOXFERRY_LINT_JOBS=1
run_quality one_job --check
grep --fixed-strings --quiet 'cargo ci-clippy [jobs=1]' "${test_root}/one_job.commands"
unset BOXFERRY_LINT_JOBS
export CARGO_BUILD_JOBS=3
run_quality inherited_jobs --check
grep --fixed-strings --quiet 'cargo ci-clippy [jobs=3]' "${test_root}/inherited_jobs.commands"
unset CARGO_BUILD_JOBS

for invalid in --unknown --tests; do
  status=0
  run_quality invalid "${invalid}" || status=$?
  [[ "${status}" == 2 && ! -s "${test_root}/invalid.commands" ]]
done
status=0
run_quality extra --check --fix || status=$?
[[ "${status}" == 2 && ! -s "${test_root}/extra.commands" ]]

for invalid_jobs in 0 -1 two '1.5'; do
  export BOXFERRY_LINT_JOBS="${invalid_jobs}"
  status=0
  run_quality invalid_jobs --check || status=$?
  [[ "${status}" == 2 && ! -s "${test_root}/invalid_jobs.commands" ]]
done
unset BOXFERRY_LINT_JOBS

export FORMAT_LINT_TEST_FAIL=actionlint
status=0
run_quality failure --check || status=$?
[[ "${status}" == 17 ]]
[[ "$(tail -n 1 "${test_root}/failure.commands")" == actionlint* ]]
grep --fixed-strings --quiet '[05/07] FAIL Lint GitHub Actions syntax after' \
  "${test_root}/failure.output"
unset FORMAT_LINT_TEST_FAIL

if grep --extended-regexp --quiet '(ci-test|cargo test|test-[^ ]*\.sh|migration-readiness|llvm-cov|ci-policy|ci-doc|semver|podman-live)' "${test_root}/default.commands" "${test_root}/check.commands"; then
  printf 'Format/lint command unexpectedly dispatched a test or complete-gate command.\n' >&2
  exit 1
fi

printf 'Format/lint command regression tests passed.\n'
