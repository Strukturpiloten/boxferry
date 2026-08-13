#!/usr/bin/env bash

set -euo pipefail

for cargo_directory_name in CARGO_HOME CARGO_TARGET_DIR; do
  cargo_directory="${!cargo_directory_name:-}"
  if [[ -z "${cargo_directory}" ]]; then
    printf 'BoxFerry Dev Container is missing %s.\n' "${cargo_directory_name}" >&2
    exit 1
  fi
  if [[ ! -w "${cargo_directory}" ]]; then
    sudo chown -R "$(id -u):$(id -g)" "${cargo_directory}"
  fi
  if [[ ! -w "${cargo_directory}" ]]; then
    printf 'BoxFerry Dev Container cannot make %s writable: %s\n' \
      "${cargo_directory_name}" "${cargo_directory}" >&2
    exit 1
  fi
done

tools=(
  actionlint
  cargo
  cargo-clippy
  cargo-deny
  cargo-llvm-cov
  cargo-semver-checks
  curl
  gh
  git
  hadolint
  jq
  lychee
  markdownlint-cli2
  node
  npm
  prettier
  rustc
  rustfmt
  rustup
  shellcheck
  shfmt
  taplo
  zizmor
)

for tool in "${tools[@]}"; do
  if ! command -v "${tool}" > /dev/null 2>&1; then
    printf 'BoxFerry Dev Container is missing required tool: %s\n' "${tool}" >&2
    exit 1
  fi
done

if ! rustup component list --installed | grep -q '^llvm-tools-'; then
  printf 'BoxFerry Dev Container is missing Rust component: llvm-tools-preview\n' >&2
  exit 1
fi

for repository in compose-lens quadlet-lens; do
  repository_path="/workspaces/boxferry/.boxferry-workspace/${repository}"
  if [[ ! -d "${repository_path}/.git" ]]; then
    printf 'BoxFerry Dev Container is missing sibling repository: %s\n' "${repository_path}" >&2
    exit 1
  fi
done

printf 'BoxFerry Dev Container tooling is ready.\n'
