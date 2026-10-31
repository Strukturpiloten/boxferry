#!/usr/bin/env bash
# Exercise the real gate's command dispatch without invoking compilers or downloading tools.
set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# Capture the selected Bash before PATH is populated with mocks (macOS ships an older /bin/bash).
bash_executable="$(command -v bash)"
# Deliberately synthetic: prove the gate follows Cargo metadata, not a copied pin.
readonly mock_msrv=9.8.7
test_root="$(mktemp -d)"
trap 'rm -r -- "${test_root}"' EXIT
mkdir -p "${test_root}/repository/scripts/lib" "${test_root}/repository/site" "${test_root}/bin"
cp -- "${script_directory}/check-all.sh" "${test_root}/repository/scripts/check-all.sh"
cp -- "${script_directory}/podman-live-conformance.sh" \
  "${test_root}/repository/scripts/podman-live-conformance.sh"
cp -- "${script_directory}/test-podman-live-cleanup.sh" \
  "${test_root}/repository/scripts/test-podman-live-cleanup.sh"
cp -- "${script_directory}/test-observability-grafana-network.sh" \
  "${test_root}/repository/scripts/test-observability-grafana-network.sh"
cp -- "${script_directory}/test-observability-application.sh" \
  "${test_root}/repository/scripts/test-observability-application.sh"
cp -- "${script_directory}/test-supabase-application.sh" \
  "${test_root}/repository/scripts/test-supabase-application.sh"
cp -- "${script_directory}/lib/observability-application.sh" \
  "${test_root}/repository/scripts/lib/observability-application.sh"
cp -- "${script_directory}/lib/supabase-application.sh" \
  "${test_root}/repository/scripts/lib/supabase-application.sh"
cp -- "${script_directory}/lib/in-shell-deadline.py" \
  "${test_root}/repository/scripts/lib/in-shell-deadline.py"
cp -- "${script_directory}/../README.md" "${test_root}/repository/README.md"

# These mock commands record exactly what the unmodified gate requests.
cat > "${test_root}/bin/mock" << 'MOCK'
#!/bin/bash
set -Eeuo pipefail
command_name="${0##*/}"
# Metadata is piped to jq, so its command order is deliberately not asserted.
case "${command_name}:$*" in
  cargo:metadata* | jq:*) ;;
  *)
    printf -v command_line '%s ' "${command_name}" "$@"
    printf '%s\n' "${command_line% }" >> "${CHECK_ALL_TEST_LOG}"
    ;;
esac
if [[ "${CHECK_ALL_TEST_FAIL:-}" == "${command_name}" ||
  "${CHECK_ALL_TEST_FAIL_COMMAND:-}" == "${command_name}:$*" ]]; then
  exit 17
fi
if [[ -n "${CHECK_ALL_TEST_FAIL_ARGUMENT:-}" && "$*" == "${CHECK_ALL_TEST_FAIL_ARGUMENT}" ]]; then
  exit 17
fi
case "${command_name}:$*" in
  git:ls-files*) printf 'README.md\0' ;;
  jq:*) printf '%s\n' "${CHECK_ALL_TEST_MSRV}" ;;
esac
MOCK
chmod +x "${test_root}/bin/mock"
for tool in actionlint bash cargo cargo-deny cargo-llvm-cov cargo-semver-checks cspell curl git \
  hadolint jq lychee markdownlint-cli2 npm prettier python3 rustup shellcheck shfmt tombi uv zizmor; do
  ln -s mock "${test_root}/bin/${tool}"
done

run_gate() {
  local label=$1
  shift
  local mock_repository="${CHECK_ALL_TEST_REPOSITORY_ROOT:-${test_root}/repository}"
  local mock_target="${CHECK_ALL_TEST_TARGET_DIR:-${mock_repository}/target}"
  : > "${test_root}/${label}.commands"
  PATH="${test_root}/bin:${PATH}" \
    CHECK_ALL_TEST_LOG="${test_root}/${label}.commands" \
    CHECK_ALL_TEST_MSRV="${CHECK_ALL_TEST_MSRV:-${mock_msrv}}" \
    CARGO_TARGET_DIR="${mock_target}" \
    BOXFERRY_SEMVER_RELEASE_TYPE="" PODMAN_LENS_SEMVER_CHECK=0 \
    BOXFERRY_WEBSITE_SOURCE_MODE=local \
    "${bash_executable}" "${mock_repository}/scripts/check-all.sh" "$@" \
    > "${test_root}/${label}.output" 2>&1
}

assert_contains() {
  if ! grep -Fxq -- "$2" "${test_root}/$1.commands"; then
    printf 'Missing expected command: %s\n' "$2" >&2
    return 1
  fi
}

export BOXFERRY_WEBSITE_FORMAT_MODE=fix
run_gate default
run_gate fix --fix
run_gate check --check

# A target owned by the former worktree must not be reused after relocation.
mkdir -p "${test_root}/relocated/scripts"
cp -- "${script_directory}/check-all.sh" "${test_root}/relocated/scripts/check-all.sh"
status=0
CHECK_ALL_TEST_REPOSITORY_ROOT="${test_root}/relocated" \
  CHECK_ALL_TEST_TARGET_DIR="${test_root}/repository/target" \
  run_gate relocated --check || status=$?
[[ "${status}" == 2 && ! -s "${test_root}/relocated.commands" ]]
grep --fixed-strings --quiet -- 'CARGO_TARGET_DIR must be inside this worktree' \
  "${test_root}/relocated.output"
[[ "$(grep --extended-regexp --count '^\[[0-9]{2}/38\]' "${test_root}/default.output")" == 38 ]]
grep --fixed-strings --quiet -- '[38/38] Check published API compatibility' "${test_root}/default.output"
[[ "$(tail -n 1 "${test_root}/default.output")" == 'BoxFerry local validation passed all 38 steps.' ]]
assert_contains default "cargo ci-docker"
assert_contains check "cargo ci-docker"
assert_contains default "bash scripts/test-application-probes.sh"
assert_contains check "bash scripts/test-application-probes.sh"
# The gate command is recorded by fake Bash; never run the expensive profile
# from this orchestration regression (its script is intentionally not copied).
[[ ! -e "${test_root}/repository/scripts/test-docker-forgejo-application.sh" ]]
for mode in default fix check; do
  assert_contains "${mode}" "bash scripts/test-docker-forgejo-application.sh"
  [[ "$(grep --fixed-strings --count 'bash scripts/test-docker-forgejo-application.sh' "${test_root}/${mode}.commands")" == 1 ]]
done
assert_contains default "python3 scripts/test-reviewed-application-catalogue.py"
assert_contains check "python3 scripts/test-reviewed-application-catalogue.py"
for mode in default fix check; do
  for command in "python3 scripts/test-release-binaries.py" "bash scripts/test-release-emulator.sh"; do
    assert_contains "${mode}" "${command}"
    [[ "$(grep --fixed-strings --line-regexp --count -- "${command}" "${test_root}/${mode}.commands")" == 1 ]]
  done
done
[[ "$(grep --extended-regexp --count '^\[[0-9]{2}/31\]' "${test_root}/default.output" || true)" == 0 ]]
[[ "$(grep --fixed-strings --count 'BoxFerry local validation passed all 31 steps.' "${test_root}/default.output" || true)" == 0 ]]
diff -u "${test_root}/default.commands" "${test_root}/fix.commands"
assert_contains fix "bash scripts/check-files.sh --fix"
assert_contains check "bash scripts/check-files.sh --check"
assert_contains check "cargo +${mock_msrv} ci-check"
assert_contains check "cargo +${mock_msrv} ci-policy"
CHECK_ALL_TEST_MSRV=9.8.8 run_gate metadata --check
assert_contains metadata "cargo +9.8.8 ci-check"
assert_contains metadata "cargo +9.8.8 ci-policy"
if grep -q '^cargo fmt' "${test_root}/fix.commands"; then
  assert_contains fix "cargo fmt --all"
  assert_contains check "cargo fmt --all -- --check"
else
  assert_contains fix "uv run --frozen ruff format scripts tests"
  assert_contains check "uv run --frozen ruff format --check scripts tests"
  assert_contains check "uv run --frozen python scripts/generate_brand_assets.py --check"
  # The explicit flag wins over the website's existing environment default.
  export BOXFERRY_WEBSITE_FORMAT_MODE=check
  run_gate explicit_fix --fix
  run_gate environment_check
  diff -u "${test_root}/fix.commands" "${test_root}/explicit_fix.commands"
  diff -u "${test_root}/check.commands" "${test_root}/environment_check.commands"
fi

# Both modes must request every non-formatting validation step, in the same order.
for mode in fix check; do
  grep -vE '^(cargo fmt|bash scripts/check-files.sh|uv run --frozen ruff format|uv run --frozen python scripts/generate_brand_assets.py)' \
    "${test_root}/${mode}.commands" > "${test_root}/${mode}.validation"
done
diff -u "${test_root}/fix.validation" "${test_root}/check.validation"

for invalid in --unknown --checks; do
  status=0
  run_gate invalid "${invalid}" || status=$?
  [[ "${status}" == 2 && ! -s "${test_root}/invalid.commands" ]]
done
status=0
run_gate extra --check --fix || status=$?
[[ "${status}" == 2 && ! -s "${test_root}/extra.commands" ]]

# Each newly registered contract check is dispatched once and is a hard failure boundary.
for failed_script in scripts/test-release-binaries.py scripts/test-release-emulator.sh; do
  for mode in fix check; do
    status=0
    CHECK_ALL_TEST_FAIL_ARGUMENT="${failed_script}" run_gate binary_failure "--${mode}" || status=$?
    [[ "${status}" == 17 ]]
    [[ "$(tail -n 1 "${test_root}/binary_failure.commands")" == *" ${failed_script}" ]]
    [[ "$(grep --fixed-strings --count -- 'BoxFerry local validation passed' "${test_root}/binary_failure.output" || true)" == 0 ]]
  done
done

# A failed check must stop the gate and preserve its exit status in either mode.
export CHECK_ALL_TEST_FAIL=actionlint
for mode in fix check; do
  status=0
  run_gate failure "--${mode}" || status=$?
  [[ "${status}" == 17 ]]
  [[ "$(tail -n 1 "${test_root}/failure.commands")" == "actionlint" ]]
done
unset CHECK_ALL_TEST_FAIL
export CHECK_ALL_TEST_FAIL_COMMAND=cargo:ci-docker
for mode in fix check; do
  status=0
  run_gate docker_failure "--${mode}" || status=$?
  [[ "${status}" == 17 ]]
  [[ "$(tail -n 1 "${test_root}/docker_failure.commands")" == "cargo ci-docker" ]]
done
export CHECK_ALL_TEST_FAIL_COMMAND=bash:scripts/test-docker-forgejo-application.sh
for mode in fix check; do
  status=0
  run_gate forgejo_failure "--${mode}" || status=$?
  [[ "${status}" == 17 ]]
  [[ "$(tail -n 1 "${test_root}/forgejo_failure.commands")" == "bash scripts/test-docker-forgejo-application.sh" ]]
done
printf 'Complete gate mode regression tests passed.\n'
