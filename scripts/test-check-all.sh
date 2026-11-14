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
if [[ "${CHECK_ALL_TEST_FAIL:-}" == "${command_name}" ]]; then
  exit 17
fi
if [[ -n "${CHECK_ALL_TEST_FAIL_COMMAND:-}" && "${command_name} $*" == "${CHECK_ALL_TEST_FAIL_COMMAND}" ]]; then
  exit 17
fi
if [[ -n "${CHECK_ALL_TEST_FAIL_ARGUMENT:-}" && "$*" == "${CHECK_ALL_TEST_FAIL_ARGUMENT}" ]]; then
  exit 17
fi
case "${command_name}:$*" in
  git:ls-files*) printf 'README.md\0' ;;
  python3:automation/src/boxferry_ci/bootstrap.py\ install-uv*) printf '%s/uv\n' "${0%/*}" ;;
  jq:*) printf '%s\n' "${CHECK_ALL_TEST_MSRV}" ;;
  uv:*boxferry_ci.cargo_tools*--package\ cargo-llvm-cov*)
    [[ "${CARGO_HOME}" == "${CHECK_ALL_TEST_COVERAGE_HOME}" ]] ;;
  uv:*boxferry_ci.cargo_tools* | uv:*boxferry_ci.api_baselines*)
    [[ "${CARGO_HOME}" == "${CARGO_TARGET_DIR%/cargo-semver-checks-target}/cargo-home" ]] ;;
  cargo:llvm-cov*)
    [[ "${CARGO_HOME}" == "${CHECK_ALL_TEST_COVERAGE_HOME}" ]]
    [[ "${RUSTUP_TOOLCHAIN}" == "${CHECK_ALL_TEST_MSRV}" ]] ;;
  cargo:semver-checks*)
    [[ "${RUSTUP_TOOLCHAIN}" == "${CHECK_ALL_TEST_MSRV}" ]]
    [[ "${CARGO_HOME}" == "${CARGO_TARGET_DIR%/cargo-semver-checks-target}/cargo-home" ]] ;;
esac
MOCK
chmod +x "${test_root}/bin/mock"
for tool in actionlint bash cargo cargo-deny cargo-llvm-cov cargo-semver-checks cspell curl git \
  hadolint jq lychee markdownlint-cli2 npm podman prettier python3 rustup shellcheck shfmt tombi uv zizmor; do
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
    CARGO_HOME="${test_root}/ambient-cargo-home" \
    CHECK_ALL_TEST_COVERAGE_HOME="${test_root}/ambient-cargo-home" \
    RUSTUP_TOOLCHAIN=stable \
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
expected_ordinary_sequence=""
for ((ordinary_step = 1; ordinary_step <= 50; ordinary_step++)); do
  printf -v ordinary_prefix '[%02d/50]' "${ordinary_step}"
  expected_ordinary_sequence+="${ordinary_prefix}"$'\n'
done
for mode in default fix check; do
  [[ "$(grep --extended-regexp --count '^\[[0-9]{2}/50\]' "${test_root}/${mode}.output")" == 50 ]]
  [[ "$(grep --extended-regexp --only-matching '^\[[0-9]{2}/50\]' "${test_root}/${mode}.output")" == "${expected_ordinary_sequence%$'\n'}" ]]
  grep --fixed-strings --quiet -- '[50/50] Check development API compatibility' "${test_root}/${mode}.output"
  [[ "$(tail -n 1 "${test_root}/${mode}.commands")" == *"python -m boxferry_ci.api_baselines run "* ]]
  [[ "$(tail -n 1 "${test_root}/${mode}.output")" == 'BoxFerry local validation passed all 50 steps.' ]]
done
# Keep the explicit source/index bridge in every complete mode exactly once.
for mode in default fix check; do
  command="python3 scripts/test-git-worktree.py"
  assert_contains "${mode}" "${command}"
  [[ "$(grep --fixed-strings --line-regexp --count -- "${command}" "${test_root}/${mode}.commands")" == 1 ]]
done
# Every new consumer check and locked diagnostic harness must run exactly once and
# preserve the first failure, before any later complete-gate command is dispatched.
for mode in default fix check; do
  consumer_files="scripts/shared-ci.py scripts/test-shared-automation.py scripts/git-worktree.py scripts/test-git-worktree.py scripts/devcontainer-source-report-diagnostic.py scripts/test-workflow-lint.py"
  consumer_format="uv run --locked --project automation ruff format"
  if [[ "${mode}" == check ]]; then
    consumer_format+=" --check"
  fi
  consumer_format+=" --config automation/pyproject.toml ${consumer_files}"
  for command in "${consumer_format}" \
    "uv run --locked --project automation ruff check --config automation/pyproject.toml ${consumer_files}" \
    "uv run --locked --project automation ty check ${consumer_files}" \
    "uv run --locked --project automation python scripts/test-shared-automation.py --with-native-renovate" \
    "uv run --locked --project automation python scripts/test-workflow-lint.py"; do
    assert_contains "${mode}" "${command}"
    [[ "$(grep --fixed-strings --line-regexp --count -- "${command}" "${test_root}/${mode}.commands")" == 1 ]]
    status=0
    if [[ "${mode}" == default ]]; then
      CHECK_ALL_TEST_FAIL_COMMAND="${command}" run_gate consumer_failure || status=$?
    else
      CHECK_ALL_TEST_FAIL_COMMAND="${command}" run_gate consumer_failure "--${mode}" || status=$?
    fi
    [[ "${status}" == 17 ]]
    [[ "$(tail -n 1 "${test_root}/consumer_failure.commands")" == "${command}" ]]
  done
done
# Every YAML-dependent application test uses the declared candidate environment.
for mode in default check; do
  for script in test-application-probes.sh test-observability-application.sh test-supabase-application.sh; do
    command="uv run --locked --project automation bash scripts/${script}"
    assert_contains "${mode}" "${command}"
    [[ "$(grep --fixed-strings --line-regexp --count -- "${command}" "${test_root}/${mode}.commands")" == 1 ]]
  done
done
# Fail the targeted UV invocation after earlier UV steps have succeeded.
for mode in default check; do
  for script in test-application-probes.sh test-observability-application.sh test-supabase-application.sh; do
    case "${script}" in
      test-application-probes.sh) expected_step=22 ;;
      test-observability-application.sh) expected_step=24 ;;
      test-supabase-application.sh) expected_step=25 ;;
    esac
    label="application-env-${mode}-${expected_step}"
    command="uv run --locked --project automation bash scripts/${script}"
    status=0
    if [[ "${mode}" == default ]]; then
      CHECK_ALL_TEST_FAIL_ARGUMENT="run --locked --project automation bash scripts/${script}" \
        run_gate "${label}" || status=$?
    else
      CHECK_ALL_TEST_FAIL_ARGUMENT="run --locked --project automation bash scripts/${script}" \
        run_gate "${label}" --check || status=$?
    fi
    [[ "${status}" == 17 ]]
    [[ "$(tail -n 1 "${test_root}/${label}.commands")" == "${command}" ]]
    [[ "$(grep --extended-regexp --count '^\[[0-9]{2}/50\]' "${test_root}/${label}.output")" == "${expected_step}" ]]
  done
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
assert_contains check "uv run --locked --project automation python -m boxferry_ci.api_baselines run --repository-root ${test_root}/repository --api-purpose development --event local --release-type  --report-json target/api-baseline-local.json"
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
  grep -vE '^(cargo fmt|bash scripts/check-files.sh|uv run --locked --project automation ruff format|uv run --frozen ruff format|uv run --frozen python scripts/generate_brand_assets.py)' \
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

# Shared tooling is installed and each new validation is dispatched exactly once.
for mode in default fix check; do
  for command in \
    "python3 automation/src/boxferry_ci/bootstrap.py install-uv --project automation --cache-root target/shared-uv --no-path-uv" \
    "uv run --locked --project automation python -m boxferry_ci.cargo_tools verify --repository-root ${test_root}/repository --package cargo-semver-checks" \
    "uv run --locked --project automation python -m boxferry_ci.cargo_tools verify --repository-root ${test_root}/repository --package cargo-llvm-cov --require-candidate" \
    "uv run --locked --project automation ruff check automation" \
    "uv run --locked --project automation ty check --project automation" \
    "uv run --locked --project automation python -m boxferry_ci.asset_verification --repository-root . --working-tree" \
    "uv run --locked --project automation python -m boxferry_ci.renovate_runtime --engine podman --pin automation/renovate-runtime.json --destination target/tools/renovate" \
    "uv run --locked --project automation python -m boxferry_ci.test_reporting --repository-root . --start-directory automation/tests" \
    "uv run --locked --project automation python -m boxferry_ci.compose_provider --repository-root ${test_root}/repository --verify-working-tree" \
    "uv run --locked --project automation python scripts/kubernetes_tool_checksum_refresh.py --verify-working-tree --repository-root ${test_root}/repository"; do
    assert_contains "${mode}" "${command}"
    [[ "$(grep -Fxc -- "${command}" "${test_root}/${mode}.commands")" == 1 ]]
  done
done
# An invalid or missing declared plugin must stop before formatting or compatibility.
status=0
CHECK_ALL_TEST_FAIL_ARGUMENT="run --locked --project automation python -m boxferry_ci.cargo_tools verify --repository-root ${test_root}/repository --package cargo-semver-checks" \
  run_gate invalid_semver --check || status=$?
[[ "${status}" == 17 ]]
if grep -q '^cargo fmt' "${test_root}/invalid_semver.commands"; then
  printf 'Invalid SemVer selection reached formatting.\n' >&2
  exit 1
fi
grep -Fq 'Verify exact local Cargo SemVer selection' "${test_root}/invalid_semver.output"
# Failed coverage selection stops before any formatting, build or coverage command.
status=0
CHECK_ALL_TEST_FAIL_ARGUMENT="run --locked --project automation python -m boxferry_ci.cargo_tools verify --repository-root ${test_root}/repository --package cargo-llvm-cov --require-candidate" \
  run_gate invalid_coverage --check || status=$?
[[ "${status}" == 17 ]]
[[ "$(grep -c '^cargo ' "${test_root}/invalid_coverage.commands" || true)" == 0 ]]
grep -Fq 'Verify exact local Cargo coverage selection' "${test_root}/invalid_coverage.output"
assert_contains fix "uv run --locked --project automation ruff format automation"
assert_contains check "uv run --locked --project automation ruff format --check automation"
for mode in fix check; do
  if [[ "${mode}" == fix ]]; then
    format_command="uv run --locked --project automation ruff format automation"
  else
    format_command="uv run --locked --project automation ruff format --check automation"
  fi
  for command in \
    "python3 automation/src/boxferry_ci/bootstrap.py install-uv --project automation --cache-root target/shared-uv --no-path-uv" \
    "${format_command}" \
    "uv run --locked --project automation python -m boxferry_ci.cargo_tools verify --repository-root ${test_root}/repository --package cargo-semver-checks" \
    "uv run --locked --project automation python -m boxferry_ci.cargo_tools verify --repository-root ${test_root}/repository --package cargo-llvm-cov --require-candidate" \
    "uv run --locked --project automation ruff check automation" \
    "uv run --locked --project automation ty check --project automation" \
    "uv run --locked --project automation python -m boxferry_ci.asset_verification --repository-root . --working-tree" \
    "uv run --locked --project automation python -m boxferry_ci.renovate_runtime --engine podman --pin automation/renovate-runtime.json --destination target/tools/renovate" \
    "uv run --locked --project automation python -m boxferry_ci.test_reporting --repository-root . --start-directory automation/tests" \
    "uv run --locked --project automation python -m boxferry_ci.compose_provider --repository-root ${test_root}/repository --verify-working-tree" \
    "uv run --locked --project automation python scripts/kubernetes_tool_checksum_refresh.py --verify-working-tree --repository-root ${test_root}/repository"; do
    status=0
    CHECK_ALL_TEST_FAIL_COMMAND="${command}" run_gate shared_failure "--${mode}" || status=$?
    [[ "${status}" == 17 ]]
    [[ "$(tail -n 1 "${test_root}/shared_failure.commands")" == "${command}" ]]
    [[ "$(grep -Fc 'BoxFerry local validation passed' "${test_root}/shared_failure.output" || true)" == 0 ]]
  done
done

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

for mode in default fix check; do
  if grep -Fq -- 'scripts/shared-ci.py devcontainer-local' "${test_root}/${mode}.commands"; then
    printf 'Ordinary validation must not build a Dev Container.\n' >&2
    exit 1
  fi
done

# Actual workflow lint precedes its CLI contracts, and either failure stops the gate.
for mode in fix check; do
  lint_line="$(grep -nFx actionlint "${test_root}/${mode}.commands" | cut -d: -f1)"
  harness_line="$(grep -nFx 'uv run --locked --project automation python scripts/test-workflow-lint.py' "${test_root}/${mode}.commands" | cut -d: -f1)"
  [[ "${harness_line}" == "$((lint_line + 1))" ]]
done

# A failed check must stop the gate and preserve its exit status in either mode.
export CHECK_ALL_TEST_FAIL=actionlint
for mode in fix check; do
  status=0
  run_gate failure "--${mode}" || status=$?
  [[ "${status}" == 17 ]]
  [[ "$(tail -n 1 "${test_root}/failure.commands")" == "actionlint" ]]
done
printf 'Complete gate mode regression tests passed.\n'
