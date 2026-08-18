#!/usr/bin/env bash

set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repository_root="$(cd -- "${script_directory}/.." && pwd -P)"
readonly repository_root

cd -- "${repository_root}"

current_step="preflight"
step=0
readonly total_steps=21

fail() {
  printf 'BoxFerry local validation failed: %s\n' "$1" >&2
  exit 2
}

report_failure() {
  local status=$?
  printf '\nBoxFerry local validation failed during: %s (exit %d)\n' \
    "${current_step}" "${status}" >&2
  exit "${status}"
}

trap report_failure ERR

# Public API compatibility normally infers the release type from the package
# versions. An explicitly recorded breaking change may validate the intended
# pre-1.0 release with the same major rule used by CI. Keep this opt-in narrow:
# an unknown spelling must fail before any of the numbered checks run.
semver_release_type="${BOXFERRY_SEMVER_RELEASE_TYPE:-}"
case "${semver_release_type}" in
  "" | major | minor | patch) ;;
  *)
    fail "BOXFERRY_SEMVER_RELEASE_TYPE must be empty, major, minor, or patch; got ${semver_release_type@Q}"
    ;;
esac
readonly semver_release_type

run_step() {
  local label=$1
  shift

  step=$((step + 1))
  current_step="${label}"
  printf '\n[%02d/%02d] %s\n  +' "${step}" "${total_steps}" "${label}"
  printf ' %q' "$@"
  printf '\n'
  "$@"
}

required_tools=(
  actionlint
  cargo
  cargo-deny
  cargo-llvm-cov
  cargo-semver-checks
  git
  hadolint
  jq
  lychee
  markdownlint-cli2
  prettier
  rustup
  shellcheck
  shfmt
  tombi
  zizmor
)

missing_tools=()
for tool in "${required_tools[@]}"; do
  if ! command -v "${tool}" > /dev/null 2>&1; then
    missing_tools+=("${tool}")
  fi
done

if ((${#missing_tools[@]} != 0)); then
  printf -v missing_list ' %s' "${missing_tools[@]}"
  fail "missing required tool(s):${missing_list}. Use the BoxFerry Dev Container."
fi

list_existing_files() {
  while IFS= read -r -d '' file; do
    if [[ -f "${file}" ]]; then
      printf '%s\0' "${file}"
    fi
  done < <(git ls-files --cached --others --exclude-standard -z -- "$@")
}

mapfile -d '' markdown_files < <(
  list_existing_files '*.md'
)
if ((${#markdown_files[@]} == 0)); then
  fail "the repository contains no tracked or untracked Markdown files"
fi

msrv="$({
  cargo metadata --locked --no-deps --format-version 1
} | jq -er '
  [.packages[].rust_version]
  | unique
  | if length == 1 and .[0] != null
    then .[0]
    else error("workspace packages must declare one rust-version")
    end
')"
readonly msrv

if ! rustup run "${msrv}" rustc --version > /dev/null 2>&1; then
  printf 'Installing the workspace MSRV toolchain %s.\n' "${msrv}"
  if ! rustup toolchain install "${msrv}" --profile minimal; then
    # A Dev Container may keep Cargo's package cache outside the Cargo home
    # containing the rustup proxies. Some rustup releases then return a
    # non-zero status after successfully installing the toolchain. Trust only
    # an explicit post-install execution check, never the partial install.
    rustup run "${msrv}" rustc --version > /dev/null 2>&1 ||
      fail "rustup could not install or run the workspace MSRV toolchain ${msrv}"
    printf 'The workspace MSRV toolchain %s is installed and usable.\n' "${msrv}"
  fi
fi

validation_storage_root="${CARGO_TARGET_DIR:-${repository_root}/target}/check-all/boxferry"
coverage_target_dir="${validation_storage_root}/coverage"
semver_cargo_home="${validation_storage_root}/cargo-home"
semver_target_dir="${validation_storage_root}/cargo-semver-checks-target"
for validation_directory in "${coverage_target_dir}" "${semver_cargo_home}" "${semver_target_dir}"; do
  if ! mkdir -p -- "${validation_directory}"; then
    fail "cannot create isolated validation storage: ${validation_directory}"
  fi
  if [[ ! -w "${validation_directory}" ]]; then
    fail "isolated validation storage is not writable: ${validation_directory}"
  fi
done
readonly validation_storage_root coverage_target_dir semver_cargo_home semver_target_dir
printf 'Using isolated coverage target directory: %s\n' "${coverage_target_dir}"
printf 'Using isolated cargo-semver-checks Cargo home: %s\n' "${semver_cargo_home}"
printf 'Using isolated cargo-semver-checks target directory: %s\n' "${semver_target_dir}"

semver_check=(cargo semver-checks check-release --workspace --all-features)
if [[ -n "${semver_release_type}" ]]; then
  semver_check+=(--release-type "${semver_release_type}")
fi

run_step "Format Rust" cargo fmt --all
run_step "Format and lint non-Rust files" bash scripts/check-files.sh --fix
run_step "Check whitespace errors" git --no-pager diff --check
run_step "Lint GitHub Actions syntax" actionlint
run_step "Audit GitHub Actions security" zizmor .github/workflows
run_step "Check all workspace targets and features" cargo ci-check
run_step "Check facade core feature boundary" cargo ci-core
run_step "Check facade Compose feature boundary" cargo ci-compose
run_step "Check facade Quadlet feature boundary" cargo ci-quadlet
run_step "Check repository policies" cargo ci-policy
run_step "Run Clippy with warnings denied" cargo ci-clippy
run_step "Run workspace tests" cargo ci-test
run_step "Run documentation tests" cargo ci-doctest
run_step "Build documentation with warnings denied" env RUSTDOCFLAGS="-D warnings" cargo ci-doc
run_step "Clean coverage artifacts" env CARGO_TARGET_DIR="${coverage_target_dir}" \
  cargo llvm-cov clean --locked
run_step "Enforce coverage ratchets" env CARGO_TARGET_DIR="${coverage_target_dir}" \
  cargo llvm-cov --locked --no-clean --workspace --all-features \
  --all-targets --summary-only --fail-under-regions 82 --fail-under-functions 87 \
  --fail-under-lines 82
run_step "Check all targets with the MSRV" cargo "+${msrv}" ci-check
run_step "Check repository policies with the MSRV" cargo "+${msrv}" ci-policy
run_step "Audit dependencies, licenses, bans, and sources" cargo deny --all-features check
run_step "Check local documentation links" lychee --config lychee.toml --root-dir . --offline \
  "${markdown_files[@]}"
run_step "Check published API compatibility" env CARGO_HOME="${semver_cargo_home}" \
  CARGO_TARGET_DIR="${semver_target_dir}" "${semver_check[@]}"

printf '\nBoxFerry local validation passed all %d steps.\n' "${total_steps}"
