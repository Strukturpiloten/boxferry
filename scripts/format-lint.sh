#!/usr/bin/env bash

set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repository_root="$(cd -- "${script_directory}/.." && pwd -P)"
readonly repository_root
cd -- "${repository_root}"

mode="${1:---fix}"
if [[ "$#" -gt 1 || ("${mode}" != "--check" && "${mode}" != "--fix") ]]; then
  printf 'Usage: %s [--check|--fix]\n' "$0" >&2
  exit 2
fi
readonly mode

cargo_jobs="${BOXFERRY_LINT_JOBS:-${CARGO_BUILD_JOBS:-2}}"
if [[ ! "${cargo_jobs}" =~ ^[1-9][0-9]*$ ]]; then
  printf 'BOXFERRY_LINT_JOBS must be a positive integer; got %q.\n' "${cargo_jobs}" >&2
  exit 2
fi
readonly cargo_jobs

required_tools=(actionlint bash cargo git zizmor)
missing_tools=()
for tool in "${required_tools[@]}"; do
  if ! command -v "${tool}" > /dev/null 2>&1; then
    missing_tools+=("${tool}")
  fi
done
if (("${#missing_tools[@]}" != 0)); then
  printf -v missing_list ' %s' "${missing_tools[@]}"
  printf 'BoxFerry format/lint missing required tool(s):%s. Use the Dev Container.\n' \
    "${missing_list}" >&2
  exit 2
fi

step=0
readonly total_steps=7
started_at="${SECONDS}"

run_step() {
  local label=$1
  shift
  local step_started status elapsed
  step=$((step + 1))
  step_started="${SECONDS}"
  printf '\n[%02d/%02d] %s\n +' "${step}" "${total_steps}" "${label}"
  printf ' %q' "$@"
  printf '\n'
  if "$@"; then
    elapsed=$((SECONDS - step_started))
    printf '[%02d/%02d] PASS %s (%ss)\n' \
      "${step}" "${total_steps}" "${label}" "${elapsed}"
    return
  else
    status=$?
  fi
  elapsed=$((SECONDS - step_started))
  printf '[%02d/%02d] FAIL %s after %ss (status %d)\n' \
    "${step}" "${total_steps}" "${label}" "${elapsed}" "${status}" >&2
  return "${status}"
}

printf 'Using at most %s Cargo build jobs for linting.\n' "${cargo_jobs}"

if [[ "${mode}" == "--check" ]]; then
  run_step "Check Rust formatting" cargo fmt --all -- --check
  run_step "Check non-Rust formatting and lint" bash scripts/check-files.sh --check
else
  run_step "Format Rust" cargo fmt --all
  run_step "Format and lint non-Rust files" bash scripts/check-files.sh --fix
fi
run_step "Check unstaged whitespace errors" git --no-pager diff --check
run_step "Check staged whitespace errors" git --no-pager diff --cached --check
run_step "Lint GitHub Actions syntax" actionlint
run_step "Lint GitHub Actions security" zizmor .github/workflows
run_step "Run Clippy without executing tests" \
  env CARGO_BUILD_JOBS="${cargo_jobs}" cargo ci-clippy

printf '\nBoxFerry format/lint passed all %d steps in %ss; no tests were executed.\n' \
  "${total_steps}" "$((SECONDS - started_at))"
