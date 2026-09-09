#!/usr/bin/env bash
# Exercise the real gate's command dispatch without invoking compilers or downloading tools.
set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# Capture the selected Bash before PATH is populated with mocks (macOS ships an older /bin/bash).
bash_executable="$(command -v bash)"
test_root="$(mktemp -d)"
trap 'rm -r -- "${test_root}"' EXIT
mkdir -p "${test_root}/repository/scripts" "${test_root}/repository/site" "${test_root}/bin"
cp -- "${script_directory}/check-all.sh" "${test_root}/repository/scripts/check-all.sh"
cp -- "${script_directory}/podman-live-conformance.sh" \
  "${test_root}/repository/scripts/podman-live-conformance.sh"
cp -- "${script_directory}/test-podman-live-cleanup.sh" \
  "${test_root}/repository/scripts/test-podman-live-cleanup.sh"
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
case "${command_name}:$*" in
  git:ls-files*) printf 'README.md\0' ;;
  jq:*) printf '1.85.0\n' ;;
esac
MOCK
chmod +x "${test_root}/bin/mock"
for tool in actionlint bash cargo cargo-deny cargo-llvm-cov cargo-semver-checks cspell curl git \
  hadolint jq lychee markdownlint-cli2 npm prettier rustup shellcheck shfmt tombi uv zizmor; do
  ln -s mock "${test_root}/bin/${tool}"
done

run_gate() {
  local label=$1
  shift
  : > "${test_root}/${label}.commands"
  PATH="${test_root}/bin:${PATH}" \
    CHECK_ALL_TEST_LOG="${test_root}/${label}.commands" \
    CARGO_TARGET_DIR="${test_root}/target" \
    BOXFERRY_SEMVER_RELEASE_TYPE="" PODMAN_LENS_SEMVER_CHECK=0 \
    BOXFERRY_WEBSITE_SOURCE_MODE=local \
    "${bash_executable}" "${test_root}/repository/scripts/check-all.sh" "$@" \
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
diff -u "${test_root}/default.commands" "${test_root}/fix.commands"
assert_contains fix "bash scripts/check-files.sh --fix"
assert_contains check "bash scripts/check-files.sh --check"
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

# A failed check must stop the gate and preserve its exit status in either mode.
export CHECK_ALL_TEST_FAIL=actionlint
for mode in fix check; do
  status=0
  run_gate failure "--${mode}" || status=$?
  [[ "${status}" == 17 ]]
  [[ "$(tail -n 1 "${test_root}/failure.commands")" == "actionlint" ]]
done
printf 'Complete gate mode regression tests passed.\n'
