#!/usr/bin/env bash

set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repository_root="$(cd -- "${script_directory}/.." && pwd -P)"
readonly repository_root

cd -- "${repository_root}"

current_step="preflight"
step=0
readonly total_steps=50

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

format_mode="fix"
if (($# > 1)); then
  fail "Usage: $0 [--check|--fix]"
fi
case "${1:---fix}" in
  --check) format_mode="check" ;;
  --fix) ;;
  *) fail "Usage: $0 [--check|--fix]" ;;
esac
readonly format_mode

# Development compares the complete proposed branch/working tree with trusted main.
# Published Release validation separately infers compatibility from its chosen version.
# Explicit breaking intent affects development only; unknown spellings fail preflight.
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
  podman
  prettier
  python3
  realpath
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

# Cargo test binaries can embed absolute fixture paths. An external target
# shared with another worktree may retain binaries after that worktree moves
# or is removed, so the complete gate only accepts worktree-owned artifacts.
if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
  resolved_target_dir="$(realpath -m -- "${CARGO_TARGET_DIR}")" ||
    fail "cannot resolve CARGO_TARGET_DIR: ${CARGO_TARGET_DIR}"
  case "${resolved_target_dir}" in
    "${repository_root}/"*) export CARGO_TARGET_DIR="${resolved_target_dir}" ;;
    *) fail "CARGO_TARGET_DIR must be inside this worktree; unset it or choose a worktree-local target directory" ;;
  esac
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
  if ! rustup toolchain install "${msrv}" --profile minimal --no-self-update; then
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

current_step="Prepare the canonical shared Python toolchain"
automation_uv="$(python3 automation/src/boxferry_ci/bootstrap.py install-uv \
  --project automation --cache-root target/shared-uv --no-path-uv)"
readonly automation_uv
PATH="$(dirname -- "${automation_uv}"):${PATH}"
export PATH

# Verify the same declared plugin/compiler pair used by shared hosted SemVer.
# Keep the isolated Cargo home used by the final command: Cargo dispatch can
# prefer CARGO_HOME/bin or a config alias over an apparently correct PATH tool.
current_step="Verify exact local Cargo SemVer selection"
semver_selection="$(env CARGO_HOME="${semver_cargo_home}" CARGO_TARGET_DIR="${semver_target_dir}" \
  uv run --locked --project automation python -m boxferry_ci.cargo_tools verify \
  --repository-root "${repository_root}" --package cargo-semver-checks)"
semver_rust_channel="$(jq -er '.rust_channel' <<< "${semver_selection}")"
readonly semver_rust_channel

# Verify coverage in the same Cargo home and working directory used below.
current_step="Verify exact local Cargo coverage selection"
coverage_selection="$(uv run --locked --project automation python -m boxferry_ci.cargo_tools verify \
  --repository-root "${repository_root}" --package cargo-llvm-cov --require-candidate)"
coverage_rust_channel="$(jq -er '.rust_channel' <<< "${coverage_selection}")"
readonly coverage_rust_channel

if [[ "${format_mode}" == "check" ]]; then
  run_step "Check Rust formatting" cargo fmt --all -- --check
  run_step "Check non-Rust files" bash scripts/check-files.sh --check
else
  run_step "Format Rust" cargo fmt --all
  run_step "Format and lint non-Rust files" bash scripts/check-files.sh --fix
fi
if [[ "${format_mode}" == "check" ]]; then
  run_step "Check shared automation Python formatting" \
    uv run --locked --project automation ruff format --check automation
  run_step "Check consumer Python formatting" \
    uv run --locked --project automation ruff format --check --config automation/pyproject.toml scripts/shared-ci.py scripts/test-shared-automation.py scripts/git-worktree.py scripts/test-git-worktree.py scripts/devcontainer-source-report-diagnostic.py scripts/test-workflow-lint.py
else
  run_step "Format shared automation Python" \
    uv run --locked --project automation ruff format automation
  run_step "Format consumer Python" \
    uv run --locked --project automation ruff format --config automation/pyproject.toml scripts/shared-ci.py scripts/test-shared-automation.py scripts/git-worktree.py scripts/test-git-worktree.py scripts/devcontainer-source-report-diagnostic.py scripts/test-workflow-lint.py
fi
run_step "Lint consumer Python" \
  uv run --locked --project automation ruff check --config automation/pyproject.toml scripts/shared-ci.py scripts/test-shared-automation.py scripts/git-worktree.py scripts/test-git-worktree.py scripts/devcontainer-source-report-diagnostic.py scripts/test-workflow-lint.py
run_step "Type-check consumer Python" \
  uv run --locked --project automation ty check scripts/shared-ci.py scripts/test-shared-automation.py scripts/git-worktree.py scripts/test-git-worktree.py scripts/devcontainer-source-report-diagnostic.py scripts/test-workflow-lint.py
run_step "Lint shared automation Python" uv run --locked --project automation ruff check automation
run_step "Type-check shared automation Python" uv run --locked --project automation ty check --project automation
run_step "Verify every canonical archive on both architectures" \
  uv run --locked --project automation python -m boxferry_ci.asset_verification \
  --repository-root . --working-tree
run_step "Prepare the integrity-pinned native Renovate test runtime" \
  uv run --locked --project automation python -m boxferry_ci.renovate_runtime \
  --engine podman --pin automation/renovate-runtime.json --destination target/tools/renovate
export BOXFERRY_RENOVATE_PACKAGE="${repository_root}/target/tools/renovate"
run_step "Test shared automation contracts" \
  uv run --locked --project automation python -m boxferry_ci.test_reporting --repository-root . --start-directory automation/tests
run_step "Verify candidate Compose provider artifact integrity" \
  uv run --locked --project automation python -m boxferry_ci.compose_provider \
  --repository-root "${repository_root}" --verify-working-tree
run_step "Test shared automation adoption" env PYTHONDONTWRITEBYTECODE=1 uv run --locked --project automation python scripts/test-shared-automation.py --with-native-renovate
run_step "Test exact Git source and standalone clone contracts" env PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-git-worktree.py
run_step "Test validation-plan contracts" env PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-validation-plan.py
run_step "Verify Kubernetes tool installer integrity" \
  uv run --locked --project automation python \
  scripts/kubernetes_tool_checksum_refresh.py \
  --verify-working-tree --repository-root "${repository_root}"
run_step "Test release binary archive contracts" env PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-release-binaries.py
run_step "Test release emulator extraction" bash scripts/test-release-emulator.sh
run_step "Test release metadata policy" bash scripts/test-release-metadata.sh
run_step "Test Podman live cleanup collections" bash scripts/test-podman-live-cleanup.sh
run_step "Test Podman live outer storage ownership" bash scripts/test-podman-live-outer-storage.sh
run_step "Test application probe contracts" uv run --locked --project automation bash scripts/test-application-probes.sh
run_step "Test observability Grafana network topology" bash scripts/test-observability-grafana-network.sh
run_step "Test observability application contracts" uv run --locked --project automation bash scripts/test-observability-application.sh
run_step "Test Supabase application timeout boundary" uv run --locked --project automation bash scripts/test-supabase-application.sh
run_step "Test migration-readiness contracts" python3 scripts/test-migration-readiness.py
run_step "Test reviewed application catalogue admission" env PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-reviewed-application-catalogue.py
run_step "Run offline migration-readiness tier" python3 scripts/migration-readiness.py run \
  --tier offline --evidence target/migration-readiness/offline-evidence-v2.json
run_step "Validate release metadata and changelog" bash scripts/validate-release-metadata.sh --development
run_step "Check whitespace errors" git --no-pager diff --check
run_step "Lint GitHub Actions syntax" actionlint
run_step "Test selected actual Actionlint CLI contracts" \
  uv run --locked --project automation python scripts/test-workflow-lint.py
run_step "Audit GitHub Actions security" zizmor .github/workflows
run_step "Check all workspace targets and features" cargo ci-check
run_step "Check facade core feature boundary" cargo ci-core
run_step "Check facade Compose feature boundary" cargo ci-compose
run_step "Check facade Podman feature boundary" cargo ci-podman
run_step "Check facade Quadlet feature boundary" cargo ci-quadlet
run_step "Check repository policies" cargo ci-policy
run_step "Run Clippy with warnings denied" cargo ci-clippy
run_step "Run workspace tests" cargo ci-test
run_step "Run documentation tests" cargo ci-doctest
run_step "Build documentation with warnings denied" env RUSTDOCFLAGS="-D warnings" cargo ci-doc
run_step "Clean coverage artifacts" env CARGO_TARGET_DIR="${coverage_target_dir}" \
  RUSTUP_TOOLCHAIN="${coverage_rust_channel}" \
  cargo llvm-cov clean --locked
run_step "Enforce coverage ratchets" env CARGO_TARGET_DIR="${coverage_target_dir}" \
  RUSTUP_TOOLCHAIN="${coverage_rust_channel}" \
  cargo llvm-cov --locked --no-clean --workspace --all-features \
  --all-targets --summary-only --fail-under-regions 82 --fail-under-functions 87 \
  --fail-under-lines 82
run_step "Check all targets with the MSRV" cargo "+${msrv}" ci-check
run_step "Check repository policies with the MSRV" cargo "+${msrv}" ci-policy
run_step "Audit dependencies, licenses, bans, and sources" cargo deny --all-features check
run_step "Check local documentation links" lychee --config lychee.toml --root-dir . --offline \
  "${markdown_files[@]}"
run_step "Check development API compatibility" env CARGO_HOME="${semver_cargo_home}" \
  CARGO_TARGET_DIR="${semver_target_dir}" RUSTUP_TOOLCHAIN="${semver_rust_channel}" \
  uv run --locked --project automation python -m boxferry_ci.api_baselines run \
  --repository-root "${repository_root}" --api-purpose development --event local \
  --release-type "${semver_release_type}" --report-json target/api-baseline-local.json

printf '\nBoxFerry local validation passed all %d steps.\n' "${total_steps}"
